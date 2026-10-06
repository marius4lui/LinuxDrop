//! Shared network selection and bounded bandwidth policy; no radio mutations.
pub mod nm;
pub mod p2p;
pub struct P2pConnection {
    pub interface: String,
    pub ipv4_address: Ipv4Addr,
}
pub trait P2pConnector: Send + Sync {
    fn connect(
        &self,
        peer_name: String,
        pin: String,
        frequency: u32,
    ) -> futures_util::future::BoxFuture<'_, Result<P2pConnection>>;
    fn disconnect(&self) -> futures_util::future::BoxFuture<'_, Result<()>>;
}
use anyhow::{bail, Result};
pub use linuxdrop_core::{SendSource, TransferPolicy};
use std::{
    ffi::CStr,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::Arc,
    time::Duration,
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

static ADVERTISEMENT_REGISTRATION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Resolve one powered controller once, then pass its name to every operation.
pub async fn bluetooth_adapter(requested: Option<&str>) -> Result<bluer::Adapter> {
    let session = bluer::Session::new().await?;
    if let Some(name) = requested {
        if !name.strip_prefix("hci").is_some_and(|suffix| {
            !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
        }) {
            bail!("Invalid Bluetooth controller name");
        }
        let adapter = session.adapter(name)?;
        if !adapter.is_powered().await? {
            bail!("Bluetooth controller {name} is switched off");
        }
        return Ok(adapter);
    }
    let mut names = session.adapter_names().await?;
    names.sort();
    for name in names {
        let adapter = session.adapter(&name)?;
        if adapter.is_powered().await.unwrap_or(false) {
            return Ok(adapter);
        }
    }
    bail!("No powered Bluetooth controller is available")
}

/// All LinuxDrop advertisement registrations share this short critical section.
/// Existing registrations/handshakes are never evicted to make room.
pub async fn advertise(
    adapter: &bluer::Adapter,
    advertisement: bluer::adv::Advertisement,
) -> Result<bluer::adv::AdvertisementHandle> {
    let _guard = ADVERTISEMENT_REGISTRATION.lock().await;
    let supported = adapter.supported_advertising_instances().await?;
    let active = adapter.active_advertising_instances().await?;
    if supported == 0 || active >= supported {
        bail!("Bluetooth controller {} has no free advertisement slots ({active}/{supported}). Select another controller or disable the other protocol's Bluetooth advertisements.",adapter.name());
    }
    Ok(adapter.advertise(advertisement).await?)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InterfaceAddress {
    pub name: String,
    pub address: IpAddr,
    pub netmask: IpAddr,
    pub index: u32,
    pub loopback: bool,
}
impl InterfaceAddress {
    pub fn contains(&self, peer: IpAddr) -> bool {
        match (self.address, self.netmask, peer) {
            (IpAddr::V4(local), IpAddr::V4(mask), IpAddr::V4(peer)) => {
                u32::from(local) & u32::from(mask) == u32::from(peer) & u32::from(mask)
            }
            (IpAddr::V6(local), IpAddr::V6(mask), IpAddr::V6(peer)) => {
                u128::from(local) & u128::from(mask) == u128::from(peer) & u128::from(mask)
            }
            _ => false,
        }
    }
}

fn permitted_name(name: &str, policy: &TransferPolicy, virtual_interface: bool) -> bool {
    if !policy.allowed_interfaces.is_empty() {
        return policy
            .allowed_interfaces
            .iter()
            .any(|allowed| allowed == name);
    }
    if policy.allow_virtual_interfaces {
        return true;
    }
    !virtual_interface
        && ![
            "tun",
            "tap",
            "wg",
            "tailscale",
            "zt",
            "docker",
            "veth",
            "virbr",
            "br-",
            "vpn",
            "ppp",
        ]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// Enumerates up local interfaces. Explicit allowlists permit an intentionally
/// selected virtual interface; otherwise tunnels/bridges are not advertised.
pub fn interfaces(
    policy: &TransferPolicy,
    include_loopback: bool,
) -> Result<Vec<InterfaceAddress>> {
    struct Addresses(*mut libc::ifaddrs);
    impl Drop for Addresses {
        fn drop(&mut self) {
            unsafe {
                libc::freeifaddrs(self.0);
            }
        }
    }
    let mut pointer = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut pointer) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let addresses = Addresses(pointer);
    let mut result = Vec::new();
    let mut current = addresses.0;
    while !current.is_null() {
        let item = unsafe { &*current };
        current = item.ifa_next;
        if item.ifa_addr.is_null()
            || item.ifa_netmask.is_null()
            || item.ifa_flags & libc::IFF_UP as u32 == 0
        {
            continue;
        }
        let name = unsafe { CStr::from_ptr(item.ifa_name) }
            .to_string_lossy()
            .into_owned();
        let loopback = item.ifa_flags & libc::IFF_LOOPBACK as u32 != 0;
        let virtual_interface = std::fs::canonicalize(format!("/sys/class/net/{name}"))
            .is_ok_and(|path| path.starts_with("/sys/devices/virtual/net"));
        if loopback {
            if !include_loopback {
                continue;
            }
        } else if !permitted_name(&name, policy, virtual_interface) {
            continue;
        }
        let family = unsafe { (*item.ifa_addr).sa_family as i32 };
        let pair = match family {
            libc::AF_INET => unsafe {
                let address = &*(item.ifa_addr as *const libc::sockaddr_in);
                let mask = &*(item.ifa_netmask as *const libc::sockaddr_in);
                Some((
                    IpAddr::V4(Ipv4Addr::from(address.sin_addr.s_addr.to_ne_bytes())),
                    IpAddr::V4(Ipv4Addr::from(mask.sin_addr.s_addr.to_ne_bytes())),
                ))
            },
            libc::AF_INET6 => unsafe {
                let address = &*(item.ifa_addr as *const libc::sockaddr_in6);
                let mask = &*(item.ifa_netmask as *const libc::sockaddr_in6);
                Some((
                    IpAddr::V6(Ipv6Addr::from(address.sin6_addr.s6_addr)),
                    IpAddr::V6(Ipv6Addr::from(mask.sin6_addr.s6_addr)),
                ))
            },
            _ => None,
        };
        if let Some((address, netmask)) = pair {
            result.push(InterfaceAddress {
                name,
                address,
                netmask,
                index: unsafe { libc::if_nametoindex(item.ifa_name) },
                loopback,
            });
        }
    }
    Ok(result)
}

/// Clones share one aggregate payload budget across sessions and backends.
#[derive(Clone, Debug)]
pub struct BandwidthLimiter {
    bytes_per_second: Option<u64>,
    next: Arc<tokio::sync::Mutex<Instant>>,
}
impl BandwidthLimiter {
    pub fn new(bytes_per_second: Option<u64>) -> Self {
        Self {
            bytes_per_second: bytes_per_second.filter(|rate| *rate > 0),
            next: Arc::new(tokio::sync::Mutex::new(Instant::now())),
        }
    }
    pub async fn acquire(&self, bytes: usize, cancel: &CancellationToken) -> Result<()> {
        if cancel.is_cancelled() {
            bail!("Transfer cancelled");
        }
        if bytes == 0 {
            return Ok(());
        }
        let Some(rate) = self.bytes_per_second else {
            return Ok(());
        };
        let mut next = tokio::select! {_=cancel.cancelled()=>bail!("Transfer cancelled"),guard=self.next.lock()=>guard};
        let deadline =
            (*next).max(Instant::now()) + Duration::from_secs_f64(bytes as f64 / rate as f64);
        tokio::select! {_=cancel.cancelled()=>bail!("Transfer cancelled"),_=tokio::time::sleep_until(deadline)=>{*next=deadline;Ok(())}}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tunnels_require_explicit_selection_and_subnet_checks_are_exact() {
        let mut policy = TransferPolicy::default();
        assert!(!permitted_name("tun0", &policy, false));
        assert!(!permitted_name("bridge0", &policy, true));
        assert!(permitted_name("eth0", &policy, false));
        policy.allowed_interfaces = vec!["tun0".into()];
        assert!(permitted_name("tun0", &policy, true));
        assert!(!permitted_name("eth0", &policy, false));
        let interface = InterfaceAddress {
            name: "eth0".into(),
            address: "192.0.2.2".parse().unwrap(),
            netmask: "255.255.255.0".parse().unwrap(),
            index: 1,
            loopback: false,
        };
        assert!(interface.contains("192.0.2.99".parse().unwrap()));
        assert!(!interface.contains("192.0.3.1".parse().unwrap()));
    }
    #[tokio::test]
    async fn bandwidth_wait_observes_cancellation() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(BandwidthLimiter::new(Some(1))
            .acquire(1024, &cancel)
            .await
            .is_err());
    }
}
