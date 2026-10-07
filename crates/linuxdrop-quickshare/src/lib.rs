//! Quick Share (formerly Nearby Share) adapter. Protocol types never cross IPC.
mod engine_lifetime;
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
    pub upgrade_lease: Option<DirectWifiLease>,
    pub p2p_connector: Option<std::sync::Arc<dyn linuxdrop_network::P2pConnector>>,
    pub policy: linuxdrop_core::TransferPolicy,
}

pub struct DirectWifiLease {
    pub interface: String,
    pub lease_id: String,
    pub connection_uuid: String,
    pub capabilities: linuxdrop_network::DirectWifiCapabilities,
}

/// Starts LAN discovery, UKEY2 encrypted send/receive and BlueZ discovery.
/// Radio-changing upgrades require an explicit idle adapter in the upstream
/// hardware policy; ordinary LAN transfers never modify NetworkManager state.
pub async fn start(
    config: Config,
    events: mpsc::Sender<BackendEvent>,
) -> Result<linuxdrop_core::CommandSender> {
    let budget = linuxdrop_network::BandwidthLimiter::new(config.policy.bandwidth_bytes_per_second);
    start_with_budget(config, events, budget).await
}
pub async fn start_with_budget(
    config: Config,
    events: mpsc::Sender<BackendEvent>,
    bandwidth: linuxdrop_network::BandwidthLimiter,
) -> Result<linuxdrop_core::CommandSender> {
    std::fs::create_dir_all(&config.download_dir)?;
    let staging = tempfile::Builder::new()
        .prefix(".linuxdrop-quickshare-")
        .tempdir_in(&config.download_dir)?;
    let engine = RQS::new(
        if config.visible {
            Visibility::Visible
        } else {
            Visibility::Invisible
        },
        config.port.map(u32::from),
        Some(staging.path().to_owned()),
        Some(config.name),
    );
    let mut engine = engine_lifetime::OwnedEngine::new(engine, staging);
    let bluetooth = if config.ble {
        match tokio::time::timeout(
            Duration::from_secs(5),
            bluetooth_ready(config.policy.bluetooth_adapter.as_deref()),
        )
        .await
        {
            Ok(result) => result,
            Err(_) => Err(anyhow::anyhow!("Bluetooth service timed out")),
        }
    } else {
        Ok(String::new())
    };
    engine.ble_enabled = config.ble && bluetooth.is_ok();
    rqs_lib::set_bluetooth_adapter(
        bluetooth
            .as_ref()
            .ok()
            .filter(|name| !name.is_empty())
            .cloned(),
    );
    rqs_lib::lan_policy::set(config.policy.clone());
    rqs_lib::set_receive_limits(config.max_receive_bytes, config.max_files);
    rqs_lib::payload_budget::set_budget(bandwidth);
    rqs_lib::hdl::set_upgrade_lease(config.upgrade_lease.map(|lease| {
        (
            linuxdrop_network::nm::Lease {
                interface: lease.interface,
                lease_id: lease.lease_id,
                connection_uuid: lease.connection_uuid,
            },
            lease.capabilities,
        )
    }));
    rqs_lib::hdl::set_p2p_connector(config.p2p_connector);
    let mut messages = engine.message_sender.subscribe();
    let (send, _) = match engine.run().await {
        Ok(channels) => channels,
        Err(error) => {
            return match engine.stop().await {
                Ok(()) => Err(error),
                Err(cleanup) => Err(anyhow::anyhow!("{error}; {cleanup}")),
            };
        }
    };
    let mut lan_state = match engine.lan_state() {
        Ok(state) => state,
        Err(error) => {
            return match engine.stop().await {
                Ok(()) => Err(error),
                Err(cleanup) => Err(anyhow::anyhow!("{error}; {cleanup}")),
            };
        }
    };
    let (discovery, mut peers_rx) = broadcast::channel::<EndpointInfo>(128);
    if let Err(error) = engine.discovery(discovery) {
        return match engine.stop().await {
            Ok(()) => Err(error),
            Err(cleanup) => Err(anyhow::anyhow!("{error}; {cleanup}")),
        };
    }
    let (commands, mut rx) = mpsc::channel(32);
    let mut bluetooth_errors = std::collections::BTreeMap::new();
    let initial_network = lan_state.borrow().clone();
    events
        .send(BackendEvent::StateChanged(network_status(
            &initial_network,
            &bluetooth,
            config.ble,
            &bluetooth_errors,
        )))
        .await
        .ok();
    let task = tokio::spawn(async move {
        let mut peers = HashMap::<String, EndpointInfo>::new();
        let mut transfers = HashMap::<String, Transfer>::new();
        let mut destinations = HashMap::<String, PathBuf>::new();
        let mut pending = HashMap::<String, std::time::Instant>::new();
        let mut receive_options = HashMap::<String, linuxdrop_core::ReceiveOptions>::new();
        let mut expiry = tokio::time::interval(Duration::from_secs(5));
        let mut visible = config.visible;
        loop {
            tokio::select! {
                changed = lan_state.changed() => {
                    if changed.is_err() { break; }
                    let snapshot = lan_state.borrow_and_update().clone();
                    events.send(BackendEvent::StateChanged(network_status(&snapshot, &bluetooth, config.ble, &bluetooth_errors))).await.ok();
                },
                command = rx.recv() => match command {
                    None | Some(BackendCommand::Shutdown) => break,
                    Some(BackendCommand::ReceiveOffer { .. }) => {}, // Routed only to LocalSend by the daemon.
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
                    Some(BackendCommand::AcceptWithOptions {transfer_id,options}) => {
                        if pending.contains_key(&transfer_id) {
                            let valid=transfers.get(&transfer_id).is_some_and(|transfer|transfer.direction=="incoming" && valid_selection(&options,transfer.files.len()));
                            if valid {receive_options.insert(transfer_id.clone(),options);pending.remove(&transfer_id);action(&engine,&transfer_id,TransferAction::ConsentAccept);}
                            else {pending.remove(&transfer_id);action(&engine,&transfer_id,TransferAction::ConsentDecline);}
                        }
                    }
                    Some(BackendCommand::ProvidePin {transfer_id,..}) => {
                        // A Quick Share SAS must be compared, never entered as a receiver PIN.
                        pending.remove(&transfer_id);action(&engine,&transfer_id,TransferAction::ConsentDecline);
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
                        if component.starts_with("bluetooth-") {
                            bluetooth_errors.insert(component, detail);
                            let snapshot = lan_state.borrow().clone();
                            events.send(BackendEvent::StateChanged(network_status(&snapshot, &bluetooth, config.ble, &bluetooth_errors))).await.ok();
                        } else {
                            events.send(BackendEvent::StateChanged(BackendState { id: "quickshare".into(), state: "error".into(), detail: format!("Quick Share {component} stopped: {detail}. Restart the backend from Settings.") })).await.ok();
                            break;
                        }
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
                                    let result = if incoming { match destinations.get(&id) {Some(dir)=>publish_received(dir, engine.staging_path(), &config.download_dir,&transfer.files,&receive_options.remove(&id).unwrap_or_default()).await,None=>Err(anyhow::anyhow!("Missing receive staging directory"))} } else {Ok(Vec::new())};
                                    match result {Ok(paths) => {transfer.saved_paths=paths; transfer.state="completed".into(); transfer.transferred_bytes=transfer.total_bytes;}, Err(error) => {transfer.state="failed".into(); transfer.error=Some(error.to_string());}}
                                }
                                _ => {},
                            }
                        }
                        if matches!(transfer.state.as_str(), "completed"|"failed"|"rejected"|"cancelled") {
                            pending.remove(&id);
                            receive_options.remove(&id);
                            if let Some(dir)=destinations.remove(&id) && dir.starts_with(engine.staging_path()) && dir != engine.staging_path() {let _=std::fs::remove_dir_all(dir);}
                        }
                        events.send(if is_request {BackendEvent::Incoming(transfer.clone())} else {BackendEvent::TransferUpdated(transfer.clone())}).await.ok();
                    }
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {},
                    Err(_) => break,
                },
                _ = expiry.tick() => {let expired:Vec<_>=pending.iter().filter(|(_,t)|t.elapsed()>Duration::from_secs(120)).map(|(id,_)|id.clone()).collect(); for id in expired {pending.remove(&id);action(&engine,&id,TransferAction::ConsentDecline);}}
            }
        }
        let cleanup = engine.stop().await;
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
        cleanup
    });
    Ok(linuxdrop_core::CommandSender::track(commands, task))
}

fn network_status(
    lan: &rqs_lib::lan_policy::LanSnapshot,
    bluetooth: &Result<String>,
    ble_enabled: bool,
    failures: &std::collections::BTreeMap<String, String>,
) -> BackendState {
    let lan_ready = lan.available();
    let ble_ready = ble_enabled && bluetooth.is_ok() && failures.is_empty();
    let mut detail = if lan_ready {
        "Quick Share LAN active".to_string()
    } else {
        "Quick Share has no enabled LAN interface; waiting for a network connection".to_string()
    };
    if !lan.errors.is_empty() {
        detail.push_str(&format!("; LAN listener errors: {}", lan.errors.join("; ")));
    }
    match bluetooth {
        Ok(name) if ble_enabled && failures.is_empty() => detail.push_str(&format!("; Bluetooth discovery on {name}")),
        Ok(name) if ble_enabled => detail.push_str(&format!("; Bluetooth controller {name}")),
        Ok(_) => detail.push_str("; Bluetooth discovery disabled in settings"),
        Err(error) => detail.push_str(&format!("; Bluetooth unavailable: {error}. Enable a BlueZ controller and restart LinuxDrop to use Bluetooth discovery")),
    }
    for (component, reason) in failures {
        detail.push_str(&format!("; {component} unavailable: {reason}"));
    }
    BackendState {
        id: "quickshare".into(),
        state: if lan_ready || ble_ready {
            "ready"
        } else if !lan.errors.is_empty() || !failures.is_empty() {
            "error"
        } else {
            "unavailable"
        }
        .into(),
        detail,
    }
}

async fn bluetooth_ready(name: Option<&str>) -> Result<String> {
    Ok(linuxdrop_network::bluetooth_adapter(name)
        .await?
        .name()
        .to_owned())
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
    files: &[linuxdrop_core::SendSource],
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
    for source in files {
        source.verify()?;
        transfer.total_bytes = transfer
            .total_bytes
            .checked_add(source.size())
            .context("File size overflow")?;
        transfer.files.push(TransferFile {
            name: source.name().into(),
            size: source.size(),
            transferred: 0,
        });
    }
    Ok((
        SendInfo {
            peer_endpoint_id: peer.endpoint_id(),
            id: id.into(),
            name: transfer.peer_name.clone(),
            addr: if peer.ble_addr.is_some() {
                "0.0.0.0:0".into()
            } else {
                peer.socket_address()?.to_string()
            },
            ob: OutboundPayload::OpenedFiles(files.to_vec()),
            ble: peer.ble_addr.is_some(),
        },
        transfer,
    ))
}
fn valid_selection(options: &linuxdrop_core::ReceiveOptions, count: usize) -> bool {
    options.selected_indices.as_ref().is_none_or(|indices| {
        !indices.is_empty()
            && indices.iter().all(|index| *index < count)
            && indices
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                == indices.len()
    })
}
async fn publish_received(
    dir: &Path,
    root: &Path,
    destination: &Path,
    files: &[TransferFile],
    options: &linuxdrop_core::ReceiveOptions,
) -> Result<Vec<String>> {
    if !dir.starts_with(root) || dir == root {
        bail!("Invalid staging directory");
    }
    let mut paths = vec![];
    if !valid_selection(options, files.len()) {
        bail!("Invalid receive selection");
    }
    let selected: std::collections::HashSet<_> = options
        .selected_indices
        .clone()
        .unwrap_or_else(|| (0..files.len()).collect())
        .into_iter()
        .map(|index| files[index].name.as_str())
        .collect();
    let store =
        linuxdrop_storage::ReceiveStore::open(options.directory.as_deref().unwrap_or(destination))?;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            bail!("Unexpected receive entry");
        }
        let filename = entry.file_name();
        let base = filename.to_str().context("Invalid filename")?;
        if !selected.contains(base) {
            continue;
        }
        let mut pending = store.create(base)?;
        let mut source = tokio::fs::File::open(entry.path()).await?;
        tokio::io::copy(&mut source, &mut pending.file).await?;
        paths.push(
            pending
                .commit_with_policy(options.collision_policy)
                .await?
                .to_string_lossy()
                .into(),
        );
    }
    Ok(paths)
}

#[cfg(test)]
mod tests {
    #[test]
    fn signed_curve_coordinates_preserve_leading_zeroes_and_reject_truncation() {
        fn bytes(hex: &str) -> Vec<u8> {
            hex.as_bytes()
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                .collect()
        }
        // Public point 379*G: x needs left-padding after signed-integer encoding;
        // y needs a sign-protection byte. These exercise both wire encodings.
        let x = bytes("005543894af3d00ed7d740abdbd75c96b06877b787db5f70eea78b90a8d7c00a");
        let y = bytes("bb4c85a3d8ea29efaafa24406912dd84d5b14dc32bf656ef6c6bd58a5d943f92");
        let mut signed_y = vec![0];
        signed_y.extend_from_slice(&y);
        let decoded = rqs_lib::utils::decode_p256_public_key(&x[1..], &signed_y).unwrap();
        let mut expected = vec![4];
        expected.extend_from_slice(&x);
        expected.extend_from_slice(&y);
        assert_eq!(decoded.to_sec1_bytes().as_ref(), expected);
        assert!(
            rqs_lib::utils::decode_p256_public_key(&x, &y).is_err(),
            "Negative signed coordinate"
        );
        assert!(rqs_lib::utils::decode_p256_public_key(&[], &signed_y).is_err());
        assert!(rqs_lib::utils::decode_p256_public_key(&[0; 34], &signed_y).is_err());
        assert!(rqs_lib::utils::decode_p256_public_key(&[1; 33], &signed_y).is_err());
        assert!(
            rqs_lib::utils::decode_p256_public_key(&[0], &[0]).is_err(),
            "Not on the curve"
        );
    }
    #[test]
    fn network_recovery_keeps_bluetooth_failures_visible() {
        let mut lan = rqs_lib::lan_policy::LanSnapshot::default();
        let bluetooth = Ok("hci0".to_owned());
        let mut failures = std::collections::BTreeMap::new();
        failures.insert("bluetooth-gatt".into(), "No advertisement slots".into());
        assert_eq!(
            super::network_status(&lan, &bluetooth, true, &failures).state,
            "error"
        );
        lan.interfaces.push(linuxdrop_network::InterfaceAddress {
            name: "eth0".into(),
            address: "192.0.2.1".parse().unwrap(),
            netmask: "255.255.255.0".parse().unwrap(),
            index: 1,
            loopback: false,
        });
        let recovered = super::network_status(&lan, &bluetooth, true, &failures);
        assert_eq!(recovered.state, "ready");
        assert!(recovered.detail.contains("LAN active"));
        assert!(recovered.detail.contains("No advertisement slots"));
        assert!(!recovered.detail.contains("Bluetooth discovery on"));
        lan.interfaces.clear();
        assert_eq!(
            super::network_status(&lan, &Ok(String::new()), false, &Default::default()).state,
            "unavailable"
        );
    }
    use super::*;
    use rqs_lib::hdl::{InboundRequest, OutboundRequest};
    use rqs_lib::utils::RemoteDeviceInfo;
    #[tokio::test]
    async fn ukey2_loopback_requires_both_consents_and_transfers_exact_bytes() {
        for address in ["127.0.0.1:0", "[::1]:0"] {
            transfer_over(address).await;
        }
    }
    async fn transfer_over(bind: &str) {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        let receive = dir.path().join("received");
        std::fs::create_dir(&receive).unwrap();
        let bytes: Vec<u8> = (0..700_123).map(|n| (n % 251) as u8).collect();
        // Both endpoints share one engine budget in this loopback test. Each
        // direction must debit its 700123-byte payload (two seconds together).
        rqs_lib::payload_budget::set(Some(bytes.len() as u64));
        std::fs::write(&source, &bytes).unwrap();
        let empty = dir.path().join("empty");
        std::fs::write(&empty, []).unwrap();
        let _engine = RQS::new(
            Visibility::Visible,
            None,
            Some(receive.clone()),
            Some("LinuxDrop protocol test".into()),
        );
        let listener = tokio::net::TcpListener::bind(bind).await.unwrap();
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
        let proxy = tokio::net::TcpListener::bind(bind).await.unwrap();
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
        let sources = vec![
            linuxdrop_core::SendSource::open(&source).unwrap(),
            linuxdrop_core::SendSource::open(&empty).unwrap(),
        ];
        std::fs::rename(&source, dir.path().join("moved-source")).unwrap();
        std::fs::write(&source, b"replacement must never be sent").unwrap();
        let outgoing = tokio::spawn(async move {
            let mut protocol = OutboundRequest::new(
                *b"TEST",
                socket,
                "outgoing-test".into(),
                tx_messages,
                OutboundPayload::OpenedFiles(sources),
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
        let mut payload_started = None;
        tokio::time::timeout(Duration::from_secs(20), async {
            while completed < 2 {
                let message = events.recv().await.unwrap();
                let Message::Client(event) = message.msg else {
                    continue;
                };
                if event.state == Some(TransferState::WaitingForUserConsent) {
                    let metadata = event.metadata.unwrap();
                    codes.insert(message.id.clone(), metadata.pin_code.unwrap());
                    if codes.len() == 2 {
                        payload_started = Some(std::time::Instant::now());
                    }
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
        assert!(payload_started.unwrap().elapsed() >= Duration::from_millis(1950));
        rqs_lib::payload_budget::set(None);
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

    #[tokio::test]
    async fn payload_budget_wait_cancels_only_the_requested_transfer() {
        let budget = linuxdrop_network::BandwidthLimiter::new(Some(1));
        let (control, mut receiver) = broadcast::channel(16);
        let waiting_budget = budget.clone();
        let mut waiting = tokio::spawn(async move {
            rqs_lib::payload_budget::acquire(&waiting_budget, 1024, &mut receiver, "slow").await
        });
        control
            .send(ChannelMessage {
                id: "other".into(),
                msg: Message::Lib {
                    action: TransferAction::TransferCancel,
                },
            })
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), &mut waiting)
                .await
                .is_err()
        );
        control
            .send(ChannelMessage {
                id: "slow".into(),
                msg: Message::Lib {
                    action: TransferAction::TransferCancel,
                },
            })
            .unwrap();
        assert!(
            !tokio::time::timeout(Duration::from_secs(1), waiting)
                .await
                .unwrap()
                .unwrap()
                .unwrap()
        );
        let mut next = control.subscribe();
        assert!(
            tokio::time::timeout(
                Duration::from_secs(1),
                rqs_lib::payload_budget::acquire(&budget, 0, &mut next, "next")
            )
            .await
            .unwrap()
            .unwrap()
        );
    }
}
