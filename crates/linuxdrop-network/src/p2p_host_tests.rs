//! Private-bus supplicant contract; no real radio or system service is used.
use super::*;
use std::sync::{Arc, Mutex};
const ROOT: &str = "/fi/w1/wpa_supplicant1";
const PARENT: &str = "/fi/w1/wpa_supplicant1/Interfaces/0";
const VIF: &str = "/fi/w1/wpa_supplicant1/Interfaces/1";
const GROUP: &str = "/fi/w1/wpa_supplicant1/Interfaces/1/Groups/0";
#[derive(Default)]
struct State {
    disconnected: usize,
    cancelled: usize,
    delay: bool,
    existing: bool,
    wrong_channel: bool,
}
type Shared = Arc<Mutex<State>>;
fn path(value: &str) -> OwnedObjectPath {
    value.try_into().unwrap()
}
struct Root(Shared);
#[zbus::interface(name = "fi.w1.wpa_supplicant1")]
impl Root {
    fn get_interface(&self, name: &str) -> OwnedObjectPath {
        assert_eq!(name, "testwifi0");
        path(PARENT)
    }
    #[zbus(property)]
    fn interfaces(&self) -> Vec<OwnedObjectPath> {
        let mut paths = vec![path(PARENT)];
        if self.0.lock().unwrap().existing {
            paths.push(path(VIF));
        }
        paths
    }
}
struct Interface;
#[zbus::interface(name = "fi.w1.wpa_supplicant1.Interface")]
impl Interface {
    #[zbus(property)]
    fn ifname(&self) -> &str {
        "p2ptest0"
    }
}
struct Details(Shared);
#[zbus::interface(name = "fi.w1.wpa_supplicant1.Group")]
impl Details {
    #[zbus(property)]
    fn role(&self) -> &str {
        "GO"
    }
    #[zbus(property, name = "SSID")]
    fn ssid(&self) -> Vec<u8> {
        b"DIRECT-test-LinuxDrop".to_vec()
    }
    #[zbus(property)]
    fn passphrase(&self) -> &str {
        "fixture-password"
    }
    #[zbus(property)]
    fn frequency(&self) -> u16 {
        if self.0.lock().unwrap().wrong_channel {
            2437
        } else {
            5180
        }
    }
}
struct Device {
    state: Shared,
    parent: bool,
    added: Arc<tokio::sync::Notify>,
}
async fn started(bus: &Connection) {
    let properties: HashMap<&str, Value<'_>> = [
        ("role", Value::from("GO")),
        ("interface_object", Value::from(path(VIF))),
        ("group_object", Value::from(path(GROUP))),
    ]
    .into_iter()
    .collect();
    bus.emit_signal(None::<&str>, PARENT, DEVICE, "GroupStarted", &(properties,))
        .await
        .unwrap();
}
#[zbus::interface(name = "fi.w1.wpa_supplicant1.Interface.P2PDevice")]
impl Device {
    #[zbus(property)]
    fn group(&self) -> OwnedObjectPath {
        path(if self.parent { "/" } else { GROUP })
    }
    async fn group_add(
        &self,
        args: HashMap<String, OwnedValue>,
        #[zbus(connection)] bus: &Connection,
    ) {
        assert!(self.parent);
        assert!(!bool::try_from(&args["persistent"]).unwrap());
        assert_eq!(i32::try_from(&args["frequency"]).unwrap(), 5180);
        self.added.notify_one();
        let delay = self.state.lock().unwrap().delay;
        if !delay {
            started(bus).await;
        }
    }
    async fn cancel(&self, #[zbus(connection)] bus: &Connection) {
        let delay = {
            let mut state = self.state.lock().unwrap();
            state.cancelled += 1;
            state.delay
        };
        if delay {
            started(bus).await;
        }
    }
    fn disconnect(&self) {
        assert!(!self.parent);
        self.state.lock().unwrap().disconnected += 1;
    }
}
fn verify(identity: &GroupIdentity) -> Result<()> {
    anyhow::ensure!(
        identity.interface == "p2ptest0" && identity.parent_interface == "testwifi0",
        "foreign group"
    );
    Ok(())
}
async fn wait_disconnect(state: &Shared, count: usize) {
    tokio::time::timeout(Duration::from_secs(3), async {
        while state.lock().unwrap().disconnected < count {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("group cleanup did not complete");
}
#[tokio::test]
#[ignore = "requires an explicitly isolated dbus-run-session"]
async fn autonomous_group_credentials_cleanup_and_cancellation_race() {
    assert_eq!(
        std::env::var("LINUXDROP_TEST_PRIVATE_P2P").as_deref(),
        Ok("1")
    );
    assert_eq!(
        std::env::var("DBUS_SYSTEM_BUS_ADDRESS"),
        std::env::var("DBUS_SESSION_BUS_ADDRESS")
    );
    let state = Shared::default();
    let added = Arc::new(tokio::sync::Notify::new());
    let _service = zbus::connection::Builder::session()
        .unwrap()
        .name(SERVICE)
        .unwrap()
        .serve_at(ROOT, Root(state.clone()))
        .unwrap()
        .serve_at(
            PARENT,
            Device {
                state: state.clone(),
                parent: true,
                added: added.clone(),
            },
        )
        .unwrap()
        .serve_at(
            VIF,
            Device {
                state: state.clone(),
                parent: false,
                added: added.clone(),
            },
        )
        .unwrap()
        .serve_at(VIF, Interface)
        .unwrap()
        .serve_at(GROUP, Details(state.clone()))
        .unwrap()
        .build()
        .await
        .unwrap();
    let connection = Connection::session().await.unwrap();
    let hosted = create_group_inner(
        connection.clone(),
        "testwifi0",
        5180,
        CancellationToken::new(),
        verify,
    )
    .await
    .unwrap();
    assert_eq!(hosted.ssid, "DIRECT-test-LinuxDrop");
    assert_eq!(hosted.password, "fixture-password");
    assert_eq!(hosted.frequency, 5180);
    assert!(!format!("{hosted:?}").contains("fixture-password"));
    drop(hosted);
    wait_disconnect(&state, 1).await;
    state.lock().unwrap().wrong_channel = true;
    assert!(create_group_inner(
        connection.clone(),
        "testwifi0",
        5180,
        CancellationToken::new(),
        verify
    )
    .await
    .is_err());
    wait_disconnect(&state, 2).await;
    {
        let mut state = state.lock().unwrap();
        state.wrong_channel = false;
        state.existing = true;
    }
    assert!(create_group_inner(
        connection.clone(),
        "testwifi0",
        5180,
        CancellationToken::new(),
        verify
    )
    .await
    .is_err());
    assert_eq!(
        state.lock().unwrap().disconnected,
        2,
        "must not disconnect a pre-existing interface"
    );
    {
        let mut state = state.lock().unwrap();
        state.existing = false;
        state.delay = true;
    }
    // Clear any earlier Notify permit before waiting for this specific GroupAdd.
    let _ = tokio::time::timeout(Duration::from_millis(1), added.notified()).await;
    let cancel = CancellationToken::new();
    let worker_cancel = cancel.clone();
    let worker = tokio::spawn(async move {
        create_group_inner(connection, "testwifi0", 5180, worker_cancel, verify).await
    });
    tokio::time::timeout(Duration::from_secs(3), added.notified())
        .await
        .unwrap();
    cancel.cancel();
    assert!(worker.await.unwrap().is_err());
    wait_disconnect(&state, 3).await;
    assert_eq!(state.lock().unwrap().cancelled, 3);
}
