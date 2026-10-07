//! Test-only BlueZ scan acknowledgements; no real adapter is accessed.
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::Notify;

#[derive(Default)]
pub struct State {
    pub powered: AtomicBool,
    pub active: AtomicBool,
    pub starts: AtomicUsize,
    pub stops: AtomicUsize,
    pub hold_stop: AtomicBool,
    pub fail_start: AtomicBool,
    pub fail_stop: AtomicBool,
    pub wake: Notify,
}
impl State {
    pub fn set_power(&self, powered: bool) {
        self.powered.store(powered, Ordering::SeqCst);
        if !powered {
            self.active.store(false, Ordering::SeqCst);
        }
    }
}
#[derive(Debug, zbus::DBusError)]
#[zbus(prefix = "org.bluez.Error")]
pub enum Error {
    Failed(String),
    NotReady(String),
}
pub struct Scanner(pub Arc<State>);
#[zbus::interface(name = "org.bluez.Adapter1")]
impl Scanner {
    #[zbus(property)]
    fn powered(&self) -> bool {
        self.0.powered.load(Ordering::SeqCst)
    }
    #[zbus(property)]
    fn discovering(&self) -> bool {
        self.0.active.load(Ordering::SeqCst)
    }
    #[zbus(property)]
    fn address(&self) -> &str {
        "00:11:22:33:44:55"
    }
    #[zbus(property)]
    fn address_type(&self) -> &str {
        "public"
    }
    #[zbus(property)]
    fn modalias(&self) -> &str {
        "usb:v1234p5678d0001"
    }
    #[zbus(property)]
    fn name(&self) -> &str {
        "Test Bluetooth controller"
    }
    #[zbus(property)]
    fn alias(&self) -> &str {
        "Test Bluetooth controller"
    }
    fn set_discovery_filter(
        &self,
        _filter: std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
    ) {
    }
    fn start_discovery(&self) -> zbus::fdo::Result<()> {
        self.0.starts.fetch_add(1, Ordering::SeqCst);
        self.0.active.store(true, Ordering::SeqCst);
        if self.0.fail_start.load(Ordering::SeqCst) {
            return Err(zbus::fdo::Error::Failed(
                "Start reply failed after processing".into(),
            ));
        }
        Ok(())
    }
    async fn stop_discovery(&self) -> Result<(), Error> {
        self.0.stops.fetch_add(1, Ordering::SeqCst);
        loop {
            let wake = self.0.wake.notified();
            if !self.0.hold_stop.load(Ordering::SeqCst) {
                break;
            }
            wake.await;
        }
        if self.0.fail_stop.load(Ordering::SeqCst) {
            return Err(Error::Failed("Stop was refused".into()));
        }
        if !self.0.powered.load(Ordering::SeqCst) {
            return Err(Error::NotReady("Controller is off".into()));
        }
        self.0.active.store(false, Ordering::SeqCst);
        Ok(())
    }
}
