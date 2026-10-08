//! Sender recovery and cooperative single-slot advertising on a private bus.
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
async fn state(
    messages: &mut tokio::sync::broadcast::Receiver<ChannelMessage>,
    wanted: &str,
    ready: bool,
) {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match messages.recv().await.unwrap().msg {
                Message::BluetoothServiceReady { component } if ready && component == wanted => {
                    return;
                }
                Message::Backend { component, .. } if !ready && component == wanted => return,
                _ => {}
            }
        }
    })
    .await
    .expect("Expected Bluetooth state was not reported");
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
#[ignore = "requires isolated BlueZ from run-bluetooth-lifecycle.sh"]
async fn sender_recovers_without_starving_receiver_or_preempting_a_connection() {
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
    let (status, mut messages) = tokio::sync::broadcast::channel(128);
    let send_cancel = CancellationToken::new();
    let sender = tokio::spawn(rqs_lib::hdl::BleAdvertiser::supervise(
        status.clone(),
        send_cancel.clone(),
    ));
    state(&mut messages, "bluetooth-discovery", false).await;
    assert_eq!(advert.registrations.load(Ordering::SeqCst), 0);
    let connection = rqs_lib::hdl::BleScanSuppressor::new();
    power.store(true, Ordering::SeqCst);
    let (visibility, visible) = tokio::sync::watch::channel(rqs_lib::Visibility::Visible);
    let recv_cancel = CancellationToken::new();
    let receiver = tokio::spawn(rqs_lib::hdl::supervise_receiver(
        *b"TEST",
        1,
        "Receiver".into(),
        12345,
        visible,
        status.clone(),
        recv_cancel.clone(),
    ));
    state(&mut messages, "bluetooth-receiver", true).await;
    let before = advert.registrations.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(2400)).await;
    assert_eq!(
        advert.registrations.load(Ordering::SeqCst),
        before,
        "A connection must protect the receiver's advertising turn"
    );
    drop(connection);
    state(&mut messages, "bluetooth-discovery", true).await;
    assert_eq!(
        gatt.registrations.load(Ordering::SeqCst),
        1,
        "Advertising turns must not recreate GATT sessions"
    );
    // External Release must lead to another acknowledged sender registration.
    let (path, owner) = {
        let mut active = advert.active.lock().unwrap();
        let (path, owner) = active
            .iter()
            .next()
            .map(|(p, o)| (p.clone(), o.clone()))
            .unwrap();
        active.remove(&path);
        (path, owner)
    };
    zbus::Proxy::new(
        &original,
        owner.as_str(),
        path.as_str(),
        "org.bluez.LEAdvertisement1",
    )
    .await
    .unwrap()
    .call::<_, _, ()>("Release", &())
    .await
    .unwrap();
    state(&mut messages, "bluetooth-discovery", false).await;
    state(&mut messages, "bluetooth-discovery", true).await;
    let while_sender = advert.registrations.load(Ordering::SeqCst);
    visibility.send(rqs_lib::Visibility::Invisible).unwrap();
    state(&mut messages, "bluetooth-receiver", true).await;
    assert!(
        advert.active.lock().unwrap().is_empty(),
        "A hidden receiver must not register after waiting for a turn"
    );
    assert_eq!(advert.registrations.load(Ordering::SeqCst), while_sender);
    visibility.send(rqs_lib::Visibility::Visible).unwrap();
    state(&mut messages, "bluetooth-receiver", true).await;
    assert_eq!(advert.active.lock().unwrap().len(), 1);
    assert!(advert.registrations.load(Ordering::SeqCst) >= before + 2);
    // Both protocol roles recover when bluetoothd changes; old cleanup stays old.
    original.release_name("org.bluez").await.unwrap();
    let new_advert = Arc::new(bluez::State::default());
    let new_gatt = Arc::new(gatt::State::default());
    let replacement = bus(power.clone(), new_advert.clone(), new_gatt.clone()).await;
    state(&mut messages, "bluetooth-discovery", true).await;
    state(&mut messages, "bluetooth-receiver", true).await;
    assert!(advert.active.lock().unwrap().is_empty());
    assert!(gatt.active.lock().unwrap().is_none());
    assert_eq!(new_advert.active.lock().unwrap().len(), 1);
    send_cancel.cancel();
    recv_cancel.cancel();
    tokio::time::timeout(Duration::from_secs(3), sender)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    tokio::time::timeout(Duration::from_secs(3), receiver)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(new_advert.active.lock().unwrap().is_empty());
    assert!(new_gatt.active.lock().unwrap().is_none());

    // A delayed removal keeps the slot reserved; a terminal timeout poisons it.
    let sender = rqs_lib::hdl::BleAdvertiser::new().await.unwrap();
    let cancel = CancellationToken::new();
    let run_cancel = cancel.clone();
    let task = tokio::spawn(async move { sender.run(run_cancel).await });
    tokio::time::timeout(Duration::from_secs(2), async {
        while new_advert.active.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    new_advert.hold_unregister.store(true, Ordering::SeqCst);
    let before = new_advert.registrations.load(Ordering::SeqCst);
    let next = rqs_lib::hdl::ReceiverAdvertiser::new(*b"TEST", 1, "Receiver", None)
        .await
        .unwrap();
    let (_visible, watched) = tokio::sync::watch::channel(rqs_lib::Visibility::Visible);
    let waiting = tokio::spawn(async move { next.run(watched, CancellationToken::new()).await });
    cancel.cancel();
    let error = tokio::time::timeout(Duration::from_secs(18), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("Unconfirmed cleanup"));
    let error = tokio::time::timeout(Duration::from_secs(2), waiting)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("Unconfirmed cleanup"));
    assert_eq!(new_advert.registrations.load(Ordering::SeqCst), before);
    new_advert.hold_unregister.store(false, Ordering::SeqCst);
    new_advert.wake.notify_waiters();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !new_advert.active.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    drop(replacement);
}
