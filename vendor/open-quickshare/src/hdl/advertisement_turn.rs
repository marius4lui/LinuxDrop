//! Fair Quick Share advertising turns on controllers with a single usable slot.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex, MutexGuard, Notify};
use tokio_util::sync::CancellationToken;

// An uncertain owner poisons admission until process restart. Never evict an
// external advertisement or open a replacement before cleanup is confirmed.
static SLOT: Mutex<bool> = Mutex::const_new(true);
static WAITING_SENDERS: AtomicUsize = AtomicUsize::new(0);
static REQUESTED: Notify = Notify::const_new();
struct Request;
impl Drop for Request {
    fn drop(&mut self) {
        WAITING_SENDERS.fetch_sub(1, Ordering::SeqCst);
    }
}
pub(super) struct Turn(MutexGuard<'static, bool>);
impl Turn {
    pub(super) async fn acquire(
        sender: bool,
        cancel: &CancellationToken,
    ) -> anyhow::Result<Option<Self>> {
        let _request = sender.then(|| {
            WAITING_SENDERS.fetch_add(1, Ordering::SeqCst);
            REQUESTED.notify_waiters();
            Request
        });
        let guard = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Ok(None),
            guard = SLOT.lock() => guard,
        };
        if !*guard {
            return Err(crate::lifecycle::cleanup_failure(
                "Prior Quick Share advertisement removal is unconfirmed",
            ));
        }
        Ok(Some(Self(guard)))
    }
    pub(super) fn registering(&mut self) {
        *self.0 = false;
    }
    pub(super) fn cleared(&mut self) {
        *self.0 = true;
    }
}
/// Yield only between connections. Register the notification before checking
/// the counter, then poll while a connection temporarily prevents yielding.
pub(super) async fn sender_requested() {
    loop {
        let notified = REQUESTED.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if WAITING_SENDERS.load(Ordering::SeqCst) > 0 {
            if !super::scanning_suppressed() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        } else {
            notified.await;
        }
    }
}
