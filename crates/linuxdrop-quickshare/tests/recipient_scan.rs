//! Actual scan ownership and endpoint selection against a private BlueZ bus.
#[path = "support/recipient.rs"]
mod recipient;
#[path = "support/scanner.rs"]
mod scanner;
use rqs_lib::channel::{ChannelMessage, Message};
use std::{
    sync::{Arc, Mutex, atomic::Ordering},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
const GOOD: &str = "/org/bluez/hci1/dev_40_11_22_33_44_55";
const WRONG: &str = "/org/bluez/hci1/dev_40_11_22_33_44_66";
async fn wait_for(mut check: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(12), async {
        while !check() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
async fn emit(bus: &zbus::Connection, path: &str) {
    let iface = bus
        .object_server()
        .interface::<_, recipient::Recipient>(path)
        .await
        .unwrap();
    iface
        .get()
        .await
        .service_data_changed(iface.signal_emitter())
        .await
        .unwrap();
}
async fn health(events: &mut tokio::sync::broadcast::Receiver<ChannelMessage>, ready: bool) {
    tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            match events.recv().await.unwrap().msg {
                Message::BluetoothServiceReady { component }
                    if ready && component == "bluetooth-peer-discovery" =>
                {
                    break;
                }
                Message::Backend { component, .. }
                    if !ready && component == "bluetooth-peer-discovery" =>
                {
                    break;
                }
                _ => {}
            }
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
#[ignore = "requires private bus in run-bluetooth-lifecycle.sh"]
async fn recipient_selection_waits_for_stop_and_recovers_without_trusting_names_or_cache() {
    assert_eq!(
        std::env::var("LINUXDROP_TEST_PRIVATE_BLUEZ").as_deref(),
        Ok("1")
    );
    assert_eq!(
        std::env::var("DBUS_SYSTEM_BUS_ADDRESS").unwrap(),
        std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap()
    );
    rqs_lib::set_bluetooth_adapter(Some("hci1".into()));
    let radio = Arc::new(scanner::State::default());
    radio.powered.store(true, Ordering::SeqCst);
    let other = Arc::new(scanner::State::default());
    other.powered.store(true, Ordering::SeqCst);
    let data = Arc::new(Mutex::new(rqs_lib::hdl::receiver_service_data(
        *b"GOOD",
        2,
        "Same phone",
        Some(129),
    )));
    let bus = zbus::connection::Builder::session()
        .unwrap()
        .name("org.bluez")
        .unwrap()
        .serve_at("/", zbus::fdo::ObjectManager)
        .unwrap()
        .serve_at("/org/bluez/hci0", scanner::Scanner(other.clone()))
        .unwrap()
        .serve_at("/org/bluez/hci1", scanner::Scanner(radio.clone()))
        .unwrap()
        .serve_at(
            GOOD,
            recipient::Recipient {
                address: "40:11:22:33:44:55".into(),
                data: data.clone(),
            },
        )
        .unwrap()
        .serve_at(
            WRONG,
            recipient::Recipient {
                address: "40:11:22:33:44:66".into(),
                data: Arc::new(Mutex::new(rqs_lib::hdl::receiver_service_data(
                    *b"EVIL",
                    2,
                    "Same phone",
                    Some(130),
                ))),
            },
        )
        .unwrap()
        .build()
        .await
        .unwrap();
    let adapter = rqs_lib::bluetooth_adapter().await.unwrap();
    let selected = adapter.clone();
    let task = tokio::spawn(async move {
        rqs_lib::hdl::scan_target(
            &selected,
            Duration::from_secs(3),
            *b"GOOD",
            CancellationToken::new(),
        )
        .await
    });
    wait_for(|| radio.starts.load(Ordering::SeqCst) == 1).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        !task.is_finished(),
        "Cached advertisements must not immediately satisfy a new scan"
    );
    emit(&bus, WRONG).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !task.is_finished(),
        "A matching display name is not the selected endpoint"
    );
    radio.hold_stop.store(true, Ordering::SeqCst);
    emit(&bus, GOOD).await;
    wait_for(|| radio.stops.load(Ordering::SeqCst) == 1).await;
    assert!(
        !task.is_finished(),
        "Connecting must wait for StopDiscovery acknowledgement"
    );
    radio.hold_stop.store(false, Ordering::SeqCst);
    radio.wake.notify_waiters();
    let targets = task.await.unwrap().unwrap();
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].endpoint_id, *b"GOOD");
    assert_eq!(targets[0].addr.to_string(), "40:11:22:33:44:55");
    assert_eq!(other.starts.load(Ordering::SeqCst), 0);

    // Dropping the caller while StartDiscovery is in flight cannot abandon cleanup.
    radio.hold_start.store(true, Ordering::SeqCst);
    let selected = adapter.clone();
    let task = tokio::spawn(async move {
        rqs_lib::hdl::scan_target(
            &selected,
            Duration::from_secs(3),
            *b"GOOD",
            CancellationToken::new(),
        )
        .await
    });
    wait_for(|| radio.starts.load(Ordering::SeqCst) == 2).await;
    task.abort();
    let _ = task.await;
    assert_eq!(radio.stops.load(Ordering::SeqCst), 1);
    radio.hold_start.store(false, Ordering::SeqCst);
    radio.wake.notify_waiters();
    wait_for(|| radio.stops.load(Ordering::SeqCst) == 2 && !radio.active.load(Ordering::SeqCst))
        .await;

    // Background discovery starts offline and treats equal names as two peers.
    radio.set_power(false);
    let (peers, mut discovered) = tokio::sync::broadcast::channel(32);
    let (status, mut events) = tokio::sync::broadcast::channel(64);
    let cancel = CancellationToken::new();
    let running = tokio::spawn(rqs_lib::hdl::ble_discovery(peers, status, cancel.clone()));
    health(&mut events, false).await;
    radio.powered.store(true, Ordering::SeqCst);
    wait_for(|| radio.starts.load(Ordering::SeqCst) == 3).await;
    emit(&bus, GOOD).await;
    emit(&bus, WRONG).await;
    health(&mut events, true).await;
    let a = discovered.recv().await.unwrap();
    let b = discovered.recv().await.unwrap();
    assert_eq!(a.name, b.name);
    assert_ne!(a.id, b.id);
    assert_ne!(a.endpoint_id(), b.endpoint_id());
    assert!(a.endpoint_id().is_some());
    radio.set_power(false);
    health(&mut events, false).await;
    assert_eq!(discovered.recv().await.unwrap().present, Some(false));
    assert_eq!(discovered.recv().await.unwrap().present, Some(false));
    radio.powered.store(true, Ordering::SeqCst);
    wait_for(|| radio.starts.load(Ordering::SeqCst) == 4).await;
    emit(&bus, GOOD).await;
    health(&mut events, true).await;
    assert_eq!(
        discovered.recv().await.unwrap().endpoint_id(),
        Some(*b"GOOD")
    );
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(3), running)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(discovered.recv().await.unwrap().present, Some(false));
    assert!(!radio.active.load(Ordering::SeqCst));
    // Ordinary repeated scans must release bus clients as well as scanner state.
    let dbus = zbus::fdo::DBusProxy::new(&bus).await.unwrap();
    let baseline = dbus.list_names().await.unwrap().len();
    for _ in 0..3 {
        rqs_lib::hdl::scan_target(
            &adapter,
            Duration::from_millis(50),
            *b"NONE",
            CancellationToken::new(),
        )
        .await
        .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert!(
        dbus.list_names().await.unwrap().len() <= baseline,
        "A scan leaked a D-Bus client"
    );

    // Actual RQS connection jobs consume a per-transfer Cancel even before UKEY2.
    rqs_lib::lan_policy::set(linuxdrop_network::TransferPolicy {
        allowed_interfaces: vec!["linuxdrop-test-absent".into()],
        ..Default::default()
    });
    let mut engine = rqs_lib::RQS::new(
        rqs_lib::Visibility::Invisible,
        None,
        None,
        Some("Test".into()),
    );
    engine.ble_enabled = false; // Isolate the requested connection from background roles.
    let (send, _) = engine.run().await.unwrap();
    let mut transfer_events = engine.message_sender.subscribe();
    let starts = radio.starts.load(Ordering::SeqCst);
    let stops = radio.stops.load(Ordering::SeqCst);
    send.send(rqs_lib::SendInfo {
        id: "pending-connect".into(),
        name: "Same phone".into(),
        addr: "0.0.0.0:0".into(),
        ob: rqs_lib::OutboundPayload::Files(Vec::new()),
        ble: true,
        peer_endpoint_id: Some(*b"NONE"),
    })
    .await
    .unwrap();
    wait_for(|| radio.starts.load(Ordering::SeqCst) == starts + 1).await;
    engine
        .message_sender
        .send(ChannelMessage {
            id: "other-transfer".into(),
            msg: Message::Lib {
                action: rqs_lib::channel::TransferAction::TransferCancel,
            },
        })
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(radio.stops.load(Ordering::SeqCst), stops);
    radio.hold_stop.store(true, Ordering::SeqCst);
    engine
        .message_sender
        .send(ChannelMessage {
            id: "pending-connect".into(),
            msg: Message::Lib {
                action: rqs_lib::channel::TransferAction::TransferCancel,
            },
        })
        .unwrap();
    wait_for(|| radio.stops.load(Ordering::SeqCst) == stops + 1).await;
    let stopping = tokio::spawn(async move { engine.stop().await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        !stopping.is_finished(),
        "RQS shutdown must wait for the owned scan's cleanup receipt"
    );
    radio.hold_stop.store(false, Ordering::SeqCst);
    radio.wake.notify_waiters();
    tokio::time::timeout(Duration::from_secs(3), stopping)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(!radio.active.load(Ordering::SeqCst));

    let mut cancelled = false;
    while let Ok(event) = transfer_events.try_recv() {
        if event.id == "pending-connect"
            && let Message::Client(client) = event.msg
        {
            cancelled |= client.state == Some(rqs_lib::TransferState::Cancelled);
            assert_ne!(
                client.state,
                Some(rqs_lib::TransferState::Disconnected),
                "Cancellation must not be overwritten by a connection error"
            );
        }
    }
    assert!(cancelled);

    // A scan already running on an old daemon cleans only that owner.
    let old_radio = radio.clone();
    let starts = old_radio.starts.load(Ordering::SeqCst);
    let stops = old_radio.stops.load(Ordering::SeqCst);
    let selected = adapter.clone();
    let task = tokio::spawn(async move {
        rqs_lib::hdl::scan_target(
            &selected,
            Duration::from_secs(5),
            *b"NONE",
            CancellationToken::new(),
        )
        .await
    });
    wait_for(|| old_radio.starts.load(Ordering::SeqCst) == starts + 1).await;
    bus.release_name("org.bluez").await.unwrap();
    let radio = Arc::new(scanner::State::default());
    radio.powered.store(true, Ordering::SeqCst);
    let replacement = zbus::connection::Builder::session()
        .unwrap()
        .name("org.bluez")
        .unwrap()
        .serve_at("/", zbus::fdo::ObjectManager)
        .unwrap()
        .serve_at("/org/bluez/hci1", scanner::Scanner(radio.clone()))
        .unwrap()
        .build()
        .await
        .unwrap();
    let error = tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("owner changed"));
    assert_eq!(old_radio.stops.load(Ordering::SeqCst), stops + 1);
    assert_eq!(radio.stops.load(Ordering::SeqCst), 0);
    assert!(!old_radio.active.load(Ordering::SeqCst));
    radio.fail_stop.store(true, Ordering::SeqCst);
    let error = rqs_lib::hdl::scan_target(
        &adapter,
        Duration::from_millis(50),
        *b"NONE",
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("Unconfirmed cleanup"));
    let starts = radio.starts.load(Ordering::SeqCst);
    let error = rqs_lib::hdl::scan_target(
        &adapter,
        Duration::from_millis(50),
        *b"NONE",
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("Unconfirmed cleanup"));
    assert_eq!(radio.starts.load(Ordering::SeqCst), starts);
    radio.fail_stop.store(false, Ordering::SeqCst);
    wait_for(|| !radio.active.load(Ordering::SeqCst)).await;
    rqs_lib::hdl::scan_target(
        &adapter,
        Duration::from_millis(50),
        *b"NONE",
        CancellationToken::new(),
    )
    .await
    .unwrap();
    drop(replacement);
}
