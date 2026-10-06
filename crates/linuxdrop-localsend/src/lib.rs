mod discovery;
pub mod download;
mod metadata;
mod rate;
pub mod reverse;
mod server;
mod tls;
#[cfg(feature = "fuzzing")]
pub mod fuzzing {
    pub fn validate_prepare_json(bytes: &[u8]) -> bool {
        bytes.len() <= 65536
            && serde_json::from_slice::<super::Prepare>(bytes)
                .is_ok_and(|request| super::validate_offer(&request, 100, 64 * 1024 * 1024).is_ok())
    }
}

use anyhow::{bail, Context, Result};
use axum::{
    body::Body,
    extract::{ConnectInfo, DefaultBodyLimit, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use futures_util::StreamExt;
use linuxdrop_core::*;
use linuxdrop_storage::{validate_name, ReceiveStore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, oneshot, Mutex},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone)]
pub struct Config {
    pub name: String,
    pub download_dir: PathBuf,
    pub identity_dir: PathBuf,
    pub visible: bool,
    pub port: u16,
    pub https: bool,
    pub multicast: bool,
    pub max_files: usize,
    pub max_bytes: u64,
    pub policy: TransferPolicy,
    pub receive_pin: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceInfo {
    alias: String,
    #[serde(default = "version")]
    version: String,
    #[serde(default)]
    device_model: Option<String>,
    #[serde(default)]
    device_type: Option<String>,
    fingerprint: String,
    #[serde(default = "port")]
    port: u16,
    #[serde(default = "https")]
    protocol: String,
    #[serde(default)]
    download: bool,
    #[serde(default)]
    announce: bool,
}
fn version() -> String {
    "2.2".into()
}
fn port() -> u16 {
    53317
}
fn https() -> String {
    "https".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireFile {
    id: String,
    file_name: String,
    size: u64,
    file_type: String,
    #[serde(default)]
    sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    metadata: Option<metadata::Metadata>,
}
#[derive(Deserialize, Serialize)]
struct Prepare {
    info: DeviceInfo,
    files: HashMap<String, WireFile>,
}
#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Prepared {
    session_id: String,
    files: HashMap<String, String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadQuery {
    session_id: String,
    file_id: String,
    token: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CancelQuery {
    session_id: String,
}
#[derive(Default, Deserialize)]
struct PrepareQuery {
    pin: Option<String>,
}

struct IncomingFile {
    index: usize,
    metadata: WireFile,
    token: String,
    started: bool,
    done: bool,
}
struct Session {
    ip: IpAddr,
    files: HashMap<String, IncomingFile>,
    transfer: Transfer,
    consent: Option<oneshot::Sender<Option<ReceiveOptions>>>,
    store: ReceiveStore,
    collision_policy: CollisionPolicy,
    accepted: bool,
    cancel: CancellationToken,
    created: Instant,
}
struct RemotePeer {
    info: DeviceInfo,
    address: SocketAddr,
    seen: Instant,
}
struct Shared {
    config: Config,
    info: DeviceInfo,
    visible: AtomicBool,
    events: EventSender,
    sessions: Mutex<HashMap<String, Session>>,
    peers: Mutex<HashMap<String, RemotePeer>>,
    outgoing: Mutex<HashMap<String, CancellationToken>>,
    stop: CancellationToken,
    store: ReceiveStore,
    prepare_gate: rate::RequestGate,
    registration_gate: rate::RequestGate,
    pin_gate: rate::RequestGate,
    pin_requests: Mutex<HashMap<String, oneshot::Sender<String>>>,
    download_decisions: Mutex<HashMap<String, oneshot::Sender<Option<ReceiveOptions>>>>,
    interfaces: std::sync::RwLock<Vec<linuxdrop_network::InterfaceAddress>>,
    bandwidth: linuxdrop_network::BandwidthLimiter,
}
type SharedState = Arc<Shared>;

pub async fn start(config: Config, events: EventSender) -> Result<CommandSender> {
    start_bound(config, events, Ipv4Addr::UNSPECIFIED).await
}
pub async fn start_with_budget(
    config: Config,
    events: EventSender,
    budget: linuxdrop_network::BandwidthLimiter,
) -> Result<CommandSender> {
    start_bound_with_budget(config, events, Ipv4Addr::UNSPECIFIED, budget).await
}

/// Explicit listener address for local protocol tools and isolated demos.
/// Multicast remains governed separately by `Config::multicast`.
pub async fn start_bound(
    config: Config,
    events: EventSender,
    bind: Ipv4Addr,
) -> Result<CommandSender> {
    let budget = linuxdrop_network::BandwidthLimiter::new(config.policy.bandwidth_bytes_per_second);
    start_bound_with_budget(config, events, bind, budget).await
}
async fn start_bound_with_budget(
    config: Config,
    events: EventSender,
    bind: Ipv4Addr,
    bandwidth: linuxdrop_network::BandwidthLimiter,
) -> Result<CommandSender> {
    if config
        .receive_pin
        .as_deref()
        .is_some_and(|pin| !valid_pin(pin))
    {
        bail!("Receive PIN must contain 4–12 digits");
    }
    let _ = rustls::crypto::ring::default_provider().install_default();
    let (cert, key, fingerprint) = tls::identity(&config.identity_dir)?;
    let info = DeviceInfo {
        alias: config.name.clone(),
        version: version(),
        device_model: Some("LinuxDrop".into()),
        device_type: Some("desktop".into()),
        fingerprint,
        port: config.port,
        protocol: if config.https { "https" } else { "http" }.into(),
        download: false,
        announce: false,
    };
    let shared = Arc::new(Shared {
        interfaces: std::sync::RwLock::new(Vec::new()),
        bandwidth,
        prepare_gate: rate::RequestGate::new(20, Duration::from_secs(60)),
        registration_gate: rate::RequestGate::new(120, Duration::from_secs(60)),
        pin_gate: rate::RequestGate::new(6, Duration::from_secs(60)),
        pin_requests: Mutex::new(HashMap::new()),
        download_decisions: Mutex::new(HashMap::new()),
        store: ReceiveStore::open(&config.download_dir)?,
        visible: AtomicBool::new(config.visible),
        config: config.clone(),
        info,
        events,
        sessions: Mutex::new(HashMap::new()),
        peers: Mutex::new(HashMap::new()),
        outgoing: Mutex::new(HashMap::new()),
        stop: CancellationToken::new(),
    });
    let routes = Router::new()
        .route("/api/localsend/v2/info", get(get_info))
        .route("/api/localsend/v2/register", post(register))
        .route("/api/localsend/v2/prepare-upload", post(prepare))
        .route(
            "/api/localsend/v2/upload",
            post(upload).layer(DefaultBodyLimit::disable()),
        )
        .route("/api/localsend/v2/cancel", post(cancel))
        .layer(DefaultBodyLimit::max(1024 * 1024))
        .with_state(shared.clone());
    let tls = if config.https {
        Some(axum_server::tls_rustls::RustlsConfig::from_pem(cert, key).await?)
    } else {
        None
    };
    let server = server::start(shared.clone(), routes, tls, bind).await?;
    let s = shared.clone();
    let maintenance = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        loop {
            tokio::select! {
                _ = s.stop.cancelled() => break,
                _ = interval.tick() => {
                    let mut sessions = s.sessions.lock().await;
                    let expired:Vec<_>=sessions.iter().filter(|(_,entry)|entry.created.elapsed()>Duration::from_secs(3600)||(!entry.files.values().any(|file|file.started)&&entry.created.elapsed()>Duration::from_secs(180))).map(|(id,_)|id.clone()).collect();
                    let mut expiry_events=Vec::new();
                    for id in expired {if let Some(mut entry)=sessions.remove(&id){entry.cancel.cancel();if !entry.transfer.is_terminal(){entry.transfer.state="failed".into();entry.transfer.error=Some("Transfer session expired".into());expiry_events.push(entry.transfer);}}}
                    drop(sessions);
                    for transfer in expiry_events{let _=s.events.send(BackendEvent::TransferUpdated(transfer)).await;}
                    let mut peers = s.peers.lock().await;
                    let expired: Vec<String> = peers.iter().filter(|(_, p)| p.seen.elapsed() > Duration::from_secs(90)).map(|(id, _)| id.clone()).collect();
                    for id in expired { peers.remove(&id); let _ = s.events.send(BackendEvent::PeerRemoved {peer_id:id}).await; }
                }
            }
        }
    });
    let (tx, mut rx) = mpsc::channel(64);
    let s = shared.clone();
    let task = tokio::spawn(async move {
        let workers = tokio_util::task::TaskTracker::new();
        while let Some(command) = rx.recv().await {
            match command {
                BackendCommand::ReceiveOffer { transfer_id, url } => {
                    let cancel = s.stop.child_token();
                    s.outgoing
                        .lock()
                        .await
                        .insert(transfer_id.clone(), cancel.clone());
                    let state = s.clone();
                    workers.spawn(async move {
                        download::run(state, transfer_id, url, cancel).await;
                    });
                }
                BackendCommand::Send {
                    transfer_id,
                    peer_id,
                    files,
                } => {
                    let s = s.clone();
                    workers.spawn(async move {
                        send(s, transfer_id, peer_id, files).await;
                    });
                }
                BackendCommand::Accept { transfer_id } => {
                    s.decide(&transfer_id, Some(ReceiveOptions::default()))
                        .await
                }
                BackendCommand::AcceptWithOptions {
                    transfer_id,
                    options,
                } => s.decide(&transfer_id, Some(options)).await,
                BackendCommand::ProvidePin { transfer_id, pin } => {
                    if valid_pin(&pin) {
                        if let Some(reply) = s.pin_requests.lock().await.remove(&transfer_id) {
                            let _ = reply.send(pin);
                        }
                    }
                }
                BackendCommand::Reject { transfer_id } => s.decide(&transfer_id, None).await,
                BackendCommand::Cancel { transfer_id } => s.cancel_transfer(&transfer_id).await,
                BackendCommand::SetVisibility { visible } => {
                    s.visible.store(visible, Ordering::Relaxed);
                }
                BackendCommand::Shutdown => break,
            }
        }
        s.stop.cancel();
        for session in s.sessions.lock().await.values() {
            session.cancel.cancel();
        }
        for token in s.outgoing.lock().await.values() {
            token.cancel();
        }
        workers.close();
        workers.wait().await;
        server.await.map_err(|error| error.to_string())?;
        maintenance.await.map_err(|error| error.to_string())?;
        Ok(())
    });
    Ok(CommandSender::track(tx, task))
}

impl Shared {
    fn interfaces(&self) -> Vec<linuxdrop_network::InterfaceAddress> {
        self.interfaces.read().unwrap().clone()
    }
    async fn error(&self, detail: String) {
        let _ = self
            .events
            .send(BackendEvent::StateChanged(BackendState {
                id: "localsend".into(),
                state: "error".into(),
                detail,
            }))
            .await;
    }
    async fn decide(&self, id: &str, options: Option<ReceiveOptions>) {
        if let Some(consent) = self.download_decisions.lock().await.remove(id) {
            let _ = consent.send(options);
            return;
        }
        if let Some(session) = self.sessions.lock().await.get_mut(id) {
            if let Some(consent) = session.consent.take() {
                let _ = consent.send(options);
            }
        }
    }
    async fn cancel_transfer(&self, id: &str) {
        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get_mut(id) {
            if session.transfer.is_terminal() {
                return;
            }
            session.cancel.cancel();
            if let Some(consent) = session.consent.take() {
                let _ = consent.send(None);
            }
            session.transfer.state = "cancelled".into();
            let _ = self
                .events
                .send(BackendEvent::TransferUpdated(session.transfer.clone()))
                .await;
        }
        drop(sessions);
        if let Some(token) = self.outgoing.lock().await.get(id) {
            token.cancel();
        }
    }
    async fn remember(&self, info: DeviceInfo, ip: IpAddr) -> Result<()> {
        if !self
            .interfaces()
            .iter()
            .any(|interface| discovery::permits(interface, ip))
        {
            bail!("Peer is outside permitted local networks");
        }
        if info.fingerprint == self.info.fingerprint {
            return Ok(());
        }
        validate_info(&info)?;
        let id = format!("localsend:{}", info.fingerprint);
        let address = SocketAddr::new(ip, info.port);
        let peer = Peer {
            id: id.clone(),
            name: info.alias.clone(),
            platform: info.device_type.clone().unwrap_or_else(|| "desktop".into()),
            protocols: vec!["localsend".into()],
            address: address.to_string(),
            available: true,
        };
        let mut peers = self.peers.lock().await;
        if peers.len() >= 1024 && !peers.contains_key(&id) {
            bail!("Peer limit reached");
        }
        peers.insert(
            id,
            RemotePeer {
                info,
                address,
                seen: Instant::now(),
            },
        );
        drop(peers);
        let _ = self.events.send(BackendEvent::PeerUpsert(peer)).await;
        Ok(())
    }
}

fn validate_info(info: &DeviceInfo) -> Result<()> {
    if info.alias.len() > 256
        || info.fingerprint.is_empty()
        || info.fingerprint.len() > 128
        || info.version.len() > 32
        || info
            .device_model
            .as_ref()
            .is_some_and(|model| model.len() > 256)
        || info
            .device_type
            .as_ref()
            .is_some_and(|kind| kind.len() > 64)
        || info.port == 0
        || !matches!(info.protocol.as_str(), "http" | "https")
    {
        bail!("Invalid peer metadata");
    }
    Ok(())
}

async fn get_info(State(s): State<SharedState>) -> Result<Json<DeviceInfo>, StatusCode> {
    if !s.visible.load(Ordering::Relaxed) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(Json(s.info.clone()))
}
async fn register(
    State(s): State<SharedState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(info): Json<DeviceInfo>,
) -> Result<Json<DeviceInfo>, StatusCode> {
    if !s.registration_gate.allow(addr.ip()) {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    s.remember(info, addr.ip())
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    if !s.visible.load(Ordering::Relaxed) {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(Json(s.info.clone()))
}
async fn prepare(
    State(s): State<SharedState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(query): Query<PrepareQuery>,
    Json(request): Json<Prepare>,
) -> Result<Json<Prepared>, StatusCode> {
    if !s.prepare_gate.allow(addr.ip()) {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    if !s.visible.load(Ordering::Relaxed) {
        return Err(StatusCode::FORBIDDEN);
    }
    if let Some(expected) = s.config.receive_pin.as_deref() {
        if !pin_matches(query.pin.as_deref().unwrap_or_default(), expected) {
            return Err(if s.pin_gate.allow(addr.ip()) {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::TOO_MANY_REQUESTS
            });
        }
    }
    let total = validate_offer(&request, s.config.max_files, s.config.max_bytes)?;
    s.remember(request.info.clone(), addr.ip())
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let id = Uuid::new_v4().to_string();
    let mut file_ids: Vec<_> = request.files.keys().cloned().collect();
    file_ids.sort();
    let transfer = Transfer {
        id: id.clone(),
        peer_id: format!("localsend:{}", request.info.fingerprint),
        peer_name: request.info.alias,
        protocol: "localsend".into(),
        direction: "incoming".into(),
        state: "waiting".into(),
        files: file_ids
            .iter()
            .map(|id| &request.files[id])
            .map(|f| TransferFile {
                name: f.file_name.clone(),
                size: f.size,
                transferred: 0,
            })
            .collect(),
        total_bytes: total,
        transferred_bytes: 0,
        error: None,
        verification_code: None,
        saved_paths: vec![],
    };
    let (tx, rx) = oneshot::channel();
    let cancellation = s.stop.child_token();
    let entries: HashMap<_, _> = file_ids
        .into_iter()
        .enumerate()
        .map(|(index, id)| {
            let metadata = request.files[&id].clone();
            (
                id,
                IncomingFile {
                    index,
                    metadata,
                    token: Uuid::new_v4().to_string(),
                    started: false,
                    done: false,
                },
            )
        })
        .collect();
    let mut sessions = s.sessions.lock().await;
    sessions.retain(|_, session| {
        !session.transfer.is_terminal() || session.created.elapsed() < Duration::from_secs(60)
    });
    if sessions.len() >= 256 {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    if sessions
        .values()
        .filter(|v| !v.transfer.is_terminal())
        .count()
        >= 8
    {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }
    sessions.insert(
        id.clone(),
        Session {
            ip: addr.ip(),
            files: entries,
            transfer: transfer.clone(),
            consent: Some(tx),
            store: s.store.clone(),
            collision_policy: CollisionPolicy::Rename,
            accepted: false,
            cancel: cancellation.clone(),
            created: Instant::now(),
        },
    );
    drop(sessions);
    let _ = s.events.send(BackendEvent::Incoming(transfer)).await;
    let options = tokio::select! { _ = cancellation.cancelled() => None, decision = tokio::time::timeout(Duration::from_secs(120),rx) => decision.ok().and_then(Result::ok).flatten() };
    let mut sessions = s.sessions.lock().await;
    let session = sessions.get_mut(&id).ok_or(StatusCode::FORBIDDEN)?;
    if options.is_none() || session.cancel.is_cancelled() || !s.visible.load(Ordering::Relaxed) {
        if !session.transfer.is_terminal() {
            session.transfer.state = "rejected".into();
        }
        let _ = s
            .events
            .send(BackendEvent::TransferUpdated(session.transfer.clone()))
            .await;
        return Err(StatusCode::FORBIDDEN);
    }
    let options = options.unwrap();
    let selection = options
        .selected_indices
        .clone()
        .unwrap_or_else(|| (0..session.transfer.files.len()).collect());
    let selected: std::collections::HashSet<_> = selection.iter().copied().collect();
    if selected.is_empty()
        || selected.len() != selection.len()
        || selected
            .iter()
            .any(|index| *index >= session.transfer.files.len())
    {
        session.transfer.state = "rejected".into();
        session.transfer.error = Some("Invalid or empty receive selection".into());
        s.events
            .send(BackendEvent::TransferUpdated(session.transfer.clone()))
            .await
            .ok();
        return Err(StatusCode::FORBIDDEN);
    }
    if let Some(directory) = options.directory {
        match ReceiveStore::open(directory) {
            Ok(store) => session.store = store,
            Err(error) => {
                session.transfer.state = "failed".into();
                session.transfer.error = Some(error.to_string());
                s.events
                    .send(BackendEvent::TransferUpdated(session.transfer.clone()))
                    .await
                    .ok();
                return Err(StatusCode::INTERNAL_SERVER_ERROR);
            }
        }
    }
    session.collision_policy = options.collision_policy;
    session
        .files
        .retain(|_, file| selected.contains(&file.index));
    let retained: Vec<_> = session
        .transfer
        .files
        .iter()
        .enumerate()
        .filter(|(index, _)| selected.contains(index))
        .map(|(index, file)| (index, file.clone()))
        .collect();
    for file in session.files.values_mut() {
        file.index = retained
            .iter()
            .position(|(index, _)| *index == file.index)
            .unwrap();
    }
    session.transfer.files = retained.into_iter().map(|(_, file)| file).collect();
    session.transfer.total_bytes = session.transfer.files.iter().map(|file| file.size).sum();
    if let Err(error) = session.store.ensure_space(session.transfer.total_bytes) {
        session.transfer.state = "failed".into();
        session.transfer.error = Some(error.to_string());
        s.events
            .send(BackendEvent::TransferUpdated(session.transfer.clone()))
            .await
            .ok();
        return Err(StatusCode::INSUFFICIENT_STORAGE);
    }
    session.accepted = true;
    session.transfer.state = "transferring".into();
    let _ = s
        .events
        .send(BackendEvent::TransferUpdated(session.transfer.clone()))
        .await;
    Ok(Json(Prepared {
        session_id: id,
        files: session
            .files
            .iter()
            .map(|(id, f)| (id.clone(), f.token.clone()))
            .collect(),
    }))
}

fn validate_offer(
    request: &Prepare,
    max_files: usize,
    max_bytes: u64,
) -> std::result::Result<u64, StatusCode> {
    if request.files.is_empty() || request.files.len() > max_files {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut total = 0u64;
    for (id, file) in &request.files {
        if id != &file.id || id.len() > 256 || validate_name(&file.file_name).is_err() {
            return Err(StatusCode::BAD_REQUEST);
        }
        total = total
            .checked_add(file.size)
            .ok_or(StatusCode::BAD_REQUEST)?;
        if total > max_bytes {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
        if file.sha256.as_ref().is_some_and(|hash| {
            hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            return Err(StatusCode::BAD_REQUEST);
        }
    }
    Ok(total)
}

async fn upload(
    State(s): State<SharedState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(q): Query<UploadQuery>,
    body: Body,
) -> StatusCode {
    let (meta, token) = {
        let mut sessions = s.sessions.lock().await;
        let Some(session) = sessions.get_mut(&q.session_id) else {
            return StatusCode::FORBIDDEN;
        };
        if session.ip != addr.ip()
            || !session.accepted
            || session.cancel.is_cancelled()
            || session.transfer.is_terminal()
        {
            return StatusCode::FORBIDDEN;
        }
        let Some(file) = session.files.get_mut(&q.file_id) else {
            return StatusCode::FORBIDDEN;
        };
        if file.token != q.token || file.started {
            return StatusCode::FORBIDDEN;
        }
        file.started = true;
        (file.metadata.clone(), session.cancel.clone())
    };
    let result = receive_body(&s, &q, &meta, token, body).await;
    let mut sessions = s.sessions.lock().await;
    let Some(session) = sessions.get_mut(&q.session_id) else {
        return StatusCode::FORBIDDEN;
    };
    match result {
        Ok(path) => {
            if session.cancel.is_cancelled() {
                return StatusCode::FORBIDDEN;
            }
            session.files.get_mut(&q.file_id).unwrap().done = true;
            session
                .transfer
                .saved_paths
                .push(path.to_string_lossy().into());
            if session.files.values().all(|f| f.done) {
                session.transfer.state = "completed".into();
            }
            let _ = s
                .events
                .send(BackendEvent::TransferUpdated(session.transfer.clone()))
                .await;
            StatusCode::OK
        }
        Err(e) => {
            if !session.transfer.is_terminal() {
                session.transfer.state = if session.cancel.is_cancelled() {
                    "cancelled"
                } else {
                    "failed"
                }
                .into();
                session.transfer.error = Some(e.to_string());
            }
            session.cancel.cancel();
            let _ = s
                .events
                .send(BackendEvent::TransferUpdated(session.transfer.clone()))
                .await;
            StatusCode::UNPROCESSABLE_ENTITY
        }
    }
}

async fn receive_body(
    s: &SharedState,
    q: &UploadQuery,
    meta: &WireFile,
    cancel: CancellationToken,
    body: Body,
) -> Result<PathBuf> {
    let (store, policy) = {
        let sessions = s.sessions.lock().await;
        let session = sessions.get(&q.session_id).context("Session expired")?;
        (session.store.clone(), session.collision_policy)
    };
    let mut pending = store.create(&meta.file_name)?;
    let mut stream = body.into_data_stream();
    let mut bytes = 0u64;
    let mut hasher = Sha256::new();
    let mut last = Instant::now();
    loop {
        let chunk = tokio::select! { _=cancel.cancelled()=>bail!("Transfer cancelled"), value=tokio::time::timeout(Duration::from_secs(30),stream.next())=>value.context("Upload timed out")? };
        let Some(chunk) = chunk else { break };
        let chunk = chunk?;
        bytes = bytes
            .checked_add(chunk.len() as u64)
            .context("Size overflow")?;
        if bytes > meta.size {
            bail!("File exceeds declared size");
        }
        s.bandwidth.acquire(chunk.len(), &cancel).await?;
        pending.file.write_all(&chunk).await?;
        hasher.update(&chunk);
        if last.elapsed() >= Duration::from_millis(100) || bytes == meta.size {
            let mut sessions = s.sessions.lock().await;
            if let Some(session) = sessions.get_mut(&q.session_id) {
                let index = session.files[&q.file_id].index;
                session.transfer.files[index].transferred = bytes;
                session.transfer.transferred_bytes =
                    session.transfer.files.iter().map(|f| f.transferred).sum();
                let _ = s
                    .events
                    .try_send(BackendEvent::TransferUpdated(session.transfer.clone()));
            }
            last = Instant::now();
        }
    }
    if bytes != meta.size {
        bail!("File size does not match offer");
    }
    let digest = hex::encode(hasher.finalize());
    if meta
        .sha256
        .as_ref()
        .is_some_and(|expected| expected.to_ascii_lowercase() != digest)
    {
        bail!("SHA-256 checksum mismatch");
    }
    if cancel.is_cancelled() {
        bail!("Transfer cancelled");
    }
    if let Some(metadata) = &meta.metadata {
        metadata.apply(&mut pending).await?;
    }
    pending.commit_with_policy(policy).await
}

fn valid_pin(pin: &str) -> bool {
    (4..=12).contains(&pin.len()) && pin.bytes().all(|byte| byte.is_ascii_digit())
}
fn pin_matches(actual: &str, expected: &str) -> bool {
    let mut difference = actual.len() ^ expected.len();
    for index in 0..12 {
        difference |= usize::from(
            actual.as_bytes().get(index).copied().unwrap_or(0)
                ^ expected.as_bytes().get(index).copied().unwrap_or(0),
        );
    }
    difference == 0 && valid_pin(actual)
}

async fn cancel(
    State(s): State<SharedState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Query(q): Query<CancelQuery>,
) -> StatusCode {
    let sessions = s.sessions.lock().await;
    if sessions
        .get(&q.session_id)
        .is_none_or(|session| session.ip != addr.ip())
    {
        return StatusCode::FORBIDDEN;
    }
    drop(sessions);
    s.cancel_transfer(&q.session_id).await;
    StatusCode::OK
}

async fn send(s: SharedState, id: String, peer_id: String, paths: Vec<SendSource>) {
    let peer = s
        .peers
        .lock()
        .await
        .get(&peer_id)
        .map(|p| (p.info.clone(), p.address));
    let mut transfer = Transfer {
        id: id.clone(),
        peer_id,
        peer_name: peer.as_ref().map(|p| p.0.alias.clone()).unwrap_or_default(),
        protocol: "localsend".into(),
        direction: "outgoing".into(),
        state: "waiting".into(),
        files: vec![],
        total_bytes: 0,
        transferred_bytes: 0,
        error: None,
        verification_code: None,
        saved_paths: vec![],
    };
    let token = s.stop.child_token();
    s.outgoing.lock().await.insert(id.clone(), token.clone());
    let result = send_files(&s, &mut transfer, paths, peer, token.clone()).await;
    match result {
        Ok(()) => transfer.state = "completed".into(),
        Err(e) => {
            transfer.state = if token.is_cancelled() {
                "cancelled"
            } else {
                "failed"
            }
            .into();
            transfer.error = Some(e.to_string());
        }
    }
    let _ = s.events.send(BackendEvent::TransferUpdated(transfer)).await;
    s.outgoing.lock().await.remove(&id);
}

async fn send_files(
    s: &SharedState,
    transfer: &mut Transfer,
    paths: Vec<SendSource>,
    peer: Option<(DeviceInfo, SocketAddr)>,
    cancel: CancellationToken,
) -> Result<()> {
    let (info, address) = peer.context("Device is no longer available")?;
    let interface = s
        .interfaces()
        .iter()
        .find(|interface| discovery::permits(interface, address.ip()))
        .cloned()
        .context("No permitted interface can reach this peer")?;
    let client = tls::client_on(&info.protocol, &info.fingerprint, Some(&interface.name))?;
    let base = format!("{}://{address}/api/localsend/v2", info.protocol);
    let mut files = HashMap::new();
    let mut opened = Vec::new();
    for source in paths {
        let name = source.name().to_owned();
        validate_name(&name)?;
        let reader = source.reader()?;
        let metadata = Some(metadata::Metadata::read(&reader)?);
        let file = tokio::fs::File::from_std(reader);
        let size = source.size();
        let file_id = Uuid::new_v4().to_string();
        transfer.total_bytes = transfer
            .total_bytes
            .checked_add(size)
            .context("Size overflow")?;
        transfer.files.push(TransferFile {
            name: name.clone(),
            size,
            transferred: 0,
        });
        files.insert(
            file_id.clone(),
            WireFile {
                id: file_id.clone(),
                file_name: name,
                size,
                file_type: "application/octet-stream".into(),
                sha256: None,
                metadata,
            },
        );
        opened.push((file_id, file, size));
    }
    if opened.is_empty() {
        bail!("No files selected");
    }
    let _ = s
        .events
        .send(BackendEvent::TransferUpdated(transfer.clone()))
        .await;
    let offer = Prepare {
        info: s.info.clone(),
        files,
    };
    let mut pin: Option<String> = None;
    let mut attempts = 0;
    let response = loop {
        let mut request = client
            .post(format!("{base}/prepare-upload"))
            .json(&offer)
            .timeout(Duration::from_secs(130));
        if let Some(pin) = &pin {
            request = request.query(&[("pin", pin)]);
        }
        let response = tokio::select! { _=cancel.cancelled()=>bail!("Transfer cancelled"),r=request.send()=>r? };
        if response.status() != StatusCode::UNAUTHORIZED {
            break response;
        }
        attempts += 1;
        if attempts > 5 {
            bail!("Recipient rejected the PIN too many times");
        }
        let (reply, receiver) = oneshot::channel();
        s.pin_requests
            .lock()
            .await
            .insert(transfer.id.clone(), reply);
        transfer.state = "pin_required".into();
        transfer.error = Some("Enter the receiving device's LocalSend PIN.".into());
        s.events
            .send(BackendEvent::TransferUpdated(transfer.clone()))
            .await
            .ok();
        let supplied = tokio::select! { _=cancel.cancelled()=>None,result=tokio::time::timeout(Duration::from_secs(120),receiver)=>result.ok().and_then(|value|value.ok()) };
        s.pin_requests.lock().await.remove(&transfer.id);
        pin = Some(supplied.context("PIN entry cancelled or expired")?);
        transfer.error = None;
        transfer.state = "waiting".into();
    };
    let response = response
        .error_for_status()
        .context("Recipient did not accept the transfer")?;
    if response.status() == StatusCode::NO_CONTENT {
        transfer.files.clear();
        transfer.total_bytes = 0;
        return Ok(());
    }
    let mut response = response;
    let mut response_body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if response_body.len().saturating_add(chunk.len()) > 1024 * 1024 {
            bail!("Recipient response exceeded metadata limit");
        }
        response_body.extend_from_slice(&chunk);
    }
    let prepared: Prepared = serde_json::from_slice(&response_body)?;
    if prepared.session_id.is_empty()
        || prepared.session_id.len() > 256
        || prepared.files.len() > opened.len()
        || prepared.files.iter().any(|(id, token)| {
            token.is_empty()
                || token.len() > 256
                || !opened.iter().any(|(offered, _, _)| offered == id)
        })
    {
        bail!("Recipient returned invalid upload tokens");
    }
    if prepared.files.is_empty() {
        bail!("Recipient did not select any files");
    }
    transfer.files = transfer
        .files
        .iter()
        .zip(&opened)
        .filter(|(_, (id, _, _))| prepared.files.contains_key(id))
        .map(|(file, _)| file.clone())
        .collect();
    transfer.total_bytes = transfer.files.iter().map(|file| file.size).sum();
    opened.retain(|(id, _, _)| prepared.files.contains_key(id));
    let result = async {
        for (index, (id, file, size)) in opened.into_iter().enumerate() {
            let Some(file_token) = prepared.files.get(&id) else {
                continue;
            };
            transfer.state = "transferring".into();
            let shared_transfer = Arc::new(std::sync::Mutex::new(transfer.clone()));
            let report = shared_transfer.clone();
            let events = s.events.clone();
            let mut last = Instant::now();
            let bandwidth = s.bandwidth.clone();
            let stream_cancel = cancel.clone();
            let stream = tokio_util::io::ReaderStream::new(file.take(size))
                .then(move |chunk| {
                    let bandwidth = bandwidth.clone();
                    let cancel = stream_cancel.clone();
                    async move {
                        let bytes = chunk?;
                        bandwidth
                            .acquire(bytes.len(), &cancel)
                            .await
                            .map_err(std::io::Error::other)?;
                        Ok::<_, std::io::Error>(bytes)
                    }
                })
                .inspect(move |chunk| {
                    if let Ok(bytes) = chunk {
                        let mut t = report.lock().unwrap();
                        t.files[index].transferred += bytes.len() as u64;
                        t.transferred_bytes = t.files.iter().map(|f| f.transferred).sum();
                        if last.elapsed() >= Duration::from_millis(100) {
                            let _ = events.try_send(BackendEvent::TransferUpdated(t.clone()));
                            last = Instant::now();
                        }
                    }
                });
            let request = client
                .post(format!("{base}/upload"))
                .query(&[
                    ("sessionId", &prepared.session_id),
                    ("fileId", &id),
                    ("token", file_token),
                ])
                .header("Content-Length", size)
                .timeout(Duration::from_secs(3600))
                .body(reqwest::Body::wrap_stream(stream))
                .send();
            let response =
                tokio::select! {_=cancel.cancelled()=>bail!("Transfer cancelled"),r=request=>r?};
            *transfer = shared_transfer.lock().unwrap().clone();
            response.error_for_status()?;
            if transfer.files[index].transferred != size {
                bail!("Recipient ended the upload before all file bytes were sent");
            }
            transfer.files[index].transferred = size;
            transfer.transferred_bytes = transfer.files.iter().map(|f| f.transferred).sum();
            let _ = s
                .events
                .send(BackendEvent::TransferUpdated(transfer.clone()))
                .await;
        }
        if transfer.transferred_bytes != transfer.total_bytes {
            bail!("Recipient accepted only part of the selection");
        }
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = client
            .post(format!("{base}/cancel"))
            .query(&[("sessionId", &prepared.session_id)])
            .timeout(Duration::from_secs(5))
            .send()
            .await;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn config(root: &std::path::Path, name: &str) -> Config {
        // Parallel fixtures must not reuse a just-released ephemeral port while
        // another fixture is still generating its TLS identity. Keep their
        // server ports distinct and below Linux's usual client port range.
        static NEXT_PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(16000);
        let port = loop {
            let candidate = NEXT_PORT.fetch_add(1, Ordering::Relaxed);
            assert!(candidate < 32000, "Test port range exhausted");
            if std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, candidate)).is_ok() {
                break candidate;
            }
        };
        Config {
            name: name.into(),
            download_dir: root.join("received"),
            identity_dir: root.join("identity"),
            visible: true,
            port,
            https: true,
            multicast: false,
            max_files: 10,
            max_bytes: 8 * 1024 * 1024,
            policy: TransferPolicy::default(),
            receive_pin: None,
        }
    }
    pub(super) async fn next_transfer(
        rx: &mut mpsc::Receiver<BackendEvent>,
        state: &str,
    ) -> Transfer {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                match rx.recv().await.unwrap() {
                    BackendEvent::Incoming(t) | BackendEvent::TransferUpdated(t)
                        if t.state == state =>
                    {
                        return t
                    }
                    _ => {}
                }
            }
        })
        .await
        .unwrap()
    }
    #[tokio::test]
    async fn pin_retry_partial_consent_and_chosen_destination() {
        let sender = tempfile::tempdir().unwrap();
        let receiver = tempfile::tempdir().unwrap();
        let sc = config(sender.path(), "Sender");
        let mut rc = config(receiver.path(), "Receiver");
        rc.receive_pin = Some("424242".into());
        let (se, mut sr) = mpsc::channel(128);
        let st = start(sc.clone(), se).await.unwrap();
        let (re, mut rr) = mpsc::channel(128);
        let rt = start(rc.clone(), re).await.unwrap();
        let (_, _, sf) = tls::identity(&sc.identity_dir).unwrap();
        let (_, _, rf) = tls::identity(&rc.identity_dir).unwrap();
        tls::client("https", &sf)
            .unwrap()
            .post(format!(
                "https://127.0.0.1:{}/api/localsend/v2/register",
                sc.port
            ))
            .json(&DeviceInfo {
                alias: "Receiver".into(),
                version: version(),
                device_model: None,
                device_type: None,
                fingerprint: rf.clone(),
                port: rc.port,
                protocol: "https".into(),
                download: false,
                announce: false,
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let keep = sender.path().join("selected.txt");
        let skip = sender.path().join("declined.txt");
        std::fs::write(&keep, b"selected bytes").unwrap();
        std::fs::write(&skip, b"do not receive").unwrap();
        st.send(BackendCommand::Send {
            transfer_id: "pin-and-partial".into(),
            peer_id: format!("localsend:{rf}"),
            files: [keep, skip]
                .iter()
                .map(|path| linuxdrop_core::SendSource::open(path).unwrap())
                .collect(),
        })
        .await
        .unwrap();
        for pin in ["0000", "424242"] {
            let challenge = next_transfer(&mut sr, "pin_required").await;
            st.send(BackendCommand::ProvidePin {
                transfer_id: challenge.id,
                pin: pin.into(),
            })
            .await
            .unwrap();
        }
        let incoming = next_transfer(&mut rr, "waiting").await;
        let selected = incoming
            .files
            .iter()
            .position(|file| file.name == "selected.txt")
            .unwrap();
        let destination = receiver.path().join("chosen");
        rt.send(BackendCommand::AcceptWithOptions {
            transfer_id: incoming.id,
            options: ReceiveOptions {
                directory: Some(destination.clone()),
                selected_indices: Some(vec![selected]),
                collision_policy: CollisionPolicy::Reject,
            },
        })
        .await
        .unwrap();
        let received = next_transfer(&mut rr, "completed").await;
        let sent = next_transfer(&mut sr, "completed").await;
        assert_eq!(received.files.len(), 1);
        assert_eq!(sent.files.len(), 1);
        assert_eq!(sent.transferred_bytes, 14);
        assert_eq!(
            std::fs::read(destination.join("selected.txt")).unwrap(),
            b"selected bytes"
        );
        assert!(!destination.join("declined.txt").exists());
        assert_eq!(std::fs::read_dir(&rc.download_dir).unwrap().count(), 0);
        st.send(BackendCommand::Shutdown).await.unwrap();
        rt.send(BackendCommand::Shutdown).await.unwrap();
    }
    #[tokio::test]
    async fn tls_send_receive_consent_zero_bytes_collision_and_rejection() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let ac = config(a.path(), "Sender");
        let bc = config(b.path(), "Receiver");
        let (ae, mut ar) = mpsc::channel(256);
        let (atx, ap) = (start(ac.clone(), ae).await.unwrap(), ac.port);
        let (be, mut br) = mpsc::channel(256);
        let btx = start(bc.clone(), be).await.unwrap();
        let (_, _, af) = tls::identity(&ac.identity_dir).unwrap();
        let (_, _, bf) = tls::identity(&bc.identity_dir).unwrap();
        let client = tls::client("https", &af).unwrap();
        let info = DeviceInfo {
            alias: "Receiver".into(),
            version: version(),
            device_model: None,
            device_type: Some("desktop".into()),
            fingerprint: bf.clone(),
            port: bc.port,
            protocol: "https".into(),
            download: false,
            announce: false,
        };
        client
            .post(format!("https://127.0.0.1:{ap}/api/localsend/v2/register"))
            .json(&info)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let wrong = tls::client("https", &"0".repeat(64)).unwrap();
        assert!(wrong
            .get(format!("https://127.0.0.1:{ap}/api/localsend/v2/info"))
            .send()
            .await
            .is_err());
        let source = a.path().join("Grüße.txt");
        let duplicate = a.path().join("another/Grüße.txt");
        tokio::fs::create_dir_all(duplicate.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&duplicate, b"Hello over pinned TLS")
            .await
            .unwrap();
        let empty = a.path().join("empty.txt");
        tokio::fs::write(&source, b"Hello over pinned TLS")
            .await
            .unwrap();
        tokio::fs::write(&empty, b"").await.unwrap();
        tokio::fs::write(bc.download_dir.join("Grüße.txt"), b"keep me")
            .await
            .unwrap();
        for path in [&source, &empty, &duplicate] {
            metadata::tests::set_test_times(path);
        }
        let sources = [source.clone(), empty, duplicate]
            .iter()
            .map(|path| linuxdrop_core::SendSource::open(path).unwrap())
            .collect();
        std::fs::rename(&source, a.path().join("moved-source")).unwrap();
        std::fs::write(&source, b"replacement must never be sent").unwrap();
        atx.send(BackendCommand::Send {
            transfer_id: "send-one".into(),
            peer_id: format!("localsend:{bf}"),
            files: sources,
        })
        .await
        .unwrap();
        let incoming = next_transfer(&mut br, "waiting").await;
        assert_eq!(
            std::fs::read_dir(&bc.download_dir).unwrap().count(),
            1,
            "no payload before acceptance"
        );
        btx.send(BackendCommand::Accept {
            transfer_id: incoming.id,
        })
        .await
        .unwrap();
        let completed = next_transfer(&mut br, "completed").await;
        assert_eq!(completed.saved_paths.len(), 3);
        for path in &completed.saved_paths {
            metadata::tests::assert_test_times(std::path::Path::new(path));
        }
        assert_eq!(completed.transferred_bytes, completed.total_bytes);
        let sent = next_transfer(&mut ar, "completed").await;
        assert_eq!(sent.id, "send-one");
        assert_eq!(
            std::fs::read(bc.download_dir.join("Grüße (1).txt")).unwrap(),
            b"Hello over pinned TLS"
        );
        assert_eq!(
            std::fs::read(bc.download_dir.join("Grüße.txt")).unwrap(),
            b"keep me"
        );
        atx.send(BackendCommand::Send {
            transfer_id: "reject-two".into(),
            peer_id: format!("localsend:{bf}"),
            files: vec![linuxdrop_core::SendSource::open(&source).unwrap()],
        })
        .await
        .unwrap();
        let incoming = next_transfer(&mut br, "waiting").await;
        btx.send(BackendCommand::Reject {
            transfer_id: incoming.id,
        })
        .await
        .unwrap();
        next_transfer(&mut br, "rejected").await;
        next_transfer(&mut ar, "failed").await;
        assert_eq!(std::fs::read_dir(&bc.download_dir).unwrap().count(), 4);
        atx.send(BackendCommand::Shutdown).await.unwrap();
        btx.send(BackendCommand::Shutdown).await.unwrap();
    }

    #[tokio::test]
    async fn malformed_and_oversized_uploads_never_publish() {
        let root = tempfile::tempdir().unwrap();
        let conf = config(root.path(), "Receiver");
        let (events, mut rx) = mpsc::channel(256);
        let commands = start(conf.clone(), events).await.unwrap();
        let (_, _, fingerprint) = tls::identity(&conf.identity_dir).unwrap();
        let client = tls::client("https", &fingerprint).unwrap();
        let base = format!("https://127.0.0.1:{}/api/localsend/v2", conf.port);
        let device = DeviceInfo {
            alias: "Client".into(),
            version: version(),
            device_model: None,
            device_type: None,
            fingerprint: "a".repeat(64),
            port: 12345,
            protocol: "https".into(),
            download: false,
            announce: false,
        };
        let file = WireFile {
            id: "one".into(),
            file_name: "../outside".into(),
            size: 1,
            file_type: "text/plain".into(),
            sha256: None,
            metadata: None,
        };
        let result = client
            .post(format!("{base}/prepare-upload"))
            .json(&Prepare {
                info: device.clone(),
                files: HashMap::from([("one".into(), file.clone())]),
            })
            .send()
            .await
            .unwrap();
        assert_eq!(result.status(), StatusCode::BAD_REQUEST);
        let mut valid = file;
        valid.file_name = "safe.txt".into();
        let pending = {
            let client = client.clone();
            let url = format!("{base}/prepare-upload");
            tokio::spawn(async move {
                client
                    .post(url)
                    .json(&Prepare {
                        info: device,
                        files: HashMap::from([("one".into(), valid)]),
                    })
                    .send()
                    .await
                    .unwrap()
                    .json::<Prepared>()
                    .await
                    .unwrap()
            })
        };
        let offer = next_transfer(&mut rx, "waiting").await;
        commands
            .send(BackendCommand::Accept {
                transfer_id: offer.id,
            })
            .await
            .unwrap();
        let prepared = pending.await.unwrap();
        let url = format!(
            "{base}/upload?sessionId={}&fileId=one&token={}",
            prepared.session_id, prepared.files["one"]
        );
        let response = client.post(&url).body("too much").send().await.unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let failed = next_transfer(&mut rx, "failed").await;
        assert!(failed.error.unwrap().contains("declared size"));
        assert_eq!(std::fs::read_dir(&conf.download_dir).unwrap().count(), 0);
        assert_eq!(
            client.post(url).body("x").send().await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
        commands.send(BackendCommand::Shutdown).await.unwrap();
    }

    #[tokio::test]
    async fn malicious_receivers_cannot_expand_metadata_or_fake_uploaded_bytes() {
        for behavior in ["oversized", "unknown-id", "early-success"] {
            let root = tempfile::tempdir().unwrap();
            let mut conf = config(root.path(), "Sender");
            conf.https = false;
            let (events, mut receiver) = mpsc::channel(256);
            let commands = start(conf.clone(), events).await.unwrap();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let router = axum::Router::new()
                .route(
                    "/api/localsend/v2/prepare-upload",
                    axum::routing::post(move |Json(offer): Json<Prepare>| async move {
                        if behavior == "oversized" {
                            return "x".repeat(1024 * 1024 + 1);
                        }
                        let ids = if behavior == "unknown-id" {
                            vec!["not-an-offered-file".to_owned()]
                        } else {
                            offer.files.into_keys().collect()
                        };
                        serde_json::to_string(&Prepared {
                            session_id: "test".into(),
                            files: ids.into_iter().map(|id| (id, "token".into())).collect(),
                        })
                        .unwrap()
                    }),
                )
                .route(
                    "/api/localsend/v2/upload",
                    axum::routing::post(|| async { StatusCode::OK }),
                );
            let server = tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            });
            let fingerprint = "f".repeat(64);
            reqwest::Client::new()
                .post(format!(
                    "http://127.0.0.1:{}/api/localsend/v2/register",
                    conf.port
                ))
                .json(&DeviceInfo {
                    alias: "Untrusted receiver".into(),
                    version: version(),
                    device_model: None,
                    device_type: None,
                    fingerprint: fingerprint.clone(),
                    port,
                    protocol: "http".into(),
                    download: false,
                    announce: false,
                })
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap();
            let source = root.path().join("payload.bin");
            std::fs::File::create(&source)
                .unwrap()
                .set_len(64 * 1024 * 1024)
                .unwrap();
            commands
                .send(BackendCommand::Send {
                    transfer_id: behavior.into(),
                    peer_id: format!("localsend:{fingerprint}"),
                    files: vec![linuxdrop_core::SendSource::open(&source).unwrap()],
                })
                .await
                .unwrap();
            let failed = next_transfer(&mut receiver, "failed").await;
            match behavior {
                "oversized" => assert!(failed.error.unwrap().contains("metadata limit")),
                "unknown-id" => assert!(failed.error.unwrap().contains("invalid upload tokens")),
                _ => assert!(failed.transferred_bytes < failed.total_bytes),
            }
            commands.send(BackendCommand::Shutdown).await.unwrap();
            server.abort();
        }
    }
}
