//! Run on a dedicated dbus-run-session bus, with DBUS_SYSTEM_BUS_ADDRESS set
//! to that session address. Never installs or talks to a real NetworkManager.
use linuxdrop_network::nm::{self, Lease};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Semaphore;
use zbus::{
    zvariant::{OwnedObjectPath, OwnedValue},
    Connection,
};

const ROOT: &str = "/org/freedesktop/NetworkManager";
const DEVICE: &str = "/org/freedesktop/NetworkManager/Devices/1";
const PROFILE: &str = "/org/freedesktop/NetworkManager/Settings/1";
const ACTIVE: &str = "/org/freedesktop/NetworkManager/ActiveConnection/1";
const IP: &str = "/org/freedesktop/NetworkManager/IP4Config/1";
const AP: &str = "/org/freedesktop/NetworkManager/AccessPoint/1";
type Settings = HashMap<String, HashMap<String, OwnedValue>>;
type Shared = Arc<Mutex<State>>;
#[derive(Default)]
struct State {
    settings: Option<Settings>,
    active: bool,
    wait_for_address: bool,
    ipv6_only: bool,
    device_state: u32,
    activations: u32,
    deactivations: u32,
    deletions: u32,
    race_deactivation: bool,
}
fn path(value: &str) -> OwnedObjectPath {
    value.try_into().unwrap()
}
fn string(value: &str) -> OwnedValue {
    zbus::zvariant::Str::from(value).into()
}

struct Manager(Shared);
#[zbus::interface(name = "org.freedesktop.NetworkManager")]
impl Manager {
    fn get_device_by_ip_iface(&self, name: &str) -> zbus::fdo::Result<OwnedObjectPath> {
        if name != "testwifi0" {
            return Err(zbus::fdo::Error::Failed("unexpected interface".into()));
        }
        Ok(path(DEVICE))
    }
    async fn add_and_activate_connection2(
        &self,
        settings: Settings,
        device: OwnedObjectPath,
        specific: OwnedObjectPath,
        options: HashMap<String, OwnedValue>,
        #[zbus(connection)] bus: &Connection,
    ) -> zbus::fdo::Result<(
        OwnedObjectPath,
        OwnedObjectPath,
        HashMap<String, OwnedValue>,
    )> {
        assert_eq!(device.as_str(), DEVICE);
        assert_eq!(specific.as_str(), "/");
        assert_eq!(<&str>::try_from(&options["persist"]).unwrap(), "volatile");
        assert_eq!(
            <&str>::try_from(&options["bind-activation"]).unwrap(),
            "dbus-client"
        );
        assert!(!bool::try_from(&settings["connection"]["autoconnect"]).unwrap());
        assert!(bool::try_from(&settings["ipv4"]["never-default"]).unwrap());
        assert!(bool::try_from(&settings["ipv4"]["ignore-auto-dns"]).unwrap());
        let state_value = {
            let mut state = self.0.lock().unwrap();
            assert!(state.settings.is_none());
            state.settings = Some(settings);
            state.active = true;
            state.activations += 1;
            state.device_state = if state.wait_for_address { 70 } else { 100 };
            state.device_state
        };
        bus.emit_signal(
            None::<&str>,
            DEVICE,
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
            &(
                "org.freedesktop.NetworkManager.Device",
                HashMap::from([("State", OwnedValue::from(state_value))]),
                Vec::<String>::new(),
            ),
        )
        .await
        .unwrap();
        Ok((path(PROFILE), path(ACTIVE), HashMap::new()))
    }
    fn deactivate_connection(&self, active: OwnedObjectPath) -> zbus::fdo::Result<()> {
        assert_eq!(active.as_str(), ACTIVE);
        let mut state = self.0.lock().unwrap();
        assert!(state.active);
        state.active = false;
        state.settings = None; // Volatile NM profiles disappear on deactivation.
        state.device_state = 30;
        state.deactivations += 1;
        if std::mem::take(&mut state.race_deactivation) {
            return Err(zbus::fdo::Error::Failed(
                "connection already deactivated by another cleanup owner".into(),
            ));
        }
        Ok(())
    }
    #[zbus(property)]
    fn active_connections(&self) -> Vec<OwnedObjectPath> {
        if self.0.lock().unwrap().active {
            vec![path(ACTIVE)]
        } else {
            vec![]
        }
    }
}
struct Profiles(Shared);
#[zbus::interface(name = "org.freedesktop.NetworkManager.Settings")]
impl Profiles {
    fn list_connections(&self) -> Vec<OwnedObjectPath> {
        if self.0.lock().unwrap().settings.is_some() {
            vec![path(PROFILE)]
        } else {
            vec![]
        }
    }
}
struct Profile(Shared);
#[zbus::interface(name = "org.freedesktop.NetworkManager.Settings.Connection")]
impl Profile {
    fn get_settings(&self) -> Settings {
        self.0
            .lock()
            .unwrap()
            .settings
            .as_ref()
            .unwrap()
            .iter()
            .map(|(group, values)| {
                (
                    group.clone(),
                    values
                        .iter()
                        .map(|(key, value)| (key.clone(), value.try_clone().unwrap()))
                        .collect(),
                )
            })
            .collect()
    }
    fn delete(&self) {
        let mut state = self.0.lock().unwrap();
        assert!(!state.active);
        assert!(state.settings.take().is_some());
        state.deletions += 1;
    }
}
struct Device(Shared);
#[zbus::interface(name = "org.freedesktop.NetworkManager.Device")]
impl Device {
    #[zbus(property)]
    fn state(&self) -> u32 {
        self.0.lock().unwrap().device_state
    }
    #[zbus(property)]
    fn ip4_config(&self) -> OwnedObjectPath {
        if self.0.lock().unwrap().ipv6_only {
            path("/")
        } else {
            path(IP)
        }
    }
    #[zbus(property)]
    fn ip6_config(&self) -> OwnedObjectPath {
        path("/org/freedesktop/NetworkManager/IP6Config/1")
    }
}
struct Wireless;
#[zbus::interface(name = "org.freedesktop.NetworkManager.Device.Wireless")]
impl Wireless {
    #[zbus(property)]
    fn active_access_point(&self) -> OwnedObjectPath {
        path(AP)
    }
}
struct Ip;
#[zbus::interface(name = "org.freedesktop.NetworkManager.IP4Config")]
impl Ip {
    #[zbus(property)]
    fn address_data(&self) -> Vec<HashMap<String, OwnedValue>> {
        vec![HashMap::from([("address".into(), string("192.168.88.2"))])]
    }
}
struct Ip6;
#[zbus::interface(name = "org.freedesktop.NetworkManager.IP6Config")]
impl Ip6 {
    #[zbus(property)]
    fn address_data(&self) -> Vec<HashMap<String, OwnedValue>> {
        vec![HashMap::from([("address".into(), string("fe80::1234"))])]
    }
}
struct AccessPoint;
#[zbus::interface(name = "org.freedesktop.NetworkManager.AccessPoint")]
impl AccessPoint {
    #[zbus(property)]
    fn frequency(&self) -> u32 {
        2437
    }
}
struct Active(Shared);
#[zbus::interface(name = "org.freedesktop.NetworkManager.Connection.Active")]
impl Active {
    #[zbus(property)]
    fn uuid(&self) -> String {
        <&str>::try_from(&self.0.lock().unwrap().settings.as_ref().unwrap()["connection"]["uuid"])
            .unwrap()
            .to_owned()
    }
}

fn lease() -> Lease {
    Lease {
        interface: "testwifi0".into(),
        lease_id: "a".repeat(32),
        connection_uuid: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into(),
    }
}
async fn wait_for(predicate: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("network lifecycle did not settle");
}

#[tokio::test]
#[ignore = "requires an explicitly isolated mock system bus; see tests/run-nm-lifecycle.sh"]
async fn volatile_profiles_are_owned_serialized_and_removed_on_cancel() {
    assert_eq!(
        std::env::var("LINUXDROP_TEST_PRIVATE_NM").as_deref(),
        Ok("1")
    );
    assert_eq!(
        std::env::var("DBUS_SYSTEM_BUS_ADDRESS").unwrap(),
        std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap()
    );
    let state = Arc::new(Mutex::new(State {
        device_state: 30,
        ..State::default()
    }));
    let _server = zbus::connection::Builder::session()
        .unwrap()
        .name("org.freedesktop.NetworkManager")
        .unwrap()
        .serve_at(ROOT, Manager(state.clone()))
        .unwrap()
        .serve_at(format!("{ROOT}/Settings"), Profiles(state.clone()))
        .unwrap()
        .serve_at(PROFILE, Profile(state.clone()))
        .unwrap()
        .serve_at(DEVICE, Device(state.clone()))
        .unwrap()
        .serve_at(DEVICE, Wireless)
        .unwrap()
        .serve_at(IP, Ip)
        .unwrap()
        .serve_at("/org/freedesktop/NetworkManager/IP6Config/1", Ip6)
        .unwrap()
        .serve_at(AP, AccessPoint)
        .unwrap()
        .serve_at(ACTIVE, Active(state.clone()))
        .unwrap()
        .build()
        .await
        .unwrap();
    let semaphore = Arc::new(Semaphore::new(1));
    let guard = nm::connect(
        lease(),
        "test-network".into(),
        "test-password".into(),
        false,
        semaphore.clone(),
    )
    .await
    .unwrap();
    assert_eq!(guard.address.unwrap().to_string(), "192.168.88.2");
    assert_eq!(guard.frequency, 2437);
    assert!(nm::connect(
        lease(),
        "other".into(),
        "other-password".into(),
        true,
        semaphore.clone()
    )
    .await
    .is_err());
    assert_eq!(state.lock().unwrap().activations, 1);
    // netd and the protocol guard can both observe the same volatile profile.
    state.lock().unwrap().race_deactivation = true;
    nm::remove_owned(&lease()).await.unwrap();
    assert_eq!(semaphore.available_permits(), 0);
    drop(guard);
    wait_for(|| semaphore.available_permits() == 1).await;
    assert_eq!(state.lock().unwrap().deactivations, 1);
    assert_eq!(state.lock().unwrap().deletions, 0); // NM already deleted volatile profile.

    // Cancellation during DHCP/address acquisition must release both profile and radio.
    state.lock().unwrap().wait_for_address = true;
    let pending = tokio::spawn(nm::connect(
        lease(),
        "test-network".into(),
        "test-password".into(),
        false,
        semaphore.clone(),
    ));
    wait_for(|| state.lock().unwrap().activations == 2).await;
    pending.abort();
    let _ = pending.await;
    wait_for(|| semaphore.available_permits() == 1).await;
    assert!(!state.lock().unwrap().active);
    assert_eq!(state.lock().unwrap().deactivations, 2);

    // A pre-existing active network must never reach the mutation API.
    state.lock().unwrap().device_state = 100;
    assert!(nm::connect(
        lease(),
        "test-network".into(),
        "test-password".into(),
        false,
        semaphore.clone()
    )
    .await
    .is_err());
    assert_eq!(state.lock().unwrap().activations, 2);
    state.lock().unwrap().device_state = 30;

    // Even a matching UUID cannot authorize deletion of a different owner's profile.
    state.lock().unwrap().settings = Some(HashMap::from([(
        "connection".into(),
        HashMap::from([
            ("uuid".into(), string(&lease().connection_uuid)),
            ("id".into(), string("family-network")),
            ("interface-name".into(), string("testwifi0")),
        ]),
    )]));
    assert!(nm::remove_owned(&lease()).await.is_err());
    assert!(nm::connect(
        lease(),
        "test-network".into(),
        "test-password".into(),
        false,
        semaphore.clone()
    )
    .await
    .is_err());
    assert_eq!(state.lock().unwrap().activations, 2);
    assert_eq!(state.lock().unwrap().deletions, 0);
    assert!(state.lock().unwrap().settings.is_some());

    // A joined network can activate with IPv6 only. It still cannot replace
    // host routes or DNS, and the exclusive lease lasts through cleanup.
    {
        let mut state = state.lock().unwrap();
        state.settings = None;
        state.device_state = 30;
        state.wait_for_address = false;
        state.ipv6_only = true;
    }
    let guard = nm::connect(
        lease(),
        "ipv6-network".into(),
        "test-password".into(),
        false,
        semaphore.clone(),
    )
    .await
    .unwrap();
    assert!(guard.address.is_none());
    {
        let state = state.lock().unwrap();
        let settings = state.settings.as_ref().unwrap();
        assert_eq!(
            <&str>::try_from(&settings["ipv6"]["method"]).unwrap(),
            "auto"
        );
        for key in ["never-default", "ignore-auto-dns", "ignore-auto-routes"] {
            assert!(bool::try_from(&settings["ipv6"][key]).unwrap());
        }
        assert!(bool::try_from(&settings["ipv4"]["may-fail"]).unwrap());
    }
    assert_eq!(semaphore.available_permits(), 0);
    drop(guard);
    wait_for(|| semaphore.available_permits() == 1).await;
    assert!(!state.lock().unwrap().active);
    let guard = nm::connect_with_ipv6(
        lease(),
        "ipv6-link-local".into(),
        "test-password".into(),
        false,
        semaphore.clone(),
        true,
    )
    .await
    .unwrap();
    assert!(guard.address.is_none());
    assert_eq!(
        <&str>::try_from(&state.lock().unwrap().settings.as_ref().unwrap()["ipv6"]["method"])
            .unwrap(),
        "link-local"
    );
    drop(guard);
    wait_for(|| semaphore.available_permits() == 1).await;
    assert!(!state.lock().unwrap().active);
}
