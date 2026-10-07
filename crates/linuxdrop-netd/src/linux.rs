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

// Compile-time packaging choice; a privileged helper never accepts its program
// directory from the runtime environment or an IPC request.
const HELPER_DIRECTORY: &str = match option_env!("LINUXDROP_LIBEXECDIR") {
    Some(directory) => directory,
    None => "/usr/libexec/linuxdrop",
};
#[path = "acquire.rs"]
mod acquire;
#[path = "channel.rs"]
mod channel;
#[path = "p2p_recovery.rs"]
mod p2p_recovery;

const JOURNAL: &str = "/var/lib/linuxdrop-netd/leases.json";
const MAX_REQUEST: u64 = 4096;
const P2P_UNCERTAIN: &str = "P2P formation outcome is unknown; radio remains reserved until verified recovery or a system reboot";
#[derive(Default)]
struct State {
    leases: HashMap<String, Lease>,
    children: HashMap<String, tokio::process::Child>,
    recovery_errors: Vec<String>,
    attached: std::collections::HashSet<String>,
    producers: HashMap<String, RadioProducer>,
    cancelled_p2p: std::collections::HashSet<String>,
    cleanups: HashMap<String, CleanupReceipt>,
    group_cleanups: HashMap<String, CleanupReceipt>,
    network_revision: u64,
}
impl State {
    fn radio_changed(&mut self) {
        self.network_revision = self.network_revision.wrapping_add(1);
    }
    fn observation_is_current(&self, revision: u64, id: &str) -> bool {
        self.network_revision == revision && self.attached.contains(id)
    }
    fn record_recovery_error(&mut self, error: String) {
        self.recovery_errors.push(error);
        let excess = self.recovery_errors.len().saturating_sub(128);
        self.recovery_errors.drain(..excess);
    }
}

type Shared = Arc<Mutex<State>>;

type CleanupReceipt = tokio::sync::watch::Receiver<Option<Result<(), String>>>;

async fn wait_cleanup(mut receipt: CleanupReceipt) -> Result<(), String> {
    loop {
        if let Some(result) = receipt.borrow().clone() {
            return result;
        }
        receipt
            .changed()
            .await
            .map_err(|_| "Cleanup worker stopped without a receipt".to_owned())?;
    }
}

async fn restore_child_and_lease(
    lease: Lease,
    child: Option<tokio::process::Child>,
) -> Result<(), String> {
    reap_child(child).await?;
    restore(&lease).await
}

async fn begin_cleanup(shared: &Shared, id: &str) -> CleanupReceipt {
    begin_cleanup_with(shared, id, restore_child_and_lease, persist).await
}

// One persistent worker owns each retiring lease. Client cancellation only drops
// a waiter, never the worker or the reservation. Tests inject delayed operations
// and journal failures here without touching a radio or the production journal.
async fn begin_cleanup_with<F, Fut, P>(
    shared: &Shared,
    id: &str,
    operation: F,
    journal: P,
) -> CleanupReceipt
where
    F: FnOnce(Lease, Option<tokio::process::Child>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<(), String>> + Send + 'static,
    P: FnOnce(&State) -> io::Result<()> + Send + 'static,
{
    let mut state = shared.lock().await;
    begin_cleanup_locked_with(&mut state, shared, id, operation, journal)
}

fn begin_cleanup_locked_with<F, Fut, P>(
    state: &mut State,
    shared: &Shared,
    id: &str,
    operation: F,
    journal: P,
) -> CleanupReceipt
where
    F: FnOnce(Lease, Option<tokio::process::Child>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<(), String>> + Send + 'static,
    P: FnOnce(&State) -> io::Result<()> + Send + 'static,
{
    state.radio_changed();
    if let Some(receipt) = state.cleanups.get(id) {
        return receipt.clone();
    }
    let (sender, receipt) = tokio::sync::watch::channel(None);
    let Some(lease) = state.leases.get(id).cloned() else {
        let _ = sender.send(Some(Ok(())));
        return receipt;
    };
    state.attached.remove(id);
    state.cancelled_p2p.remove(id);
    let producer = state.producers.get(id).map(|pending| {
        pending.cancel.cancel();
        pending.settled.clone()
    });
    let group_cleanup = state.group_cleanups.get(id).cloned();
    state.cleanups.insert(id.to_owned(), receipt.clone());
    let shared = shared.clone();
    tokio::spawn(async move {
        let producer_result = if let Some(receipt) = producer {
            wait_cleanup(receipt).await
        } else {
            Ok(())
        };
        // A group leave already owns teardown and its child. Full retirement
        // first waits for that operation to settle, then reads its final journal
        // identity. A failed leave retains that identity for this retry.
        if let Some(receipt) = group_cleanup {
            let _ = wait_cleanup(receipt).await;
        }
        let (lease, child) = {
            let mut state = shared.lock().await;
            state.radio_changed();
            let current = state.leases.get(&lease.id).cloned().unwrap_or(lease);
            let child = if producer_result.is_ok() {
                state.children.remove(&current.id)
            } else {
                None
            };
            (current, child)
        };
        let original = lease.clone();
        // Observe panics/cancellation of the I/O task too: the supervisor retains
        // the lease and publishes a failed receipt, allowing explicit recovery.
        let mut result = match producer_result {
            Ok(()) => tokio::spawn(async move { operation(lease, child).await })
                .await
                .unwrap_or_else(|error| Err(format!("Cleanup operation stopped: {error}"))),
            Err(error) => Err(format!("Radio producer did not settle: {error}")),
        };
        let mut state = shared.lock().await;
        state.radio_changed();
        if result.is_ok() {
            state.leases.remove(&original.id);
            if let Err(error) = journal(&state) {
                state.leases.insert(original.id.clone(), original.clone());
                result = Err(format!("Cleanup journal update failed: {error}"));
            }
        }
        if let Err(error) = &result {
            state.record_recovery_error(format!("{}: {error}", original.interface));
        }
        state.cleanups.remove(&original.id);
        let _ = sender.send(Some(result));
    });
    receipt
}

// Leaving a P2P group preserves the parent's direct-Wi-Fi reservation. Like
// full retirement, its worker survives a disconnected or cancelled caller.
async fn begin_group_cleanup(shared: &Shared, id: &str) -> Result<CleanupReceipt, String> {
    begin_group_cleanup_with(
        shared,
        id,
        |lease, child| async move {
            reap_child(child).await?;
            restore_p2p(&lease).await
        },
        persist,
    )
    .await
}

async fn begin_group_cleanup_with<F, Fut, P>(
    shared: &Shared,
    id: &str,
    operation: F,
    journal: P,
) -> Result<CleanupReceipt, String>
where
    F: FnOnce(Lease, Option<tokio::process::Child>) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<(), String>> + Send + 'static,
    P: FnOnce(&State) -> io::Result<()> + Send + 'static,
{
    let mut state = shared.lock().await;
    state.radio_changed();
    if state.cleanups.contains_key(id) {
        return Err("lease cleanup is in progress".into());
    }
    if let Some(receipt) = state.group_cleanups.get(id) {
        return Ok(receipt.clone());
    }
    let lease = state.leases.get(id).ok_or("lease not found")?.clone();
    if lease.kind != LeaseKind::DirectWifi {
        return Err("direct Wi-Fi lease required".into());
    }
    if !state.attached.contains(id) {
        return Err("lease is revoked; complete radio recovery first".into());
    }
    // A producer must settle before partial cleanup can start. Full retirement
    // owns cancellation of outstanding formation. Never guess its final group.
    if state.producers.contains_key(id) {
        return Err("P2P group formation is still in progress".into());
    }
    let child = state.children.remove(id);
    state.attached.remove(id);
    let (sender, receipt) = tokio::sync::watch::channel(None);
    state.group_cleanups.insert(id.to_owned(), receipt.clone());
    let shared = shared.clone();
    tokio::spawn(async move {
        let original = lease.clone();
        let mut result = tokio::spawn(async move { operation(lease, child).await })
            .await
            .unwrap_or_else(|error| Err(format!("Group cleanup operation stopped: {error}")));
        let mut state = shared.lock().await;
        state.radio_changed();
        if result.is_ok() {
            let mut restored = original.clone();
            restored.p2p_group = None;
            state.leases.insert(original.id.clone(), restored);
            if let Err(error) = journal(&state) {
                state.leases.insert(original.id.clone(), original.clone());
                result = Err(format!("Group cleanup journal update failed: {error}"));
            }
        }
        if let Err(error) = &result {
            state.record_recovery_error(format!("{}: {error}", original.interface));
        } else {
            state.cancelled_p2p.remove(&original.id);
            // Full retirement may have been queued while this leave was slow.
            // Its lease must stay unavailable until that subsequent job ends.
            if !state.cleanups.contains_key(&original.id) {
                state.attached.insert(original.id.clone());
            }
        }
        state.group_cleanups.remove(&original.id);
        let _ = sender.send(Some(result));
    });
    Ok(receipt)
}

async fn reap_child(child: Option<tokio::process::Child>) -> Result<(), String> {
    if let Some(mut child) = child {
        child
            .kill()
            .await
            .map_err(|error| format!("Cannot stop owned helper: {error}"))?;
        child
            .wait()
            .await
            .map_err(|error| format!("Cannot reap owned helper: {error}"))?;
    }
    Ok(())
}

async fn cleanup_lease(shared: &Shared, id: &str) -> Result<(), String> {
    wait_cleanup(begin_cleanup(shared, id).await).await
}

async fn cleanup_all(shared: &Shared, ids: Vec<String>) -> Vec<(String, Result<(), String>)> {
    // Retire every owned radio before waiting for the first slow one. A dead
    // client must not leave its second adapter advertised as healthy meanwhile.
    let mut receipts = Vec::new();
    for id in ids {
        let receipt = begin_cleanup(shared, &id).await;
        receipts.push((id, receipt));
    }
    let mut results = Vec::new();
    for (id, receipt) in receipts {
        results.push((id, wait_cleanup(receipt).await));
    }
    results
}

struct RadioProducer {
    cancel: tokio_util::sync::CancellationToken,
    interfaces: std::collections::HashSet<String>,
    settled: CleanupReceipt,
}

// The producer outlives its requesting socket. Its cancellation token requests
// a stop; only its settlement receipt authorizes a subsequent radio retirement.
fn spawn_radio_with<F, Fut>(
    state: &mut State,
    shared: &Shared,
    lease_id: String,
    cancel: tokio_util::sync::CancellationToken,
    interfaces: std::collections::HashSet<String>,
    operation: F,
) -> tokio::sync::oneshot::Receiver<Result<Response, String>>
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<Response, String>> + Send + 'static,
{
    let (settled, receipt) = tokio::sync::watch::channel(None);
    let (reply, receiver) = tokio::sync::oneshot::channel();
    state.producers.insert(
        lease_id.clone(),
        RadioProducer {
            cancel,
            interfaces,
            settled: receipt,
        },
    );
    state.radio_changed();
    let shared = shared.clone();
    tokio::spawn(async move {
        let (result, settlement) = match tokio::spawn(async move { operation().await }).await {
            Ok(result) => (result, Ok(())),
            Err(error) => {
                let error = format!("Radio producer stopped unexpectedly: {error}");
                (Err(error.clone()), Err(error))
            }
        };
        let mut state = shared.lock().await;
        state.radio_changed();
        if let Err(error) = &settlement {
            // An unexpected stop has no proven cleanup outcome. Preserve the
            // failed receipt and reservation instead of admitting another actor.
            state.attached.remove(&lease_id);
            state.record_recovery_error(format!("{lease_id}: {error}"));
        } else {
            state.producers.remove(&lease_id);
        }
        settled.send_replace(Some(settlement));
        let _ = reply.send(result);
    });
    receiver
}

fn competing_use(
    lease: &Lease,
    interface: &linuxdrop_hardware::NetworkInterface,
    pending: Option<&RadioProducer>,
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

fn state_from_journal(data: io::Result<Vec<u8>>) -> io::Result<State> {
    let bytes = match data {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(State::default()),
        // An unreadable journal must not be interpreted as an empty journal.
        Err(error) => return Err(error),
    };
    let leases: Vec<Lease> = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    let mut state = State::default();
    for lease in leases {
        if state.leases.insert(lease.id.clone(), lease).is_some() {
            return Err(io::Error::other(
                "Duplicate lease identity in recovery journal",
            ));
        }
    }
    Ok(state)
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
    let state = state_from_journal(std::fs::read(JOURNAL))?;
    persist(&state)?;
    let listener = UnixListener::bind(socket)?;
    // Every mutation requires an active local session and polkit authorization.
    std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o666))?;
    let startup_leases: Vec<_> = state.leases.keys().cloned().collect();
    let state = Arc::new(Mutex::new(state));
    // Listen while recovery proceeds. All journaled radios are reserved and
    // unattached from the outset; slow network restoration cannot prevent status
    // or recovery clients from learning what is happening.
    for id in startup_leases {
        let _ = begin_cleanup(&state, &id).await;
    }
    let limit = Arc::new(Semaphore::new(16));
    let mut tasks = tokio::task::JoinSet::new();
    let monitor = state.clone();
    tasks.spawn(async move {
        let mut next_recovery = tokio::time::Instant::now();
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            if tokio::time::Instant::now() >= next_recovery {
                next_recovery = tokio::time::Instant::now() + Duration::from_secs(30);
                let pending: Vec<_> = {
                    let state = monitor.lock().await;
                    state.leases.values().filter(|lease| lease.p2p_pending && lease.p2p_recovery.is_some()
                        && !state.attached.contains(&lease.id) && !state.producers.contains_key(&lease.id)
                        && !state.cleanups.contains_key(&lease.id) && !state.group_cleanups.contains_key(&lease.id))
                        .map(|lease| lease.id.clone()).collect()
                };
                for id in pending { let _ = begin_cleanup(&monitor, &id).await; }
            }
            let revision = {
                let state = monitor.lock().await;
                if state.leases.is_empty() { continue; }
                state.network_revision
            };
            let inventory = inventory().await;
            let needs_supplicant = monitor.lock().await.leases.values().any(|lease| lease.p2p_group.is_some());
            let supplicant = if needs_supplicant {
                timeout(Duration::from_secs(4), async {
                    let connection = zbus::Connection::system().await?;
                    linuxdrop_network::p2p::service_instance(&connection).await
                }).await.ok().and_then(Result::ok)
            } else { None };
            let leases: Vec<_> = {
                let state = monitor.lock().await;
                if state.network_revision != revision { continue; }
                state.leases.values().cloned().collect()
            };
            for lease in leases {
                let dead = {
                    let mut state = monitor.lock().await;
                    if !state.observation_is_current(revision, &lease.id) { continue; }
                    state.children.get_mut(&lease.id).is_some_and(|child| !matches!(child.try_wait(), Ok(None)))
                };
                // Address inspection can wait on an external process. It must
                // not block status or cancellation on unrelated radios.
                let ipv6_survives = dead
                    && lease.p2p_group.as_ref().is_some_and(|group| group.peer_object != "/")
                    && p2p_addresses(&lease).await.is_ok_and(|a| a.ipv6.is_some());
                let link_invalid = lease.kind == LeaseKind::Monitor
                    && (verify_owned(&lease).is_err() || lease.awdl_interface.as_ref().is_some_and(|tap|
                        !Path::new("/sys/class/net").join(tap).try_exists().unwrap_or(false)));
                {
                    let mut state = monitor.lock().await;
                    if !state.observation_is_current(revision, &lease.id) { continue; }
                    if ipv6_survives { state.children.remove(&lease.id); }
                    let owner_lost = lease.p2p_group.as_ref().is_some_and(|group| {
                        supplicant.as_ref().is_none_or(|(owner, bus)| group.service_owner != *owner || group.bus_guid != *bus)
                    });
                    let radio_gone = !inventory.radios.iter().any(|r| r.phy == lease.phy);
                    let regulatory_change = inventory.radios.iter().find(|r| r.phy == lease.phy).is_some_and(|radio| {
                        lease.allowed_frequencies.iter().any(|frequency| !radio.channels.iter().any(|c| c.frequency_mhz == *frequency && !c.disabled && !c.no_ir && !c.radar))
                    });
                    let unsafe_use = inventory.interfaces.iter().any(|i| competing_use(&lease, i, state.producers.get(&lease.id)));
                    if (dead && !ipv6_survives) || radio_gone || unsafe_use || regulatory_change || owner_lost || link_invalid {
                        // Validate the observation and reserve cleanup under the
                        // same lock. Slow I/O still runs in the persistent worker.
                        let _ = begin_cleanup_locked_with(&mut state, &monitor, &lease.id, restore_child_and_lease, persist);
                        state.record_recovery_error(format!(
                            "{}: lease stopped after helper exit, interface loss/ownership change, supplicant owner loss, unplug, regulatory change, or competing radio use", lease.interface));
                    }
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
    let leases: Vec<_> = state.lock().await.leases.keys().cloned().collect();
    for (id, result) in cleanup_all(&state, leases).await {
        if let Err(error) = result {
            eprintln!("Recovery failed for {id}: {error}");
        }
    }
    persist(&*state.lock().await)?;
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
                    let preparing = matches!(&request,
                        Request::JoinP2p { .. } | Request::HostP2p { .. }
                        | Request::Acquire { .. } | Request::AcquireAwdl { .. }
                        | Request::Diagnose { .. } | Request::SetChannel { .. });
                    let response = tokio::select! {
                        result = apply(request, uid, &mut owned, &state) => result,
                        result = reader.fill_buf(), if preparing => {
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
    let _ = cleanup_all(&state, owned).await;

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

fn direct_capabilities(
    modes: &[String],
    channels: &[linuxdrop_hardware::Channel],
) -> linuxdrop_network::DirectWifiCapabilities {
    let mode = |name| modes.iter().any(|value| value == name);
    let frequencies: Vec<_> = channels
        .iter()
        .filter(|channel| !channel.disabled)
        .map(|channel| channel.frequency_mhz)
        .collect();
    let can_initiate = |band: std::ops::Range<u32>| {
        channels.iter().any(|channel| {
            band.contains(&channel.frequency_mhz)
                && !channel.disabled
                && !channel.no_ir
                && !channel.radar
        })
    };
    linuxdrop_network::DirectWifiCapabilities {
        station: mode("managed") && !frequencies.is_empty(),
        // The NM hotspot profile currently requests the 2.4 GHz band explicitly.
        hotspot: mode("AP") && can_initiate(2400..2500),
        p2p_group_owner: mode("P2P-GO") && (can_initiate(2400..2500) || can_initiate(4900..5900)),
        p2p_client: mode("P2P-client") && !frequencies.is_empty(),
        frequencies,
    }
}

async fn join_p2p(
    shared: &Shared,
    lease_id: String,
    peer_name: String,
    pin: String,
    mut frequency: u32,
    host_auth: Option<linuxdrop_network::P2pHostAuth>,
) -> Result<Response, String> {
    let host = host_auth.is_some();
    // Inventory and supplicant/DHCP I/O may take seconds. Other leased radios
    // and read-only status requests must remain serviceable throughout.
    let inv = inventory().await;
    let mut state = shared.lock().await;
    state.radio_changed();
    let mut lease = state
        .leases
        .get(&lease_id)
        .ok_or("lease not found")?
        .clone();
    if !state.attached.contains(&lease_id) {
        return Err("lease is revoked; complete radio recovery first".into());
    }
    if state.cancelled_p2p.contains(&lease_id) {
        return Err("P2P operation cancelled".into());
    }
    if lease.kind != LeaseKind::DirectWifi
        || lease.p2p_group.is_some()
        || lease.p2p_pending
        || state.producers.contains_key(&lease_id)
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
    if host {
        if !radio.modes.iter().any(|mode| mode == "P2P-GO") {
            return Err("Adapter does not support P2P group-owner mode".into());
        }
        frequency = [5180, 2437, 5745, 2412]
            .into_iter()
            .chain(radio.channels.iter().map(|channel| channel.frequency_mhz))
            .find(|frequency| {
                radio.channels.iter().any(|channel| {
                    channel.frequency_mhz == *frequency
                        && !channel.disabled
                        && !channel.no_ir
                        && !channel.radar
                })
            })
            .ok_or("No permitted P2P group-owner channel")?;
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
    // Survive a helper crash even before supplicant returns a group identity.
    lease.p2p_pending = true;
    lease.p2p_recovery = None;
    state.leases.insert(lease_id.clone(), lease.clone());
    if let Err(error) = persist(&state) {
        lease.p2p_pending = false;
        state.leases.insert(lease_id.clone(), lease);
        return Err(error.to_string());
    }
    let cancel = tokio_util::sync::CancellationToken::new();
    let stop_on_drop = cancel.clone().drop_guard();
    let worker_cancel = cancel.clone();
    let worker_shared = shared.clone();
    let interfaces = inv
        .interfaces
        .iter()
        .filter(|interface| interface.phy.as_deref() == Some(&lease.phy))
        .map(|interface| interface.name.clone())
        .collect();
    let setup = P2pSetup {
        peer_name,
        pin,
        frequency,
        host_auth,
    };
    let reply = spawn_radio_with(
        &mut state,
        shared,
        lease_id,
        cancel,
        interfaces,
        move || async move { perform_p2p(&worker_shared, lease, setup, worker_cancel).await },
    );
    drop(state);
    let result = reply
        .await
        .map_err(|_| "P2P supervisor stopped without a reply".to_owned())?;
    stop_on_drop.disarm();
    result
}

struct P2pSetup {
    peer_name: String,
    pin: String,
    frequency: u32,
    host_auth: Option<linuxdrop_network::P2pHostAuth>,
}

fn formation_error(message: String) -> linuxdrop_network::p2p::FormationError {
    io::Error::other(message).into()
}

async fn perform_p2p(
    shared: &Shared,
    mut lease: Lease,
    setup: P2pSetup,
    cancel: tokio_util::sync::CancellationToken,
) -> Result<Response, String> {
    let P2pSetup {
        peer_name,
        pin,
        frequency,
        host_auth,
    } = setup;
    let host = host_auth.is_some();
    let lease_id = lease.id.clone();
    let formed = async {
        let provenance = p2p_recovery::prepare(&lease)
            .await
            .map_err(formation_error)?;
        {
            let mut state = shared.lock().await;
            if cancel.is_cancelled() || !state.attached.contains(&lease_id) {
                return Err(formation_error("P2P lease ended during preparation".into()));
            }
            lease.p2p_recovery = Some(provenance.clone());
            state.leases.insert(lease_id.clone(), lease.clone());
            persist(&state).map_err(|error| formation_error(error.to_string()))?;
        }
        if let Some(auth) = host_auth {
            let hosted = linuxdrop_network::p2p::create_group_prepared(
                &lease.interface,
                frequency,
                auth,
                cancel.clone(),
                Some(provenance.formation),
            )
            .await?;
            Ok((
                hosted.group,
                Some((
                    hosted.ssid,
                    hosted.password,
                    hosted.frequency,
                    hosted.device_name,
                )),
            ))
        } else {
            let group = linuxdrop_network::p2p::connect_wps_prepared(
                &lease.interface,
                &peer_name,
                &pin,
                frequency,
                cancel.clone(),
                Some(provenance.formation),
            )
            .await?;
            Ok((group, None))
        }
    }
    .await;
    let (group, credentials) = match formed {
        Ok(group) => group,
        Err(error) => {
            let error: linuxdrop_network::p2p::FormationError = error;
            lease.p2p_pending = error
                .downcast_ref::<linuxdrop_network::p2p::FormationUncertain>()
                .is_some();
            if !lease.p2p_pending {
                lease.p2p_recovery = None;
            }
            if let Some(failed) =
                error.downcast_ref::<linuxdrop_network::p2p::GroupCleanupFailure>()
            {
                lease.p2p_group = Some(failed.identity.clone());
            }
            let mut state = shared.lock().await;
            state.radio_changed();
            if lease.p2p_pending || lease.p2p_group.is_some() {
                state.attached.remove(&lease_id);
                state.record_recovery_error(format!("{lease_id}: {error}"));
            }
            state.leases.insert(lease_id.clone(), lease.clone());
            if let Err(journal) = persist(&state) {
                state.attached.remove(&lease_id);
                return Err(format!(
                    "{error}; formation journal update failed: {journal}"
                ));
            }
            return Err(error.to_string());
        }
    };
    let interface = group.identity.interface.clone();
    let formed_identity = group.identity.clone();
    let settlement = group.settlement();
    let published = async {
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
        state.radio_changed();
        if cancel.is_cancelled() || !state.leases.contains_key(&lease_id) {
            return Err("P2P radio lease ended during group formation".into());
        }
        lease.p2p_pending = false;
        lease.p2p_group = Some(group.identity.clone());
        lease.p2p_recovery = None;
        state.leases.insert(lease_id.clone(), lease.clone());
        persist(&state).map_err(|e| e.to_string())?;
        group.into_journaled();
        drop(state);
        Ok::<(), String>(())
    }
    .await;
    let settled = settlement.wait().await.map_err(|error| error.to_string());
    if published.is_err() || settled.is_err() {
        let mut state = shared.lock().await;
        state.radio_changed();
        state.attached.remove(&lease_id);
        lease.p2p_pending = false;
        lease.p2p_group = settled.is_err().then_some(formed_identity);
        lease.p2p_recovery = None;
        state.leases.insert(lease_id.clone(), lease.clone());
        persist(&state).map_err(|error| error.to_string())?;
    }
    published?;
    settled?;
    let network = if host {
        start_p2p_host_network(&lease, &cancel).await
    } else {
        start_p2p_network(&lease, &cancel).await
    };
    let mut state = shared.lock().await;
    state.radio_changed();
    let lease_exists = state.leases.contains_key(&lease_id);
    let cleanup_running = state.cleanups.contains_key(&lease_id);
    let still_owned = lease_exists && !cancel.is_cancelled();
    match network {
        Ok((child, addresses)) if still_owned && (!host || addresses.ipv4.is_some()) => {
            if let Some(child) = child {
                state.children.insert(lease_id.clone(), child);
            }
            drop(state);
            if let Some((ssid, password, frequency, device_name)) = credentials {
                Ok(Response::P2pHosted {
                    device_name,
                    interface,
                    ssid,
                    password,
                    frequency,
                    ipv4_address: addresses
                        .ipv4
                        .ok_or("P2P group owner lost its IPv4 address")?,
                    ipv6_address: addresses.ipv6,
                })
            } else {
                Ok(Response::P2pJoined {
                    interface,
                    ipv4_address: addresses.ipv4,
                    ipv6_address: addresses.ipv6,
                })
            }
        }
        result => {
            // Reap a returned DHCP child before acknowledging an ended lease.
            drop(state);
            let error = match result {
                Err(error) => error,
                Ok((child, _)) => {
                    reap_child(child).await?;
                    "P2P radio lease ended during address acquisition".into()
                }
            };
            if lease_exists && !cleanup_running {
                let restored = restore_p2p(&lease).await;
                let mut state = shared.lock().await;
                state.radio_changed();
                if let Err(cleanup_error) = restored {
                    state.attached.remove(&lease_id);
                    state.record_recovery_error(format!("{lease_id}: {cleanup_error}"));
                    return Err(format!("{error}; cleanup failed: {cleanup_error}"));
                }
                if let Some(current) = state.leases.get_mut(&lease_id) {
                    current.p2p_group = None;
                }
                if let Err(journal_error) = persist(&state) {
                    state.leases.insert(lease_id.clone(), lease.clone());
                    state.attached.remove(&lease_id);
                    return Err(format!("{error}; cleanup journal failed: {journal_error}"));
                }
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
        let original_owned = owned.clone();
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
                    .any(|l| l.uid == uid && !original_owned.contains(&l.id));
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
    if let Request::SetChannel { lease_id, channel } = request {
        return channel::set_channel(shared, owned, lease_id, channel).await;
    }
    if let Request::Reserve { radio_id } = request {
        return acquire::reserve_radio(shared, uid, owned, radio_id).await;
    }
    let awdl = matches!(&request, Request::AcquireAwdl { .. });
    if let Request::Acquire { radio_id, channel } | Request::AcquireAwdl { radio_id, channel } =
        request
    {
        return acquire::acquire_radio(shared, uid, owned, radio_id, channel, awdl).await;
    }
    if let Request::HostP2p { lease_id, auth } = request {
        if !owned.contains(&lease_id) {
            return Err("lease does not belong to this connection".into());
        }
        return join_p2p(
            shared,
            lease_id,
            String::new(),
            String::new(),
            0,
            Some(auth),
        )
        .await;
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
        return join_p2p(shared, lease_id, peer_name, pin, frequency, None).await;
    }
    if let Request::Release { ref lease_id } = request {
        if !owned.contains(lease_id) {
            return Err("lease does not belong to this connection".into());
        }
        cleanup_lease(shared, lease_id).await?;
        owned.retain(|id| id != lease_id);
        return Ok(Response::Ok);
    }
    if let Request::LeaveP2p { ref lease_id } = request {
        if !owned.contains(lease_id) {
            return Err("lease does not belong to this connection".into());
        }
        wait_cleanup(begin_group_cleanup(shared, lease_id).await?).await?;
        return Ok(Response::Ok);
    }
    if matches!(request, Request::RetryRecovery) {
        let abandoned: Vec<_> = {
            let state = shared.lock().await;
            state
                .leases
                .values()
                .filter(|l| l.uid == uid && !state.attached.contains(&l.id))
                .map(|l| l.id.clone())
                .collect()
        };
        let _ = cleanup_all(shared, abandoned).await;
    }
    let mut state = shared.lock().await;
    if !matches!(
        request,
        Request::Status | Request::RecoveryStatus | Request::RetryRecovery
    ) {
        state.radio_changed();
    }
    match request {
        Request::Diagnose { .. } => unreachable!(),
        Request::JoinP2p { .. } | Request::HostP2p { .. } => unreachable!(),
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
            if let Some(operation) = state.producers.get(&lease_id) {
                operation.cancel.cancel();
            }
            // Also cover cancellation while JoinP2p is collecting inventory,
            // before a supplicant operation has been registered.
            state.cancelled_p2p.insert(lease_id);
            Ok(Response::Ok)
        }
        Request::LeaveP2p { .. } => unreachable!(),
        Request::RecoveryStatus | Request::RetryRecovery => {
            let issues = state
                .leases
                .values()
                .filter(|l| l.uid == uid && !state.attached.contains(&l.id))
                .map(|l| {
                    let check = if l.p2p_pending {
                        Err(P2P_UNCERTAIN.to_owned())
                    } else if l.kind == LeaseKind::DirectWifi {
                        Ok(())
                    } else {
                        verify_owned(l)
                    };
                    RecoveryIssue {
                        lease_id: l.id.clone(),
                        interface: l.interface.clone(),
                        ownership_verified: check.is_ok(),
                        detail: if state.cleanups.contains_key(&l.id) || state.group_cleanups.contains_key(&l.id) {
                            "Cleanup is in progress; the radio remains reserved".into()
                        } else if state.producers.contains_key(&l.id) {
                            "Radio preparation is in progress; the radio remains reserved".into()
                        } else { check.err().unwrap_or_else(|| {
                            "Orphaned lease; authorized retry restores only this owned resource".into()
                        }) },
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
        Request::Reserve { .. } => unreachable!(),
        Request::Status => Ok(Response::State {
            leases: state
                .leases
                .values()
                .filter(|l| l.uid == uid && state.attached.contains(&l.id))
                .cloned()
                .collect(),
            recovery_errors: state.recovery_errors.clone(),
        }),
        Request::Acquire { .. } | Request::AcquireAwdl { .. } => unreachable!(),
        Request::SetChannel { .. } => unreachable!(),
        Request::Release { .. } => unreachable!(),
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
    let boot =
        std::fs::read_to_string("/proc/sys/kernel/random/boot_id").map_err(|e| e.to_string())?;
    if lease.p2p_pending && boot.trim() == lease.boot_id {
        p2p_recovery::recover(lease).await?;
    }
    let Some(group) = &lease.p2p_group else {
        return Ok(());
    };
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
        // Supplicant has retired its object; wait for the kernel VIF too. Do
        // not free the physical-radio reservation while its link still exists.
        timeout(Duration::from_secs(5), async {
            while path.try_exists().map_err(|error| error.to_string())? {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok::<(), String>(())
        })
        .await
        .map_err(|_| "P2P kernel interface did not disappear".to_owned())??;
    }
    for suffix in ["ipv4.json", "dhcp.conf", "dhcp.leases", "dhcp.pid"] {
        let _ = std::fs::remove_file(format!("/run/linuxdrop/{}.{suffix}", lease.id));
    }
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
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<(Option<tokio::process::Child>, P2pAddresses), String> {
    let group = lease.p2p_group.as_ref().ok_or("P2P group missing")?;
    // Verify ownership before starting any DHCP traffic on this interface.
    let mut addresses = p2p_addresses(lease).await?;
    if cancel.is_cancelled() {
        return Err("P2P address setup cancelled".into());
    }
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
            &format!("{HELPER_DIRECTORY}/p2p-dhcp"),
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
    let ready = async {
        loop {
            if cancel.is_cancelled() {
                return Err("P2P address setup cancelled".to_owned());
            }
            if let Some(process) = child.as_mut() {
                if process
                    .try_wait()
                    .map_err(|error| error.to_string())?
                    .is_some()
                {
                    child = None;
                }
            }
            if addresses.ipv6.is_some() || (addresses.ipv4.is_some() && child.is_some()) {
                return Ok(addresses);
            }
            if tokio::time::Instant::now() >= deadline {
                return Err("P2P group received neither usable IPv4 nor IPv6".into());
            }
            tokio::select! {
                _ = cancel.cancelled() => return Err("P2P address setup cancelled".into()),
                _ = tokio::time::sleep(Duration::from_millis(200)) => {},
            }
            addresses = p2p_addresses(lease).await?;
        }
    }
    .await;
    match ready {
        Ok(addresses) => Ok((child, addresses)),
        Err(error) => {
            reap_child(child).await?;
            Err(error)
        }
    }
}

fn p2p_subnet(routes: &[serde_json::Value]) -> Result<std::net::Ipv4Addr, String> {
    use std::net::Ipv4Addr;
    let mut occupied = Vec::new();
    for route in routes {
        let Some(destination) = route["dst"].as_str() else {
            continue;
        };
        if destination == "default" {
            continue;
        }
        let (address, prefix) = destination.split_once('/').unwrap_or((destination, "32"));
        let address: Ipv4Addr = address.parse().map_err(|_| "Unrecognized IPv4 route")?;
        let prefix: u32 = prefix.parse().map_err(|_| "Invalid IPv4 route prefix")?;
        if prefix > 32 {
            return Err("Invalid IPv4 route prefix".into());
        }
        if prefix > 0 {
            occupied.push((u32::from(address), prefix));
        }
    }
    let candidates = std::iter::once(Ipv4Addr::new(192, 168, 49, 1))
        .chain((200..=254).map(|subnet| Ipv4Addr::new(192, 168, subnet, 1)))
        .chain((200..=254).map(|subnet| Ipv4Addr::new(172, 31, subnet, 1)));
    candidates
        .into_iter()
        .find(|candidate| {
            !occupied.iter().any(|(address, prefix)| {
                let mask = u32::MAX << (32 - (*prefix).min(24));
                u32::from(*candidate) & mask == *address & mask
            })
        })
        .ok_or_else(|| "No conflict-free private subnet for the P2P group".into())
}

fn p2p_dhcp_config(interface: &str, lease: &str, gateway: std::net::Ipv4Addr) -> String {
    let [a, b, c, _] = gateway.octets();
    // Deliberately no router, DNS, domain or host settings: this group carries
    // the explicit transfer only, not the peer's ordinary Internet traffic.
    format!("start {a}.{b}.{c}.2\nend {a}.{b}.{c}.33\ninterface {interface}\nmax_leases 32\nlease_file /run/linuxdrop/{lease}.dhcp.leases\npidfile /run/linuxdrop/{lease}.dhcp.pid\noption subnet 255.255.255.0\noption lease 600\nauto_time 30\n")
}

async fn start_p2p_host_network(
    lease: &Lease,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<(Option<tokio::process::Child>, P2pAddresses), String> {
    use std::io::Write;
    if cancel.is_cancelled() {
        return Err("P2P host setup cancelled".into());
    }
    let group = lease.p2p_group.as_ref().ok_or("P2P group missing")?;
    p2p_addresses(lease).await?; // Verify the exact newly marked group first.
    let output = timeout(
        Duration::from_secs(3),
        Command::new("/usr/sbin/ip")
            .args(["-j", "-4", "route", "show", "table", "all"])
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "Route inspection timed out")?
    .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err("Could not inspect existing network routes".into());
    }
    let routes: Vec<serde_json::Value> =
        serde_json::from_slice(&output.stdout).map_err(|e| e.to_string())?;
    let gateway = p2p_subnet(&routes)?;
    run_command(
        "/usr/sbin/ip",
        &["link", "set", "dev", &group.interface, "up"],
        3,
    )
    .await?;
    run_command(
        "/usr/sbin/ip",
        &[
            "-4",
            "address",
            "add",
            &format!("{gateway}/24"),
            "dev",
            &group.interface,
        ],
        3,
    )
    .await?;
    let config_path = format!("/run/linuxdrop/{}.dhcp.conf", lease.id);
    let mut config = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&config_path)
        .map_err(|e| e.to_string())?;
    config
        .write_all(p2p_dhcp_config(&group.interface, &lease.id, gateway).as_bytes())
        .map_err(|e| e.to_string())?;
    config.sync_all().map_err(|e| e.to_string())?;
    drop(config);
    if cancel.is_cancelled() {
        return Err("P2P host setup cancelled".into());
    }
    let mut child = Command::new("/usr/bin/busybox")
        .args(["udhcpd", "-f", "-I", &gateway.to_string(), &config_path])
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| e.to_string())?;
    let ready = async {
        // Catch bind/configuration errors before publishing any credentials.
        tokio::time::sleep(Duration::from_millis(250)).await;
        if !matches!(child.try_wait(), Ok(None)) {
            return Err("P2P DHCP server could not start".into());
        }
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        let addresses = loop {
            if cancel.is_cancelled() {
                return Err("P2P host setup cancelled".to_owned());
            }
            let addresses = p2p_addresses(lease).await?;
            if addresses.ipv6.is_some() || tokio::time::Instant::now() >= deadline {
                break addresses;
            }
            if !matches!(child.try_wait(), Ok(None)) {
                return Err("P2P DHCP server stopped during address setup".into());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        };
        if addresses.ipv4 != Some(gateway) {
            return Err("P2P group owner address changed".into());
        }
        Ok(addresses)
    }
    .await;
    match ready {
        Ok(addresses) => Ok((Some(child), addresses)),
        Err(error) => {
            reap_child(Some(child)).await?;
            Err(error)
        }
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
    let child = Command::new(program)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin")
        .env("LC_ALL", "C")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| error.to_string())?;
    wait_command(child, program, Duration::from_secs(seconds)).await
}

async fn wait_command(
    mut child: tokio::process::Child,
    program: &str,
    budget: Duration,
) -> Result<(), String> {
    let mut stderr = child.stderr.take().ok_or("Command stderr is unavailable")?;
    // Drain concurrently to prevent a full pipe from blocking termination. Keep
    // bounded diagnostics even if a utility produces unexpectedly large output.
    let mut reader = tokio::spawn(async move {
        let mut bytes = Vec::new();
        (&mut stderr)
            .take(64 * 1024)
            .read_to_end(&mut bytes)
            .await?;
        tokio::io::copy(&mut stderr, &mut tokio::io::sink()).await?;
        Ok::<_, io::Error>(bytes)
    });
    let status = match timeout(budget, child.wait()).await {
        Ok(Ok(status)) => status,
        outcome => {
            let message = match outcome {
                Err(_) => format!("{program} timed out"),
                Ok(Err(error)) => format!("{program}: {error}"),
                Ok(Ok(_)) => unreachable!(),
            };
            let stopped = reap_child(Some(child)).await;
            reader.abort();
            let _ = reader.await;
            stopped?;
            return Err(message);
        }
    };
    let diagnostics = match timeout(Duration::from_secs(1), &mut reader).await {
        Ok(result) => result
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?,
        Err(_) => {
            reader.abort();
            let _ = reader.await;
            return Err(format!("{program}: diagnostic pipe did not close"));
        }
    };
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "{program}: {}",
            String::from_utf8_lossy(&diagnostics).trim()
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
    #[test]
    fn startup_retains_all_reservations_and_rejects_unreadable_journals() {
        let lease = direct_lease();
        let data = serde_json::to_vec(&vec![lease.clone()]).unwrap();
        let state = state_from_journal(Ok(data)).unwrap();
        assert!(state.leases.contains_key(&lease.id));
        assert!(state.attached.is_empty());
        let duplicates = serde_json::to_vec(&vec![lease.clone(), lease]).unwrap();
        assert!(state_from_journal(Ok(duplicates)).is_err());
        assert!(state_from_journal(Ok(b"invalid journal".to_vec())).is_err());
        assert!(state_from_journal(Err(io::Error::from(io::ErrorKind::PermissionDenied))).is_err());
        assert!(
            state_from_journal(Err(io::Error::from(io::ErrorKind::NotFound)))
                .unwrap()
                .leases
                .is_empty()
        );
    }

    #[tokio::test]
    async fn cleanup_receipt_preserves_reservation_without_blocking_status() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let lease = direct_lease();
        let id = lease.id.clone();
        let uid = lease.uid;
        let shared = Arc::new(Mutex::new(State::default()));
        {
            let mut state = shared.lock().await;
            state.leases.insert(id.clone(), lease);
            state.attached.insert(id.clone());
        }
        let runs = Arc::new(AtomicUsize::new(0));
        let count = runs.clone();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, held) = tokio::sync::oneshot::channel();
        let first = begin_cleanup_with(
            &shared,
            &id,
            move |_, _| async move {
                count.fetch_add(1, Ordering::SeqCst);
                let _ = started.send(());
                held.await.unwrap()
            },
            |_| Ok(()),
        )
        .await;
        timeout(Duration::from_secs(1), ready)
            .await
            .unwrap()
            .unwrap();
        let duplicate = begin_cleanup_with(
            &shared,
            &id,
            |_, _| async { panic!("Duplicate cleanup must share the existing receipt") },
            |_| Ok(()),
        )
        .await;
        let status = timeout(
            Duration::from_millis(250),
            apply(Request::Status, uid, &mut vec![], &shared),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            matches!(status, Response::State { leases, .. } if leases.is_empty()),
            "Retiring lease must not appear healthy"
        );
        let recovery = timeout(
            Duration::from_millis(250),
            apply(Request::RecoveryStatus, uid, &mut vec![], &shared),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            matches!(recovery, Response::Recovery { issues, .. } if issues[0].detail.contains("in progress"))
        );
        assert!(
            shared.lock().await.leases.contains_key(&id),
            "Retiring radio cannot be reallocated"
        );
        assert!(apply(
            Request::Release {
                lease_id: id.clone()
            },
            uid + 1,
            &mut vec![],
            &shared
        )
        .await
        .is_err());
        assert!(apply(
            Request::LeaveP2p {
                lease_id: id.clone()
            },
            uid,
            &mut vec![id.clone()],
            &shared
        )
        .await
        .unwrap_err()
        .contains("cleanup"));
        let mut retained_ownership = vec![id.clone()];
        let mut release_request = Box::pin(apply(
            Request::Release {
                lease_id: id.clone(),
            },
            uid,
            &mut retained_ownership,
            &shared,
        ));
        let pending = std::future::poll_fn(|cx| {
            std::task::Poll::Ready(
                std::future::Future::poll(release_request.as_mut(), cx).is_pending(),
            )
        })
        .await;
        assert!(pending, "Release must wait on the already-started fake cleanup; never run production journal I/O in this test");
        let waiting = tokio::spawn(wait_cleanup(first));
        waiting.abort();
        let _ = waiting.await;
        release.send(Err("delayed cleanup failure".into())).unwrap();
        assert!(wait_cleanup(duplicate)
            .await
            .unwrap_err()
            .contains("delayed cleanup"));
        let result = release_request.await;
        assert!(result.is_err());
        assert_eq!(retained_ownership, vec![id.clone()]);
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert!(shared.lock().await.leases.contains_key(&id));
        assert!(!shared.lock().await.attached.contains(&id));

        let receipt = begin_cleanup_with(
            &shared,
            &id,
            |_, _| async { Ok(()) },
            |_| Err(io::Error::other("test journal unavailable")),
        )
        .await;
        assert!(wait_cleanup(receipt).await.unwrap_err().contains("journal"));
        assert!(
            shared.lock().await.leases.contains_key(&id),
            "A failed durable receipt must retain recovery state"
        );
        let expected = id.clone();
        let receipt = begin_cleanup_with(
            &shared,
            &id,
            |_, _| async { Ok(()) },
            move |state| {
                assert!(!state.leases.contains_key(&expected));
                Ok(())
            },
        )
        .await;
        wait_cleanup(receipt).await.unwrap();
        let state = shared.lock().await;
        assert!(state.leases.is_empty() && state.cleanups.is_empty());
    }

    fn lease_with_group() -> Lease {
        let mut lease = direct_lease();
        lease.p2p_group = Some(linuxdrop_network::p2p::GroupIdentity {
            interface: "p2p-test0".into(),
            interface_object: "/test/interface".into(),
            group_object: "/test/group".into(),
            parent_interface: lease.interface.clone(),
            peer_object: "/test/peer".into(),
            service_owner: ":1.42".into(),
            bus_guid: "test-bus".into(),
        });
        lease
    }

    #[tokio::test]
    async fn group_cleanup_is_responsive_and_full_retirement_waits_for_it() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let lease = lease_with_group();
        let id = lease.id.clone();
        let uid = lease.uid;
        let shared = Arc::new(Mutex::new(State::default()));
        {
            let mut state = shared.lock().await;
            state.leases.insert(id.clone(), lease);
            state.attached.insert(id.clone());
        }
        let before_leave = shared.lock().await.network_revision;
        let (release, held) = tokio::sync::oneshot::channel();
        let receipt = begin_group_cleanup_with(
            &shared,
            &id,
            |lease, _| async move {
                assert!(lease.p2p_group.is_some());
                held.await.unwrap();
                Ok(())
            },
            |state| {
                assert!(state.leases.values().all(|lease| lease.p2p_group.is_none()));
                Ok(())
            },
        )
        .await
        .unwrap();
        assert!(
            !shared
                .lock()
                .await
                .observation_is_current(before_leave, &id),
            "A watchdog observation made before leave cannot retire the changed group"
        );
        let duplicate = begin_group_cleanup_with(
            &shared,
            &id,
            |_, _| async { panic!("A duplicate must share the existing group receipt") },
            |_| Ok(()),
        )
        .await
        .unwrap();
        assert!(matches!(
            timeout(Duration::from_millis(250), apply(Request::Status, uid, &mut vec![], &shared))
                .await.unwrap().unwrap(),
            Response::State { leases, .. } if leases.is_empty()
        ));
        assert!(matches!(
            timeout(Duration::from_millis(250), apply(Request::RecoveryStatus, uid, &mut vec![], &shared))
                .await.unwrap().unwrap(),
            Response::Recovery { issues, .. } if issues[0].detail.contains("in progress")
        ));
        assert!(apply(
            Request::LeaveP2p {
                lease_id: id.clone()
            },
            uid,
            &mut vec![],
            &shared
        )
        .await
        .is_err());
        let mut owned = vec![id.clone()];
        let mut leave_request = Box::pin(apply(
            Request::LeaveP2p {
                lease_id: id.clone(),
            },
            uid,
            &mut owned,
            &shared,
        ));
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(
                std::future::Future::poll(leave_request.as_mut(), cx).is_pending()
            ))
            .await,
            "LeaveP2p must wait on the injected group operation"
        );
        let started = Arc::new(AtomicBool::new(false));
        let observed = started.clone();
        let full = begin_cleanup_with(
            &shared,
            &id,
            move |lease, _| async move {
                // Full retirement must read the post-leave identity instead of
                // disconnecting the now obsolete group a second time.
                assert!(lease.p2p_group.is_none());
                observed.store(true, Ordering::SeqCst);
                Ok(())
            },
            |_| Ok(()),
        )
        .await;
        tokio::task::yield_now().await;
        assert!(!started.load(Ordering::SeqCst));
        assert!(shared.lock().await.leases.contains_key(&id));
        let abandoned = tokio::spawn(wait_cleanup(receipt));
        abandoned.abort();
        let _ = abandoned.await;
        release.send(()).unwrap();
        wait_cleanup(duplicate).await.unwrap();
        assert!(matches!(leave_request.await, Ok(Response::Ok)));
        assert_eq!(owned, vec![id.clone()]);
        wait_cleanup(full).await.unwrap();
        assert!(started.load(Ordering::SeqCst));
        let state = shared.lock().await;
        assert!(state.leases.is_empty());
        assert!(state.cleanups.is_empty() && state.group_cleanups.is_empty());
        assert!(!state.attached.contains(&id));
    }

    #[tokio::test]
    async fn group_cleanup_preserves_direct_lease_and_failed_journal_identity() {
        for (operation_ok, journal_ok) in [(true, true), (true, false), (false, true)] {
            let success = operation_ok && journal_ok;
            let lease = lease_with_group();
            let id = lease.id.clone();
            let shared = Arc::new(Mutex::new(State::default()));
            {
                let mut state = shared.lock().await;
                state.leases.insert(id.clone(), lease);
                state.attached.insert(id.clone());
                state.cancelled_p2p.insert(id.clone());
            }
            let observed_revision = shared.lock().await.network_revision;
            let receipt = begin_group_cleanup_with(
                &shared,
                &id,
                move |_, _| async move {
                    if operation_ok {
                        Ok(())
                    } else {
                        Err("test teardown failure".into())
                    }
                },
                move |_| {
                    if journal_ok {
                        Ok(())
                    } else {
                        Err(io::Error::other("test journal failure"))
                    }
                },
            )
            .await
            .unwrap();
            assert_eq!(wait_cleanup(receipt).await.is_ok(), success);
            let state = shared.lock().await;
            let lease = state
                .leases
                .get(&id)
                .expect("Partial cleanup preserves the direct reservation");
            assert_eq!(lease.kind, LeaseKind::DirectWifi);
            assert_eq!(lease.p2p_group.is_none(), success);
            assert_eq!(state.attached.contains(&id), success);
            if success {
                assert!(!state.observation_is_current(observed_revision, &id),
                    "A completed leave reattaches the lease but must invalidate old watchdog observations");
                assert!(state.observation_is_current(state.network_revision, &id));
            }
            assert_eq!(!state.cancelled_p2p.contains(&id), success);
            assert!(state.group_cleanups.is_empty());
        }
    }

    #[tokio::test]
    async fn cleanup_receipt_waits_for_owned_child_exit() {
        let mut lease = direct_lease();
        lease.kind = LeaseKind::Monitor;
        // A previous-boot synthetic record cannot authorize network mutation.
        lease.boot_id = "synthetic-previous-boot".into();
        let id = lease.id.clone();
        let child = Command::new("/usr/bin/sleep")
            .arg("60")
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        let shared = Arc::new(Mutex::new(State::default()));
        {
            let mut state = shared.lock().await;
            state.leases.insert(id.clone(), lease);
            state.attached.insert(id.clone());
            state.children.insert(id.clone(), child);
        }
        let receipt = begin_cleanup_with(&shared, &id, restore_child_and_lease, |_| Ok(())).await;
        timeout(Duration::from_secs(2), wait_cleanup(receipt))
            .await
            .unwrap()
            .unwrap();
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "Child must be reaped before cleanup success"
        );
        assert!(shared.lock().await.leases.is_empty());
    }

    #[test]
    fn reserved_capabilities_exclude_unsupported_and_regulatory_blocked_roles() {
        use linuxdrop_hardware::Channel;
        let modes = vec![
            "managed".into(),
            "AP".into(),
            "P2P-GO".into(),
            "P2P-client".into(),
        ];
        let mut channels = vec![
            Channel {
                frequency_mhz: 2437,
                number: 6,
                ..Default::default()
            },
            Channel {
                frequency_mhz: 5180,
                number: 36,
                disabled: true,
                ..Default::default()
            },
        ];
        let cap = direct_capabilities(&modes, &channels);
        assert!(cap.station && cap.hotspot && cap.p2p_group_owner && cap.p2p_client);
        assert_eq!(cap.frequencies, vec![2437]);
        channels[0].no_ir = true;
        let cap = direct_capabilities(&modes, &channels);
        assert!(cap.station && cap.p2p_client);
        assert!(!cap.hotspot && !cap.p2p_group_owner);
        channels[0].disabled = true;
        assert!(!direct_capabilities(&modes, &channels).station);
        channels[1].disabled = false;
        let cap = direct_capabilities(&["managed".into()], &channels);
        assert!(cap.station);
        assert!(!cap.hotspot && !cap.p2p_group_owner && !cap.p2p_client);
    }

    #[test]
    fn hosted_subnet_avoids_specific_and_aggregate_routes() {
        use serde_json::json;
        use std::net::Ipv4Addr;
        assert_eq!(
            p2p_subnet(&[json!({"dst":"default"})]).unwrap(),
            Ipv4Addr::new(192, 168, 49, 1)
        );
        assert_eq!(
            p2p_subnet(&[json!({"dst":"192.168.49.12/32"})]).unwrap(),
            Ipv4Addr::new(192, 168, 200, 1)
        );
        assert_eq!(
            p2p_subnet(&[json!({"dst":"192.168.0.0/16"})]).unwrap(),
            Ipv4Addr::new(172, 31, 200, 1)
        );
        assert!(p2p_subnet(&[
            json!({"dst":"192.168.0.0/16"}),
            json!({"dst":"172.16.0.0/12"})
        ])
        .is_err());
        assert!(p2p_subnet(&[json!({"dst":"broken/24"})]).is_err());
        let config = p2p_dhcp_config("p2p-wlan2-0", "lease", Ipv4Addr::new(192, 168, 49, 1));
        assert!(config.contains("interface p2p-wlan2-0\n"));
        assert!(config.contains("start 192.168.49.2\nend 192.168.49.33\n"));
        assert!(
            !config.contains("router") && !config.contains("dns") && !config.contains("domain")
        );
    }
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
            p2p_pending: false,
            p2p_recovery: None,
            direct_capabilities: Default::default(),
        }
    }
    #[tokio::test]
    async fn uncertain_formation_survives_journal_and_blocks_radio_reuse() {
        let mut lease = direct_lease();
        lease.boot_id = std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .unwrap()
            .trim()
            .into();
        lease.p2p_pending = true;
        let bytes = serde_json::to_vec(&lease).unwrap();
        let restored: Lease = serde_json::from_slice(&bytes).unwrap();
        let id = restored.id.clone();
        let shared = Arc::new(Mutex::new(State::default()));
        shared.lock().await.leases.insert(id.clone(), restored);
        let receipt = begin_cleanup_with(
            &shared,
            &id,
            |lease, _| async move { restore_p2p(&lease).await },
            |_| Ok(()),
        )
        .await;
        assert!(wait_cleanup(receipt)
            .await
            .unwrap_err()
            .contains("outcome is unknown"));
        assert!(shared.lock().await.leases[&id].p2p_pending);
        assert!(!shared.lock().await.attached.contains(&id));
        // An old boot cannot still have a pending kernel formation operation.
        lease.boot_id = "previous-boot".into();
        assert!(restore_p2p(&lease).await.is_ok());
        let mut legacy = serde_json::to_value(&lease).unwrap();
        legacy.as_object_mut().unwrap().remove("p2p_pending");
        assert!(!serde_json::from_value::<Lease>(legacy).unwrap().p2p_pending);
    }

    #[tokio::test]
    async fn revoked_reservation_is_recovery_only_and_cannot_change_channel() {
        let lease = direct_lease();
        let id = lease.id.clone();
        let uid = lease.uid;
        let shared = Arc::new(Mutex::new(State::default()));
        {
            let mut state = shared.lock().await;
            state.leases.insert(id.clone(), lease);
            state.attached.insert(id.clone());
        }
        let mut owned = vec![id.clone()];
        match apply(Request::Status, uid, &mut owned, &shared)
            .await
            .unwrap()
        {
            Response::State { leases, .. } => assert_eq!(leases.len(), 1),
            _ => panic!("unexpected status"),
        }
        // This is the revocation boundary used by watchdog owner-loss handling.
        shared.lock().await.attached.remove(&id);
        match apply(Request::Status, uid, &mut owned, &shared)
            .await
            .unwrap()
        {
            Response::State { leases, .. } => assert!(leases.is_empty()),
            _ => panic!("unexpected status"),
        }
        match apply(Request::RecoveryStatus, uid, &mut owned, &shared)
            .await
            .unwrap()
        {
            Response::Recovery { issues, .. } => assert_eq!(issues[0].lease_id, id),
            _ => panic!("missing retained recovery receipt"),
        }
        assert!(apply(
            Request::SetChannel {
                lease_id: id.clone(),
                channel: 6
            },
            uid,
            &mut owned,
            &shared
        )
        .await
        .unwrap_err()
        .contains("revoked"));
        assert!(
            shared.lock().await.leases.contains_key(&id),
            "unconfirmed cleanup must retain the journal record"
        );
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
        let pending = RadioProducer {
            settled: tokio::sync::watch::channel(Some(Ok(()))).1,
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
            service_owner: String::new(),
            bus_guid: String::new(),
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
    async fn abandoned_producer_settles_before_full_retirement() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let lease = direct_lease();
        let id = lease.id.clone();
        let shared = Arc::new(Mutex::new(State::default()));
        let cancel = tokio_util::sync::CancellationToken::new();
        let stopped = cancel.clone().drop_guard();
        let worker_cancel = cancel.clone();
        let worker_shared = shared.clone();
        let worker_id = id.clone();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (finish, finishing) = tokio::sync::oneshot::channel();
        let mut state = shared.lock().await;
        state.leases.insert(id.clone(), lease);
        state.attached.insert(id.clone());
        let reply = spawn_radio_with(
            &mut state,
            &shared,
            id.clone(),
            cancel,
            Default::default(),
            move || async move {
                worker_cancel.cancelled().await;
                let _ = started.send(());
                finishing.await.unwrap();
                // Model a producer reporting its final group after stop was requested.
                worker_shared
                    .lock()
                    .await
                    .leases
                    .get_mut(&worker_id)
                    .unwrap()
                    .p2p_group = lease_with_group().p2p_group;
                Err("cancelled after settlement".into())
            },
        );
        drop(state);
        drop(reply);
        drop(stopped);
        timeout(Duration::from_secs(1), ready)
            .await
            .unwrap()
            .unwrap();
        let restored = Arc::new(AtomicBool::new(false));
        let observed = restored.clone();
        let cleanup = begin_cleanup_with(
            &shared,
            &id,
            move |lease, _| async move {
                assert!(
                    lease.p2p_group.is_some(),
                    "Retirement must use the settled identity"
                );
                observed.store(true, Ordering::SeqCst);
                Ok(())
            },
            |_| Ok(()),
        )
        .await;
        tokio::task::yield_now().await;
        assert!(!restored.load(Ordering::SeqCst));
        assert!(shared.lock().await.leases.contains_key(&id));
        timeout(
            Duration::from_millis(250),
            apply(Request::Status, 1000, &mut vec![], &shared),
        )
        .await
        .unwrap()
        .unwrap();
        finish.send(()).unwrap();
        wait_cleanup(cleanup).await.unwrap();
        assert!(restored.load(Ordering::SeqCst));
        let state = shared.lock().await;
        assert!(state.producers.is_empty() && state.leases.is_empty());
    }

    #[tokio::test]
    async fn channel_retirement_waits_for_command_and_final_journal() {
        let mut lease = direct_lease();
        lease.kind = LeaseKind::Monitor;
        lease.channel = 6;
        let id = lease.id.clone();
        let shared = Arc::new(Mutex::new(State::default()));
        let (started, ready) = tokio::sync::oneshot::channel();
        let (finish, finishing) = tokio::sync::oneshot::channel();
        let mut state = shared.lock().await;
        state.leases.insert(id.clone(), lease.clone());
        state.attached.insert(id.clone());
        let reply = channel::spawn_channel_with(
            &mut state,
            &shared,
            lease,
            11,
            tokio_util::sync::CancellationToken::new(),
            move |_| async move {
                started.send(()).unwrap();
                finishing.await.unwrap();
                Ok(())
            },
            |_| Ok(()),
        );
        drop(state);
        ready.await.unwrap();
        timeout(
            Duration::from_millis(250),
            apply(Request::Status, 1000, &mut vec![], &shared),
        )
        .await
        .unwrap()
        .unwrap();
        let competing = timeout(
            Duration::from_millis(250),
            apply(
                Request::SetChannel {
                    lease_id: id.clone(),
                    channel: 1,
                },
                1000,
                &mut vec![id.clone()],
                &shared,
            ),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert!(competing.contains("still in progress"));
        let cleanup = begin_cleanup_with(
            &shared,
            &id,
            |lease, _| async move {
                assert_eq!(
                    lease.channel, 11,
                    "Retirement must observe the completed mutation"
                );
                Ok(())
            },
            |_| Ok(()),
        )
        .await;
        assert!(cleanup.borrow().is_none());
        finish.send(()).unwrap();
        assert!(reply.await.unwrap().unwrap_err().contains("cancelled"));
        wait_cleanup(cleanup).await.unwrap();
        let state = shared.lock().await;
        assert!(state.leases.is_empty() && state.producers.is_empty() && state.attached.is_empty());
    }

    #[tokio::test]
    async fn failed_channel_mutation_or_journal_revokes_the_lease() {
        for failed_command in [true, false] {
            let mut lease = direct_lease();
            lease.kind = LeaseKind::Monitor;
            lease.channel = 6;
            let id = lease.id.clone();
            let shared = Arc::new(Mutex::new(State::default()));
            let mut state = shared.lock().await;
            state.leases.insert(id.clone(), lease.clone());
            state.attached.insert(id.clone());
            let reply = channel::spawn_channel_with(
                &mut state,
                &shared,
                lease,
                11,
                tokio_util::sync::CancellationToken::new(),
                move |_| async move {
                    if failed_command {
                        Err("Synthetic driver failure".into())
                    } else {
                        Ok(())
                    }
                },
                move |_| {
                    assert!(!failed_command, "Failed mutation must not journal success");
                    Err(io::Error::other("Synthetic disk failure"))
                },
            );
            drop(state);
            assert!(reply.await.unwrap().is_err());
            let state = shared.lock().await;
            assert!(state.producers.is_empty());
            assert!(!state.attached.contains(&id));
            assert_eq!(
                state.leases[&id].channel,
                if failed_command { 6 } else { 11 }
            );
            assert!(!state.recovery_errors.is_empty());
        }
    }

    #[tokio::test]
    async fn timed_out_network_command_is_reaped_before_return() {
        let child = Command::new("/usr/bin/sleep")
            .arg("60")
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        let error = wait_command(child, "sleep fixture", Duration::from_millis(1))
            .await
            .unwrap_err();
        assert!(error.contains("timed out"));
        assert!(!Path::new(&format!("/proc/{pid}")).exists());
    }

    #[tokio::test]
    async fn cancelled_acquisition_retains_late_child_until_retirement() {
        let mut lease = direct_lease();
        lease.kind = LeaseKind::Monitor;
        let id = lease.id.clone();
        let shared = Arc::new(Mutex::new(State::default()));
        let cancel = tokio_util::sync::CancellationToken::new();
        let (finish, finishing) = tokio::sync::oneshot::channel();
        let (spawned, child_pid) = tokio::sync::oneshot::channel();
        let mut state = shared.lock().await;
        state.leases.insert(id.clone(), lease.clone());
        let reply = acquire::spawn_acquisition_with(
            &mut state,
            &shared,
            lease,
            cancel,
            move |_, token| async move {
                token.cancelled().await;
                finishing.await.unwrap();
                let child = Command::new("/usr/bin/sleep")
                    .arg("60")
                    .kill_on_drop(true)
                    .spawn()
                    .unwrap();
                spawned.send(child.id().unwrap()).unwrap();
                Ok(Some(child))
            },
        );
        drop(state);
        drop(reply);
        let cleanup = begin_cleanup_with(
            &shared,
            &id,
            |_, child| async move {
                assert!(child.is_some(), "A late child must be handed to retirement");
                reap_child(child).await
            },
            |_| Ok(()),
        )
        .await;
        timeout(
            Duration::from_millis(250),
            apply(Request::Status, 1000, &mut vec![], &shared),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(shared.lock().await.leases.contains_key(&id));
        assert!(cleanup.borrow().is_none());
        finish.send(()).unwrap();
        let pid = child_pid.await.unwrap();
        timeout(Duration::from_secs(3), wait_cleanup(cleanup))
            .await
            .unwrap()
            .unwrap();
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "Acknowledgement requires child reaping"
        );
        let state = shared.lock().await;
        assert!(state.leases.is_empty() && state.producers.is_empty() && state.children.is_empty());
        assert!(
            !state.attached.contains(&id),
            "A cancelled producer must not attach its lease"
        );
    }

    #[tokio::test]
    async fn failed_acquisition_settles_before_queued_recovery() {
        let mut lease = direct_lease();
        lease.kind = LeaseKind::Monitor;
        let id = lease.id.clone();
        let shared = Arc::new(Mutex::new(State::default()));
        let (finish, finishing) = tokio::sync::oneshot::channel();
        let mut state = shared.lock().await;
        state.leases.insert(id.clone(), lease.clone());
        let reply = acquire::spawn_acquisition_with(
            &mut state,
            &shared,
            lease,
            tokio_util::sync::CancellationToken::new(),
            move |_, _| async move {
                finishing.await.unwrap();
                Err("Synthetic creation failure".into())
            },
        );
        drop(state);
        let cleanup = begin_cleanup_with(
            &shared,
            &id,
            |_, child| async move {
                assert!(child.is_none());
                Err("Synthetic ownership mismatch".into())
            },
            |_| Ok(()),
        )
        .await;
        let recovery = timeout(
            Duration::from_millis(250),
            apply(Request::RecoveryStatus, 1000, &mut vec![], &shared),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            matches!(recovery, Response::Recovery { issues, .. } if issues.len() == 1 && issues[0].detail.contains("in progress"))
        );
        assert!(cleanup.borrow().is_none());
        finish.send(()).unwrap();
        assert!(reply
            .await
            .unwrap()
            .unwrap_err()
            .contains("creation failure"));
        assert!(wait_cleanup(cleanup)
            .await
            .unwrap_err()
            .contains("ownership mismatch"));
        let state = shared.lock().await;
        assert!(state.producers.is_empty());
        assert!(state.leases.contains_key(&id) && !state.attached.contains(&id));
    }

    #[tokio::test]
    async fn panicked_producer_cannot_publish_a_retired_radio() {
        let lease = direct_lease();
        let id = lease.id.clone();
        let shared = Arc::new(Mutex::new(State::default()));
        let mut state = shared.lock().await;
        state.leases.insert(id.clone(), lease);
        state.attached.insert(id.clone());
        let reply = spawn_radio_with(
            &mut state,
            &shared,
            id.clone(),
            tokio_util::sync::CancellationToken::new(),
            Default::default(),
            || async {
                panic!("synthetic producer fault");
            },
        );
        drop(state);
        assert!(reply.await.unwrap().is_err());
        let cleanup = begin_cleanup_with(
            &shared,
            &id,
            |_, _| async {
                panic!("Unacknowledged producer must block reuse");
            },
            |_| Ok(()),
        )
        .await;
        assert!(wait_cleanup(cleanup)
            .await
            .unwrap_err()
            .contains("producer"));
        let state = shared.lock().await;
        assert!(state.leases.contains_key(&id));
        assert!(!state.attached.contains(&id));
    }

    #[tokio::test]
    async fn p2p_producer_can_be_cancelled_only_by_its_authorized_user() {
        let lease = direct_lease();
        let cancel = tokio_util::sync::CancellationToken::new();
        let shared = Arc::new(Mutex::new(State::default()));
        {
            let mut state = shared.lock().await;
            state.attached.insert(lease.id.clone());
            state.leases.insert(lease.id.clone(), lease.clone());
            state.producers.insert(
                lease.id.clone(),
                RadioProducer {
                    settled: tokio::sync::watch::channel(Some(Ok(()))).1,
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
            service_owner: String::new(),
            bus_guid: String::new(),
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
        let (child, addresses) = timeout(
            Duration::from_secs(3),
            start_p2p_network(&lease, &tokio_util::sync::CancellationToken::new()),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(addresses.ipv4.is_none());
        assert!(addresses.ipv6.is_some());
        if let Some(mut child) = child {
            child.kill().await.unwrap();
            child.wait().await.unwrap();
        }
        // No usable addresses: cancellation must reap the real DHCP process
        // before acknowledging failure to the producer/retirement supervisor.
        run_command(
            "/usr/sbin/ip",
            &["-6", "address", "flush", "dev", interface],
            3,
        )
        .await
        .unwrap();
        let cancel = tokio_util::sync::CancellationToken::new();
        let worker_cancel = cancel.clone();
        let worker_lease = lease.clone();
        let worker =
            tokio::spawn(async move { start_p2p_network(&worker_lease, &worker_cancel).await });
        let child_path = timeout(Duration::from_secs(3), async {
            'found: loop {
                for entry in std::fs::read_dir("/proc").unwrap().flatten() {
                    let path = entry.path();
                    let Ok(status) = std::fs::read_to_string(path.join("status")) else {
                        continue;
                    };
                    let parent = format!("PPid:\t{}", std::process::id());
                    if !status.lines().any(|line| line == parent) {
                        continue;
                    }
                    let command = std::fs::read(path.join("cmdline")).unwrap_or_default();
                    let args: Vec<_> = command.split(|byte| *byte == 0).collect();
                    if args.contains(&b"udhcpc".as_slice()) && args.contains(&interface.as_bytes())
                    {
                        break 'found path;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        });
        let child_path = child_path.await.unwrap();
        cancel.cancel();
        let result = timeout(Duration::from_secs(3), worker)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(result, Err(ref error) if error.contains("cancelled")));
        assert!(
            !child_path.exists(),
            "DHCP must be reaped before cancellation settles"
        );
        run_command(
            "/usr/sbin/ip",
            &["link", "set", "dev", interface, "alias", "foreign"],
            3,
        )
        .await
        .unwrap();
        assert!(p2p_addresses(&lease).await.is_err());
        assert!(
            start_p2p_network(&lease, &tokio_util::sync::CancellationToken::new())
                .await
                .is_err()
        );
        run_command("/usr/sbin/ip", &["link", "del", interface], 3)
            .await
            .unwrap();
    }

    #[tokio::test]
    #[ignore = "requires a private network namespace; use run-p2p-host-network.sh"]
    async fn group_owner_serves_dhcp_without_router_or_dns() {
        use std::os::unix::fs::PermissionsExt;
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
        lease.p2p_group = Some(linuxdrop_network::p2p::GroupIdentity {
            service_owner: String::new(),
            bus_guid: String::new(),
            interface: "ld-go0".into(),
            interface_object: "/test".into(),
            group_object: "/group".into(),
            parent_interface: "wlan2".into(),
            peer_object: "/".into(),
        });
        run_command(
            "/usr/sbin/ip",
            &[
                "link", "add", "ld-go0", "type", "veth", "peer", "name", "ld-peer0",
            ],
            3,
        )
        .await
        .unwrap();
        run_command(
            "/usr/sbin/ip",
            &[
                "link",
                "set",
                "ld-go0",
                "alias",
                &format!("linuxdrop:p2p:{}", lease.id),
            ],
            3,
        )
        .await
        .unwrap();
        run_command("/usr/sbin/ip", &["link", "set", "ld-peer0", "up"], 3)
            .await
            .unwrap();
        run_command(
            "/usr/sbin/ip",
            &["route", "add", "blackhole", "192.168.49.0/24"],
            3,
        )
        .await
        .unwrap();
        std::fs::create_dir_all("/run/linuxdrop").unwrap();
        let (server, addresses) =
            start_p2p_host_network(&lease, &tokio_util::sync::CancellationToken::new())
                .await
                .unwrap();
        assert_eq!(addresses.ipv4, Some("192.168.200.1".parse().unwrap()));
        assert!(
            addresses.ipv6.is_some(),
            "group owner must wait for usable link-local IPv6"
        );
        let hook = format!("/run/linuxdrop/{}.test-hook", lease.id);
        let result = format!("/run/linuxdrop/{}.test-result", lease.id);
        std::fs::write(&hook, format!("#!/bin/sh\ncase \"$1\" in bound|renew) printf '%s\\n' \"$ip\" \"$subnet\" \"${{router-unset}}\" \"${{dns-unset}}\" > {result};; esac\n")).unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o700)).unwrap();
        run_command(
            "/usr/bin/busybox",
            &[
                "udhcpc", "-f", "-n", "-q", "-i", "ld-peer0", "-s", &hook, "-t", "3", "-T", "1",
            ],
            8,
        )
        .await
        .unwrap();
        let assigned = std::fs::read_to_string(&result).unwrap();
        let fields: Vec<_> = assigned.lines().collect();
        assert!(fields[0].starts_with("192.168.200."));
        assert_eq!(&fields[1..], &["255.255.255.0", "unset", "unset"]);
        let mut server = server.unwrap();
        server.kill().await.unwrap();
        server.wait().await.unwrap();
        for suffix in [
            "dhcp.conf",
            "dhcp.leases",
            "dhcp.pid",
            "test-hook",
            "test-result",
        ] {
            let _ = std::fs::remove_file(format!("/run/linuxdrop/{}.{suffix}", lease.id));
        }
        run_command("/usr/sbin/ip", &["link", "del", "ld-go0"], 3)
            .await
            .unwrap();
    }

    #[test]
    fn process_start_is_available() {
        assert!(process_start(std::process::id() as i32).unwrap() > 0);
    }
    #[test]
    fn protocol_rejects_unrecognized_mutations() {
        assert!(matches!(
            serde_json::from_str::<Request>(r#"{"operation":"host_p2p","lease_id":"x"}"#).unwrap(),
            Request::HostP2p {
                auth: linuxdrop_network::P2pHostAuth::Password,
                ..
            }
        ));
        assert!(matches!(
            serde_json::from_str::<Request>(
                r#"{"operation":"host_p2p","lease_id":"x","auth":"device_name"}"#
            )
            .unwrap(),
            Request::HostP2p {
                auth: linuxdrop_network::P2pHostAuth::DeviceName,
                ..
            }
        ));
        assert!(serde_json::from_str::<Request>(
            r#"{"operation":"host_p2p","lease_id":"x","auth":"open"}"#
        )
        .is_err());
        assert!(
            serde_json::from_str::<Request>(r#"{"operation":"execute","command":"rm"}"#).is_err()
        );
        assert!(serde_json::from_str::<Request>(
            r#"{"operation":"acquire","radio_id":"x","channel":6,"command":"oops"}"#
        )
        .is_err());
    }
}
