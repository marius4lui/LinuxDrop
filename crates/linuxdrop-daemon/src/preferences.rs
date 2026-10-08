use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PeerPreferences {
    pub favorite: bool,
    pub display_name: String,
    pub blocked: bool,
    pub preferred_protocol: String,
}

impl PeerPreferences {
    pub fn patch(&self, patch: Value) -> Result<Self> {
        let mut value = serde_json::to_value(self)?;
        let object = patch
            .as_object()
            .ok_or_else(|| anyhow::anyhow!("Preferences must be an object"))?;
        for (key, new) in object {
            let slot = value
                .get_mut(key)
                .ok_or_else(|| anyhow::anyhow!("Unknown peer preference"))?;
            if slot.is_string() != new.is_string() || slot.is_boolean() != new.is_boolean() {
                bail!("Invalid peer preference type");
            }
            *slot = new.clone();
        }
        let next: Self = serde_json::from_value(value)?;
        if next.display_name.len() > 80 || next.display_name.chars().any(char::is_control) {
            bail!("Display name is too long or contains control characters");
        }
        if !matches!(
            next.preferred_protocol.as_str(),
            "" | "automatic" | "localsend" | "quickshare" | "airdrop"
        ) {
            bail!("Unknown protocol preference");
        }
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn user_labels_cannot_enable_trust_or_escape_validation() {
        let old = PeerPreferences::default();
        assert!(old.patch(json!({"trusted":true})).is_err());
        assert!(old.patch(json!({"blocked":"false"})).is_err());
        assert!(old.patch(json!({"display_name":"bad\nname"})).is_err());
        assert!(
            old.patch(json!({"favorite":true,"display_name":"My phone"}))
                .unwrap()
                .favorite
        );
    }
}
