//! Persistent worker cleanup results, independent of transient service errors.
use futures::FutureExt;
use std::{
    fmt,
    future::Future,
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex},
};
use tokio_util::task::TaskTracker;

#[derive(Debug)]
struct CleanupFailure(String);
impl fmt::Display for CleanupFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Unconfirmed cleanup: {}", self.0)
    }
}
impl std::error::Error for CleanupFailure {}

pub fn cleanup_failure(error: impl fmt::Display) -> anyhow::Error {
    anyhow::Error::new(CleanupFailure(error.to_string()))
}

/// Keep operational errors recoverable, but never hide failed cleanup behind
/// the earlier error which caused the worker to stop.
pub fn finish(operation: anyhow::Result<()>, cleanup: anyhow::Result<()>) -> anyhow::Result<()> {
    match (operation, cleanup) {
        (Err(operation), Err(cleanup)) => Err(cleanup_failure(format!(
            "{cleanup:#}; service failed: {operation:#}"
        ))),
        (_, Err(cleanup)) => Err(cleanup_failure(format!("{cleanup:#}"))),
        (operation, Ok(())) => operation,
    }
}

#[derive(Clone, Debug, Default)]
pub struct Workers {
    tasks: TaskTracker,
    failures: Arc<Mutex<Vec<String>>>,
    events: Option<tokio::sync::broadcast::Sender<crate::channel::ChannelMessage>>,
}
impl Workers {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_events(
        events: tokio::sync::broadcast::Sender<crate::channel::ChannelMessage>,
    ) -> Self {
        Self {
            events: Some(events),
            ..Self::default()
        }
    }
    pub fn spawn(
        &self,
        name: &'static str,
        future: impl Future<Output = anyhow::Result<()>> + Send + 'static,
    ) {
        let failures = self.failures.clone();
        let events = self.events.clone();
        self.tasks.spawn(async move {
            let failure = match AssertUnwindSafe(future).catch_unwind().await {
                Ok(Err(error)) if error.downcast_ref::<CleanupFailure>().is_some() => {
                    Some(format!("{name}: {error:#}"))
                }
                Err(_) => Some(format!("{name}: worker panicked before confirming cleanup")),
                _ => None,
            };
            if let Some(failure) = failure {
                failures.lock().unwrap().push(failure.clone());
                if let Some(events) = events {
                    let _ = events.send(crate::channel::ChannelMessage {
                        id: "backend".into(),
                        msg: crate::channel::Message::Backend {
                            component: name.into(),
                            detail: failure,
                        },
                    });
                }
            }
        });
    }
    pub fn close(&self) {
        self.tasks.close();
    }
    pub async fn wait(&self) -> Result<(), String> {
        self.tasks.wait().await;
        let mut failures = self.failures.lock().unwrap().clone();
        failures.sort();
        if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
        }
    }
}
