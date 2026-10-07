use rqs_lib::hdl::BluetoothTasks;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

struct Held(Arc<AtomicUsize>);
impl Drop for Held {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn migrated_child_survives_transport_return_but_shutdown_closes_it() {
    let owner = CancellationToken::new();
    let tasks = BluetoothTasks::new(&owner);
    let sessions = tasks.clone();
    let (mut remote, mut socket) = tokio::io::duplex(16);
    let (started, ready) = tokio::sync::oneshot::channel();
    assert!(tasks.spawn(async move {
        assert!(sessions.spawn(async move {
            socket.write_all(b"upgraded").await.unwrap();
            let mut buf = [0];
            let _ = socket.read(&mut buf).await;
        }));
        let _ = started.send(());
        // Original BLE transport ends; upgraded session remains under server ownership.
    }));
    ready.await.unwrap();
    let mut bytes = [0; 8];
    remote.read_exact(&mut bytes).await.unwrap();
    assert_eq!(&bytes, b"upgraded");
    tokio::time::timeout(Duration::from_secs(1), tasks.shutdown())
        .await
        .unwrap();
    assert_eq!(
        remote.read(&mut [0]).await.unwrap(),
        0,
        "Shutdown must drop the migrated socket before returning"
    );
    assert!(
        !tasks.spawn(async {}),
        "Late callbacks cannot reopen a drained server"
    );
    tasks.shutdown().await;
}

#[tokio::test]
async fn stalled_clients_are_bounded_and_parent_cancellation_releases_all() {
    let owner = CancellationToken::new();
    let tasks = BluetoothTasks::new(&owner);
    let dropped = Arc::new(AtomicUsize::new(0));
    let mut admitted = 0;
    for _ in 0..100 {
        let held = Held(dropped.clone());
        if tasks.spawn(async move {
            let _held = held;
            std::future::pending::<()>().await;
        }) {
            admitted += 1;
        }
    }
    assert!(
        admitted > 0 && admitted < 100,
        "Unresponsive clients cannot allocate unlimited tasks"
    );
    owner.cancel();
    assert!(!tasks.spawn(async {}));
    tokio::time::timeout(Duration::from_secs(1), tasks.shutdown())
        .await
        .unwrap();
    assert_eq!(
        dropped.load(Ordering::SeqCst),
        100,
        "Both refused and accepted requests must release owned resources"
    );
}
