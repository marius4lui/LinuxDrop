use linuxdrop_core::Transfer;
use serde::{Deserialize, Serialize};

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Serialize, Deserialize)]
pub struct Entry {
    #[serde(flatten)]
    pub transfer: Transfer,
    // Old records have no timestamp. Start retention when they are migrated.
    #[serde(default = "now")]
    pub completed_at: u64,
}

pub fn retained(completed_at: u64, days: u64, current: u64) -> bool {
    days == 0 || current.saturating_sub(completed_at) < days.saturating_mul(86400)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retention_expires_at_boundary_and_zero_preserves() {
        assert!(retained(100, 1, 86499));
        assert!(!retained(100, 1, 86500));
        assert!(retained(100, 0, u64::MAX));
        assert!(retained(200, 1, 100));
    }
}
