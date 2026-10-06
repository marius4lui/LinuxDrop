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
}
type Shared = Arc<Mutex<State>>;

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
                let unsafe_use = inventory.interfaces.iter().any(|i| {
                    i.phy.as_deref() == Some(&lease.phy) && i.in_use() && if lease.kind == LeaseKind::DirectWifi { i.active_connection_uuid.as_ref() != lease.connection_uuid.as_ref() } else { i.name != lease.interface }
                });
                if dead || radio_gone || unsafe_use || regulatory_change {
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
                    match apply(request, uid, &mut owned, &state).await {
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
    let mut state = shared.lock().await;
    let awdl = matches!(request, Request::AcquireAwdl { .. });
    match request {
        Request::Diagnose { .. } => unreachable!(),
        Request::JoinP2p {
            lease_id,
            peer_name,
            pin,
            frequency,
        } => {
            if !owned.contains(&lease_id) {
                return Err("lease does not belong to this connection".into());
            }
            let mut lease = state
                .leases
                .get(&lease_id)
                .ok_or("lease not found")?
                .clone();
            if lease.kind != LeaseKind::DirectWifi || lease.p2p_group.is_some() {
                return Err("a free direct Wi-Fi lease is required".into());
            }
            let inv = inventory().await;
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
            let group = linuxdrop_network::p2p::connect_wps(
                &lease.interface,
                &peer_name,
                &pin,
                frequency,
                tokio_util::sync::CancellationToken::new(),
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
            lease.p2p_group = Some(group.identity.clone());
            state.leases.insert(lease_id.clone(), lease.clone());
            persist(&state).map_err(|e| e.to_string())?;
            group.into_journaled();
            match start_p2p_dhcp(&lease).await {
                Ok((child, address)) => {
                    state.children.insert(lease_id, child);
                    Ok(Response::P2pJoined {
                        interface,
                        ipv4_address: address,
                    })
                }
                Err(error) => {
                    if restore_p2p(&lease).await.is_ok() {
                        lease.p2p_group = None;
                        state.leases.insert(lease_id, lease);
                        persist(&state).map_err(|e| e.to_string())?;
                    }
                    Err(error)
                }
            }
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

async fn start_p2p_dhcp(lease: &Lease) -> Result<(tokio::process::Child, String), String> {
    let group = lease.p2p_group.as_ref().ok_or("P2P group missing")?;
    let result_path = format!("/run/linuxdrop/{}.ipv4.json", lease.id);
    let _ = std::fs::remove_file(&result_path);
    let mut child = Command::new("/usr/sbin/udhcpc")
        .args([
            "-f",
            "-n",
            "-t",
            "3",
            "-T",
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
        .map_err(|e| e.to_string())?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(bytes) = tokio::fs::read(&result_path).await {
            let value: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            let address = value["ipv4_address"]
                .as_str()
                .ok_or("DHCP address missing")?;
            let parsed: std::net::Ipv4Addr = address.parse().map_err(|_| "DHCP address invalid")?;
            if value["interface"] != group.interface
                || parsed.is_unspecified()
                || parsed.is_loopback()
                || parsed.is_multicast()
            {
                return Err("DHCP result does not match the leased group".into());
            }
            return Ok((child, address.into()));
        }
        if !matches!(child.try_wait(), Ok(None)) {
            return Err("P2P DHCP client stopped before receiving an address".into());
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("P2P DHCP timed out".into());
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
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
    let connection = zbus::Connection::system()
        .await
        .map_err(|e| e.to_string())?;
    let settings = zbus::Proxy::new(
        &connection,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager/Settings",
        "org.freedesktop.NetworkManager.Settings",
    )
    .await
    .map_err(|e| e.to_string())?;
    let paths: Vec<zbus::zvariant::OwnedObjectPath> =
        match settings.call("ListConnections", &()).await {
            Ok(paths) => paths,
            Err(zbus::Error::MethodError(name, _, _))
                if name.as_str() == "org.freedesktop.DBus.Error.ServiceUnknown" =>
            {
                return Ok(())
            }
            Err(error) => return Err(error.to_string()),
        };
    for path in paths {
        let profile = zbus::Proxy::new(
            &connection,
            "org.freedesktop.NetworkManager",
            path,
            "org.freedesktop.NetworkManager.Settings.Connection",
        )
        .await
        .map_err(|e| e.to_string())?;
        let settings: HashMap<String, HashMap<String, zbus::zvariant::OwnedValue>> = profile
            .call("GetSettings", &())
            .await
            .map_err(|e| e.to_string())?;
        let Some(properties) = settings.get("connection") else {
            continue;
        };
        let text = |key: &str| properties.get(key).and_then(|v| <&str>::try_from(v).ok());
        if text("uuid") != Some(uuid) {
            continue;
        }
        if text("id") != Some(format!("linuxdrop-{}", lease.id).as_str()) {
            return Err(
                "direct connection ownership marker mismatch; retained for inspection".into(),
            );
        }
        let manager = zbus::Proxy::new(
            &connection,
            "org.freedesktop.NetworkManager",
            "/org/freedesktop/NetworkManager",
            "org.freedesktop.NetworkManager",
        )
        .await
        .map_err(|e| e.to_string())?;
        let active_paths: Vec<zbus::zvariant::OwnedObjectPath> = manager
            .get_property("ActiveConnections")
            .await
            .map_err(|e| e.to_string())?;
        for active_path in active_paths {
            let active = zbus::Proxy::new(
                &connection,
                "org.freedesktop.NetworkManager",
                active_path.clone(),
                "org.freedesktop.NetworkManager.Connection.Active",
            )
            .await
            .map_err(|e| e.to_string())?;
            let active_uuid: String = active
                .get_property("Uuid")
                .await
                .map_err(|e| e.to_string())?;
            if active_uuid == *uuid {
                let _: () = manager
                    .call("DeactivateConnection", &(active_path,))
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
        let _: () = profile
            .call("Delete", &())
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
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
