use std::collections::HashMap;

use mdns_sd::{ServiceDaemon, ServiceEvent};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;

use crate::DeviceType;
use crate::utils::{is_not_self_ip, parse_mdns_endpoint_info};

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct EndpointInfo {
    pub fullname: String,
    pub id: String,
    pub name: Option<String>,
    pub ip: Option<String>,
    pub port: Option<String>,
    pub rtype: Option<DeviceType>,
    pub present: Option<bool>,
    /// Set for a recipient discovered over BLE (a phone on its receive screen).
    /// `ble_addr` is the peer's current LE address; `ble_psm` its L2CAP PSM.
    /// Both rotate, so the send path re-scans by name before dialing — these are
    /// a hint and the marker that this endpoint is reachable over BLE.
    pub ble_addr: Option<String>,
    pub ble_psm: Option<u16>,
}

pub struct MDnsDiscovery {
    daemon: ServiceDaemon,
    sender: broadcast::Sender<EndpointInfo>,
    lan_state: tokio::sync::watch::Receiver<crate::lan_policy::LanSnapshot>,
}

impl Drop for MDnsDiscovery {
    fn drop(&mut self) {
        let _ = self.daemon.shutdown();
    }
}

impl MDnsDiscovery {
    pub fn new(
        sender: broadcast::Sender<EndpointInfo>,
        lan_state: tokio::sync::watch::Receiver<crate::lan_policy::LanSnapshot>,
    ) -> Result<Self, anyhow::Error> {
        let daemon = ServiceDaemon::new()?;
        crate::lan_policy::configure_mdns_on(&daemon, &lan_state.borrow().interfaces)?;

        Ok(Self {
            daemon,
            sender,
            lan_state,
        })
    }

    pub async fn run(mut self, ctk: CancellationToken) -> Result<(), anyhow::Error> {
        let service_type = "_FC9F5ED42C8A._tcp.local.";
        let mut receiver = self.daemon.browse(service_type)?;
        let mut cache: HashMap<String, EndpointInfo> = HashMap::new();
        let monitor = self.daemon.monitor()?;
        let mut probes = tokio::task::JoinSet::<(String, u64, Option<EndpointInfo>)>::new();
        let mut pending = HashMap::new();
        let mut sequence = 0u64;
        loop {
            tokio::select! {
                biased;
                _ = ctk.cancelled() => break,
                changed = self.lan_state.changed() => {
                    if changed.is_err() { break; }
                    let snapshot = self.lan_state.borrow_and_update().clone();
                    probes.abort_all();
                    pending.clear();
                    cache.retain(|_, endpoint| {
                        let valid = endpoint.ip.as_ref().and_then(|ip| ip.parse().ok())
                            .is_some_and(|ip| snapshot.interfaces.iter().any(|interface| !interface.loopback && interface.contains(ip)));
                        if !valid { let _ = self.sender.send(EndpointInfo { id: endpoint.id.clone(), ..Default::default() }); }
                        valid
                    });
                    self.daemon.stop_browse(service_type)?;
                    crate::lan_policy::configure_mdns_on(&self.daemon, &snapshot.interfaces)?;
                    receiver = self.daemon.browse(service_type)?;
                },
                event = monitor.recv_async() => match event {
                    Ok(mdns_sd::DaemonEvent::Error(error)) => return Err(error.into()),
                    Err(error) => return Err(error.into()),
                    _ => {},
                },
                Some(result) = probes.join_next(), if !probes.is_empty() => {
                    if let Ok((fullname, request, endpoint)) = result {
                        if pending.get(&fullname) != Some(&request) { continue; }
                        pending.remove(&fullname);
                        if let Some(endpoint) = endpoint {
                            if let Some(old) = cache.insert(fullname, endpoint.clone()) {
                                if old.id != endpoint.id { let _ = self.sender.send(EndpointInfo { id: old.id, ..Default::default() }); }
                            }
                            let _ = self.sender.send(endpoint);
                        }
                    }
                },
                event = receiver.recv_async() => match event? {
                    ServiceEvent::ServiceResolved(info) => {
                        let fullname = info.get_fullname().to_owned();
                        if probes.len() >= 16 || pending.contains_key(&fullname) || (cache.len() >= 512 && !cache.contains_key(&fullname)) { continue; }
                        sequence = sequence.wrapping_add(1);
                        pending.insert(fullname.clone(), sequence);
                        let request = sequence;
                        let interfaces = self.lan_state.borrow().interfaces.clone();
                        probes.spawn(async move { (fullname, request, probe(info, interfaces).await) });
                    },
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        pending.remove(&fullname);
                        if let Some(old) = cache.remove(&fullname) {
                            let _ = self.sender.send(EndpointInfo { id: old.id, ..Default::default() });
                        }
                    },
                    _ => {},
                }
            }
        }
        self.daemon.stop_browse(service_type)?;
        Ok(())
    }
}

async fn probe(
    info: mdns_sd::ServiceInfo,
    interfaces: Vec<linuxdrop_network::InterfaceAddress>,
) -> Option<EndpointInfo> {
    let port = info.get_port();
    if port == 0 {
        return None;
    }
    let (rtype, name) = parse_mdns_endpoint_info(info.get_property("n")?.val_str()).ok()?;
    let addresses = info.get_addresses_v4();
    let mut addresses: Vec<_> = addresses.into_iter().copied().collect();
    addresses.sort();
    // A peer may advertise multiple LANs. Try each usable candidate, with a
    // single bounded deadline, instead of choosing a random HashSet entry.
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        use futures::StreamExt;
        let mut connections = futures::stream::FuturesUnordered::new();
        for ip in addresses
            .into_iter()
            .filter(|ip| {
                is_not_self_ip(ip)
                    && interfaces.iter().any(|interface| {
                        !interface.loopback && interface.contains(std::net::IpAddr::V4(*ip))
                    })
            })
            .take(16)
        {
            connections.push(async move {
                (
                    ip,
                    crate::lan_policy::connect(std::net::SocketAddr::new(
                        std::net::IpAddr::V4(ip),
                        port,
                    ))
                    .await,
                )
            });
        }
        while let Some((ip, result)) = connections.next().await {
            if result.is_ok() {
                return Some(EndpointInfo {
                    fullname: info.get_fullname().to_owned(),
                    id: format!("{ip}:{port}"),
                    name: Some(name),
                    ip: Some(ip.to_string()),
                    port: Some(port.to_string()),
                    rtype: Some(rtype),
                    present: Some(true),
                    ble_addr: None,
                    ble_psm: None,
                });
            }
        }
        None
    })
    .await
    .ok()
    .flatten()
}
