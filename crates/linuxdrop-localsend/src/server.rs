//! Reconcile exact listener addresses without restarting unaffected transfers.
use super::*;
use axum_server::{tls_rustls::RustlsConfig, Handle};
use linuxdrop_network::InterfaceAddress;

struct Listener {
    handle: Handle,
    task: tokio::task::JoinHandle<()>,
    discovery: Option<tokio::task::JoinHandle<Result<()>>>,
}
impl Drop for Listener {
    fn drop(&mut self) {
        self.handle.graceful_shutdown(Some(Duration::from_secs(2)));
        if let Some(task) = &self.discovery {
            task.abort();
        }
    }
}

struct Network {
    state: SharedState,
    routes: Router,
    tls: Option<RustlsConfig>,
    listeners: HashMap<InterfaceAddress, Listener>,
    last_status: Option<(String, String)>,
    tasks: tokio_util::task::TaskTracker,
}
impl Network {
    async fn shutdown(&mut self) {
        for listener in self.listeners.values() {
            listener.handle.shutdown();
        }
        self.listeners.clear(); // Aborts discovery and closes listeners.
        self.tasks.close();
        self.tasks.wait().await; // Includes listeners retired during reconciliation.
    }

    fn reconcile(&mut self, interfaces: Vec<InterfaceAddress>) -> Result<Vec<String>> {
        self.listeners.retain(|interface, listener| {
            interfaces.contains(interface) && !listener.task.is_finished()
        });
        let mut failures = Vec::new();
        for interface in interfaces {
            if !interface.address.is_ipv4() {
                continue;
            }
            if !self.listeners.contains_key(&interface) {
                let result = (|| -> Result<std::net::TcpListener> {
                    let socket = socket2::Socket::new(
                        socket2::Domain::IPV4,
                        socket2::Type::STREAM,
                        Some(socket2::Protocol::TCP),
                    )?;
                    socket.set_reuse_address(true)?;
                    socket.bind_device(Some(interface.name.as_bytes()))?;
                    socket
                        .bind(&SocketAddr::new(interface.address, self.state.config.port).into())?;
                    socket.listen(128)?;
                    socket.set_nonblocking(true)?;
                    Ok(socket.into())
                })();
                let listener = match result {
                    Ok(listener) => listener,
                    Err(error) => {
                        failures.push(format!(
                            "{} ({}): {error}",
                            interface.name, interface.address
                        ));
                        continue;
                    }
                };
                let handle = Handle::new();
                let task_handle = handle.clone();
                let routes = self.routes.clone();
                let tls = self.tls.clone();
                let state = self.state.clone();
                let task = self.tasks.spawn(async move {
                    let result = if let Some(tls) = tls {
                        axum_server::from_tcp_rustls(listener, tls)
                            .handle(task_handle.clone())
                            .serve(routes.into_make_service_with_connect_info::<SocketAddr>())
                            .await
                    } else {
                        axum_server::from_tcp(listener)
                            .handle(task_handle.clone())
                            .serve(routes.into_make_service_with_connect_info::<SocketAddr>())
                            .await
                    };
                    // axum-server returns before detached connections finish on
                    // forced shutdown (including TLS handshakes). Keep this task
                    // alive until their watcher guards have actually been dropped.
                    task_handle.shutdown();
                    while task_handle.connection_count() != 0 {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    if let Err(error) = result {
                        state.error(format!("LocalSend server: {error}")).await;
                    }
                });
                self.listeners.insert(
                    interface.clone(),
                    Listener {
                        handle,
                        task,
                        discovery: None,
                    },
                );
            }
            let entry = self.listeners.get_mut(&interface).unwrap();
            if self.state.config.multicast
                && !interface.loopback
                && entry
                    .discovery
                    .as_ref()
                    .is_none_or(|task| task.is_finished())
            {
                match discovery::socket(&interface, self.state.config.port) {
                    Ok(udp) => {
                        let state = self.state.clone();
                        entry.discovery = Some(self.tasks.spawn(discovery::on_interface(
                            state,
                            interface.clone(),
                            udp,
                        )));
                    }
                    Err(error) => {
                        failures.push(format!("Discovery on {}: {error}", interface.name))
                    }
                }
            }
        }
        // Publish only interfaces whose HTTP listener was actually opened.
        *self.state.interfaces.write().unwrap() = self.listeners.keys().cloned().collect();
        Ok(failures)
    }

    async fn report(&mut self, failures: Vec<String>) {
        let lan_count = self
            .listeners
            .keys()
            .filter(|interface| !interface.loopback)
            .count();
        let status = if !failures.is_empty() {
            ("error".into(), failures.join("; "))
        } else if lan_count == 0 {
            (
                "unavailable".into(),
                "No enabled IPv4 LAN interface; waiting for a network connection".into(),
            )
        } else {
            (
                "ready".into(),
                format!(
                    "{} on port {} · {lan_count} local address(es)",
                    self.state.info.protocol, self.state.config.port
                ),
            )
        };
        if self.last_status.as_ref() != Some(&status) {
            self.last_status = Some(status.clone());
            let _ = self
                .state
                .events
                .send(BackendEvent::StateChanged(BackendState {
                    id: "localsend".into(),
                    state: status.0,
                    detail: status.1,
                }))
                .await;
        }
        let removed = {
            let interfaces = self.state.interfaces();
            let mut peers = self.state.peers.lock().await;
            let removed: Vec<_> = peers
                .iter()
                .filter(|(_, peer)| {
                    !interfaces
                        .iter()
                        .any(|interface| discovery::permits(interface, peer.address.ip()))
                })
                .map(|(id, _)| id.clone())
                .collect();
            for id in &removed {
                peers.remove(id);
            }
            removed
        };
        for peer_id in removed {
            let _ = self
                .state
                .events
                .send(BackendEvent::PeerRemoved { peer_id })
                .await;
        }
    }
}

pub(super) async fn start(
    state: SharedState,
    routes: Router,
    tls: Option<RustlsConfig>,
    bind: Ipv4Addr,
) -> Result<tokio::task::JoinHandle<()>> {
    let selected = move |state: &SharedState| -> Result<Vec<InterfaceAddress>> {
        Ok(linuxdrop_network::interfaces(&state.config.policy, true)?
            .into_iter()
            .filter(|interface| {
                interface.address.is_ipv4()
                    && (bind.is_unspecified() || interface.address == IpAddr::V4(bind))
            })
            .collect())
    };
    let mut network = Network {
        state: state.clone(),
        routes,
        tls,
        listeners: HashMap::new(),
        last_status: None,
        tasks: tokio_util::task::TaskTracker::new(),
    };
    let failures = network.reconcile(selected(&state)?)?;
    // At startup, an occupied configured port is actionable, not silently ignored.
    if !failures.is_empty() {
        network.shutdown().await;
        bail!("{}", failures.join("; "));
    }
    Ok(tokio::spawn(async move {
        network.report(failures).await;
        let mut changes = tokio::time::interval(Duration::from_secs(3));
        changes.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = state.stop.cancelled() => break,
                _ = changes.tick() => {
                    match selected(&state).and_then(|interfaces| network.reconcile(interfaces)) {
                        Ok(failures) => network.report(failures).await,
                        Err(error) => state.error(format!("LocalSend network update: {error}")).await,
                    }
                }
            }
        }
        network.shutdown().await;
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shutdown_waits_for_an_accepted_tls_connection_before_rebinding() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let root = tempfile::tempdir().unwrap();
        let state = discovery::tests::local_state(root.path());
        let interface = state
            .interfaces()
            .into_iter()
            .find(|entry| entry.loopback && entry.address.is_ipv4())
            .unwrap();
        let address = SocketAddr::new(interface.address, state.config.port);
        let (cert, key, _) = tls::identity(&state.config.identity_dir).unwrap();
        let mut network = Network {
            state,
            routes: Router::new(),
            tls: Some(RustlsConfig::from_pem(cert, key).await.unwrap()),
            listeners: HashMap::new(),
            last_status: None,
            tasks: tokio_util::task::TaskTracker::new(),
        };
        assert!(network
            .reconcile(vec![interface.clone()])
            .unwrap()
            .is_empty());
        // TCP is accepted, but the peer deliberately does not finish TLS.
        let socket = tokio::net::TcpStream::connect(address).await.unwrap();
        let handle = network.listeners[&interface].handle.clone();
        tokio::time::timeout(Duration::from_secs(2), async {
            while handle.connection_count() == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let shutdown = tokio::spawn(async move {
            network.shutdown().await;
        });
        tokio::task::yield_now().await;
        assert!(!shutdown.is_finished());
        drop(socket);
        tokio::time::timeout(Duration::from_secs(2), shutdown)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(handle.connection_count(), 0);
        let _rebound = tokio::net::TcpListener::bind(address).await.unwrap();
    }

    #[tokio::test]
    async fn address_changes_preserve_other_connections_and_remove_stale_peers() {
        let root = tempfile::tempdir().unwrap();
        let state = discovery::tests::local_state(root.path());
        let first = state
            .interfaces()
            .into_iter()
            .find(|interface| interface.address == IpAddr::V4(Ipv4Addr::LOCALHOST))
            .unwrap();
        let second = InterfaceAddress {
            address: "127.0.0.2".parse().unwrap(),
            ..first.clone()
        };
        let routes = Router::new().route("/test", get(|| async { "network still connected" }));
        let mut network = Network {
            state: state.clone(),
            routes,
            tls: None,
            listeners: HashMap::new(),
            last_status: None,
            tasks: tokio_util::task::TaskTracker::new(),
        };
        assert!(network.reconcile(vec![first.clone()]).unwrap().is_empty());
        let mut retained = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, state.config.port))
            .await
            .unwrap();
        assert!(network
            .reconcile(vec![first.clone(), second.clone()])
            .unwrap()
            .is_empty());
        let probe =
            tokio::net::TcpStream::connect((Ipv4Addr::new(127, 0, 0, 2), state.config.port))
                .await
                .unwrap();
        drop(probe);
        assert!(network.reconcile(vec![first.clone()]).unwrap().is_empty());
        // A connection accepted before reconciliation remains usable afterward.
        retained
            .write_all(b"GET /test HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .await
            .unwrap();
        let mut response = String::new();
        tokio::time::timeout(
            Duration::from_secs(2),
            retained.read_to_string(&mut response),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(response.contains("network still connected"));
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if tokio::net::TcpStream::connect((Ipv4Addr::new(127, 0, 0, 2), state.config.port))
                    .await
                    .is_err()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let peer = DeviceInfo {
            fingerprint: "removed-peer".into(),
            ..state.info.clone()
        };
        state
            .remember(peer, "127.0.0.3".parse().unwrap())
            .await
            .unwrap();
        assert_eq!(state.peers.lock().await.len(), 1);
        assert!(network.reconcile(vec![]).unwrap().is_empty());
        network.report(vec![]).await;
        assert!(state.peers.lock().await.is_empty());
        assert!(state.interfaces().is_empty());
        assert_eq!(network.last_status.as_ref().unwrap().0, "unavailable");
        // Removing all networks does not tear down the backend owner; it can
        // open a newly appearing address on the following reconciliation.
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(network.reconcile(vec![second]).unwrap().is_empty());
        assert!(
            tokio::net::TcpStream::connect((Ipv4Addr::new(127, 0, 0, 2), state.config.port))
                .await
                .is_ok()
        );
    }
}
