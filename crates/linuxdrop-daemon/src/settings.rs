use anyhow::{bail, Result};
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
use std::path::Path;

pub fn defaults() -> Value {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    linuxdrop_ipc::Settings::defaults(download_directory(&home)).to_value()
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
            ("hardware", "auto_use_usb"),
            ("transfers", "bandwidth_limit_mbps"),
        ]
        .iter()
        .any(|(section, key)| old[section][key] != next[section][key])
}
pub fn validate(value: &Value) -> Result<()> {
    linuxdrop_ipc::Settings::from_value(value).map(|_| ())
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
        let mut next = old.clone();
        merge(&mut next, &json!({"hardware":{"auto_use_usb":false}})).unwrap();
        assert!(needs_backend_restart(&old, &next));
    }
}
