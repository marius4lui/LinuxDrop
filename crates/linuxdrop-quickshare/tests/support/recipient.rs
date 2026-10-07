use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use zbus::zvariant::OwnedValue;
pub struct Recipient {
    pub address: String,
    pub data: Arc<Mutex<Vec<u8>>>,
}
#[zbus::interface(name = "org.bluez.Device1")]
impl Recipient {
    #[zbus(property)]
    fn address(&self) -> &str {
        &self.address
    }
    #[zbus(property)]
    fn address_type(&self) -> &str {
        "random"
    }
    #[zbus(property)]
    fn name(&self) -> &str {
        "Same phone"
    }
    #[zbus(property)]
    fn alias(&self) -> &str {
        "Same phone"
    }
    #[zbus(property)]
    fn paired(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn connected(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn services_resolved(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn trusted(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn blocked(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn legacy_pairing(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn rssi(&self) -> i16 {
        -40
    }
    #[zbus(property)]
    fn service_data(&self) -> HashMap<String, OwnedValue> {
        HashMap::from([(
            "0000fef3-0000-1000-8000-00805f9b34fb".into(),
            zbus::zvariant::Value::from(self.data.lock().unwrap().clone())
                .try_into()
                .unwrap(),
        )])
    }
}
