use tokio::sync::broadcast::Sender;
use tokio::sync::mpsc::Receiver;
use tokio_util::sync::CancellationToken;

use crate::channel::{self, ChannelMessage, MessageClient, TransferKind};
use crate::errors::AppError;
use crate::hdl::{InboundRequest, OutboundPayload, OutboundRequest, TransferState};
use crate::utils::RemoteDeviceInfo;

const INNER_NAME: &str = "TcpServer";

/// Payloads at or below this ride BLE without a Wi-Fi upgrade (matches the
/// send path's own pure-BLE size guard in `outbound.rs`).
#[cfg(all(feature = "experimental", target_os = "linux"))]
const SMALL_SEND_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SendInfo {
    pub id: String,
    pub name: String,
    pub addr: String,
    pub ob: OutboundPayload,
    /// When set, send over BLE instead of Wi-Fi/TCP: the recipient is a phone
    /// discovered over BLE. The send path re-scans for it by `name` (its LE
    /// address and PSM rotate) and dials the fresh target. `addr`/`id` may be a
    /// placeholder in this case. Ignored on non-Linux / non-experimental builds.
    #[serde(default)]
    pub ble: bool,
    #[serde(default)]
    pub peer_endpoint_id: Option<[u8; 4]>,
}

pub struct TcpServer {
    endpoint_id: [u8; 4],
    tcp_listeners: crate::lan_policy::LanListeners,
    sender: Sender<ChannelMessage>,
    connect_receiver: Receiver<SendInfo>,
}

impl TcpServer {
    pub fn new(
        endpoint_id: [u8; 4],
        tcp_listeners: crate::lan_policy::LanListeners,
        sender: Sender<ChannelMessage>,
        connect_receiver: Receiver<SendInfo>,
    ) -> Result<Self, anyhow::Error> {
        Ok(Self {
            endpoint_id,
            tcp_listeners,
            sender,
            connect_receiver,
        })
    }

    pub async fn run(&mut self, ctk: CancellationToken) -> Result<(), anyhow::Error> {
        let mut jobs = tokio::task::JoinSet::new();
        let mut network_changes = tokio::time::interval(std::time::Duration::from_secs(3));
        network_changes.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        info!("{INNER_NAME}: service starting");

        loop {
            let cctk = ctk.clone();

            tokio::select! {
                _ = ctk.cancelled() => {
                    info!("{INNER_NAME}: tracker cancelled, breaking");
                    break;
                }
                Some(_) = jobs.join_next(), if !jobs.is_empty() => {},
                _ = network_changes.tick() => { self.tcp_listeners.refresh().await?; },
                Some(i) = self.connect_receiver.recv() => {
                    info!("{INNER_NAME}: connect_receiver: got {:?}", i);
                    let report_id = i.id.clone();
                    let connector = TransferConnector { endpoint_id: self.endpoint_id, sender: self.sender.clone() };
                    let error_sender = self.sender.clone();
                    jobs.spawn(async move {
                    if let Err(e) = connector.connect(cctk, i).await {
                        error!("{INNER_NAME}: error sending: {}", e.to_string());
                        // Surface the failure -- a click that only dies in the
                        // log looks like a dead button in the UI.
                        let _ = error_sender.send(ChannelMessage {
                            id: report_id,
                            msg: channel::Message::Client(MessageClient {
                                kind: TransferKind::Outbound,
                                state: Some(TransferState::Disconnected),
                                metadata: Default::default(),
                            }),
                        });
                    }
                    });
                }
                r = self.tcp_listeners.accept() => {
                    match r {
                        Ok((socket, remote_addr)) => {
                            if !socket.local_addr().is_ok_and(|local| crate::lan_policy::permits(local.ip(), remote_addr.ip())) { continue; }
                            trace!("{INNER_NAME}: new client: {remote_addr}");
                            let esender = self.sender.clone();
                            let csender = self.sender.clone();

                            if jobs.len() >= 32 { continue; }
                            jobs.spawn(async move {
                                // Detect a Wi-Fi bandwidth-upgrade CLIENT_INTRODUCTION so it can be
                                // routed to its in-flight BLE session (Milestone A: observe only).
                                #[cfg(all(feature = "experimental", target_os = "linux"))]
                                {
                                    let mut pbuf = [0u8; 512];
                                    if let Ok(n) = socket.peek(&mut pbuf).await {
                                        if let Some(eid) = crate::hdl::peek_client_introduction(&pbuf[..n]) {
                                            info!("{INNER_NAME}: BWU CLIENT_INTRODUCTION from {remote_addr} (endpoint_id={eid}) — routing TODO");
                                            return;
                                        }
                                    }
                                }
                                let session_id = uuid::Uuid::new_v4().to_string();
                                let mut ir = InboundRequest::new(socket, session_id.clone(), csender);

                                loop {
                                    match tokio::select! { _ = cctk.cancelled() => break, result = ir.handle() => result } {
                                        Ok(_) => {},
                                        Err(e) => match e.downcast_ref() {
                                            Some(AppError::NotAnError) => break,
                                            None => {
                                                if ir.state.state == TransferState::Initial {
                                                    break;
                                                }

                                                if ir.state.state != TransferState::Finished {
                                                    let _ = esender.send(ChannelMessage {
                                                        id: session_id.clone(),
                                                        msg: channel::Message::Client(MessageClient {
                                                            kind: TransferKind::Inbound,
                                                            state: Some(TransferState::Disconnected),
                                                            metadata: Default::default()
                                                        }),
                                                    });
                                                }
                                                error!("{INNER_NAME}: error while handling client: {e} ({:?})", ir.state.state);
                                                break;
                                            }
                                        },
                                    }
                                }
                            });
                        },
                        Err(err) => {
                            error!("{INNER_NAME}: error accepting: {}", err);
                            break;
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

struct TransferConnector {
    endpoint_id: [u8; 4],
    sender: Sender<ChannelMessage>,
}
impl TransferConnector {
    /// Drives one independent connection; parent JoinSet owns its lifetime.
    pub async fn connect(&self, ctk: CancellationToken, si: SendInfo) -> Result<(), anyhow::Error> {
        #[cfg(all(feature = "experimental", target_os = "linux"))]
        if si.ble {
            return self.connect_ble(ctk, si).await;
        }

        debug!("{INNER_NAME}: Connecting to: {}", si.addr);
        // A stale mDNS endpoint (device left the network) otherwise spins for
        // the OS's full connect timeout (~40s) with the UI stuck on it.
        let socket = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            crate::lan_policy::connect(si.addr.parse()?),
        )
        .await
        .map_err(|_| {
            anyhow::anyhow!(
                "connect to {} timed out — the device may have left the network",
                si.addr
            )
        })??;

        // Wrap in the migratable stream so a bandwidth upgrade can swap the
        // transport even on a direct TCP send (Windows receivers demand the
        // payload over an upgraded channel).
        #[cfg(all(feature = "experimental", target_os = "linux"))]
        let socket = crate::hdl::MigratableStream::Tcp(socket);

        let report_id = si.id.clone();
        let mut or = OutboundRequest::new(
            self.endpoint_id,
            socket,
            si.id,
            self.sender.clone(),
            si.ob,
            RemoteDeviceInfo {
                device_type: crate::DeviceType::Unknown,
                name: si.name,
            },
        );

        or.set_peer_endpoint(si.peer_endpoint_id);

        // Send connection request
        or.send_connection_request().await?;
        // Send UKEY init
        or.send_ukey2_client_init().await?;

        self.drive_outbound(ctk, or, report_id).await;
        Ok(())
    }

    /// Send over BLE to a phone discovered on its receive screen. Re-scans for
    /// it by name (its LE address and PSM rotate), dials the fresh target, then
    /// drives the same outbound handshake over the L2CAP-backed stream.
    #[cfg(all(feature = "experimental", target_os = "linux"))]
    async fn connect_ble(&self, ctk: CancellationToken, si: SendInfo) -> Result<(), anyhow::Error> {
        use crate::hdl::{dial, scan_once};
        use std::time::Duration;

        debug!("{INNER_NAME}: BLE send to {:?}", si.name);
        let adapter = crate::bluetooth_adapter().await?;
        if !adapter.is_powered().await? {
            anyhow::bail!("Bluetooth is switched off");
        }

        // Airtime for the scan/connect; also stops our own receiver scanner.
        let _suppressor = crate::hdl::BleScanSuppressor::new();

        // The phone rotates its LE address (and PSM) every few minutes, and a
        // scan can still surface the pre-rotation address from BlueZ's cache.
        // A failed dial retries with a fresh scan. Addresses that already
        // failed are deprioritized but NOT banned: a dial failure is usually
        // transient (BlueZ mid-wind-down after the scan), not proof the
        // address is dead -- the very same address often connects on the
        // next, settled attempt.
        let mut tried: Vec<bluer::Address> = Vec::new();
        let mut connected = None;
        let mut last_err: Option<anyhow::Error> = None;
        for round in 1..=3u8 {
            if round > 1 {
                // Give BlueZ time to wind the previous discovery/dial down --
                // starting a new scan immediately fails with "operation
                // already in progress".
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
            let targets = match scan_once(&adapter, Duration::from_secs(20), Some(&si.name)).await {
                Ok(t) => t,
                Err(e) => {
                    warn!("{INNER_NAME}: scan round {round} failed ({e}); retrying");
                    last_err = Some(e);
                    continue;
                }
            };
            let (fresh, retry): (Vec<_>, Vec<_>) =
                targets.into_iter().partition(|t| !tried.contains(&t.addr));
            let Some(target) = fresh
                .into_iter()
                .next()
                .or_else(|| retry.into_iter().next())
            else {
                last_err = Some(anyhow::anyhow!(
                    "{} is no longer advertising over BLE",
                    si.name
                ));
                continue;
            };
            match dial(&adapter, &target).await {
                Ok(stream) => {
                    connected = Some((stream, target.rdi, target.endpoint_id));
                    break;
                }
                Err(e) => {
                    warn!(
                        "{INNER_NAME}: dial {} failed on round {round} ({e}); re-scanning",
                        target.addr
                    );
                    tried.push(target.addr);
                    last_err = Some(e);
                }
            }
        }
        let Some((stream, rdi, peer_endpoint)) = connected else {
            return Err(last_err.unwrap_or_else(|| anyhow::anyhow!("BLE connect failed")));
        };

        // A payload that fits comfortably over BLE never needs a Wi-Fi
        // upgrade: a few bytes of pasted text shouldn't cost a multi-second
        // Wi-Fi Direct join that also drops this machine off its own network.
        let total_bytes = si.ob.sources()?.iter().try_fold(0u64, |sum, file| {
            sum.checked_add(file.size())
                .ok_or_else(|| anyhow::anyhow!("File size overflow"))
        })?;

        // The UI knows this transfer by `si.id` (the `ble://<name>` endpoint
        // id) -- a Disconnected report under any other id renders as a
        // detached "Unknown" card.
        let report_id = si.id.clone();
        let mut or = OutboundRequest::new(
            self.endpoint_id,
            stream,
            si.id,
            self.sender.clone(),
            si.ob,
            rdi,
        );
        or.set_peer_endpoint(Some(peer_endpoint));
        if total_bytes <= SMALL_SEND_BYTES {
            // BLE_L2CAP (10) only: the phone (advertiser) has no Wi-Fi medium
            // in common with us, so it never offers an upgrade.
            info!("{INNER_NAME}: {total_bytes}-byte payload; BLE only, no Wi-Fi upgrade");
            or.set_mediums(vec![10]);
        } else {
            // Advertise WIFI_LAN (5) + WIFI_DIRECT (8) + WIFI_HOTSPOT (3) +
            // BLE_L2CAP (10). The phone (advertiser) picks the best and offers it:
            // same LAN → WIFI_LAN (we connect to its ip:port); no shared LAN →
            // WIFI_DIRECT (it hosts its own group, we join it). Either way the
            // payload leaves BLE for Wi-Fi speed.
            or.set_mediums(crate::hdl::upgrade_mediums());
        }

        or.send_connection_request().await?;
        or.send_ukey2_client_init().await?;

        self.drive_outbound(ctk, or, report_id).await;
        Ok(())
    }

    /// Drives an outbound transfer to completion, reporting a Disconnected state
    /// on an unexpected error. Generic over the transport (`TcpStream` for
    /// Wi-Fi, the BLE-backed stream for L2CAP).
    async fn drive_outbound<S>(
        &self,
        ctk: CancellationToken,
        mut or: OutboundRequest<S>,
        report_id: String,
    ) where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + crate::hdl::WifiUpgradable,
    {
        loop {
            tokio::select! {
                _ = ctk.cancelled() => {
                    info!("{INNER_NAME}: tracker cancelled, breaking");
                    break;
                },
                r = or.handle() => {
                    if let Err(e) = r {
                        match e.downcast_ref() {
                            Some(AppError::NotAnError) => break,
                            None => {
                                if or.state.state == TransferState::Initial {
                                    break;
                                }

                                if or.state.state != TransferState::Finished && or.state.state != TransferState::Cancelled {
                                    let _ = self.sender.clone().send(ChannelMessage {
                                        id: report_id.clone(),
                                        msg: channel::Message::Client(MessageClient {
                                            kind: TransferKind::Outbound,
                                            state: Some(TransferState::Disconnected),
                                            metadata: Default::default()
                                        }),
                                    });
                                }
                                error!("{INNER_NAME}: error while handling client: {e} ({:?})", or.state.state);
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
}
