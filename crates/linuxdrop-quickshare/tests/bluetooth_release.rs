//! Real Quick Share advertisement actors against the isolated BlueZ mock.
#[path = "../../linuxdrop-network/tests/support/bluez.rs"]
mod bluez;
#[path = "support/gatt.rs"]
mod gatt;
#[path = "support/scanner.rs"]
mod scanner;
use bluez::{Adapter, Advertising, State};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

async fn wait_for(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(8), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Quick Share advertisement actor did not settle");
}

async fn confirmed_scan_cleanup() {
    tokio::time::timeout(Duration::from_secs(8), async {
        while rqs_lib::hdl::wait_for_scans().await.is_err() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("The original scanner did not confirm its own cleanup");
}

async fn release(bus: &zbus::Connection, state: &State) {
    let (path, owner) = {
        let mut active = state.active.lock().unwrap();
        let (path, owner) = active
            .iter()
            .next()
            .map(|(p, o)| (p.clone(), o.clone()))
            .unwrap();
        active.remove(&path);
        (path, owner)
    };
    let proxy = zbus::Proxy::new(
        bus,
        owner.as_str(),
        path.as_str(),
        "org.bluez.LEAdvertisement1",
    )
    .await
    .unwrap();
    proxy.call::<_, _, ()>("Release", &()).await.unwrap();
}

#[tokio::test]
#[ignore = "requires the private mock bus in run-bluetooth-lifecycle.sh"]
async fn receiver_recovers_release_and_sender_reports_loss_on_the_selected_controller() {
    assert_eq!(
        std::env::var("LINUXDROP_TEST_PRIVATE_BLUEZ").as_deref(),
        Ok("1")
    );
    assert_eq!(
        std::env::var("DBUS_SYSTEM_BUS_ADDRESS").unwrap(),
        std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap()
    );
    let first = Arc::new(State::default());
    let second = Arc::new(State::default());
    let bus = zbus::connection::Builder::session()
        .unwrap()
        .name("org.bluez")
        .unwrap()
        .serve_at("/", zbus::fdo::ObjectManager)
        .unwrap()
        .serve_at("/org/bluez/hci0", Adapter(Arc::new(AtomicBool::new(true))))
        .unwrap()
        .serve_at("/org/bluez/hci0", Advertising(first.clone()))
        .unwrap()
        .serve_at("/org/bluez/hci1", Adapter(Arc::new(AtomicBool::new(true))))
        .unwrap()
        .serve_at("/org/bluez/hci1", Advertising(second.clone()))
        .unwrap()
        .build()
        .await
        .unwrap();
    rqs_lib::set_bluetooth_adapter(Some("hci1".into()));
    let receiver = rqs_lib::hdl::ReceiverAdvertiser::new(*b"TEST", 1, "Test receiver", None)
        .await
        .unwrap();
    let (visibility, watched) = tokio::sync::watch::channel(rqs_lib::Visibility::Visible);
    let cancel = CancellationToken::new();
    let run_cancel = cancel.clone();
    let task = tokio::spawn(async move { receiver.run(watched, run_cancel).await });
    wait_for(|| second.active.lock().unwrap().len() == 1).await;
    let count = second.registrations.load(Ordering::SeqCst);
    release(&bus, &second).await;
    wait_for(|| {
        second.registrations.load(Ordering::SeqCst) > count
            && second.active.lock().unwrap().len() == 1
    })
    .await;
    assert!(
        !task.is_finished(),
        "The receiver should recover from an external Release"
    );
    // A replacement bluetoothd gets new exported objects. The running receiver
    // must renew on it; cleanup remains directed at the still-live old owner.
    let old_bus = bus;
    assert!(old_bus.release_name("org.bluez").await.unwrap());
    let previous_state = second;
    let second = Arc::new(State::default());
    let bus = zbus::connection::Builder::session()
        .unwrap()
        .name("org.bluez")
        .unwrap()
        .serve_at("/", zbus::fdo::ObjectManager)
        .unwrap()
        .serve_at("/org/bluez/hci0", Adapter(Arc::new(AtomicBool::new(true))))
        .unwrap()
        .serve_at("/org/bluez/hci0", Advertising(first.clone()))
        .unwrap()
        .serve_at("/org/bluez/hci1", Adapter(Arc::new(AtomicBool::new(true))))
        .unwrap()
        .serve_at("/org/bluez/hci1", Advertising(second.clone()))
        .unwrap()
        .build()
        .await
        .unwrap();
    wait_for(|| second.active.lock().unwrap().len() == 1).await;
    assert!(previous_state.active.lock().unwrap().is_empty());
    assert_eq!(
        second.removals.load(Ordering::SeqCst),
        0,
        "Old cleanup must not touch the replacement service"
    );
    assert!(!task.is_finished());
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(second.active.lock().unwrap().is_empty());
    drop(visibility);

    let sender = rqs_lib::hdl::BleAdvertiser::new().await.unwrap();
    let task = tokio::spawn(async move { sender.run(CancellationToken::new()).await });
    wait_for(|| second.active.lock().unwrap().len() == 1).await;
    release(&bus, &second).await;
    let error = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("no longer active"));
    assert!(second.active.lock().unwrap().is_empty());
    assert_eq!(first.registrations.load(Ordering::SeqCst), 0);

    // StartNotify must acknowledge registration immediately, not await a peer's
    // handshake. A cancelled server must release its scan suppressor before the
    // real scanner below is started.
    let gatt_state = Arc::new(gatt::State::default());
    bus.object_server()
        .at("/org/bluez/hci1", gatt::Gatt(gatt_state.clone()))
        .await
        .unwrap();
    let (events, _) = tokio::sync::broadcast::channel(16);
    let server = rqs_lib::hdl::ReceiverGattServer::new(vec![1, 2, 3], events, 53318)
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    let stopping = cancel.clone();
    let running = tokio::spawn(async move { server.run(stopping).await });
    wait_for(|| gatt_state.active.lock().unwrap().is_some()).await;
    let (path, owner) = gatt_state.active.lock().unwrap().clone().unwrap();
    let writer_path = format!("{path}/service0/char1");
    let writer = zbus::Proxy::new(
        &bus,
        owner.as_str(),
        writer_path.as_str(),
        "org.bluez.GattCharacteristic1",
    )
    .await
    .unwrap();
    let device =
        zbus::zvariant::OwnedObjectPath::try_from("/org/bluez/hci1/dev_00_11_22_33_44_55").unwrap();
    let options = std::collections::HashMap::from([
        ("device", zbus::zvariant::Value::from(device)),
        ("mtu", zbus::zvariant::Value::from(512_u16)),
    ]);
    let oversized = writer
        .call::<_, _, ()>("WriteValue", &(vec![0_u8; 513], &options))
        .await;
    assert!(
        oversized
            .unwrap_err()
            .to_string()
            .contains("InvalidValueLength")
    );
    for _ in 0..128 {
        writer
            .call::<_, _, ()>("WriteValue", &(vec![0_u8], &options))
            .await
            .unwrap();
    }
    let full = writer
        .call::<_, _, ()>("WriteValue", &(vec![0_u8], &options))
        .await;
    assert!(full.unwrap_err().to_string().contains("InProgress"));
    let notify_path = format!("{path}/service0/char2");
    let notify = zbus::Proxy::new(
        &bus,
        owner.as_str(),
        notify_path.as_str(),
        "org.bluez.GattCharacteristic1",
    )
    .await
    .unwrap();
    tokio::time::timeout(
        Duration::from_secs(1),
        notify.call::<_, _, ()>("StartNotify", &()),
    )
    .await
    .unwrap()
    .unwrap();
    gatt_state.hold_unregister.store(true, Ordering::SeqCst);
    cancel.cancel();
    wait_for(|| gatt_state.removals.load(Ordering::SeqCst) == 1).await;
    assert!(
        !running.is_finished(),
        "GATT server must wait for BlueZ removal acknowledgement"
    );
    gatt_state.hold_unregister.store(false, Ordering::SeqCst);
    gatt_state.wake.notify_waiters();
    tokio::time::timeout(Duration::from_secs(1), running)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(gatt_state.active.lock().unwrap().is_none());
    assert!(
        notify.call::<_, _, ()>("StartNotify", &()).await.is_err(),
        "Removed GATT objects must reject late callbacks"
    );

    // Abandon an in-flight registration after BlueZ processed it. The owned
    // registration worker must still remove the late successful registration.
    gatt_state.hold_register.store(true, Ordering::SeqCst);
    let (events, _) = tokio::sync::broadcast::channel(16);
    let server = rqs_lib::hdl::ReceiverGattServer::new(vec![1], events, 53318)
        .await
        .unwrap();
    let running = tokio::spawn(async move { server.run(CancellationToken::new()).await });
    wait_for(|| gatt_state.active.lock().unwrap().is_some()).await;
    running.abort();
    assert!(running.await.unwrap_err().is_cancelled());
    gatt_state.hold_register.store(false, Ordering::SeqCst);
    gatt_state.wake.notify_waiters();
    wait_for(|| {
        gatt_state.removals.load(Ordering::SeqCst) == 2
            && gatt_state.active.lock().unwrap().is_none()
    })
    .await;

    // A failed reply is also uncertain: cleanup must complete before returning
    // the registration error to the backend.
    gatt_state.fail_register.store(true, Ordering::SeqCst);
    let (events, _) = tokio::sync::broadcast::channel(16);
    let server = rqs_lib::hdl::ReceiverGattServer::new(vec![1], events, 53318)
        .await
        .unwrap();
    assert!(server.run(CancellationToken::new()).await.is_err());
    assert!(gatt_state.active.lock().unwrap().is_none());
    assert_eq!(gatt_state.removals.load(Ordering::SeqCst), 3);

    // Exercise the real btleplug/bluez-async scanner against the same private
    // bus, with the full Adapter1 property schema that it reads.
    bus.object_server()
        .remove::<Adapter, _>("/org/bluez/hci0")
        .await
        .unwrap();
    bus.object_server()
        .remove::<Adapter, _>("/org/bluez/hci1")
        .await
        .unwrap();
    let first_scan = Arc::new(scanner::State::default());
    let selected_scan = Arc::new(scanner::State::default());
    first_scan.powered.store(true, Ordering::SeqCst);
    selected_scan.powered.store(true, Ordering::SeqCst);
    bus.object_server()
        .at("/org/bluez/hci0", scanner::Scanner(first_scan.clone()))
        .await
        .unwrap();
    bus.object_server()
        .at("/org/bluez/hci1", scanner::Scanner(selected_scan.clone()))
        .await
        .unwrap();
    let (alerts, _) = tokio::sync::broadcast::channel(4);
    let listener = rqs_lib::hdl::BleListener::new(alerts.clone())
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    let run_cancel = cancel.clone();
    let running = tokio::spawn(async move { listener.run(run_cancel).await });
    wait_for(|| selected_scan.starts.load(Ordering::SeqCst) == 1 || running.is_finished()).await;
    assert!(
        !running.is_finished(),
        "Scanner ended before its first scan: {:?}",
        running.await
    );
    selected_scan.hold_stop.store(true, Ordering::SeqCst);
    cancel.cancel();
    wait_for(|| selected_scan.stops.load(Ordering::SeqCst) == 1).await;
    assert!(
        !running.is_finished(),
        "Scanner must retain its cleanup until acknowledgement"
    );
    selected_scan.hold_stop.store(false, Ordering::SeqCst);
    selected_scan.wake.notify_waiters();
    tokio::time::timeout(Duration::from_secs(2), running)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(!selected_scan.active.load(Ordering::SeqCst));
    assert_eq!(first_scan.starts.load(Ordering::SeqCst), 0);

    selected_scan.fail_start.store(true, Ordering::SeqCst);
    let listener = rqs_lib::hdl::BleListener::new(alerts.clone())
        .await
        .unwrap();
    let error = listener.run(CancellationToken::new()).await.unwrap_err();
    assert!(error.to_string().contains("could not start"));
    assert_eq!(selected_scan.stops.load(Ordering::SeqCst), 2);
    assert!(!selected_scan.active.load(Ordering::SeqCst));
    selected_scan.fail_start.store(false, Ordering::SeqCst);

    selected_scan.fail_stop.store(true, Ordering::SeqCst);
    let listener = rqs_lib::hdl::BleListener::new(alerts.clone())
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    let run_cancel = cancel.clone();
    let running = tokio::spawn(async move { listener.run(run_cancel).await });
    wait_for(|| selected_scan.starts.load(Ordering::SeqCst) == 3).await;
    cancel.cancel();
    let error = tokio::time::timeout(Duration::from_secs(2), running)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("cleanup failed"));
    assert_eq!(first_scan.starts.load(Ordering::SeqCst), 0);
    selected_scan.powered.store(false, Ordering::SeqCst);
    assert!(
        rqs_lib::hdl::BleListener::new(alerts).await.is_err(),
        "An unavailable selection must not switch to hci0"
    );

    // Exercise the complete adapter -> RQS worker tracker -> CommandSender
    // receipt path. The runner disables recipient discovery for this case;
    // its separately owned scan is covered by recipient_scan.rs. A recoverable
    // scan-start error must not block shutdown;
    // an unacknowledged StopDiscovery must remain visible on every retry.
    selected_scan.powered.store(true, Ordering::SeqCst);
    // Prevent the outgoing advertiser from intentionally suppressing this scanner.
    bus.object_server()
        .remove::<Advertising, _>("/org/bluez/hci1")
        .await
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    for (start_fails, cleanup_fails) in [(true, false), (false, true)] {
        selected_scan.fail_stop.store(false, Ordering::SeqCst);
        confirmed_scan_cleanup().await;
        selected_scan
            .fail_start
            .store(start_fails, Ordering::SeqCst);
        selected_scan
            .fail_stop
            .store(cleanup_fails, Ordering::SeqCst);
        let (events, mut receiver) = tokio::sync::mpsc::channel(128);
        let commands = linuxdrop_quickshare::start(
            linuxdrop_quickshare::Config {
                name: "Cleanup receipt test".into(),
                download_dir: directory.path().into(),
                visible: false,
                port: None,
                ble: true,
                max_receive_bytes: 1024,
                max_files: 1,
                upgrade_lease: None,
                p2p_connector: None,
                policy: linuxdrop_core::TransferPolicy {
                    bluetooth_adapter: Some("hci1".into()),
                    allowed_interfaces: vec!["lo".into()],
                    ..Default::default()
                },
            },
            events,
        )
        .await
        .unwrap();
        // Outbound discovery has its own scanner. Wait for this worker's status,
        // not an adapter-wide start counter that could belong to discovery.
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                let event = receiver.recv().await.expect("Backend state stream closed");
                if let linuxdrop_core::BackendEvent::StateChanged(state) = event
                    && (start_fails
                        && state.detail.contains(
                            "bluetooth-listener unavailable: Bluetooth scan could not start",
                        )
                        || !start_fails
                            && !state.detail.contains("bluetooth-listener")
                            && !state.detail.contains("scanner paused"))
                {
                    break;
                }
            }
        })
        .await
        .unwrap();
        let result = commands.shutdown().await;
        if cleanup_fails {
            let error = result.unwrap_err();
            assert!(
                error.contains("bluetooth-listener") && error.contains("cleanup failed"),
                "{error}"
            );
            assert_eq!(commands.shutdown().await.unwrap_err(), error);
        } else {
            result.unwrap();
        }
    }
    selected_scan.fail_stop.store(false, Ordering::SeqCst);
    confirmed_scan_cleanup().await;
    let (alerts, _) = tokio::sync::broadcast::channel(4);
    let powered = rqs_lib::hdl::BleListener::new(alerts).await.unwrap();
    let starts = selected_scan.starts.load(Ordering::SeqCst);
    selected_scan.powered.store(false, Ordering::SeqCst);
    let error = powered.run(CancellationToken::new()).await.unwrap_err();
    assert!(format!("{error:#}").contains("switched off"), "{error:#}");
    assert_eq!(selected_scan.starts.load(Ordering::SeqCst), starts);
    selected_scan.powered.store(true, Ordering::SeqCst);

    // A stale scanner must clean its original daemon, never its replacement.
    let (alerts, _) = tokio::sync::broadcast::channel(4);
    let parked = rqs_lib::hdl::BleListener::new(alerts.clone())
        .await
        .unwrap();
    let old = rqs_lib::hdl::BleListener::new(alerts.clone())
        .await
        .unwrap();
    let cancel_old = CancellationToken::new();
    let run_cancel = cancel_old.clone();
    let starts = selected_scan.starts.load(Ordering::SeqCst);
    let old_task = tokio::spawn(async move { old.run(run_cancel).await });
    wait_for(|| selected_scan.starts.load(Ordering::SeqCst) > starts).await;
    bus.release_name("org.bluez").await.unwrap();
    let replacement_state = Arc::new(scanner::State::default());
    replacement_state.powered.store(true, Ordering::SeqCst);
    let replacement = zbus::connection::Builder::session()
        .unwrap()
        .name("org.bluez")
        .unwrap()
        .serve_at("/", zbus::fdo::ObjectManager)
        .unwrap()
        .serve_at(
            "/org/bluez/hci1",
            scanner::Scanner(replacement_state.clone()),
        )
        .unwrap()
        .build()
        .await
        .unwrap();
    let fresh = rqs_lib::hdl::BleListener::new(alerts).await.unwrap();
    let cancel_fresh = CancellationToken::new();
    let run_cancel = cancel_fresh.clone();
    let fresh_task = tokio::spawn(async move { fresh.run(run_cancel).await });
    wait_for(|| replacement_state.starts.load(Ordering::SeqCst) == 1).await;
    cancel_old.cancel();
    let error = tokio::time::timeout(Duration::from_secs(2), old_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    // The shared scan turn means the old generation has already acknowledged
    // cleanup after owner loss before the replacement is allowed to start.
    assert!(error.to_string().contains("owner changed"));
    assert_eq!(
        replacement_state.stops.load(Ordering::SeqCst),
        0,
        "Old scanner cleanup reached the replacement BlueZ daemon"
    );
    assert!(replacement_state.active.load(Ordering::SeqCst));
    assert!(!selected_scan.active.load(Ordering::SeqCst));
    let error = tokio::time::timeout(Duration::from_secs(2), parked.run(CancellationToken::new()))
        .await
        .unwrap()
        .unwrap_err();
    assert!(format!("{error:#}").contains("owner changed"), "{error:#}");
    assert_eq!(replacement_state.starts.load(Ordering::SeqCst), 1);
    assert_eq!(replacement_state.stops.load(Ordering::SeqCst), 0);
    cancel_fresh.cancel();
    tokio::time::timeout(Duration::from_secs(2), fresh_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(replacement_state.stops.load(Ordering::SeqCst), 1);
    // A dead unique owner has released its per-client scan implicitly.
    let (alerts, _) = tokio::sync::broadcast::channel(4);
    let dying = rqs_lib::hdl::BleListener::new(alerts).await.unwrap();
    let cancel_dying = CancellationToken::new();
    let run_cancel = cancel_dying.clone();
    let dying_task = tokio::spawn(async move { dying.run(run_cancel).await });
    wait_for(|| replacement_state.starts.load(Ordering::SeqCst) == 2).await;
    replacement.close().await.unwrap();
    let third_state = Arc::new(scanner::State::default());
    third_state.powered.store(true, Ordering::SeqCst);
    let third = zbus::connection::Builder::session()
        .unwrap()
        .name("org.bluez")
        .unwrap()
        .serve_at("/", zbus::fdo::ObjectManager)
        .unwrap()
        .serve_at("/org/bluez/hci1", scanner::Scanner(third_state.clone()))
        .unwrap()
        .build()
        .await
        .unwrap();
    cancel_dying.cancel();
    tokio::time::timeout(Duration::from_secs(2), dying_task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        third_state.stops.load(Ordering::SeqCst),
        0,
        "Cleanup after daemon disconnection must not mutate a replacement"
    );
    // The production supervisor starts offline and recovers without backend restart.
    third_state.set_power(false);
    let (alerts, _) = tokio::sync::broadcast::channel(4);
    let (status, mut messages) = tokio::sync::broadcast::channel(64);
    let cancel = CancellationToken::new();
    let run_cancel = cancel.clone();
    let supervisor = tokio::spawn(async move {
        rqs_lib::hdl::BleListener::supervise(alerts, status, run_cancel).await
    });
    async fn scanner_state(
        messages: &mut tokio::sync::broadcast::Receiver<rqs_lib::channel::ChannelMessage>,
        ready: Option<bool>,
    ) {
        tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                match messages.recv().await.unwrap().msg {
                    rqs_lib::channel::Message::BluetoothScannerReady { adapter, paused }
                        if ready == Some(paused) =>
                    {
                        assert_eq!(adapter, "hci1");
                        return;
                    }
                    rqs_lib::channel::Message::Backend { component, .. }
                        if ready.is_none() && component == "bluetooth-listener" =>
                    {
                        return;
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("Scanner recovery state did not arrive");
    }
    scanner_state(&mut messages, None).await;
    assert_eq!(third_state.starts.load(Ordering::SeqCst), 0);
    let suppression = rqs_lib::hdl::BleScanSuppressor::new();
    third_state.powered.store(true, Ordering::SeqCst);
    scanner_state(&mut messages, Some(true)).await;
    assert_eq!(
        third_state.starts.load(Ordering::SeqCst),
        0,
        "Protected radio airtime must remain paused, not reported as an active scan"
    );
    third_state.set_power(false);
    scanner_state(&mut messages, None).await;
    drop(suppression);
    third_state.powered.store(true, Ordering::SeqCst);
    scanner_state(&mut messages, Some(false)).await;
    assert!(third_state.active.load(Ordering::SeqCst));
    third_state.set_power(false);
    scanner_state(&mut messages, None).await;
    assert!(!third_state.active.load(Ordering::SeqCst));
    third_state.powered.store(true, Ordering::SeqCst);
    scanner_state(&mut messages, Some(false)).await;
    third
        .object_server()
        .remove::<scanner::Scanner, _>("/org/bluez/hci1")
        .await
        .unwrap();
    third_state.active.store(false, Ordering::SeqCst);
    scanner_state(&mut messages, None).await;
    third
        .object_server()
        .at("/org/bluez/hci1", scanner::Scanner(third_state.clone()))
        .await
        .unwrap();
    scanner_state(&mut messages, Some(false)).await;
    let fourth_state = Arc::new(scanner::State::default());
    fourth_state.powered.store(true, Ordering::SeqCst);
    third.release_name("org.bluez").await.unwrap();
    let fourth = zbus::connection::Builder::session()
        .unwrap()
        .name("org.bluez")
        .unwrap()
        .serve_at("/", zbus::fdo::ObjectManager)
        .unwrap()
        .serve_at("/org/bluez/hci0", scanner::Scanner(first_scan.clone()))
        .unwrap()
        .serve_at("/org/bluez/hci1", scanner::Scanner(fourth_state.clone()))
        .unwrap()
        .build()
        .await
        .unwrap();
    scanner_state(&mut messages, None).await;
    scanner_state(&mut messages, Some(false)).await;
    assert!(!third_state.active.load(Ordering::SeqCst));
    assert_eq!(fourth_state.starts.load(Ordering::SeqCst), 1);
    assert_eq!(first_scan.starts.load(Ordering::SeqCst), 0);
    assert!(!supervisor.is_finished());
    // Never retry when cleanup of a still-live controller is unconfirmed.
    fourth_state.fail_stop.store(true, Ordering::SeqCst);
    fourth_state.powered.store(false, Ordering::SeqCst);
    let error = tokio::time::timeout(Duration::from_secs(5), supervisor)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("Unconfirmed cleanup"));
    assert_eq!(fourth_state.starts.load(Ordering::SeqCst), 1);
    fourth_state.fail_stop.store(false, Ordering::SeqCst);
    cancel.cancel();
    // Cancelling during unavailable-controller backoff must return immediately.
    let (alerts, _) = tokio::sync::broadcast::channel(4);
    let (status, mut messages) = tokio::sync::broadcast::channel(4);
    let cancel = CancellationToken::new();
    let run_cancel = cancel.clone();
    let waiting = tokio::spawn(async move {
        rqs_lib::hdl::BleListener::supervise(alerts, status, run_cancel).await
    });
    scanner_state(&mut messages, None).await;
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(1), waiting)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(fourth_state.starts.load(Ordering::SeqCst), 1);
    drop(fourth);
}
