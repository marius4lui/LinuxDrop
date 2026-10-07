use rqs_lib::lifecycle::{Workers, cleanup_failure, finish};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

#[tokio::test]
async fn shutdown_waits_for_all_workers_and_retains_failures_after_cancelled_wait() {
    let workers = Workers::new();
    let (release, waiting) = tokio::sync::oneshot::channel();
    let finished = Arc::new(AtomicBool::new(false));
    let drained = finished.clone();
    workers.spawn("mdns", async {
        finish(
            Err(anyhow::anyhow!("browse failed")),
            Err(anyhow::anyhow!("exit refused")),
        )
    });
    workers.spawn("bluetooth", async move {
        waiting.await.unwrap();
        drained.store(true, Ordering::SeqCst);
        Ok(())
    });
    workers.close();
    assert!(
        tokio::time::timeout(Duration::from_millis(10), workers.wait())
            .await
            .is_err()
    );
    assert!(!finished.load(Ordering::SeqCst));
    release.send(()).unwrap();
    let error = workers.wait().await.unwrap_err();
    assert!(finished.load(Ordering::SeqCst));
    assert!(
        error.contains("mdns") && error.contains("exit refused") && error.contains("browse failed")
    );
    assert_eq!(workers.wait().await.unwrap_err(), error);
}

#[tokio::test]
async fn panicked_workers_and_contextual_cleanup_failures_cannot_report_success() {
    let (events, mut observed) = tokio::sync::broadcast::channel(4);
    let workers = Workers::with_events(events);
    workers.spawn("panic-test", async { panic!("simulated worker panic") });
    workers.spawn("gatt", async {
        Err(cleanup_failure("unregister refused").context("stopping service"))
    });
    workers.close();
    let error = workers.wait().await.unwrap_err();
    assert!(error.contains("panic-test") && error.contains("worker panicked"));
    assert!(error.contains("gatt") && error.contains("unregister refused"));
    let mut failed_components = Vec::new();
    for _ in 0..2 {
        let event = observed.try_recv().unwrap();
        let rqs_lib::channel::Message::Backend { component, .. } = event.msg else {
            panic!("Expected backend failure")
        };
        failed_components.push(component);
    }
    failed_components.sort();
    assert_eq!(failed_components, ["gatt", "panic-test"]);

    let recovered = Workers::new();
    recovered.spawn("unavailable-adapter", async {
        finish(Err(anyhow::anyhow!("powered off")), Ok(()))
    });
    recovered.close();
    recovered.wait().await.unwrap();
}
