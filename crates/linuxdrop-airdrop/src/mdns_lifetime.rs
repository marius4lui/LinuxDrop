//! Own discovery from its first fallible setup step through acknowledged exit.
use mdns_sd::{DaemonStatus, ServiceDaemon};
use std::{ops::Deref, time::Duration};
use tokio::sync::watch;

type Completion = Result<(), String>;
pub(crate) struct OwnedMdns {
    daemon: ServiceDaemon,
    cleanup: Option<watch::Receiver<Option<Completion>>>,
}
impl OwnedMdns {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            daemon: ServiceDaemon::new()?,
            cleanup: None,
        })
    }
    fn begin_shutdown(&mut self) {
        if self.cleanup.is_some() {
            return;
        }
        let daemon = self.daemon.clone();
        let initial_request = daemon.shutdown();
        let (done, receipt) = watch::channel(None);
        self.cleanup = Some(receipt);
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            // The worker owns cleanup even if the caller's startup/stop future
            // is cancelled. A full command queue requires retry, not a leak.
            runtime.spawn(async move {
                let result = tokio::time::timeout(Duration::from_secs(10), async {
                    let mut request = initial_request;
                    let receipt = loop {
                        match request {
                            Ok(receipt) => break receipt,
                            Err(mdns_sd::Error::Again) => {
                                tokio::time::sleep(Duration::from_millis(25)).await;
                                request = daemon.shutdown();
                            }
                            Err(error) => return Err(error.to_string()),
                        }
                    };
                    match receipt.recv_async().await.map_err(|e| e.to_string())? {
                        DaemonStatus::Shutdown => Ok(()),
                        _ => Err("Discovery did not confirm shutdown".into()),
                    }
                })
                .await
                .unwrap_or_else(|_| Err("Discovery shutdown timed out".into()));
                done.send_replace(Some(result));
            });
        } else {
            // Runtime teardown cannot await the acknowledgement, but still
            // asks the independent mDNS thread to release its sockets.
            let result = match initial_request {
                Ok(_) => Err(
                    "Discovery shutdown requested without a runtime to await confirmation".into(),
                ),
                Err(error) => Err(error.to_string()),
            };
            done.send_replace(Some(result));
        }
    }
    pub async fn stop(&mut self) -> Completion {
        self.begin_shutdown();
        let receipt = self.cleanup.as_mut().unwrap();
        loop {
            if let Some(result) = receipt.borrow().clone() {
                return result;
            }
            receipt
                .changed()
                .await
                .map_err(|_| "Discovery cleanup worker stopped".to_string())?;
        }
    }
}
impl Deref for OwnedMdns {
    type Target = ServiceDaemon;
    fn deref(&self) -> &Self::Target {
        &self.daemon
    }
}
impl Drop for OwnedMdns {
    fn drop(&mut self) {
        self.begin_shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn stopped(observer: &ServiceDaemon) {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                // A status command racing Exit can be queued after it and
                // never answered. Re-query after a bounded observation window.
                if let Ok(receipt) = observer.status()
                    && matches!(
                        tokio::time::timeout(Duration::from_millis(100), receipt.recv_async())
                            .await,
                        Ok(Ok(DaemonStatus::Shutdown))
                    )
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn failed_setup_and_cancelled_shutdown_both_stop_the_real_daemon() {
        let daemon = OwnedMdns::new().unwrap();
        let observer = daemon.daemon.clone();
        daemon.disable_interface(mdns_sd::IfKind::All).unwrap();
        assert!(daemon.browse("invalid-service-type").is_err());
        drop(daemon);
        stopped(&observer).await;

        let mut daemon = OwnedMdns::new().unwrap();
        let observer = daemon.daemon.clone();
        {
            let future = daemon.stop();
            tokio::pin!(future);
            assert!(futures_util::poll!(future.as_mut()).is_pending());
        }
        drop(daemon);
        stopped(&observer).await;

        let mut daemon = OwnedMdns::new().unwrap();
        daemon.stop().await.unwrap();
        daemon.stop().await.unwrap();
    }
}
