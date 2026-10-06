use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CollisionPolicy {
    #[default]
    Rename,
    Reject,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReceiveOptions {
    pub directory: Option<PathBuf>,
    pub selected_indices: Option<Vec<usize>>,
    pub collision_policy: CollisionPolicy,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TransferPolicy {
    pub allowed_interfaces: Vec<String>,
    pub allow_virtual_interfaces: bool,
    pub bandwidth_bytes_per_second: Option<u64>,
    pub bluetooth_adapter: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub protocols: Vec<String>,
    pub address: String,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferFile {
    pub name: String,
    pub size: u64,
    pub transferred: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transfer {
    pub id: String,
    pub peer_id: String,
    pub peer_name: String,
    pub protocol: String,
    pub direction: String,
    pub state: String,
    pub files: Vec<TransferFile>,
    pub total_bytes: u64,
    pub transferred_bytes: u64,
    pub error: Option<String>,
    pub verification_code: Option<String>,
    pub saved_paths: Vec<String>,
}

impl Transfer {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.state.as_str(),
            "completed" | "rejected" | "cancelled" | "failed"
        )
    }

    /// Reject stale terminal updates and progress that could mislead a client.
    pub fn accepts_update(&self, next: &Self) -> bool {
        self.id == next.id
            && !self.is_terminal()
            && next.transferred_bytes <= next.total_bytes
            && next.transferred_bytes >= self.transferred_bytes
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendState {
    pub id: String,
    pub state: String,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub enum BackendCommand {
    Send {
        transfer_id: String,
        peer_id: String,
        files: Vec<PathBuf>,
    },
    Accept {
        transfer_id: String,
    },
    AcceptWithOptions {
        transfer_id: String,
        options: ReceiveOptions,
    },
    ProvidePin {
        transfer_id: String,
        pin: String,
    },
    Reject {
        transfer_id: String,
    },
    Cancel {
        transfer_id: String,
    },
    SetVisibility {
        visible: bool,
    },
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum BackendEvent {
    PeerUpsert(Peer),
    PeerRemoved { peer_id: String },
    Incoming(Transfer),
    TransferUpdated(Transfer),
    StateChanged(BackendState),
}

pub type EventSender = tokio::sync::mpsc::Sender<BackendEvent>;
pub type CommandSender = tokio::sync::mpsc::Sender<BackendCommand>;

#[cfg(test)]
mod tests {
    use super::*;
    fn transfer(state: &str, bytes: u64) -> Transfer {
        Transfer {
            id: "one".into(),
            peer_id: "peer".into(),
            peer_name: "Device".into(),
            protocol: "localsend".into(),
            direction: "incoming".into(),
            state: state.into(),
            files: vec![],
            total_bytes: 100,
            transferred_bytes: bytes,
            error: None,
            verification_code: None,
            saved_paths: vec![],
        }
    }
    #[test]
    fn terminal_cancellation_cannot_be_overwritten() {
        assert!(!transfer("cancelled", 25).accepts_update(&transfer("completed", 100)));
        assert!(!transfer("transferring", 25).accepts_update(&transfer("transferring", 24)));
        assert!(!transfer("transferring", 25).accepts_update(&transfer("completed", 101)));
        assert!(transfer("transferring", 25).accepts_update(&transfer("completed", 100)));
    }
}
