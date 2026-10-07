//! Bounded production BlueZ queues on a private bus, including scanner recovery.
#[path = "support/scanner.rs"]
mod scanner;
use futures_util::StreamExt;
use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
const ADAPTER: &str = "/org/bluez/hci1";
async fn emit(bus: &zbus::Connection, large: bool) {
    let mut properties = std::collections::HashMap::new();
    if large {
        properties.insert(
            "Unknown",
            zbus::zvariant::Value::from(vec![0u8; 2 * 1024 * 1024 + 1]),
        );
    } else {
        properties.insert("Powered", zbus::zvariant::Value::from(true));
    }
    bus.emit_signal(
        None::<&str>,
        ADAPTER,
        "org.freedesktop.DBus.Properties",
        "PropertiesChanged",
        &("org.bluez.Adapter1", properties, Vec::<String>::new()),
    )
    .await
    .unwrap();
}
async fn wait_for(mut check: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !check() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
#[ignore = "requires private bus in run-bluetooth-lifecycle.sh"]
async fn signal_overflow_ends_all_matches_and_scanners_recover_after_cleanup() {
    assert_eq!(
        std::env::var("LINUXDROP_TEST_PRIVATE_BLUEZ").as_deref(),
        Ok("1")
    );
    assert_eq!(
        std::env::var("DBUS_SYSTEM_BUS_ADDRESS").unwrap(),
        std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap()
    );
    let radio = Arc::new(scanner::State::default());
    radio.set_power(true);
    let bus = zbus::connection::Builder::session()
        .unwrap()
        .name("org.bluez")
        .unwrap()
        .serve_at("/", zbus::fdo::ObjectManager)
        .unwrap()
        .serve_at(ADAPTER, scanner::Scanner(radio.clone()))
        .unwrap()
        .build()
        .await
        .unwrap();
    let (_, session) = bluez_async::BluetoothSession::new().await.unwrap();
    let id = session.get_adapters().await.unwrap().remove(0).id;
    // Slow consumers cannot retain unbounded properties or hang on the sibling
    // InterfacesAdded match after only PropertiesChanged overflows.
    for _ in 0..3 {
        let events = session.adapter_event_stream(&id).await.unwrap();
        tokio::pin!(events);
        for _ in 0..1024 {
            emit(&bus, false).await;
        }
        // The method reply follows this connection's signals in bus order. Its
        // receipt proves the client's D-Bus driver processed the preceding flood.
        session.get_adapters().await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(2), events.next())
                .await
                .unwrap()
                .is_none()
        );
    }
    // A fresh subscription can receive a real state change after the old one ends.
    let events = session.adapter_event_stream(&id).await.unwrap();
    tokio::pin!(events);
    emit(&bus, false).await;
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), events.next())
            .await
            .unwrap(),
        Some(bluez_async::BluetoothEvent::Adapter {
            event: bluez_async::AdapterEvent::Powered { powered: true },
            ..
        })
    ));
    drop(session);

    rqs_lib::set_bluetooth_adapter(Some("hci1".into()));
    let (wake, _) = tokio::sync::broadcast::channel(16);
    let (status, mut statuses) = tokio::sync::broadcast::channel(64);
    let cancel = CancellationToken::new();
    let worker = tokio::spawn(rqs_lib::hdl::BleListener::supervise(
        wake,
        status,
        cancel.clone(),
    ));
    wait_for(|| radio.active.load(Ordering::SeqCst)).await;
    let starts = radio.starts.load(Ordering::SeqCst);
    radio.hold_stop.store(true, Ordering::SeqCst);
    emit(&bus, true).await;
    wait_for(|| radio.stops.load(Ordering::SeqCst) > 0).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        radio.starts.load(Ordering::SeqCst),
        starts,
        "Must not recover before stop acknowledgement"
    );
    radio.hold_stop.store(false, Ordering::SeqCst);
    radio.wake.notify_waiters();
    wait_for(|| radio.starts.load(Ordering::SeqCst) > starts).await;
    let mut reported_failure = false;
    while let Ok(event) = statuses.try_recv() {
        if let rqs_lib::channel::Message::Backend { component, .. } = event.msg {
            reported_failure |= component == "bluetooth-listener";
        }
    }
    assert!(reported_failure);
    cancel.cancel();
    tokio::time::timeout(Duration::from_secs(3), worker)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(!radio.active.load(Ordering::SeqCst));

    // Foreground selection must also fail and clean up, rather than publish a
    // partial recipient result after a signal was lost.
    let adapter = rqs_lib::bluetooth_adapter().await.unwrap();
    let starts = radio.starts.load(Ordering::SeqCst);
    let work = tokio::spawn(async move {
        rqs_lib::hdl::scan_target(
            &adapter,
            Duration::from_secs(10),
            *b"TEST",
            CancellationToken::new(),
        )
        .await
    });
    wait_for(|| radio.starts.load(Ordering::SeqCst) > starts).await;
    emit(&bus, true).await;
    let error = tokio::time::timeout(Duration::from_secs(3), work)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("event stream ended"), "{error}");
    assert!(!radio.active.load(Ordering::SeqCst));
    rqs_lib::hdl::wait_for_scans().await.unwrap();
}
