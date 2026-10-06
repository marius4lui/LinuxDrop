//! Runs only against an explicitly isolated D-Bus mock, never real radios.
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::Notify;
use zbus::{
    zvariant::{OwnedObjectPath, OwnedValue},
    Connection,
};

#[derive(Default)]
struct State {
    active: Mutex<HashMap<String, String>>,
    properties: Mutex<Vec<HashMap<String, OwnedValue>>>,
    registrations: AtomicUsize,
    removals: AtomicUsize,
    hold_register: AtomicBool,
    hold_unregister: AtomicBool,
    fail_unregister: AtomicBool,
    wake: Notify,
}
struct Adapter(bool);
#[zbus::interface(name = "org.bluez.Adapter1")]
impl Adapter {
    #[zbus(property)]
    fn powered(&self) -> bool {
        self.0
    }
    #[zbus(property)]
    fn address(&self) -> &str {
        "00:11:22:33:44:55"
    }
}
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
enum MockError {
    DoesNotExist(String),
    NotPermitted(String),
    Failed(String),
}
struct Advertising(Arc<State>);
#[zbus::interface(name = "org.bluez.LEAdvertisingManager1")]
impl Advertising {
    #[zbus(property)]
    fn supported_instances(&self) -> u8 {
        1
    }
    #[zbus(property)]
    fn active_instances(&self) -> u8 {
        self.0.active.lock().unwrap().len() as u8
    }
    async fn register_advertisement(
        &self,
        advertisement: OwnedObjectPath,
        _options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] connection: &Connection,
    ) -> Result<(), MockError> {
        self.0.registrations.fetch_add(1, Ordering::SeqCst);
        loop {
            let wake = self.0.wake.notified();
            if !self.0.hold_register.load(Ordering::SeqCst) {
                break;
            }
            wake.await;
        }
        let sender = header.sender().unwrap().to_string();
        let proxy = zbus::fdo::PropertiesProxy::builder(connection)
            .destination(sender.clone())
            .unwrap()
            .path(advertisement.clone())
            .unwrap()
            .build()
            .await
            .map_err(|error| MockError::Failed(error.to_string()))?;
        let properties = proxy
            .get_all("org.bluez.LEAdvertisement1".try_into().unwrap())
            .await
            .map_err(|error| MockError::Failed(error.to_string()))?;
        let mut active = self.0.active.lock().unwrap();
        if !active.is_empty() {
            return Err(MockError::NotPermitted("No advertising capacity".into()));
        }
        active.insert(advertisement.to_string(), sender);
        self.0.properties.lock().unwrap().push(properties);
        Ok(())
    }
    async fn unregister_advertisement(
        &self,
        advertisement: OwnedObjectPath,
    ) -> Result<(), MockError> {
        self.0.removals.fetch_add(1, Ordering::SeqCst);
        loop {
            let wake = self.0.wake.notified();
            if !self.0.hold_unregister.load(Ordering::SeqCst) {
                break;
            }
            wake.await;
        }
        if self.0.fail_unregister.swap(false, Ordering::SeqCst) {
            return Err(MockError::Failed("Controller busy".into()));
        }
        if self
            .0
            .active
            .lock()
            .unwrap()
            .remove(advertisement.as_str())
            .is_none()
        {
            return Err(MockError::DoesNotExist("Already removed".into()));
        }
        Ok(())
    }
}
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
    let bus = zbus::connection::Builder::session()
        .unwrap()
        .name("org.bluez")
        .unwrap()
        .serve_at("/", zbus::fdo::ObjectManager)
        .unwrap()
        .serve_at("/org/bluez/hci0", Adapter(false))
        .unwrap()
        .serve_at("/org/bluez/hci0", Advertising(first.clone()))
        .unwrap()
        .serve_at("/org/bluez/hci1", Adapter(true))
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
}
