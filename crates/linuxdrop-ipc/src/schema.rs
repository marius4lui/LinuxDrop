//! Build-time schema export; excluded from normal application/daemon features.
use serde_json::{json, Value};
fn setting<'a>(schema: &'a mut Value, section: &str, key: &str) -> &'a mut Value {
    let settings = schema["properties"]["settings"]["$ref"]
        .as_str()
        .unwrap()
        .trim_start_matches('#')
        .to_owned();
    let section = schema.pointer(&settings).unwrap()["properties"][section]["$ref"]
        .as_str()
        .unwrap()
        .trim_start_matches('#')
        .to_owned();
    schema
        .pointer_mut(&format!("{section}/properties/{key}"))
        .unwrap()
}
pub fn snapshot_schema() -> Value {
    let schema = schemars::generate::SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<crate::Snapshot>();
    let mut schema = serde_json::to_value(schema).unwrap();
    for &(section, key, min, max) in crate::SETTING_NUMBER_LIMITS {
        let field = setting(&mut schema, section, key);
        field["minimum"] = json!(min);
        field["maximum"] = json!(max);
    }
    for &(section, key, choices) in crate::SETTING_CHOICES {
        setting(&mut schema, section, key)["enum"] = json!(choices);
    }
    setting(&mut schema, "hardware", "protect_active_connection")["const"] = json!(true);
    schema["$defs"]["Settings"]["properties"]["schema_version"]["const"] = json!(1);
    schema["properties"]["hardware"] = json!({"type":"object"});
    schema["properties"]["epoch"]["minLength"] = json!(1);
    schema["$defs"]["TransferView"]["properties"]["direction"]["enum"] =
        json!(["incoming", "outgoing"]);
    schema["$defs"]["TransferView"]["properties"]["id"]["minLength"] = json!(1);
    schema["$defs"]["PeerView"]["properties"]["id"]["minLength"] = json!(1);
    schema
}
