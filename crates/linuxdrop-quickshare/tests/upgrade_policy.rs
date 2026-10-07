use linuxdrop_network::{DirectWifiCapabilities, P2pConnector, P2pHosted};
use prost::Message;
use rqs_lib::hdl::{self, OutboundRequest};
use rqs_lib::location_nearby_connections::bandwidth_upgrade_negotiation_frame::upgrade_path_info::Medium;
use rqs_lib::location_nearby_connections::medium_metadata::WifiDirectAuthType;
use rqs_lib::location_nearby_connections::{
    MediumMetadata, MediumRole, OfflineFrame, ServiceAddress,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::io::AsyncReadExt;

#[test]
fn role_selection_respects_radio_peer_and_authentication() {
    let none = DirectWifiCapabilities::default();
    let direct_only = DirectWifiCapabilities {
        station: true,
        p2p_group_owner: true,
        frequencies: vec![2437],
        ..Default::default()
    };
    let both = DirectWifiCapabilities {
        hotspot: true,
        frequencies: vec![2437, 5180, 6115],
        ..direct_only.clone()
    };
    let metadata = hdl::metadata_for(&none, None);
    assert!(!metadata.supports_5_ghz());
    assert!(
        !metadata
            .medium_role
            .unwrap()
            .support_wifi_direct_group_owner()
    );
    assert!(metadata.supported_wifi_direct_auth_types.is_empty());
    assert!(!hdl::metadata_for(&direct_only, None).supports_5_ghz());
    assert!(hdl::metadata_for(&both, None).supports_5_ghz());
    assert!(hdl::metadata_for(&both, None).supports_6_ghz());
    let mediums = [Medium::WifiDirect as i32, Medium::WifiHotspot as i32];
    assert_eq!(hdl::host_medium_for(&none, &mediums, None), None);
    assert_eq!(
        hdl::host_medium_for(&both, &mediums, None),
        Some(Medium::WifiDirect)
    );
    assert_eq!(hdl::host_medium_for(&both, &[], None), None);
    let mut peer = MediumMetadata {
        medium_role: Some(MediumRole {
            support_wifi_direct_group_client: Some(false),
            support_wifi_hotspot_client: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(
        hdl::host_medium_for(&both, &mediums, Some(&peer)),
        Some(Medium::WifiHotspot)
    );
    assert_eq!(
        hdl::host_medium_for(&direct_only, &mediums, Some(&peer)),
        None
    );
    peer.medium_role = None;
    peer.supported_wifi_direct_auth_types =
        vec![WifiDirectAuthType::WifiDirectWithDeviceName as i32];
    assert_eq!(
        hdl::host_medium_for(&both, &mediums, Some(&peer)),
        Some(Medium::WifiHotspot)
    );
    assert_eq!(
        hdl::host_medium_for(&direct_only, &mediums, Some(&peer)),
        None,
        "A device-name-only peer cannot receive a password-only offer"
    );
    peer.supported_wifi_direct_auth_types = vec![999];
    assert_eq!(
        hdl::host_medium_for(&direct_only, &mediums, Some(&peer)),
        None
    );
    peer.supported_wifi_direct_auth_types
        .push(WifiDirectAuthType::WifiDirectWithPassword as i32);
    assert_eq!(
        hdl::host_medium_for(&direct_only, &mediums, Some(&peer)),
        Some(Medium::WifiDirect)
    );
}

struct Helper(Arc<AtomicUsize>);
impl P2pConnector for Helper {
    fn host(&self) -> BoxFuture<'_, anyhow::Result<P2pHosted>> {
        Box::pin(async {
            Ok(P2pHosted {
                interface: "testp2p0".into(),
                ssid: "DIRECT-from-supplicant".into(),
                password: "test-password".into(),
                frequency: 2437,
                ipv4_address: "192.168.49.1".parse().unwrap(),
                ipv6_address: Some("fe80::1234".parse().unwrap()),
            })
        })
    }
    fn connect(
        &self,
        _: String,
        _: String,
        _: u32,
    ) -> BoxFuture<'_, anyhow::Result<linuxdrop_network::P2pConnection>> {
        Box::pin(async { anyhow::bail!("Not a join fixture") })
    }
    fn disconnect(&self) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async move {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}
// The trait's standard Future alias; no additional test dependency.
type BoxFuture<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

#[tokio::test]
async fn connection_request_and_owned_group_offer_use_actual_capabilities() {
    hdl::set_upgrade_lease(None);
    hdl::set_p2p_connector(None);
    assert_eq!(hdl::upgrade_mediums(), vec![5, 10]);
    let releases = Arc::new(AtomicUsize::new(0));
    hdl::set_upgrade_lease(Some((
        linuxdrop_network::nm::Lease {
            interface: "reserved0".into(),
            lease_id: "test-lease".into(),
            connection_uuid: "test-uuid".into(),
        },
        DirectWifiCapabilities {
            station: true,
            p2p_group_owner: true,
            p2p_client: true,
            frequencies: vec![2437],
            ..Default::default()
        },
    )));
    assert!(
        !hdl::upgrade_metadata()
            .medium_role
            .unwrap()
            .support_wifi_direct_group_owner(),
        "Missing helper must not advertise P2P hosting"
    );
    hdl::set_p2p_connector(Some(Arc::new(Helper(releases.clone()))));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let socket = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (mut peer, _) = listener.accept().await.unwrap();
    let (events, _) = tokio::sync::broadcast::channel(8);
    let mut request = OutboundRequest::new(
        *b"TEST",
        socket,
        "capability-wire-test".into(),
        events,
        rqs_lib::OutboundPayload::Files(vec![]),
        rqs_lib::utils::RemoteDeviceInfo {
            name: "Peer".into(),
            device_type: rqs_lib::DeviceType::Phone,
        },
    );
    request.set_mediums(hdl::upgrade_mediums());
    request.send_connection_request().await.unwrap();
    let len = peer.read_u32().await.unwrap() as usize;
    assert!(len < 4096);
    let mut bytes = vec![0; len];
    peer.read_exact(&mut bytes).await.unwrap();
    let decoded = OfflineFrame::decode(bytes.as_slice())
        .unwrap()
        .v1
        .unwrap()
        .connection_request
        .unwrap();
    assert_eq!(decoded.mediums, vec![5, 8, 3, 10]);
    let metadata = decoded.medium_metadata.unwrap();
    assert!(!metadata.supports_5_ghz());
    let roles = metadata.medium_role.unwrap();
    assert!(roles.support_wifi_direct_group_owner() && roles.support_wifi_direct_group_client());
    assert!(!roles.support_wifi_hotspot_host());
    assert!(roles.support_wifi_hotspot_client());
    let guard = hdl::start_direct_group().await.unwrap();
    assert!(
        hdl::start_direct_group().await.is_err(),
        "Concurrent transfers cannot share a changing radio"
    );
    let ipv6: std::net::Ipv6Addr = "fe80::1234".parse().unwrap();
    let offer = guard
        .offer(
            61812,
            vec![ServiceAddress {
                ip_address: Some(ipv6.octets().to_vec()),
                port: Some(61812),
            }],
            Medium::WifiDirect,
        )
        .unwrap();
    assert!(offer.wifi_hotspot_credentials.is_none());
    let direct = offer.wifi_direct_credentials.unwrap();
    assert_eq!(direct.ssid(), "DIRECT-from-supplicant");
    assert_eq!(direct.frequency(), 2437);
    assert_eq!(hdl::direct_candidates(&direct).unwrap().len(), 2);
    assert!(
        guard.offer(61812, vec![], Medium::WifiHotspot).is_err(),
        "A P2P group is never relabelled as an ordinary hotspot"
    );
    drop(guard);
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while releases.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    hdl::set_upgrade_lease(None);
    hdl::set_p2p_connector(None);
    assert!(
        !hdl::upgrade_metadata()
            .medium_role
            .unwrap()
            .support_wifi_direct_group_owner()
    );
}
