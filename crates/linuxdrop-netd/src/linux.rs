use linuxdrop_hardware::{inventory, select_radio, SelectionRequest};
use linuxdrop_netd::{
    DiagnosticReport, DiagnosticStep, Lease, LeaseKind, RecoveryIssue, Request, Response,
    SOCKET_PATH,
};
use std::{
    collections::HashMap,
    io,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
    sync::Arc,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    process::Command,
    sync::{Mutex, Semaphore},
    time::{timeout, Duration},
};

const JOURNAL: &str = "/var/lib/linuxdrop-netd/leases.json";
const MAX_REQUEST: u64 = 4096;
#[derive(Default)]
struct State {
    leases: HashMap<String, Lease>,
    children: HashMap<String, tokio::process::Child>,
    recovery_errors: Vec<String>,
    attached: std::collections::HashSet<String>,
    pending_p2p: HashMap<String, PendingP2p>,
    cancelled_p2p: std::collections::HashSet<String>,
}
type Shared = Arc<Mutex<State>>;

struct PendingP2p {
    cancel: tokio_util::sync::CancellationToken,
    interfaces: std::collections::HashSet<String>,
}

/// Cancellation and bookkeeping survive an abruptly disconnected helper client.
struct P2pOperation {
    shared: Shared,
    lease_id: String,
    cancel: tokio_util::sync::CancellationToken,
}
impl Drop for P2pOperation {
    fn drop(&mut self) {
        self.cancel.cancel();
        let shared = self.shared.clone();
        let lease_id = self.lease_id.clone();
        tokio::spawn(async move {
            let mut state = shared.lock().await;
            if state
                .pending_p2p
                .get(&lease_id)
                .is_some_and(|p| p.cancel.is_cancelled())
            {
                state.pending_p2p.remove(&lease_id);
            }
        });
    }
}

fn competing_use(
    lease: &Lease,
    interface: &linuxdrop_hardware::NetworkInterface,
    pending: Option<&PendingP2p>,
) -> bool {
    if interface.phy.as_deref() != Some(&lease.phy) || !interface.in_use() {
        return false;
    }
    if lease.kind != LeaseKind::DirectWifi {
        return interface.name != lease.interface;
    }
    if interface.active_connection_uuid.is_some()
        && interface.active_connection_uuid == lease.connection_uuid
    {
        return false;
    }
    // A supplicant-created group has no NetworkManager connection UUID. It must
    // never gain a default route or be taken over by another NM connection.
    if interface.default_route
        || interface.active_connection.is_some()
        || interface.active_connection_uuid.is_some()
    {
        return true;
    }
    if lease
        .p2p_group
        .as_ref()
        .is_some_and(|group| group.interface == interface.name)
    {
        return false;
    }
    // Between GroupStarted and journal publication, tolerate only a newly
    // created VIF. Existing parent/sibling interfaces remain protected.
    !pending.is_some_and(|operation| !operation.interfaces.contains(&interface.name))
}

pub async fn run() -> io::Result<()> {
    let socket = Path::new(SOCKET_PATH);
    if socket.exists() {
        if UnixStream::connect(socket).await.is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "netd already running",
            ));
        }
        std::fs::remove_file(socket)?;
    }
    let mut state = State::default();
    if let Ok(data) = std::fs::read(JOURNAL) {
        let leases: Vec<Lease> = serde_json::from_slice(&data).map_err(io::Error::other)?;
        for lease in leases {
            if let Err(e) = restore(&lease).await {
                state
                    .recovery_errors
                    .push(format!("{}: {e}", lease.interface));
                state.leases.insert(lease.id.clone(), lease);
            }
        }
    }
    persist(&state)?;
    let listener = UnixListener::bind(socket)?;
    // Every mutation requires an active local session and polkit authorization.
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o666))?;
    let state = Arc::new(Mutex::new(state));
    let limit = Arc::new(Semaphore::new(16));
    let mut tasks = tokio::task::JoinSet::new();
    let monitor = state.clone();
    tasks.spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            if monitor.lock().await.leases.is_empty() { continue; }
            let inventory = inventory().await;
            let mut state = monitor.lock().await;
            let leases: Vec<_> = state.leases.values().cloned().collect();
            for lease in leases {
                let dead = state
                    .children
                    .get_mut(&lease.id)
                    .is_some_and(|child| !matches!(child.try_wait(), Ok(None)));
                // DHCP is optional on a group with an assigned IPv6 link-local
                // address. Its exit must not tear down an active IPv6 transfer.
                let ipv6_survives = dead
                    && lease.p2p_group.is_some()
                    && p2p_addresses(&lease).await.is_ok_and(|a| a.ipv6.is_some());
                if ipv6_survives {
                    state.children.remove(&lease.id);
                }
                let dead = dead && !ipv6_survives;
                let radio_gone = !inventory.radios.iter().any(|r| r.phy == lease.phy);
                let regulatory_change = inventory
                    .radios
                    .iter()
                    .find(|r| r.phy == lease.phy)
                    .is_some_and(|radio| {
                        lease.allowed_frequencies.iter().any(|frequency| {
                            !radio.channels.iter().any(|c| {
                                c.frequency_mhz == *frequency && !c.disabled && !c.no_ir && !c.radar
                            })
                        })
                    });
                let unsafe_use = inventory.interfaces.iter().any(|i| competing_use(&lease, i, state.pending_p2p.get(&lease.id)));
                if dead || radio_gone || unsafe_use || regulatory_change {
                    if let Some(operation) = state.pending_p2p.remove(&lease.id) { operation.cancel.cancel(); }
                    stop_child(&mut state, &lease.id).await;
                    match restore(&lease).await {
                        Ok(()) => {
                            state.leases.remove(&lease.id);
                        }
                        Err(e) => state.recovery_errors.push(e),
                    }
                    state.recovery_errors.push(format!(
                        "{}: lease stopped after helper exit, unplug, regulatory change, or competing radio use",
                        lease.interface
                    ));
                    let _ = persist(&state);
                }
            }
        }
    });
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    loop {
        tokio::select! {
            _=terminate.recv()=>break,
            _=tokio::signal::ctrl_c()=>break,
            accepted=listener.accept()=> {
                let (socket,_)=accepted?;
                if let Ok(permit)=limit.clone().try_acquire_owned() {
                    let state=state.clone();tasks.spawn(async move { let _permit=permit; let _=serve(socket,state).await; });
                }
            }
            _=tasks.join_next(), if !tasks.is_empty()=>{}
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    let mut state = state.lock().await;
    for lease in state.leases.values().cloned().collect::<Vec<_>>() {
        stop_child(&mut state, &lease.id).await;
        match restore(&lease).await {
            Ok(()) => {
                state.leases.remove(&lease.id);
            }
            Err(e) => eprintln!("Recovery failed for {}: {e}", lease.interface),
        }
    }
    persist(&state)?;
    std::fs::remove_file(SOCKET_PATH)?;
    Ok(())
}

async fn serve(socket: UnixStream, state: Shared) -> io::Result<()> {
    let credentials = socket.peer_cred()?;
    let pid = credentials
        .pid()
        .ok_or_else(|| io::Error::other("missing peer PID"))?;
    if pid <= 0 {
        return Err(io::Error::other("invalid peer PID"));
    }
    let uid = credentials.uid();
    let start = process_start(pid)?;
    let (reader, mut writer) = socket.into_split();
    let mut reader = BufReader::new(reader);
    let mut owned: Vec<String> = Vec::new();
    let mut authorized = false;
    let result = async {
        loop {
            let mut line = Vec::new();
            // Bound unauthenticated clients; a held lease remains live while its socket
            // is open, including long transfers. EOF always triggers restoration.
            let mut bounded = (&mut reader).take(MAX_REQUEST + 1);
            let read = bounded.read_until(b'\n', &mut line);
            let length = if owned.is_empty() {
                timeout(Duration::from_secs(if authorized { 120 } else { 10 }), read)
                    .await
                    .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "idle connection"))??
            } else {
                read.await?
            };
            if length == 0 {
                break;
            }
            if length as u64 > MAX_REQUEST || line.last() != Some(&b'\n') {
                return Err(io::Error::other("invalid request framing"));
            }
            let response = match serde_json::from_slice::<Request>(&line) {
                Err(_) => Response::Error {
                    message: "invalid request".into(),
                },
                Ok(request) => {
                    if !authorized && !matches!(&request, Request::Status | Request::RecoveryStatus)
                    {
                        match authorize(pid, uid, start).await {
                            Ok(()) => authorized = true,
                            Err(e) => {
                                let mut out = serde_json::to_vec(&Response::Error { message: e })?;
                                out.push(b'\n');
                                writer.write_all(&out).await?;
                                break;
                            }
                        }
                    }
                    let joining = matches!(&request, Request::JoinP2p { .. });
                    let response = tokio::select! {
                        result = apply(request, uid, &mut owned, &state) => result,
                        result = reader.fill_buf(), if joining => {
                            if result?.is_empty() { break; }
                            return Err(io::Error::other("pipelined helper requests are not supported"));
                        }
                    };
                    match response {
                        Ok(r) => r,
                        Err(e) => Response::Error { message: e },
                    }
                }
            };
            let mut out = serde_json::to_vec(&response)?;
            out.push(b'\n');
            writer.write_all(&out).await?;
        }
        Ok(())
    }
    .await;
    let mut state = state.lock().await;
    for id in owned {
        state.cancelled_p2p.remove(&id);
        if let Some(operation) = state.pending_p2p.remove(&id) {
            operation.cancel.cancel();
        }
        state.attached.remove(&id);
        stop_child(&mut state, &id).await;
        if let Some(lease) = state.leases.get(&id).cloned() {
            match restore(&lease).await {
                Ok(()) => {
                    state.leases.remove(&id);
                }
                Err(e) => state
                    .recovery_errors
                    .push(format!("{}: {e}", lease.interface)),
            }
        }
    }
    persist(&state)?;
    result
}

fn process_start(pid: i32) -> io::Result<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    // comm is parenthesized and can contain spaces or closing parentheses.
    let tail = stat
        .rsplit_once(')')
        .ok_or_else(|| io::Error::other("invalid process stat"))?
        .1;
    tail.split_whitespace()
        .nth(19)
        .ok_or_else(|| io::Error::other("missing process start"))?
        .parse()
        .map_err(io::Error::other)
}
async fn authorize(pid: i32, uid: u32, start: u64) -> Result<(), String> {
    if process_start(pid).map_err(|e| e.to_string())? != start {
        return Err("client process changed".into());
    }
    let bus = zbus::Connection::system()
        .await
        .map_err(|e| e.to_string())?;
    let login = zbus::Proxy::new(
        &bus,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await
    .map_err(|e| e.to_string())?;
    let path: zbus::zvariant::OwnedObjectPath = login
        .call("GetUser", &uid)
        .await
        .map_err(|_| "A local desktop session is required".to_string())?;
    let user = zbus::Proxy::new(
        &bus,
        "org.freedesktop.login1",
        path,
        "org.freedesktop.login1.User",
    )
    .await
    .map_err(|e| e.to_string())?;
    let sessions: Vec<(String, zbus::zvariant::OwnedObjectPath)> = user
        .get_property("Sessions")
        .await
        .map_err(|e| e.to_string())?;
    let mut local_active = false;
    for (_, path) in sessions {
        let session = zbus::Proxy::new(
            &bus,
            "org.freedesktop.login1",
            path,
            "org.freedesktop.login1.Session",
        )
        .await
        .map_err(|e| e.to_string())?;
        let active: bool = session
            .get_property("Active")
            .await
            .map_err(|e| e.to_string())?;
        let remote: bool = session
            .get_property("Remote")
            .await
            .map_err(|e| e.to_string())?;
        local_active |= active && !remote;
    }
    if !local_active {
        return Err("An active local desktop session is required".into());
    }
    let process = format!("{pid},{start},{uid}");
    run_command(
        "/usr/bin/pkcheck",
        &[
            "--action-id",
            "io.github.marius4lui.LinuxDrop.manage-radio",
            "--process",
            &process,
            "--allow-user-interaction",
        ],
        60,
    )
    .await?;
    Ok(())
}

async fn join_p2p(
    shared: &Shared,
    lease_id: String,
    peer_name: String,
    pin: String,
    frequency: u32,
) -> Result<Response, String> {
    // Inventory and supplicant/DHCP I/O may take seconds. Other leased radios
    // and read-only status requests must remain serviceable throughout.
    let inv = inventory().await;
    let mut state = shared.lock().await;
    let mut lease = state
        .leases
        .get(&lease_id)
        .ok_or("lease not found")?
        .clone();
    if state.cancelled_p2p.contains(&lease_id) {
        return Err("P2P operation cancelled".into());
    }
    if lease.kind != LeaseKind::DirectWifi
        || lease.p2p_group.is_some()
        || state.pending_p2p.contains_key(&lease_id)
    {
        return Err("a free direct Wi-Fi lease is required".into());
    }
    let radio = inv
        .radios
        .iter()
        .find(|radio| radio.phy == lease.phy)
        .ok_or("radio disappeared")?;
    if radio.protected || radio.rfkill {
        return Err("radio became active or blocked".into());
    }
    if frequency != 0
        && !radio.channels.iter().any(|channel| {
            channel.frequency_mhz == frequency
                && !channel.disabled
                && !channel.no_ir
                && !channel.radar
        })
    {
        return Err("P2P frequency is not permitted by the radio regulatory policy".into());
    }
    let cancel = tokio_util::sync::CancellationToken::new();
    state.pending_p2p.insert(
        lease_id.clone(),
        PendingP2p {
            cancel: cancel.clone(),
            interfaces: inv
                .interfaces
                .iter()
                .filter(|i| i.phy.as_deref() == Some(&lease.phy))
                .map(|i| i.name.clone())
                .collect(),
        },
    );
    let operation = P2pOperation {
        shared: shared.clone(),
        lease_id: lease_id.clone(),
        cancel: cancel.clone(),
    };
    drop(state);
    let group = linuxdrop_network::p2p::connect_wps(
        &lease.interface,
        &peer_name,
        &pin,
        frequency,
        cancel.clone(),
    )
    .await
    .map_err(|e| e.to_string())?;
    let interface = group.identity.interface.clone();
    run_command(
        "/usr/sbin/ip",
        &[
            "link",
            "set",
            "dev",
            &interface,
            "alias",
            &format!("linuxdrop:p2p:{}", lease.id),
        ],
        5,
    )
    .await?;
    let mut state = shared.lock().await;
    if cancel.is_cancelled() || !state.leases.contains_key(&lease_id) {
        return Err("P2P radio lease ended during group formation".into());
    }
    lease.p2p_group = Some(group.identity.clone());
    state.leases.insert(lease_id.clone(), lease.clone());
    persist(&state).map_err(|e| e.to_string())?;
    group.into_journaled();
    drop(state);
    let network = tokio::select! {
        result = start_p2p_network(&lease) => result,
        _ = cancel.cancelled() => Err("P2P radio lease ended during address acquisition".into()),
    };
    let mut state = shared.lock().await;
    let lease_exists = state.leases.contains_key(&lease_id);
    let still_owned = lease_exists && !cancel.is_cancelled();
    match network {
        Ok((child, addresses)) if still_owned => {
            if let Some(child) = child {
                state.children.insert(lease_id.clone(), child);
            }
            state.pending_p2p.remove(&lease_id);
            drop(state);
            drop(operation);
            Ok(Response::P2pJoined {
                interface,
                ipv4_address: addresses.ipv4,
                ipv6_address: addresses.ipv6,
            })
        }
        result => {
            // A successful DHCP child is kill-on-drop if the radio lease ended.
            let error = result
                .err()
                .unwrap_or_else(|| "P2P radio lease ended during address acquisition".into());
            state.pending_p2p.remove(&lease_id);
            drop(state);
            if lease_exists && restore_p2p(&lease).await.is_ok() {
                let mut state = shared.lock().await;
                if let Some(current) = state.leases.get_mut(&lease_id) {
                    current.p2p_group = None;
                }
                persist(&state).map_err(|e| e.to_string())?;
            }
            Err(error)
        }
    }
}

async fn apply(
    request: Request,
    uid: u32,
    owned: &mut Vec<String>,
    shared: &Shared,
) -> Result<Response, String> {
    if let Request::Diagnose { radio_id, channel } = request {
        let acquired = Box::pin(apply(
            Request::Acquire {
                radio_id: radio_id.clone(),
                channel,
            },
            uid,
            owned,
            shared,
        ))
        .await;
        let mut report = DiagnosticReport {
            radio_id,
            steps: Vec::new(),
            restored: true,
            transmitted_frames: 0,
        };
        match acquired {
            Ok(Response::Acquired { lease }) => {
                report.steps.push(DiagnosticStep { name: "monitor_interface_and_channel".into(), passed: true, detail: "Dedicated temporary monitor created on an allowed channel; original interfaces unchanged".into() });
                let raw = probe_raw_socket(&lease.interface);
                report.steps.push(DiagnosticStep { name: "raw_socket".into(), passed: raw.is_ok(), detail: raw.err().unwrap_or_else(|| "AF_PACKET socket opened and bound; no frame transmitted, injection and AWDL remain unverified".into()) });
                let released = Box::pin(apply(
                    Request::Release { lease_id: lease.id },
                    uid,
                    owned,
                    shared,
                ))
                .await;
                report.restored = released.is_ok();
                report.steps.push(DiagnosticStep {
                    name: "restore".into(),
                    passed: report.restored,
                    detail: released.err().unwrap_or_else(|| {
                        "Temporary interface removed and lease journal cleared".into()
                    }),
                });
            }
            Err(error) => {
                report.restored = !shared
                    .lock()
                    .await
                    .leases
                    .values()
                    .any(|l| l.uid == uid && !owned.contains(&l.id));
                report.steps.push(DiagnosticStep {
                    name: "prepare".into(),
                    passed: false,
                    detail: error,
                });
            }
            _ => return Err("unexpected diagnostic lease response".into()),
        }
        return Ok(Response::Diagnostic { report });
    }
    if let Request::JoinP2p {
        lease_id,
        peer_name,
        pin,
        frequency,
    } = request
    {
        if !owned.contains(&lease_id) {
            return Err("lease does not belong to this connection".into());
        }
        return join_p2p(shared, lease_id, peer_name, pin, frequency).await;
    }
    let mut state = shared.lock().await;
    let awdl = matches!(request, Request::AcquireAwdl { .. });
    match request {
        Request::Diagnose { .. } => unreachable!(),
        Request::JoinP2p { .. } => unreachable!(),
        Request::CancelP2p { lease_id } => {
            // The original connection is busy awaiting JoinP2p. A separately
            // authorized connection of the same uid may only cancel its pending
            // operation; it cannot take over or mutate another user's lease.
            let lease = state.leases.get(&lease_id).ok_or("lease not found")?;
            if lease.uid != uid
                || lease.kind != LeaseKind::DirectWifi
                || !state.attached.contains(&lease_id)
            {
                return Err("P2P operation does not belong to this user".into());
            }
            if let Some(operation) = state.pending_p2p.get(&lease_id) {
                operation.cancel.cancel();
            }
            // Also cover cancellation while JoinP2p is collecting inventory,
            // before a supplicant operation has been registered.
            state.cancelled_p2p.insert(lease_id);
            Ok(Response::Ok)
        }
        Request::LeaveP2p { lease_id } => {
            if !owned.contains(&lease_id) {
                return Err("lease does not belong to this connection".into());
            }
            let mut lease = state
                .leases
                .get(&lease_id)
                .ok_or("lease not found")?
                .clone();
            if lease.kind != LeaseKind::DirectWifi {
                return Err("direct Wi-Fi lease required".into());
            }
            stop_child(&mut state, &lease_id).await;
            restore_p2p(&lease).await?;
            lease.p2p_group = None;
            state.cancelled_p2p.remove(&lease_id);
            state.leases.insert(lease_id, lease);
            persist(&state).map_err(|e| e.to_string())?;
            Ok(Response::Ok)
        }
        Request::RecoveryStatus | Request::RetryRecovery => {
            if matches!(request, Request::RetryRecovery) {
                let abandoned: Vec<_> = state
                    .leases
                    .values()
                    .filter(|l| l.uid == uid && !state.attached.contains(&l.id))
                    .cloned()
                    .collect();
                for lease in abandoned {
                    match restore(&lease).await {
                        Ok(()) => {
                            state.leases.remove(&lease.id);
                        }
                        Err(error) => state
                            .recovery_errors
                            .push(format!("{}: {error}", lease.interface)),
                    }
                }
                persist(&state).map_err(|e| e.to_string())?;
            }
            let issues = state
                .leases
                .values()
                .filter(|l| l.uid == uid && !state.attached.contains(&l.id))
                .map(|l| {
                    let check = if l.kind == LeaseKind::DirectWifi {
                        Ok(())
                    } else {
                        verify_owned(l)
                    };
                    RecoveryIssue {
                        lease_id: l.id.clone(),
                        interface: l.interface.clone(),
                        ownership_verified: check.is_ok(),
                        detail: check.err().unwrap_or_else(|| {
                            "Orphaned lease; authorized retry restores only this owned resource"
                                .into()
                        }),
                    }
                })
                .collect();
            Ok(Response::Recovery {
                issues,
                recent_errors: state
                    .recovery_errors
                    .iter()
                    .rev()
                    .take(32)
                    .cloned()
                    .collect(),
            })
        }
        Request::Reserve { radio_id } => {
            if owned.len() >= 2 {
                return Err("two radio leases per client maximum".into());
            }
            let inv = inventory().await;
            let radio = inv
                .radios
                .iter()
                .find(|r| r.id == radio_id)
                .ok_or("radio not found")?;
            if radio.protected || radio.rfkill || radio.driver.is_none() {
                return Err("radio is active, blocked, or has no driver".into());
            }
            if state.leases.values().any(|l| l.phy == radio.phy) {
                return Err("radio is already leased".into());
            }
            if !radio.modes.iter().any(|m| m == "managed") {
                return Err("managed station mode is unavailable".into());
            }
            let interface = inv
                .interfaces
                .iter()
                .find(|i| i.phy.as_deref() == Some(&radio.phy) && !i.in_use())
                .ok_or("no idle interface on selected radio")?;
            let id = uuid::Uuid::new_v4().simple().to_string();
            let lease = Lease {
                id: id.clone(),
                uid,
                phy: radio.phy.clone(),
                interface: interface.name.clone(),
                channel: 0,
                boot_id: std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
                    .map_err(|e| e.to_string())?
                    .trim()
                    .into(),
                awdl_interface: None,
                allowed_frequencies: vec![],
                kind: LeaseKind::DirectWifi,
                connection_uuid: Some(uuid::Uuid::new_v4().to_string()),
                p2p_group: None,
            };
            state.leases.insert(id.clone(), lease.clone());
            if let Err(e) = persist(&state) {
                state.leases.remove(&id);
                return Err(e.to_string());
            }
            state.attached.insert(id.clone());
            owned.push(id);
            Ok(Response::Acquired {
                lease: Box::new(lease),
            })
        }
        Request::Status => Ok(Response::State {
            leases: state
                .leases
                .values()
                .filter(|l| l.uid == uid)
                .cloned()
                .collect(),
            recovery_errors: state.recovery_errors.clone(),
        }),
        Request::Acquire { radio_id, channel } | Request::AcquireAwdl { radio_id, channel } => {
            if owned.len() >= 2 {
                return Err("two radio leases per client maximum".into());
            }
            let inventory = inventory().await;
            let radio = inventory
                .radios
                .iter()
                .find(|r| r.id == radio_id)
                .ok_or("radio not found")?;
            if !radio.phy.starts_with("phy") || !radio.phy[3..].chars().all(|c| c.is_ascii_digit())
            {
                return Err("invalid kernel radio name".into());
            }
            let leased = inventory
                .radios
                .iter()
                .filter(|r| state.leases.values().any(|l| l.phy == r.phy))
                .map(|r| r.id.clone())
                .collect();
            let decision = select_radio(
                &inventory,
                &SelectionRequest {
                    preferred: Some(radio_id.clone()),
                    leased,
                    require_tested_awdl: false,
                    channel: Some(channel),
                    prefer_usb: true,
                },
            );
            let candidate = decision
                .candidates
                .iter()
                .find(|c| c.id == radio_id)
                .ok_or("radio disappeared")?;
            if !candidate.exclusions.is_empty() {
                return Err(candidate.exclusions.join("; "));
            }
            let id = uuid::Uuid::new_v4().simple().to_string();
            let lease = Lease {
                id: id.clone(),
                uid,
                phy: radio.phy.clone(),
                interface: format!("ld{}", &id[..10]),
                channel,
                boot_id: std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
                    .map_err(|e| e.to_string())?
                    .trim()
                    .into(),
                awdl_interface: awdl.then(|| format!("la{}", &id[..10])),
                kind: LeaseKind::Monitor,
                connection_uuid: None,
                p2p_group: None,
                allowed_frequencies: radio
                    .channels
                    .iter()
                    .filter(|c| !c.disabled && !c.no_ir && !c.radar)
                    .map(|c| c.frequency_mhz)
                    .collect(),
            };
            state.leases.insert(id.clone(), lease.clone());
            if let Err(e) = persist(&state) {
                state.leases.remove(&id);
                return Err(e.to_string());
            }
            // Original interfaces and NetworkManager properties remain untouched.
            let created = async {
                run_command(
                    "/usr/sbin/iw",
                    &[
                        "phy",
                        &lease.phy,
                        "interface",
                        "add",
                        &lease.interface,
                        "type",
                        "monitor",
                    ],
                    5,
                )
                .await?;
                run_command(
                    "/usr/sbin/ip",
                    &[
                        "link",
                        "set",
                        "dev",
                        &lease.interface,
                        "alias",
                        &format!("linuxdrop:{}", lease.id),
                    ],
                    5,
                )
                .await?;
                // Recheck after VIF creation and before the first channel change:
                // a NetworkManager activation may have raced the initial scan.
                let current = linuxdrop_hardware::inventory().await;
                if current.interfaces.iter().any(|i| {
                    i.phy.as_deref() == Some(&lease.phy) && i.name != lease.interface && i.in_use()
                }) {
                    return Err("radio became active while preparing its lease".into());
                }
                run_command(
                    "/usr/sbin/iw",
                    &[
                        "dev",
                        &lease.interface,
                        "set",
                        "channel",
                        &channel.to_string(),
                    ],
                    5,
                )
                .await?;
                run_command(
                    "/usr/sbin/ip",
                    &["link", "set", "dev", &lease.interface, "up"],
                    5,
                )
                .await
            }
            .await;
            if let Err(error) = created {
                match restore(&lease).await {
                    Ok(()) => {
                        state.leases.remove(&id);
                    }
                    Err(e) => state.recovery_errors.push(e),
                }
                persist(&state).map_err(|e| e.to_string())?;
                return Err(error);
            }
            if let Some(tap) = &lease.awdl_interface {
                let allowed = radio
                    .channels
                    .iter()
                    .filter(|c| !c.disabled && !c.no_ir && !c.radar)
                    .map(|c| c.frequency_mhz.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                let child = Command::new("/usr/libexec/linuxdrop/filin")
                    .args([
                        "-i",
                        &lease.interface,
                        "-h",
                        tap,
                        "-c",
                        &channel.to_string(),
                        "-N",
                        "--no-http",
                        "--no-force-master",
                    ])
                    .env_clear()
                    .env("PATH", "/usr/sbin:/usr/bin")
                    .env("LC_ALL", "C")
                    .env("LINUXDROP_ALLOWED_FREQUENCIES", allowed)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .kill_on_drop(true)
                    .spawn();
                match child {
                    Err(e) => {
                        match restore(&lease).await {
                            Ok(()) => {
                                state.leases.remove(&id);
                            }
                            Err(error) => state.recovery_errors.push(error),
                        }
                        persist(&state).map_err(|e| e.to_string())?;
                        return Err(format!("AWDL helper unavailable: {e}"));
                    }
                    Ok(child) => {
                        state.children.insert(id.clone(), child);
                    }
                }
                let mut ready = false;
                for _ in 0..30 {
                    if !matches!(state.children.get_mut(&id).unwrap().try_wait(), Ok(None)) {
                        break;
                    }
                    if Path::new("/sys/class/net").join(tap).exists() {
                        ready = true;
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                if !ready {
                    stop_child(&mut state, &id).await;
                    if restore(&lease).await.is_ok() {
                        state.leases.remove(&id);
                    }
                    persist(&state).map_err(|e| e.to_string())?;
                    return Err("AWDL helper exited or did not create its interface".into());
                }
            }
            state.attached.insert(id.clone());
            owned.push(id);
            Ok(Response::Acquired {
                lease: Box::new(lease),
            })
        }
        Request::SetChannel { lease_id, channel } => {
            if !owned.contains(&lease_id) {
                return Err("lease does not belong to this connection".into());
            }
            let lease = state
                .leases
                .get(&lease_id)
                .ok_or("lease not found")?
                .clone();
            if lease.kind == LeaseKind::DirectWifi {
                return Err("direct Wi-Fi reservation cannot change monitor channels".into());
            }
            if lease.awdl_interface.is_some() {
                return Err("AWDL channel scheduling belongs to the link helper".into());
            }
            let inventory = inventory().await;
            let radio = inventory
                .radios
                .iter()
                .find(|r| r.phy == lease.phy)
                .ok_or("radio unplugged")?;
            // Our monitor is up by design. Any other newly active interface blocks mutation.
            if inventory.interfaces.iter().any(|i| {
                i.phy.as_deref() == Some(&lease.phy) && i.name != lease.interface && i.in_use()
            }) {
                return Err("radio became active outside the lease".into());
            }
            if !radio
                .channels
                .iter()
                .any(|c| c.number == channel && !c.disabled && !c.no_ir && !c.radar)
            {
                return Err("channel is restricted or unavailable".into());
            }
            verify_owned(&lease)?;
            run_command(
                "/usr/sbin/iw",
                &[
                    "dev",
                    &lease.interface,
                    "set",
                    "channel",
                    &channel.to_string(),
                ],
                5,
            )
            .await?;
            state.leases.get_mut(&lease_id).unwrap().channel = channel;
            persist(&state).map_err(|e| e.to_string())?;
            Ok(Response::Ok)
        }
        Request::Release { lease_id } => {
            if !owned.contains(&lease_id) {
                return Err("lease does not belong to this connection".into());
            }
            let lease = state
                .leases
                .get(&lease_id)
                .ok_or("lease not found")?
                .clone();
            stop_child(&mut state, &lease_id).await;
            restore(&lease).await?;
            state.leases.remove(&lease_id);
            state.attached.remove(&lease_id);
            state.cancelled_p2p.remove(&lease_id);
            owned.retain(|id| id != &lease_id);
            persist(&state).map_err(|e| e.to_string())?;
            Ok(Response::Ok)
        }
    }
}

async fn stop_child(state: &mut State, id: &str) {
    if let Some(mut child) = state.children.remove(id) {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}

fn verify_owned(lease: &Lease) -> Result<(), String> {
    let path = Path::new("/sys/class/net").join(&lease.interface);
    let alias = std::fs::read_to_string(path.join("ifalias")).map_err(|e| e.to_string())?;
    let phy = std::fs::canonicalize(path.join("phy80211")).map_err(|e| e.to_string())?;
    if alias.trim() != format!("linuxdrop:{}", lease.id)
        || phy.file_name().and_then(|s| s.to_str()) != Some(&lease.phy)
    {
        return Err("interface ownership could not be verified; manual recovery required".into());
    }
    Ok(())
}
async fn restore(lease: &Lease) -> Result<(), String> {
    if lease.kind == LeaseKind::DirectWifi {
        restore_p2p(lease).await?;
        return timeout(Duration::from_secs(10), restore_connection(lease))
            .await
            .map_err(|_| "direct connection recovery timed out".to_owned())?;
    }
    let boot =
        std::fs::read_to_string("/proc/sys/kernel/random/boot_id").map_err(|e| e.to_string())?;
    if boot.trim() != lease.boot_id || !Path::new("/sys/class/net").join(&lease.interface).exists()
    {
        return Ok(());
    }
    verify_owned(lease)?;
    run_command("/usr/sbin/iw", &["dev", &lease.interface, "del"], 5).await
}

async fn restore_p2p(lease: &Lease) -> Result<(), String> {
    let Some(group) = &lease.p2p_group else {
        return Ok(());
    };
    let boot =
        std::fs::read_to_string("/proc/sys/kernel/random/boot_id").map_err(|e| e.to_string())?;
    let path = Path::new("/sys/class/net").join(&group.interface);
    if boot.trim() == lease.boot_id && path.exists() {
        let alias = std::fs::read_to_string(path.join("ifalias")).map_err(|e| e.to_string())?;
        if alias.trim() != format!("linuxdrop:p2p:{}", lease.id) {
            return Err("P2P ownership marker changed; manual recovery required".into());
        }
        let connection = zbus::Connection::system()
            .await
            .map_err(|e| e.to_string())?;
        timeout(
            Duration::from_secs(10),
            linuxdrop_network::p2p::disconnect(&connection, group),
        )
        .await
        .map_err(|_| "P2P disconnect timed out".to_owned())?
        .map_err(|e| e.to_string())?;
    }
    let _ = std::fs::remove_file(format!("/run/linuxdrop/{}.ipv4.json", lease.id));
    Ok(())
}

#[derive(Debug, Default, PartialEq)]
struct P2pAddresses {
    ipv4: Option<std::net::Ipv4Addr>,
    ipv6: Option<std::net::Ipv6Addr>,
}

fn parse_p2p_addresses(
    bytes: &[u8],
    interface: &str,
    lease_id: &str,
) -> Result<P2pAddresses, String> {
    let links: Vec<serde_json::Value> = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let link = links
        .iter()
        .find(|link| link["ifname"] == interface)
        .ok_or("P2P group interface disappeared")?;
    if link["ifalias"] != format!("linuxdrop:p2p:{lease_id}") {
        return Err("P2P interface ownership changed".into());
    }
    let mut addresses = P2pAddresses::default();
    if !link["flags"]
        .as_array()
        .is_some_and(|flags| flags.iter().any(|f| f == "UP"))
    {
        return Ok(addresses);
    }
    for address in link["addr_info"].as_array().into_iter().flatten() {
        // Do not report an IPv6 address until duplicate-address detection has
        // finished. Expired and failed addresses cannot make a group ready.
        if address["tentative"] == true
            || address["dadfailed"] == true
            || address["valid_life_time"] == 0
        {
            continue;
        }
        let Some(local) = address["local"].as_str() else {
            continue;
        };
        match local.parse::<std::net::IpAddr>() {
            Ok(std::net::IpAddr::V4(ip))
                if address["family"] == "inet"
                    && !ip.is_unspecified()
                    && !ip.is_loopback()
                    && !ip.is_multicast()
                    && !ip.is_broadcast() =>
            {
                addresses.ipv4 = Some(ip)
            }
            Ok(std::net::IpAddr::V6(ip))
                if address["family"] == "inet6" && ip.is_unicast_link_local() =>
            {
                addresses.ipv6 = Some(ip)
            }
            _ => {}
        }
    }
    Ok(addresses)
}

async fn p2p_addresses(lease: &Lease) -> Result<P2pAddresses, String> {
    let group = lease.p2p_group.as_ref().ok_or("P2P group missing")?;
    let output = timeout(
        Duration::from_secs(2),
        Command::new("/usr/sbin/ip")
            .args(["-d", "-j", "address", "show", "dev", &group.interface])
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin")
            .env("LC_ALL", "C")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "P2P address inspection timed out")?
    .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("P2P address inspection failed".into());
    }
    parse_p2p_addresses(&output.stdout, &group.interface, &lease.id)
}

async fn start_p2p_network(
    lease: &Lease,
) -> Result<(Option<tokio::process::Child>, P2pAddresses), String> {
    let group = lease.p2p_group.as_ref().ok_or("P2P group missing")?;
    // Verify ownership before starting any DHCP traffic on this interface.
    let mut addresses = p2p_addresses(lease).await?;
    let result_path = format!("/run/linuxdrop/{}.ipv4.json", lease.id);
    let _ = std::fs::remove_file(&result_path);
    let mut child = Command::new("/usr/bin/busybox")
        .args([
            "udhcpc",
            "-f",
            "-t",
            "3",
            "-T",
            "3",
            "-A",
            "3",
            "-i",
            &group.interface,
            "-s",
            "/usr/libexec/linuxdrop/p2p-dhcp",
        ])
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin")
        .env("LINUXDROP_P2P_INTERFACE", &group.interface)
        .env("LINUXDROP_LEASE_ID", &lease.id)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok();
    // Keep renewing/acquiring IPv4 while the lease is owned, but never require
    // DHCP when the newly created, ownership-marked group already has IPv6.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if child
            .as_mut()
            .is_some_and(|child| !matches!(child.try_wait(), Ok(None)))
        {
            child = None;
        }
        if addresses.ipv6.is_some() || (addresses.ipv4.is_some() && child.is_some()) {
            return Ok((child, addresses));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("P2P group received neither usable IPv4 nor IPv6".into());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        addresses = p2p_addresses(lease).await?;
    }
}

fn persist(state: &State) -> io::Result<()> {
    use std::io::Write;
    let data = serde_json::to_vec(&state.leases.values().collect::<Vec<_>>())?;
    let tmp = format!("{JOURNAL}.tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(&data)?;
    file.sync_all()?;
    std::fs::rename(tmp, JOURNAL)?;
    std::fs::File::open(Path::new(JOURNAL).parent().unwrap())?.sync_all()
}
async fn run_command(program: &str, args: &[&str], seconds: u64) -> Result<(), String> {
    let output = timeout(
        Duration::from_secs(seconds),
        Command::new(program)
            .args(args)
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin")
            .env("LC_ALL", "C")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| format!("{program} timed out"))?
    .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{program}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn probe_raw_socket(interface: &str) -> Result<(), String> {
    use std::os::fd::{FromRawFd, OwnedFd};
    let name = std::ffi::CString::new(interface).map_err(|e| e.to_string())?;
    // SAFETY: valid NUL-terminated interface name, initialized sockaddr_ll and
    // RAII-owned descriptor. Protocol zero receives/transmits no packets.
    unsafe {
        let index = libc::if_nametoindex(name.as_ptr());
        if index == 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        let fd = libc::socket(libc::AF_PACKET, libc::SOCK_RAW | libc::SOCK_CLOEXEC, 0);
        if fd < 0 {
            return Err(io::Error::last_os_error().to_string());
        }
        let _socket = OwnedFd::from_raw_fd(fd);
        let mut address: libc::sockaddr_ll = std::mem::zeroed();
        address.sll_family = libc::AF_PACKET as u16;
        address.sll_ifindex = index as i32;
        if libc::bind(
            fd,
            &address as *const _ as *const libc::sockaddr,
            std::mem::size_of_val(&address) as u32,
        ) != 0
        {
            return Err(io::Error::last_os_error().to_string());
        }
    }
    Ok(())
}

async fn restore_connection(lease: &Lease) -> Result<(), String> {
    let Some(uuid) = &lease.connection_uuid else {
        return Ok(());
    };
    linuxdrop_network::nm::remove_owned(&linuxdrop_network::nm::Lease {
        interface: lease.interface.clone(),
        lease_id: lease.id.clone(),
        connection_uuid: uuid.clone(),
    })
    .await
    .map_err(|error| error.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn direct_lease() -> Lease {
        Lease {
            id: "a".repeat(32),
            uid: 1000,
            phy: "phy2".into(),
            interface: "wlan2".into(),
            channel: 0,
            boot_id: "test".into(),
            awdl_interface: None,
            allowed_frequencies: vec![],
            kind: LeaseKind::DirectWifi,
            connection_uuid: Some("owned-uuid".into()),
            p2p_group: None,
        }
    }
    fn active_interface(name: &str) -> linuxdrop_hardware::NetworkInterface {
        linuxdrop_hardware::NetworkInterface {
            name: name.into(),
            ifindex: 3,
            phy: Some("phy2".into()),
            state: "up".into(),
            default_route: false,
            nm_state: None,
            nm_managed: None,
            active_connection: None,
            active_connection_uuid: None,
        }
    }
    #[test]
    fn watchdog_distinguishes_owned_p2p_from_competing_networks() {
        let mut lease = direct_lease();
        let pending = PendingP2p {
            cancel: tokio_util::sync::CancellationToken::new(),
            interfaces: ["wlan2".into(), "sibling0".into()].into_iter().collect(),
        };
        let mut group = active_interface("p2p-wlan2-0");
        assert!(competing_use(&lease, &group, None));
        assert!(!competing_use(&lease, &group, Some(&pending)));
        assert!(competing_use(
            &lease,
            &active_interface("wlan2"),
            Some(&pending)
        ));
        assert!(competing_use(
            &lease,
            &active_interface("sibling0"),
            Some(&pending)
        ));
        lease.p2p_group = Some(linuxdrop_network::p2p::GroupIdentity {
            interface: group.name.clone(),
            interface_object: "/test".into(),
            group_object: "/group".into(),
            parent_interface: "wlan2".into(),
            peer_object: "/peer".into(),
        });
        assert!(!competing_use(&lease, &group, None));
        group.default_route = true;
        assert!(competing_use(&lease, &group, None));
        group.default_route = false;
        group.active_connection_uuid = Some("somebody-elses-network".into());
        assert!(competing_use(&lease, &group, Some(&pending)));
        let mut parent = active_interface("wlan2");
        parent.active_connection_uuid = lease.connection_uuid.clone();
        assert!(!competing_use(&lease, &parent, None));
    }
    #[tokio::test]
    async fn abandoned_p2p_operation_cancels_without_removing_a_new_operation() {
        let shared = Arc::new(Mutex::new(State::default()));
        let cancel = tokio_util::sync::CancellationToken::new();
        let old = P2pOperation {
            shared: shared.clone(),
            lease_id: "lease".into(),
            cancel: cancel.clone(),
        };
        let fresh = tokio_util::sync::CancellationToken::new();
        shared.lock().await.pending_p2p.insert(
            "lease".into(),
            PendingP2p {
                cancel: fresh.clone(),
                interfaces: Default::default(),
            },
        );
        drop(old);
        tokio::task::yield_now().await;
        assert!(cancel.is_cancelled());
        assert!(!fresh.is_cancelled());
        assert!(shared.lock().await.pending_p2p.contains_key("lease"));
    }
    #[tokio::test]
    async fn pending_p2p_can_be_cancelled_only_by_its_authorized_user() {
        let lease = direct_lease();
        let cancel = tokio_util::sync::CancellationToken::new();
        let shared = Arc::new(Mutex::new(State::default()));
        {
            let mut state = shared.lock().await;
            state.attached.insert(lease.id.clone());
            state.leases.insert(lease.id.clone(), lease.clone());
            state.pending_p2p.insert(
                lease.id.clone(),
                PendingP2p {
                    cancel: cancel.clone(),
                    interfaces: Default::default(),
                },
            );
        }
        let request = Request::CancelP2p {
            lease_id: lease.id.clone(),
        };
        assert!(apply(request.clone(), 2000, &mut vec![], &shared)
            .await
            .is_err());
        assert!(!cancel.is_cancelled());
        assert!(matches!(
            apply(request, 1000, &mut vec![], &shared).await,
            Ok(Response::Ok)
        ));
        assert!(cancel.is_cancelled());
        assert!(shared.lock().await.cancelled_p2p.contains(&lease.id));
        assert!(matches!(
            apply(Request::Status, 1000, &mut vec![], &shared).await,
            Ok(Response::State { .. })
        ));
    }
    #[test]
    fn p2p_readiness_requires_owned_usable_addresses() {
        let mut link = serde_json::json!([{
            "ifname": "p2p-test0", "ifalias": "linuxdrop:p2p:owned", "flags": ["UP"],
            "addr_info": [{"family":"inet6", "local":"fe80::1234", "tentative": true}]
        }]);
        let parse = |v: &serde_json::Value| {
            parse_p2p_addresses(&serde_json::to_vec(v).unwrap(), "p2p-test0", "owned")
        };
        assert_eq!(parse(&link).unwrap(), P2pAddresses::default());
        link[0]["addr_info"][0]["tentative"] = false.into();
        assert_eq!(
            parse(&link).unwrap().ipv6,
            Some("fe80::1234".parse().unwrap())
        );
        for flag in ["dadfailed", "tentative"] {
            link[0]["addr_info"][0][flag] = true.into();
            assert!(parse(&link).unwrap().ipv6.is_none());
            link[0]["addr_info"][0][flag] = false.into();
        }
        link[0]["addr_info"][0]["valid_life_time"] = 0.into();
        assert!(parse(&link).unwrap().ipv6.is_none());
        link[0]["addr_info"][0]["valid_life_time"] = 100.into();
        link[0]["flags"] = serde_json::json!([]);
        assert_eq!(parse(&link).unwrap(), P2pAddresses::default());
        link[0]["flags"] = serde_json::json!(["UP"]);
        link[0]["addr_info"] = serde_json::json!([
            {"family":"inet", "local":"255.255.255.255"},
            {"family":"inet", "local":"127.0.0.1"},
            {"family":"inet6", "local":"ff02::1"},
            {"family":"inet6", "local":"fd00::1"}
        ]);
        assert_eq!(parse(&link).unwrap(), P2pAddresses::default());
        link[0]["addr_info"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"family":"inet","local":"192.168.49.2"}));
        assert_eq!(
            parse(&link).unwrap().ipv4,
            Some("192.168.49.2".parse().unwrap())
        );
        link[0]["ifalias"] = "somebody-else".into();
        assert!(parse(&link).is_err());
        link[0]["ifname"] = "other0".into();
        assert!(parse(&link).is_err());
    }

    #[tokio::test]
    #[ignore = "requires a private network namespace; use run-p2p-addresses.sh"]
    async fn ipv6_only_p2p_group_is_ready_without_dhcp() {
        assert_eq!(
            std::env::var("LINUXDROP_TEST_PRIVATE_P2P").as_deref(),
            Ok("1")
        );
        assert_ne!(
            std::fs::read_link("/proc/self/ns/net").unwrap(),
            std::fs::read_link("/proc/1/ns/net").unwrap()
        );
        let mut lease = direct_lease();
        lease.id = uuid::Uuid::new_v4().simple().to_string();
        let interface = "ld-p2p0";
        lease.p2p_group = Some(linuxdrop_network::p2p::GroupIdentity {
            interface: interface.into(),
            interface_object: "/test".into(),
            group_object: "/group".into(),
            parent_interface: "wlan2".into(),
            peer_object: "/peer".into(),
        });
        run_command(
            "/usr/sbin/ip",
            &["link", "add", interface, "type", "dummy"],
            3,
        )
        .await
        .unwrap();
        run_command(
            "/usr/sbin/ip",
            &[
                "link",
                "set",
                "dev",
                interface,
                "alias",
                &format!("linuxdrop:p2p:{}", lease.id),
                "up",
            ],
            3,
        )
        .await
        .unwrap();
        run_command(
            "/usr/sbin/ip",
            &[
                "-6",
                "address",
                "replace",
                "fe80::1234/64",
                "dev",
                interface,
                "nodad",
            ],
            3,
        )
        .await
        .unwrap();
        let (child, addresses) = timeout(Duration::from_secs(3), start_p2p_network(&lease))
            .await
            .unwrap()
            .unwrap();
        assert!(addresses.ipv4.is_none());
        assert!(addresses.ipv6.is_some());
        if let Some(mut child) = child {
            child.kill().await.unwrap();
            child.wait().await.unwrap();
        }
        run_command(
            "/usr/sbin/ip",
            &["link", "set", "dev", interface, "alias", "foreign"],
            3,
        )
        .await
        .unwrap();
        assert!(p2p_addresses(&lease).await.is_err());
        assert!(start_p2p_network(&lease).await.is_err());
        run_command("/usr/sbin/ip", &["link", "del", interface], 3)
            .await
            .unwrap();
    }

    #[test]
    fn process_start_is_available() {
        assert!(process_start(std::process::id() as i32).unwrap() > 0);
    }
    #[test]
    fn protocol_rejects_unrecognized_mutations() {
        assert!(
            serde_json::from_str::<Request>(r#"{"operation":"execute","command":"rm"}"#).is_err()
        );
        assert!(serde_json::from_str::<Request>(
            r#"{"operation":"acquire","radio_id":"x","channel":6,"command":"oops"}"#
        )
        .is_err());
    }
}
