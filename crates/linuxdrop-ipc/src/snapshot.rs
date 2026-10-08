//! Typed status envelope; hardware detail and diagnostics have separate schemas.
use crate::{HardwareStatus, Settings};
use anyhow::{bail, Result};
use linuxdrop_core::{BackendState, Peer, Transfer};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

pub const IDENTITY_SCOPE: &str = "Preferences apply to this protocol identifier; they do not authenticate a person. Discovery identifiers can change.";

#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerView {
    #[serde(flatten)]
    pub peer: Peer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub favorite: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preferred_protocol: Option<String>,
    pub identity_scope: String,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownPeer {
    pub id: String,
    pub favorite: bool,
    pub display_name: String,
    pub blocked: bool,
    pub preferred_protocol: String,
    pub available: bool,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionMode {
    Native,
    PublishSelected,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferView {
    #[serde(flatten)]
    pub transfer: Transfer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receive_directory: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_mode: Option<SelectionMode>,
}
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub epoch: String,
    pub revision: u64,
    pub restarting: bool,
    pub download_link_active: bool,
    pub peers: Vec<PeerView>,
    pub known_peers: Vec<KnownPeer>,
    pub transfers: Vec<TransferView>,
    pub backends: Vec<BackendState>,
    pub settings: Settings,
    pub hardware: HardwareStatus,
}
impl Snapshot {
    pub fn from_value(value: &Value) -> Result<Self> {
        let snapshot: Self = serde_json::from_value(value.clone())?;
        Settings::from_value(&value["settings"])?;
        snapshot.validate()?;
        Ok(snapshot)
    }
    pub fn validate(&self) -> Result<()> {
        self.hardware.validate()?;
        if self.epoch.is_empty() {
            bail!("Invalid status envelope");
        }
        let mut peers = std::collections::HashSet::new();
        if self
            .peers
            .iter()
            .any(|p| p.peer.id.is_empty() || !peers.insert(&p.peer.id))
        {
            bail!("Invalid or duplicate peer identifier");
        }
        let mut transfers = std::collections::HashSet::new();
        for view in &self.transfers {
            let t = &view.transfer;
            if t.id.is_empty()
                || !transfers.insert(&t.id)
                || !matches!(t.direction.as_str(), "incoming" | "outgoing")
                || t.transferred_bytes > t.total_bytes
                || t.files.iter().any(|f| f.transferred > f.size)
            {
                bail!("Invalid transfer status");
            }
        }
        Ok(())
    }
}
