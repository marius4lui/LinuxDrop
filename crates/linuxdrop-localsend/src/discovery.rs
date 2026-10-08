//! LocalSend announcements and HTTP-first replies, scoped to one allowed LAN.
use super::*;
use futures_util::stream::FuturesUnordered;
use linuxdrop_network::InterfaceAddress;

const GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 167);

pub(super) fn permits(interface: &InterfaceAddress, peer: IpAddr) -> bool {
    if peer.is_unspecified() || peer.is_multicast() || !interface.contains(peer) {
        return false;
    }
    if let (IpAddr::V4(local), IpAddr::V4(mask), IpAddr::V4(peer)) =
        (interface.address, interface.netmask, peer)
    {
        // /31 and /32 have no conventional network/broadcast host exclusion.
        let host_bits = !u32::from(mask);
        if host_bits > 1 {
            let network = u32::from(local) & u32::from(mask);
            return u32::from(peer) != network && u32::from(peer) != network | host_bits;
        }
    }
    true
}

pub(super) fn socket(interface: &InterfaceAddress, port: u16) -> Result<tokio::net::UdpSocket> {
    let IpAddr::V4(address) = interface.address else {
        bail!("LocalSend multicast requires an IPv4 interface");
    };
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )?;
    socket.set_reuse_address(true)?;
    socket.set_reuse_port(true)?;
    // Wildcard is required for multicast delivery, but SO_BINDTODEVICE keeps
    // both received packets and outgoing responses on this specific adapter.
    socket.bind_device(Some(interface.name.as_bytes()))?;
    socket.bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, port)).into())?;
    socket.set_multicast_all_v4(false)?;
    socket.set_multicast_if_v4(&address)?;
    socket.join_multicast_v4(&GROUP, &address)?;
    socket.set_multicast_ttl_v4(1)?;
    socket.set_nonblocking(true)?;
    Ok(tokio::net::UdpSocket::from_std(socket.into())?)
}

pub(super) async fn on_interface(
    s: SharedState,
    interface: InterfaceAddress,
    udp: tokio::net::UdpSocket,
) -> Result<()> {
    let udp = Arc::new(udp);
    let mut interval = tokio::time::interval(Duration::from_secs(20));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let target = SocketAddr::from((GROUP, s.config.port));
    let mut buffer = vec![0u8; 16 * 1024];
    let mut replies = FuturesUnordered::new();
    let gate = rate::RequestGate::new(6, Duration::from_secs(60));
    loop {
        tokio::select! {
            biased;
            _ = s.stop.cancelled() => break,
            _ = replies.next(), if !replies.is_empty() => {},
            _ = interval.tick() => {
                if s.visible.load(Ordering::Relaxed) {
                    let mut info = s.info.clone();
                    info.announce = true;
                    udp.send_to(&serde_json::to_vec(&info)?, target).await?;
                }
            },
            packet = udp.recv_from(&mut buffer) => {
                let (length, addr) = packet?;
                if length == buffer.len() || !permits(&interface, addr.ip()) || !gate.allow(addr.ip()) {
                    continue;
                }
                if let Ok(info) = serde_json::from_slice::<DeviceInfo>(&buffer[..length]) {
                    if info.fingerprint == s.info.fingerprint { continue; }
                    let announce = info.announce;
                    if s.remember(info.clone(), addr.ip()).await.is_ok()
                        && announce && s.visible.load(Ordering::Relaxed) && replies.len() < 8
                    {
                        replies.push(reply(s.clone(), info, addr.ip(), interface.name.clone(), udp.clone(), target));
                    }
                }
            }
        }
    }
    // Dropping pending replies cancels all their HTTP requests, without tasks
    // surviving backend shutdown or retaining sockets.
    Ok(())
}

async fn register_http(
    local: &DeviceInfo,
    remote: &DeviceInfo,
    ip: IpAddr,
    interface: &str,
) -> Result<()> {
    let client = tls::client_on(&remote.protocol, &remote.fingerprint, Some(interface))?;
    let address = SocketAddr::new(ip, remote.port);
    let response = client
        .post(format!(
            "{}://{address}/api/localsend/v2/register",
            remote.protocol
        ))
        .timeout(Duration::from_secs(3))
        .json(local)
        .send()
        .await?;
    if !response.status().is_success() {
        bail!("LocalSend registration was not accepted");
    }
    // Registration already learned this peer from its announcement. No response
    // body is needed, so an untrusted peer cannot force an unbounded allocation.
    Ok(())
}

async fn reply(
    s: SharedState,
    remote: DeviceInfo,
    ip: IpAddr,
    interface: String,
    udp: Arc<tokio::net::UdpSocket>,
    target: SocketAddr,
) {
    if !s.visible.load(Ordering::Relaxed) {
        return;
    }
    if register_http(&s.info, &remote, ip, &interface)
        .await
        .is_err()
        && s.visible.load(Ordering::Relaxed)
        && !s.stop.is_cancelled()
    {
        if let Ok(body) = serde_json::to_vec(&s.info) {
            let _ = udp.send_to(&body, target).await;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn local_state(root: &std::path::Path) -> SharedState {
        let config = super::super::tests::config(root, "Discovery sender");
        let (_, _, fingerprint) = tls::identity(&config.identity_dir).unwrap();
        let (events, _) = mpsc::channel(64);
        Arc::new(Shared {
            info: DeviceInfo {
                alias: config.name.clone(),
                version: version(),
                device_model: None,
                device_type: None,
                fingerprint,
                port: config.port,
                protocol: "https".into(),
                download: false,
                announce: false,
            },
            visible: AtomicBool::new(true),
            events,
            sessions: Mutex::new(HashMap::new()),
            peers: Mutex::new(HashMap::new()),
            outgoing: Mutex::new(HashMap::new()),
            stop: CancellationToken::new(),
            store: ReceiveStore::open(&config.download_dir).unwrap(),
            prepare_gate: rate::RequestGate::new(20, Duration::from_secs(60)),
            registration_gate: rate::RequestGate::new(120, Duration::from_secs(60)),
            pin_gate: rate::RequestGate::new(6, Duration::from_secs(60)),
            pin_requests: Mutex::new(HashMap::new()),
            download_decisions: Mutex::new(HashMap::new()),
            interfaces: std::sync::RwLock::new(
                linuxdrop_network::interfaces(&config.policy, true).unwrap(),
            ),
            bandwidth: linuxdrop_network::BandwidthLimiter::new(None),
            config,
        })
    }

    #[tokio::test]
    async fn announcement_registers_over_pinned_https_and_failed_pin_falls_back() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let local = tempfile::tempdir().unwrap();
        let remote_root = tempfile::tempdir().unwrap();
        let state = local_state(local.path());
        let config = super::super::tests::config(remote_root.path(), "Remote");
        let (events, mut receiver) = mpsc::channel(64);
        let commands = start_bound(config.clone(), events, Ipv4Addr::LOCALHOST)
            .await
            .unwrap();
        let (_, _, fingerprint) = tls::identity(&config.identity_dir).unwrap();
        let remote = DeviceInfo {
            fingerprint,
            port: config.port,
            announce: true,
            alias: config.name.clone(),
            ..state.info.clone()
        };
        let interface = state
            .interfaces()
            .iter()
            .find(|entry| entry.address == IpAddr::V4(Ipv4Addr::LOCALHOST))
            .unwrap()
            .clone();
        let udp = socket(&interface, 0).unwrap();
        let port = udp.local_addr().unwrap().port();
        let scoped = socket2::SockRef::from(&udp);
        assert_eq!(scoped.device().unwrap().unwrap(), interface.name.as_bytes());
        assert_eq!(scoped.multicast_if_v4().unwrap(), Ipv4Addr::LOCALHOST);
        assert!(!scoped.multicast_all_v4().unwrap());
        let worker_state = state.clone();
        let worker = tokio::spawn(async move { on_interface(worker_state, interface, udp).await });
        let sender = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        sender
            .send_to(
                &serde_json::to_vec(&remote).unwrap(),
                (Ipv4Addr::LOCALHOST, port),
            )
            .await
            .unwrap();
        let peer = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let BackendEvent::PeerUpsert(peer) = receiver.recv().await.unwrap() {
                    break peer;
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(peer.name, state.info.alias);
        state.stop.cancel();
        tokio::time::timeout(Duration::from_secs(1), worker)
            .await
            .unwrap()
            .unwrap()
            .unwrap();

        let observer = tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let udp = Arc::new(
            tokio::net::UdpSocket::bind((Ipv4Addr::LOCALHOST, 0))
                .await
                .unwrap(),
        );
        let fallback_state = local_state(local.path());
        let mut wrong_pin = remote.clone();
        wrong_pin.fingerprint = "00".repeat(32);
        reply(
            fallback_state.clone(),
            wrong_pin.clone(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            "lo".into(),
            udp.clone(),
            observer.local_addr().unwrap(),
        )
        .await;
        let mut buffer = [0u8; 4096];
        let length = tokio::time::timeout(Duration::from_secs(1), observer.recv(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        let response: DeviceInfo = serde_json::from_slice(&buffer[..length]).unwrap();
        assert!(!response.announce);
        assert_eq!(response.fingerprint, fallback_state.info.fingerprint);
        fallback_state.visible.store(false, Ordering::Relaxed);
        reply(
            fallback_state,
            wrong_pin,
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            "lo".into(),
            udp,
            observer.local_addr().unwrap(),
        )
        .await;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), observer.recv(&mut buffer))
                .await
                .is_err()
        );
        commands.send(BackendCommand::Shutdown).await.unwrap();
    }

    #[test]
    fn discovery_rejects_off_link_multicast_and_broadcast_targets() {
        let interface = InterfaceAddress {
            name: "test".into(),
            address: "192.0.2.5".parse().unwrap(),
            netmask: "255.255.255.0".parse().unwrap(),
            index: 1,
            loopback: false,
        };
        assert!(permits(&interface, "192.0.2.6".parse().unwrap()));
        for address in [
            "192.0.2.0",
            "192.0.2.255",
            "198.51.100.1",
            "224.0.0.167",
            "0.0.0.0",
            "::1",
        ] {
            assert!(!permits(&interface, address.parse().unwrap()), "{address}");
        }
    }
}
