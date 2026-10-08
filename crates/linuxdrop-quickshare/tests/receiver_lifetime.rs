//! Production receiver recovery against an explicitly isolated BlueZ service.
#[path = "../../linuxdrop-network/tests/support/bluez.rs"]
mod bluez;
#[path = "support/gatt.rs"]
mod gatt;
use rqs_lib::channel::{ChannelMessage, Message};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio_util::sync::CancellationToken;

async fn wait_for(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(12), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("receiver lifecycle did not settle");
}
async fn ready(messages: &mut tokio::sync::broadcast::Receiver<ChannelMessage>) {
    tokio::time::timeout(Duration::from_secs(12), async {
        let mut gatt = false;
        let mut advert = false;
        while !gatt || !advert {
            if let Message::BluetoothServiceReady { component } = messages.recv().await.unwrap().msg
            {
                gatt |= component == "bluetooth-gatt";
                advert |= component == "bluetooth-receiver";
            }
        }
    })
    .await
    .expect("receiver did not report readiness");
}
async fn bus(
    power: Arc<AtomicBool>,
    advert: Arc<bluez::State>,
    gatt: Arc<gatt::State>,
) -> zbus::Connection {
    zbus::connection::Builder::session()
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
        .serve_at("/org/bluez/hci1", bluez::Adapter(power))
        .unwrap()
        .serve_at("/org/bluez/hci1", bluez::Advertising(advert))
        .unwrap()
        .serve_at("/org/bluez/hci1", gatt::Gatt(gatt))
        .unwrap()
        .build()
        .await
        .unwrap()
}
#[tokio::test]
#[ignore = "requires private BlueZ bus from run-bluetooth-lifecycle.sh"]
async fn receiver_rebuilds_only_after_acknowledged_cleanup_and_never_reports_late_ready() {
    assert_eq!(
        std::env::var("LINUXDROP_TEST_PRIVATE_BLUEZ").as_deref(),
        Ok("1")
    );
    assert_eq!(
        std::env::var("DBUS_SYSTEM_BUS_ADDRESS").unwrap(),
        std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap()
    );
    rqs_lib::set_bluetooth_adapter(Some("hci1".into()));
    let power = Arc::new(AtomicBool::new(false));
    let advert = Arc::new(bluez::State::default());
    let gatt = Arc::new(gatt::State::default());
    let original = bus(power.clone(), advert.clone(), gatt.clone()).await;
    let (_visibility, watched) = tokio::sync::watch::channel(rqs_lib::Visibility::Visible);
    let (status, mut messages) = tokio::sync::broadcast::channel(128);
    let cancel = CancellationToken::new();
    let run_cancel = cancel.clone();
    let task = tokio::spawn(rqs_lib::hdl::supervise_receiver(
        *b"TEST",
        1,
        "Receiver".into(),
        12345,
        watched,
        status.clone(),
        run_cancel,
    ));
    // Unavailable preferred hci1 never silently falls back to powered hci0.
    let initial = tokio::time::timeout(Duration::from_secs(1), messages.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(initial.msg, Message::Backend { .. }));
    assert_eq!(gatt.registrations.load(Ordering::SeqCst), 0);
    power.store(true, Ordering::SeqCst);
    ready(&mut messages).await;
    assert_eq!(gatt.registrations.load(Ordering::SeqCst), 1);
    gatt.hold_unregister.store(true, Ordering::SeqCst);
    power.store(false, Ordering::SeqCst);
    wait_for(|| gatt.removals.load(Ordering::SeqCst) == 1).await;
    power.store(true, Ordering::SeqCst);
    // This exceeds the first retry delay: no replacement before acknowledgement.
    tokio::time::sleep(Duration::from_millis(2200)).await;
    assert_eq!(gatt.registrations.load(Ordering::SeqCst), 1);
    assert!(!task.is_finished());
    gatt.hold_unregister.store(false, Ordering::SeqCst);
    gatt.wake.notify_waiters();
    ready(&mut messages).await;
    assert_eq!(gatt.registrations.load(Ordering::SeqCst), 2);
    assert_eq!(advert.active.lock().unwrap().len(), 1);
    // Replace bluetoothd while keeping its old unique owner reachable for cleanup.
    original.release_name("org.bluez").await.unwrap();
    let new_advert = Arc::new(bluez::State::default());
    let new_gatt = Arc::new(gatt::State::default());
    let replacement = bus(power.clone(), new_advert.clone(), new_gatt.clone()).await;
    ready(&mut messages).await;
    assert!(gatt.active.lock().unwrap().is_none());
    assert!(advert.active.lock().unwrap().is_empty());
    assert_eq!(new_gatt.registrations.load(Ordering::SeqCst), 1);
    assert_eq!(new_gatt.removals.load(Ordering::SeqCst), 0);
    assert_eq!(new_advert.active.lock().unwrap().len(), 1);
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(new_gatt.active.lock().unwrap().is_none());
    assert!(new_advert.active.lock().unwrap().is_empty());

    // Registration can finish late, but it must be removed, never announced ready.
    new_gatt.hold_register.store(true, Ordering::SeqCst);
    let cancel = CancellationToken::new();
    let (_visible, watched) = tokio::sync::watch::channel(rqs_lib::Visibility::Invisible);
    let mut events = status.subscribe();
    let task = tokio::spawn(rqs_lib::hdl::supervise_receiver(
        *b"TEST",
        1,
        "Receiver".into(),
        12345,
        watched,
        status,
        cancel.clone(),
    ));
    wait_for(|| new_gatt.registrations.load(Ordering::SeqCst) == 2).await;
    cancel.cancel();
    new_gatt.hold_register.store(false, Ordering::SeqCst);
    new_gatt.wake.notify_waiters();
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    while let Ok(event) = events.try_recv() {
        assert!(
            !matches!(event.msg, Message::BluetoothServiceReady {component} if component == "bluetooth-gatt")
        );
    }
    assert!(new_gatt.active.lock().unwrap().is_none());
    assert!(new_advert.active.lock().unwrap().is_empty());
    // A cleanup deadline is terminal: keep ownership, do not create duplicates.
    new_gatt.hold_unregister.store(true, Ordering::SeqCst);
    let cancel = CancellationToken::new();
    let (_visible, watched) = tokio::sync::watch::channel(rqs_lib::Visibility::Visible);
    let (status, mut events) = tokio::sync::broadcast::channel(128);
    let task = tokio::spawn(rqs_lib::hdl::supervise_receiver(
        *b"TEST",
        1,
        "Receiver".into(),
        12345,
        watched,
        status,
        cancel.clone(),
    ));
    ready(&mut events).await;
    let registrations = new_gatt.registrations.load(Ordering::SeqCst);
    power.store(false, Ordering::SeqCst);
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Message::Backend { detail, .. } = events.recv().await.unwrap().msg
                && detail.contains("Unconfirmed cleanup")
            {
                break;
            }
        }
    })
    .await
    .expect("cleanup deadline was not reported");
    power.store(true, Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(2200)).await;
    assert!(!task.is_finished());
    assert_eq!(new_gatt.registrations.load(Ordering::SeqCst), registrations);
    cancel.cancel();
    let error = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("Unconfirmed cleanup"));
    new_gatt.hold_unregister.store(false, Ordering::SeqCst);
    new_gatt.wake.notify_waiters();
    wait_for(|| new_gatt.active.lock().unwrap().is_none()).await;
    drop(replacement);
}
