//! Capability and peer-role policy shared by both bandwidth-upgrade directions.
use crate::location_nearby_connections::bandwidth_upgrade_negotiation_frame::upgrade_path_info::Medium;
use crate::location_nearby_connections::medium_metadata::WifiDirectAuthType;
use crate::location_nearby_connections::{MediumMetadata, MediumRole};
use linuxdrop_network::DirectWifiCapabilities;

fn local_capabilities() -> DirectWifiCapabilities {
    #[cfg(all(feature = "experimental", target_os = "linux"))]
    {
        super::upgrade_capabilities()
    }
    #[cfg(not(all(feature = "experimental", target_os = "linux")))]
    {
        DirectWifiCapabilities::default()
    }
}

pub fn upgrade_metadata() -> MediumMetadata {
    metadata_for(&local_capabilities(), crate::utils::local_lan_ip())
}

/// Do not claim a radio-changing role merely because the protocol engine exists.
pub fn metadata_for(cap: &DirectWifiCapabilities, ip: Option<std::net::IpAddr>) -> MediumMetadata {
    MediumMetadata {
        supports_5_ghz: Some(cap.frequencies.iter().any(|f| (4900..5900).contains(f))),
        supports_6_ghz: Some(cap.frequencies.iter().any(|f| (5925..=7125).contains(f))),
        ip_address: ip.map(crate::lan_policy::address_bytes),
        ap_frequency: Some(-1),
        medium_role: Some(MediumRole {
            support_wifi_direct_group_owner: Some(cap.p2p_group_owner),
            // Password-authenticated group joining uses a managed station.
            support_wifi_direct_group_client: Some(cap.station),
            support_wifi_hotspot_host: Some(cap.hotspot),
            support_wifi_hotspot_client: Some(cap.station),
            ..Default::default()
        }),
        supported_wifi_direct_auth_types: if cap.station || cap.p2p_group_owner {
            vec![WifiDirectAuthType::WifiDirectWithPassword.into()]
        } else {
            vec![]
        },
        ..Default::default()
    }
}

pub fn upgrade_mediums() -> Vec<i32> {
    let cap = local_capabilities();
    let mut mediums = vec![Medium::WifiLan as i32];
    if cap.station || cap.p2p_group_owner {
        mediums.push(Medium::WifiDirect as i32);
    }
    if cap.station || cap.hotspot {
        mediums.push(Medium::WifiHotspot as i32);
    }
    mediums.push(10); // BLE_L2CAP, the current transport.
    mediums
}

pub fn select_host_medium(mediums: &[i32], metadata: Option<&MediumMetadata>) -> Option<Medium> {
    host_medium_for(&local_capabilities(), mediums, metadata)
}

/// Respect explicit peer exclusions; missing role/auth metadata remains compatible
/// with older password-based peers. Unknown/auth-only-device-name is not password.
pub fn host_medium_for(
    cap: &DirectWifiCapabilities,
    mediums: &[i32],
    metadata: Option<&MediumMetadata>,
) -> Option<Medium> {
    let roles = metadata.and_then(|metadata| metadata.medium_role.as_ref());
    let password = metadata.is_none_or(|metadata| {
        metadata.supported_wifi_direct_auth_types.is_empty()
            || metadata
                .supported_wifi_direct_auth_types
                .contains(&(WifiDirectAuthType::WifiDirectWithPassword as i32))
    });
    if cap.p2p_group_owner
        && mediums.contains(&(Medium::WifiDirect as i32))
        && roles.and_then(|roles| roles.support_wifi_direct_group_client) != Some(false)
        && password
    {
        Some(Medium::WifiDirect)
    } else if cap.hotspot
        && mediums.contains(&(Medium::WifiHotspot as i32))
        && roles.and_then(|roles| roles.support_wifi_hotspot_client) != Some(false)
    {
        Some(Medium::WifiHotspot)
    } else {
        None
    }
}
