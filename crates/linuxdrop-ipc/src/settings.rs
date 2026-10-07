//! Shared schema-v1 settings. Requests still use a bounded, strict patch API.
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub schema_version: u64,
    pub general: GeneralSettings,
    pub receive: ReceiveSettings,
    pub visibility: VisibilitySettings,
    pub localsend: LocalsendSettings,
    pub quickshare: QuickshareSettings,
    pub airdrop: AirdropSettings,
    pub hardware: HardwareSettings,
    pub bluetooth: BluetoothSettings,
    pub notifications: NotificationsSettings,
    pub transfers: TransfersSettings,
    pub network: NetworkSettings,
    pub diagnostics: DiagnosticsSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralSettings {
    pub device_name: String,
    pub autostart: bool,
    pub appearance: String,
    pub language: String,
    pub close_behavior: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiveSettings {
    pub directory: String,
    pub open_after: bool,
    pub open_folder: bool,
    pub max_files: u64,
    pub max_bytes: u64,
    pub ask_directory: bool,
    pub collision_policy: String,
    pub subfolders: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisibilitySettings {
    pub mode: String,
    pub duration_minutes: u64,
    pub hide_on_lock: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalsendSettings {
    pub enabled: bool,
    pub port: u64,
    pub https: bool,
    pub multicast: bool,
    pub require_pin: bool,
    pub pin: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickshareSettings {
    pub enabled: bool,
    pub ble: bool,
    pub port: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AirdropSettings {
    pub enabled: bool,
    pub ble_wakeup: bool,
    pub send: bool,
    pub receive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareSettings {
    pub preferred_adapter: String,
    pub prefer_usb: bool,
    pub protect_active_connection: bool,
    pub auto_use_usb: bool,
    pub open_on_adapter: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BluetoothSettings {
    pub adapter: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationsSettings {
    pub completed: bool,
    pub errors: bool,
    pub incoming: bool,
    pub sound: bool,
    pub private_content: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransfersSettings {
    pub max_parallel: u64,
    pub history_limit: u64,
    pub history_days: u64,
    pub bandwidth_limit_mbps: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkSettings {
    pub allowed_interfaces: Vec<String>,
    pub allow_virtual_interfaces: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticsSettings {
    pub log_level: String,
}

impl Settings {
    pub fn defaults(directory: impl Into<String>) -> Self {
        let mut settings: Self = serde_json::from_str(include_str!("../settings.defaults.json"))
            .expect("embedded settings match their schema");
        settings.receive.directory = directory.into();
        settings
    }
    pub fn from_value(value: &Value) -> Result<Self> {
        let settings: Self =
            serde_json::from_value(value.clone()).context("Invalid settings data")?;
        validate_semantics(value)?;
        Ok(settings)
    }
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("settings contain only JSON values")
    }
}

fn validate_semantics(value: &Value) -> Result<()> {
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
