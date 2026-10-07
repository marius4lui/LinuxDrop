//! Owned network upgrades. Every mutation uses a dedicated netd radio lease.
use linuxdrop_network::{P2pConnector, nm};
use rand::Rng;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, OnceLock, RwLock},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

static UPGRADE_LEASE: RwLock<Option<(nm::Lease, linuxdrop_network::DirectWifiCapabilities)>> =
    RwLock::new(None);
static P2P_CONNECTOR: RwLock<Option<Arc<dyn P2pConnector>>> = RwLock::new(None);
fn exclusive() -> Arc<Semaphore> {
    static SLOT: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOT.get_or_init(|| Arc::new(Semaphore::new(1))).clone()
}
pub fn set_upgrade_lease(lease: Option<(nm::Lease, linuxdrop_network::DirectWifiCapabilities)>) {
    *UPGRADE_LEASE.write().unwrap() = lease;
}
pub fn set_p2p_connector(connector: Option<Arc<dyn P2pConnector>>) {
    *P2P_CONNECTOR.write().unwrap() = connector;
}
fn lease() -> anyhow::Result<nm::Lease> {
    UPGRADE_LEASE
        .read()
        .unwrap()
        .clone()
        .map(|(lease, _)| lease)
        .ok_or_else(|| anyhow::anyhow!("Wi-Fi upgrade requires a reserved dedicated adapter"))
}
pub fn upgrade_capabilities() -> linuxdrop_network::DirectWifiCapabilities {
    let mut capabilities = UPGRADE_LEASE
        .read()
        .unwrap()
        .as_ref()
        .map(|(_, capabilities)| capabilities.clone())
        .unwrap_or_default();
    if P2P_CONNECTOR.read().unwrap().is_none() {
        capabilities.p2p_client = false;
        capabilities.p2p_group_owner = false;
    }
    capabilities
}
pub const HOTSPOT_TCP_PORT: u16 = 61812;

pub struct HotspotGuard {
    pub ssid: String,
    pub password: String,
    pub gateway: Ipv4Addr,
    pub frequency: i32,
    pub interface: String,
    _network: Option<nm::Guard>,
    _p2p: Option<JoinGuard>,
}
impl std::fmt::Debug for HotspotGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HotspotGuard")
            .field("gateway", &self.gateway)
            .field("frequency", &self.frequency)
            .finish_non_exhaustive()
    }
}
pub struct JoinGuard {
    pub interface: String,
    _network: Option<nm::Guard>,
    p2p: Option<(Arc<dyn P2pConnector>, OwnedSemaphorePermit)>,
}
impl std::fmt::Debug for JoinGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JoinGuard")
            .field("interface", &self.interface)
            .finish_non_exhaustive()
    }
}
impl Drop for JoinGuard {
    fn drop(&mut self) {
        if let Some((connector, permit)) = self.p2p.take() {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = connector.disconnect().await;
                    drop(permit);
                });
            }
        }
    }
}
pub async fn join_wifi(
    ssid: &str,
    password: &str,
    candidates: &[SocketAddr],
) -> anyhow::Result<JoinGuard> {
    anyhow::ensure!(
        upgrade_capabilities().station,
        "Reserved adapter cannot join a Wi-Fi network"
    );
    let lease = lease()?;
    let interface = lease.interface.clone();
    let link_local = candidates
        .iter()
        .any(|candidate| matches!(candidate.ip(), IpAddr::V6(ip) if ip.is_unicast_link_local()))
        && !candidates.iter().any(
            |candidate| matches!(candidate.ip(), IpAddr::V6(ip) if !ip.is_unicast_link_local()),
        );
    let network = nm::connect_with_ipv6(
        lease,
        ssid.into(),
        password.into(),
        false,
        exclusive(),
        link_local,
    )
    .await?;
    Ok(JoinGuard {
        interface,
        _network: Some(network),
        p2p: None,
    })
}
pub async fn join_p2p(peer_name: &str, pin: &str, frequency: u32) -> anyhow::Result<JoinGuard> {
    anyhow::ensure!(
        upgrade_capabilities().p2p_client,
        "Reserved adapter cannot join a P2P group"
    );
    let connector = P2P_CONNECTOR
        .read()
        .unwrap()
        .clone()
        .ok_or_else(|| anyhow::anyhow!("P2P helper is unavailable"))?;
    let permit = exclusive()
        .try_acquire_owned()
        .map_err(|_| anyhow::anyhow!("Dedicated adapter is already in use"))?;
    let mut guard = JoinGuard {
        interface: String::new(),
        _network: None,
        p2p: Some((connector.clone(), permit)),
    };
    let connected = connector
        .connect(peer_name.into(), pin.into(), frequency)
        .await?;
    guard.interface = connected.interface;
    Ok(guard)
}
/// Create an autonomous P2P group on the reserved radio. Keep the permit and
/// cleanup guard alive from before the first await through transfer completion.
pub async fn start_direct_group() -> anyhow::Result<HotspotGuard> {
    anyhow::ensure!(
        upgrade_capabilities().p2p_group_owner,
        "Reserved adapter cannot host a P2P group"
    );
    let connector = P2P_CONNECTOR
        .read()
        .unwrap()
        .clone()
        .ok_or_else(|| anyhow::anyhow!("P2P helper is unavailable"))?;
    let permit = exclusive()
        .try_acquire_owned()
        .map_err(|_| anyhow::anyhow!("Dedicated adapter is already in use"))?;
    let mut ownership = JoinGuard {
        interface: String::new(),
        _network: None,
        p2p: Some((connector.clone(), permit)),
    };
    let hosted = connector.host().await?;
    ownership.interface = hosted.interface.clone();
    Ok(HotspotGuard {
        ssid: hosted.ssid,
        password: hosted.password,
        gateway: hosted.ipv4_address,
        frequency: hosted.frequency.into(),
        interface: hosted.interface,
        _network: None,
        _p2p: Some(ownership),
    })
}
pub async fn start_hotspot() -> anyhow::Result<HotspotGuard> {
    anyhow::ensure!(
        upgrade_capabilities().hotspot,
        "Reserved adapter cannot host a hotspot"
    );
    let lease = lease()?;
    let interface = lease.interface.clone();
    let (ssid, password) = {
        let mut rng = rand::rng();
        let suffix: String = (0..4)
            .map(|_| rng.sample(rand::distr::Alphanumeric) as char)
            .collect();
        let password: String = (0..16)
            .map(|_| rng.sample(rand::distr::Alphanumeric) as char)
            .collect();
        (format!("DIRECT-{suffix}-LinuxDrop"), password)
    };
    let network = nm::connect(lease, ssid.clone(), password.clone(), true, exclusive()).await?;
    Ok(HotspotGuard {
        ssid,
        password,
        gateway: network
            .address
            .ok_or_else(|| anyhow::anyhow!("Hosted network has no IPv4 gateway"))?,
        interface: interface.clone(),
        frequency: network.frequency,
        _network: Some(network),
        _p2p: None,
    })
}

/// Enumerate only the lease's interface, independently of the ordinary LAN policy.
pub fn leased_interfaces(
    interface: &str,
) -> anyhow::Result<Vec<linuxdrop_network::InterfaceAddress>> {
    if interface.is_empty() {
        anyhow::bail!("Missing leased network interface");
    }
    let policy = linuxdrop_network::TransferPolicy {
        allowed_interfaces: vec![interface.into()],
        ..Default::default()
    };
    Ok(linuxdrop_network::interfaces(&policy, false)?
        .into_iter()
        .filter(|local| local.name == interface)
        .collect())
}
pub async fn listen_hosted(
    guard: &HotspotGuard,
) -> anyhow::Result<crate::lan_policy::LanListeners> {
    let interfaces = leased_interfaces(&guard.interface)?
        .into_iter()
        .filter(|local| local.address.is_ipv6() || local.address == IpAddr::V4(guard.gateway))
        .collect();
    crate::lan_policy::LanListeners::new_on(HOTSPOT_TCP_PORT, interfaces).await
}
pub fn hosted_offer(
    guard: &HotspotGuard,
    listener: &crate::lan_policy::LanListeners,
    medium: crate::location_nearby_connections::bandwidth_upgrade_negotiation_frame::upgrade_path_info::Medium,
) -> anyhow::Result<
    crate::location_nearby_connections::bandwidth_upgrade_negotiation_frame::UpgradePathInfo,
> {
    guard.offer(listener.port(), hosted_candidates(listener), medium)
}
impl HotspotGuard {
    pub fn offer(
        &self,
        port: u16,
        address_candidates: Vec<crate::location_nearby_connections::ServiceAddress>,
        medium: crate::location_nearby_connections::bandwidth_upgrade_negotiation_frame::upgrade_path_info::Medium,
    ) -> anyhow::Result<
        crate::location_nearby_connections::bandwidth_upgrade_negotiation_frame::UpgradePathInfo,
    > {
        use crate::location_nearby_connections::bandwidth_upgrade_negotiation_frame::{
            UpgradePathInfo,
            upgrade_path_info::{Medium, WifiDirectCredentials, WifiHotspotCredentials},
        };
        anyhow::ensure!(port != 0, "Hosted upgrade has no listening port");
        let mut info = UpgradePathInfo {
            medium: Some(medium.into()),
            supports_client_introduction_ack: Some(true),
            ..Default::default()
        };
        match medium {
            Medium::WifiDirect if self._p2p.is_some() => {
                info.wifi_direct_credentials = Some(WifiDirectCredentials {
                    ssid: Some(self.ssid.clone()),
                    password: Some(self.password.clone()),
                    port: Some(port.into()),
                    frequency: Some(self.frequency),
                    gateway: Some(self.gateway.to_string()),
                    ip_v6_address: address_candidates.iter().find_map(|candidate| {
                        let ip = crate::lan_policy::decode_ip(candidate.ip_address()).ok()?;
                        matches!(ip, IpAddr::V6(ip) if ip.is_unicast_link_local())
                            .then(|| candidate.ip_address().to_vec())
                    }),
                    ..Default::default()
                });
            }
            Medium::WifiHotspot if self._network.is_some() => {
                info.wifi_hotspot_credentials = Some(WifiHotspotCredentials {
                    ssid: Some(self.ssid.clone()),
                    password: Some(self.password.clone()),
                    port: Some(port.into()),
                    frequency: Some(self.frequency),
                    gateway: Some(self.gateway.to_string()),
                    address_candidates,
                });
            }
            _ => anyhow::bail!("Hosted upgrade medium does not match its owned network"),
        }
        Ok(info)
    }
}
pub fn hosted_candidates(
    listener: &crate::lan_policy::LanListeners,
) -> Vec<crate::location_nearby_connections::ServiceAddress> {
    let mut interfaces = listener.subscribe().borrow().interfaces.clone();
    interfaces.sort_by_key(|local| (local.address.is_ipv4(), local.address));
    interfaces
        .into_iter()
        .filter(|local| crate::lan_policy::valid_unicast(local.address))
        .map(|local| crate::location_nearby_connections::ServiceAddress {
            ip_address: Some(crate::lan_policy::address_bytes(local.address)),
            port: Some(listener.port().into()),
        })
        .collect()
}
fn network_address(ip: IpAddr, port: i32) -> anyhow::Result<SocketAddr> {
    let port = u16::try_from(port)?;
    if port == 0 || !crate::lan_policy::valid_unicast(ip) || ip.is_loopback() {
        anyhow::bail!("Invalid dedicated-network candidate");
    }
    Ok(SocketAddr::new(ip, port))
}
pub fn direct_candidates(
    credentials: &crate::location_nearby_connections::bandwidth_upgrade_negotiation_frame::upgrade_path_info::WifiDirectCredentials,
) -> anyhow::Result<Vec<SocketAddr>> {
    let mut result = Vec::new();
    if !credentials.ip_v6_address().is_empty() {
        let ip = crate::lan_policy::decode_ip(credentials.ip_v6_address())?;
        if !matches!(ip, IpAddr::V6(ip) if ip.is_unicast_link_local()) {
            anyhow::bail!("Wi-Fi Direct IPv6 address must be link-local");
        }
        result.push(network_address(ip, credentials.port())?);
    }
    if let Ok(ip) = credentials.gateway().parse::<Ipv4Addr>() {
        if !ip.is_unspecified() {
            result.push(network_address(ip.into(), credentials.port())?);
        }
    }
    if result.is_empty() {
        anyhow::bail!("Wi-Fi Direct has no usable address");
    }
    Ok(result)
}
pub fn hotspot_candidates(
    credentials: &crate::location_nearby_connections::bandwidth_upgrade_negotiation_frame::upgrade_path_info::WifiHotspotCredentials,
) -> anyhow::Result<Vec<SocketAddr>> {
    if credentials.address_candidates.len() > 64 {
        anyhow::bail!("Too many hotspot address candidates");
    }
    if credentials.address_candidates.is_empty() {
        return Ok(vec![network_address(
            credentials.gateway().parse()?,
            credentials.port(),
        )?]);
    }
    credentials
        .address_candidates
        .iter()
        .map(|candidate| {
            network_address(
                crate::lan_policy::decode_ip(candidate.ip_address())?,
                candidate.port(),
            )
        })
        .collect()
}
/// Bind every attempt to the joined interface and scope remote link-local
/// addresses using that interface's local index, never a remote-supplied index.
pub async fn connect_joined(
    interface: &str,
    candidates: &[SocketAddr],
) -> anyhow::Result<tokio::net::TcpStream> {
    if candidates.is_empty() || candidates.len() > 64 {
        anyhow::bail!("Invalid dedicated-network candidate count");
    }
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
    tokio::time::timeout_at(deadline, async {
        loop {
            let locals = leased_interfaces(interface)?;
            for offered in candidates.iter().take(64) {
                network_address(offered.ip(), offered.port().into())?;
                for local in locals.iter().filter(|local| local.contains(offered.ip())) {
                    if matches!(offered, SocketAddr::V6(v6) if v6.scope_id() != 0 && v6.scope_id() != local.index) { continue; }
                    let peer = crate::lan_policy::scoped_address(local, offered.ip(), offered.port());
                    let attempt = async {
                        let socket = if peer.is_ipv4() { tokio::net::TcpSocket::new_v4()? } else { tokio::net::TcpSocket::new_v6()? };
                        socket.bind_device(Some(interface.as_bytes()))?;
                        socket.bind(crate::lan_policy::scoped_address(local, local.address, 0))?;
                        socket.connect(peer).await
                    };
                    if let Ok(Ok(stream)) = tokio::time::timeout(std::time::Duration::from_millis(800), attempt).await { return Ok(stream); }
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }).await.map_err(|_| anyhow::anyhow!("Dedicated-network candidates timed out"))?
}
