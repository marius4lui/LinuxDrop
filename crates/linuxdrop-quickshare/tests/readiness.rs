use linuxdrop_core::BackendEvent;
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
        upgrade_lease: None,
        p2p_connector: None,
        policy: linuxdrop_core::TransferPolicy {
            allow_virtual_interfaces: true,
            ..Default::default()
        },
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
    commands.shutdown().await.unwrap();
    // The receipt and closed event stream both confirm engine.stop completed.
    tokio::time::timeout(Duration::from_secs(12), async {
        while receiver.recv().await.is_some() {}
    })
    .await
    .unwrap();

    let mut restricted = configuration(None);
    restricted.ble = false;
    restricted.policy.allowed_interfaces = vec!["lo".into()];
    let (events, mut receiver) = mpsc::channel(128);
    let commands = start(restricted, events).await.unwrap();
    let BackendEvent::StateChanged(state) = receiver.recv().await.unwrap() else {
        panic!("Expected startup status");
    };
    assert_eq!(state.state, "unavailable");
    assert!(state.detail.contains("no enabled LAN interface"));
    commands.shutdown().await.unwrap();
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

    // Cancel startup after the real listener is bound but before the initial
    // status can enter a full event queue. Workers and staging must be owned
    // through cleanup even though no CommandSender was returned to the caller.
    let reservation = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let mut config = configuration(Some(port));
    config.ble = false;
    let (events, mut receiver) = mpsc::channel(1);
    events
        .send(BackendEvent::StateChanged(linuxdrop_core::BackendState {
            id: "test".into(),
            state: "queued".into(),
            detail: String::new(),
        }))
        .await
        .unwrap();
    let starting = tokio::spawn(async move { start(config, events).await });
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if std::net::TcpListener::bind(("0.0.0.0", port)).is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!starting.is_finished());
    starting.abort();
    assert!(starting.await.unwrap_err().is_cancelled());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let staging_remains = std::fs::read_dir(directory.path()).unwrap().any(|entry| {
                entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".linuxdrop-quickshare-")
            });
            if !staging_remains && std::net::TcpListener::bind(("0.0.0.0", port)).is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    receiver.recv().await.unwrap();
    assert!(receiver.recv().await.is_none());
}
