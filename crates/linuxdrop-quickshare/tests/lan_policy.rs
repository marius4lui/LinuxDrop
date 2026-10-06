use linuxdrop_core::TransferPolicy;
use rqs_lib::lan_policy;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};

#[tokio::test]
async fn lan_binds_only_enabled_addresses_and_rejects_routes_outside_them() {
    lan_policy::set(TransferPolicy {
        allowed_interfaces: vec!["lo".into()],
        ..Default::default()
    });
    let listeners = lan_policy::listeners(0).await.unwrap();
    assert!(!listeners.is_empty());
    let enabled = lan_policy::interfaces(true).unwrap();
    assert!(enabled.iter().all(|interface| interface.name == "lo"));
    // WSL also places a non-127/8 DNS alias on lo. Check actual interface
    // ownership, not the spelling of all addresses assigned to that interface.
    assert!(listeners.iter().all(|listener| {
        enabled
            .iter()
            .any(|interface| interface.address == listener.local_addr().unwrap().ip())
    }));
    let address = listeners
        .iter()
        .map(|listener| listener.local_addr().unwrap())
        .find(|address| address.ip().is_loopback())
        .unwrap();
    let client = lan_policy::connect(address).await.unwrap();
    let (server, peer) =
        tokio::time::timeout(Duration::from_secs(1), lan_policy::accept(&listeners))
            .await
            .unwrap()
            .unwrap();
    assert!(client.local_addr().unwrap().ip().is_loopback());
    assert!(lan_policy::permits(
        server.local_addr().unwrap().ip(),
        peer.ip()
    ));
    assert!(
        lan_policy::connect(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
            address.port()
        ))
        .await
        .is_err()
    );
    assert!(!lan_policy::permits(
        address.ip(),
        "192.0.2.1".parse().unwrap()
    ));
    assert!(lan_policy::source_for("224.0.0.1".parse().unwrap()).is_err());
    assert!(rqs_lib::utils::local_ipv4().is_none());
    let service = rqs_lib::hdl::MDnsServer::build_service(
        *b"TEST",
        address.port(),
        rqs_lib::DeviceType::Laptop,
    )
    .unwrap();
    assert!(
        service.get_addresses().is_empty(),
        "Loopback and excluded interfaces must not leak through mDNS auto-addressing"
    );
    assert!(!service.is_addr_auto());
    // A forbidden local interface must not have an open listener, even when a
    // remote endpoint can route to the machine. Probe only this machine's IPs.
    for interface in linuxdrop_network::interfaces(
        &TransferPolicy {
            allow_virtual_interfaces: true,
            ..Default::default()
        },
        false,
    )
    .unwrap()
    {
        if interface.address.is_ipv4() {
            assert!(
                tokio::time::timeout(
                    Duration::from_secs(1),
                    tokio::net::TcpStream::connect(SocketAddr::new(
                        interface.address,
                        address.port()
                    ))
                )
                .await
                .unwrap()
                .is_err()
            );
        }
    }
    lan_policy::set(TransferPolicy {
        allow_virtual_interfaces: true,
        ..Default::default()
    });
    let service = rqs_lib::hdl::MDnsServer::build_service(
        *b"TEST",
        address.port(),
        rqs_lib::DeviceType::Laptop,
    )
    .unwrap();
    let allowed: std::collections::HashSet<_> = lan_policy::interfaces(false)
        .unwrap()
        .into_iter()
        .filter(|interface| interface.address.is_ipv4())
        .map(|interface| interface.address)
        .collect();
    assert_eq!(service.get_addresses(), &allowed);
    assert!(!service.is_addr_auto());
}
