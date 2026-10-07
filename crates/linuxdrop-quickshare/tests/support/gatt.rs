//! Minimal GATT manager on the private bus; actual service callbacks run in BlueR.
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::Notify;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};
#[derive(Default)]
pub struct State {
    pub active: Mutex<Option<(String, String)>>,
    pub hold_register: AtomicBool,
    pub fail_register: AtomicBool,
    pub hold_unregister: AtomicBool,
    pub removals: AtomicUsize,
    pub wake: Notify,
}
pub struct Gatt(pub Arc<State>);
#[zbus::interface(name = "org.bluez.GattManager1")]
impl Gatt {
    async fn register_application(
        &self,
        path: OwnedObjectPath,
        _options: HashMap<String, OwnedValue>,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> zbus::fdo::Result<()> {
        *self.0.active.lock().unwrap() =
            Some((path.to_string(), header.sender().unwrap().to_string()));
        loop {
            let wake = self.0.wake.notified();
            if !self.0.hold_register.load(Ordering::SeqCst) {
                break;
            }
            wake.await;
        }
        if self.0.fail_register.load(Ordering::SeqCst) {
            return Err(zbus::fdo::Error::Failed(
                "registration reply failed after processing".into(),
            ));
        }
        Ok(())
    }
    async fn unregister_application(&self, path: OwnedObjectPath) {
        self.0.removals.fetch_add(1, Ordering::SeqCst);
        loop {
            let wake = self.0.wake.notified();
            if !self.0.hold_unregister.load(Ordering::SeqCst) {
                break;
            }
            wake.await;
        }
        assert_eq!(
            self.0.active.lock().unwrap().as_ref().unwrap().0,
            path.as_str()
        );
        *self.0.active.lock().unwrap() = None;
    }
}
