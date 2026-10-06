//! Real Quick Share advertisement actors against the isolated BlueZ mock.
#[path = "../../linuxdrop-network/tests/support/bluez.rs"]
mod bluez;
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
    tokio::time::timeout(Duration::from_secs(2), async {
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
    assert!(error.to_string().contains("removed by BlueZ"));
    assert!(second.active.lock().unwrap().is_empty());
    assert_eq!(first.registrations.load(Ordering::SeqCst), 0);
}
