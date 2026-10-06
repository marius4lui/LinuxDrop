//! Apply the same interface policy to discovery, listening and outgoing LAN
//! sockets. Owned Wi-Fi upgrade links are managed separately by the netd lease.
use anyhow::{Context, Result, bail};
use linuxdrop_network::{InterfaceAddress, TransferPolicy};
use once_cell::sync::Lazy;
use std::{
    net::{IpAddr, SocketAddr},
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
    if peer.is_unspecified() || peer.is_multicast() {
        bail!("Invalid LAN peer address");
    }
    interfaces(true)?
        .into_iter()
        .find(|local| local.address.is_ipv4() == peer.is_ipv4() && local.contains(peer))
        .context("Peer is outside the enabled local networks")
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
        .filter(|interface| !interface.loopback && interface.address.is_ipv4())
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
        let (updates, _) = tokio::sync::watch::channel(LanSnapshot::default());
        let mut result = Self {
            port,
            sockets: Default::default(),
            updates,
        };
        let snapshot = result.reconcile(interfaces(true)?).await;
        if !snapshot.errors.is_empty() {
            bail!("{}", snapshot.errors.join("; "));
        }
        if result.sockets.is_empty() {
            bail!("No enabled local IPv4 interface is available");
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
            if !interface.address.is_ipv4() || self.sockets.contains_key(&interface) {
                continue;
            }
            let bind = (|| -> Result<TcpListener> {
                let socket = TcpSocket::new_v4()?;
                socket.set_reuseaddr(true)?;
                #[cfg(target_os = "linux")]
                socket.bind_device(Some(interface.name.as_bytes()))?;
                socket.bind(SocketAddr::new(interface.address, self.port))?;
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
    let local = source_for(peer.ip())?;
    let socket = if peer.is_ipv4() {
        TcpSocket::new_v4()?
    } else {
        TcpSocket::new_v6()?
    };
    #[cfg(target_os = "linux")]
    socket.bind_device(Some(local.name.as_bytes()))?;
    socket.bind(SocketAddr::new(local.address, 0))?;
    Ok(socket.connect(peer).await?)
}
