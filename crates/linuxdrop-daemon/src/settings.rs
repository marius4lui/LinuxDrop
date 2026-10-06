use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::Path;

pub fn defaults() -> Value {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    json!({
      "schema_version":1,
      "general":{"device_name":"LinuxDrop","autostart":false,"appearance":"system","language":"system","close_behavior":"background"},
      "receive":{"directory":download_directory(&home),"open_after":false,"open_folder":false,"max_files":1000,"max_bytes":107374182400u64,"ask_directory":false,"collision_policy":"rename","subfolders":"none"},
      "visibility":{"mode":"hidden","duration_minutes":10,"hide_on_lock":true},
      "localsend":{"enabled":true,"port":53317,"https":true,"multicast":true,"require_pin":false,"pin":""},
      "quickshare":{"enabled":true,"ble":true,"port":53320},
      "airdrop":{"enabled":false,"ble_wakeup":true,"send":true,"receive":true},
      "hardware":{"preferred_adapter":"","prefer_usb":true,"protect_active_connection":true,"auto_use_usb":true,"open_on_adapter":false},
      "bluetooth":{"adapter":""},
      "notifications":{"completed":true,"errors":true,"incoming":true,"sound":false,"private_content":true},
      "transfers":{"max_parallel":3,"history_limit":100,"history_days":30,"bandwidth_limit_mbps":0},
      "network":{"allowed_interfaces":[],"allow_virtual_interfaces":false},
      "diagnostics":{"log_level":"info"}
    })
}

fn download_directory(home: &str) -> String {
    let config = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{home}/.config"));
    if let Ok(text) = std::fs::read_to_string(Path::new(&config).join("user-dirs.dirs")) {
        for line in text.lines() {
            if let Some(raw) = line.trim().strip_prefix("XDG_DOWNLOAD_DIR=") {
                let path = raw.trim().trim_matches('"').replace("$HOME", home);
                if Path::new(&path).is_absolute() && !path.contains(['$', '`']) {
                    return Path::new(&path)
                        .join("LinuxDrop")
                        .to_string_lossy()
                        .into_owned();
                }
            }
        }
    }
    format!("{home}/Downloads/LinuxDrop")
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
                || slot.is_array() != value.is_array()
            {
                bail!("Invalid type for {key}");
            }
            *slot = value.clone();
        }
    }
    Ok(())
}

pub fn needs_backend_restart(old: &Value, next: &Value) -> bool {
    ["localsend", "quickshare", "airdrop", "bluetooth", "network"]
        .iter()
        .any(|key| old[key] != next[key])
        || [
            ("general", "device_name"),
            ("receive", "directory"),
            ("receive", "max_files"),
            ("receive", "max_bytes"),
            ("hardware", "preferred_adapter"),
            ("hardware", "prefer_usb"),
            ("transfers", "bandwidth_limit_mbps"),
        ]
        .iter()
        .any(|(section, key)| old[section][key] != next[section][key])
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
        ("quickshare", "port", 1024, 65535),
        ("receive", "max_files", 1, 10000),
        ("receive", "max_bytes", 1, 10995116277760),
        ("visibility", "duration_minutes", 0, 1440),
        ("transfers", "max_parallel", 1, 16),
        ("transfers", "history_limit", 0, 10000),
        ("transfers", "history_days", 0, 3650),
        ("transfers", "bandwidth_limit_mbps", 0, 100000),
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
    for (section, key, choices) in [
        ("general", "language", &["system", "de", "en"][..]),
        (
            "general",
            "close_behavior",
            &["background", "quit_when_idle"][..],
        ),
        ("receive", "collision_policy", &["rename", "reject"][..]),
        (
            "receive",
            "subfolders",
            &["none", "sender", "date", "sender_date"][..],
        ),
        ("diagnostics", "log_level", &["warn", "info", "debug"][..]),
    ] {
        if !value[section][key]
            .as_str()
            .is_some_and(|s| choices.contains(&s))
        {
            bail!("Invalid {section}.{key}");
        }
    }
    if value["localsend"]["port"] == value["quickshare"]["port"] {
        bail!("LocalSend and Quick Share require distinct TCP ports");
    }
    let pin = value["localsend"]["pin"].as_str().unwrap_or("");
    if !pin.is_empty()
        && (!(4..=12).contains(&pin.len()) || !pin.bytes().all(|b| b.is_ascii_digit()))
    {
        bail!("Receive PIN must have 4 to 12 digits");
    }
    if value["localsend"]["require_pin"] == true && pin.is_empty() {
        bail!("Set a PIN before requiring it");
    }
    let adapter = value["bluetooth"]["adapter"].as_str().unwrap_or("");
    if !adapter.is_empty()
        && !adapter
            .strip_prefix("hci")
            .is_some_and(|n| !n.is_empty() && n.len() <= 4 && n.bytes().all(|b| b.is_ascii_digit()))
    {
        bail!("Bluetooth adapter must be an hci controller name");
    }
    let interfaces = value["network"]["allowed_interfaces"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Interface list must be an array"))?;
    if interfaces.len() > 32
        || interfaces.iter().any(|v| {
            !v.as_str().is_some_and(|s| {
                !s.is_empty()
                    && s.len() <= 15
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
            })
        })
    {
        bail!("Invalid network interface allowlist");
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
    #[test]
    fn presentation_edits_do_not_interrupt_transfers() {
        let old = defaults();
        let mut next = old.clone();
        merge(
            &mut next,
            &json!({"general":{"appearance":"dark"},"notifications":{"sound":true}}),
        )
        .unwrap();
        assert!(!needs_backend_restart(&old, &next));
        merge(
            &mut next,
            &json!({"network":{"allowed_interfaces":["wlan0"]}}),
        )
        .unwrap();
        assert!(needs_backend_restart(&old, &next));
    }
}
