use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[path = "../../linuxdrop-network/tests/support/bluez.rs"]
mod bluez;
#[path = "../../linuxdrop-quickshare/tests/support/gatt.rs"]
mod gatt;

async fn wait_for(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(12), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("AirDrop Bluetooth recovery did not settle");
}

#[tokio::test]
#[ignore = "requires the private BlueZ bus runner"]
async fn wake_recovers_without_restarting_awdl_and_drains_shutdown() {
    assert_eq!(
        std::env::var("LINUXDROP_TEST_PRIVATE_BLUEZ").as_deref(),
        Ok("1")
    );
    assert_eq!(
        std::env::var("DBUS_SYSTEM_BUS_ADDRESS").unwrap(),
        std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap()
    );
    let state = Arc::new(bluez::State::default());
    let power = Arc::new(AtomicBool::new(false));
    let foreign = Arc::new(bluez::State::default());
    let bus = zbus::connection::Builder::session()
        .unwrap()
        .name("org.bluez")
        .unwrap()
        .serve_at("/", zbus::fdo::ObjectManager)
        .unwrap()
        .serve_at(
            "/org/bluez/hci0",
            bluez::Adapter(Arc::new(AtomicBool::new(true))),
        )
        .unwrap()
        .serve_at("/org/bluez/hci0", bluez::Advertising(foreign.clone()))
        .unwrap()
        .serve_at("/org/bluez/hci1", bluez::Adapter(power.clone()))
        .unwrap()
        .serve_at("/org/bluez/hci1", bluez::Advertising(state.clone()))
        .unwrap()
        .build()
        .await
        .unwrap();
    let (status, updates) = watch::channel(String::new());
    let stop = CancellationToken::new();
    let task = tokio::spawn(super::supervise(Some("hci1".into()), status, stop.clone()));
    wait_for(|| updates.borrow().contains("unavailable")).await;
    assert_eq!(
        foreign.registrations.load(Ordering::SeqCst),
        0,
        "Explicit selection must not fall back to another controller"
    );
    power.store(true, Ordering::SeqCst);
    wait_for(|| updates.borrow().contains("shared discovery windows")).await;
    assert_eq!(state.active.lock().unwrap().len(), 1);

    power.store(false, Ordering::SeqCst);
    let adapter = bus
        .object_server()
        .interface::<_, bluez::Adapter>("/org/bluez/hci1")
        .await
        .unwrap();
    adapter
        .get()
        .await
        .powered_changed(adapter.signal_emitter())
        .await
        .unwrap();
    wait_for(|| state.active.lock().unwrap().is_empty()).await;
    power.store(true, Ordering::SeqCst);
    wait_for(|| {
        state.registrations.load(Ordering::SeqCst) == 2 && state.active.lock().unwrap().len() == 1
    })
    .await;

    // A new bluetoothd generation must receive a fresh advertisement; cleanup
    // for the old generation stays on its old unique bus owner.
    bus.release_name("org.bluez").await.unwrap();
    let replacement = Arc::new(bluez::State::default());
    let gatt = Arc::new(gatt::State::default());
    let _new_bus = zbus::connection::Builder::session()
        .unwrap()
        .name("org.bluez")
        .unwrap()
        .serve_at("/", zbus::fdo::ObjectManager)
        .unwrap()
        .serve_at("/org/bluez/hci1", bluez::Adapter(power))
        .unwrap()
        .serve_at("/org/bluez/hci1", bluez::Advertising(replacement.clone()))
        .unwrap()
        .serve_at("/org/bluez/hci1", gatt::Gatt(gatt.clone()))
        .unwrap()
        .build()
        .await
        .unwrap();
    wait_for(|| replacement.active.lock().unwrap().len() == 1).await;
    assert!(state.active.lock().unwrap().is_empty());
    assert_eq!(replacement.removals.load(Ordering::SeqCst), 0);
    replacement.hold_unregister.store(true, Ordering::SeqCst);
    stop.cancel();
    wait_for(|| replacement.removals.load(Ordering::SeqCst) == 1).await;
    assert!(!task.is_finished(), "Shutdown must await BlueZ removal");
    replacement.hold_unregister.store(false, Ordering::SeqCst);
    replacement.wake.notify_waiters();
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(replacement.active.lock().unwrap().is_empty());
    assert_eq!(foreign.registrations.load(Ordering::SeqCst), 0);

    // Exercise both real protocol advertising workers on the same single-slot
    // controller. An in-progress connection protects the receiver's turn.
    rqs_lib::set_bluetooth_adapter(Some("hci1".into()));
    let (visible, visibility) = watch::channel(rqs_lib::Visibility::Visible);
    let (messages, mut receiver_events) = tokio::sync::broadcast::channel(128);
    let receiver_stop = CancellationToken::new();
    let receiver = tokio::spawn(rqs_lib::hdl::supervise_receiver(
        *b"TEST",
        1,
        "Receiver".into(),
        12345,
        visibility,
        messages,
        receiver_stop.clone(),
    ));
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let rqs_lib::channel::Message::BluetoothServiceReady { component } =
                receiver_events.recv().await.unwrap().msg
                && component == "bluetooth-receiver"
            {
                break;
            }
        }
    })
    .await
    .unwrap();
    let connection = rqs_lib::hdl::BleScanSuppressor::new();
    let before = replacement.registrations.load(Ordering::SeqCst);
    let (status, _updates) = watch::channel(String::new());
    let apple_stop = CancellationToken::new();
    let apple = tokio::spawn(super::supervise(
        Some("hci1".into()),
        status,
        apple_stop.clone(),
    ));
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        replacement.registrations.load(Ordering::SeqCst),
        before,
        "AirDrop must wait for the active Quick Share connection"
    );
    drop(connection);
    wait_for(|| replacement.registrations.load(Ordering::SeqCst) >= before + 4).await;
    let types = replacement
        .properties
        .lock()
        .unwrap()
        .iter()
        .skip(before)
        .map(|properties| String::try_from(properties["Type"].try_clone().unwrap()).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        &types[..4],
        &["broadcast", "peripheral", "broadcast", "peripheral"],
        "Both protocols must receive repeated turns"
    );
    assert_eq!(
        gatt.registrations.load(Ordering::SeqCst),
        1,
        "Advertising turns must preserve the GATT server"
    );
    assert_eq!(replacement.active.lock().unwrap().len(), 1);
    apple_stop.cancel();
    receiver_stop.cancel();
    drop(visible);
    tokio::time::timeout(Duration::from_secs(5), async {
        apple.await.unwrap().unwrap();
        receiver.await.unwrap().unwrap();
    })
    .await
    .unwrap();
    assert!(replacement.active.lock().unwrap().is_empty());
}
