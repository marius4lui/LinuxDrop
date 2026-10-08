//! Apply the same interface policy to discovery, listening and outgoing LAN
//! sockets. Owned Wi-Fi upgrade links are managed separately by the netd lease.
use anyhow::{Context, Result, bail};
use linuxdrop_network::{InterfaceAddress, TransferPolicy};
use once_cell::sync::Lazy;
use std::{
    net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6},
    sync::RwLock,
};
use tokio::net::{TcpListener, TcpSocket, TcpStream};

static POLICY: Lazy<RwLock<TransferPolicy>> = Lazy::new(|| RwLock::new(TransferPolicy::default()));
pub fn set(policy: TransferPolicy) {
    *POLICY.write().unwrap() = policy;
}
pub fn interfaces(loopback: bool) -> Result<Vec<InterfaceAddress>> {
    linuxdrop_network::interfaces(&POLICY.read().unwrap(), loopback)
}
pub fn source_for(peer: IpAddr) -> Result<InterfaceAddress> {
    source_for_on(SocketAddr::new(peer, 0), &interfaces(true)?)
}
pub fn source_for_on(peer: SocketAddr, available: &[InterfaceAddress]) -> Result<InterfaceAddress> {
    if !valid_unicast(peer.ip()) {
        bail!("Invalid LAN peer address");
    }
    let scope = match peer {
        SocketAddr::V6(v6) => v6.scope_id(),
        _ => 0,
    };
    let choices: Vec<_> = available
        .iter()
        .filter(|local| local.contains(peer.ip()) && (scope == 0 || scope == local.index))
        .collect();
    if matches!(peer.ip(), IpAddr::V6(ip) if ip.is_unicast_link_local()) && scope == 0 {
        bail!("Link-local peer requires an explicit enabled interface scope");
    }
    choices
        .first()
        .map(|local| (*local).clone())
        .context("Peer is outside the enabled local networks")
}
pub fn valid_unicast(ip: IpAddr) -> bool {
    !ip.is_unspecified()
        && !ip.is_multicast()
        && match ip {
            IpAddr::V4(ip) => !ip.is_broadcast(),
            IpAddr::V6(ip) => ip.to_ipv4_mapped().is_none(),
        }
}
pub fn scoped_address(interface: &InterfaceAddress, ip: IpAddr, port: u16) -> SocketAddr {
    match ip {
        IpAddr::V6(ip) if ip.is_unicast_link_local() => {
            SocketAddr::V6(SocketAddrV6::new(ip, port, 0, interface.index))
        }
        _ => SocketAddr::new(ip, port),
    }
}
/// Expand link-local discovery results only on explicitly enabled local links.
pub fn discovery_candidates(
    ip: IpAddr,
    port: u16,
    available: &[InterfaceAddress],
) -> Vec<SocketAddr> {
    if port == 0
        || !valid_unicast(ip)
        || ip.is_loopback()
        || available.iter().any(|local| local.address == ip)
    {
        return vec![];
    }
    let mut result = Vec::new();
    for local in available
        .iter()
        .filter(|local| !local.loopback && local.contains(ip))
    {
        let address = scoped_address(local, ip, port);
        if !result.contains(&address) {
            result.push(address);
        }
    }
    result
}
pub fn permits(local: IpAddr, peer: IpAddr) -> bool {
    interfaces(true).is_ok_and(|interfaces| {
        interfaces
            .iter()
            .any(|interface| interface.address == local && interface.contains(peer))
    })
}
pub fn configure_mdns(daemon: &mdns_sd::ServiceDaemon) -> Result<()> {
    configure_mdns_on(daemon, &interfaces(false)?)
}
pub fn configure_mdns_on(
    daemon: &mdns_sd::ServiceDaemon,
    interfaces: &[InterfaceAddress],
) -> Result<()> {
    daemon.disable_interface(mdns_sd::IfKind::All)?;
    let addresses: std::collections::BTreeSet<_> = interfaces
        .iter()
        .filter(|interface| !interface.loopback && valid_unicast(interface.address))
        .map(|interface| interface.address)
        .collect();
    for address in addresses {
        daemon.enable_interface(mdns_sd::IfKind::Addr(address))?;
    }
    Ok(())
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LanSnapshot {
    pub interfaces: Vec<InterfaceAddress>,
    pub errors: Vec<String>,
}
impl LanSnapshot {
    pub fn available(&self) -> bool {
        self.interfaces.iter().any(|interface| !interface.loopback)
    }
}

/// Own the listen sockets and publish only addresses actually bound. Consumers
/// share this snapshot instead of racing independent network enumerations.
pub struct LanListeners {
    port: u16,
    sockets: std::collections::HashMap<InterfaceAddress, TcpListener>,
    updates: tokio::sync::watch::Sender<LanSnapshot>,
}
impl LanListeners {
    pub async fn new(port: u16) -> Result<Self> {
        Self::new_on(port, interfaces(true)?).await
    }
    pub async fn new_on(port: u16, interfaces: Vec<InterfaceAddress>) -> Result<Self> {
        let (updates, _) = tokio::sync::watch::channel(LanSnapshot::default());
        let mut result = Self {
            port,
            sockets: Default::default(),
            updates,
        };
        let snapshot = result.reconcile(interfaces).await;
        if !snapshot.errors.is_empty() {
            bail!("{}", snapshot.errors.join("; "));
        }
        if result.sockets.is_empty() {
            bail!(
                "No enabled local interface is available: {}",
                snapshot.errors.join("; ")
            );
        }
        Ok(result)
    }
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<LanSnapshot> {
        self.updates.subscribe()
    }
    pub fn port(&self) -> u16 {
        self.port
    }
    pub async fn reconcile(&mut self, desired: Vec<InterfaceAddress>) -> LanSnapshot {
        self.sockets
            .retain(|interface, _| desired.contains(interface));
        let mut errors = Vec::new();
        for interface in desired {
            if !valid_unicast(interface.address) || self.sockets.contains_key(&interface) {
                continue;
            }
            let bind = (|| -> Result<TcpListener> {
                let socket = if interface.address.is_ipv4() {
                    TcpSocket::new_v4()?
                } else {
                    TcpSocket::new_v6()?
                };
                socket.set_reuseaddr(true)?;
                #[cfg(target_os = "linux")]
                socket.bind_device(Some(interface.name.as_bytes()))?;
                socket.bind(scoped_address(&interface, interface.address, self.port))?;
                Ok(socket.listen(128)?)
            })();
            match bind {
                Ok(socket) => {
                    if self.port == 0 {
                        self.port = socket.local_addr().expect("bound socket address").port();
                    }
                    self.sockets.insert(interface, socket);
                }
                Err(error) => errors.push(format!(
                    "{} ({}): {error}",
                    interface.name, interface.address
                )),
            }
        }
        let mut bound: Vec<_> = self.sockets.keys().cloned().collect();
        bound.sort_by(|a, b| (&a.name, a.address).cmp(&(&b.name, b.address)));
        errors.sort();
        let snapshot = LanSnapshot {
            interfaces: bound,
            errors,
        };
        self.updates.send_if_modified(|old| {
            if old == &snapshot {
                false
            } else {
                *old = snapshot.clone();
                true
            }
        });
        snapshot
    }
    pub async fn refresh(&mut self) -> Result<LanSnapshot> {
        Ok(self.reconcile(interfaces(true)?).await)
    }
    pub async fn accept(&self) -> std::io::Result<(TcpStream, SocketAddr)> {
        if self.sockets.is_empty() {
            return std::future::pending().await;
        }
        futures::future::select_all(
            self.sockets
                .values()
                .map(|listener| Box::pin(listener.accept())),
        )
        .await
        .0
    }
}
pub async fn listeners(port: u16) -> Result<Vec<TcpListener>> {
    Ok(LanListeners::new(port)
        .await?
        .sockets
        .into_values()
        .collect())
}
pub async fn accept(listeners: &[TcpListener]) -> std::io::Result<(TcpStream, SocketAddr)> {
    futures::future::select_all(listeners.iter().map(|listener| Box::pin(listener.accept())))
        .await
        .0
}
pub async fn connect(peer: SocketAddr) -> Result<TcpStream> {
    let local = source_for_on(peer, &interfaces(true)?)?;
    let socket = if peer.is_ipv4() {
        TcpSocket::new_v4()?
    } else {
        TcpSocket::new_v6()?
    };
    #[cfg(target_os = "linux")]
    socket.bind_device(Some(local.name.as_bytes()))?;
    socket.bind(scoped_address(&local, local.address, 0))?;
    Ok(socket.connect(peer).await?)
}

/// Network-order address bytes used by Google's ServiceAddress and metadata.
pub fn decode_ip(bytes: &[u8]) -> Result<IpAddr> {
    match bytes.len() {
        4 => Ok(std::net::Ipv4Addr::from(<[u8; 4]>::try_from(bytes)?).into()),
        16 => Ok(Ipv6Addr::from(<[u8; 16]>::try_from(bytes)?).into()),
        _ => bail!("Invalid protocol address length"),
    }
}
pub fn address_bytes(ip: IpAddr) -> Vec<u8> {
    match ip {
        IpAddr::V4(ip) => ip.octets().to_vec(),
        IpAddr::V6(ip) => ip.octets().to_vec(),
    }
}
pub fn upgrade_address(ip: IpAddr) -> bool {
    valid_unicast(ip)
        && !ip.is_loopback()
        && !match ip {
            IpAddr::V6(ip) => ip.is_unicast_link_local(),
            IpAddr::V4(ip) => ip.is_link_local(),
        }
}
pub fn upgrade_offer(addresses: &[SocketAddr]) -> Result<crate::location_nearby_connections::bandwidth_upgrade_negotiation_frame::upgrade_path_info::WifiLanSocket>{
    let mut addresses: Vec<_> = addresses
        .iter()
        .copied()
        .filter(|address| upgrade_address(address.ip()) && address.port() != 0)
        .collect();
    addresses.sort_by_key(|address| (address.is_ipv4(), *address));
    addresses.dedup();
    addresses.truncate(64);
    let last = addresses
        .last()
        .context("No usable WIFI_LAN upgrade listener")?;
    Ok(crate::location_nearby_connections::bandwidth_upgrade_negotiation_frame::upgrade_path_info::WifiLanSocket {
        ip_address: Some(address_bytes(last.ip())), wifi_port: Some(last.port().into()),
        address_candidates: addresses.iter().map(|address| crate::location_nearby_connections::ServiceAddress {
            ip_address: Some(address_bytes(address.ip())), port: Some(address.port().into()),
        }).collect(),
    })
}
pub fn upgrade_candidates(
    offer: &crate::location_nearby_connections::bandwidth_upgrade_negotiation_frame::upgrade_path_info::WifiLanSocket,
) -> Result<Vec<SocketAddr>> {
    fn address(bytes: &[u8], port: i32) -> Result<SocketAddr> {
        let ip = decode_ip(bytes)?;
        let port = u16::try_from(port).context("Invalid WIFI_LAN port")?;
        if port == 0 || !upgrade_address(ip) {
            bail!("Invalid WIFI_LAN upgrade candidate");
        }
        Ok(SocketAddr::new(ip, port))
    }
    if offer.address_candidates.len() > 64 {
        bail!("Too many WIFI_LAN candidates");
    }
    if offer.address_candidates.is_empty() {
        return Ok(vec![address(offer.ip_address(), offer.wifi_port())?]);
    }
    // The ordered candidate list replaces the legacy fields, even if malformed.
    offer
        .address_candidates
        .iter()
        .map(|candidate| address(candidate.ip_address(), candidate.port()))
        .collect()
}
pub async fn connect_candidates(addresses: &[SocketAddr]) -> Result<TcpStream> {
    // Preserve offered order, with bounded per-candidate and total deadlines.
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        for address in addresses.iter().take(64) {
            if let Ok(Ok(socket)) =
                tokio::time::timeout(std::time::Duration::from_millis(800), connect(*address)).await
            {
                return Ok(socket);
            }
        }
        bail!("No enabled WIFI_LAN candidate is reachable")
    })
    .await
    .context("WIFI_LAN candidates timed out")?
}
