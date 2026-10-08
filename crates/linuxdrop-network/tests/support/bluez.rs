//! Runs only against an explicitly isolated D-Bus mock, never real radios.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;
use zbus::Connection;

use zbus::zvariant::{OwnedObjectPath, OwnedValue};

#[derive(Default)]
pub struct State {
    pub active: Mutex<HashMap<String, String>>,
    pub properties: Mutex<Vec<HashMap<String, OwnedValue>>>,
    pub registrations: AtomicUsize,
    pub removals: AtomicUsize,
    pub hold_register: AtomicBool,
    pub hold_unregister: AtomicBool,
    pub fail_unregister: AtomicBool,
    pub wake: Notify,
}
pub struct Adapter(pub Arc<AtomicBool>);
#[zbus::interface(name = "org.bluez.Adapter1")]
impl Adapter {
    #[zbus(property)]
    fn powered(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    #[zbus(property)]
    fn address(&self) -> &str {
        "00:11:22:33:44:55"
    }
}
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
pub enum MockError {
    DoesNotExist(String),
    NotPermitted(String),
    Failed(String),
}
pub struct Advertising(pub Arc<State>);
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
