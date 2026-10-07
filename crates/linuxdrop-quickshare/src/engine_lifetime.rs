//! Keep staging alive until all engine workers have released their files.
use rqs_lib::RQS;
use std::ops::{Deref, DerefMut};

pub(crate) struct OwnedEngine {
    engine: Option<RQS>,
    staging: Option<tempfile::TempDir>,
}
impl OwnedEngine {
    pub fn new(engine: RQS, staging: tempfile::TempDir) -> Self {
        Self {
            engine: Some(engine),
            staging: Some(staging),
        }
    }
    pub fn staging_path(&self) -> &std::path::Path {
        self.staging.as_ref().expect("staging is owned").path()
    }
    pub async fn stop(&mut self) {
        if let Some(engine) = self.engine.as_mut() {
            engine.stop().await;
        }
        self.engine.take();
        self.staging.take();
    }
}
impl Deref for OwnedEngine {
    type Target = RQS;
    fn deref(&self) -> &Self::Target {
        self.engine.as_ref().expect("engine is running")
    }
}
impl DerefMut for OwnedEngine {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.engine.as_mut().expect("engine is running")
    }
}
impl Drop for OwnedEngine {
    fn drop(&mut self) {
        if let Some(mut engine) = self.engine.take() {
            let staging = self.staging.take();
            // Cancelling startup must not detach a listening server or remove
            // staging underneath workers. Normal stop awaits this same order.
            tokio::spawn(async move {
                engine.stop().await;
                drop(staging);
            });
        }
    }
}
