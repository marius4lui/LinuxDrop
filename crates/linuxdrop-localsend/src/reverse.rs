//! Explicit, expiring LocalSend v2.2 reverse-download offers.
//! HTTP is intentional for browser interoperability; the offer PIN is mandatory.
use anyhow::{bail, Context, Result};
use axum::{
    body::Body,
    extract::{ConnectInfo, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const MAX_SESSIONS: usize = 16;
const MAX_DOWNLOADS: usize = 16;
const MAX_ATTEMPT_IPS: usize = 256;

pub struct OfferConfig {
    pub alias: String,
    pub bind: SocketAddr,
    pub expires_after: Duration,
    pub max_files: usize,
    pub max_bytes: u64,
}

/// Dropping the offer stops its listener and outstanding streams. Holding this
/// object is the application's explicit authorization to share these files.
pub struct ReverseOffer {
    pub address: SocketAddr,
    pub pin: String,
    stop: CancellationToken,
    completion: linuxdrop_core::ShutdownReceipt,
    accepting: Arc<std::sync::Mutex<bool>>,
    downloads: Arc<Semaphore>,
}
impl ReverseOffer {
    pub fn stop(&self) {
        self.stop.cancel();
    }
    pub async fn shutdown(&self) -> Result<(), String> {
        self.stop.cancel();
        self.completion.wait().await
    }
    pub fn is_active(&self) -> bool {
        // Keep an unresolved cleanup visible and retryable after cancellation.
        !self.completion.is_complete()
    }
    /// Close admission without interrupting streams that already hold a permit.
    pub fn quiesce(&self) {
        *self
            .accepting
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = false;
    }
    pub fn quiesce_if_idle(&self) -> bool {
        let mut accepting = self
            .accepting
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.active_downloads() != 0 {
            return false;
        }
        *accepting = false;
        true
    }
    pub fn active_downloads(&self) -> usize {
        MAX_DOWNLOADS - self.downloads.available_permits()
    }
}
impl Drop for ReverseOffer {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileMetadata {
    id: String,
    file_name: String,
    size: u64,
    file_type: String,
    sha256: Option<String>,
}
struct Source {
    file: Arc<std::fs::File>,
    metadata: FileMetadata,
}
struct DownloadSession {
    ip: IpAddr,
}
struct OfferState {
    accepting: Arc<std::sync::Mutex<bool>>,
    readers: tokio_util::task::TaskTracker,
    bandwidth: linuxdrop_network::BandwidthLimiter,
    alias: String,
    fingerprint: String,
    pin: String,
    created: Instant,
    expires_after: Duration,
    files: HashMap<String, Source>,
    sessions: Mutex<HashMap<String, DownloadSession>>,
    attempts: Mutex<HashMap<IpAddr, Vec<Instant>>>,
    downloads: Arc<Semaphore>,
    stop: CancellationToken,
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrepareQuery {
    session_id: Option<String>,
    pin: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadQuery {
    session_id: String,
    file_id: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Info {
    alias: String,
    version: &'static str,
    device_model: &'static str,
    device_type: &'static str,
    fingerprint: String,
    download: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Prepared {
    info: Info,
    session_id: String,
    files: HashMap<String, FileMetadata>,
}
impl OfferState {
    fn active(&self) -> bool {
        !self.stop.is_cancelled()
            && self.created.elapsed() < self.expires_after
            && *self
                .accepting
                .lock()
                .unwrap_or_else(|error| error.into_inner())
    }
    fn info(&self) -> Info {
        Info {
            alias: self.alias.clone(),
            version: "2.2",
            device_model: "LinuxDrop",
            device_type: "desktop",
            fingerprint: self.fingerprint.clone(),
            download: self.active(),
        }
    }
}

/// Sources are opened before publishing. FDs prevent subsequent path/symlink
/// substitution; independent read_at offsets allow concurrent downloads safely.
pub async fn start_offer(config: OfferConfig, paths: Vec<PathBuf>) -> Result<ReverseOffer> {
    start_offer_with_budget(
        config,
        paths,
        linuxdrop_network::BandwidthLimiter::new(None),
    )
    .await
}
pub async fn start_offer_with_budget(
    config: OfferConfig,
    paths: Vec<PathBuf>,
    bandwidth: linuxdrop_network::BandwidthLimiter,
) -> Result<ReverseOffer> {
    if paths.is_empty() || paths.len() > config.max_files || paths.len() > 256 {
        bail!("Invalid download offer file count");
    }
    let sources = paths
        .into_iter()
        .map(linuxdrop_core::SendSource::open)
        .collect::<std::io::Result<Vec<_>>>()?;
    start_sources_with_budget(config, sources, bandwidth).await
}
pub async fn start_sources_with_budget(
    config: OfferConfig,
    sources: Vec<linuxdrop_core::SendSource>,
    bandwidth: linuxdrop_network::BandwidthLimiter,
) -> Result<ReverseOffer> {
    if sources.is_empty() || sources.len() > config.max_files || sources.len() > 256 {
        bail!("Invalid download offer file count");
    }
    if config.expires_after.is_zero() || config.expires_after > Duration::from_secs(3600) {
        bail!("Download offers must expire within one hour");
    }
    let mut files = HashMap::new();
    let mut total = 0u64;
    for source in sources {
        let file = source.reader()?;
        total = total
            .checked_add(source.size())
            .context("Offer size overflow")?;
        if total > config.max_bytes {
            bail!("Download offer exceeds configured size limit");
        }
        let name = source.name().to_owned();
        linuxdrop_storage::validate_name(&name)?;
        let id = Uuid::new_v4().to_string();
        files.insert(
            id.clone(),
            Source {
                file: Arc::new(file),
                metadata: FileMetadata {
                    id,
                    file_name: name,
                    size: source.size(),
                    file_type: "application/octet-stream".into(),
                    sha256: None,
                },
            },
        );
    }
    let random = Uuid::new_v4();
    let bytes = random.as_bytes();
    let pin = format!(
        "{:06}",
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) % 1_000_000
    );
    let stop = CancellationToken::new();
    let accepting = Arc::new(std::sync::Mutex::new(true));
    let downloads = Arc::new(Semaphore::new(MAX_DOWNLOADS));
    let readers = tokio_util::task::TaskTracker::new();
    let state = Arc::new(OfferState {
        accepting: accepting.clone(),
        readers: readers.clone(),
        bandwidth,
        alias: config.alias,
        fingerprint: Uuid::new_v4().to_string(),
        pin: pin.clone(),
        created: Instant::now(),
        expires_after: config.expires_after,
        files,
        sessions: Mutex::new(HashMap::new()),
        attempts: Mutex::new(HashMap::new()),
        downloads: downloads.clone(),
        stop: stop.clone(),
    });
    let router = Router::new()
        .route("/", get(index))
        .route("/app.js", get(script))
        .route("/api/localsend/v2/prepare-download", post(prepare))
        .route("/api/localsend/v2/download", get(download))
        .route("/api/localsend/v2/info", get(info))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(config.bind).await?;
    let address = listener.local_addr()?;
    let cancel = stop.clone();
    let task = tokio::spawn(async move {
        let handle = axum_server::Handle::new();
        let server = axum_server::from_tcp(listener.into_std().map_err(|error| error.to_string())?)
            .handle(handle.clone())
            .serve(router.into_make_service_with_connect_info::<SocketAddr>());
        tokio::pin!(server);
        let result = tokio::select! {
            result = &mut server => result,
            _ = async {
                tokio::select! { _ = cancel.cancelled() => {}, _ = tokio::time::sleep(config.expires_after) => {} }
            } => {
                cancel.cancel();
                handle.shutdown();
                server.await
            },
        };
        cancel.cancel();
        handle.shutdown();
        // Forced shutdown returns before detached HTTP connections are dropped.
        while handle.connection_count() != 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        readers.close();
        readers.wait().await;
        result.map_err(|error| error.to_string())
    });
    Ok(ReverseOffer {
        address,
        pin,
        stop,
        accepting,
        downloads,
        completion: linuxdrop_core::ShutdownReceipt::track(task),
    })
}

async fn info(State(state): State<Arc<OfferState>>) -> Response {
    if !state.active() {
        return StatusCode::GONE.into_response();
    }
    private(Json(state.info()).into_response())
}
async fn prepare(
    State(state): State<Arc<OfferState>>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    Query(query): Query<PrepareQuery>,
) -> Response {
    if !state.active() {
        return StatusCode::GONE.into_response();
    }
    let mut sessions = state.sessions.lock().await;
    if let Some(id) = &query.session_id {
        if sessions
            .get(id)
            .is_some_and(|session| session.ip == address.ip())
        {
            return private(Json(prepared(&state, id.clone())).into_response());
        }
    }
    let mut attempts = state.attempts.lock().await;
    attempts.retain(|_, entries| {
        entries.retain(|t| t.elapsed() < Duration::from_secs(60));
        !entries.is_empty()
    });
    if attempts.len() >= MAX_ATTEMPT_IPS && !attempts.contains_key(&address.ip()) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let tries = attempts.entry(address.ip()).or_default();
    if tries.len() >= 8 {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    tries.push(Instant::now());
    if !constant_time_equal(
        query.pin.as_deref().unwrap_or_default().as_bytes(),
        state.pin.as_bytes(),
    ) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if sessions.len() >= MAX_SESSIONS {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let id = Uuid::new_v4().to_string();
    sessions.insert(id.clone(), DownloadSession { ip: address.ip() });
    private(Json(prepared(&state, id)).into_response())
}
fn constant_time_equal(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |diff, (a, b)| diff | (a ^ b)) == 0
}
fn prepared(state: &OfferState, id: String) -> Prepared {
    Prepared {
        info: state.info(),
        session_id: id,
        files: state
            .files
            .iter()
            .map(|(id, source)| (id.clone(), source.metadata.clone()))
            .collect(),
    }
}

async fn download(
    State(state): State<Arc<OfferState>>,
    ConnectInfo(address): ConnectInfo<SocketAddr>,
    Query(query): Query<DownloadQuery>,
) -> Response {
    if !state.active() {
        return StatusCode::GONE.into_response();
    }
    if !state
        .sessions
        .lock()
        .await
        .get(&query.session_id)
        .is_some_and(|s| s.ip == address.ip())
    {
        return StatusCode::FORBIDDEN.into_response();
    }
    let Some(source) = state.files.get(&query.file_id) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let permit = {
        let accepting = state
            .accepting
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !*accepting || state.stop.is_cancelled() {
            return StatusCode::GONE.into_response();
        }
        let Ok(permit) = state.downloads.clone().try_acquire_owned() else {
            return StatusCode::TOO_MANY_REQUESTS.into_response();
        };
        permit
    };
    if source.file.metadata().map(|m| m.len()).ok() != Some(source.metadata.size) {
        return StatusCode::CONFLICT.into_response();
    }
    let file = source.file.clone();
    let size = source.metadata.size;
    let stop = state.stop.clone();
    let readers = state.readers.clone();
    let stream = futures_util::stream::try_unfold(
        (file, 0u64, stop, permit, state.bandwidth.clone()),
        move |(file, offset, stop, permit, bandwidth)| {
            let readers = readers.clone();
            async move {
                if stop.is_cancelled() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::Interrupted,
                        "Offer ended",
                    ));
                }
                if offset == size {
                    return Ok(None);
                }
                let handle = file.clone();
                let bytes = readers
                    .spawn_blocking(move || {
                        let mut bytes = vec![0u8; ((size - offset).min(64 * 1024)) as usize];
                        #[cfg(unix)]
                        let count = {
                            use std::os::unix::fs::FileExt;
                            handle.read_at(&mut bytes, offset)?
                        };
                        #[cfg(windows)]
                        let count = {
                            use std::os::windows::fs::FileExt;
                            handle.seek_read(&mut bytes, offset)?
                        };
                        if count == 0 {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::UnexpectedEof,
                                "Source changed during download",
                            ));
                        }
                        bytes.truncate(count);
                        Ok::<_, std::io::Error>(bytes)
                    })
                    .await
                    .map_err(std::io::Error::other)??;
                bandwidth
                    .acquire(bytes.len(), &stop)
                    .await
                    .map_err(std::io::Error::other)?;
                let next = offset + bytes.len() as u64;
                Ok(Some((bytes, (file, next, stop, permit, bandwidth))))
            }
        },
    );
    let encoded = source
        .metadata
        .file_name
        .as_bytes()
        .iter()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(b) {
                (*b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect::<String>();
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&size.to_string()).unwrap(),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "attachment; filename=\"download\"; filename*=UTF-8''{encoded}"
        ))
        .unwrap(),
    );
    private((headers, Body::from_stream(stream)).into_response())
}

fn private(mut response: Response) -> Response {
    let h = response.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert("content-security-policy",HeaderValue::from_static("default-src 'self'; script-src 'self'; style-src 'unsafe-inline'; frame-ancestors 'none'; form-action 'self'"));
    response
}
async fn index(State(state): State<Arc<OfferState>>) -> Response {
    if !state.active() {
        return StatusCode::GONE.into_response();
    }
    private(Html(INDEX).into_response())
}
async fn script() -> Response {
    private(
        (
            [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
            SCRIPT,
        )
            .into_response(),
    )
}
const INDEX: &str = r#"<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>LinuxDrop · Download</title><style>body{font:16px system-ui;background:#10131d;color:#eff2ff;display:grid;min-height:95vh;place-items:center;margin:0}main{width:min(440px,85vw);padding:32px;border:1px solid #30374c;border-radius:28px;background:#181d2b}h1{font-size:28px;margin-top:0}p{line-height:1.5;color:#b9c2d9}input,button{font:inherit;padding:14px;border-radius:12px;border:1px solid #536285}input{width:8em;background:#10131d;color:white;letter-spacing:.3em}button{background:#aec5ff;cursor:pointer;color:#10254d}a{display:block;color:#aec5ff;padding:14px 0;overflow-wrap:anywhere}small{color:#96a2bd}</style><main><h1>LinuxDrop</h1><p id="description">Enter the six-digit code shown on the sender's screen.</p><form id="code"><label for="pin">Sharing code</label><p><input id="pin" inputmode="numeric" pattern="[0-9]{6}" maxlength="6" required autocomplete="off" autofocus> <button>Connect</button></p></form><div id="files" aria-live="polite"></div><p id="status" role="status"></p><small>This local download uses HTTP. Only share on a network you trust. The offer expires automatically.</small></main><script src="/app.js"></script></html>"#;
const SCRIPT: &str = r#"const form=document.querySelector('#code'),status=document.querySelector('#status'),files=document.querySelector('#files');async function connect(pin){status.textContent='Connecting…';const query=new URLSearchParams();const session=sessionStorage.getItem('linuxdrop-session');if(session)query.set('sessionId',session);if(pin)query.set('pin',pin);try{const response=await fetch('/api/localsend/v2/prepare-download?'+query,{method:'POST',cache:'no-store'});if(!response.ok)throw new Error(response.status===401?'That code is not valid.':response.status===429?'Too many attempts. Try again in a minute.':'This offer is no longer available.');const data=await response.json();sessionStorage.setItem('linuxdrop-session',data.sessionId);files.replaceChildren();document.querySelector('#description').textContent='Files shared by '+data.info.alias;for(const file of Object.values(data.files)){const link=document.createElement('a');link.textContent=file.fileName+' · '+new Intl.NumberFormat().format(file.size)+' bytes';link.href='/api/localsend/v2/download?'+new URLSearchParams({sessionId:data.sessionId,fileId:file.id});link.download=file.fileName;files.append(link);}form.hidden=true;status.textContent='Choose a file to download.';}catch(error){status.textContent=error.message;}}form.addEventListener('submit',event=>{event.preventDefault();connect(document.querySelector('#pin').value);});if(sessionStorage.getItem('linuxdrop-session'))connect();"#;

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn offer_requires_pin_and_session_streams_exact_bytes_and_expires() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Grüße file.txt");
        let bytes = vec![42u8; 150_000];
        std::fs::write(&path, &bytes).unwrap();
        let source = linuxdrop_core::SendSource::open(&path).unwrap();
        std::fs::rename(&path, dir.path().join("moved-source")).unwrap();
        std::fs::write(&path, b"replacement must never be sent").unwrap();
        let offer = start_sources_with_budget(
            OfferConfig {
                alias: "Test".into(),
                bind: "127.0.0.1:0".parse().unwrap(),
                expires_after: Duration::from_secs(60),
                max_files: 10,
                max_bytes: 1_000_000,
            },
            vec![source],
            linuxdrop_network::BandwidthLimiter::new(Some(bytes.len() as u64)),
        )
        .await
        .unwrap();
        let base = format!("http://{}", offer.address);
        let client = reqwest::Client::new();
        assert_eq!(
            client
                .post(format!("{base}/api/localsend/v2/prepare-download"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        let data: serde_json::Value = client
            .post(format!("{base}/api/localsend/v2/prepare-download"))
            .query(&[("pin", &offer.pin)])
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let session = data["sessionId"].as_str().unwrap();
        let file = data["files"].as_object().unwrap().keys().next().unwrap();
        let url = format!("{base}/api/localsend/v2/download?sessionId={session}&fileId={file}");
        let started = Instant::now();
        let (first, second) = tokio::join!(client.get(&url).send(), client.get(&url).send());
        let (first, second) = tokio::join!(first.unwrap().bytes(), second.unwrap().bytes());
        assert_eq!(first.unwrap().as_ref(), bytes);
        assert_eq!(second.unwrap().as_ref(), bytes);
        assert!(
            started.elapsed() >= Duration::from_millis(1950),
            "Concurrent downloads must share their supplied payload budget"
        );
        assert_eq!(
            client
                .get(format!(
                    "{base}/api/localsend/v2/download?sessionId=wrong&fileId={file}"
                ))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        let refresh = client
            .post(format!(
                "{base}/api/localsend/v2/prepare-download?sessionId={session}"
            ))
            .send()
            .await
            .unwrap();
        assert!(refresh.status().is_success());
        offer.shutdown().await.unwrap();
        assert!(client.get(&url).send().await.is_err());
    }
    async fn slow_offer(
        root: &std::path::Path,
        expires_after: Duration,
        rate: u64,
    ) -> (ReverseOffer, String) {
        let path = root.join("stream.bin");
        std::fs::write(&path, vec![42u8; 16_384]).unwrap();
        let offer = start_offer_with_budget(
            OfferConfig {
                alias: "Lifecycle test".into(),
                bind: "127.0.0.1:0".parse().unwrap(),
                expires_after,
                max_files: 1,
                max_bytes: 16_384,
            },
            vec![path],
            linuxdrop_network::BandwidthLimiter::new(Some(rate)),
        )
        .await
        .unwrap();
        let prepared: serde_json::Value = reqwest::Client::new()
            .post(format!(
                "http://{}/api/localsend/v2/prepare-download?pin={}",
                offer.address, offer.pin
            ))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let session = prepared["sessionId"].as_str().unwrap();
        let file = prepared["files"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap();
        let url = format!(
            "http://{}/api/localsend/v2/download?sessionId={session}&fileId={file}",
            offer.address
        );
        (offer, url)
    }
    async fn wait_for_download(offer: &ReverseOffer) {
        tokio::time::timeout(Duration::from_secs(2), async {
            while offer.active_downloads() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn quiescing_preserves_admitted_downloads_and_rejects_new_ones() {
        let dir = tempfile::tempdir().unwrap();
        let (offer, url) = slow_offer(dir.path(), Duration::from_secs(60), 16_384).await;
        let target = url.clone();
        let download =
            tokio::spawn(async move { reqwest::get(target).await.unwrap().bytes().await.unwrap() });
        wait_for_download(&offer).await;
        assert!(
            !offer.quiesce_if_idle(),
            "Replacing a link may not cancel an active stream"
        );
        offer.quiesce();
        assert_eq!(reqwest::get(url).await.unwrap().status(), StatusCode::GONE);
        assert_eq!(download.await.unwrap().as_ref(), vec![42u8; 16_384]);
        assert_eq!(offer.active_downloads(), 0);
        offer.shutdown().await.unwrap();
    }
    #[tokio::test]
    async fn stop_and_expiry_drain_streams_and_incomplete_requests_before_port_reuse() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for explicit_stop in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let lifetime = if explicit_stop {
                Duration::from_secs(60)
            } else {
                Duration::from_millis(250)
            };
            let (offer, url) = slow_offer(dir.path(), lifetime, 1).await;
            let download = tokio::spawn(async move {
                match reqwest::get(url).await {
                    Ok(response) => response.bytes().await,
                    Err(error) => Err(error),
                }
            });
            wait_for_download(&offer).await;
            let mut incomplete = tokio::net::TcpStream::connect(offer.address).await.unwrap();
            incomplete
                .write_all(b"GET / HTTP/1.1\r\nHost:")
                .await
                .unwrap();
            if explicit_stop {
                offer.shutdown().await.unwrap();
            } else {
                offer.completion.wait().await.unwrap();
            }
            assert!(
                download.await.unwrap().is_err(),
                "Revocation must terminate the blocked body"
            );
            assert_eq!(offer.active_downloads(), 0);
            assert!(!offer.is_active());
            let mut tail = Vec::new();
            let _ = tokio::time::timeout(Duration::from_secs(1), incomplete.read_to_end(&mut tail))
                .await
                .unwrap();
            let _replacement = tokio::net::TcpListener::bind(offer.address).await.unwrap();
            offer.shutdown().await.unwrap(); // persistent receipt
        }
    }

    #[tokio::test]
    async fn repeated_bad_codes_are_rate_limited() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty");
        std::fs::write(&path, []).unwrap();
        let offer = start_offer(
            OfferConfig {
                alias: "Test".into(),
                bind: "127.0.0.1:0".parse().unwrap(),
                expires_after: Duration::from_secs(60),
                max_files: 1,
                max_bytes: 1,
            },
            vec![path],
        )
        .await
        .unwrap();
        let client = reqwest::Client::new();
        let url = format!(
            "http://{}/api/localsend/v2/prepare-download?pin=wrong",
            offer.address
        );
        for _ in 0..8 {
            assert_eq!(
                client.post(&url).send().await.unwrap().status(),
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(
            client.post(&url).send().await.unwrap().status(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }
}
