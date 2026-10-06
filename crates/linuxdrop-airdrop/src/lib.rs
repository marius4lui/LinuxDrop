//! AirDrop application transport, isolated from the privileged AWDL radio link.
//! Everyone mode is unauthenticated; consent is mandatory and contacts are not
//! represented as verified. The AWDL interface must be leased by linuxdrop-netd.
mod archive;
mod transport;
#[cfg(feature = "fuzzing")]
pub mod fuzzing {
    use std::io::{Seek, Write};
    pub fn archive_input(data: &[u8], compressed: bool) -> anyhow::Result<()> {
        anyhow::ensure!(data.len() <= 65536, "Input too large");
        let mut file = tempfile::tempfile()?;
        file.write_all(data)?;
        file.rewind()?;
        let mut decoded = if compressed {
            super::archive::inflate(file, "application/x-dvzip", 65536, None)?
        } else {
            file
        };
        let offer = luftlift_rs::plist_impl::AskFile {
            file_name: "file".into(),
            file_type: "public.data".into(),
            file_size: 0,
            file_bom_path: "./file".into(),
        };
        super::archive::index(&mut decoded, &[offer])?;
        Ok(())
    }
}
use anyhow::{Context, Result, bail};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Request, Response, StatusCode, body::Incoming};
use hyper_util::rt::TokioIo;
use linuxdrop_core::{BackendCommand, BackendEvent, BackendState, Peer, Transfer, TransferFile};
use linuxdrop_storage::{ReceiveStore, validate_name};
use luftlift_rs::plist_impl::{AskRequest, ReceiverConfig};
use std::{
    collections::HashMap,
    convert::Infallible,
    net::{IpAddr, SocketAddr, SocketAddrV6},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::{Mutex, Semaphore, mpsc, oneshot},
};
use tokio_util::sync::CancellationToken;

pub struct Config {
    pub name: String,
    pub download_dir: PathBuf,
    pub interface: String,
    pub visible: bool,
    pub ble_wake: bool,
    pub max_receive_bytes: u64,
    pub max_files: usize,
    pub policy: linuxdrop_core::TransferPolicy,
}
struct Offer {
    request: AskRequest,
    transfer: Transfer,
    expires: Instant,
    cancel: CancellationToken,
    options: linuxdrop_core::ReceiveOptions,
    store: ReceiveStore,
}
struct Shared {
    bandwidth: linuxdrop_network::BandwidthLimiter,
    name: String,
    events: mpsc::Sender<BackendEvent>,
    store: ReceiveStore,
    visible: AtomicBool,
    max: u64,
    max_files: usize,
    asking: std::sync::Mutex<std::collections::HashSet<IpAddr>>,
    pending: Mutex<HashMap<String, oneshot::Sender<Option<linuxdrop_core::ReceiveOptions>>>>,
    offers: Mutex<HashMap<IpAddr, Offer>>,
    jobs: Mutex<HashMap<String, CancellationToken>>,
}

struct AskGuard<'a> {
    shared: &'a Shared,
    remote: IpAddr,
}
impl Drop for AskGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut asking) = self.shared.asking.lock() {
            asking.remove(&self.remote);
        }
    }
}
fn reserve_ask(shared: &Shared, remote: IpAddr) -> Result<AskGuard<'_>> {
    let mut asking = shared
        .asking
        .lock()
        .map_err(|_| anyhow::anyhow!("Consent state unavailable"))?;
    if asking.len() >= 8 || !asking.insert(remote) {
        bail!("Another request from this sender is awaiting consent");
    }
    Ok(AskGuard { shared, remote })
}

pub async fn start(
    config: Config,
    events: mpsc::Sender<BackendEvent>,
) -> Result<mpsc::Sender<BackendCommand>> {
    let budget = linuxdrop_network::BandwidthLimiter::new(config.policy.bandwidth_bytes_per_second);
    start_with_budget(config, events, budget).await
}
pub async fn start_with_budget(
    config: Config,
    events: mpsc::Sender<BackendEvent>,
    bandwidth: linuxdrop_network::BandwidthLimiter,
) -> Result<mpsc::Sender<BackendCommand>> {
    let address = luftlift_rs::netutil::get_ipv6_for_interface(&config.interface)
        .context("AirDrop needs an active AWDL interface from the hardware helper")?;
    let index = luftlift_rs::netutil::if_index_for(&config.interface)
        .context("AWDL interface disappeared")?;
    let listener =
        tokio::net::TcpListener::bind(SocketAddrV6::new(address, 8771, 0, index)).await?;
    let cert = luftlift_rs::tls::generate_self_signed_cert()?;
    let tls =
        tokio_rustls::TlsAcceptor::from(Arc::new(luftlift_rs::tls::build_server_config(&cert)?));
    let shared = Arc::new(Shared {
        bandwidth,
        name: config.name.clone(),
        events: events.clone(),
        store: ReceiveStore::open(&config.download_dir)?,
        visible: AtomicBool::new(config.visible),
        max: config.max_receive_bytes.min(1024 * 1024 * 1024 * 1024),
        max_files: config.max_files.min(1000),
        asking: Default::default(),
        pending: Mutex::new(HashMap::new()),
        offers: Mutex::new(HashMap::new()),
        jobs: Mutex::new(HashMap::new()),
    });
    let mdns = mdns_sd::ServiceDaemon::new()?;
    mdns.disable_interface(mdns_sd::IfKind::All)?;
    mdns.enable_interface(mdns_sd::IfKind::Name(config.interface.clone()))?;
    let service = luftlift_rs::mdns::build_airdrop_service_info(
        &luftlift_rs::mdns::MdnsConfig {
            computer_name: format!("linuxdrop-{}", uuid::Uuid::new_v4().simple()),
            port: 8771,
            flags: 0x88,
        },
        IpAddr::V6(address),
    )?;
    if config.visible {
        mdns.register(service.clone())?;
    }
    let discovery = mdns.browse("_airdrop._tcp.local.")?;
    let mdns_health = mdns.monitor()?;
    let (tx, mut commands) = mpsc::channel(32);
    let stop = CancellationToken::new();
    let server_stop = stop.clone();
    let server_shared = shared.clone();
    tokio::spawn(async move {
        let slots = Arc::new(Semaphore::new(8));
        loop {
            tokio::select! {
                _=server_stop.cancelled()=>break,
                accepted=listener.accept()=>{
                let (socket,remote)=match accepted {Ok(pair)=>pair,Err(error)=>{server_shared.events.send(BackendEvent::StateChanged(BackendState{id:"airdrop".into(),state:"error".into(),detail:format!("AirDrop listener stopped: {error}")})).await.ok();server_stop.cancel();break;}};
                let Ok(slot)=slots.clone().try_acquire_owned() else {continue};
                    let acceptor=tls.clone();let shared=server_shared.clone();let stop=server_stop.clone();
                    tokio::spawn(async move {let _slot=slot;
                        let connection=async {let tls=tokio::time::timeout(Duration::from_secs(10),acceptor.accept(socket)).await??;
                            hyper::server::conn::http1::Builder::new().max_buf_size(32*1024).serve_connection(TokioIo::new(tls),hyper::service::service_fn(move |request|{let shared=shared.clone();async move {Ok::<_,Infallible>(match route(shared,remote.ip(),request).await {Ok(r)=>r,Err(_)=>response(StatusCode::BAD_REQUEST,b"Transfer request failed".to_vec())})}})).await?;Ok::<_,anyhow::Error>(())};
                        tokio::select! {_=stop.cancelled()=>{},_=tokio::time::timeout(Duration::from_secs(1800),connection)=>{}}
                    });
                }
            }
        }
    });
    events.send(BackendEvent::StateChanged(BackendState{id:"airdrop".into(),state:"ready".into(),detail:"Experimental AirDrop on leased AWDL hardware. Everyone mode; Apple device verification pending.".into()})).await.ok();
    tokio::spawn(async move {
        let mut peers = HashMap::<String, (SocketAddr, Instant)>::new();
        let mut tick = tokio::time::interval(Duration::from_secs(15));
        let mut advertisement = None;
        if config.ble_wake {
            match transport::ble_wake(config.policy.bluetooth_adapter.as_deref()).await {
                Ok(handle) => advertisement = Some(handle),
                Err(error) => {
                    events.send(BackendEvent::StateChanged(BackendState{id:"airdrop".into(),state:"ready".into(),detail:format!("AirDrop AWDL receive is ready. Bluetooth wake unavailable: {error}. Enable a BlueZ controller or open the Apple device's AirDrop panel manually.")})).await.ok();
                }
            }
        }
        loop {
            tokio::select! {
                _=stop.cancelled()=>break,
                event=mdns_health.recv_async()=>match event {
                    Ok(mdns_sd::DaemonEvent::Error(error))=>{events.send(BackendEvent::StateChanged(BackendState{id:"airdrop".into(),state:"error".into(),detail:format!("AirDrop discovery error: {error}")})).await.ok();break;},
                    Err(_)=>{events.send(BackendEvent::StateChanged(BackendState{id:"airdrop".into(),state:"error".into(),detail:"AirDrop discovery stopped. Restart the backend.".into()})).await.ok();break;},
                    _=>{},
                },
                command=commands.recv()=>match command {
                    None|Some(BackendCommand::Shutdown)=>break,
                    Some(BackendCommand::SetVisibility{visible})=>{
                        shared.visible.store(visible,Ordering::Relaxed);
                        if visible {let _=mdns.register(service.clone());} else {let _=mdns.unregister(service.get_fullname());}
                    }
                    Some(BackendCommand::Accept{transfer_id})=>{if let Some(reply)=shared.pending.lock().await.remove(&transfer_id){let _=reply.send(Some(Default::default()));}}
                    Some(BackendCommand::AcceptWithOptions{transfer_id,options})=>{if let Some(reply)=shared.pending.lock().await.remove(&transfer_id){let _=reply.send(Some(options));}}
                    Some(BackendCommand::Reject{transfer_id})|Some(BackendCommand::ProvidePin{transfer_id,..})=>{if let Some(reply)=shared.pending.lock().await.remove(&transfer_id){let _=reply.send(None);}}
                    Some(BackendCommand::Cancel{transfer_id})=>{if let Some(cancel)=shared.jobs.lock().await.get(&transfer_id){cancel.cancel();}
                    if let Some(reply)=shared.pending.lock().await.remove(&transfer_id){let _=reply.send(None);}}
                    Some(BackendCommand::Send{transfer_id,peer_id,files})=>{
                        let peer=peers.get(&peer_id).map(|(address,_)|*address);
                        let cancel=stop.child_token();shared.jobs.lock().await.insert(transfer_id.clone(),cancel.clone());
                        let shared=shared.clone();let name=config.name.clone();
                        tokio::spawn(async move {
                            let mut transfer=empty_transfer(&transfer_id,&peer_id,"Apple device","outgoing");
                            let result=async {let address=peer.context("This AirDrop device is no longer nearby")?;transport::send(address,&name,files,&mut transfer,&shared.events,cancel.clone(),shared.bandwidth.clone()).await}.await;
                            match result {Ok(())=>{transfer.state="completed".into();transfer.transferred_bytes=transfer.total_bytes;},Err(error)=>{transfer.state=if cancel.is_cancelled(){"cancelled"}else{"failed"}.into();transfer.error=Some(error.to_string());}}
                            shared.events.send(BackendEvent::TransferUpdated(transfer)).await.ok();shared.jobs.lock().await.remove(&transfer_id);
                        });
                    }
                },
                event=discovery.recv_async()=>{if let Ok(mdns_sd::ServiceEvent::ServiceResolved(info))=event {
                    if info.get_fullname()==service.get_fullname(){continue;}
                    if let Some(IpAddr::V6(ip))=info.get_addresses().iter().find(|ip|ip.is_ipv6()) {
                        let id=format!("airdrop:{}",info.get_fullname());let addr=SocketAddr::V6(SocketAddrV6::new(*ip,info.get_port(),0,index));
                        peers.insert(id.clone(),(addr,Instant::now()));events.send(BackendEvent::PeerUpsert(Peer{id,name:info.get_fullname().trim_end_matches("._airdrop._tcp.local.").into(),platform:"Apple".into(),protocols:vec!["airdrop".into()],address:addr.to_string(),available:true})).await.ok();
                    }
                }},
                _=tick.tick()=>{
                    if shared.visible.load(Ordering::Relaxed){let _=mdns.register(service.clone());}
                    let expired:Vec<_>=peers.iter().filter(|(_,(_,seen))|seen.elapsed()>Duration::from_secs(120)).map(|(id,_)|id.clone()).collect();for id in expired{peers.remove(&id);events.send(BackendEvent::PeerRemoved{peer_id:id}).await.ok();}
                let mut offers=shared.offers.lock().await;
                let expired:Vec<_>=offers.iter().filter(|(_,offer)|offer.expires<=Instant::now()||offer.cancel.is_cancelled()).map(|(remote,_)|*remote).collect();
                for remote in expired {if let Some(mut offer)=offers.remove(&remote) {shared.jobs.lock().await.remove(&offer.transfer.id);offer.transfer.state=if offer.cancel.is_cancelled(){"cancelled"}else{"failed"}.into();offer.transfer.error=Some("The sender did not start the accepted upload.".into());events.send(BackendEvent::TransferUpdated(offer.transfer)).await.ok();}}
                }
            }
        }
        stop.cancel();
        for cancel in shared.jobs.lock().await.values() {
            cancel.cancel();
        }
        shared.pending.lock().await.clear();
        for (_, mut offer) in shared.offers.lock().await.drain() {
            offer.transfer.state = "failed".into();
            offer.transfer.error = Some("AirDrop stopped before the accepted upload began.".into());
            events
                .send(BackendEvent::TransferUpdated(offer.transfer))
                .await
                .ok();
        }
        let _ = mdns.shutdown();
        drop(advertisement);
    });
    Ok(tx)
}

fn empty_transfer(id: &str, peer_id: &str, name: &str, direction: &str) -> Transfer {
    Transfer {
        id: id.into(),
        peer_id: peer_id.into(),
        peer_name: name.into(),
        protocol: "airdrop".into(),
        direction: direction.into(),
        state: "connecting".into(),
        files: vec![],
        total_bytes: 0,
        transferred_bytes: 0,
        error: None,
        verification_code: None,
        saved_paths: vec![],
    }
}

/// HTTP connection cancellation drops the route future. Preserve a terminal
/// event even if the sender disconnects while the user is considering consent.
struct TransferGuard {
    transfer: Option<Transfer>,
    shared: Arc<Shared>,
}
impl TransferGuard {
    fn new(transfer: &Transfer, shared: &Arc<Shared>) -> Self {
        Self {
            transfer: Some(transfer.clone()),
            shared: shared.clone(),
        }
    }
    fn finish(&mut self) {
        self.transfer = None;
    }
}
impl Drop for TransferGuard {
    fn drop(&mut self) {
        if let Some(mut transfer) = self.transfer.take() {
            transfer.state = "failed".into();
            transfer.error =
                Some("The AirDrop connection ended before the transfer completed.".into());
            let shared = self.shared.clone();
            tokio::spawn(async move {
                shared.pending.lock().await.remove(&transfer.id);
                shared.jobs.lock().await.remove(&transfer.id);
                shared
                    .events
                    .send(BackendEvent::TransferUpdated(transfer))
                    .await
                    .ok();
            });
        }
    }
}
fn response(status: StatusCode, body: Vec<u8>) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header("content-type", "application/x-apple-binary-plist")
        .body(Full::new(Bytes::from(body)))
        .expect("constant response headers")
}
async fn small_body(mut body: Incoming) -> Result<Vec<u8>> {
    let mut data = vec![];
    while let Some(frame) = tokio::time::timeout(Duration::from_secs(15), body.frame()).await? {
        if let Ok(bytes) = frame?.into_data() {
            if data.len() + bytes.len() > 1024 * 1024 {
                bail!("Metadata too large");
            }
            data.extend_from_slice(&bytes);
        }
    }
    Ok(data)
}

async fn route(
    shared: Arc<Shared>,
    remote: IpAddr,
    request: Request<Incoming>,
) -> Result<Response<Full<Bytes>>> {
    if request.method() != hyper::Method::POST {
        return Ok(response(StatusCode::METHOD_NOT_ALLOWED, vec![]));
    }
    let receiver = ReceiverConfig {
        computer_name: shared.name.clone(),
        computer_model: "Linux".into(),
        record_data: None,
    };
    match request.uri().path() {
        "/Discover" => {
            if !shared.visible.load(Ordering::Relaxed) {
                return Ok(response(StatusCode::FORBIDDEN, vec![]));
            }
            let _ = small_body(request.into_body()).await?;
            Ok(response(
                StatusCode::OK,
                luftlift_rs::plist_impl::build_discover_response(&receiver),
            ))
        }
        "/Ask" => {
            let _ask_guard = reserve_ask(&shared, remote)?;
            if !shared.visible.load(Ordering::Relaxed) {
                return Ok(response(StatusCode::FORBIDDEN, vec![]));
            }
            let ask = luftlift_rs::plist_impl::parse_ask_request(
                &small_body(request.into_body()).await?,
            )?;
            if ask.files.is_empty() || ask.files.len() > shared.max_files {
                bail!("Invalid file count");
            }
            let mut total = 0u64;
            let mut names = std::collections::HashSet::new();
            for file in &ask.files {
                validate_name(&file.file_name)?;
                if !names.insert(file.file_name.clone()) {
                    bail!("Duplicate name");
                }
                total = total.checked_add(file.file_size).context("Size overflow")?;
            }
            if total > shared.max {
                bail!("Receive limit exceeded");
            }
            if shared.offers.lock().await.contains_key(&remote) {
                return Ok(response(StatusCode::CONFLICT, vec![]));
            }
            let id = uuid::Uuid::new_v4().to_string();
            let mut transfer = empty_transfer(
                &id,
                &format!("airdrop:{remote}"),
                &ask.sender_computer_name,
                "incoming",
            );
            transfer.state = "waiting".into();
            transfer.total_bytes = total;
            transfer.files = ask
                .files
                .iter()
                .map(|f| TransferFile {
                    name: f.file_name.clone(),
                    size: f.file_size,
                    transferred: 0,
                })
                .collect();
            let (tx, rx) = oneshot::channel();
            let mut guard = TransferGuard::new(&transfer, &shared);
            shared.pending.lock().await.insert(id.clone(), tx);
            shared
                .events
                .send(BackendEvent::Incoming(transfer.clone()))
                .await?;
            let options = tokio::time::timeout(Duration::from_secs(120), rx)
                .await
                .ok()
                .and_then(|r| r.ok())
                .flatten();
            shared.pending.lock().await.remove(&id);
            if options.is_none() || !shared.visible.load(Ordering::Relaxed) {
                transfer.state = "rejected".into();
                shared
                    .events
                    .send(BackendEvent::TransferUpdated(transfer))
                    .await
                    .ok();
                guard.finish();
                return Ok(response(StatusCode::FORBIDDEN, vec![]));
            }
            let options = options.unwrap();
            if options.selected_indices.as_ref().is_some_and(|indices| {
                indices.is_empty()
                    || indices.iter().any(|index| *index >= ask.files.len())
                    || indices
                        .iter()
                        .collect::<std::collections::HashSet<_>>()
                        .len()
                        != indices.len()
            }) {
                bail!("Invalid file selection");
            }
            let store = match &options.directory {
                Some(path) => ReceiveStore::open(path)?,
                None => shared.store.clone(),
            };
            let cancel = CancellationToken::new();
            shared.jobs.lock().await.insert(id, cancel.clone());
            shared.offers.lock().await.insert(
                remote,
                Offer {
                    request: ask,
                    transfer,
                    expires: Instant::now() + Duration::from_secs(120),
                    cancel,
                    options,
                    store,
                },
            );
            guard.finish();
            Ok(response(
                StatusCode::OK,
                luftlift_rs::plist_impl::build_ask_response(&receiver),
            ))
        }
        "/Upload" => {
            let offer = shared
                .offers
                .lock()
                .await
                .remove(&remote)
                .context("Upload has no accepted request")?;
            let mut guard = TransferGuard::new(&offer.transfer, &shared);
            if offer.expires < Instant::now() || offer.cancel.is_cancelled() {
                bail!("Consent expired");
            }
            let mut transfer = offer.transfer;
            transfer.state = "transferring".into();
            shared
                .events
                .send(BackendEvent::TransferUpdated(transfer.clone()))
                .await
                .ok();
            let result = tokio::select! {_=offer.cancel.cancelled()=>Err(anyhow::anyhow!("Transfer cancelled")),result=receive_upload(shared.clone(),request,&offer.request,&mut transfer,offer.cancel.clone(),&offer.store,&offer.options)=>result};
            shared.jobs.lock().await.remove(&transfer.id);
            let status = match result {
                Ok(paths) => {
                    transfer.saved_paths = paths;
                    transfer.state = "completed".into();
                    transfer.transferred_bytes = transfer.total_bytes;
                    StatusCode::OK
                }
                Err(error) => {
                    transfer.state = if offer.cancel.is_cancelled() {
                        "cancelled"
                    } else {
                        "failed"
                    }
                    .into();
                    transfer.error = Some(error.to_string());
                    StatusCode::BAD_REQUEST
                }
            };
            shared
                .events
                .send(BackendEvent::TransferUpdated(transfer))
                .await
                .ok();
            guard.finish();
            Ok(response(status, vec![]))
        }
        _ => Ok(response(StatusCode::NOT_FOUND, vec![])),
    }
}

async fn receive_upload(
    shared: Arc<Shared>,
    request: Request<Incoming>,
    ask: &AskRequest,
    transfer: &mut Transfer,
    cancel: CancellationToken,
    store: &ReceiveStore,
    options: &linuxdrop_core::ReceiveOptions,
) -> Result<Vec<String>> {
    let content_type = request
        .headers()
        .get("content-type")
        .context("Missing upload type")?
        .to_str()?
        .to_owned();
    let limit = if ask.files.iter().any(|f| f.file_size == 0) {
        shared.max
    } else {
        transfer.total_bytes
    };
    let archive_limit = limit
        .checked_add(ask.files.len() as u64 * 4096 + 65536)
        .context("Size overflow")?;
    let mut spool = tokio::fs::File::from_std(tempfile::tempfile()?);
    let mut body = request.into_body();
    let mut received = 0u64;
    let mut last = Instant::now();
    while let Some(frame) = tokio::time::timeout(Duration::from_secs(30), body.frame()).await? {
        if let Ok(bytes) = frame?.into_data() {
            received += bytes.len() as u64;
            if received > archive_limit + archive_limit / 100 + 65536 {
                bail!("Compressed upload too large");
            }
            shared.bandwidth.acquire(bytes.len(), &cancel).await?;
            spool.write_all(&bytes).await?;
            if last.elapsed() > Duration::from_millis(200) {
                shared
                    .events
                    .send(BackendEvent::TransferUpdated(transfer.clone()))
                    .await
                    .ok();
                last = Instant::now();
            }
        }
    }
    spool.flush().await?;
    let file = spool.into_std().await;
    let expected = ask.files.clone();
    let (decoded, entries) = tokio::task::spawn_blocking(move || -> Result<_> {
        let mut decoded = archive::inflate(file, &content_type, archive_limit, Some(&cancel))?;
        let entries = archive::index(&mut decoded, &expected)?;
        Ok((decoded, entries))
    })
    .await??;
    let mut decoded = tokio::fs::File::from_std(decoded);
    let mut pending = vec![];
    transfer.total_bytes = entries.iter().map(|e| e.size).sum();
    if transfer.total_bytes > shared.max {
        bail!("Receive size limit exceeded");
    }
    for entry in entries {
        if options.selected_indices.as_ref().is_some_and(|indices| {
            !indices
                .iter()
                .any(|index| ask.files[*index].file_name == entry.name)
        }) {
            continue;
        }
        let mut file = store.create(&entry.name)?;
        decoded.seek(std::io::SeekFrom::Start(entry.offset)).await?;
        let copied = tokio::io::copy(&mut (&mut decoded).take(entry.size), &mut file.file).await?;
        if copied != entry.size {
            bail!("Truncated archive");
        }
        transfer.transferred_bytes += copied;
        for meta in &mut transfer.files {
            if meta.name == entry.name {
                meta.size = entry.size;
                meta.transferred = entry.size;
            }
        }
        pending.push(file);
    }
    let mut saved = vec![];
    for file in pending {
        saved.push(
            file.commit_with_policy(options.collision_policy)
                .await?
                .to_string_lossy()
                .into(),
        );
    }
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn tls_send_receive_requires_consent_and_preserves_collisions() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        std::fs::write(source.path().join("photo.bin"), vec![42; 400_123]).unwrap();
        std::fs::write(source.path().join("empty.txt"), []).unwrap();
        std::fs::write(target.path().join("photo.bin"), b"existing").unwrap();
        let wire_size = archive::encode(
            &[
                source.path().join("photo.bin"),
                source.path().join("empty.txt"),
            ],
            None,
        )
        .unwrap()
        .1;
        let (events, mut rx) = mpsc::channel(128);
        let shared = Arc::new(Shared {
            bandwidth: linuxdrop_network::BandwidthLimiter::new(Some(wire_size)),
            name: "Receiver".into(),
            events: events.clone(),
            store: ReceiveStore::open(target.path()).unwrap(),
            visible: AtomicBool::new(true),
            max: 1024 * 1024,
            max_files: 10,
            asking: Default::default(),
            pending: Mutex::new(HashMap::new()),
            offers: Mutex::new(HashMap::new()),
            jobs: Mutex::new(HashMap::new()),
        });
        let cert = luftlift_rs::tls::generate_self_signed_cert().unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(
            luftlift_rs::tls::build_server_config(&cert).unwrap(),
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server_shared = shared.clone();
        let server = tokio::spawn(async move {
            loop {
                let (socket, remote) = listener.accept().await.unwrap();
                let tls = acceptor.accept(socket).await.unwrap();
                let shared = server_shared.clone();
                tokio::spawn(async move {
                    hyper::server::conn::http1::Builder::new()
                        .serve_connection(
                            TokioIo::new(tls),
                            hyper::service::service_fn(move |request| {
                                let shared = shared.clone();
                                async move {
                                    Ok::<_, Infallible>(
                                        match route(shared, remote.ip(), request).await {
                                            Ok(response) => response,
                                            Err(_) => response(StatusCode::BAD_REQUEST, vec![]),
                                        },
                                    )
                                }
                            }),
                        )
                        .await
                        .ok();
                });
            }
        });
        assert!(transport::unsolicited_upload(address).await.is_err());
        assert_eq!(
            std::fs::read_dir(target.path()).unwrap().count(),
            1,
            "unsolicited upload cannot create files"
        );
        let files = vec![
            source.path().join("photo.bin"),
            source.path().join("empty.txt"),
        ];
        let sender = tokio::spawn(async move {
            let mut transfer = empty_transfer("sender", "loopback", "Receiver", "outgoing");
            transport::send(
                address,
                "Sender",
                files,
                &mut transfer,
                &events,
                CancellationToken::new(),
                linuxdrop_network::BandwidthLimiter::new(Some(wire_size)),
            )
            .await
            .unwrap();
        });
        let mut saw_offer = false;
        let mut payload_started = None;
        tokio::time::timeout(Duration::from_secs(15), async {
            while let Some(event) = rx.recv().await {
                match event {
                    BackendEvent::Incoming(transfer) => {
                        saw_offer = true;
                        payload_started = Some(Instant::now());
                        assert_eq!(std::fs::read_dir(target.path()).unwrap().count(), 1);
                        shared
                            .pending
                            .lock()
                            .await
                            .remove(&transfer.id)
                            .unwrap()
                            .send(Some(Default::default()))
                            .unwrap();
                    }
                    BackendEvent::TransferUpdated(transfer)
                        if transfer.direction == "incoming" && transfer.state == "completed" =>
                    {
                        assert_eq!(transfer.saved_paths.len(), 2);
                        break;
                    }
                    _ => {}
                }
            }
        })
        .await
        .unwrap();
        sender.await.unwrap();
        // The compressed archive consumes one second at each endpoint. Charging
        // only uncompressed progress or wiring only one direction would fail.
        assert!(payload_started.unwrap().elapsed() >= Duration::from_millis(1900));
        let cancel = CancellationToken::new();
        let sender_cancel = cancel.clone();
        let sender_events = shared.events.clone();
        let second_files = vec![source.path().join("photo.bin")];
        let interrupted = tokio::spawn(async move {
            let mut transfer = empty_transfer("interrupted", "loopback", "Receiver", "outgoing");
            assert!(
                transport::send(
                    address,
                    "Sender",
                    second_files,
                    &mut transfer,
                    &sender_events,
                    sender_cancel,
                    linuxdrop_network::BandwidthLimiter::new(None),
                )
                .await
                .is_err()
            );
        });
        let interrupted_id = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(BackendEvent::Incoming(t)) = rx.recv().await {
                    break t.id;
                }
            }
        })
        .await
        .unwrap();
        cancel.cancel();
        interrupted.await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(BackendEvent::TransferUpdated(t)) = rx.recv().await
                    && t.id == interrupted_id
                {
                    assert_eq!(t.state, "failed");
                    break;
                }
            }
        })
        .await
        .unwrap();
        assert!(!shared.pending.lock().await.contains_key(&interrupted_id));
        server.abort();
        assert!(saw_offer);
        assert_eq!(
            std::fs::read(target.path().join("photo.bin")).unwrap(),
            b"existing"
        );
        assert_eq!(
            std::fs::read(target.path().join("photo (1).bin")).unwrap(),
            vec![42; 400_123]
        );
        assert_eq!(
            std::fs::metadata(target.path().join("empty.txt"))
                .unwrap()
                .len(),
            0
        );
    }
}
