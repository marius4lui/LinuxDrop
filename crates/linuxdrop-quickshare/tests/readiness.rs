use linuxdrop_core::{BackendCommand, BackendEvent};
use linuxdrop_quickshare::{Config, start};
use std::time::Duration;
use tokio::sync::mpsc;

#[tokio::test]
async fn lan_starts_without_bluetooth_and_port_conflicts_fail() {
    let directory = tempfile::tempdir().unwrap();
    let configuration = |port| Config {
        name: "LinuxDrop readiness test".into(),
        download_dir: directory.path().to_owned(),
        visible: false,
        port,
        ble: true,
        max_receive_bytes: 1024,
        max_files: 1,
        upgrade_interface: None,
    };
    let (events, mut receiver) = mpsc::channel(128);
    let commands = start(configuration(None), events).await.unwrap();
    let BackendEvent::StateChanged(state) = receiver.recv().await.unwrap() else {
        panic!("Expected startup status");
    };
    assert_eq!(state.state, "ready");
    assert!(state.detail.contains("LAN active"));
    println!("{}", state.detail);
    // Let queued mDNS registration failures surface instead of testing only
    // the synchronous startup event.
    let deadline = tokio::time::sleep(Duration::from_millis(250));
    tokio::pin!(deadline);
    loop {
        tokio::select! {
            _=&mut deadline=>break,
            event=receiver.recv()=>if let Some(BackendEvent::StateChanged(state))=event {
                assert_ne!(state.state,"error","{}",state.detail);
            }
        }
    }
    commands.send(BackendCommand::Shutdown).await.unwrap();
    // Closing the event stream proves the actor completed engine.stop().
    tokio::time::timeout(Duration::from_secs(12), async {
        while receiver.recv().await.is_some() {}
    })
    .await
    .unwrap();

    let occupied = tokio::net::TcpListener::bind("0.0.0.0:0").await.unwrap();
    let (events, mut receiver) = mpsc::channel(128);
    assert!(
        start(
            configuration(Some(occupied.local_addr().unwrap().port())),
            events
        )
        .await
        .is_err()
    );
    assert!(
        receiver.recv().await.is_none(),
        "A failed bind must not claim readiness"
    );
}
