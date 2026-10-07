//! Radio lease health belongs to one backend generation, including during startup.
use super::*;
use linuxdrop_netd::{Client, Lease, Request, Response};
use std::sync::atomic::Ordering;

pub(super) struct HelperLease {
    pub client: Client,
    pub expected: Lease,
    pub generation: u64,
}

impl HelperLease {
    pub fn new(client: Client, expected: Lease, shared: &Shared) -> Self {
        Self {
            client,
            expected,
            generation: shared.backend_generation.load(Ordering::Acquire),
        }
    }

    fn matches(&self, lease: &Lease) -> bool {
        let expected = &self.expected;
        lease.id == expected.id
            && lease.uid == expected.uid
            && lease.boot_id == expected.boot_id
            && lease.kind == expected.kind
            && lease.phy == expected.phy
            && lease.interface == expected.interface
            && lease.awdl_interface == expected.awdl_interface
    }
}

/// Each backend owns its health loop. A direct-Wi-Fi operation can hold its
/// helper socket for minutes and must not postpone AWDL failure detection.
pub(super) fn watch(
    shared: Arc<Shared>,
    stop: tokio_util::sync::CancellationToken,
) -> Vec<tokio::task::JoinHandle<()>> {
    ["airdrop", "quickshare"]
        .into_iter()
        .map(|backend| {
            let shared = shared.clone();
            let stop = stop.clone();
            tokio::spawn(async move {
                let socket = match backend {
                    "airdrop" => &shared.helper,
                    _ => &shared.quickshare_helper,
                };
                let mut tick = tokio::time::interval(Duration::from_secs(5));
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tokio::select! {
                        biased;
                        _ = stop.cancelled() => break,
                        _ = tick.tick() => {}
                    }
                    // Await a started request completely. Cancelling a framed
                    // request could leave its reply ahead of the Release reply.
                    poll(&shared, backend, socket).await;
                }
            })
        })
        .collect()
}

pub(super) async fn poll(shared: &Arc<Shared>, backend: &str, socket: &Mutex<Option<HelperLease>>) {
    let mut slot = socket.lock().await;
    let Some(lease) = slot.as_mut() else {
        return;
    };
    let response = tokio::time::timeout(
        Duration::from_secs(5),
        lease.client.request(&Request::Status),
    )
    .await;
    if matches!(response, Ok(Ok(Response::State { leases, .. })) if leases.iter().any(|item| lease.matches(item)))
    {
        return;
    }
    let generation = lease.generation;
    slot.take();
    drop(slot);
    failed(shared, backend, generation).await;
}

const FAILURE: &str = "Network helper is unavailable. The sharing service stopped; radio cleanup may still be running. Reconnect the adapter and restart sharing services.";

pub(super) async fn release(socket: &Mutex<Option<HelperLease>>) -> Result<()> {
    let Some(mut lease) = socket.lock().await.take() else {
        return Ok(());
    };
    match tokio::time::timeout(
        Duration::from_secs(45),
        lease.client.request(&Request::Release {
            lease_id: lease.expected.id,
        }),
    )
    .await
    .context("Network helper did not confirm radio restoration within 45 seconds")??
    {
        Response::Ok => Ok(()),
        Response::Error { message } => anyhow::bail!(message),
        _ => anyhow::bail!("Unexpected network helper release response"),
    }
}

/// A failed startup must not retain a lease until a later manual restart.
/// Preserve both the startup error and an unconfirmed restoration in the UI.
pub(super) async fn release_after_failure<T>(
    socket: &Mutex<Option<HelperLease>>,
    result: Result<T>,
) -> Result<T> {
    match result {
        Ok(value) => Ok(value),
        Err(error) => match release(socket).await {
            Ok(()) => Err(error),
            Err(cleanup) => Err(anyhow::anyhow!(
                "{error}; radio restoration was not confirmed: {cleanup}"
            )),
        },
    }
}

pub(super) async fn failed(shared: &Arc<Shared>, backend: &str, generation: u64) {
    let mut data = shared.data.lock().await;
    if generation != shared.backend_generation.load(Ordering::Acquire)
        || !data.failed_backends.insert(backend.into())
    {
        return;
    }
    let commands = data.commands.remove(backend);
    if let Some(commands) = &commands {
        data.retiring.insert(backend.into(), commands.clone());
    }
    data.peers.retain(|_, peer| {
        peer.protocols.retain(|protocol| protocol != backend);
        !peer.protocols.is_empty()
    });
    let mut finished = Vec::new();
    for transfer in data.transfers.values_mut() {
        if transfer.protocol == backend && !transfer.is_terminal() {
            transfer.state = "failed".into();
            transfer.error = Some(FAILURE.into());
            finished.push(transfer.id.clone());
        }
    }
    for id in &finished {
        data.decisions.remove(id);
        data.completed_at.insert(id.clone(), history::now());
    }
    let notifications: Vec<_> = finished
        .iter()
        .filter_map(|id| data.transfers.get(id).cloned())
        .collect();
    data.backends.insert(
        backend.into(),
        BackendState {
            id: backend.into(),
            state: "error".into(),
            detail: FAILURE.into(),
        },
    );
    drop(data);
    // A P2P actor may be awaiting this failure result itself. Never await its
    // shutdown here, or hold Data while draining. Retain the receipt for restart.
    if let Some(commands) = commands {
        drain(backend.into(), commands);
    }
    shared.changed().await;
    for transfer in notifications {
        let shared = shared.clone();
        tokio::spawn(async move {
            notifications::notify(shared, transfer).await;
        });
    }
    if !finished.is_empty() {
        if let Err(error) = shared.persist_history().await {
            tracing::warn!(%error, "Could not persist transfers after helper loss");
        }
    }
}

pub(super) fn drain(backend: String, commands: CommandSender) {
    tokio::spawn(async move {
        if let Err(error) = commands.shutdown().await {
            tracing::warn!(%backend, %error, "Helper-loss cleanup is incomplete; restart will retry");
        }
    });
}

/// Already queued discovery/ready/progress events cannot revive a failed actor.
/// Incoming offers still reach the rejection path; terminal updates remain valid.
pub(super) fn filter_event(data: &Data, event: BackendEvent) -> Option<BackendEvent> {
    match event {
        BackendEvent::PeerUpsert(mut peer) => {
            peer.protocols
                .retain(|protocol| !data.failed_backends.contains(protocol));
            (!peer.protocols.is_empty()).then_some(BackendEvent::PeerUpsert(peer))
        }
        BackendEvent::StateChanged(state) if data.failed_backends.contains(&state.id) => None,
        BackendEvent::TransferUpdated(transfer)
            if data.failed_backends.contains(&transfer.protocol) && !transfer.is_terminal() =>
        {
            None
        }
        event => Some(event),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::sync::oneshot;

    pub(crate) struct Fixture(pub(crate) Arc<Shared>);
    impl Fixture {
        pub(crate) fn new() -> Self {
            let directory =
                std::env::temp_dir().join(format!("linuxdrop-helper-{}", Uuid::new_v4()));
            std::fs::create_dir(&directory).unwrap();
            let (_, log_filter) =
                tracing_subscriber::reload::Layer::new(tracing_subscriber::EnvFilter::new("off"));
            let (restart, _) = mpsc::channel(1);
            Self(Arc::new(Shared {
                bandwidth: Mutex::new(linuxdrop_network::BandwidthLimiter::new(None)),
                data: Mutex::new(Data {
                    epoch: Uuid::new_v4().to_string(),
                    revision: 0,
                    settings: settings::defaults(),
                    peers: HashMap::new(),
                    peer_preferences: HashMap::new(),
                    transfers: HashMap::new(),
                    transfer_order: Vec::new(),
                    completed_at: HashMap::new(),
                    backends: HashMap::new(),
                    hardware: linuxdrop_ipc::HardwareStatus::default(),
                    drafts: HashMap::new(),
                    commands: HashMap::new(),
                    retiring: HashMap::new(),
                    failed_backends: HashSet::new(),
                    visibility_since: None,
                    decisions: HashSet::new(),
                    stop_when_idle: false,
                    restarting: false,
                }),
                connection: std::sync::OnceLock::new(),
                config_path: directory.join("settings.json"),
                data_dir: directory,
                restart,
                helper: Mutex::new(None),
                quickshare_helper: Mutex::new(None),
                backend_generation: 1.into(),
                event_forwarders: tokio_util::task::TaskTracker::new(),
                locked: false.into(),
                download_offer: Mutex::new(None),
                history_io: Mutex::new(()),
                log_filter,
            }))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0.data_dir).unwrap();
        }
    }

    fn lease() -> Lease {
        Lease {
            id: "our-lease".into(),
            uid: 1000,
            phy: "phy1".into(),
            interface: "wlan1".into(),
            channel: 6,
            boot_id: "boot-one".into(),
            awdl_interface: Some("awdl0".into()),
            allowed_frequencies: vec![2437],
            kind: linuxdrop_netd::LeaseKind::Monitor,
            connection_uuid: None,
            p2p_group: None,
            p2p_pending: false,
            direct_capabilities: Default::default(),
        }
    }

    fn status_client(responses: Vec<Vec<Lease>>) -> Client {
        let (client, server) = tokio::net::UnixStream::pair().unwrap();
        tokio::spawn(async move {
            let (read, mut write) = server.into_split();
            let mut read = BufReader::new(read);
            for leases in responses {
                let mut line = String::new();
                read.read_line(&mut line).await.unwrap();
                assert!(matches!(
                    serde_json::from_str::<Request>(&line).unwrap(),
                    Request::Status
                ));
                let response = Response::State {
                    leases,
                    recovery_errors: vec![],
                };
                let mut bytes = serde_json::to_vec(&response).unwrap();
                bytes.push(b'\n');
                write.write_all(&bytes).await.unwrap();
            }
        });
        Client::from_stream(client)
    }

    fn actor() -> (CommandSender, oneshot::Receiver<()>, oneshot::Sender<()>) {
        let (commands, mut receiver) = mpsc::channel(8);
        let (entered, entering) = oneshot::channel();
        let (release, released) = oneshot::channel();
        let task = tokio::spawn(async move {
            assert!(matches!(
                receiver.recv().await,
                Some(BackendCommand::Shutdown)
            ));
            entered.send(()).unwrap();
            released.await.unwrap();
            Ok(())
        });
        (CommandSender::track(commands, task), entering, release)
    }

    fn peer(protocols: &[&str]) -> Peer {
        Peer {
            id: "peer".into(),
            name: "Phone".into(),
            platform: "test".into(),
            protocols: protocols.iter().map(|s| (*s).into()).collect(),
            address: "::1".into(),
            available: true,
        }
    }
    fn transfer() -> Transfer {
        Transfer {
            id: "transfer".into(),
            peer_id: "peer".into(),
            peer_name: "Phone".into(),
            protocol: "airdrop".into(),
            direction: "incoming".into(),
            state: "transferring".into(),
            files: vec![],
            total_bytes: 10,
            transferred_bytes: 5,
            error: None,
            verification_code: None,
            saved_paths: vec![],
        }
    }

    #[tokio::test]
    async fn failed_backend_start_waits_for_radio_release_and_reports_cleanup_failure() {
        for (backend, cleanup_failure) in [("quickshare", false), ("airdrop", true)] {
            let fixture = Fixture::new();
            let shared = &fixture.0;
            let (client, server) = tokio::net::UnixStream::pair().unwrap();
            let (entered, entering) = oneshot::channel();
            let (finish, finished) = oneshot::channel();
            let server_task = tokio::spawn(async move {
                let (read, mut write) = server.into_split();
                let mut read = BufReader::new(read);
                let mut line = String::new();
                read.read_line(&mut line).await.unwrap();
                assert!(
                    matches!(serde_json::from_str::<Request>(&line).unwrap(), Request::Release { lease_id } if lease_id == "our-lease")
                );
                entered.send(()).unwrap();
                finished.await.unwrap();
                let response = if cleanup_failure {
                    Response::Error {
                        message: "restoration test failure".into(),
                    }
                } else {
                    Response::Ok
                };
                let mut bytes = serde_json::to_vec(&response).unwrap();
                bytes.push(b'\n');
                write.write_all(&bytes).await.unwrap();
            });
            let socket = if backend == "quickshare" {
                &shared.quickshare_helper
            } else {
                &shared.helper
            };
            *socket.lock().await = Some(HelperLease::new(
                Client::from_stream(client),
                lease(),
                shared,
            ));
            let task = {
                let shared = shared.clone();
                tokio::spawn(async move {
                    install_backend(
                        &shared,
                        backend,
                        Err(anyhow::anyhow!("listener startup failed")),
                    )
                    .await;
                })
            };
            entering.await.unwrap();
            assert!(
                !task.is_finished(),
                "Startup completion must wait for restoration acknowledgement"
            );
            assert!(
                shared.data.try_lock().is_ok(),
                "Radio cleanup must not block snapshot access"
            );
            finish.send(()).unwrap();
            task.await.unwrap();
            server_task.await.unwrap();
            assert!(socket.lock().await.is_none());
            let data = shared.data.lock().await;
            assert!(!data.commands.contains_key(backend));
            let state = &data.backends[backend];
            assert_eq!(state.state, "error");
            assert!(state.detail.contains("listener startup failed"));
            assert_eq!(
                state.detail.contains("restoration test failure"),
                cleanup_failure
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires the private network namespace in run-awdl-lifecycle.sh"]
    async fn actual_airdrop_actor_retires_after_helper_loss_and_reopens_on_reconnect() {
        assert_eq!(
            std::env::var("LINUXDROP_TEST_PRIVATE_AWDL").as_deref(),
            Ok("1")
        );
        assert_ne!(
            std::fs::read_link("/proc/self/ns/net").unwrap(),
            std::fs::read_link("/proc/1/ns/net").unwrap(),
        );
        async fn ip(args: &[&str]) {
            let output = tokio::process::Command::new("ip")
                .args(args)
                .kill_on_drop(true)
                .output()
                .await
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        ip(&["link", "set", "lo", "up"]).await;
        let fixture = Fixture::new();
        let shared = &fixture.0;
        let config = || linuxdrop_airdrop::Config {
            name: "AWDL lifecycle fixture".into(),
            download_dir: shared.data_dir.clone(),
            interface: "ldawdltest".into(),
            visible: false,
            ble_wake: false,
            max_receive_bytes: 1024 * 1024,
            max_files: 4,
            policy: Default::default(),
        };
        // The first generation sees explicit lease revocation, the replacement
        // sees helper socket EOF. Both use the actual AirDrop listener, discovery
        // worker and shutdown receipt, not a stand-in backend actor.
        for generation in 1..=2 {
            ip(&["link", "add", "ldawdltest", "type", "dummy"]).await;
            ip(&["link", "set", "dev", "ldawdltest", "addrgenmode", "none"]).await;
            ip(&["link", "set", "ldawdltest", "up"]).await;
            ip(&[
                "-6",
                "addr",
                "add",
                "fd42:77::1/64",
                "dev",
                "ldawdltest",
                "nodad",
            ])
            .await;
            shared
                .backend_generation
                .store(generation, Ordering::Release);
            shared.data.lock().await.failed_backends.clear();
            let (events, mut incoming) = mpsc::channel(64);
            let commands = linuxdrop_airdrop::start(config(), events.clone())
                .await
                .unwrap();
            install_backend(shared, "airdrop", Ok(commands.clone())).await;
            let ready = tokio::time::timeout(Duration::from_secs(3), incoming.recv())
                .await
                .unwrap()
                .unwrap();
            assert!(matches!(&ready, BackendEvent::StateChanged(state) if state.state == "ready"));
            handle_event(shared, generation, ready.clone()).await;
            assert_eq!(shared.data.lock().await.backends["airdrop"].state, "ready");
            drop(
                tokio::net::TcpStream::connect("[fd42:77::1]:8771")
                    .await
                    .unwrap(),
            );
            let mut expected = lease();
            expected.awdl_interface = Some("ldawdltest".into());
            let mut replies = vec![vec![expected.clone()]];
            if generation == 1 {
                replies.push(vec![]);
            }
            *shared.helper.lock().await =
                Some(HelperLease::new(status_client(replies), expected, shared));
            poll(shared, "airdrop", &shared.helper).await;
            assert!(shared.data.lock().await.commands.contains_key("airdrop"));
            ip(&["link", "del", "ldawdltest"]).await;
            poll(shared, "airdrop", &shared.helper).await;
            tokio::time::timeout(Duration::from_secs(8), commands.shutdown())
                .await
                .unwrap()
                .unwrap();
            handle_event(shared, generation, ready).await;
            while let Ok(event) = incoming.try_recv() {
                handle_event(shared, generation, event).await;
            }
            assert_eq!(shared.data.lock().await.backends["airdrop"].state, "error");
            assert!(!shared.data.lock().await.commands.contains_key("airdrop"));
            assert!(linuxdrop_airdrop::start(config(), events).await.is_err());
            shared.data.lock().await.retiring.remove("airdrop");
        }
    }

    #[tokio::test]
    async fn awdl_loss_is_detected_while_direct_wifi_owns_its_socket() {
        let fixture = Fixture::new();
        let shared = &fixture.0;
        *shared.helper.lock().await = Some(HelperLease::new(
            status_client(vec![vec![lease()], vec![]]),
            lease(),
            shared,
        ));
        let (commands, entering, release) = actor();
        shared
            .data
            .lock()
            .await
            .commands
            .insert("airdrop".into(), commands.clone());
        // Model an in-flight P2P negotiation without releasing its socket. The
        // first AWDL status is healthy; the next must still observe its loss.
        let direct_operation = shared.quickshare_helper.lock().await;
        let stop = tokio_util::sync::CancellationToken::new();
        let watches = watch(shared.clone(), stop.clone());
        tokio::time::timeout(Duration::from_secs(8), entering)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(shared.data.lock().await.backends["airdrop"].state, "error");
        assert!(shared.helper.lock().await.is_none());
        assert!(!watches[1].is_finished());
        stop.cancel();
        drop(direct_operation);
        release.send(()).unwrap();
        commands.shutdown().await.unwrap();
        for worker in watches {
            tokio::time::timeout(Duration::from_secs(1), worker)
                .await
                .unwrap()
                .unwrap();
        }
    }

    #[tokio::test]
    async fn exact_lease_loss_invalidates_peers_and_transfers_but_retains_cleanup() {
        let fixture = Fixture::new();
        let shared = &fixture.0;
        let expected = lease();
        let mut unrelated = expected.clone();
        unrelated.id = "another-lease-for-the-same-user".into();
        let client = status_client(vec![vec![expected.clone()], vec![unrelated]]);
        *shared.helper.lock().await = Some(HelperLease::new(client, expected, shared));
        let (commands, entering, release) = actor();
        {
            let mut data = shared.data.lock().await;
            data.commands.insert("airdrop".into(), commands.clone());
            data.peers
                .insert("peer".into(), peer(&["airdrop", "localsend"]));
            let mut exclusive = peer(&["airdrop"]);
            exclusive.id = "exclusive".into();
            data.peers.insert(exclusive.id.clone(), exclusive);
            data.transfers.insert("transfer".into(), transfer());
            data.transfer_order.push("transfer".into());
            data.decisions.insert("transfer".into());
        }
        poll(shared, "airdrop", &shared.helper).await;
        assert!(shared.data.lock().await.commands.contains_key("airdrop"));
        poll(shared, "airdrop", &shared.helper).await;
        entering.await.unwrap();
        {
            let data = tokio::time::timeout(Duration::from_secs(1), shared.data.lock())
                .await
                .unwrap();
            assert!(!data.commands.contains_key("airdrop"));
            assert!(data.retiring.contains_key("airdrop"));
            assert_eq!(data.backends["airdrop"].state, "error");
            assert_eq!(data.peers["peer"].protocols, ["localsend"]);
            assert!(!data.peers.contains_key("exclusive"));
            assert_eq!(data.transfers["transfer"].state, "failed");
            assert_eq!(data.transfers["transfer"].transferred_bytes, 5);
            assert!(!data.decisions.contains("transfer"));
            assert!(data.completed_at.contains_key("transfer"));
            assert!(filter_event(&data, BackendEvent::PeerUpsert(peer(&["airdrop"]))).is_none());
            assert!(filter_event(&data, BackendEvent::TransferUpdated(transfer())).is_none());
            assert!(filter_event(
                &data,
                BackendEvent::StateChanged(BackendState {
                    id: "airdrop".into(),
                    state: "ready".into(),
                    detail: "stale".into(),
                })
            )
            .is_none());
        }
        let history = tokio::fs::read(shared.data_dir.join("history.json"))
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&history).unwrap()[0]["state"],
            "failed"
        );
        release.send(()).unwrap();
        commands.shutdown().await.unwrap();
        assert!(shared.data.lock().await.retiring.contains_key("airdrop"));
    }

    #[tokio::test]
    async fn old_health_result_cannot_stop_replacement_generation() {
        let fixture = Fixture::new();
        let shared = &fixture.0;
        *shared.helper.lock().await = Some(HelperLease::new(
            status_client(vec![vec![]]),
            lease(),
            shared,
        ));
        let mut data = shared.data.lock().await;
        let polling = {
            let shared = shared.clone();
            tokio::spawn(async move { poll(&shared, "airdrop", &shared.helper).await })
        };
        // Force the response to finish before generation replacement, while its
        // application is blocked on Data. This is the formerly destructive race.
        tokio::time::timeout(Duration::from_secs(1), async {
            while shared.helper.lock().await.is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        shared.backend_generation.store(2, Ordering::Release);
        let (commands, mut entering, release) = actor();
        data.commands.insert("airdrop".into(), commands.clone());
        data.peers.insert("peer".into(), peer(&["airdrop"]));
        data.backends.insert(
            "airdrop".into(),
            BackendState {
                id: "airdrop".into(),
                state: "ready".into(),
                detail: "new".into(),
            },
        );
        *shared.helper.lock().await =
            Some(HelperLease::new(status_client(vec![]), lease(), shared));
        drop(data);
        polling.await.unwrap();
        let data = shared.data.lock().await;
        assert!(data.commands.contains_key("airdrop"));
        assert!(data.peers.contains_key("peer"));
        assert_eq!(data.backends["airdrop"].state, "ready");
        assert!(data.failed_backends.is_empty());
        assert!(data.retiring.is_empty());
        assert!(matches!(
            entering.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
        drop(data);
        release.send(()).unwrap();
        commands.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn helper_failure_during_startup_quarantines_late_actor() {
        let fixture = Fixture::new();
        let shared = &fixture.0;
        failed(shared, "quickshare", 1).await;
        let (commands, entering, release) = actor();
        install_backend(shared, "quickshare", Ok(commands.clone())).await;
        entering.await.unwrap();
        let data = shared.data.lock().await;
        assert!(!data.commands.contains_key("quickshare"));
        assert!(data.retiring.contains_key("quickshare"));
        assert_eq!(data.backends["quickshare"].state, "error");
        drop(data);
        release.send(()).unwrap();
        commands.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn p2p_io_failure_marks_service_failed_and_stale_connector_cannot_use_new_lease() {
        let fixture = Fixture::new();
        let shared = &fixture.0;
        let (client, server) = tokio::net::UnixStream::pair().unwrap();
        drop(server);
        *shared.quickshare_helper.lock().await = Some(HelperLease::new(
            Client::from_stream(client),
            lease(),
            shared,
        ));
        let mut connector = HelperP2p {
            shared: Arc::downgrade(shared),
            lease_id: "old-lease".into(),
        };
        assert!(connector
            .request_inner(Request::LeaveP2p {
                lease_id: "old-lease".into()
            })
            .await
            .is_err());
        assert!(shared.quickshare_helper.lock().await.is_some());
        assert!(shared.data.lock().await.failed_backends.is_empty());
        connector.lease_id = "our-lease".into();
        assert!(connector
            .request_inner(Request::LeaveP2p {
                lease_id: "our-lease".into()
            })
            .await
            .is_err());
        assert!(shared.quickshare_helper.lock().await.is_none());
        assert!(shared
            .data
            .lock()
            .await
            .failed_backends
            .contains("quickshare"));
    }
}
