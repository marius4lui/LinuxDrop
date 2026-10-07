//! BLE *initiator* for sending (PC -> phone), the mirror image of the receiver
//! stack in `blea`/`gatt`/`l2cap`.
//!
//! A phone on its Quick Share receive screen advertises itself as a Nearby
//! Connections endpoint on service 0xFEF3 (see `blea::receiver_service_data`
//! for the byte layout we emit; the phone's is the same shape). This module
//!
//!   1. scans for that advertisement and decodes the endpoint id, device
//!      name/type and the L2CAP PSM (`scan_once` / `decode_receiver_advert`),
//!   2. opens an LE CoC to that PSM and drives the *client* half of the GmsCore
//!      u32-framed dialect -- send `RequestDataConnection`, await
//!      `DataConnectionReady`, send our socket INTRODUCTION (`dial`),
//!   3. bridges that socket to an in-memory duplex so the transport-agnostic
//!      `OutboundRequest` can run its ConnectionRequest + Ukey2 + payload
//!      handshake over it, exactly as it does over TCP.
//!
//! Stage-0 ground truth (Pixel 9 Pro): the CoC connects in ~350-475ms, the
//! client must speak first (reading first is reset by the phone), and the
//! `03`->`17` handshake is symmetric with what the phone sent us as a client.

use std::collections::HashMap;
use std::time::Duration;
use tokio::time::Instant;

use bluer::l2cap::{Security, SecurityLevel, Socket, SocketAddr, Stream};
use bluer::{Adapter, Address, AddressType};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use tokio::sync::broadcast::Sender;
use tokio_util::sync::CancellationToken;

use crate::hdl::{
    CMD_REQUEST_DATA_CONNECTION, CMD_RESPONSE_DATA_CONNECTION_READY, EndpointInfo,
    MigratableStream, SOCKET_CTRL_DISCONNECTION, SVC_HASH, build_ack_frame, build_intro_frame,
    read_frame, send_frame,
};
use crate::utils::{DeviceType, RemoteDeviceInfo};

const INNER_NAME: &str = "BleClient";

/// Android caps its L2CAP transmit at 1024 bytes; extra receive room is free.
const RECV_MTU: u16 = 4096;
/// Hard cap on the CoC connect itself (Stage 0 measured ~0.5s; a phone that
/// wandered out of range should fail fast, not hang the send).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(6);
/// How long to wait for the phone's `DataConnectionReady` after we ask.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// A phone discovered over BLE that we can send to.
#[derive(Debug, Clone)]
pub struct BleTarget {
    pub addr: Address,
    pub addr_type: AddressType,
    pub psm: u16,
    /// The 4-byte Nearby endpoint id from the advertisement.
    pub endpoint_id: [u8; 4],
    /// Device name + type parsed from the advertisement's endpoint_info.
    pub rdi: RemoteDeviceInfo,
}

/// The interesting fields of a decoded 0xFEF3 receiver advertisement.
#[derive(Debug, Clone)]
pub struct ReceiverAdvert {
    pub endpoint_id: [u8; 4],
    pub device_type: DeviceType,
    pub name: Option<String>,
    pub psm: Option<u16>,
    /// endpoint_info visibility bit clear == visible to "Everyone".
    pub visible: bool,
}

/// Inverse of `blea::receiver_service_data` (and its fast/background form).
///
/// Two layouts appear on the air:
/// * **full** `[0x48][svc hash 3][u32 len][data][token 2][mask][psm 2]?` -- the
///   "Everyone" receive screen; carries the name and (bit 0 of the mask) a PSM.
/// * **fast** `[0x4a][u8 len][data][token 2]` -- the idle/contacts-only
///   background beacon; no name, no PSM.
///
/// `data` is `[0x23][svc hash 3]?[endpoint_id 4][einfo len][einfo][mac 6][extra 2]?`
/// (the service hash and MAC are present only in the full form).
pub fn decode_receiver_advert(sd: &[u8]) -> Option<ReceiverAdvert> {
    if sd.is_empty() || sd[0] >> 5 != 2 {
        return None;
    }
    // Bit 1 of the mediums header selects the fast (background) form.
    let fast = sd[0] & 0x02 != 0;
    let (data, trailer): (&[u8], &[u8]) = if fast {
        let len = *sd.get(1)? as usize;
        (sd.get(2..2 + len)?, sd.get(2 + len..)?)
    } else {
        if sd.len() < 8 || sd[1..4] != SVC_HASH {
            return None;
        }
        let len = u32::from_be_bytes([sd[4], sd[5], sd[6], sd[7]]) as usize;
        (sd.get(8..8 + len)?, sd.get(8 + len..)?)
    };

    let eid_at = if fast { 1 } else { 4 };
    if !fast && (data.len() < 4 || data[1..4] != SVC_HASH) {
        return None;
    }
    let endpoint_id: [u8; 4] = data.get(eid_at..eid_at + 4)?.try_into().ok()?;

    let elen = *data.get(eid_at + 4)? as usize;
    let einfo = data.get(eid_at + 5..eid_at + 5 + elen)?;
    // einfo: [ver 3b|visibility 1b|device_type 3b|reserved][identity 16][name len][name]
    let (device_type, visible) = match einfo.first() {
        Some(b) => (DeviceType::from_raw_value((b >> 1) & 0x7), b & 0x10 == 0),
        None => (DeviceType::Unknown, true),
    };
    let name = einfo.get(17).and_then(|nlen| {
        einfo
            .get(18..18 + *nlen as usize)
            .map(|n| String::from_utf8_lossy(n).into_owned())
    });

    // PSM lives in the mediums trailer: [token 2][mask][fields...], mask bit 0.
    let psm = if !fast && trailer.len() >= 5 && trailer[2] & 0x01 != 0 {
        Some(u16::from_be_bytes([trailer[3], trailer[4]])).filter(|p| *p != 0)
    } else {
        None
    };

    Some(ReceiverAdvert {
        endpoint_id,
        device_type,
        name,
        psm,
        visible,
    })
}

impl ReceiverAdvert {
    pub(crate) fn into_target(self, addr: Address, addr_type: AddressType) -> Option<BleTarget> {
        Some(BleTarget {
            addr,
            addr_type,
            psm: self.psm?,
            endpoint_id: self.endpoint_id,
            rdi: RemoteDeviceInfo {
                name: self.name.unwrap_or_else(|| addr.to_string()),
                device_type: self.device_type,
            },
        })
    }
}

/// Compatibility entry point for diagnostic examples. Names are exact filters,
/// never identity: multiple matching endpoints are rejected as ambiguous.
pub async fn scan_once(
    adapter: &Adapter,
    timeout: Duration,
    want_name: Option<&str>,
) -> anyhow::Result<Vec<BleTarget>> {
    let mut targets = super::recipient_scan::scan(
        adapter.name().into(),
        timeout,
        None,
        CancellationToken::new(),
        false,
    )
    .await?;
    if let Some(name) = want_name {
        targets.retain(|target| target.rdi.name == name);
        if targets.len() > 1 {
            anyhow::bail!("Multiple BLE endpoints have that name; select a specific device");
        }
    }
    Ok(targets)
}

/// The selected endpoint is correlation data, not authentication. UKEY2 and
/// explicit SAS/consent still authenticate the eventual transfer session.
pub async fn scan_target(
    adapter: &Adapter,
    timeout: Duration,
    endpoint: [u8; 4],
    cancel: CancellationToken,
) -> anyhow::Result<Vec<BleTarget>> {
    super::recipient_scan::scan(
        adapter.name().into(),
        timeout,
        Some(endpoint),
        cancel,
        false,
    )
    .await
}

/// Recover recipient discovery independently of transfer workers. Peer entries
/// expire instead of surviving indefinitely after a phone leaves receive mode.
pub async fn ble_discovery(
    sender: Sender<EndpointInfo>,
    status: Sender<crate::channel::ChannelMessage>,
    ctk: CancellationToken,
) -> anyhow::Result<()> {
    let mut retry = Duration::from_secs(2);
    let mut known = HashMap::<String, (EndpointInfo, tokio::time::Instant)>::new();
    let result = loop {
        if ctk.is_cancelled() {
            break Ok(());
        }
        let result = async {
            let adapter =
                tokio::time::timeout(Duration::from_secs(5), crate::bluetooth_adapter()).await??;
            super::recipient_scan::scan(
                adapter.name().into(),
                Duration::from_secs(3),
                None,
                ctk.clone(),
                true,
            )
            .await
        }
        .await;
        let pause = match result {
            Ok(found) => {
                retry = Duration::from_secs(2);
                if !ctk.is_cancelled() {
                    let _ = status.send(crate::channel::ChannelMessage {
                        id: "backend".into(),
                        msg: crate::channel::Message::BluetoothServiceReady {
                            component: "bluetooth-peer-discovery".into(),
                        },
                    });
                    for target in found {
                        let peer = EndpointInfo {
                            id: format!("ble://{}", hex::encode(target.endpoint_id)),
                            name: Some(target.rdi.name),
                            rtype: Some(target.rdi.device_type),
                            present: Some(true),
                            ble_addr: Some(target.addr.to_string()),
                            ble_psm: Some(target.psm),
                            ble_endpoint_id: Some(target.endpoint_id),
                            ..Default::default()
                        };
                        known.insert(peer.id.clone(), (peer.clone(), tokio::time::Instant::now()));
                        let _ = sender.send(peer);
                    }
                }
                Duration::from_secs(4)
            }
            Err(error) => {
                crate::backend_failure(&status, "bluetooth-peer-discovery", &error);
                for (_, (mut peer, _)) in known.drain() {
                    peer.present = Some(false);
                    let _ = sender.send(peer);
                }
                if crate::lifecycle::cleanup_unconfirmed(&error) {
                    break Err(error);
                }
                let pause = retry;
                retry = (retry * 2).min(Duration::from_secs(15));
                pause
            }
        };
        known.retain(|_, (peer, seen)| {
            if seen.elapsed() < Duration::from_secs(30) {
                return true;
            }
            let mut removed = peer.clone();
            removed.present = Some(false);
            let _ = sender.send(removed);
            false
        });
        tokio::select! { _ = ctk.cancelled() => break Ok(()), _ = tokio::time::sleep(pause) => {} }
    };
    for (_, (mut peer, _)) in known.drain() {
        peer.present = Some(false);
        let _ = sender.send(peer);
    }
    result
}

/// Opens the LE CoC and runs the client half of the data-connection handshake,
/// returning a [`MigratableStream`] the outbound handshake can drive. A
/// background task bridges that stream to the raw socket (service-hash framing
/// + per-frame acknowledgements), mirroring the receiver's `serve_weave_messages`.
///
/// The caller MUST have stopped any discovery first: connecting while a scan is
/// live comes back as an instantly-"connected" but dead socket (ENOTCONN on
/// first I/O).
pub async fn dial(
    adapter: &Adapter,
    target: &BleTarget,
) -> Result<MigratableStream, anyhow::Error> {
    let local = adapter.address().await?;

    // Drop any ACL link BlueZ still holds to the phone from discovery: while one
    // is up, the raw L2CAP `connect()` returns instant success on a channel that
    // isn't actually established. Best-effort -- the retry below is the backstop.
    if let Ok(dev) = adapter.device(target.addr) {
        let _ = dev.disconnect().await;
    }
    // Let BlueZ finish winding the scan/disconnect down before we connect.
    tokio::time::sleep(Duration::from_millis(500)).await;

    // `bluer`'s connect returns Ok without validating when the `connect()`
    // syscall succeeds *immediately* (SO_ERROR is only checked on the
    // EINPROGRESS path). Right after discovery, BlueZ often still holds a link
    // to the phone, so connect returns instantly but the CoC isn't really up --
    // the first write then fails ENOTCONN. So validate by sending the opening
    // RequestDataConnection frame, and retry the whole connect if it fails.
    let mut stream = None;
    for attempt in 1..=5 {
        let socket = Socket::<Stream>::new_stream()?;
        // The phone accepts an insecure (unbonded) channel; asking for more fails.
        socket.set_security(Security {
            level: SecurityLevel::Low,
            key_size: 0,
        })?;
        socket.set_recv_mtu(RECV_MTU)?;
        socket.bind(SocketAddr::new(local, AddressType::LePublic, 0))?;

        let t0 = Instant::now();
        let mut s = match tokio::time::timeout(
            CONNECT_TIMEOUT,
            socket.connect(SocketAddr::new(target.addr, target.addr_type, target.psm)),
        )
        .await
        {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                debug!("{INNER_NAME}: connect attempt {attempt} failed: {e}");
                tokio::time::sleep(Duration::from_millis(700)).await;
                continue;
            }
            Err(_) => {
                debug!(
                    "{INNER_NAME}: connect attempt {attempt} to {} timed out",
                    target.addr
                );
                continue;
            }
        };
        debug!(
            "{INNER_NAME}: connect returned in {:?} (attempt {attempt}); validating",
            t0.elapsed()
        );
        // Validate by sending the opening frame. When `connect()` returned
        // instantly (0), the CoC is still establishing and the first write can
        // race it (ENOTCONN); the socket usually becomes writable within a
        // moment, so retry the write on the SAME socket before giving up on it.
        let mut sent = false;
        for w in 1..=12 {
            match send_frame(&mut s, &[CMD_REQUEST_DATA_CONNECTION]).await {
                Ok(()) => {
                    sent = true;
                    break;
                }
                Err(e) => {
                    let enotconn = e
                        .downcast_ref::<std::io::Error>()
                        .and_then(|io| io.raw_os_error())
                        == Some(107);
                    if !enotconn {
                        debug!("{INNER_NAME}: opening frame errored (not ENOTCONN): {e}");
                        break;
                    }
                    trace!("{INNER_NAME}: channel not up yet (write {w}); waiting");
                    tokio::time::sleep(Duration::from_millis(300)).await;
                }
            }
        }
        if sent {
            info!(
                "{INNER_NAME}: connected to {} (psm {}) in {:?} (attempt {attempt})",
                target.addr,
                target.psm,
                t0.elapsed()
            );
            stream = Some(s);
            break;
        }
        debug!("{INNER_NAME}: attempt {attempt} never became writable; reconnecting");
        drop(s);
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let mut stream = stream.ok_or_else(|| {
        anyhow::anyhow!("couldn't establish a live L2CAP channel to {}", target.addr)
    })?;

    // The opening RequestDataConnection is already sent; await DataConnectionReady.
    // (Stage 0: reading before speaking gets us reset.)
    let mut leftover: Vec<u8> = Vec::new();
    let ready_deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    loop {
        let msg = tokio::select! {
            _ = tokio::time::sleep_until(ready_deadline) => {
                anyhow::bail!("no DataConnectionReady from {} within {:?}", target.addr, HANDSHAKE_TIMEOUT);
            }
            frame = read_frame(&mut stream, &mut leftover) => match frame? {
                Some(m) => m,
                None => anyhow::bail!("{} closed during the data-connection handshake", target.addr),
            },
        };
        if msg == [CMD_RESPONSE_DATA_CONNECTION_READY] {
            break;
        }
        debug!(
            "{INNER_NAME}: pre-ready frame {}",
            hex::encode(&msg[..msg.len().min(16)])
        );
    }

    // Introduce our socket (both sides do this before data flows).
    stream.write_all(&build_intro_frame()).await?;

    let (outbound_side, local_dup) = tokio::io::duplex(64 * 1024);
    tokio::spawn(bridge(stream, local_dup, leftover));
    Ok(MigratableStream::Ble(outbound_side))
}

/// Bridges the L2CAP socket to the duplex the outbound handshake reads/writes:
/// outbound -> `[u32 len][OfflineFrame]` wrapped as `[u32][svc hash][entry]`;
/// phone -> service-hash entries unwrapped back to the duplex (and acked), the
/// client-role twin of `l2cap::serve_weave_messages`.
async fn bridge(mut stream: Stream, local: tokio::io::DuplexStream, mut leftover: Vec<u8>) {
    let (mut local_rd, mut local_wr) = tokio::io::split(local);
    let mut out_buf: Vec<u8> = Vec::new();
    let mut rbuf = [0u8; 2048];

    loop {
        tokio::select! {
            frame = read_frame(&mut stream, &mut leftover) => {
                let msg = match frame {
                    Ok(Some(m)) => m,
                    Ok(None) => break,
                    Err(e) => { debug!("{INNER_NAME}: read error: {e}"); break; }
                };
                if msg.len() < 3 {
                    continue;
                }
                if msg[..3] == [0, 0, 0] {
                    let ctrl = if msg.len() >= 5 && msg[3] == 0x08 { msg[4] } else { 0 };
                    if ctrl == SOCKET_CTRL_DISCONNECTION {
                        debug!("{INNER_NAME}: phone sent DISCONNECTION");
                        break;
                    }
                } else if msg[..3] == SVC_HASH {
                    // Payload is the [u32 len][OfflineFrame] the outbound handler reads.
                    if local_wr.write_all(&msg[3..]).await.is_err() {
                        break;
                    }
                    // Acknowledge or the phone's sender stalls.
                    if stream.write_all(&build_ack_frame(msg.len() - 3)).await.is_err() {
                        break;
                    }
                } else {
                    debug!("{INNER_NAME}: frame for foreign service {}", hex::encode(&msg[..3]));
                }
            }
            r = local_rd.read(&mut rbuf) => {
                match r {
                    Ok(0) => break,
                    Ok(n) => {
                        out_buf.extend_from_slice(&rbuf[..n]);
                        // Re-frame each complete [u32 len][entry] the outbound side wrote.
                        loop {
                            if out_buf.len() < 4 { break; }
                            let len = u32::from_be_bytes(out_buf[..4].try_into().unwrap()) as usize;
                            if out_buf.len() < 4 + len { break; }
                            let entry: Vec<u8> = out_buf.drain(0..4 + len).collect();
                            let mut frame = Vec::with_capacity(4 + 3 + entry.len());
                            frame.extend_from_slice(&((3 + entry.len()) as u32).to_be_bytes());
                            frame.extend_from_slice(&SVC_HASH);
                            frame.extend_from_slice(&entry);
                            if let Err(e) = stream.write_all(&frame).await {
                                warn!(
                                    "{INNER_NAME}: L2CAP write of {}-byte frame failed: {e}",
                                    frame.len()
                                );
                                return;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }
        }
    }
    info!("{INNER_NAME}: BLE send session ended");
}
