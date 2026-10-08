//! Cooperative advertising for the daemon's selected Bluetooth controller.
//! AirDrop wake, Quick Share sender and receiver all share this fair queue.
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
pub struct Turn(MutexGuard<'static, bool>);
impl Turn {
    pub async fn acquire(sender: bool, cancel: &CancellationToken) -> anyhow::Result<Option<Self>> {
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
            return Err(anyhow::anyhow!(
                "Prior Bluetooth advertisement removal is unconfirmed",
            ));
        }
        // A live outgoing connection may exist without a visible receiver
        // holding a turn. Protect that connection before starting a nudge too.
        while sender && scanning_suppressed() {
            tokio::select! {
                _ = cancel.cancelled() => return Ok(None),
                _ = tokio::time::sleep(Duration::from_millis(250)) => {},
            }
        }
        Ok(Some(Self(guard)))
    }
    pub fn registering(&mut self) {
        *self.0 = false;
    }
    pub fn cleared(&mut self) {
        *self.0 = true;
    }
}
/// Yield only between connections. Register the notification before checking
/// the counter, then poll while a connection temporarily prevents yielding.
pub async fn sender_requested() {
    loop {
        let notified = REQUESTED.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if WAITING_SENDERS.load(Ordering::SeqCst) > 0 {
            if !scanning_suppressed() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        } else {
            notified.await;
        }
    }
}

/// Number of things currently asking us to stay off the air (an in-progress
/// GATT session, an outgoing-share advertisement, ...).
static SCAN_SUPPRESSORS: AtomicUsize = AtomicUsize::new(0);

/// Suppresses BLE scanning for as long as it is held, so that the advertising
/// and connection work the radio is busy with gets the whole antenna.
#[derive(Debug)]
pub struct BleScanSuppressor(());

impl BleScanSuppressor {
    pub fn new() -> Self {
        SCAN_SUPPRESSORS.fetch_add(1, Ordering::SeqCst);
        Self(())
    }
}

impl Default for BleScanSuppressor {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for BleScanSuppressor {
    fn drop(&mut self) {
        SCAN_SUPPRESSORS.fetch_sub(1, Ordering::SeqCst);
    }
}

pub fn scanning_suppressed() -> bool {
    SCAN_SUPPRESSORS.load(Ordering::SeqCst) > 0
}
