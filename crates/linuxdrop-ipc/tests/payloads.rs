use linuxdrop_ipc::{Settings, Snapshot};
use serde_json::{json, Value};
fn snapshot() -> Value {
    json!({"epoch":"test","revision":1,"restarting":false,"download_link_active":false,"peers":[],"known_peers":[],"transfers":[],"backends":[],"hardware":{},"settings":Settings::defaults("/tmp/received")})
}
#[test]
fn every_settings_leaf_rejects_the_wrong_wire_type() {
    let defaults = Settings::defaults("/tmp/received").to_value();
    Settings::from_value(&defaults).unwrap();
    assert_eq!(
        Settings::defaults("/tmp/LinuxDrop").to_value(),
        serde_json::from_str::<Value>(include_str!("../settings.defaults.json")).unwrap(),
        "Defaults and all typed fields must remain aligned"
    );
    let mut checked = 0;
    for (section, fields) in defaults.as_object().unwrap() {
        if let Some(fields) = fields.as_object() {
            for (field, original) in fields {
                let mut malformed = defaults.clone();
                malformed[section][field] = if original.is_string() {
                    json!(false)
                } else {
                    json!("false")
                };
                assert!(
                    Settings::from_value(&malformed).is_err(),
                    "{section}.{field}"
                );
                checked += 1;
            }
        }
    }
    assert!(checked > 40);
    let mut value = defaults.clone();
    value["network"]["allowed_interfaces"] = json!([false]);
    assert!(Settings::from_value(&value).is_err());
    value = defaults.clone();
    value["localsend"]["port"] = json!(1.5);
    assert!(Settings::from_value(&value).is_err());
    value = defaults.clone();
    value["receive"]["max_bytes"] = json!(-1);
    assert!(Settings::from_value(&value).is_err());
    value = defaults.clone();
    value["hardware"]["protect_active_connection"] = json!(false);
    assert!(Settings::from_value(&value).is_err());
    value = defaults;
    value["visibility"]["mode"] = json!("contacts");
    assert!(Settings::from_value(&value).is_err());
}
#[test]
fn status_rejects_missing_safety_fields_and_impossible_progress() {
    let valid = snapshot();
    Snapshot::from_value(&valid).unwrap();
    for name in [
        "epoch",
        "revision",
        "restarting",
        "settings",
        "transfers",
        "peers",
    ] {
        let mut value = valid.clone();
        value.as_object_mut().unwrap().remove(name);
        assert!(Snapshot::from_value(&value).is_err(), "{name}");
    }
    let mut value = valid.clone();
    value["restarting"] = json!("false");
    assert!(Snapshot::from_value(&value).is_err());
    value = valid;
    value["transfers"] = json!([{"id":"transfer","peer_id":"peer","peer_name":"Phone","protocol":"quickshare","direction":"incoming","state":"transferring","files":[{"name":"a","size":2,"transferred":3}],"total_bytes":2,"transferred_bytes":2,"saved_paths":[],"error":null,"verification_code":null}]);
    assert!(Snapshot::from_value(&value).is_err());
    value["transfers"][0]["files"][0]["transferred"] = json!(2);
    Snapshot::from_value(&value).unwrap();
    value["transfers"][0]["transferred_bytes"] = json!(3);
    assert!(Snapshot::from_value(&value).is_err());
}
#[test]
fn additive_fields_survive_validation_without_becoming_permissions() {
    let mut value = snapshot();
    value["future"] = json!({"trusted":true});
    value["settings"]["future"] = json!({"auto_accept":true});
    let typed = Snapshot::from_value(&value).unwrap();
    assert!(!typed.settings.receive.open_after);
    assert_eq!(typed.settings.visibility.mode, "hidden");
    assert!(value["future"]["trusted"].as_bool().unwrap());
}
