//! The mDNS worker is an independent OS thread, not a tracked Tokio task.
use mdns_sd::{DaemonStatus, ServiceDaemon};
use std::time::Duration;

pub async fn shutdown(daemon: &ServiceDaemon) -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let receipt = loop {
            match daemon.shutdown() {
                Ok(receipt) => break receipt,
                Err(mdns_sd::Error::Again) => tokio::time::sleep(Duration::from_millis(25)).await,
                Err(error) => return Err(error.into()),
            }
        };
        match receipt.recv_async().await? {
            DaemonStatus::Shutdown => Ok(()),
            _ => anyhow::bail!("mDNS did not confirm shutdown"),
        }
    })
    .await?
}

pub fn abandoned(daemon: &ServiceDaemon) {
    // Submit before spawning: runtime teardown can discard unpolled tasks.
    // Only a full command queue needs the asynchronous retry worker.
    if !matches!(daemon.shutdown(), Err(mdns_sd::Error::Again)) {
        return;
    }
    let daemon = daemon.clone();
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        runtime.spawn(async move {
            if let Err(error) = shutdown(&daemon).await {
                warn!("Abandoned mDNS cleanup failed: {error}");
            }
        });
    } else {
        let _ = daemon.shutdown();
    }
}
