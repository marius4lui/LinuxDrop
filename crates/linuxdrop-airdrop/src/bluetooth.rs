//! Recover the optional Apple wake advertisement independently of AWDL traffic.
use anyhow::Result;
use std::time::Duration;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

pub(super) async fn supervise(
    controller: Option<String>,
    status: watch::Sender<String>,
    stop: CancellationToken,
) -> Result<()> {
    loop {
        if stop.is_cancelled() {
            return Ok(());
        }
        // This independent worker drains registration even when stopped: a
        // late BlueZ success must be unregistered before shutdown completes.
        // Backend commands and AWDL transfers remain responsive throughout.
        let attempt = super::transport::ble_wake(controller.as_deref()).await;
        match attempt {
            Ok(mut advertisement) => {
                status.send_replace("AirDrop AWDL is ready; Bluetooth wake is active. Apple device verification pending.".into());
                tokio::select! {
                    biased;
                    _ = stop.cancelled() => {},
                    _ = advertisement.released() => {
                        status.send_replace("AirDrop AWDL remains ready. Bluetooth wake was interrupted; reconnecting automatically.".into());
                    }
                }
                // Settle the old registration before retrying on either the
                // selected controller or a newly selected automatic controller.
                if let Err(error) = advertisement.unregister().await {
                    status.send_replace(format!("AirDrop AWDL remains ready. Bluetooth wake cleanup failed: {error}. Restart sharing services to retry."));
                    stop.cancelled().await;
                    return Err(error.into());
                }
            }
            Err(error) => {
                status.send_replace(format!("AirDrop AWDL remains ready. Bluetooth wake unavailable: {error}. Retrying automatically; the Apple device's AirDrop panel can also be opened manually."));
            }
        }
        tokio::select! {
            _ = stop.cancelled() => return Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(5)) => {},
        }
    }
}

#[cfg(test)]
#[path = "bluetooth_tests.rs"]
mod tests;
