//! One payload budget per engine, shared by its incoming and outgoing sessions.
//! Handshake, consent and keep-alive metadata do not consume the file budget.
use crate::channel::{ChannelMessage, Message, TransferAction};
use linuxdrop_network::BandwidthLimiter;
use once_cell::sync::Lazy;
use std::sync::RwLock;
use tokio::sync::broadcast::Receiver;
use tokio_util::sync::CancellationToken;

static BUDGET: Lazy<RwLock<BandwidthLimiter>> =
    Lazy::new(|| RwLock::new(BandwidthLimiter::new(None)));
pub fn set(rate: Option<u64>) {
    *BUDGET.write().unwrap() = BandwidthLimiter::new(rate);
}
pub fn set_budget(budget: BandwidthLimiter) {
    *BUDGET.write().unwrap() = budget;
}
pub fn current() -> BandwidthLimiter {
    BUDGET.read().unwrap().clone()
}

/// Returns false for a matching cancellation. Call only after payload consent;
/// unrelated transfer messages cannot interrupt this request's budget wait.
pub async fn acquire(
    budget: &BandwidthLimiter,
    bytes: usize,
    control: &mut Receiver<ChannelMessage>,
    id: &str,
) -> anyhow::Result<bool> {
    let cancel = CancellationToken::new();
    let permit = budget.acquire(bytes, &cancel);
    tokio::pin!(permit);
    loop {
        tokio::select! {
            biased;
            message = control.recv() => {
                let message = message?;
                if (message.id == id || message.id == "*") && matches!(message.msg, Message::Lib {action: TransferAction::TransferCancel}) {
                    return Ok(false);
                }
            }
            result = &mut permit => return result.map(|()| true),
        }
    }
}
