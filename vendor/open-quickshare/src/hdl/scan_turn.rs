//! Serialize LinuxDrop scans and refuse replacement after uncertain cleanup.
use btleplug::api::Central;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::sync::{Mutex, MutexGuard};
use tokio_util::sync::CancellationToken;
static INCOMPLETE: AtomicBool = AtomicBool::new(false);
static SCAN: Mutex<bool> = Mutex::const_new(true);
pub(crate) struct ScanTurn(MutexGuard<'static, bool>);
impl ScanTurn {
    pub(crate) async fn acquire(cancel: &CancellationToken) -> anyhow::Result<Option<Self>> {
        if INCOMPLETE.load(Ordering::SeqCst) {
            return Err(crate::lifecycle::cleanup_failure(
                "A prior Bluetooth scan has unconfirmed cleanup",
            ));
        }
        let guard = tokio::select! { biased; _ = cancel.cancelled() => return Ok(None), guard = SCAN.lock() => guard };
        if !*guard {
            return Err(crate::lifecycle::cleanup_failure(
                "A prior Bluetooth scan has unconfirmed cleanup",
            ));
        }
        Ok(Some(Self(guard)))
    }
    pub(crate) async fn confirm_idle() -> anyhow::Result<()> {
        if INCOMPLETE.load(Ordering::SeqCst) {
            return Err(crate::lifecycle::cleanup_failure(
                "Bluetooth scan removal is still pending",
            ));
        }
        match tokio::time::timeout(Duration::from_secs(5), SCAN.lock()).await {
            Ok(guard) if *guard => Ok(()),
            _ => Err(crate::lifecycle::cleanup_failure(
                "Bluetooth scan removal is unconfirmed",
            )),
        }
    }
    pub(crate) fn started(&mut self) {
        *self.0 = false;
    }
    pub(crate) fn cleared(&mut self) {
        *self.0 = true;
    }
    pub(crate) fn defer_cleanup(adapter: btleplug::platform::Adapter, mut turn: Self) {
        INCOMPLETE.store(true, Ordering::SeqCst);
        // Exactly one uncertain scanner owns the turn. Cleanup stays pinned to
        // its original bus owner and can later restore safe admission.
        tokio::spawn(async move {
            loop {
                if matches!(
                    tokio::time::timeout(Duration::from_secs(5), adapter.stop_scan()).await,
                    Ok(Ok(()))
                ) {
                    turn.cleared();
                    INCOMPLETE.store(false, Ordering::SeqCst);
                    break;
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
    }
}
