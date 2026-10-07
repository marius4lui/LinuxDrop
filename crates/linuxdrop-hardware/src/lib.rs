//! Passive Linux hardware inventory. No method in this crate changes radio state.
//! `iw` is the kernel nl80211 client; it is invoked directly, never through a shell.
pub mod allocation;
pub mod details;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    process::Command,
    sync::watch,
    time::{timeout, Duration},
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityValue {
    Yes,
    No,
    #[default]
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    pub value: CapabilityValue,
    pub source: String,
    pub evidence: String,
}
impl Capability {
    fn unknown(reason: &str) -> Self {
        Self {
            value: CapabilityValue::Unknown,
            source: "passive inventory".into(),
            evidence: reason.into(),
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channel {
    pub frequency_mhz: u32,
    pub number: u16,
    pub disabled: bool,
    pub no_ir: bool,
    pub radar: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Radio {
    pub id: String,
    pub phy: String,
    pub bus: String,
    pub device_path: String,
    pub vendor_id: Option<String>,
    pub product_id: Option<String>,
    pub serial: Option<String>,
    pub driver: Option<String>,
    pub kernel: String,
    #[serde(default)]
    pub driver_details: details::DriverDetails,
    #[serde(default)]
    pub bands: Vec<String>,
    #[serde(default)]
    pub combinations: Vec<details::InterfaceCombination>,
    #[serde(default)]
    pub evidence_profile: details::EvidenceProfile,
    pub rfkill: bool,
    pub interfaces: Vec<String>,
    pub channels: Vec<Channel>,
    pub modes: Vec<String>,
    pub interface_combinations: Vec<String>,
    pub monitor: Capability,
    pub management_injection: Capability,
    pub data_injection: Capability,
    pub awdl: Capability,
    pub protected: bool,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkInterface {
    pub name: String,
    pub ifindex: u32,
    pub phy: Option<String>,
    pub state: String,
    pub default_route: bool,
    pub nm_state: Option<u32>,
    pub nm_managed: Option<bool>,
    pub active_connection: Option<String>,
    #[serde(default)]
    pub active_connection_uuid: Option<String>,
}
impl NetworkInterface {
    /// Treat NetworkManager's connecting states as use, before carrier appears.
    pub fn in_use(&self) -> bool {
        self.default_route
            || self.active_connection.is_some()
            || self.state == "up"
            || self
                .nm_state
                .is_some_and(|state| (40..=100).contains(&state))
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BluetoothController {
    pub id: String,
    pub name: String,
    pub powered: bool,
    pub discovering: bool,
    pub supported_advertisements: Option<u8>,
    pub active_advertisements: Option<u8>,
    #[serde(default)]
    pub remaining_advertisements: Option<u8>,
    #[serde(default)]
    pub supported_includes: Vec<String>,
    #[serde(default)]
    pub supported_secondary_channels: Vec<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Inventory {
    pub observed_unix: u64,
    pub radios: Vec<Radio>,
    pub interfaces: Vec<NetworkInterface>,
    pub bluetooth: Vec<BluetoothController>,
    pub warnings: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SelectionRequest {
    pub preferred: Option<String>,
    pub leased: Vec<String>,
    pub require_tested_awdl: bool,
    pub channel: Option<u16>,
    pub prefer_usb: bool,
}
impl Default for SelectionRequest {
    fn default() -> Self {
        Self {
            preferred: None,
            leased: Vec::new(),
            require_tested_awdl: false,
            channel: None,
            prefer_usb: true,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub score: i32,
    pub exclusions: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectionDecision {
    pub selected: Option<String>,
    pub candidates: Vec<Candidate>,
}

pub fn select_radio(inventory: &Inventory, request: &SelectionRequest) -> SelectionDecision {
    let mut candidates: Vec<_> = inventory
        .radios
        .iter()
        .map(|r| {
            let mut exclusions = Vec::new();
            if r.rfkill {
                exclusions.push("radio is blocked by rfkill".into());
            }
            if r.driver.is_none() {
                exclusions.push("no driver detected".into());
            }
            if r.protected {
                exclusions.push("radio is active or carries a default route".into());
            }
            if r.monitor.value != CapabilityValue::Yes {
                exclusions.push("monitor mode not reported by kernel".into());
            }
            if request.leased.contains(&r.id) || request.leased.contains(&r.phy) {
                exclusions.push("radio is already leased".into());
            }
            if request.require_tested_awdl && r.awdl.value != CapabilityValue::Yes {
                exclusions.push("AWDL has not been validated on this radio".into());
            }
            if let Some(channel) = request.channel {
                if !r
                    .channels
                    .iter()
                    .any(|c| c.number == channel && !c.disabled && !c.no_ir && !c.radar)
                {
                    exclusions.push("channel unavailable or transmission restricted".into());
                }
            }
            let score = if request.preferred.as_ref() == Some(&r.id) {
                1000
            } else {
                0
            } + if r.awdl.value == CapabilityValue::Yes {
                200
            } else {
                0
            } + if request.prefer_usb && r.bus == "usb" {
                50
            } else {
                0
            };
            Candidate {
                id: r.id.clone(),
                score,
                exclusions,
            }
        })
        .collect();
    candidates.sort_by(|a, b| b.score.cmp(&a.score).then(a.id.cmp(&b.id)));
    let selected = candidates
        .iter()
        .find(|c| c.exclusions.is_empty())
        .map(|c| c.id.clone());
    SelectionDecision {
        selected,
        candidates,
    }
}

fn read(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_owned())
}
fn link_name(path: impl AsRef<Path>) -> Option<String> {
    std::fs::canonicalize(path)
        .ok()?
        .file_name()?
        .to_str()
        .map(str::to_owned)
}
fn entries(path: impl AsRef<Path>) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(path)
        .map(|it| it.filter_map(Result::ok).map(|e| e.path()).collect())
        .unwrap_or_default()
}
async fn command(program: &str, args: &[&str]) -> Result<String, String> {
    let output = timeout(
        Duration::from_secs(4),
        Command::new(program)
            .args(args)
            .env("LC_ALL", "C")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| format!("{program} timed out"))?
    .map_err(|e| format!("{program}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "{program}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Interpret only fields explicitly reported by nl80211; no device whitelist implies support.
pub fn parse_iw(text: &str) -> (Vec<String>, Vec<Channel>, Vec<String>) {
    let mut modes = Vec::new();
    let mut channels = Vec::new();
    let mut combinations = Vec::new();
    let mut section = "";
    for raw in text.lines() {
        let line = raw.trim();
        if line == "Supported interface modes:" {
            section = "modes";
            continue;
        }
        if line == "valid interface combinations:" {
            section = "combinations";
            continue;
        }
        if section == "modes" && line.starts_with("* ") {
            modes.push(line[2..].into());
        } else if section == "modes" {
            section = "";
        }
        if section == "combinations" {
            if line.starts_with("* ")
                || line.starts_with("#{")
                || line.starts_with("total")
                || line.starts_with("#channels")
            {
                combinations.push(line.into());
            } else if !line.is_empty() {
                section = "";
            }
        }
        let words: Vec<_> = line.split_whitespace().collect();
        if words.len() > 3 && words[0] == "*" && words[2] == "MHz" {
            if let (Ok(frequency_mhz), Ok(number)) =
                (words[1].parse(), words[3].trim_matches(['[', ']']).parse())
            {
                channels.push(Channel {
                    frequency_mhz,
                    number,
                    disabled: line.contains("disabled"),
                    no_ir: line.contains("no IR") || line.contains("passive scanning"),
                    radar: line.contains("radar detection"),
                });
            }
        }
    }
    (modes, channels, combinations)
}

pub async fn inventory() -> Inventory {
    let mut result = Inventory {
        observed_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
        ..Default::default()
    };
    if !cfg!(target_os = "linux") {
        result
            .warnings
            .push("Hardware inventory requires Linux".into());
        return result;
    }
    let routes = read("/proc/net/route").unwrap_or_default();
    let routes6 = read("/proc/net/ipv6_route").unwrap_or_default();
    for path in entries("/sys/class/net") {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let default_route = routes.lines().skip(1).any(|l| {
            let f: Vec<_> = l.split_whitespace().collect();
            f.len() > 3
                && f[0] == name
                && f[1] == "00000000"
                && u32::from_str_radix(f[3], 16).is_ok_and(|flags| flags & 1 != 0)
        }) || routes6.lines().any(|l| {
            let f: Vec<_> = l.split_whitespace().collect();
            f.len() >= 10
                && f[0] == "00000000000000000000000000000000"
                && f[1] == "00"
                && f[9] == name
                && u32::from_str_radix(f[8], 16)
                    .is_ok_and(|flags| flags & 1 != 0 && flags & 0x200 == 0)
        });
        result.interfaces.push(NetworkInterface {
            name,
            ifindex: read(path.join("ifindex"))
                .and_then(|s| s.parse().ok())
                .unwrap_or_default(),
            phy: link_name(path.join("phy80211")),
            state: read(path.join("operstate")).unwrap_or_default(),
            default_route,
            ..Default::default()
        });
    }
    let bus = zbus::Connection::system().await;
    match bus {
        Ok(ref connection) => {
            match timeout(
                Duration::from_secs(3),
                network_manager(connection, &mut result.interfaces),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(e)) => result
                    .warnings
                    .push(format!("NetworkManager unavailable: {e}")),
                Err(_) => result
                    .warnings
                    .push("NetworkManager query timed out".into()),
            }
            match timeout(
                Duration::from_secs(3),
                bluez(connection, &mut result.bluetooth),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(e)) => result
                    .warnings
                    .push(format!("Bluetooth service unavailable: {e}")),
                Err(_) => result.warnings.push("Bluetooth query timed out".into()),
            }
        }
        Err(e) => result
            .warnings
            .push(format!("System D-Bus unavailable: {e}")),
    }
    let kernel = read("/proc/sys/kernel/osrelease").unwrap_or_default();
    for path in entries("/sys/class/ieee80211") {
        let phy = path.file_name().unwrap().to_string_lossy().into_owned();
        let device =
            std::fs::canonicalize(path.join("device")).unwrap_or_else(|_| path.join("device"));
        let mut usb = None;
        for ancestor in device.ancestors() {
            if ancestor.join("idVendor").exists() {
                usb = Some(ancestor);
                break;
            }
        }
        let identity = usb.unwrap_or(&device);
        let bus = if usb.is_some() {
            "usb"
        } else if device.join("vendor").exists() {
            "pci"
        } else {
            "platform"
        };
        let vendor_id = read(identity.join(if bus == "usb" { "idVendor" } else { "vendor" }));
        let product_id = read(identity.join(if bus == "usb" { "idProduct" } else { "device" }));
        let serial = read(identity.join("serial"));
        let id = format!(
            "{bus}:{}:{}:{}",
            vendor_id.as_deref().unwrap_or("unknown"),
            product_id.as_deref().unwrap_or("unknown"),
            serial
                .as_deref()
                .unwrap_or_else(|| identity.to_str().unwrap_or("unknown"))
        );
        let rfkill = entries(&path)
            .iter()
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("rfkill"))
            })
            .any(|p| {
                read(p.join("soft")).as_deref() == Some("1")
                    || read(p.join("hard")).as_deref() == Some("1")
            });
        let (modes, channels, interface_combinations) =
            match command("/usr/sbin/iw", &["phy", &phy, "info"]).await {
                Ok(text) => parse_iw(&text),
                Err(e) => {
                    result.warnings.push(e);
                    (Vec::new(), Vec::new(), Vec::new())
                }
            };
        let monitor = if modes.is_empty() {
            Capability::unknown("nl80211 capability query unavailable")
        } else {
            Capability {
                value: if modes.iter().any(|s| s == "monitor") {
                    CapabilityValue::Yes
                } else {
                    CapabilityValue::No
                },
                source: "nl80211 via iw".into(),
                evidence: "Supported interface modes".into(),
            }
        };
        let interfaces: Vec<_> = result
            .interfaces
            .iter()
            .filter(|i| i.phy.as_ref() == Some(&phy))
            .collect();
        let protected = interfaces.iter().any(|i| i.in_use());
        let driver = link_name(device.join("driver"));
        let mut driver_details = details::DriverDetails::default();
        if let Some(interface) = interfaces.first() {
            if let Ok(text) = command("/usr/sbin/ethtool", &["-i", &interface.name]).await {
                driver_details = details::parse_ethtool(&text);
            }
        }
        let mut bands = Vec::new();
        for (low, high, name) in [
            (2400, 2500, "2.4 GHz"),
            (4900, 5925, "5 GHz"),
            (5925, 7125, "6 GHz"),
        ] {
            if channels
                .iter()
                .any(|c| (low..high).contains(&c.frequency_mhz))
            {
                bands.push(name.to_owned());
            }
        }
        let evidence_profile = details::profile(driver.as_deref(), &bands);
        let combinations = details::parse_combinations(&interface_combinations);
        result.radios.push(Radio {
            id,
            phy,
            bus: bus.into(),
            device_path: device.to_string_lossy().into_owned(),
            vendor_id,
            product_id,
            serial,
            driver,
            driver_details,
            bands,
            combinations,
            evidence_profile,
            kernel: kernel.clone(),
            rfkill,
            interfaces: interfaces.iter().map(|i| i.name.clone()).collect(),
            channels,
            modes,
            interface_combinations,
            monitor,
            management_injection: Capability::unknown(
                "Requires explicit frame injection validation",
            ),
            data_injection: Capability::unknown("Requires explicit data frame validation"),
            awdl: Capability::unknown("Requires real Apple-device interoperability validation"),
            protected,
        });
    }
    result.interfaces.sort_by(|a, b| a.name.cmp(&b.name));
    result.radios.sort_by(|a, b| a.id.cmp(&b.id));
    result.bluetooth.sort_by(|a, b| a.id.cmp(&b.id));
    result
}

async fn network_manager(
    connection: &zbus::Connection,
    interfaces: &mut [NetworkInterface],
) -> zbus::Result<()> {
    let proxy = zbus::Proxy::new(
        connection,
        "org.freedesktop.NetworkManager",
        "/org/freedesktop/NetworkManager",
        "org.freedesktop.NetworkManager",
    )
    .await?;
    let devices: Vec<zbus::zvariant::OwnedObjectPath> = proxy.call("GetDevices", &()).await?;
    for path in devices {
        let device = zbus::Proxy::new(
            connection,
            "org.freedesktop.NetworkManager",
            path,
            "org.freedesktop.NetworkManager.Device",
        )
        .await?;
        let name: String = device.get_property("Interface").await?;
        if let Some(interface) = interfaces.iter_mut().find(|i| i.name == name) {
            interface.nm_state = device.get_property("State").await.ok();
            interface.nm_managed = device.get_property("Managed").await.ok();
            let active: Option<zbus::zvariant::OwnedObjectPath> =
                device.get_property("ActiveConnection").await.ok();
            interface.active_connection =
                active.filter(|p| p.as_str() != "/").map(|p| p.to_string());
            if let Some(path) = &interface.active_connection {
                if let Ok(active) = zbus::Proxy::new(
                    connection,
                    "org.freedesktop.NetworkManager",
                    path.as_str(),
                    "org.freedesktop.NetworkManager.Connection.Active",
                )
                .await
                {
                    interface.active_connection_uuid = active.get_property("Uuid").await.ok();
                }
            }
        }
    }
    Ok(())
}
async fn bluez(
    connection: &zbus::Connection,
    controllers: &mut Vec<BluetoothController>,
) -> zbus::Result<()> {
    let proxy = zbus::Proxy::new(
        connection,
        "org.bluez",
        "/",
        "org.freedesktop.DBus.ObjectManager",
    )
    .await?;
    type Objects = HashMap<
        zbus::zvariant::OwnedObjectPath,
        HashMap<String, HashMap<String, zbus::zvariant::OwnedValue>>,
    >;
    let objects: Objects = proxy.call("GetManagedObjects", &()).await?;
    for (path, interfaces) in objects {
        if let Some(adapter) = interfaces.get("org.bluez.Adapter1") {
            let advertising = interfaces.get("org.bluez.LEAdvertisingManager1");
            let supported = advertising
                .and_then(|p| p.get("SupportedInstances"))
                .and_then(|v| u8::try_from(v).ok());
            let active = advertising
                .and_then(|p| p.get("ActiveInstances"))
                .and_then(|v| u8::try_from(v).ok());
            let strings = |key: &str| {
                advertising
                    .and_then(|p| p.get(key))
                    .and_then(|v| v.try_clone().ok())
                    .and_then(|v| Vec::<String>::try_from(v).ok())
                    .unwrap_or_default()
            };
            controllers.push(BluetoothController {
                remaining_advertisements: supported.zip(active).map(|(s, a)| s.saturating_sub(a)),
                supported_includes: strings("SupportedIncludes"),
                supported_secondary_channels: strings("SupportedSecondaryChannels"),
                id: path.to_string(),
                name: adapter
                    .get("Alias")
                    .and_then(|v| <&str>::try_from(v).ok())
                    .unwrap_or("Bluetooth")
                    .into(),
                powered: adapter
                    .get("Powered")
                    .and_then(|v| bool::try_from(v).ok())
                    .unwrap_or(false),
                discovering: adapter
                    .get("Discovering")
                    .and_then(|v| bool::try_from(v).ok())
                    .unwrap_or(false),
                supported_advertisements: advertising
                    .and_then(|p| p.get("SupportedInstances"))
                    .and_then(|v| u8::try_from(v).ok()),
                active_advertisements: advertising
                    .and_then(|p| p.get("ActiveInstances"))
                    .and_then(|v| u8::try_from(v).ok()),
            });
        }
    }
    Ok(())
}

/// udev hotplug wakeups plus bounded reconciliation catch lost events and service restarts.
/// Consumers receive the latest complete inventory and cannot block collection.
pub fn watch_inventory() -> watch::Receiver<Inventory> {
    let (sender, receiver) = watch::channel(Inventory::default());
    tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut monitor = Command::new("/usr/bin/udevadm")
            .args([
                "monitor",
                "--udev",
                "--subsystem-match=net",
                "--subsystem-match=ieee80211",
                "--subsystem-match=rfkill",
                "--subsystem-match=bluetooth",
            ])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .ok();
        let mut lines = monitor
            .as_mut()
            .and_then(|c| c.stdout.take())
            .map(|s| tokio::io::BufReader::new(s).lines());
        loop {
            if sender.is_closed() {
                break;
            }
            let next = inventory().await;
            if sender.send(next).is_err() {
                break;
            }
            if let Some(ref mut lines) = lines {
                tokio::select! {
                    _=tokio::time::sleep(Duration::from_secs(5))=>{},
                    line=lines.next_line()=>{if matches!(line,Ok(Some(_))){tokio::time::sleep(Duration::from_millis(300)).await;}else{tokio::time::sleep(Duration::from_secs(5)).await;}},
                    _=sender.closed()=>break,
                }
            } else {
                tokio::select! {_=tokio::time::sleep(Duration::from_secs(5))=>{},_=sender.closed()=>break}
            }
        }
    });
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn channels_preserve_regulatory_limits() {
        let (modes,channels,_)=parse_iw("Supported interface modes:\n\t * managed\n\t * monitor\nBand 1:\n * 2412 MHz [1] (20.0 dBm)\n * 5260 MHz [52] (20.0 dBm) (no IR, radar detection)\n * 5745 MHz [149] (disabled)\n");
        assert_eq!(modes, vec!["managed", "monitor"]);
        assert_eq!(channels.len(), 3);
        assert!(channels[1].no_ir && channels[1].radar);
        assert!(channels[2].disabled);
    }
    #[test]
    fn active_adapter_never_wins_even_when_preferred() {
        let r = Radio {
            id: "active".into(),
            phy: "phy0".into(),
            bus: "usb".into(),
            device_path: String::new(),
            vendor_id: None,
            product_id: None,
            serial: None,
            driver: Some("ath9k_htc".into()),
            kernel: String::new(),
            driver_details: Default::default(),
            bands: vec![],
            combinations: vec![],
            evidence_profile: Default::default(),
            rfkill: false,
            interfaces: vec![],
            channels: vec![],
            modes: vec!["monitor".into()],
            interface_combinations: vec![],
            monitor: Capability {
                value: CapabilityValue::Yes,
                source: String::new(),
                evidence: String::new(),
            },
            management_injection: Capability::unknown("untested"),
            data_injection: Capability::unknown("untested"),
            awdl: Capability::unknown("untested"),
            protected: true,
        };
        let decision = select_radio(
            &Inventory {
                radios: vec![r],
                ..Default::default()
            },
            &SelectionRequest {
                preferred: Some("active".into()),
                ..Default::default()
            },
        );
        assert!(decision.selected.is_none());
        assert!(decision.candidates[0]
            .exclusions
            .iter()
            .any(|s| s.contains("active")));
    }
}
