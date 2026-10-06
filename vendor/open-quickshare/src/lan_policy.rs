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
    daemon.disable_interface(mdns_sd::IfKind::All)?;
    let names: std::collections::BTreeSet<_> = interfaces(false)?
        .into_iter()
        .map(|interface| interface.name)
        .collect();
    for name in names {
        daemon.enable_interface(mdns_sd::IfKind::Name(name))?;
    }
    Ok(())
}
pub async fn listeners(port: u16) -> Result<Vec<TcpListener>> {
    // The current mDNS engine advertises IPv4. Bind those exact local addresses,
    // including localhost for local IPC/diagnostics, instead of 0.0.0.0.
    let addresses: std::collections::BTreeSet<_> = interfaces(true)?
        .into_iter()
        .filter(|i| i.address.is_ipv4())
        .map(|i| i.address)
        .collect();
    let mut listeners = Vec::new();
    let mut port = port;
    for address in addresses {
        let listener = TcpListener::bind(SocketAddr::new(address, port)).await?;
        port = listener.local_addr()?.port();
        listeners.push(listener);
    }
    if listeners.is_empty() {
        bail!("No enabled local IPv4 interface is available");
    }
    Ok(listeners)
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
