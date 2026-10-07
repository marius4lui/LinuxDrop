use super::*;
use crate::hdl::{MigratableStream, set_p2p_connector, set_upgrade_lease};
use crate::location_nearby_connections::bandwidth_upgrade_negotiation_frame::upgrade_path_info::Medium;
use crate::location_nearby_connections::bandwidth_upgrade_negotiation_frame::{
    ClientIntroduction, EventType,
};
use linuxdrop_network::{DirectWifiCapabilities, P2pConnector, P2pHostAuth, P2pHosted};
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
                device_name: None,
                interface: "ld-bwu0".into(),
                ssid: "DIRECT-kernel-fixture".into(),
                password: "test-password".into(),
                frequency: 2437,
                ipv4_address: "192.0.2.1".parse().unwrap(),
                ipv6_address: Some("fe80::1234".parse().unwrap()),
            })
        })
    }
    fn host_with_auth(&self, auth: P2pHostAuth) -> BoxFuture<'_, anyhow::Result<P2pHosted>> {
        Box::pin(async move {
            let mut hosted = self.host().await?;
            hosted.device_name =
                (auth == P2pHostAuth::DeviceName).then(|| "Kernel fixture GO".into());
            Ok(hosted)
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
    for auth in [P2pHostAuth::Password, P2pHostAuth::DeviceName] {
        receiver_hosts_group(auth).await;
    }
}
async fn receiver_hosts_group(auth: P2pHostAuth) {
    fixture_network().await;
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
                        station: auth == P2pHostAuth::Password,
                        p2p_client: auth == P2pHostAuth::DeviceName,
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
        match auth {
            P2pHostAuth::Password => {
                assert_eq!(credentials.ssid(), "DIRECT-kernel-fixture");
                assert!(credentials.device_name.is_none());
            }
            P2pHostAuth::DeviceName => {
                assert_eq!(credentials.device_name(), "Kernel fixture GO");
                assert!(
                    credentials.ssid.is_none()
                        && credentials.password.is_none()
                        && credentials.pin.is_none()
                );
            }
        }
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
        // A wrong endpoint and an oversized unauthenticated introduction must
        // close only their own TCP socket, never the established BLE session.
        let mut rogue =
            tokio::net::TcpStream::connect((credentials.gateway(), credentials.port() as u16))
                .await
                .unwrap();
        let mut wrong = introduction.clone();
        wrong
            .v1
            .as_mut()
            .unwrap()
            .bandwidth_upgrade_negotiation
            .as_mut()
            .unwrap()
            .client_introduction
            .as_mut()
            .unwrap()
            .endpoint_id = Some("NOPE".into());
        // Close the initially accepted socket first, so the fixture verifies
        // failed reads too and lets the listener move to the rogue connection.
        tcp.shutdown().await.unwrap();
        send_frame_on(&mut rogue, &wrong.encode_to_vec())
            .await
            .unwrap();
        let mut byte = [0];
        assert_eq!(
            tokio::io::AsyncReadExt::read(&mut rogue, &mut byte)
                .await
                .unwrap(),
            0
        );
        let mut oversized =
            tokio::net::TcpStream::connect((credentials.gateway(), credentials.port() as u16))
                .await
                .unwrap();
        tokio::io::AsyncWriteExt::write_u32(&mut oversized, 1025)
            .await
            .unwrap();
        assert_eq!(
            tokio::io::AsyncReadExt::read(&mut oversized, &mut byte)
                .await
                .unwrap(),
            0
        );
        let mut tcp =
            tokio::net::TcpStream::connect((credentials.gateway(), credentials.port() as u16))
                .await
                .unwrap();
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

async fn fixture_network() {
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
}

struct ClientHelper {
    releases: Arc<AtomicUsize>,
    started: Arc<tokio::sync::Notify>,
}
impl P2pConnector for ClientHelper {
    fn host(&self) -> BoxFuture<'_, anyhow::Result<P2pHosted>> {
        Box::pin(async { anyhow::bail!("Client-only radio must not host") })
    }
    fn connect(
        &self,
        name: String,
        pin: String,
        frequency: u32,
    ) -> BoxFuture<'_, anyhow::Result<linuxdrop_network::P2pConnection>> {
        Box::pin(async move {
            assert_eq!(name, "sender-fixture");
            assert!(pin.is_empty(), "Device-name mode allows PBC without a PIN");
            assert_eq!(frequency, 2437);
            self.started.notify_one();
            tokio::time::sleep(Duration::from_millis(80)).await;
            Ok(linuxdrop_network::P2pConnection {
                interface: "ld-bwu0".into(),
                ipv4_address: Some("192.0.2.1".parse().unwrap()),
                ipv6_address: None,
            })
        })
    }
    fn disconnect(&self) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async move {
            self.releases.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

fn bwu_event(frame: OfflineFrame) -> EventType {
    frame
        .v1
        .unwrap()
        .bandwidth_upgrade_negotiation
        .unwrap()
        .event_type()
}

#[tokio::test]
#[ignore = "requires run-direct-upgrade.sh private kernel network namespace"]
async fn receiver_joins_sender_group_with_consent_and_handles_upgrade_failures() {
    fixture_network().await;
    crate::lan_policy::set(linuxdrop_network::TransferPolicy {
        allowed_interfaces: vec!["no-lan0".into()],
        ..Default::default()
    });
    for scenario in [
        "ack",
        "no-ack",
        "wrong-ack",
        "cancel",
        "incomplete-drain",
        "unrequested",
        "bad-address",
    ] {
        let outcome = tokio::time::timeout(Duration::from_secs(8), client_scenario(scenario)).await;
        assert!(outcome.is_ok(), "Client upgrade timed out: {scenario}");
    }
    set_upgrade_lease(None);
    set_p2p_connector(None);
    ip(&["link", "del", "ld-bwu0"]).await;
}

async fn client_scenario(scenario: &str) {
    use location_nearby_connections::bandwidth_upgrade_negotiation_frame::{
        UpgradePathInfo, upgrade_path_info::WifiDirectCredentials,
    };
    use location_nearby_connections::{
        MediumMetadata, MediumRole, medium_metadata::WifiDirectAuthType,
    };
    let releases = Arc::new(AtomicUsize::new(0));
    let started = Arc::new(tokio::sync::Notify::new());
    set_upgrade_lease(Some((
        linuxdrop_network::nm::Lease {
            interface: "reserved0".into(),
            lease_id: "fixture".into(),
            connection_uuid: "fixture".into(),
        },
        DirectWifiCapabilities {
            p2p_client: true,
            frequencies: vec![2437],
            ..Default::default()
        },
    )));
    set_p2p_connector(Some(Arc::new(ClientHelper {
        releases: releases.clone(),
        started: started.clone(),
    })));
    let (local, remote) = tokio::io::duplex(16384);
    let (events, _) = tokio::sync::broadcast::channel(64);
    let mut receiver = InboundRequest::new(
        MigratableStream::Ble(local),
        "receiver-client".into(),
        events.clone(),
    );
    let mut peer =
        InboundRequest::new(MigratableStream::Ble(remote), "peer".into(), events.clone());
    keys(&mut receiver, 1, 2);
    keys(&mut peer, 2, 1);
    let request = OfflineFrame {
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
                medium_metadata: Some(MediumMetadata {
                    medium_role: Some(MediumRole {
                        support_wifi_direct_group_owner: Some(true),
                        support_wifi_direct_group_client: Some(false),
                        ..Default::default()
                    }),
                    supported_wifi_direct_auth_types: vec![
                        WifiDirectAuthType::WifiDirectWithDeviceName as i32,
                    ],
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        }),
    };
    receiver.process_connection_request(&request).unwrap();
    let advertisement =
        crate::hdl::receiver_service_data(*b"LOCL", DeviceType::Laptop as u8, "Receiver", None);
    receiver.set_bwu_config(InboundUpgrade::new(&advertisement, 1));
    receiver.state.state = TransferState::WaitingForUserConsent;
    receiver.do_bwu().await.unwrap();
    assert!(
        tokio::time::timeout(
            Duration::from_millis(5),
            peer.read_encrypted_offline_frame()
        )
        .await
        .is_err(),
        "Role requests wait for consent"
    );
    receiver.state.state = TransferState::ReceivingFiles;
    let task = tokio::spawn(async move {
        let result = receiver.do_bwu().await;
        (receiver, result)
    });
    let request = peer
        .read_encrypted_offline_frame()
        .await
        .unwrap()
        .v1
        .unwrap()
        .bandwidth_upgrade_negotiation
        .unwrap();
    assert_eq!(request.event_type(), EventType::UpgradePathRequest);
    let request = request
        .upgrade_path_info
        .unwrap()
        .upgrade_path_request
        .unwrap();
    assert_eq!(request.mediums, vec![Medium::WifiDirect as i32]);
    let metadata = request.medium_meta_data.unwrap();
    assert_eq!(
        metadata
            .medium_role
            .unwrap()
            .support_wifi_direct_group_client,
        Some(true)
    );
    assert_eq!(
        metadata.supported_wifi_direct_auth_types,
        vec![WifiDirectAuthType::WifiDirectWithDeviceName as i32]
    );

    let listener = tokio::net::TcpListener::bind("192.0.2.1:0").await.unwrap();
    let mut offer = UpgradePathInfo {
        medium: Some(Medium::WifiDirect.into()),
        wifi_direct_credentials: Some(WifiDirectCredentials {
            device_name: Some("sender-fixture".into()),
            gateway: Some("192.0.2.1".into()),
            port: Some(listener.local_addr().unwrap().port().into()),
            frequency: Some(2437),
            ..Default::default()
        }),
        supports_client_introduction_ack: Some(scenario != "no-ack"),
        ..Default::default()
    };
    if scenario == "unrequested" {
        offer.medium = Some(Medium::WifiHotspot.into());
    }
    if scenario == "bad-address" {
        offer.wifi_direct_credentials.as_mut().unwrap().port = Some(0);
    }
    peer.encrypt_and_send(&InboundRequest::<MigratableStream>::bwu_frame(
        EventType::UpgradePathAvailable,
        Some(offer),
        None,
    ))
    .await
    .unwrap();
    if matches!(scenario, "unrequested" | "bad-address") {
        assert_eq!(
            bwu_event(peer.read_encrypted_offline_frame().await.unwrap()),
            EventType::UpgradeFailure
        );
        let (mut receiver, result) = task.await.unwrap();
        result.unwrap();
        assert!(matches!(receiver.socket, MigratableStream::Ble(_)));
        assert!(receiver.join_guard.is_none());
        assert_eq!(releases.load(Ordering::SeqCst), 0);
        assert!(
            tokio::time::timeout(Duration::from_millis(5), started.notified())
                .await
                .is_err(),
            "Invalid offers must not touch the reserved radio"
        );
        receiver.state.state = TransferState::Finished;
        peer.state.state = TransferState::Finished;
        return;
    }
    started.notified().await;
    if scenario == "cancel" {
        events
            .send(ChannelMessage {
                id: "receiver-client".into(),
                msg: channel::Message::Lib {
                    action: TransferAction::TransferCancel,
                },
            })
            .unwrap();
        let (receiver, result) = task.await.unwrap();
        assert!(result.is_err());
        assert_eq!(receiver.state.state, TransferState::Cancelled);
        assert!(matches!(receiver.socket, MigratableStream::Ble(_)));
        drop(receiver);
    } else {
        // More than 16 encrypted chunks must survive a handoff. These bytes
        // use the real payload assembler while network setup is still running.
        for offset in 0..24 {
            let frame = OfflineFrame {
                version: Some(1),
                v1: Some(location_nearby_connections::V1Frame {
                    r#type: Some(
                        location_nearby_connections::v1_frame::FrameType::PayloadTransfer.into(),
                    ),
                    payload_transfer: Some(PayloadTransferFrame {
                        packet_type: Some(PacketType::Data.into()),
                        payload_header: Some(PayloadHeader {
                            id: Some(99),
                            r#type: Some(payload_header::PayloadType::Bytes.into()),
                            total_size: Some(100),
                            ..Default::default()
                        }),
                        payload_chunk: Some(PayloadChunk {
                            offset: Some(offset),
                            flags: Some(0),
                            body: Some(vec![42]),
                            ..Default::default()
                        }),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
            };
            peer.encrypt_and_send(&frame).await.unwrap();
        }
        let (mut tcp, _) = listener.accept().await.unwrap();
        let intro = OfflineFrame::decode(read_frame_from(&mut tcp).await.unwrap().as_slice())
            .unwrap()
            .v1
            .unwrap()
            .bandwidth_upgrade_negotiation
            .unwrap();
        assert_eq!(intro.event_type(), EventType::ClientIntroduction);
        let intro = intro.client_introduction.unwrap();
        assert_eq!(intro.endpoint_id(), "LOCL");
        assert!(!intro.supports_disabling_encryption());
        if scenario == "ack" {
            // LAST_WRITE can race ahead on BLE while the TCP ACK is in flight.
            peer.encrypt_and_send(&InboundRequest::<MigratableStream>::bwu_frame(
                EventType::LastWriteToPriorChannel,
                None,
                None,
            ))
            .await
            .unwrap();
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        if scenario != "no-ack" {
            let ack = if scenario == "wrong-ack" {
                InboundRequest::<MigratableStream>::bwu_frame(EventType::UpgradeFailure, None, None)
            } else {
                InboundRequest::<MigratableStream>::bwu_ack_frame()
            };
            send_frame_on(&mut tcp, &ack.encode_to_vec()).await.unwrap();
        }
        if scenario == "wrong-ack" {
            assert_eq!(
                bwu_event(peer.read_encrypted_offline_frame().await.unwrap()),
                EventType::UpgradeFailure
            );
            let (mut receiver, result) = task.await.unwrap();
            result.unwrap();
            assert!(matches!(receiver.socket, MigratableStream::Ble(_)));
            assert!(receiver.join_guard.is_none());
            peer.send_keepalive(false).await.unwrap();
            receiver.handle().await.unwrap();
            assert_eq!(
                peer.read_encrypted_offline_frame()
                    .await
                    .unwrap()
                    .v1
                    .unwrap()
                    .r#type(),
                location_nearby_connections::v1_frame::FrameType::KeepAlive
            );
            receiver.state.state = TransferState::Finished;
        } else {
            assert_eq!(
                bwu_event(peer.read_encrypted_offline_frame().await.unwrap()),
                EventType::LastWriteToPriorChannel
            );
            if scenario == "incomplete-drain" {
                // An ended prior stream is not proof that its payload was drained.
                drop(peer);
                let (receiver, result) = task.await.unwrap();
                assert!(result.is_err());
                assert!(matches!(receiver.socket, MigratableStream::Ble(_)));
                assert!(receiver.join_guard.is_none());
                drop(receiver);
            } else {
                if scenario != "ack" {
                    peer.encrypt_and_send(&InboundRequest::<MigratableStream>::bwu_frame(
                        EventType::LastWriteToPriorChannel,
                        None,
                        None,
                    ))
                    .await
                    .unwrap();
                }
                peer.encrypt_and_send(&InboundRequest::<MigratableStream>::bwu_frame(
                    EventType::SafeToClosePriorChannel,
                    None,
                    None,
                ))
                .await
                .unwrap();
                assert_eq!(
                    bwu_event(peer.read_encrypted_offline_frame().await.unwrap()),
                    EventType::SafeToClosePriorChannel
                );
                let disconnected = OfflineFrame::decode(
                    read_frame_from(&mut peer.socket).await.unwrap().as_slice(),
                )
                .unwrap();
                assert_eq!(
                    disconnected.v1.unwrap().r#type(),
                    location_nearby_connections::v1_frame::FrameType::Disconnection
                );
                peer.socket = MigratableStream::Tcp(tcp);
                assert_eq!(
                    peer.read_encrypted_offline_frame()
                        .await
                        .unwrap()
                        .v1
                        .unwrap()
                        .r#type(),
                    location_nearby_connections::v1_frame::FrameType::KeepAlive
                );
                let (mut receiver, result) = task.await.unwrap();
                result.unwrap();
                assert!(matches!(receiver.socket, MigratableStream::Tcp(_)));
                assert!(receiver.join_guard.is_some());
                assert_eq!(receiver.state.payload_buffers[&99], vec![42; 24]);
                assert_eq!(receiver.state.client_seq, peer.state.server_seq);
                assert_eq!(receiver.state.server_seq, peer.state.client_seq);
                receiver.state.state = TransferState::Finished;
                peer.state.state = TransferState::Finished;
                drop(receiver);
            }
        }
    }
    tokio::time::timeout(Duration::from_secs(2), async {
        while releases.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        releases.load(Ordering::SeqCst),
        1,
        "Each join owns exactly one cleanup"
    );
}

#[tokio::test]
async fn consent_during_prior_channel_drain_is_preserved() {
    let (local, remote) = tokio::io::duplex(16384);
    let (events, _) = tokio::sync::broadcast::channel(32);
    let mut receiver = InboundRequest::new(
        MigratableStream::Ble(local),
        "consent".into(),
        events.clone(),
    );
    let mut peer =
        InboundRequest::new(MigratableStream::Ble(remote), "peer".into(), events.clone());
    keys(&mut receiver, 1, 2);
    keys(&mut peer, 2, 1);
    receiver.state.state = TransferState::WaitingForUserConsent;
    let task = tokio::spawn(async move {
        receiver.drain_prior_channel().await.unwrap();
        receiver
    });
    assert_eq!(
        bwu_event(peer.read_encrypted_offline_frame().await.unwrap()),
        EventType::LastWriteToPriorChannel
    );
    events
        .send(ChannelMessage {
            id: "consent".into(),
            msg: channel::Message::Lib {
                action: TransferAction::ConsentAccept,
            },
        })
        .unwrap();
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
        bwu_event(peer.read_encrypted_offline_frame().await.unwrap()),
        EventType::SafeToClosePriorChannel
    );
    let mut receiver = task.await.unwrap();
    assert!(receiver.deferred_accept);
    assert_eq!(receiver.state.state, TransferState::WaitingForUserConsent);
    receiver.handle().await.unwrap();
    assert_eq!(receiver.state.state, TransferState::ReceivingFiles);
    let confirmation = peer.read_encrypted_offline_frame().await.unwrap();
    let packet = confirmation
        .v1
        .unwrap()
        .payload_transfer
        .unwrap()
        .payload_chunk
        .unwrap();
    let sharing = sharing_nearby::Frame::decode(packet.body().as_ref()).unwrap();
    assert_eq!(
        sharing.v1.unwrap().connection_response.unwrap().status(),
        sharing_nearby::connection_response_frame::Status::Accept
    );
    receiver.state.state = TransferState::Finished;
    peer.state.state = TransferState::Finished;
}

#[tokio::test]
async fn rejecting_during_upgrade_remains_a_rejection() {
    let (local, remote) = tokio::io::duplex(16384);
    let (events, _) = tokio::sync::broadcast::channel(32);
    let mut receiver = InboundRequest::new(
        MigratableStream::Ble(local),
        "reject".into(),
        events.clone(),
    );
    let mut peer =
        InboundRequest::new(MigratableStream::Ble(remote), "peer".into(), events.clone());
    keys(&mut receiver, 1, 2);
    keys(&mut peer, 2, 1);
    receiver.state.state = TransferState::WaitingForUserConsent;
    events
        .send(ChannelMessage {
            id: "reject".into(),
            msg: channel::Message::Lib {
                action: TransferAction::ConsentDecline,
            },
        })
        .unwrap();
    assert!(
        receiver
            .next_upgrade_frame(tokio::time::Instant::now() + Duration::from_secs(1))
            .await
            .is_err()
    );
    assert_eq!(receiver.state.state, TransferState::Rejected);
    let rejection = peer.read_encrypted_offline_frame().await.unwrap();
    let packet = rejection
        .v1
        .unwrap()
        .payload_transfer
        .unwrap()
        .payload_chunk
        .unwrap();
    let sharing = sharing_nearby::Frame::decode(packet.body()).unwrap();
    assert_eq!(
        sharing.v1.unwrap().connection_response.unwrap().status(),
        sharing_nearby::connection_response_frame::Status::Reject
    );
    peer.state.state = TransferState::Finished;
}
