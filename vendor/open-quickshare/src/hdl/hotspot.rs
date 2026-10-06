//! Owned network upgrades. Every mutation uses a dedicated netd radio lease.
use linuxdrop_network::{P2pConnector, nm};
use rand::Rng;
use std::{
    net::Ipv4Addr,
    sync::{Arc, OnceLock, RwLock},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

static UPGRADE_LEASE: RwLock<Option<nm::Lease>> = RwLock::new(None);
static P2P_CONNECTOR: RwLock<Option<Arc<dyn P2pConnector>>> = RwLock::new(None);
fn exclusive() -> Arc<Semaphore> {
    static SLOT: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOT.get_or_init(|| Arc::new(Semaphore::new(1))).clone()
}
pub fn set_upgrade_lease(lease: Option<nm::Lease>) {
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
        .ok_or_else(|| anyhow::anyhow!("Wi-Fi upgrade requires a reserved dedicated adapter"))
}
pub const HOTSPOT_TCP_PORT: u16 = 61812;

pub struct HotspotGuard {
    pub ssid: String,
    pub password: String,
    pub gateway: Ipv4Addr,
    pub frequency: i32,
    _network: nm::Guard,
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
    pub address: Ipv4Addr,
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
pub async fn join_wifi(ssid: &str, password: &str) -> anyhow::Result<JoinGuard> {
    let lease = lease()?;
    let interface = lease.interface.clone();
    let network = nm::connect(lease, ssid.into(), password.into(), false, exclusive()).await?;
    Ok(JoinGuard {
        address: network.address,
        interface,
        _network: Some(network),
        p2p: None,
    })
}
pub async fn join_p2p(peer_name: &str, pin: &str, frequency: u32) -> anyhow::Result<JoinGuard> {
    let connector = P2P_CONNECTOR
        .read()
        .unwrap()
        .clone()
        .ok_or_else(|| anyhow::anyhow!("P2P helper is unavailable"))?;
    let permit = exclusive()
        .try_acquire_owned()
        .map_err(|_| anyhow::anyhow!("Dedicated adapter is already in use"))?;
    let mut guard = JoinGuard {
        address: Ipv4Addr::UNSPECIFIED,
        interface: String::new(),
        _network: None,
        p2p: Some((connector.clone(), permit)),
    };
    let connected = connector
        .connect(peer_name.into(), pin.into(), frequency)
        .await?;
    guard.address = connected.ipv4_address;
    guard.interface = connected.interface;
    Ok(guard)
}
pub async fn start_hotspot() -> anyhow::Result<HotspotGuard> {
    let lease = lease()?;
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
        gateway: network.address,
        frequency: network.frequency,
        _network: network,
    })
}
