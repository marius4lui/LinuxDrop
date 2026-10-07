use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[path = "../../linuxdrop-network/tests/support/bluez.rs"]
mod bluez;

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
    wait_for(|| updates.borrow().contains("wake is active")).await;
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
}
