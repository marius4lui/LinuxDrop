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
    let ipv6 = listeners
        .iter()
        .map(|listener| listener.local_addr().unwrap())
        .find(|address| address.ip() == "::1".parse::<IpAddr>().unwrap())
        .expect("IPv6 loopback listener");
    let ipv6_client = lan_policy::connect(ipv6).await.unwrap();
    let (ipv6_server, _) = lan_policy::accept(&listeners).await.unwrap();
    assert!(ipv6_client.local_addr().unwrap().is_ipv6());
    assert!(ipv6_server.local_addr().unwrap().is_ipv6());
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
        .map(|interface| interface.address)
        .collect();
    assert_eq!(service.get_addresses(), &allowed);
    assert!(!service.is_addr_auto());
}

#[test]
fn discovery_scopes_link_local_peers_to_enabled_adapters() {
    use linuxdrop_network::InterfaceAddress;
    let first = InterfaceAddress {
        name: "wlan0".into(),
        address: "fe80::1".parse().unwrap(),
        netmask: "ffff:ffff:ffff:ffff::".parse().unwrap(),
        index: 7,
        loopback: false,
    };
    let second = InterfaceAddress {
        name: "wlan1".into(),
        index: 8,
        ..first.clone()
    };
    let candidates = lan_policy::discovery_candidates(
        "fe80::abcd".parse().unwrap(),
        53318,
        &[first.clone(), second.clone()],
    );
    assert_eq!(
        candidates,
        vec![
            "[fe80::abcd%7]:53318".parse().unwrap(),
            "[fe80::abcd%8]:53318".parse().unwrap()
        ]
    );
    assert_eq!(
        lan_policy::source_for_on(candidates[1], &[first.clone(), second.clone()]).unwrap(),
        second
    );
    assert!(
        lan_policy::source_for_on(
            "[fe80::abcd]:53318".parse().unwrap(),
            std::slice::from_ref(&first)
        )
        .is_err()
    );
    assert!(lan_policy::source_for_on(candidates[1], &[first]).is_err());
    for address in candidates {
        let std::net::SocketAddr::V6(v6) = address else {
            unreachable!()
        };
        let endpoint = rqs_lib::EndpointInfo {
            ip: Some(format!("{}%{}", v6.ip(), v6.scope_id())),
            port: Some(v6.port().to_string()),
            ..Default::default()
        };
        assert_eq!(endpoint.socket_address().unwrap(), address);
    }
}

#[test]
fn wifi_lan_wire_candidates_replace_legacy_and_preserve_ip_family_order() {
    let candidates = vec![
        "192.168.4.2:5000".parse().unwrap(),
        "[fd42::2]:5000".parse().unwrap(),
    ];
    let mut offer = lan_policy::upgrade_offer(&candidates).unwrap();
    assert_eq!(offer.ip_address(), &[192, 168, 4, 2]);
    assert_eq!(
        lan_policy::upgrade_candidates(&offer).unwrap(),
        vec![candidates[1], candidates[0]]
    );
    offer.ip_address = Some(vec![1]); // ignored when the new list is present
    assert!(lan_policy::upgrade_candidates(&offer).is_ok());
    offer.address_candidates[0].port = Some(65536);
    assert!(lan_policy::upgrade_candidates(&offer).is_err());
    offer.address_candidates.clear();
    offer.ip_address = Some(lan_policy::address_bytes("fd42::2".parse().unwrap()));
    assert_eq!(
        lan_policy::upgrade_candidates(&offer).unwrap(),
        vec![candidates[1]]
    );
    for invalid in [
        "fe80::2",
        "::",
        "ff02::1",
        "::ffff:192.168.4.2",
        "127.0.0.1",
        "169.254.1.2",
    ] {
        offer.ip_address = Some(lan_policy::address_bytes(invalid.parse().unwrap()));
        assert!(lan_policy::upgrade_candidates(&offer).is_err(), "{invalid}");
    }
}
