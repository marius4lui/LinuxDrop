//! Quick Share (formerly Nearby Share) adapter. Protocol types never cross IPC.
use anyhow::{Context, Result, bail};
use linuxdrop_core::{BackendCommand, BackendEvent, BackendState, Peer, Transfer, TransferFile};
use rqs_lib::channel::{ChannelMessage, Message, TransferAction, TransferKind};
use rqs_lib::hdl::info::TransferPayload;
use rqs_lib::{EndpointInfo, OutboundPayload, RQS, SendInfo, TransferState, Visibility};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::sync::{broadcast, mpsc};

pub struct Config {
    pub name: String,
    pub download_dir: PathBuf,
    pub visible: bool,
    pub port: Option<u16>,
    pub ble: bool,
    pub max_receive_bytes: u64,
    pub max_files: usize,
    pub upgrade_interface: Option<String>,
}

/// Starts LAN discovery, UKEY2 encrypted send/receive and BlueZ discovery.
/// Radio-changing upgrades require an explicit idle adapter in the upstream
/// hardware policy; ordinary LAN transfers never modify NetworkManager state.
pub async fn start(
    config: Config,
    events: mpsc::Sender<BackendEvent>,
) -> Result<mpsc::Sender<BackendCommand>> {
    std::fs::create_dir_all(&config.download_dir)?;
    let staging = tempfile::Builder::new()
        .prefix(".linuxdrop-quickshare-")
        .tempdir_in(&config.download_dir)?;
    let mut engine = RQS::new(
        if config.visible {
            Visibility::Visible
        } else {
            Visibility::Invisible
        },
        config.port.map(u32::from),
        Some(staging.path().to_owned()),
        Some(config.name),
    );
    let bluetooth = if config.ble {
        match tokio::time::timeout(Duration::from_secs(5), bluetooth_ready()).await {
            Ok(result) => result,
            Err(_) => Err(anyhow::anyhow!("Bluetooth service timed out")),
        }
    } else {
        Ok(())
    };
    engine.ble_enabled = config.ble && bluetooth.is_ok();
    let readiness = match &bluetooth {
        Ok(()) if config.ble => "Quick Share LAN active; Bluetooth discovery enabled.".to_owned(),
        Ok(()) => "Quick Share LAN active; Bluetooth discovery disabled in settings.".to_owned(),
        Err(error) => format!(
            "Quick Share LAN active. Bluetooth unavailable: {error}. Enable a BlueZ controller and restart LinuxDrop to use Bluetooth discovery."
        ),
    };
    rqs_lib::set_receive_limits(config.max_receive_bytes, config.max_files);
    rqs_lib::hdl::set_upgrade_interface(config.upgrade_interface);
    let mut messages = engine.message_sender.subscribe();
    let (send, _) = match engine.run().await {
        Ok(channels) => channels,
        Err(error) => {
            let _ = tokio::time::timeout(Duration::from_secs(10), engine.stop()).await;
            return Err(error);
        }
    };
    let (discovery, mut peers_rx) = broadcast::channel::<EndpointInfo>(128);
    if let Err(error) = engine.discovery(discovery) {
        let _ = tokio::time::timeout(Duration::from_secs(10), engine.stop()).await;
        return Err(error);
    }
    let (commands, mut rx) = mpsc::channel(32);
    events
        .send(BackendEvent::StateChanged(BackendState {
            id: "quickshare".into(),
            state: "ready".into(),
            detail: readiness,
        }))
        .await
        .ok();
    tokio::spawn(async move {
        let mut peers = HashMap::<String, EndpointInfo>::new();
        let mut transfers = HashMap::<String, Transfer>::new();
        let mut destinations = HashMap::<String, PathBuf>::new();
        let mut pending = HashMap::<String, std::time::Instant>::new();
        let mut expiry = tokio::time::interval(Duration::from_secs(5));
        let mut visible = config.visible;
        loop {
            tokio::select! {
                command = rx.recv() => match command {
                    None | Some(BackendCommand::Shutdown) => break,
                    Some(BackendCommand::SetVisibility {visible:value}) => {visible=value;engine.change_visibility(if visible {Visibility::Visible} else {Visibility::Invisible});},
                    Some(BackendCommand::Send {transfer_id, peer_id, files}) => {
                        let result = prepare_send(&transfer_id, &peer_id, &files, &peers);
                        match result {
                            Ok((info, transfer)) => {
                                transfers.insert(transfer_id.clone(), transfer.clone());
                                events.send(BackendEvent::TransferUpdated(transfer)).await.ok();
                                if send.send(info).await.is_err() { break; }
                            }
                            Err(error) => {
                                let mut transfer = empty_transfer(&transfer_id, &peer_id, "Quick Share", "outgoing");
                                transfer.state = "failed".into(); transfer.error = Some(error.to_string());
                                events.send(BackendEvent::TransferUpdated(transfer)).await.ok();
                            }
                        }
                    }
                    Some(BackendCommand::Accept {transfer_id}) => {
                        // Consent is valid once, while the authenticated-session SAS is displayed.
                        if pending.remove(&transfer_id).is_some() { action(&engine, &transfer_id, TransferAction::ConsentAccept); }
                    }
                    Some(BackendCommand::Reject {transfer_id}) => {
                        if pending.remove(&transfer_id).is_some() { action(&engine, &transfer_id, TransferAction::ConsentDecline); }
                    }
                    Some(BackendCommand::Cancel {transfer_id}) => { pending.remove(&transfer_id); action(&engine, &transfer_id, TransferAction::TransferCancel); }
                },
                peer = peers_rx.recv() => match peer {
                    Ok(endpoint) => {
                        let id = format!("quickshare:{}", endpoint.id);
                        if endpoint.present != Some(true) {
                            peers.remove(&id); events.send(BackendEvent::PeerRemoved {peer_id:id}).await.ok();
                        } else {
                            let peer = Peer {id:id.clone(), name:endpoint.name.clone().unwrap_or_else(|| "Android device".into()), platform:format!("{:?}", endpoint.rtype), protocols:vec!["quickshare".into()], address:endpoint.ip.clone().unwrap_or_else(|| "bluetooth".into()), available:true};
                            peers.insert(id, endpoint); events.send(BackendEvent::PeerUpsert(peer)).await.ok();
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {},
                    Err(_) => break,
                },
                message = messages.recv() => match message {
                    Ok(ChannelMessage {msg:Message::Backend {component,detail},..}) => {
                        let bluetooth=component.starts_with("bluetooth-");
                        events.send(BackendEvent::StateChanged(BackendState{id:"quickshare".into(),state:if bluetooth {"ready"} else {"error"}.into(),detail:if bluetooth {format!("Quick Share LAN active; {component} unavailable: {detail}. Check BlueZ and restart the backend from Settings.")} else {format!("Quick Share {component} stopped: {detail}. Restart the backend from Settings.")}})).await.ok();
                        if !bluetooth {break;}
                    }
                    Ok(ChannelMessage {id, msg:Message::Client(message)}) => {
                        let incoming = message.kind == TransferKind::Inbound;
                        let transfer = transfers.entry(id.clone()).or_insert_with(|| empty_transfer(&id, &format!("quickshare:{id}"), "Nearby device", if incoming {"incoming"} else {"outgoing"}));
                        if matches!(transfer.state.as_str(), "completed"|"failed"|"rejected"|"cancelled") { continue; }
                        if let Some(metadata) = message.metadata {
                            if let Some(source) = metadata.source { transfer.peer_name = source.name; }
                            transfer.verification_code = metadata.pin_code;
                            if metadata.total_bytes>0 || transfer.total_bytes==0 {transfer.total_bytes = metadata.total_bytes;}
                            transfer.transferred_bytes = transfer.transferred_bytes.max(metadata.ack_bytes);
                            if let Some(destination) = metadata.destination { destinations.insert(id.clone(), PathBuf::from(destination)); }
                            if let Some(TransferPayload::Files(names)) = metadata.payload && transfer.files.is_empty() {
                                transfer.files = names.into_iter().map(|name| TransferFile {name, size:0, transferred:0}).collect();
                            }
                        }
                        let mut is_request = false;
                        if let Some(state) = message.state {
                            match state {
                                TransferState::WaitingForUserConsent => {
                                    if incoming && !visible {action(&engine,&id,TransferAction::ConsentDecline);continue;}
                                    // Reject non-file offers until the product has an explicit text UI.
                                    if transfer.files.is_empty() || transfer.verification_code.is_none() { action(&engine, &id, TransferAction::ConsentDecline); continue; }
                                    transfer.state = "verification".into();
                                    if !pending.contains_key(&id) { pending.insert(id.clone(), std::time::Instant::now()); is_request = true; }
                                }
                                TransferState::ReceivingFiles | TransferState::SendingFiles => transfer.state = "transferring".into(),
                                TransferState::Rejected => transfer.state = "rejected".into(),
                                TransferState::Cancelled => transfer.state = "cancelled".into(),
                                TransferState::Disconnected => {transfer.state = "failed".into(); transfer.error = Some("The Quick Share connection ended before completion.".into());},
                                TransferState::Finished => {
                                    let result = if incoming { match destinations.get(&id) {Some(dir)=>publish_received(dir, staging.path(), &config.download_dir).await,None=>Err(anyhow::anyhow!("Missing receive staging directory"))} } else {Ok(Vec::new())};
                                    match result {Ok(paths) => {transfer.saved_paths=paths; transfer.state="completed".into(); transfer.transferred_bytes=transfer.total_bytes;}, Err(error) => {transfer.state="failed".into(); transfer.error=Some(error.to_string());}}
                                }
                                _ => {},
                            }
                        }
                        if matches!(transfer.state.as_str(), "completed"|"failed"|"rejected"|"cancelled") {
                            pending.remove(&id);
                            if let Some(dir)=destinations.remove(&id) && dir.starts_with(staging.path()) && dir != staging.path() {let _=std::fs::remove_dir_all(dir);}
                        }
                        events.send(if is_request {BackendEvent::Incoming(transfer.clone())} else {BackendEvent::TransferUpdated(transfer.clone())}).await.ok();
                    }
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {},
                    Err(_) => break,
                },
                _ = expiry.tick() => {let expired:Vec<_>=pending.iter().filter(|(_,t)|t.elapsed()>Duration::from_secs(120)).map(|(id,_)|id.clone()).collect(); for id in expired {pending.remove(&id);action(&engine,&id,TransferAction::ConsentDecline);}}
            }
        }
        let _ = tokio::time::timeout(Duration::from_secs(10), engine.stop()).await;
        for (_, mut transfer) in transfers {
            if !matches!(
                transfer.state.as_str(),
                "completed" | "failed" | "rejected" | "cancelled"
            ) {
                transfer.state = "failed".into();
                transfer.error = Some("Quick Share stopped before the transfer completed.".into());
                events
                    .send(BackendEvent::TransferUpdated(transfer))
                    .await
                    .ok();
            }
        }
        drop(staging);
    });
    Ok(commands)
}

async fn bluetooth_ready() -> Result<()> {
    let session = bluer::Session::new().await?;
    let adapter = session.default_adapter().await?;
    if !adapter.is_powered().await? {
        bail!("Bluetooth is switched off");
    }
    Ok(())
}

fn action(engine: &RQS, id: &str, action: TransferAction) {
    let _ = engine.message_sender.send(ChannelMessage {
        id: id.into(),
        msg: Message::Lib { action },
    });
}
fn empty_transfer(id: &str, peer_id: &str, name: &str, direction: &str) -> Transfer {
    Transfer {
        id: id.into(),
        peer_id: peer_id.into(),
        peer_name: name.into(),
        protocol: "quickshare".into(),
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
fn prepare_send(
    id: &str,
    peer_id: &str,
    files: &[PathBuf],
    peers: &HashMap<String, EndpointInfo>,
) -> Result<(SendInfo, Transfer)> {
    let peer = peers
        .get(peer_id)
        .context("This device is no longer nearby")?;
    if files.is_empty() || files.len() > 1000 {
        bail!("Choose between 1 and 1000 files");
    }
    let mut transfer = empty_transfer(
        id,
        peer_id,
        peer.name.as_deref().unwrap_or("Android device"),
        "outgoing",
    );
    let mut paths = Vec::new();
    for path in files {
        let meta = std::fs::metadata(path)?;
        if !meta.is_file() {
            bail!("Only regular files can be shared");
        }
        transfer.total_bytes = transfer
            .total_bytes
            .checked_add(meta.len())
            .context("File size overflow")?;
        transfer.files.push(TransferFile {
            name: path
                .file_name()
                .context("Missing filename")?
                .to_string_lossy()
                .into(),
            size: meta.len(),
            transferred: 0,
        });
        paths.push(
            path.to_str()
                .context("Quick Share requires UTF-8 paths")?
                .to_owned(),
        );
    }
    Ok((
        SendInfo {
            id: id.into(),
            name: transfer.peer_name.clone(),
            addr: format!(
                "{}:{}",
                peer.ip.as_deref().unwrap_or("0.0.0.0"),
                peer.port.as_deref().unwrap_or("0")
            ),
            ob: OutboundPayload::Files(paths),
            ble: peer.ble_addr.is_some(),
        },
        transfer,
    ))
}
async fn publish_received(dir: &Path, root: &Path, destination: &Path) -> Result<Vec<String>> {
    if !dir.starts_with(root) || dir == root {
        bail!("Invalid staging directory");
    }
    let mut paths = vec![];
    let store = linuxdrop_storage::ReceiveStore::open(destination)?;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            bail!("Unexpected receive entry");
        }
        let filename = entry.file_name();
        let base = filename.to_str().context("Invalid filename")?;
        let mut pending = store.create(base)?;
        let mut source = tokio::fs::File::open(entry.path()).await?;
        tokio::io::copy(&mut source, &mut pending.file).await?;
        paths.push(pending.commit().await?.to_string_lossy().into());
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rqs_lib::hdl::{InboundRequest, OutboundRequest};
    use rqs_lib::utils::RemoteDeviceInfo;
    #[tokio::test]
    async fn ukey2_loopback_requires_both_consents_and_transfers_exact_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let receive = dir.path().join("received");
        std::fs::create_dir(&receive).unwrap();
        let bytes: Vec<u8> = (0..700_123).map(|n| (n % 251) as u8).collect();
        std::fs::write(&source, &bytes).unwrap();
        let empty = dir.path().join("empty");
        std::fs::write(&empty, []).unwrap();
        let _engine = RQS::new(
            Visibility::Visible,
            None,
            Some(receive.clone()),
            Some("LinuxDrop protocol test".into()),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (messages, mut events) = broadcast::channel(1024);
        let rx_messages = messages.clone();
        let incoming = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut protocol = InboundRequest::new(socket, "incoming-test".into(), rx_messages);
            loop {
                if let Err(error) = protocol.handle().await {
                    if protocol.state.state != TransferState::Finished {
                        panic!("inbound: {error:#}");
                    }
                    break;
                }
            }
        });
        // Fragment the first length prefix while unrelated UI events arrive.
        // Cancellation of a read_exact future used to lose prefix bytes.
        let proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_address = proxy.local_addr().unwrap();
        let chatter = messages.clone();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (client, _) = proxy.accept().await.unwrap();
            let server = tokio::net::TcpStream::connect(address).await.unwrap();
            let (mut cr, mut cw) = client.into_split();
            let (mut sr, mut sw) = server.into_split();
            let reverse = tokio::spawn(async move {
                tokio::io::copy(&mut sr, &mut cw).await.ok();
            });
            for _ in 0..4 {
                let mut byte = [0];
                cr.read_exact(&mut byte).await.unwrap();
                sw.write_all(&byte).await.unwrap();
                tokio::time::sleep(Duration::from_millis(5)).await;
                let _ = chatter.send(ChannelMessage {
                    id: "unrelated-ui".into(),
                    msg: Message::Lib {
                        action: TransferAction::ConsentDecline,
                    },
                });
            }
            tokio::io::copy(&mut cr, &mut sw).await.ok();
            reverse.abort();
        });
        let socket = tokio::net::TcpStream::connect(proxy_address).await.unwrap();
        let tx_messages = messages.clone();
        let source_string = source.to_str().unwrap().to_owned();
        let outgoing = tokio::spawn(async move {
            let mut protocol = OutboundRequest::new(
                *b"TEST",
                socket,
                "outgoing-test".into(),
                tx_messages,
                OutboundPayload::Files(vec![source_string, empty.to_str().unwrap().to_owned()]),
                RemoteDeviceInfo {
                    name: "Receiver".into(),
                    device_type: rqs_lib::DeviceType::Laptop,
                },
            );
            protocol.send_connection_request().await.unwrap();
            protocol.send_ukey2_client_init().await.unwrap();
            loop {
                if let Err(error) = protocol.handle().await {
                    if protocol.state.state != TransferState::Finished {
                        panic!("outbound: {error:#}");
                    }
                    break;
                }
            }
        });
        let mut codes = HashMap::new();
        let mut destination = None;
        let mut completed = 0;
        tokio::time::timeout(Duration::from_secs(20), async {
            while completed < 2 {
                let message = events.recv().await.unwrap();
                let Message::Client(event) = message.msg else {
                    continue;
                };
                if event.state == Some(TransferState::WaitingForUserConsent) {
                    let metadata = event.metadata.unwrap();
                    codes.insert(message.id.clone(), metadata.pin_code.unwrap());
                    if event.kind == TransferKind::Inbound {
                        destination = metadata.destination;
                        assert_eq!(
                            std::fs::read_dir(&receive).unwrap().count(),
                            0,
                            "files cannot be opened before consent"
                        );
                    }
                    messages
                        .send(ChannelMessage {
                            id: message.id,
                            msg: Message::Lib {
                                action: TransferAction::ConsentAccept,
                            },
                        })
                        .unwrap();
                }
                if event.state == Some(TransferState::Finished) {
                    completed += 1;
                }
            }
        })
        .await
        .unwrap();
        incoming.await.unwrap();
        outgoing.await.unwrap();
        assert_eq!(codes.len(), 2);
        assert_eq!(codes["incoming-test"], codes["outgoing-test"]);
        let destination = destination.unwrap();
        assert_eq!(
            std::fs::metadata(Path::new(&destination).join("empty"))
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            std::fs::read(Path::new(&destination).join("source")).unwrap(),
            bytes
        );
    }
}
