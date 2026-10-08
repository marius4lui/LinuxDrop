//! Passive inventory plus the user daemon's radio reservation annotations.
use anyhow::{bail, Result};
use linuxdrop_hardware::{BluetoothController, Inventory, NetworkInterface, Radio};
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Reservation {
    AirDrop,
    #[serde(rename = "Quick Share")]
    QuickShare,
    #[serde(rename = "Not reserved")]
    None,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RadioView {
    #[serde(flatten)]
    pub radio: Radio,
    pub reserved_for: Reservation,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HardwareStatus {
    pub observed_unix: u64,
    pub radios: Vec<RadioView>,
    pub interfaces: Vec<NetworkInterface>,
    pub bluetooth: Vec<BluetoothController>,
    pub warnings: Vec<String>,
}
impl HardwareStatus {
    pub fn from_inventory(
        inventory: Inventory,
        awdl_phy: Option<&str>,
        direct_phy: Option<&str>,
    ) -> Self {
        Self {
            observed_unix: inventory.observed_unix,
            radios: inventory
                .radios
                .into_iter()
                .map(|radio| {
                    let reserved_for = if Some(radio.phy.as_str()) == awdl_phy {
                        Reservation::AirDrop
                    } else if Some(radio.phy.as_str()) == direct_phy {
                        Reservation::QuickShare
                    } else {
                        Reservation::None
                    };
                    RadioView {
                        radio,
                        reserved_for,
                    }
                })
                .collect(),
            interfaces: inventory.interfaces,
            bluetooth: inventory.bluetooth,
            warnings: inventory.warnings,
        }
    }
    pub fn validate(&self) -> Result<()> {
        for ids in [
            self.radios
                .iter()
                .map(|r| r.radio.id.as_str())
                .collect::<Vec<_>>(),
            self.radios.iter().map(|r| r.radio.phy.as_str()).collect(),
            self.interfaces.iter().map(|i| i.name.as_str()).collect(),
            self.bluetooth.iter().map(|b| b.id.as_str()).collect(),
        ] {
            let mut seen = std::collections::HashSet::new();
            if ids.iter().any(|id| id.is_empty() || !seen.insert(id)) {
                bail!("Invalid hardware identifier");
            }
        }
        for view in &self.radios {
            let radio = &view.radio;
            if !radio.protected
                && self.interfaces.iter().any(|i| {
                    (i.phy.as_deref() == Some(&radio.phy) || radio.interfaces.contains(&i.name))
                        && i.in_use()
                })
            {
                bail!("Active network interface must remain protected");
            }
        }
        Ok(())
    }
}
