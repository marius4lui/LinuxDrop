//! Runs only against an explicitly isolated D-Bus mock, never real radios.
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use zbus::zvariant::OwnedValue;

#[path = "support/bluez.rs"]
mod bluez;
use bluez::{Adapter, Advertising, State};

async fn wait_for(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Bluetooth lifecycle did not settle");
}
fn apple() -> bluer::adv::Advertisement {
    bluer::adv::Advertisement {
        advertisement_type: bluer::adv::Type::Broadcast,
        manufacturer_data: [(0x004c, vec![0x05, 0x12, 1, 2, 3])].into(),
        ..Default::default()
    }
}
fn quickshare() -> bluer::adv::Advertisement {
    bluer::adv::Advertisement {
        advertisement_type: bluer::adv::Type::Peripheral,
        discoverable: Some(true),
        service_data: [(
            "0000fef3-0000-1000-8000-00805f9b34fb".parse().unwrap(),
            vec![1, 2, 3],
        )]
        .into(),
        min_interval: Some(Duration::from_millis(100)),
        max_interval: Some(Duration::from_millis(150)),
        ..Default::default()
    }
}

#[tokio::test]
#[ignore = "requires the private mock bus in tests/run-bluetooth-lifecycle.sh"]
async fn advertisements_use_selected_controller_and_confirm_cleanup() {
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
    let second_power = Arc::new(AtomicBool::new(true));
    let bus = zbus::connection::Builder::session()
        .unwrap()
        .name("org.bluez")
        .unwrap()
        .serve_at("/", zbus::fdo::ObjectManager)
        .unwrap()
        .serve_at("/org/bluez/hci0", Adapter(Arc::new(AtomicBool::new(false))))
        .unwrap()
        .serve_at("/org/bluez/hci0", Advertising(first.clone()))
        .unwrap()
        .serve_at("/org/bluez/hci1", Adapter(second_power.clone()))
        .unwrap()
        .serve_at("/org/bluez/hci1", Advertising(second.clone()))
        .unwrap()
        .build()
        .await
        .unwrap();
    assert!(linuxdrop_network::bluetooth_adapter(Some("hci0"))
        .await
        .is_err());
    let adapter = linuxdrop_network::bluetooth_adapter(Some("hci1"))
        .await
        .unwrap();
    assert_eq!(adapter.name(), "hci1");
    assert_eq!(
        linuxdrop_network::bluetooth_adapter(None)
            .await
            .unwrap()
            .name(),
        "hci1"
    );
    let mut advertisement = linuxdrop_network::advertise(&adapter, apple())
        .await
        .unwrap();
    assert_eq!(first.registrations.load(Ordering::SeqCst), 0);
    assert_eq!(second.active.lock().unwrap().len(), 1);
    assert!(linuxdrop_network::advertise(&adapter, quickshare())
        .await
        .unwrap_err()
        .to_string()
        .contains("no free advertisement slots"));
    let (_path, owner) = second
        .active
        .lock()
        .unwrap()
        .iter()
        .next()
        .map(|(k, v)| (k.clone(), v.clone()))
        .unwrap();
    {
        let mut props = second.properties.lock().unwrap();
        let values = &mut props[0];
        assert_eq!(
            String::try_from(values.remove("Type").unwrap()).unwrap(),
            "broadcast"
        );
        let data = HashMap::<u16, OwnedValue>::try_from(values.remove("ManufacturerData").unwrap())
            .unwrap();
        assert_eq!(
            Vec::<u8>::try_from(data[&0x004c].try_clone().unwrap()).unwrap(),
            vec![0x05, 0x12, 1, 2, 3]
        );
        assert!(!values.contains_key("Discoverable"));
    }
    second.hold_unregister.store(true, Ordering::SeqCst);
    let shutdown = tokio::spawn(async move {
        advertisement.unregister().await.unwrap();
        advertisement
    });
    wait_for(|| second.removals.load(Ordering::SeqCst) == 1).await;
    assert!(
        !shutdown.is_finished(),
        "Local drop may not stand in for BlueZ acknowledgement"
    );
    assert_eq!(second.active.lock().unwrap().len(), 1);
    second.hold_unregister.store(false, Ordering::SeqCst);
    second.wake.notify_waiters();
    let mut advertisement = shutdown.await.unwrap();
    advertisement.unregister().await.unwrap();
    assert!(second.active.lock().unwrap().is_empty());
    let dbus = zbus::fdo::DBusProxy::new(&bus).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while dbus
            .name_has_owner(owner.as_str().try_into().unwrap())
            .await
            .unwrap()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("Dedicated advertising owner must disconnect after cleanup");
    assert!(
        adapter.is_powered().await.unwrap(),
        "Advertising cleanup must not disconnect the controller session"
    );

    let mut receiver = linuxdrop_network::advertise(&adapter, quickshare())
        .await
        .unwrap();
    {
        let mut props = second.properties.lock().unwrap();
        let values = props.last_mut().unwrap();
        assert_eq!(
            String::try_from(values.remove("Type").unwrap()).unwrap(),
            "peripheral"
        );
        assert!(bool::try_from(values.remove("Discoverable").unwrap()).unwrap());
        assert_eq!(
            u32::try_from(values.remove("MinInterval").unwrap()).unwrap(),
            100
        );
    }
    second.fail_unregister.store(true, Ordering::SeqCst);
    receiver.unregister().await.unwrap();
    assert!(
        second.removals.load(Ordering::SeqCst) >= 3,
        "Transient unregister failure must retry"
    );
    assert!(second.active.lock().unwrap().is_empty());

    // Cancelling an in-flight registration must clean a late successful reply.
    second.hold_register.store(true, Ordering::SeqCst);
    let before = second.registrations.load(Ordering::SeqCst);
    let registering_adapter = adapter.clone();
    let registering =
        tokio::spawn(
            async move { linuxdrop_network::advertise(&registering_adapter, apple()).await },
        );
    wait_for(|| second.registrations.load(Ordering::SeqCst) > before).await;
    registering.abort();
    let _ = registering.await;
    second.hold_register.store(false, Ordering::SeqCst);
    second.wake.notify_waiters();
    wait_for(|| {
        second.removals.load(Ordering::SeqCst) >= 4 && second.active.lock().unwrap().is_empty()
    })
    .await;
    let final_advertisement = linuxdrop_network::advertise(&adapter, apple())
        .await
        .unwrap();
    drop(final_advertisement);
    wait_for(|| {
        second.removals.load(Ordering::SeqCst) >= 5 && second.active.lock().unwrap().is_empty()
    })
    .await;
    // Only the exact bluetoothd owner may release this registration. Another
    // advertisement taking its slot must not disguise loss of our own handle.
    let mut external = linuxdrop_network::advertise(&adapter, apple())
        .await
        .unwrap();
    let (path, owner) = second
        .active
        .lock()
        .unwrap()
        .iter()
        .next()
        .map(|(path, owner)| (path.clone(), owner.clone()))
        .unwrap();
    let outsider = zbus::connection::Builder::session()
        .unwrap()
        .build()
        .await
        .unwrap();
    let foreign_proxy = zbus::Proxy::new(
        &outsider,
        owner.as_str(),
        path.as_str(),
        "org.bluez.LEAdvertisement1",
    )
    .await
    .unwrap();
    assert!(foreign_proxy
        .call::<_, _, ()>("Release", &())
        .await
        .is_err());
    assert!(
        tokio::time::timeout(Duration::from_millis(30), external.released())
            .await
            .is_err()
    );
    let removals = second.removals.load(Ordering::SeqCst);
    {
        let mut active = second.active.lock().unwrap();
        active.remove(&path);
        active.insert("/foreign/advertisement".into(), ":foreign".into());
    }
    let proxy = zbus::Proxy::new(
        &bus,
        owner.as_str(),
        path.as_str(),
        "org.bluez.LEAdvertisement1",
    )
    .await
    .unwrap();
    proxy.call::<_, _, ()>("Release", &()).await.unwrap();
    tokio::time::timeout(Duration::from_secs(1), external.released())
        .await
        .unwrap();
    external.unregister().await.unwrap();
    assert_eq!(
        second.removals.load(Ordering::SeqCst),
        removals,
        "Release already confirms removal; do not unregister again"
    );
    assert_eq!(
        second.active.lock().unwrap().len(),
        1,
        "Another owner's advertisement must remain intact"
    );
    second.active.lock().unwrap().clear();
    second_power.store(false, Ordering::SeqCst);
    let registrations = second.registrations.load(Ordering::SeqCst);
    assert!(linuxdrop_network::advertise(&adapter, apple())
        .await
        .is_err());
    assert_eq!(second.registrations.load(Ordering::SeqCst), registrations);
    // A forged controller signal is ignored; the real owner's power signal
    // invalidates and cleans exactly this registration.
    second_power.store(true, Ordering::SeqCst);
    let mut powered = linuxdrop_network::advertise(&adapter, apple())
        .await
        .unwrap();
    let powered_change: HashMap<&str, zbus::zvariant::Value<'_>> =
        [("Powered", false.into())].into();
    outsider
        .emit_signal(
            None::<&str>,
            "/org/bluez/hci1",
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            &("org.bluez.Adapter1", &powered_change, Vec::<String>::new()),
        )
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(30), powered.released())
            .await
            .is_err()
    );
    second_power.store(false, Ordering::SeqCst);
    let object = bus
        .object_server()
        .interface::<_, Adapter>("/org/bluez/hci1")
        .await
        .unwrap();
    object
        .get()
        .await
        .powered_changed(object.signal_emitter())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), powered.released())
        .await
        .unwrap();
    powered.unregister().await.unwrap();
    assert!(second.active.lock().unwrap().is_empty());
    drop(object);

    // InterfacesRemoved is independent of Powered/Release; USB disappearance
    // need not send either of those signals.
    second_power.store(true, Ordering::SeqCst);
    let mut removed = linuxdrop_network::advertise(&adapter, apple())
        .await
        .unwrap();
    bus.object_server()
        .remove::<Adapter, _>("/org/bluez/hci1")
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), removed.released())
        .await
        .unwrap();
    removed.unregister().await.unwrap();
    assert!(second.active.lock().unwrap().is_empty());
    bus.object_server()
        .at("/org/bluez/hci1", Adapter(second_power.clone()))
        .await
        .unwrap();

    // Keep the old unique owner alive after replacing the well-known service.
    // Its cleanup must never be sent to the newly started Bluetooth daemon.
    let mut old = linuxdrop_network::advertise(&adapter, apple())
        .await
        .unwrap();
    assert!(bus.release_name("org.bluez").await.unwrap());
    let replacement_state = Arc::new(State::default());
    replacement_state
        .active
        .lock()
        .unwrap()
        .insert("/replacement/advertisement".into(), ":replacement".into());
    let _replacement = zbus::connection::Builder::session()
        .unwrap()
        .name("org.bluez")
        .unwrap()
        .serve_at("/org/bluez/hci1", Advertising(replacement_state.clone()))
        .unwrap()
        .build()
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(1), old.released())
        .await
        .unwrap();
    old.unregister().await.unwrap();
    assert!(second.active.lock().unwrap().is_empty());
    assert_eq!(replacement_state.removals.load(Ordering::SeqCst), 0);
    assert_eq!(replacement_state.active.lock().unwrap().len(), 1);
}
