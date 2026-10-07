use linuxdrop_ipc::{Settings, Snapshot};
use serde_json::{json, Value};
fn snapshot() -> Value {
    json!({"epoch":"test","revision":1,"restarting":false,"download_link_active":false,"peers":[],"known_peers":[],"transfers":[],"backends":[],"hardware":linuxdrop_ipc::HardwareStatus::default(),"settings":Settings::defaults("/tmp/received")})
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

#[test]
fn hardware_preserves_evidence_and_rejects_unsafe_shapes() {
    let mut value = snapshot();
    value["hardware"] = serde_json::from_str(include_str!("hardware.fixture.json")).unwrap();
    let valid = value.clone();
    let typed = Snapshot::from_value(&value).unwrap();
    assert_eq!(
        typed.hardware.radios[0]
            .radio
            .driver_details
            .firmware
            .as_deref(),
        Some("test-firmware")
    );
    for key in [
        "protected",
        "rfkill",
        "monitor",
        "awdl",
        "reserved_for",
        "phy",
    ] {
        let mut bad = valid.clone();
        bad["hardware"]["radios"][0]
            .as_object_mut()
            .unwrap()
            .remove(key);
        assert!(Snapshot::from_value(&bad).is_err(), "missing {key}");
    }
    value["hardware"]["radios"][0]["protected"] = json!("false");
    assert!(Snapshot::from_value(&value).is_err());
    value = valid.clone();
    value["hardware"]["radios"][0]["awdl"]["value"] = json!("supported");
    assert!(Snapshot::from_value(&value).is_err());
    value = valid.clone();
    value["hardware"]["interfaces"] = json!([{"name":"wlan-test","ifindex":3,"phy":"phy-test","state":"up","default_route":true,"nm_state":100,"nm_managed":true,"active_connection":"active"}]);
    assert!(
        Snapshot::from_value(&value).is_err(),
        "Active interface falsely declared unprotected"
    );
    value["hardware"]["radios"][0]["protected"] = json!(true);
    Snapshot::from_value(&value).unwrap();
    value = valid;
    let radio = value["hardware"]["radios"][0].clone();
    value["hardware"]["radios"]
        .as_array_mut()
        .unwrap()
        .push(radio);
    assert!(Snapshot::from_value(&value).is_err(), "Duplicate radio");
}

#[test]
fn report_and_offer_contracts_reject_misleading_results() {
    use linuxdrop_ipc::{validate_response, ManagerMethod::*};
    let offer =
        json!({"url":"http://127.0.0.1:53318","pin":"123456","expires_in":600,"encrypted":false});
    validate_response(CreateDownloadOffer, offer.clone()).unwrap();
    for (field, bad) in [
        ("url", json!("file:///tmp/secret")),
        ("url", json!("http://name.invalid:53318")),
        ("url", json!("http://127.0.0.1:0")),
        ("url", json!("http://127.0.0.1:53318/path")),
        ("pin", json!(false)),
        ("pin", json!("1234\n")),
        ("expires_in", json!(0)),
        ("encrypted", json!(true)),
    ] {
        let mut value = offer.clone();
        value[field] = bad;
        assert!(
            validate_response(CreateDownloadOffer, value).is_err(),
            "{field}"
        );
    }
    let mut report = json!({"radio_id":"radio-test","restored":false,"transmitted_frames":0,"steps":[{"name":"restore","passed":false,"detail":"pending cleanup"}]});
    validate_response(RunHardwareDiagnostic, report.clone()).unwrap();
    report["restored"] = json!("true");
    assert!(validate_response(RunHardwareDiagnostic, report).is_err());
    let recovery = json!({"status":"recovery","issues":[{"lease_id":"test","interface":"ld0","ownership_verified":false,"detail":"inspect"}],"recent_errors":[]});
    validate_response(GetRecoveryStatus, recovery.clone()).unwrap();
    let mut bad = recovery;
    bad["status"] = json!("ok");
    assert!(validate_response(GetRecoveryStatus, bad).is_err());
    let export = json!({"version":"test","platform":"linux","backends":[{"id":"localsend","state":"ready","detail":"private"}],"radios":[],"active_transfers":0,"redacted":true,"omitted":[],"private_extension":"secret"});
    let projected = validate_response(ExportDiagnostics, export.clone()).unwrap();
    assert!(projected.get("private_extension").is_none());
    assert!(projected["backends"][0].get("detail").is_none());
    let mut bad = export;
    bad["redacted"] = json!(false);
    assert!(validate_response(ExportDiagnostics, bad).is_err());
}
