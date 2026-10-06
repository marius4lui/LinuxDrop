use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::Path;

pub fn defaults() -> Value {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    json!({
      "schema_version":1,
      "general":{"device_name":"LinuxDrop","autostart":false,"appearance":"system"},
      "receive":{"directory":format!("{home}/Downloads/LinuxDrop"),"open_after":false,"open_folder":false,"max_files":1000,"max_bytes":107374182400u64},
      "visibility":{"mode":"hidden","duration_minutes":10,"hide_on_lock":true},
      "localsend":{"enabled":true,"port":53317,"https":true,"multicast":true},
      "quickshare":{"enabled":true,"ble":true},
      "airdrop":{"enabled":false,"ble_wakeup":true},
      "hardware":{"preferred_adapter":"","prefer_usb":true,"protect_active_connection":true},
      "notifications":{"completed":true,"errors":true},
      "transfers":{"max_parallel":3,"history_limit":100}
    })
}

pub fn merge(target: &mut Value, patch: &Value) -> Result<()> {
    let object = patch
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("Settings must be an object"))?;
    for (key, value) in object {
        let slot = target
            .get_mut(key)
            .ok_or_else(|| anyhow::anyhow!("Unknown setting: {key}"))?;
        if slot.is_object() {
            merge(slot, value)?;
        } else {
            if slot.is_boolean() != value.is_boolean()
                || slot.is_number() != value.is_number()
                || slot.is_string() != value.is_string()
            {
                bail!("Invalid type for {key}");
            }
            *slot = value.clone();
        }
    }
    Ok(())
}
pub fn validate(value: &Value) -> Result<()> {
    if value["schema_version"] != 1 {
        bail!("Unsupported settings schema version");
    }
    let name = value["general"]["device_name"].as_str().unwrap_or("");
    if name.trim().is_empty() || name.len() > 80 || name.chars().any(char::is_control) {
        bail!("Device name must be 1–80 bytes without control characters");
    }
    if !matches!(
        value["general"]["appearance"].as_str(),
        Some("system" | "light" | "dark")
    ) {
        bail!("Invalid appearance");
    }
    if !matches!(
        value["visibility"]["mode"].as_str(),
        Some("hidden" | "everyone")
    ) {
        bail!("Invalid visibility");
    }
    for (section, key, min, max) in [
        ("localsend", "port", 1024, 65535),
        ("receive", "max_files", 1, 10000),
        ("receive", "max_bytes", 1, 10995116277760),
        ("visibility", "duration_minutes", 0, 1440),
        ("transfers", "max_parallel", 1, 16),
        ("transfers", "history_limit", 0, 10000),
    ] {
        let number = value[section][key]
            .as_u64()
            .ok_or_else(|| anyhow::anyhow!("Invalid number {section}.{key}"))?;
        if number < min || number > max {
            bail!("{section}.{key} must be between {min} and {max}");
        }
    }
    let path = value["receive"]["directory"].as_str().unwrap_or("");
    if !Path::new(path).is_absolute() || path.len() > 4096 {
        bail!("Receive directory must be absolute");
    }
    if value["hardware"]["protect_active_connection"] != true {
        bail!("Active internet adapters remain protected; choose a dedicated adapter");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_patch_rejects_unknown_and_unsafe_values() {
        let mut value = defaults();
        assert!(merge(&mut value, &json!({"general":{"bad":true}})).is_err());
        merge(&mut value, &json!({"localsend":{"port":0}})).unwrap();
        assert!(validate(&value).is_err());
    }
}
