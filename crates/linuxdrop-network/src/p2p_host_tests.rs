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
    found: usize,
    stopped_find: usize,
    cancelled: usize,
    delay: bool,
    existing: bool,
    wrong_channel: bool,
    wps_started: usize,
    wps_reject: bool,
    name_changed: bool,
    name_empty: bool,
    disconnect_gate: Option<Arc<tokio::sync::Semaphore>>,
    disconnect_started: Arc<tokio::sync::Notify>,
    disconnect_reject: bool,
    group_present: bool,
    retain_after_disconnect: bool,
    malformed_started: bool,
    cancel_reject: bool,
    cancel_silent: bool,
    parent_group: bool,
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
        let state = self.0.lock().unwrap();
        if state.existing || state.group_present {
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
struct Wps(Shared);
#[zbus::interface(name = "fi.w1.wpa_supplicant1.Interface.WPS")]
impl Wps {
    #[zbus(property)]
    fn device_name(&self) -> &str {
        if self.0.lock().unwrap().name_changed {
            "Changed"
        } else {
            "Existing LinuxDrop radio"
        }
    }
    fn start(
        &self,
        args: HashMap<String, OwnedValue>,
    ) -> zbus::fdo::Result<HashMap<String, OwnedValue>> {
        assert_eq!(args.len(), 2);
        assert_eq!(<&str>::try_from(&args["Role"]).unwrap(), "registrar");
        assert_eq!(<&str>::try_from(&args["Type"]).unwrap(), "pbc");
        let mut state = self.0.lock().unwrap();
        state.wps_started += 1;
        if state.wps_reject {
            return Err(zbus::fdo::Error::Failed("PBC rejected".into()));
        }
        Ok(HashMap::new())
    }
}
struct Device {
    state: Shared,
    parent: bool,
    added: Arc<tokio::sync::Notify>,
}
async fn started(bus: &Connection) {
    emit_started(bus, false).await;
}
async fn emit_started(bus: &Connection, malformed: bool) {
    let mut properties: HashMap<&str, Value<'_>> = [
        ("role", Value::from("GO")),
        ("interface_object", Value::from(path(VIF))),
        ("group_object", Value::from(path(GROUP))),
    ]
    .into_iter()
    .collect();
    if malformed {
        properties.remove("interface_object");
    }
    bus.emit_signal(None::<&str>, PARENT, DEVICE, "GroupStarted", &(properties,))
        .await
        .unwrap();
}
#[zbus::interface(name = "fi.w1.wpa_supplicant1.Interface.P2PDevice")]
impl Device {
    #[zbus(property, name = "P2PDeviceConfig")]
    fn config(&self) -> HashMap<String, Value<'_>> {
        let name = if self.state.lock().unwrap().name_empty {
            ""
        } else {
            "Existing LinuxDrop radio"
        };
        [("DeviceName".into(), Value::from(name))]
            .into_iter()
            .collect()
    }
    #[zbus(property)]
    fn group(&self) -> OwnedObjectPath {
        path(if self.parent && !self.state.lock().unwrap().parent_group {
            "/"
        } else {
            GROUP
        })
    }
    #[zbus(property)]
    fn peers(&self) -> Vec<OwnedObjectPath> {
        vec![]
    }
    fn find(&self, args: HashMap<String, OwnedValue>) {
        assert_eq!(i32::try_from(&args["Timeout"]).unwrap(), 20);
        self.state.lock().unwrap().found += 1;
        self.added.notify_one();
    }
    fn stop_find(&self) {
        self.state.lock().unwrap().stopped_find += 1;
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
            self.state.lock().unwrap().group_present = true;
            let malformed = self.state.lock().unwrap().malformed_started;
            emit_started(bus, malformed).await;
        }
    }
    async fn cancel(&self, #[zbus(connection)] bus: &Connection) -> zbus::fdo::Result<()> {
        let (delay, malformed, silent, reject) = {
            let mut state = self.state.lock().unwrap();
            state.cancelled += 1;
            (
                state.delay,
                state.malformed_started,
                state.cancel_silent,
                state.cancel_reject,
            )
        };
        if reject {
            return Err(zbus::fdo::Error::Failed(
                "fixture cancellation rejected".into(),
            ));
        }
        if delay && !silent {
            self.state.lock().unwrap().group_present = true;
            emit_started(bus, malformed).await;
        }
        Ok(())
    }
    async fn disconnect(&self) -> zbus::fdo::Result<()> {
        assert!(!self.parent);
        let (gate, started) = {
            let state = self.state.lock().unwrap();
            (
                state.disconnect_gate.clone(),
                state.disconnect_started.clone(),
            )
        };
        started.notify_one();
        if let Some(gate) = gate {
            gate.acquire_owned().await.unwrap().forget();
        }
        let mut state = self.state.lock().unwrap();
        if state.disconnect_reject {
            return Err(zbus::fdo::Error::Failed(
                "fixture disconnect rejected".into(),
            ));
        }
        state.disconnected += 1;
        if !state.retain_after_disconnect {
            state.group_present = false;
        }
        Ok(())
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
async fn serve(state: Shared, added: Arc<tokio::sync::Notify>) -> Connection {
    zbus::connection::Builder::session()
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
        .serve_at(VIF, Wps(state.clone()))
        .unwrap()
        .serve_at(VIF, Interface)
        .unwrap()
        .serve_at(GROUP, Details(state.clone()))
        .unwrap()
        .build()
        .await
        .unwrap()
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
    let service = serve(state.clone(), added.clone()).await;
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
        state.group_present = false;
        state.delay = true;
    }
    // Clear any earlier Notify permit before waiting for this specific GroupAdd.
    let _ = tokio::time::timeout(Duration::from_millis(1), added.notified()).await;
    let cancel = CancellationToken::new();
    let worker_cancel = cancel.clone();
    let worker = tokio::spawn(async move {
        create_group_inner(connection.clone(), "testwifi0", 5180, worker_cancel, verify).await
    });
    tokio::time::timeout(Duration::from_secs(3), added.notified())
        .await
        .unwrap();
    cancel.cancel();
    assert!(worker.await.unwrap().is_err());
    wait_disconnect(&state, 3).await;
    assert_eq!(state.lock().unwrap().cancelled, 2);
    let connection = Connection::session().await.unwrap();
    state.lock().unwrap().delay = false;
    let hosted = create_group_inner(
        connection.clone(),
        "testwifi0",
        5180,
        CancellationToken::new(),
        verify,
    )
    .await
    .unwrap();
    let identity = hosted.group.into_journaled();
    assert_eq!(
        identity.service_owner,
        service.unique_name().unwrap().as_str()
    );
    assert_eq!(identity.bus_guid, connection.server_guid().as_str());
    let serialized = serde_json::to_value(&identity).unwrap();
    let restored: GroupIdentity = serde_json::from_value(serialized.clone()).unwrap();
    assert!(service_matches(&connection, &restored).await);
    service.release_name(SERVICE).await.unwrap();
    let replacement_state = Shared::default();
    let replacement_added = Arc::new(tokio::sync::Notify::new());
    let replacement = serve(replacement_state.clone(), replacement_added.clone()).await;
    assert!(!service_matches(&connection, &restored).await);
    // Both owners export identical paths. Cleanup must target the old connection.
    disconnect_checked(&connection, &restored, verify)
        .await
        .unwrap();
    assert_eq!(state.lock().unwrap().disconnected, 4);
    assert_eq!(replacement_state.lock().unwrap().disconnected, 0);
    let mut legacy = serialized;
    legacy.as_object_mut().unwrap().remove("service_owner");
    legacy.as_object_mut().unwrap().remove("bus_guid");
    let legacy: GroupIdentity = serde_json::from_value(legacy).unwrap();
    assert!(disconnect_checked(&connection, &legacy, verify)
        .await
        .is_err());
    let mut other_bus = restored.clone();
    other_bus.bus_guid = "different-bus".into();
    assert!(disconnect_checked(&connection, &other_bus, verify)
        .await
        .is_err());
    assert_eq!(replacement_state.lock().unwrap().disconnected, 0);

    replacement_state.lock().unwrap().delay = true;
    let client = connection.clone();
    let worker = tokio::spawn(async move {
        create_group_inner(client, "testwifi0", 5180, CancellationToken::new(), verify).await
    });
    tokio::time::timeout(Duration::from_secs(3), replacement_added.notified())
        .await
        .unwrap();
    replacement.release_name(SERVICE).await.unwrap();
    service.request_name(SERVICE).await.unwrap();
    started(&service).await; // Identical foreign GroupStarted cannot complete the operation.
    let error = tokio::time::timeout(Duration::from_secs(3), worker)
        .await
        .unwrap()
        .unwrap()
        .expect_err("owner replacement must abort host");
    assert!(error.to_string().contains("owner changed"), "{error:#}");
    assert_eq!(
        state.lock().unwrap().cancelled,
        2,
        "must not cancel the replacement instance"
    );
    assert_eq!(replacement_state.lock().unwrap().cancelled, 1);
    wait_disconnect(&replacement_state, 1).await;

    // The client role must also abort while discovering, rather than waiting
    // its full peer-discovery timeout or stopping discovery on the new owner.
    let _ = tokio::time::timeout(Duration::from_millis(1), added.notified()).await;
    let client = connection.clone();
    let worker = tokio::spawn(async move {
        connect_on(
            client,
            "testwifi0".into(),
            "Peer".into(),
            String::new(),
            2437,
            CancellationToken::new(),
            FormationChecks {
                verify,
                prepared: None,
            },
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(3), added.notified())
        .await
        .unwrap();
    service.release_name(SERVICE).await.unwrap();
    replacement.request_name(SERVICE).await.unwrap();
    let error = tokio::time::timeout(Duration::from_secs(3), worker)
        .await
        .unwrap()
        .unwrap()
        .err()
        .expect("owner replacement must abort client");
    assert!(error.to_string().contains("owner changed"), "{error:#}");
    assert_eq!(state.lock().unwrap().found, 1);
    assert_eq!(state.lock().unwrap().stopped_find, 1);
    assert_eq!(replacement_state.lock().unwrap().stopped_find, 0);
    assert_eq!(replacement_state.lock().unwrap().cancelled, 1);
    let old_unique = service.unique_name().unwrap().to_string();
    drop(service);
    let bus = zbus::fdo::DBusProxy::new(&connection).await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while bus
            .name_has_owner(old_unique.as_str().try_into().unwrap())
            .await
            .unwrap()
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(tokio::time::timeout(
        Duration::from_secs(2),
        disconnect_checked(&connection, &restored, verify)
    )
    .await
    .unwrap()
    .is_err());
    assert_eq!(
        replacement_state.lock().unwrap().disconnected,
        1,
        "a dead old owner must not redirect cleanup to the replacement"
    );
}

#[tokio::test]
#[ignore = "requires an explicitly isolated dbus-run-session"]
async fn device_name_host_uses_owned_go_and_cleans_up_failed_wps() {
    assert_eq!(
        std::env::var("LINUXDROP_TEST_PRIVATE_P2P").as_deref(),
        Ok("1")
    );
    let state = Shared::default();
    let service = serve(state.clone(), Arc::new(tokio::sync::Notify::new())).await;
    let connection = Connection::session().await.unwrap();
    let group = create_group_inner(
        connection.clone(),
        "testwifi0",
        5180,
        CancellationToken::new(),
        verify,
    )
    .await
    .unwrap();
    assert!(group.device_name.is_none());
    assert_eq!(state.lock().unwrap().wps_started, 0);
    drop(group);
    wait_disconnect(&state, 1).await;
    for case in ["ok", "rejected", "changed", "empty"] {
        {
            let mut state = state.lock().unwrap();
            state.wps_reject = case == "rejected";
            state.name_changed = case == "changed";
            state.name_empty = case == "empty";
        }
        let before = state.lock().unwrap().disconnected;
        let result = create_group_authenticated(
            connection.clone(),
            "testwifi0",
            5180,
            crate::P2pHostAuth::DeviceName,
            CancellationToken::new(),
            verify,
            None,
        )
        .await;
        if case == "ok" {
            let hosted = result.unwrap();
            assert_eq!(
                hosted.device_name.as_deref(),
                Some("Existing LinuxDrop radio")
            );
            assert_eq!(
                hosted.group.identity.service_owner,
                service.unique_name().unwrap().as_str()
            );
            drop(hosted);
        } else {
            assert!(result.is_err(), "{case}");
        }
        wait_disconnect(&state, before + 1).await;
    }
    // Invalid/changing names never start WPS, and no shared identity is written.
    assert_eq!(state.lock().unwrap().wps_started, 2);
}

#[tokio::test]
#[ignore = "requires an explicitly isolated dbus-run-session"]
async fn failed_group_creation_and_drop_wait_for_cleanup_receipts() {
    assert_eq!(
        std::env::var("LINUXDROP_TEST_PRIVATE_P2P").as_deref(),
        Ok("1")
    );
    let state = Shared::default();
    let gate = Arc::new(tokio::sync::Semaphore::new(0));
    let started = {
        let mut state = state.lock().unwrap();
        state.wrong_channel = true;
        state.disconnect_gate = Some(gate.clone());
        state.disconnect_started.clone()
    };
    let _service = serve(state.clone(), Arc::new(tokio::sync::Notify::new())).await;
    let connection = Connection::session().await.unwrap();
    let client = connection.clone();
    let mut worker = tokio::spawn(async move {
        create_group_inner(client, "testwifi0", 5180, CancellationToken::new(), verify).await
    });
    tokio::time::timeout(Duration::from_secs(3), started.notified())
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut worker)
            .await
            .is_err(),
        "Creation failure must not return while its dropped group is still disconnecting"
    );
    gate.add_permits(1);
    let error = worker.await.unwrap().unwrap_err();
    assert!(error.to_string().contains("unapproved frequency"));
    assert_eq!(state.lock().unwrap().disconnected, 1);

    state.lock().unwrap().wrong_channel = false;
    let hosted = create_group_inner(
        connection.clone(),
        "testwifi0",
        5180,
        CancellationToken::new(),
        verify,
    )
    .await
    .unwrap();
    let receipt = hosted.group.settlement();
    let abandoned = tokio::spawn(receipt.clone().wait());
    abandoned.abort();
    let _ = abandoned.await;
    drop(hosted);
    tokio::time::timeout(Duration::from_secs(3), started.notified())
        .await
        .unwrap();
    let mut waiting = Box::pin(receipt.wait());
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut waiting)
            .await
            .is_err()
    );
    gate.add_permits(1);
    waiting.await.unwrap();
    assert_eq!(state.lock().unwrap().disconnected, 2);

    // Durable handoff settles this guard without issuing a disconnect; the
    // journal owner is then responsible for teardown.
    let hosted = create_group_inner(
        connection.clone(),
        "testwifi0",
        5180,
        CancellationToken::new(),
        verify,
    )
    .await
    .unwrap();
    let receipt = hosted.group.settlement();
    hosted.group.into_journaled();
    tokio::time::timeout(Duration::from_secs(1), receipt.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.lock().unwrap().disconnected, 2);

    {
        let mut state = state.lock().unwrap();
        state.disconnect_gate = None;
        state.group_present = false;
        state.disconnect_reject = true;
        state.wrong_channel = true;
    }
    let error = create_group_inner(
        connection,
        "testwifi0",
        5180,
        CancellationToken::new(),
        verify,
    )
    .await
    .unwrap_err();
    assert!(format!("{error:#}").contains("fixture disconnect rejected"));
    assert!(error.to_string().contains("cleanup failed"));
    let recovery = error.downcast_ref::<GroupCleanupFailure>().unwrap();
    assert_eq!(recovery.identity.parent_interface, "testwifi0");
    assert!(!recovery.identity.interface_object.is_empty());
    assert_eq!(state.lock().unwrap().disconnected, 2);
}

#[tokio::test]
#[ignore = "requires an explicitly isolated dbus-run-session"]
async fn disconnect_reply_does_not_settle_before_interface_removal() {
    assert_eq!(
        std::env::var("LINUXDROP_TEST_PRIVATE_P2P").as_deref(),
        Ok("1")
    );
    let state = Shared::default();
    state.lock().unwrap().retain_after_disconnect = true;
    let _service = serve(state.clone(), Arc::new(tokio::sync::Notify::new())).await;
    let connection = Connection::session().await.unwrap();
    let hosted = create_group_inner(
        connection,
        "testwifi0",
        5180,
        CancellationToken::new(),
        verify,
    )
    .await
    .unwrap();
    let receipt = hosted.group.settlement();
    drop(hosted);
    wait_disconnect(&state, 1).await;
    let mut receipt = Box::pin(receipt.wait());
    assert!(
        tokio::time::timeout(Duration::from_millis(150), &mut receipt)
            .await
            .is_err(),
        "A successful Disconnect reply cannot retire a still-present group interface"
    );
    // No PropertiesChanged signal: the confirmation must bypass stale caches.
    state.lock().unwrap().group_present = false;
    tokio::time::timeout(Duration::from_secs(1), receipt)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
#[ignore = "requires an explicitly isolated dbus-run-session"]
async fn unknown_formation_is_never_a_successful_cleanup_receipt() {
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
    let _service = serve(state.clone(), added.clone()).await;
    // Malformed initial signal, malformed signal racing Cancel, rejected Cancel,
    // and successful Cancel with no group: only the last has a safe receipt.
    for (delay, malformed, rejected, silent, uncertain) in [
        (false, true, false, false, true),
        (true, true, false, false, true),
        (true, false, true, true, true),
        (true, false, false, true, false),
    ] {
        *state.lock().unwrap() = State {
            delay,
            malformed_started: malformed,
            cancel_reject: rejected,
            cancel_silent: silent,
            ..Default::default()
        };
        let _ = tokio::time::timeout(Duration::from_millis(1), added.notified()).await;
        let connection = Connection::session().await.unwrap();
        let cancel = CancellationToken::new();
        let worker_cancel = cancel.clone();
        let worker = tokio::spawn(async move {
            create_group_inner(connection, "testwifi0", 5180, worker_cancel, verify).await
        });
        tokio::time::timeout(Duration::from_secs(3), added.notified())
            .await
            .unwrap();
        if delay {
            cancel.cancel();
        }
        let error = tokio::time::timeout(Duration::from_secs(3), worker)
            .await
            .unwrap()
            .unwrap()
            .expect_err("fixture must not produce a usable group");
        assert_eq!(
            error.downcast_ref::<FormationUncertain>().is_some(),
            uncertain,
            "{error:#}"
        );
        assert_eq!(
            state.lock().unwrap().disconnected,
            0,
            "never guess a group to delete"
        );
    }
}

#[tokio::test]
#[ignore = "private supplicant bus"]
async fn durable_formation_recovery_is_pinned_and_never_disconnects_unknown_groups() {
    assert_eq!(
        std::env::var("LINUXDROP_TEST_PRIVATE_P2P").as_deref(),
        Ok("1")
    );
    let state = Shared::default();
    let service = serve(state.clone(), Arc::new(tokio::sync::Notify::new())).await;
    let saved = prepare_formation("testwifi0").await.unwrap();
    let saved: FormationIdentity =
        serde_json::from_slice(&serde_json::to_vec(&saved).unwrap()).unwrap();
    recover_formation(&saved).await.unwrap();
    assert_eq!(state.lock().unwrap().cancelled, 1);
    state.lock().unwrap().cancel_reject = true;
    assert!(recover_formation(&saved).await.is_err());
    state.lock().unwrap().cancel_reject = false;
    // An existing or late unidentified group is retained, never disconnected.
    for parent in [false, true] {
        state.lock().unwrap().group_present = !parent;
        state.lock().unwrap().parent_group = parent;
        let before = state.lock().unwrap().cancelled;
        assert!(recover_formation(&saved).await.is_err());
        assert_eq!(state.lock().unwrap().cancelled, before);
    }
    state.lock().unwrap().parent_group = false;
    state.lock().unwrap().delay = true;
    assert!(recover_formation(&saved).await.is_err());
    assert!(state.lock().unwrap().group_present);
    assert_eq!(state.lock().unwrap().disconnected, 0);
    state.lock().unwrap().group_present = false;
    state.lock().unwrap().delay = false;
    let mut foreign_bus = saved.clone();
    foreign_bus.bus_guid = "another-bus".into();
    assert!(recover_formation(&foreign_bus).await.is_err());
    service.release_name(SERVICE).await.unwrap();
    let replacement_state = Shared::default();
    let _replacement = serve(
        replacement_state.clone(),
        Arc::new(tokio::sync::Notify::new()),
    )
    .await;
    assert!(recover_formation(&saved).await.is_err());
    assert_eq!(replacement_state.lock().unwrap().cancelled, 0);
    assert_eq!(replacement_state.lock().unwrap().disconnected, 0);
}

#[tokio::test]
#[ignore = "private supplicant bus"]
async fn prepared_formation_refuses_changed_inventory_before_any_submission() {
    assert_eq!(
        std::env::var("LINUXDROP_TEST_PRIVATE_P2P").as_deref(),
        Ok("1")
    );
    let state = Shared::default();
    let _service = serve(state.clone(), Arc::new(tokio::sync::Notify::new())).await;
    let prepared = prepare_formation("testwifi0").await.unwrap();
    state.lock().unwrap().existing = true;
    assert!(create_group_authenticated(
        Connection::session().await.unwrap(),
        "testwifi0",
        5180,
        crate::P2pHostAuth::Password,
        CancellationToken::new(),
        verify,
        Some(prepared.clone())
    )
    .await
    .is_err());
    assert!(connect_on(
        Connection::session().await.unwrap(),
        "testwifi0".into(),
        "Peer".into(),
        String::new(),
        2437,
        CancellationToken::new(),
        FormationChecks {
            verify,
            prepared: Some(prepared)
        }
    )
    .await
    .is_err());
    let state = state.lock().unwrap();
    assert!(!state.group_present);
    assert_eq!(state.found, 0);
    assert_eq!(state.cancelled, 0);
}
