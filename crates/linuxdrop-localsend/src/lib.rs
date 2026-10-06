pub mod reverse;
mod tls;

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
    "2.1".into()
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
    consent: Option<oneshot::Sender<bool>>,
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
}
type SharedState = Arc<Shared>;

pub async fn start(config: Config, events: EventSender) -> Result<CommandSender> {
    start_bound(config, events, Ipv4Addr::UNSPECIFIED).await
}

/// Explicit listener address for local protocol tools and isolated demos.
/// Multicast remains governed separately by `Config::multicast`.
pub async fn start_bound(
    config: Config,
    events: EventSender,
    bind: Ipv4Addr,
) -> Result<CommandSender> {
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
    let listener = std::net::TcpListener::bind((bind, config.port))?;
    listener.set_nonblocking(true)?;
    let handle = axum_server::Handle::new();
    let stop_handle = handle.clone();
    let stopper = shared.stop.clone();
    tokio::spawn(async move {
        stopper.cancelled().await;
        stop_handle.graceful_shutdown(Some(Duration::from_secs(2)));
    });
    let state = shared.clone();
    if config.https {
        let tls = axum_server::tls_rustls::RustlsConfig::from_pem(cert, key).await?;
        tokio::spawn(async move {
            if let Err(error) = axum_server::from_tcp_rustls(listener, tls)
                .handle(handle)
                .serve(routes.into_make_service_with_connect_info::<SocketAddr>())
                .await
            {
                state.error(format!("LocalSend server: {error}")).await;
            }
        });
    } else {
        tokio::spawn(async move {
            if let Err(error) = axum_server::from_tcp(listener)
                .handle(handle)
                .serve(routes.into_make_service_with_connect_info::<SocketAddr>())
                .await
            {
                state.error(format!("LocalSend server: {error}")).await;
            }
        });
    }
    if config.multicast {
        let s = shared.clone();
        tokio::spawn(async move {
            if let Err(e) = discovery(s.clone()).await {
                s.error(format!("Multicast discovery: {e}")).await;
            }
        });
    }
    let (tx, mut rx) = mpsc::channel(64);
    let s = shared.clone();
    tokio::spawn(async move {
        while let Some(command) = rx.recv().await {
            match command {
                BackendCommand::Send {
                    transfer_id,
                    peer_id,
                    files,
                } => {
                    let s = s.clone();
                    tokio::spawn(async move {
                        send(s, transfer_id, peer_id, files).await;
                    });
                }
                BackendCommand::Accept { transfer_id } => s.decide(&transfer_id, true).await,
                BackendCommand::Reject { transfer_id } => s.decide(&transfer_id, false).await,
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
    });
    let s = shared.clone();
    tokio::spawn(async move {
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
    let _ = shared
        .events
        .send(BackendEvent::StateChanged(BackendState {
            id: "localsend".into(),
            state: "ready".into(),
            detail: format!("{} on port {}", shared.info.protocol, config.port),
        }))
        .await;
    Ok(tx)
}

impl Shared {
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
    async fn decide(&self, id: &str, accept: bool) {
        if let Some(session) = self.sessions.lock().await.get_mut(id) {
            if let Some(consent) = session.consent.take() {
                let _ = consent.send(accept);
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
                let _ = consent.send(false);
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
        if info.fingerprint == self.info.fingerprint {
            return Ok(());
        }
        if info.alias.len() > 256
            || info.fingerprint.len() > 128
            || info.port == 0
            || !matches!(info.protocol.as_str(), "http" | "https")
        {
            bail!("Invalid peer metadata");
        }
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
    Json(request): Json<Prepare>,
) -> Result<Json<Prepared>, StatusCode> {
    if !s.visible.load(Ordering::Relaxed) {
        return Err(StatusCode::FORBIDDEN);
    }
    if request.files.is_empty() || request.files.len() > s.config.max_files {
        return Err(StatusCode::BAD_REQUEST);
    }
    let mut total = 0u64;
    for (id, f) in &request.files {
        if id != &f.id || id.len() > 256 || validate_name(&f.file_name).is_err() {
            return Err(StatusCode::BAD_REQUEST);
        }
        total = total.checked_add(f.size).ok_or(StatusCode::BAD_REQUEST)?;
        if total > s.config.max_bytes {
            return Err(StatusCode::PAYLOAD_TOO_LARGE);
        }
        if f.sha256
            .as_ref()
            .is_some_and(|h| h.len() != 64 || !h.chars().all(|c| c.is_ascii_hexdigit()))
        {
            return Err(StatusCode::BAD_REQUEST);
        }
    }
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
            accepted: false,
            cancel: cancellation.clone(),
            created: Instant::now(),
        },
    );
    drop(sessions);
    let _ = s.events.send(BackendEvent::Incoming(transfer)).await;
    let accepted = tokio::select! { _ = cancellation.cancelled() => false, decision = tokio::time::timeout(Duration::from_secs(120),rx) => matches!(decision,Ok(Ok(true))) };
    let mut sessions = s.sessions.lock().await;
    let session = sessions.get_mut(&id).ok_or(StatusCode::FORBIDDEN)?;
    if !accepted || session.cancel.is_cancelled() || !s.visible.load(Ordering::Relaxed) {
        if !session.transfer.is_terminal() {
            session.transfer.state = "rejected".into();
        }
        let _ = s
            .events
            .send(BackendEvent::TransferUpdated(session.transfer.clone()))
            .await;
        return Err(StatusCode::FORBIDDEN);
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
    let mut pending = s.store.create(&meta.file_name)?;
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
    pending.commit().await
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

async fn discovery(s: SharedState) -> Result<()> {
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;
    socket.set_reuse_address(true)?;
    socket.set_reuse_port(true)?;
    socket.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, s.config.port)).into())?;
    socket.join_multicast_v4(&Ipv4Addr::new(224, 0, 0, 167), &Ipv4Addr::UNSPECIFIED)?;
    socket.set_multicast_ttl_v4(1)?;
    socket.set_nonblocking(true)?;
    let udp = tokio::net::UdpSocket::from_std(socket.into())?;
    let mut interval = tokio::time::interval(Duration::from_secs(20));
    let target = SocketAddr::from((Ipv4Addr::new(224, 0, 0, 167), s.config.port));
    let mut buffer = vec![0u8; 65536];
    loop {
        tokio::select! {
            _=s.stop.cancelled()=>break,
            _=interval.tick()=> {
                if s.visible.load(Ordering::Relaxed) {let mut info=s.info.clone();info.announce=true;udp.send_to(&serde_json::to_vec(&info)?,target).await?;}
            },
            packet=udp.recv_from(&mut buffer)=> {
                let (length,addr)=packet?;
                if let Ok(info)=serde_json::from_slice::<DeviceInfo>(&buffer[..length]) {
                    if info.fingerprint==s.info.fingerprint {continue;}
                    let announce=info.announce;
                    if s.remember(info,addr.ip()).await.is_ok() && announce && s.visible.load(Ordering::Relaxed) {
                        udp.send_to(&serde_json::to_vec(&s.info)?,target).await?;
                    }
                }
            }
        }
    }
    Ok(())
}

async fn send(s: SharedState, id: String, peer_id: String, paths: Vec<PathBuf>) {
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
    paths: Vec<PathBuf>,
    peer: Option<(DeviceInfo, SocketAddr)>,
    cancel: CancellationToken,
) -> Result<()> {
    let (info, address) = peer.context("Device is no longer available")?;
    let client = tls::client(&info.protocol, &info.fingerprint)?;
    let base = format!("{}://{address}/api/localsend/v2", info.protocol);
    let mut files = HashMap::new();
    let mut opened = Vec::new();
    for path in paths {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .context("Invalid UTF-8 file name")?
            .to_string();
        validate_name(&name)?;
        let file = tokio::fs::File::open(&path).await?;
        let metadata = file.metadata().await?;
        if !metadata.is_file() {
            bail!("Only regular files can be sent");
        }
        let file_id = Uuid::new_v4().to_string();
        let size = metadata.len();
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
    let response = tokio::select! { _=cancel.cancelled()=>bail!("Transfer cancelled"),r=client.post(format!("{base}/prepare-upload")).json(&Prepare{info:s.info.clone(),files}).timeout(Duration::from_secs(130)).send()=>r? };
    let response = response
        .error_for_status()
        .context("Recipient did not accept the transfer")?;
    if response.status() == StatusCode::NO_CONTENT {
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
            let stream = tokio_util::io::ReaderStream::new(file.take(size)).inspect(move |chunk| {
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
    fn config(root: &std::path::Path, name: &str) -> Config {
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = socket.local_addr().unwrap().port();
        drop(socket);
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
        }
    }
    async fn next_transfer(rx: &mut mpsc::Receiver<BackendEvent>, state: &str) -> Transfer {
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
        atx.send(BackendCommand::Send {
            transfer_id: "send-one".into(),
            peer_id: format!("localsend:{bf}"),
            files: vec![source.clone(), empty, duplicate],
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
            files: vec![source],
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
                    files: vec![source],
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
