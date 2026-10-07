use linuxdrop_core::TransferPolicy;
use rqs_lib::lan_policy::{self, LanListeners};
use std::{net::SocketAddr, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::test]
async fn listener_updates_preserve_sessions_and_advertise_only_bound_addresses() {
    lan_policy::set(TransferPolicy {
        allowed_interfaces: vec!["lo".into()],
        ..Default::default()
    });
    let mut listeners = LanListeners::new(0).await.unwrap();
    let mut updates = listeners.subscribe();
    let first = updates
        .borrow()
        .interfaces
        .iter()
        .find(|entry| entry.address.is_ipv4() && entry.address.is_loopback())
        .unwrap()
        .clone();
    let added = linuxdrop_network::InterfaceAddress {
        address: "127.0.0.2".parse().unwrap(),
        ..first.clone()
    };
    let unavailable = linuxdrop_network::InterfaceAddress {
        address: "192.0.2.254".parse().unwrap(),
        name: "no-such-interface".into(),
        loopback: false,
        ..first.clone()
    };
    let mut client =
        tokio::net::TcpStream::connect(SocketAddr::new(first.address, listeners.port()))
            .await
            .unwrap();
    let (mut server, _) = listeners.accept().await.unwrap();
    listeners
        .reconcile(vec![first.clone(), added.clone(), unavailable])
        .await;
    updates.changed().await.unwrap();
    let snapshot = updates.borrow_and_update().clone();
    assert_eq!(snapshot.interfaces.len(), 2);
    assert_eq!(snapshot.errors.len(), 1);
    let metadata = rqs_lib::hdl::MDnsServer::build_service_on(
        *b"TEST",
        listeners.port(),
        rqs_lib::DeviceType::Laptop,
        &snapshot.interfaces,
    )
    .unwrap();
    assert!(
        metadata.get_addresses().is_empty(),
        "Neither loopback aliases nor failed listener binds are advertised"
    );
    let _second = tokio::net::TcpStream::connect(SocketAddr::new(added.address, listeners.port()))
        .await
        .unwrap();
    listeners.reconcile(vec![first.clone()]).await;
    assert!(
        tokio::net::TcpStream::connect(SocketAddr::new(added.address, listeners.port()))
            .await
            .is_err()
    );
    client.write_all(b"still connected").await.unwrap();
    let mut received = [0; 15];
    tokio::time::timeout(Duration::from_secs(1), server.read_exact(&mut received))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&received, b"still connected");
    listeners.reconcile(vec![]).await;
    assert!(!updates.borrow().available());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listeners.accept())
            .await
            .is_err()
    );
    listeners.reconcile(vec![added.clone()]).await;
    assert!(
        tokio::net::TcpStream::connect(SocketAddr::new(added.address, listeners.port()))
            .await
            .is_ok()
    );
}

#[tokio::test]
#[ignore = "Requires the isolated network namespace created by run-lan-lifecycle.sh"]
async fn running_engine_follows_real_address_and_link_changes() {
    assert_eq!(
        std::env::var("LINUXDROP_TEST_PRIVATE_LAN").as_deref(),
        Ok("1")
    );
    assert_ne!(
        std::fs::read_link("/proc/self/ns/net").unwrap(),
        std::fs::read_link("/proc/1/ns/net").unwrap()
    );
    fn ip(args: &[&str]) {
        assert!(
            std::process::Command::new("ip")
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    ip(&["link", "set", "lo", "up"]);
    ip(&["link", "add", "ld-test0", "type", "dummy"]);
    ip(&["addr", "add", "198.18.0.1/24", "dev", "ld-test0"]);
    ip(&["link", "set", "ld-test0", "up"]);
    ip(&[
        "-6",
        "addr",
        "add",
        "fd42:1234::1/64",
        "dev",
        "ld-test0",
        "nodad",
    ]);
    lan_policy::set(TransferPolicy {
        allowed_interfaces: vec!["ld-test0".into()],
        ..Default::default()
    });
    let root = tempfile::tempdir().unwrap();
    let mut engine = rqs_lib::RQS::new(
        rqs_lib::Visibility::Visible,
        None,
        Some(root.path().into()),
        Some("Private network test".into()),
    );
    engine.ble_enabled = false;
    engine.run().await.unwrap();
    let mut updates = engine.lan_state().unwrap();
    let (discovery, _discovery_rx) = tokio::sync::broadcast::channel(32);
    engine.discovery(discovery).unwrap();
    let mut status = engine.message_sender.subscribe();
    let port = engine.port_number.unwrap() as u16;
    assert!(updates.borrow().available());
    assert!(
        tokio::net::TcpStream::connect(("198.18.0.1", port))
            .await
            .is_ok()
    );
    let ipv6: SocketAddr = format!("[fd42:1234::1]:{port}").parse().unwrap();
    assert!(lan_policy::connect(ipv6).await.is_ok());
    let offer = lan_policy::upgrade_offer(&[ipv6]).unwrap();
    let candidates = lan_policy::upgrade_candidates(&offer).unwrap();
    assert!(lan_policy::connect_candidates(&candidates).await.is_ok());
    let mdns = rqs_lib::hdl::MDnsServer::build_service_on(
        *b"TEST",
        port,
        rqs_lib::DeviceType::Laptop,
        &updates.borrow().interfaces,
    )
    .unwrap();
    assert!(mdns.get_addresses().contains(&ipv6.ip()));
    // Remove IPv4 entirely: the running engine must keep its IPv6 LAN ready.
    ip(&["addr", "del", "198.18.0.1/24", "dev", "ld-test0"]);
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            updates.changed().await.unwrap();
            if !updates
                .borrow_and_update()
                .interfaces
                .iter()
                .any(|local| !local.loopback && local.address.is_ipv4())
            {
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!(updates.borrow().available());
    assert!(lan_policy::connect(ipv6).await.is_ok());
    ip(&["addr", "add", "198.18.1.1/24", "dev", "ld-test0"]);
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            updates.changed().await.unwrap();
            if updates
                .borrow_and_update()
                .interfaces
                .iter()
                .any(|entry| entry.address.to_string() == "198.18.1.1")
            {
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!(
        tokio::net::TcpStream::connect(("198.18.1.1", port))
            .await
            .is_ok()
    );
    ip(&["link", "set", "ld-test0", "down"]);
    tokio::time::timeout(Duration::from_secs(8), async {
        while updates.borrow().available() {
            updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    ip(&["link", "set", "ld-test0", "up"]);
    tokio::time::timeout(Duration::from_secs(8), async {
        while !updates.borrow().available() {
            updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    assert!(
        tokio::net::TcpStream::connect(("198.18.1.1", port))
            .await
            .is_ok()
    );
    // Give the shared snapshot consumers time to re-register and re-browse.
    tokio::time::sleep(Duration::from_millis(250)).await;
    while let Ok(message) = status.try_recv() {
        assert!(
            !matches!(message.msg, rqs_lib::channel::Message::Backend { .. }),
            "{message:?}"
        );
    }
    tokio::time::timeout(Duration::from_secs(5), engine.stop())
        .await
        .unwrap();
}
