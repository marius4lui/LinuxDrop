//! Recover the optional Apple wake advertisement independently of AWDL traffic.
use anyhow::Result;
use linuxdrop_network::bluetooth_airtime::{BleScanSuppressor, Turn};
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
        let mut turn = match Turn::acquire(true, &stop).await {
            Ok(Some(turn)) => turn,
            Ok(None) => return Ok(()),
            Err(error) => {
                status.send_replace(format!("AirDrop AWDL remains ready. Bluetooth wake is paused: {error}. Restart LinuxDrop's background service after cleanup."));
                stop.cancelled().await;
                return Err(error);
            }
        };
        // Short wake windows leave the same controller available for Quick
        // Share's connectable receiver and sender discovery. Its receiver only
        // yields between connections; no live GATT/L2CAP session is preempted.
        let suppress_scan = BleScanSuppressor::new();
        turn.registering();
        // This independent worker drains registration even when stopped: a
        // late BlueZ success must be unregistered before shutdown completes.
        // Backend commands and AWDL transfers remain responsive throughout.
        let attempt = super::transport::ble_wake(controller.as_deref()).await;
        match attempt {
            Ok(mut advertisement) => {
                status.send_if_modified(|detail| {
                    let ready = "AirDrop AWDL is ready; Bluetooth wake uses shared discovery windows. Apple device verification pending.";
                    if detail == ready { return false; }
                    *detail = ready.into();
                    true
                });
                tokio::select! {
                    biased;
                    _ = stop.cancelled() => {},
                    _ = advertisement.released() => {
                        status.send_replace("AirDrop AWDL remains ready. Bluetooth wake was interrupted; reconnecting automatically.".into());
                    },
                    _ = tokio::time::sleep(Duration::from_secs(3)) => {},
                }
                // Settle the old registration before retrying on either the
                // selected controller or a newly selected automatic controller.
                if let Err(error) = advertisement.unregister().await {
                    status.send_replace(format!("AirDrop AWDL remains ready. Bluetooth wake cleanup failed: {error}. Restart sharing services to retry."));
                    stop.cancelled().await;
                    return Err(error.into());
                }
                turn.cleared();
            }
            Err(error) => {
                // The BlueR registration worker reports errors only after
                // confirming removal of any partially registered object.
                turn.cleared();
                status.send_replace(format!("AirDrop AWDL remains ready. Bluetooth wake unavailable: {error}. Retrying automatically; the Apple device's AirDrop panel can also be opened manually."));
            }
        }
        drop(suppress_scan);
        drop(turn);
        tokio::select! {
            _ = stop.cancelled() => return Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(5)) => {},
        }
    }
}

#[cfg(test)]
#[path = "bluetooth_tests.rs"]
mod tests;
