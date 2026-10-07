//! Temporary NetworkManager profiles belonging to an existing netd radio lease.
//! Passwords are sent over D-Bus, never placed in process arguments or logs.
use anyhow::{bail, Context, Result};
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use zbus::{
    zvariant::{OwnedObjectPath, OwnedValue, Value},
    Connection, Proxy,
};

const NM: &str = "org.freedesktop.NetworkManager";
const ROOT: &str = "/org/freedesktop/NetworkManager";
type Settings<'a> = HashMap<&'static str, HashMap<&'static str, Value<'a>>>;

#[derive(Clone, Debug)]
pub struct Lease {
    pub interface: String,
    pub lease_id: String,
    pub connection_uuid: String,
}
impl Lease {
    fn validate(&self) -> Result<()> {
        if self.interface.is_empty()
            || self.interface.len() > 15
            || !self
                .interface
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            bail!("Invalid leased interface");
        }
        if self.lease_id.len() != 32
            || !self.lease_id.bytes().all(|b| b.is_ascii_hexdigit())
            || self.connection_uuid.len() != 36
            || !self
                .connection_uuid
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b'-')
        {
            bail!("Invalid network lease identity");
        }
        Ok(())
    }
    fn name(&self) -> String {
        format!("linuxdrop-{}", self.lease_id)
    }
}

/// The permit stays held until asynchronous cleanup finishes.
pub struct Guard {
    pub address: Option<Ipv4Addr>,
    pub frequency: i32,
    lease: Lease,
    permit: Option<OwnedSemaphorePermit>,
    connection: Option<Connection>,
}
impl Drop for Guard {
    fn drop(&mut self) {
        let lease = self.lease.clone();
        let permit = self.permit.take();
        let connection = self.connection.take();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = tokio::time::timeout(Duration::from_secs(15), remove_owned(&lease)).await;
                drop(connection);
                drop(permit);
            });
        }
    }
}

fn profile<'a>(
    lease: &Lease,
    ssid: &'a str,
    password: &'a str,
    host: bool,
    ipv6_link_local: bool,
) -> Result<Settings<'a>> {
    lease.validate()?;
    if ssid.is_empty()
        || ssid.len() > 32
        || ssid.contains('\0')
        || !(8..=63).contains(&password.len())
    {
        bail!("Invalid WPA2 network credentials");
    }
    let connection = HashMap::from([
        ("id", Value::from(lease.name())),
        ("uuid", Value::from(lease.connection_uuid.clone())),
        ("type", Value::from("802-11-wireless")),
        ("interface-name", Value::from(lease.interface.clone())),
        ("autoconnect", Value::from(false)),
    ]);
    let mut wifi = HashMap::from([
        ("ssid", Value::from(ssid.as_bytes().to_vec())),
        (
            "mode",
            Value::from(if host { "ap" } else { "infrastructure" }),
        ),
    ]);
    if host {
        wifi.insert("band", Value::from("bg"));
    } else {
        wifi.insert("hidden", Value::from(true));
    }
    let security = HashMap::from([
        ("key-mgmt", Value::from("wpa-psk")),
        ("psk", Value::from(password)),
    ]);
    let ipv4 = HashMap::from([
        ("method", Value::from(if host { "shared" } else { "auto" })),
        ("may-fail", Value::from(!host)),
        ("never-default", Value::from(true)),
        ("ignore-auto-dns", Value::from(true)),
        ("ignore-auto-routes", Value::from(true)),
    ]);
    Ok(HashMap::from([
        ("connection", connection),
        ("802-11-wireless", wifi),
        ("802-11-wireless-security", security),
        ("ipv4", ipv4),
        (
            "ipv6",
            HashMap::from([
                (
                    "method",
                    Value::from(if host || ipv6_link_local {
                        "link-local"
                    } else {
                        "auto"
                    }),
                ),
                ("never-default", Value::from(true)),
                ("ignore-auto-dns", Value::from(true)),
                ("ignore-auto-routes", Value::from(true)),
                ("may-fail", Value::from(true)),
            ]),
        ),
    ]))
}

/// A caller's cancellation drops the guarded activation in a detached worker.
/// The guard exists before the D-Bus mutation; its connection-bound volatile
/// profile also disappears if the caller exits while activation is in flight.
pub async fn connect(
    lease: Lease,
    ssid: String,
    password: String,
    host: bool,
    semaphore: Arc<Semaphore>,
) -> Result<Guard> {
    connect_with_ipv6(lease, ssid, password, host, semaphore, false).await
}

/// Link-local mode is selected only when offered IPv6 candidates require no RA.
pub async fn connect_with_ipv6(
    lease: Lease,
    ssid: String,
    password: String,
    host: bool,
    semaphore: Arc<Semaphore>,
    ipv6_link_local: bool,
) -> Result<Guard> {
    let permit = semaphore
        .try_acquire_owned()
        .context("The reserved adapter is already in use by another transfer")?;
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let cancel = tokio_util::sync::CancellationToken::new();
    let cancel_on_drop = cancel.clone().drop_guard();
    tokio::spawn(async move {
        let result = tokio::select! {
            result=connect_inner(lease,ssid,password,host,permit,ipv6_link_local)=>result,
            _=cancel.cancelled()=>Err(anyhow::anyhow!("Network activation cancelled")),
        };
        let _ = sender.send(result);
    });
    let result = receiver
        .await
        .context("Network activation worker stopped")?;
    cancel_on_drop.disarm();
    result
}

async fn connect_inner(
    lease: Lease,
    ssid: String,
    password: String,
    host: bool,
    permit: OwnedSemaphorePermit,
    ipv6_link_local: bool,
) -> Result<Guard> {
    let settings = profile(&lease, &ssid, &password, host, ipv6_link_local)?;
    let bus = Connection::system().await?;
    let manager = Proxy::new(&bus, NM, ROOT, NM).await?;
    let path: OwnedObjectPath = manager
        .call("GetDeviceByIpIface", &(lease.interface.as_str(),))
        .await?;
    let device = Proxy::new(
        &bus,
        NM,
        path.clone(),
        "org.freedesktop.NetworkManager.Device",
    )
    .await?;
    if device.get_property::<u32>("State").await? != 30 {
        bail!("Wi-Fi upgrade refuses an active or unavailable adapter");
    }
    ensure_uuid_free(&bus, &lease).await?;
    let mut guard = Guard {
        address: None,
        frequency: 0,
        lease: lease.clone(),
        permit: Some(permit),
        connection: Some(bus.clone()),
    };
    let options = HashMap::from([
        ("persist", Value::from("volatile")),
        ("bind-activation", Value::from("dbus-client")),
    ]);
    let root = OwnedObjectPath::try_from("/")?;
    let (_profile, _active, _): (
        OwnedObjectPath,
        OwnedObjectPath,
        HashMap<String, OwnedValue>,
    ) = manager
        .call(
            "AddAndActivateConnection2",
            &(settings, path, root, options),
        )
        .await?;
    let connected = async {
        loop {
            let state: u32 = device.get_property("State").await?;
            if state == 120 {
                bail!("NetworkManager could not activate the reserved network");
            }
            if state == 100 {
                for (property, interface) in [
                    ("Ip4Config", "org.freedesktop.NetworkManager.IP4Config"),
                    ("Ip6Config", "org.freedesktop.NetworkManager.IP6Config"),
                ] {
                    if host && property == "Ip6Config" {
                        continue;
                    }
                    let ip_path: OwnedObjectPath = device.get_property(property).await?;
                    if ip_path.as_str() == "/" {
                        continue;
                    }
                    let ip = Proxy::new(&bus, NM, ip_path, interface).await?;
                    let addresses: Vec<HashMap<String, OwnedValue>> =
                        ip.get_property("AddressData").await?;
                    for address in addresses {
                        if let Some(value) = address
                            .get("address")
                            .and_then(|v| <&str>::try_from(v).ok())
                            .and_then(|v| v.parse::<IpAddr>().ok())
                            .filter(|v| {
                                !v.is_unspecified() && !v.is_loopback() && !v.is_multicast()
                            })
                        {
                            match value {
                                IpAddr::V4(ip) if !ip.is_broadcast() => {
                                    return Ok::<_, anyhow::Error>(Some(ip))
                                }
                                IpAddr::V6(ip) if !host && ip.to_ipv4_mapped().is_none() => {
                                    return Ok(None)
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    };
    guard.address = tokio::time::timeout(Duration::from_secs(45), connected)
        .await
        .context("Reserved network did not acquire an address in time")??;
    let wireless = Proxy::new(
        &bus,
        NM,
        device.path().clone(),
        "org.freedesktop.NetworkManager.Device.Wireless",
    )
    .await?;
    let ap_path: OwnedObjectPath = wireless.get_property("ActiveAccessPoint").await?;
    let ap = Proxy::new(
        &bus,
        NM,
        ap_path,
        "org.freedesktop.NetworkManager.AccessPoint",
    )
    .await?;
    guard.frequency = ap.get_property::<u32>("Frequency").await? as i32;
    Ok(guard)
}

async fn profiles(bus: &Connection) -> Result<Vec<OwnedObjectPath>> {
    let proxy = Proxy::new(
        bus,
        NM,
        format!("{ROOT}/Settings"),
        "org.freedesktop.NetworkManager.Settings",
    )
    .await?;
    Ok(proxy.call("ListConnections", &()).await?)
}
async fn profile_identity(
    bus: &Connection,
    path: OwnedObjectPath,
) -> Result<(Proxy<'_>, HashMap<String, HashMap<String, OwnedValue>>)> {
    let proxy = Proxy::new(
        bus,
        NM,
        path,
        "org.freedesktop.NetworkManager.Settings.Connection",
    )
    .await?;
    let value = proxy.call("GetSettings", &()).await?;
    Ok((proxy, value))
}
async fn ensure_uuid_free(bus: &Connection, lease: &Lease) -> Result<()> {
    for path in profiles(bus).await? {
        let (_, settings) = profile_identity(bus, path).await?;
        if settings
            .get("connection")
            .and_then(|v| v.get("uuid"))
            .and_then(|v| <&str>::try_from(v).ok())
            == Some(&lease.connection_uuid)
        {
            bail!("Reserved profile still exists; wait for cleanup or inspect hardware recovery");
        }
    }
    Ok(())
}
pub async fn remove_owned(lease: &Lease) -> Result<()> {
    lease.validate()?;
    let bus = Connection::system().await?;
    for path in profiles(&bus).await? {
        let (proxy, settings) = match profile_identity(&bus, path.clone()).await {
            Ok(identity) => identity,
            Err(error) => {
                if !profiles(&bus).await?.contains(&path) {
                    continue;
                }
                return Err(error);
            }
        };
        let Some(connection) = settings.get("connection") else {
            continue;
        };
        let text = |key: &str| connection.get(key).and_then(|v| <&str>::try_from(v).ok());
        if text("uuid") != Some(&lease.connection_uuid) {
            continue;
        }
        if text("id") != Some(lease.name().as_str())
            || text("interface-name") != Some(&lease.interface)
        {
            bail!("Network profile ownership changed; refusing to delete it");
        }
        // Another cleanup owner (the protocol guard or netd) can deactivate
        // concurrently. Re-read live state before treating a disappearance as
        // a recovery failure; never swallow an error on a still-owned object.
        let manager = zbus::proxy::Builder::<Proxy<'_>>::new(&bus)
            .destination(NM)?
            .path(ROOT)?
            .interface(NM)?
            .cache_properties(zbus::proxy::CacheProperties::No)
            .build()
            .await?;
        let active: Vec<OwnedObjectPath> = manager.get_property("ActiveConnections").await?;
        for active in active {
            let status = Proxy::new(
                &bus,
                NM,
                active.clone(),
                "org.freedesktop.NetworkManager.Connection.Active",
            )
            .await?;
            let uuid = match status.get_property::<String>("Uuid").await {
                Ok(uuid) => uuid,
                Err(error) => {
                    if !manager
                        .get_property::<Vec<OwnedObjectPath>>("ActiveConnections")
                        .await?
                        .contains(&active)
                    {
                        continue;
                    }
                    return Err(error.into());
                }
            };
            if uuid == lease.connection_uuid {
                if let Err(error) = manager
                    .call::<_, _, ()>("DeactivateConnection", &(active.clone(),))
                    .await
                {
                    if manager
                        .get_property::<Vec<OwnedObjectPath>>("ActiveConnections")
                        .await?
                        .contains(&active)
                    {
                        return Err(error.into());
                    }
                }
            }
        }
        if profiles(&bus)
            .await?
            .iter()
            .any(|path| path.as_str() == proxy.path().as_str())
        {
            if let Err(error) = proxy.call::<_, _, ()>("Delete", &()).await {
                if profiles(&bus)
                    .await?
                    .iter()
                    .any(|path| path.as_str() == proxy.path().as_str())
                {
                    return Err(error.into());
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn temporary_network_has_owned_identity_and_cannot_replace_default_routes() {
        let lease = Lease {
            interface: "wlan2".into(),
            lease_id: "a".repeat(32),
            connection_uuid: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into(),
        };
        for host in [false, true] {
            let config = profile(&lease, "DIRECT-test", "12345678", host, false).unwrap();
            assert_eq!(
                <&str>::try_from(&config["connection"]["id"]).unwrap(),
                lease.name()
            );
            assert!(!bool::try_from(&config["connection"]["autoconnect"]).unwrap());
            assert!(bool::try_from(&config["ipv4"]["never-default"]).unwrap());
            assert!(bool::try_from(&config["ipv4"]["ignore-auto-dns"]).unwrap());
        }
        assert!(profile(&lease, "bad", "short", false, false).is_err());
    }
}
