use serde::{Deserialize, Serialize};
use std::path::PathBuf;
mod source;
pub use source::SendSource;

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
    ReceiveOffer {
        transfer_id: String,
        url: String,
    },
    Send {
        transfer_id: String,
        peer_id: String,
        files: Vec<SendSource>,
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
/// Commands and a persistent receipt for the backend's complete cleanup.
#[derive(Debug, Clone)]
pub struct CommandSender {
    commands: tokio::sync::mpsc::Sender<BackendCommand>,
    completion: tokio::sync::watch::Receiver<Option<Result<(), String>>>,
}
impl std::ops::Deref for CommandSender {
    type Target = tokio::sync::mpsc::Sender<BackendCommand>;
    fn deref(&self) -> &Self::Target {
        &self.commands
    }
}
impl CommandSender {
    pub fn track(
        commands: tokio::sync::mpsc::Sender<BackendCommand>,
        task: tokio::task::JoinHandle<Result<(), String>>,
    ) -> Self {
        let (finished, completion) = tokio::sync::watch::channel(None);
        tokio::spawn(async move {
            let result = task
                .await
                .unwrap_or_else(|error| Err(format!("Backend cleanup task failed: {error}")));
            finished.send_replace(Some(result));
        });
        Self {
            commands,
            completion,
        }
    }
    pub async fn shutdown(&self) -> Result<(), String> {
        tokio::time::timeout(std::time::Duration::from_secs(20), async {
            let mut completion = self.completion.clone();
            if completion.borrow().is_none() {
                let _ = self.commands.send(BackendCommand::Shutdown).await;
            }
            loop {
                if let Some(result) = completion.borrow().clone() {
                    return result;
                }
                completion
                    .changed()
                    .await
                    .map_err(|_| "Backend ended without a cleanup receipt".to_owned())?;
            }
        })
        .await
        .map_err(|_| {
            "Backend resources have not stopped within 20 seconds; retry after cleanup finishes"
                .to_owned()
        })?
    }
}

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

#[cfg(test)]
mod shutdown_tests {
    use super::*;
    use tokio::sync::{mpsc, oneshot};

    #[tokio::test(start_paused = true)]
    async fn shutdown_receipt_waits_for_cleanup_and_survives_timeout() {
        let (commands, mut receiver) = mpsc::channel(1);
        let (entered, entering) = oneshot::channel();
        let (release, cleanup) = oneshot::channel();
        let backend = tokio::spawn(async move {
            assert!(matches!(
                receiver.recv().await,
                Some(BackendCommand::Shutdown)
            ));
            entered.send(()).unwrap();
            cleanup.await.unwrap();
            Ok(())
        });
        let handle = CommandSender::track(commands, backend);
        let retiring = handle.clone();
        let shutdown = tokio::spawn(async move { retiring.shutdown().await });
        entering.await.unwrap();
        assert!(
            !shutdown.is_finished(),
            "Command receipt is not resource cleanup"
        );
        tokio::time::advance(std::time::Duration::from_secs(21)).await;
        assert!(shutdown.await.unwrap().unwrap_err().contains("retry"));
        // A timeout must not abort the backend or lose the eventual receipt.
        release.send(()).unwrap();
        handle.shutdown().await.unwrap();
        handle.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn actor_panic_cannot_claim_successful_cleanup() {
        let (commands, mut receiver) = mpsc::channel(1);
        let backend = tokio::spawn(async move {
            receiver.recv().await;
            panic!("simulated cleanup failure");
        });
        let handle = CommandSender::track(commands, backend);
        let error = handle.shutdown().await.unwrap_err();
        assert!(error.contains("cleanup task failed"));
        assert_eq!(handle.shutdown().await.unwrap_err(), error);
    }
}
