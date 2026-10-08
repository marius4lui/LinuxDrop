//! Bounded ownership for server callbacks and sessions that can migrate to Wi-Fi.
use std::future::Future;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

const MAX_TASKS: usize = 32;

#[derive(Clone)]
pub struct BluetoothTasks {
    inner: Arc<Inner>,
}
struct Inner {
    accepting: Mutex<bool>,
    tracker: TaskTracker,
    cancel: CancellationToken,
}
impl BluetoothTasks {
    pub fn new(parent: &CancellationToken) -> Self {
        Self {
            inner: Arc::new(Inner {
                accepting: Mutex::new(true),
                tracker: TaskTracker::new(),
                cancel: parent.child_token(),
            }),
        }
    }

    pub fn cancellation(&self) -> CancellationToken {
        self.inner.cancel.clone()
    }

    /// Register before spawning, under the same gate used to close admission.
    /// The task's resources are dropped before shutdown acknowledges completion.
    pub fn spawn(&self, future: impl Future<Output = ()> + Send + 'static) -> bool {
        let accepting = self.inner.accepting.lock().unwrap();
        if !*accepting || self.inner.cancel.is_cancelled() || self.inner.tracker.len() >= MAX_TASKS
        {
            return false;
        }
        let cancel = self.inner.cancel.clone();
        self.inner.tracker.spawn(async move {
            tokio::select! {
                biased;
                _ = cancel.cancelled() => {},
                _ = future => {},
            }
        });
        true
    }

    pub async fn shutdown(&self) {
        {
            let mut accepting = self.inner.accepting.lock().unwrap();
            *accepting = false;
            self.inner.cancel.cancel();
            self.inner.tracker.close();
        }
        self.inner.tracker.wait().await;
    }
}
