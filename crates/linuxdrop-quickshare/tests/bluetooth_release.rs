//! Real Quick Share advertisement actors against the isolated BlueZ mock.
#[path = "../../linuxdrop-network/tests/support/bluez.rs"]
mod bluez;
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
}
