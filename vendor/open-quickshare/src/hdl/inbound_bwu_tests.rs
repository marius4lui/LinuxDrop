use super::*;
use crate::hdl::{MigratableStream, set_p2p_connector, set_upgrade_lease};
use crate::location_nearby_connections::bandwidth_upgrade_negotiation_frame::upgrade_path_info::Medium;
use crate::location_nearby_connections::bandwidth_upgrade_negotiation_frame::{
    ClientIntroduction, EventType,
};
use linuxdrop_network::{DirectWifiCapabilities, P2pConnector, P2pHosted};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;
struct Helper(Arc<AtomicUsize>);
impl P2pConnector for Helper {
    fn host(&self) -> BoxFuture<'_, anyhow::Result<P2pHosted>> {
        Box::pin(async {
            Ok(P2pHosted {
                interface: "ld-bwu0".into(),
                ssid: "DIRECT-kernel-fixture".into(),
                password: "test-password".into(),
                frequency: 2437,
                ipv4_address: "192.0.2.1".parse().unwrap(),
                ipv6_address: Some("fe80::1234".parse().unwrap()),
            })
        })
    }
    fn connect(
        &self,
        _: String,
        _: String,
        _: u32,
    ) -> BoxFuture<'_, anyhow::Result<linuxdrop_network::P2pConnection>> {
        Box::pin(async { anyhow::bail!("Not a join fixture") })
    }
    fn disconnect(&self) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}
async fn ip(args: &[&str]) {
    let output = tokio::process::Command::new("/usr/sbin/ip")
        .args(args)
        .env_clear()
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
fn keys(request: &mut InboundRequest<MigratableStream>, sender: u8, receiver: u8) {
    request.state.encrypt_key = Some(vec![sender; 32]);
    request.state.send_hmac_key = Some(vec![sender + 10; 32]);
    request.state.decrypt_key = Some(vec![receiver; 32]);
    request.state.recv_hmac_key = Some(vec![receiver + 10; 32]);
    request.state.encryption_done = true;
    request.state.state = TransferState::ReceivingFiles;
}
#[tokio::test]
#[ignore = "requires run-direct-upgrade.sh private kernel network namespace"]
async fn receiver_hosts_direct_group_and_preserves_encrypted_channel_state() {
    assert_eq!(std::env::var("LINUXDROP_TEST_BWU").as_deref(), Ok("1"));
    assert_ne!(
        std::fs::read_link("/proc/self/ns/net").unwrap(),
        std::fs::read_link("/proc/1/ns/net").unwrap()
    );
    ip(&["link", "set", "lo", "up"]).await;
    ip(&["link", "add", "ld-bwu0", "type", "dummy"]).await;
    ip(&["link", "set", "ld-bwu0", "up"]).await;
    ip(&["address", "add", "192.0.2.1/24", "dev", "ld-bwu0"]).await;
    ip(&[
        "-6",
        "address",
        "replace",
        "fe80::1234/64",
        "dev",
        "ld-bwu0",
        "nodad",
    ])
    .await;
    // The ordinary LAN policy excludes this fixture radio; the reserved-radio
    // listener must independently bind it, as it does for a real group VIF.
    crate::lan_policy::set(linuxdrop_network::TransferPolicy {
        allowed_interfaces: vec!["no-lan0".into()],
        ..Default::default()
    });
    let releases = Arc::new(AtomicUsize::new(0));
    set_upgrade_lease(Some((
        linuxdrop_network::nm::Lease {
            interface: "reserved0".into(),
            lease_id: "fixture".into(),
            connection_uuid: "fixture".into(),
        },
        DirectWifiCapabilities {
            station: true,
            p2p_group_owner: true,
            frequencies: vec![2437],
            ..Default::default()
        },
    )));
    set_p2p_connector(Some(Arc::new(Helper(releases.clone()))));
    let (local, remote) = tokio::io::duplex(16384);
    let (events, _) = tokio::sync::broadcast::channel(32);
    let mut receiver = InboundRequest::new(
        MigratableStream::Ble(local),
        "receiver".into(),
        events.clone(),
    );
    let mut peer = InboundRequest::new(MigratableStream::Ble(remote), "peer".into(), events);
    keys(&mut receiver, 1, 2);
    keys(&mut peer, 2, 1);
    let frame = OfflineFrame {
        version: Some(1),
        v1: Some(location_nearby_connections::V1Frame {
            r#type: Some(
                location_nearby_connections::v1_frame::FrameType::ConnectionRequest.into(),
            ),
            connection_request: Some(location_nearby_connections::ConnectionRequestFrame {
                endpoint_id: Some("PEER".into()),
                endpoint_info: Some(
                    RemoteDeviceInfo {
                        name: "Peer".into(),
                        device_type: DeviceType::Phone,
                    }
                    .serialize(),
                ),
                mediums: vec![Medium::WifiDirect as i32],
                medium_metadata: Some(crate::hdl::metadata_for(
                    &DirectWifiCapabilities {
                        station: true,
                        ..Default::default()
                    },
                    None,
                )),
                ..Default::default()
            }),
            ..Default::default()
        }),
    };
    receiver.process_connection_request(&frame).unwrap();
    receiver.set_bwu_tcp_port(1);
    let task = tokio::spawn(async move {
        receiver.do_bwu().await.unwrap();
        receiver
    });
    let mut stage = "offer";
    let result = tokio::time::timeout(Duration::from_secs(8), async {
        let offered = peer
            .read_encrypted_offline_frame()
            .await
            .unwrap()
            .v1
            .unwrap()
            .bandwidth_upgrade_negotiation
            .unwrap()
            .upgrade_path_info
            .unwrap();
        assert_eq!(offered.medium(), Medium::WifiDirect);
        assert!(offered.wifi_hotspot_credentials.is_none());
        let credentials = offered.wifi_direct_credentials.unwrap();
        assert_eq!(credentials.ssid(), "DIRECT-kernel-fixture");
        assert_eq!(credentials.frequency(), 2437);
        assert_eq!(credentials.ip_v6_address().len(), 16);
        stage = "TCP connect";
        let mut tcp =
            tokio::net::TcpStream::connect((credentials.gateway(), credentials.port() as u16))
                .await
                .unwrap();
        let introduction = InboundRequest::<MigratableStream>::bwu_frame(
            EventType::ClientIntroduction,
            None,
            Some(ClientIntroduction {
                endpoint_id: Some("PEER".into()),
                supports_disabling_encryption: Some(false),
            }),
        );
        send_frame_on(&mut tcp, &introduction.encode_to_vec())
            .await
            .unwrap();
        stage = "introduction acknowledgment";
        let ack =
            OfflineFrame::decode(read_frame_from(&mut tcp).await.unwrap().as_slice()).unwrap();
        assert_eq!(
            ack.v1
                .unwrap()
                .bandwidth_upgrade_negotiation
                .unwrap()
                .event_type(),
            EventType::ClientIntroductionAck
        );
        stage = "last write";
        assert_eq!(
            peer.read_encrypted_offline_frame()
                .await
                .unwrap()
                .v1
                .unwrap()
                .bandwidth_upgrade_negotiation
                .unwrap()
                .event_type(),
            EventType::LastWriteToPriorChannel
        );
        peer.encrypt_and_send(&InboundRequest::<MigratableStream>::bwu_frame(
            EventType::LastWriteToPriorChannel,
            None,
            None,
        ))
        .await
        .unwrap();
        peer.encrypt_and_send(&InboundRequest::<MigratableStream>::bwu_frame(
            EventType::SafeToClosePriorChannel,
            None,
            None,
        ))
        .await
        .unwrap();
        assert_eq!(
            peer.read_encrypted_offline_frame()
                .await
                .unwrap()
                .v1
                .unwrap()
                .bandwidth_upgrade_negotiation
                .unwrap()
                .event_type(),
            EventType::SafeToClosePriorChannel
        );
        stage = "prior-channel disconnect";
        let disconnected =
            OfflineFrame::decode(read_frame_from(&mut peer.socket).await.unwrap().as_slice())
                .unwrap();
        assert_eq!(
            disconnected.v1.unwrap().r#type(),
            location_nearby_connections::v1_frame::FrameType::Disconnection
        );
        peer.socket = MigratableStream::Tcp(tcp);
        stage = "encrypted TCP resumption";
        let resumed = peer.read_encrypted_offline_frame().await.unwrap();
        assert_eq!(
            resumed.v1.unwrap().r#type(),
            location_nearby_connections::v1_frame::FrameType::KeepAlive
        );
    })
    .await;
    if result.is_err() {
        task.abort();
        panic!("Receiver direct upgrade timed out at {stage}");
    }
    let mut receiver = task.await.unwrap();
    assert!(matches!(receiver.socket, MigratableStream::Tcp(_)));
    assert!(receiver.hotspot_guard.is_some());
    assert_eq!(receiver.state.server_seq, peer.state.client_seq);
    receiver.state.state = TransferState::Finished;
    peer.state.state = TransferState::Finished;
    drop(receiver);
    tokio::time::timeout(Duration::from_secs(2), async {
        while releases.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    set_upgrade_lease(None);
    set_p2p_connector(None);
    ip(&["link", "del", "ld-bwu0"]).await;
}
