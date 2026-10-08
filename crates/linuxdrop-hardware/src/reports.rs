//! Shared diagnostic data. These records do not authorize hardware changes.
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiagnosticStep {
    pub name: String,
    pub passed: bool,
    pub detail: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiagnosticReport {
    pub radio_id: String,
    pub steps: Vec<DiagnosticStep>,
    pub restored: bool,
    pub transmitted_frames: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecoveryIssue {
    pub lease_id: String,
    pub interface: String,
    pub ownership_verified: bool,
    pub detail: String,
}
