//! Typed non-status Manager1 response payloads.
use crate::HardwareStatus;
use anyhow::{bail, Result};
use linuxdrop_core::BackendState;
pub use linuxdrop_hardware::{DiagnosticReport, DiagnosticStep, RecoveryIssue};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostics {
    pub version: String,
    pub epoch: String,
    pub backends: HashMap<String, BackendState>,
    pub hardware: HardwareStatus,
    pub active_transfers: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RedactedBackend {
    pub id: String,
    pub state: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RedactedRadio {
    pub driver: Option<String>,
    pub firmware: Option<String>,
    pub bands: Vec<String>,
    pub rfkill: bool,
    pub protected: bool,
    pub monitor: linuxdrop_hardware::Capability,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RedactedDiagnostics {
    pub version: String,
    pub platform: String,
    pub backends: Vec<RedactedBackend>,
    pub radios: Vec<RedactedRadio>,
    pub active_transfers: u64,
    pub redacted: bool,
    pub omitted: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RecoveryReport {
    Recovery {
        issues: Vec<RecoveryIssue>,
        recent_errors: Vec<String>,
    },
}
#[derive(Clone, Serialize, Deserialize)]
pub struct DownloadOffer {
    pub url: String,
    pub pin: String,
    pub expires_in: u64,
    pub encrypted: bool,
}
impl DownloadOffer {
    pub fn validate(&self) -> Result<()> {
        let address: std::net::SocketAddr = self
            .url
            .strip_prefix("http://")
            .ok_or_else(|| anyhow::anyhow!("Invalid local download URL"))?
            .parse()?;
        if address.port() == 0
            || address.ip().is_unspecified()
            || address.ip().is_multicast()
            || self.encrypted
            || self.expires_in == 0
            || !(4..=12).contains(&self.pin.len())
            || !self.pin.bytes().all(|b| b.is_ascii_digit())
        {
            bail!("Invalid download offer");
        }
        Ok(())
    }
}
/// Validate all JSON-returning Manager1 responses at a single client boundary.
/// Exported diagnostics are projected through an allowlist so additive fields
/// cannot silently enter a report labelled redacted.
pub fn validate_response(method: crate::ManagerMethod, value: Value) -> Result<Value> {
    use crate::ManagerMethod::*;
    match method {
        GetSnapshot => {
            crate::Snapshot::from_value(&value)?;
        }
        GetSettings | GetDefaults => {
            crate::Settings::from_value(&value)?;
        }
        GetDiagnostics => {
            let report: Diagnostics = serde_json::from_value(value.clone())?;
            report.hardware.validate()?;
        }
        ExportDiagnostics => {
            let report: RedactedDiagnostics = serde_json::from_value(value)?;
            if !report.redacted {
                bail!("Diagnostic export is not redacted");
            }
            return Ok(serde_json::to_value(report)?);
        }
        GetRecoveryStatus => {
            let _: RecoveryReport = serde_json::from_value(value.clone())?;
        }
        RunHardwareDiagnostic => {
            let report: DiagnosticReport = serde_json::from_value(value.clone())?;
            if report.radio_id.is_empty() || report.steps.is_empty() {
                bail!("Incomplete hardware report");
            }
        }
        CreateDownloadOffer => {
            let report: DownloadOffer = serde_json::from_value(value.clone())?;
            report.validate()?;
        }
        _ => bail!("Method does not return a JSON response"),
    }
    Ok(value)
}
