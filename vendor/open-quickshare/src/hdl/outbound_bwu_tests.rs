//! Real TCP/duplex encrypted handoffs without radio mutations.
use super::*;
use crate::hdl::MigratableStream;
use location_nearby_connections::{
    bandwidth_upgrade_negotiation_frame::EventType, v1_frame::FrameType,
};
type Request = OutboundRequest<MigratableStream>;
fn request(stream: tokio::io::DuplexStream, send: u8, receive: u8) -> Request {
    let (events, _) = tokio::sync::broadcast::channel(32);
    let mut request = Request::new(
        *b"LOCL",
        MigratableStream::Ble(stream),
        "test".into(),
        events,
        OutboundPayload::Files(vec![]),
        RemoteDeviceInfo {
            name: "Peer".into(),
            device_type: DeviceType::Phone,
        },
    );
    request.state.encrypt_key = Some(vec![send; 32]);
    request.state.send_hmac_key = Some(vec![send + 10; 32]);
    request.state.decrypt_key = Some(vec![receive; 32]);
    request.state.recv_hmac_key = Some(vec![receive + 10; 32]);
    request.state.encryption_done = true;
    request.state.state = TransferState::SendingFiles;
    request
}
fn event(frame: OfflineFrame) -> EventType {
    frame
        .v1
        .unwrap()
        .bandwidth_upgrade_negotiation
        .unwrap()
        .event_type()
}
async fn tcp_pair() -> (TcpStream, TcpStream) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let local = TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    (local, listener.accept().await.unwrap().0)
}
#[tokio::test]
async fn client_handoff_requires_negotiated_ack_and_complete_encrypted_drain() {
    for case in [
        "ack",
        "no-ack",
        "bad-ack",
        "missing-body",
        "bad-version",
        "oversized-ack",
        "early-last-write",
        "early-bad-ack",
        "truncated-drain",
        "cancel",
    ] {
        let (local, remote) = tokio::io::duplex(16384);
        let mut sender = request(local, 1, 2);
        let mut peer = request(remote, 2, 1);
        let (tcp, mut new_peer) = tcp_pair().await;
        let control = sender.sender.clone();
        let worker = tokio::spawn(async move {
            let outcome = sender.finish_upgrade_over(tcp, case != "no-ack").await;
            (sender, outcome)
        });
        tokio::time::timeout(Duration::from_secs(3), async {
            let intro =
                OfflineFrame::decode(read_plain_frame(&mut new_peer).await.unwrap().as_slice())
                    .unwrap();
            let intro = intro
                .v1
                .unwrap()
                .bandwidth_upgrade_negotiation
                .unwrap()
                .client_introduction
                .unwrap();
            assert_eq!(intro.endpoint_id(), "LOCL");
            assert!(!intro.supports_disabling_encryption());
            if case == "cancel" {
                control
                    .send(ChannelMessage {
                        id: "test".into(),
                        msg: channel::Message::Lib {
                            action: TransferAction::TransferCancel,
                        },
                    })
                    .unwrap();
            } else {
                if case.starts_with("early-") {
                    peer.encrypt_and_send(&Request::bwu_frame(
                        EventType::LastWriteToPriorChannel,
                        None,
                        None,
                    ))
                    .await
                    .unwrap();
                    // Keep ACK pending long enough to exercise the competing old channel.
                    tokio::time::sleep(Duration::from_millis(30)).await;
                }
                if case == "oversized-ack" {
                    tokio::io::AsyncWriteExt::write_u32(&mut new_peer, 1025)
                        .await
                        .unwrap();
                } else if case != "no-ack" {
                    let mut ack = Request::bwu_ack_frame();
                    if case == "bad-ack" || case == "early-bad-ack" {
                        ack.v1.as_mut().unwrap().r#type = Some(FrameType::KeepAlive.into());
                    }
                    if case == "missing-body" {
                        ack.v1
                            .as_mut()
                            .unwrap()
                            .bandwidth_upgrade_negotiation
                            .as_mut()
                            .unwrap()
                            .client_introduction_ack = None;
                    }
                    if case == "bad-version" {
                        ack.version = Some(99);
                    }
                    send_plain_frame(&mut new_peer, &ack.encode_to_vec())
                        .await
                        .unwrap();
                }
                if case == "early-bad-ack" {
                    // The sender must end the damaged session without writing
                    // a fallback after its peer already ended the old channel.
                } else if ["bad-ack", "missing-body", "bad-version", "oversized-ack"]
                    .contains(&case)
                {
                    assert_eq!(
                        event(peer.read_encrypted_offline_frame().await.unwrap()),
                        EventType::UpgradeFailure
                    );
                } else {
                    assert_eq!(
                        event(peer.read_encrypted_offline_frame().await.unwrap()),
                        EventType::LastWriteToPriorChannel
                    );
                    if case == "truncated-drain" {
                        peer.encrypt_and_send(&Request::bwu_frame(
                            EventType::SafeToClosePriorChannel,
                            None,
                            None,
                        ))
                        .await
                        .unwrap();
                        peer.socket.shutdown().await.unwrap();
                    } else {
                        if case != "early-last-write" {
                            peer.encrypt_and_send(&Request::bwu_frame(
                                EventType::LastWriteToPriorChannel,
                                None,
                                None,
                            ))
                            .await
                            .unwrap();
                        }
                        peer.encrypt_and_send(&Request::bwu_frame(
                            EventType::SafeToClosePriorChannel,
                            None,
                            None,
                        ))
                        .await
                        .unwrap();
                        assert_eq!(
                            event(peer.read_encrypted_offline_frame().await.unwrap()),
                            EventType::SafeToClosePriorChannel
                        );
                    }
                }
            }
            let (mut sender, outcome) = worker.await.unwrap();
            match case {
                "cancel" => {
                    assert!(outcome.is_err());
                    assert_eq!(sender.state.state, TransferState::Cancelled);
                }
                "early-bad-ack" => assert!(outcome.is_err()),
                "truncated-drain" => assert!(
                    outcome.is_err(),
                    "Missing LAST_WRITE cannot commit a channel swap"
                ),
                "bad-ack" | "missing-body" | "bad-version" | "oversized-ack" => {
                    assert!(!outcome.unwrap())
                }
                _ => {
                    assert!(outcome.unwrap());
                    assert!(matches!(sender.socket, MigratableStream::Tcp(_)));
                    peer.socket = MigratableStream::Tcp(new_peer);
                    sender.send_keepalive(false).await.unwrap();
                    assert_eq!(
                        peer.read_encrypted_offline_frame()
                            .await
                            .unwrap()
                            .v1
                            .unwrap()
                            .r#type(),
                        FrameType::KeepAlive
                    );
                    assert_eq!(sender.state.server_seq, peer.state.client_seq);
                    return;
                }
            }
            assert!(matches!(sender.socket, MigratableStream::Ble(_)), "{case}");
        })
        .await
        .unwrap_or_else(|_| panic!("{case} stalled"));
    }
}
