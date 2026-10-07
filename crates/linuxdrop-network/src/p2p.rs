//! Narrow supplicant operations for a radio already exclusively leased by netd.
//! This module neither chooses a radio nor configures addresses/default routes.
use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, time::Duration};
use tokio_util::sync::CancellationToken;
use zbus::{
    zvariant::{OwnedObjectPath, OwnedValue, Value},
    Connection, Proxy,
};

#[path = "p2p_owner.rs"]
mod owner;

pub type FormationError = anyhow::Error;

const SERVICE: &str = "fi.w1.wpa_supplicant1";
const DEVICE: &str = "fi.w1.wpa_supplicant1.Interface.P2PDevice";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GroupIdentity {
    pub interface: String,
    pub interface_object: String,
    pub group_object: String,
    pub parent_interface: String,
    pub peer_object: String,
    #[serde(default)]
    pub service_owner: String,
    #[serde(default)]
    pub bus_guid: String,
}

/// Retain this guard until transfer completion. Drop disconnects only this group.
pub struct Group {
    pub identity: GroupIdentity,
    connection: Connection,
    armed: bool,
    verify: fn(&GroupIdentity) -> Result<()>,
    settled: tokio::sync::watch::Sender<Option<std::result::Result<(), String>>>,
}

/// Completes only once the guard has finished cleanup or transferred ownership
/// to netd's durable journal. Dropping a waiter never cancels the cleanup task.
#[derive(Clone)]
pub struct GroupSettlement {
    identity: GroupIdentity,
    receiver: tokio::sync::watch::Receiver<Option<std::result::Result<(), String>>>,
}
/// A known group still requiring recovery after guard cleanup failed. The
/// identity carries no credentials and lets netd retain the precise resource.
#[derive(Debug)]
pub struct GroupCleanupFailure {
    pub identity: GroupIdentity,
    reason: String,
}
impl std::fmt::Display for GroupCleanupFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "P2P group cleanup failed: {}", self.reason)
    }
}
impl std::error::Error for GroupCleanupFailure {}

impl GroupSettlement {
    pub async fn wait(mut self) -> Result<()> {
        loop {
            if let Some(result) = self.receiver.borrow().clone() {
                return result.map_err(|reason| {
                    GroupCleanupFailure {
                        identity: self.identity.clone(),
                        reason,
                    }
                    .into()
                });
            }
            self.receiver
                .changed()
                .await
                .map_err(|_| GroupCleanupFailure {
                    identity: self.identity.clone(),
                    reason: "worker stopped without acknowledgement".into(),
                })?;
        }
    }
}

impl Group {
    fn new(
        identity: GroupIdentity,
        connection: Connection,
        verify: fn(&GroupIdentity) -> Result<()>,
    ) -> Self {
        let (settled, _) = tokio::sync::watch::channel(None);
        Self {
            identity,
            connection,
            armed: true,
            verify,
            settled,
        }
    }
    pub fn settlement(&self) -> GroupSettlement {
        GroupSettlement {
            identity: self.identity.clone(),
            receiver: self.settled.subscribe(),
        }
    }

    /// Called only after netd durably journals the identity for crash recovery.
    pub fn into_journaled(mut self) -> GroupIdentity {
        self.armed = false;
        self.settled.send_replace(Some(Ok(())));
        self.identity.clone()
    }
}
impl Drop for Group {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let connection = self.connection.clone();
        let identity = self.identity.clone();
        let verify = self.verify;
        let settled = self.settled.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let result = tokio::time::timeout(
                    Duration::from_secs(10),
                    disconnect_checked(&connection, &identity, verify),
                )
                .await
                .context("P2P group cleanup timed out")
                .and_then(|result| result)
                .map_err(|error| format!("{error:#}"));
                settled.send_replace(Some(result));
            });
        } else {
            settled.send_replace(Some(Err("P2P group cleanup has no active runtime".into())));
        }
    }
}

fn validate(interface: &str, name: &str, pin: &str, frequency: u32) -> Result<()> {
    if interface.is_empty()
        || interface.len() > 15
        || !interface
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        bail!("Invalid reserved Wi-Fi interface");
    }
    if name.is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
        bail!("Invalid P2P peer name");
    }
    if !pin.is_empty() && (pin.len() != 8 || !pin.bytes().all(|b| b.is_ascii_digit())) {
        bail!("Wi-Fi Direct requires an eight digit WPS PIN");
    }
    if frequency != 0 && !(2300..=7200).contains(&frequency) {
        bail!("Invalid P2P frequency");
    }
    Ok(())
}

/// `frequency` must also have been checked against the leased radio's regulatory
/// channels by netd. DeviceName is routing metadata; the existing encrypted
/// Nearby session authenticates the upgrade, never the name or Wi-Fi address.
pub async fn connect_wps(
    interface: &str,
    peer_name: &str,
    pin: &str,
    frequency: u32,
    cancel: CancellationToken,
) -> Result<Group> {
    validate(interface, peer_name, pin, frequency)?;
    let interface = interface.to_owned();
    let peer_name = peer_name.to_owned();
    let pin = pin.to_owned();
    let cancel_on_drop = cancel.clone().drop_guard();
    // The worker keeps cleanup alive even if its caller drops the future.
    let (sender, receiver) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let result = connect_inner(interface, peer_name, pin, frequency, cancel).await;
        let _ = sender.send(result); // An abandoned result drops the group guard.
    });
    let result = receiver.await.context("P2P connection worker stopped")?;
    cancel_on_drop.disarm();
    result
}

async fn connect_inner(
    interface: String,
    peer_name: String,
    pin: String,
    frequency: u32,
    cancel: CancellationToken,
) -> Result<Group> {
    connect_on(
        Connection::system().await?,
        interface,
        peer_name,
        pin,
        frequency,
        cancel,
        verify_phy,
    )
    .await
}

async fn connect_on(
    connection: Connection,
    interface: String,
    peer_name: String,
    pin: String,
    frequency: u32,
    cancel: CancellationToken,
    verify: fn(&GroupIdentity) -> Result<()>,
) -> Result<Group> {
    let mut owner = owner::Watch::bind(&connection).await?;
    let destination = owner.name.clone();
    let bus_guid = connection.server_guid().to_string();
    let (device, existing, mut started) = tokio::time::timeout(Duration::from_secs(5), async {
        let root = Proxy::new(
            &connection,
            destination.as_str(),
            "/fi/w1/wpa_supplicant1",
            SERVICE,
        )
        .await?;
        let parent: OwnedObjectPath = root
            .call("GetInterface", &(interface.as_str(),))
            .await
            .context("Reserved adapter is not managed by wpa_supplicant")?;
        let device = Proxy::new(&connection, destination.clone(), parent, DEVICE).await?;
        let group: OwnedObjectPath = device.get_property("Group").await?;
        if group.as_str() != "/" {
            bail!("Reserved adapter already has a P2P group");
        }
        let existing: Vec<OwnedObjectPath> = root.get_property("Interfaces").await?;
        let started = device.receive_signal("GroupStarted").await?;
        Ok::<_, anyhow::Error>((device, existing, started))
    })
    .await
    .context("Supplicant setup timed out")??;
    let mut find = HashMap::new();
    find.insert("Timeout", Value::from(20i32));
    find.insert("DiscoveryType", Value::from("start_with_full"));
    let mut settlement = None;
    let operation = async {
        device.call::<_, _, ()>("Find", &(find,)).await?;
        let peer = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let paths: Vec<OwnedObjectPath> = device.get_property("Peers").await?;
                let mut matches = Vec::new();
                for path in paths.into_iter().take(256) {
                    let peer = Proxy::new(
                        &connection,
                        destination.as_str(),
                        path.as_str(),
                        "fi.w1.wpa_supplicant1.Peer",
                    )
                    .await?;
                    if peer
                        .get_property::<String>("DeviceName")
                        .await
                        .ok()
                        .as_deref()
                        == Some(peer_name.as_str())
                    {
                        matches.push(path);
                    }
                }
                if matches.len() > 1 {
                    bail!("P2P peer name is ambiguous; refusing to choose a different device");
                }
                if let Some(peer) = matches.pop() {
                    break Ok::<_, anyhow::Error>(peer);
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await
        .context("Wi-Fi Direct peer was not discovered within 20 seconds")??;
        device.call::<_, _, ()>("StopFind", &()).await?;
        let mut args = HashMap::new();
        args.insert("peer", Value::from(peer.clone()));
        args.insert("join", Value::from(true));
        args.insert("persistent", Value::from(false));
        args.insert(
            "wps_method",
            Value::from(if pin.is_empty() { "pbc" } else { "keypad" }),
        );
        if !pin.is_empty() {
            args.insert("pin", Value::from(pin.as_str()));
        }
        if frequency != 0 {
            args.insert("frequency", Value::from(frequency as i32));
        }
        let _: String = device.call("Connect", &(args,)).await?;
        let signal = tokio::time::timeout(Duration::from_secs(45), started.next())
            .await
            .context("Wi-Fi Direct group formation timed out")?
            .context("Supplicant stopped during group formation")?;
        let (mut properties,): (HashMap<String, OwnedValue>,) = signal.body().deserialize()?;
        let role = String::try_from(properties.remove("role").context("Missing P2P role")?)?;
        let object = OwnedObjectPath::try_from(
            properties
                .remove("interface_object")
                .context("Missing P2P interface")?,
        )?;
        let group = OwnedObjectPath::try_from(
            properties
                .remove("group_object")
                .context("Missing P2P group")?,
        )?;
        if role != "client" || existing.contains(&object) || group.as_str() == "/" {
            bail!("Supplicant returned an existing or unexpected P2P group");
        }
        let proxy = Proxy::new(
            &connection,
            destination.as_str(),
            object.as_str(),
            "fi.w1.wpa_supplicant1.Interface",
        )
        .await?;
        let group_interface: String = proxy.get_property("Ifname").await?;
        let identity = GroupIdentity {
            interface: group_interface,
            interface_object: object.to_string(),
            group_object: group.to_string(),
            parent_interface: interface.clone(),
            peer_object: peer.to_string(),
            service_owner: destination.clone(),
            bus_guid: bus_guid.clone(),
        };
        let guard = Group::new(identity, connection.clone(), verify);
        settlement = Some(guard.settlement());
        verify(&guard.identity)?;
        Ok::<_, anyhow::Error>(guard)
    };
    let result = tokio::select! {
        biased;
        _ = owner.lost() => Err(anyhow::anyhow!("Supplicant owner changed during P2P connection")),
        _ = cancel.cancelled() => Err(anyhow::anyhow!("P2P connection cancelled")),
        result = tokio::time::timeout(Duration::from_secs(70), operation) => result.unwrap_or_else(|_| Err(anyhow::anyhow!("P2P connection timed out"))),
    };
    let _ = tokio::time::timeout(
        Duration::from_secs(5),
        device.call::<_, _, ()>("StopFind", &()),
    )
    .await;
    let mut late_cleanup = Ok(());
    if result.is_err() {
        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            device.call::<_, _, ()>("Cancel", &()),
        )
        .await;
        if let Ok(Some(signal)) =
            tokio::time::timeout(Duration::from_millis(250), started.next()).await
        {
            if let Ok((properties,)) = signal
                .body()
                .deserialize::<(HashMap<String, OwnedValue>,)>()
            {
                if let Ok(Ok(identity)) = tokio::time::timeout(
                    Duration::from_secs(5),
                    started_identity(
                        &connection,
                        &destination,
                        &interface,
                        &existing,
                        properties,
                        "client",
                    ),
                )
                .await
                {
                    late_cleanup = disconnect_acknowledged(&connection, &identity, verify).await;
                }
            }
        }
    }
    if result.is_err() {
        if let Some(settlement) = settlement {
            settlement
                .wait()
                .await
                .context("P2P client cleanup failed")?;
        }
    }
    late_cleanup?;
    result
}

/// Credentials come from the running group; they are never persisted in the
/// lease journal or supplied in process arguments. Debug deliberately redacts them.
pub struct HostedGroup {
    pub device_name: Option<String>,
    pub group: Group,
    pub ssid: String,
    pub password: String,
    pub frequency: u16,
}
impl std::fmt::Debug for HostedGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostedGroup")
            .field("interface", &self.group.identity.interface)
            .field("frequency", &self.frequency)
            .finish_non_exhaustive()
    }
}

pub async fn create_group(
    interface: &str,
    frequency: u32,
    cancel: CancellationToken,
) -> Result<HostedGroup> {
    create_group_with_auth(interface, frequency, crate::P2pHostAuth::Password, cancel).await
}

pub async fn create_group_with_auth(
    interface: &str,
    frequency: u32,
    auth: crate::P2pHostAuth,
    cancel: CancellationToken,
) -> Result<HostedGroup> {
    validate(interface, "LinuxDrop", "", frequency)?;
    anyhow::ensure!(
        frequency != 0,
        "A P2P group owner needs an explicitly permitted frequency"
    );
    let interface = interface.to_owned();
    let cancel_on_drop = cancel.clone().drop_guard();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let result = async {
            let connection = Connection::system().await?;
            create_group_authenticated(connection, &interface, frequency, auth, cancel, verify_phy)
                .await
        }
        .await;
        let _ = sender.send(result);
    });
    let result = receiver.await.context("P2P group owner worker stopped")?;
    cancel_on_drop.disarm();
    result
}

#[cfg(test)]
async fn create_group_inner(
    connection: Connection,
    interface: &str,
    frequency: u32,
    cancel: CancellationToken,
    verify: fn(&GroupIdentity) -> Result<()>,
) -> Result<HostedGroup> {
    create_group_authenticated(
        connection,
        interface,
        frequency,
        crate::P2pHostAuth::Password,
        cancel,
        verify,
    )
    .await
}

async fn create_group_authenticated(
    connection: Connection,
    interface: &str,
    frequency: u32,
    auth: crate::P2pHostAuth,
    cancel: CancellationToken,
    verify: fn(&GroupIdentity) -> Result<()>,
) -> Result<HostedGroup> {
    let mut owner = owner::Watch::bind(&connection).await?;
    let destination = owner.name.clone();
    let (device, existing, mut started) = tokio::time::timeout(Duration::from_secs(5), async {
        let root = Proxy::new(
            &connection,
            destination.as_str(),
            "/fi/w1/wpa_supplicant1",
            SERVICE,
        )
        .await?;
        let parent: OwnedObjectPath = root
            .call("GetInterface", &(interface,))
            .await
            .context("Reserved adapter is not managed by wpa_supplicant")?;
        let device = Proxy::new(&connection, destination.clone(), parent, DEVICE).await?;
        let group: OwnedObjectPath = device.get_property("Group").await?;
        anyhow::ensure!(
            group.as_str() == "/",
            "Reserved adapter already has a P2P group"
        );
        let existing: Vec<OwnedObjectPath> = root.get_property("Interfaces").await?;
        let started = device.receive_signal("GroupStarted").await?;
        Ok::<_, anyhow::Error>((device, existing, started))
    })
    .await
    .context("Supplicant setup timed out")??;
    let mut settlement = None;
    let operation = async {
        let args: HashMap<&str, Value<'_>> = [
            ("persistent", Value::from(false)),
            ("frequency", Value::from(frequency as i32)),
        ]
        .into_iter()
        .collect();
        device.call::<_, _, ()>("GroupAdd", &(args,)).await?;
        let signal = started
            .next()
            .await
            .context("Supplicant stopped during group creation")?;
        let (properties,): (HashMap<String, OwnedValue>,) = signal.body().deserialize()?;
        let identity = started_identity(
            &connection,
            &destination,
            interface,
            &existing,
            properties,
            "GO",
        )
        .await?;
        // Own cleanup before reading any further properties, including invalid
        // credentials or an unexpected operating channel.
        let group = Group::new(identity, connection.clone(), verify);
        settlement = Some(group.settlement());
        verify(&group.identity)?;
        let details = Proxy::new(
            &connection,
            destination.as_str(),
            group.identity.group_object.as_str(),
            "fi.w1.wpa_supplicant1.Group",
        )
        .await?;
        let role: String = details.get_property("Role").await?;
        anyhow::ensure!(role == "GO", "Supplicant did not create a group owner");
        let ssid = String::from_utf8(details.get_property::<Vec<u8>>("SSID").await?)
            .context("P2P SSID is not UTF-8")?;
        let password: String = details.get_property("Passphrase").await?;
        let actual: u16 = details.get_property("Frequency").await?;
        anyhow::ensure!(
            actual as u32 == frequency,
            "P2P group chose an unapproved frequency"
        );
        anyhow::ensure!(
            !ssid.is_empty() && ssid.len() <= 32 && !ssid.chars().any(char::is_control),
            "Invalid P2P group SSID"
        );
        anyhow::ensure!(
            (8..=63).contains(&password.len()) && password.bytes().all(|b| (32..=126).contains(&b)),
            "Invalid P2P group passphrase"
        );
        let device_name = if auth == crate::P2pHostAuth::DeviceName {
            // Use the existing P2P identity: never rewrite shared parent config.
            let mut config: HashMap<String, OwnedValue> =
                device.get_property("P2PDeviceConfig").await?;
            let name = String::try_from(
                config
                    .remove("DeviceName")
                    .context("Supplicant has no P2P device name")?,
            )?;
            validate(interface, &name, "", frequency)?;
            let wps = Proxy::new(
                &connection,
                destination.as_str(),
                group.identity.interface_object.as_str(),
                "fi.w1.wpa_supplicant1.Interface.WPS",
            )
            .await?;
            let group_name: String = wps.get_property("DeviceName").await?;
            anyhow::ensure!(
                group_name == name,
                "P2P group device name changed during creation"
            );
            // On an AP/GO, supplicant dispatches Type=pbc to its AP registrar.
            // Do not invoke Start on the parent, where it would enroll a station.
            let args: HashMap<&str, Value<'_>> = [
                ("Role", Value::from("registrar")),
                ("Type", Value::from("pbc")),
            ]
            .into_iter()
            .collect();
            let _: HashMap<String, OwnedValue> = wps.call("Start", &(args,)).await?;
            Some(name)
        } else {
            None
        };
        Ok::<_, anyhow::Error>(HostedGroup {
            device_name,
            group,
            ssid,
            password,
            frequency: actual,
        })
    };
    let result = tokio::select! {
        biased;
        _ = owner.lost() => Err(anyhow::anyhow!("Supplicant owner changed during P2P group creation")),
        _ = cancel.cancelled() => Err(anyhow::anyhow!("P2P group creation cancelled")),
        result = tokio::time::timeout(Duration::from_secs(45), operation) => result.unwrap_or_else(|_| Err(anyhow::anyhow!("P2P group creation timed out"))),
    };
    let mut late_cleanup = Ok(());
    if result.is_err() {
        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            device.call::<_, _, ()>("Cancel", &()),
        )
        .await;
        // GroupStarted can race cancellation after GroupAdd has succeeded.
        // Drain that exact new group rather than leaving an unjournaled VIF.
        if let Ok(Some(signal)) =
            tokio::time::timeout(Duration::from_millis(250), started.next()).await
        {
            if let Ok((properties,)) = signal
                .body()
                .deserialize::<(HashMap<String, OwnedValue>,)>()
            {
                if let Ok(Ok(identity)) = tokio::time::timeout(
                    Duration::from_secs(5),
                    started_identity(
                        &connection,
                        &destination,
                        interface,
                        &existing,
                        properties,
                        "GO",
                    ),
                )
                .await
                {
                    late_cleanup = disconnect_acknowledged(&connection, &identity, verify).await;
                }
            }
        }
    }
    if result.is_err() {
        if let Some(settlement) = settlement {
            settlement
                .wait()
                .await
                .context("P2P group-owner cleanup failed")?;
        }
    }
    late_cleanup?;
    result
}

async fn started_identity(
    connection: &Connection,
    destination: &str,
    parent: &str,
    existing: &[OwnedObjectPath],
    mut properties: HashMap<String, OwnedValue>,
    expected_role: &str,
) -> Result<GroupIdentity> {
    let role = String::try_from(properties.remove("role").context("Missing P2P role")?)?;
    let object = OwnedObjectPath::try_from(
        properties
            .remove("interface_object")
            .context("Missing P2P interface")?,
    )?;
    let group = OwnedObjectPath::try_from(
        properties
            .remove("group_object")
            .context("Missing P2P group")?,
    )?;
    anyhow::ensure!(
        role == expected_role && !existing.contains(&object) && group.as_str() != "/",
        "Supplicant returned an existing or unexpected P2P group"
    );
    let proxy = Proxy::new(
        connection,
        destination,
        object.as_str(),
        "fi.w1.wpa_supplicant1.Interface",
    )
    .await?;
    let interface: String = proxy.get_property("Ifname").await?;
    validate(&interface, "LinuxDrop", "", 0)?;
    Ok(GroupIdentity {
        interface,
        interface_object: object.to_string(),
        group_object: group.to_string(),
        parent_interface: parent.into(),
        peer_object: "/".into(),
        service_owner: destination.into(),
        bus_guid: connection.server_guid().to_string(),
    })
}

fn verify_phy(identity: &GroupIdentity) -> Result<()> {
    let parent = std::fs::canonicalize(format!(
        "/sys/class/net/{}/phy80211",
        identity.parent_interface
    ))?;
    let group = std::fs::canonicalize(format!("/sys/class/net/{}/phy80211", identity.interface))?;
    if parent != group {
        bail!("P2P group does not belong to the reserved radio");
    }
    Ok(())
}

/// Snapshot for netd's health monitor; no request is routed to the supplicant.
pub async fn service_instance(connection: &Connection) -> Result<(String, String)> {
    Ok((
        owner::current(connection).await?,
        connection.server_guid().to_string(),
    ))
}

pub async fn service_matches(connection: &Connection, identity: &GroupIdentity) -> bool {
    identity.bus_guid == connection.server_guid().as_str()
        && owner::current(connection)
            .await
            .is_ok_and(|name| name == identity.service_owner)
}

/// Crash recovery uses the journal's original unique owner and bus identity.
/// Never route a stale group path through the replacement well-known owner.
pub async fn disconnect(connection: &Connection, identity: &GroupIdentity) -> Result<()> {
    disconnect_checked(connection, identity, verify_phy).await
}
async fn disconnect_acknowledged(
    connection: &Connection,
    identity: &GroupIdentity,
    verify: fn(&GroupIdentity) -> Result<()>,
) -> Result<()> {
    tokio::time::timeout(
        Duration::from_secs(10),
        disconnect_checked(connection, identity, verify),
    )
    .await
    .context("P2P group cleanup timed out")
    .and_then(|result| result)
    .map_err(|error| {
        GroupCleanupFailure {
            identity: identity.clone(),
            reason: format!("{error:#}"),
        }
        .into()
    })
}

async fn disconnect_checked(
    connection: &Connection,
    identity: &GroupIdentity,
    verify: fn(&GroupIdentity) -> Result<()>,
) -> Result<()> {
    anyhow::ensure!(
        identity.bus_guid == connection.server_guid().as_str(),
        "P2P group belongs to another or unknown D-Bus instance"
    );
    zbus::names::UniqueName::try_from(identity.service_owner.as_str())
        .context("P2P group has no pinned service owner; manual recovery required")?;
    let interface = Proxy::new(
        connection,
        identity.service_owner.as_str(),
        identity.interface_object.as_str(),
        "fi.w1.wpa_supplicant1.Interface",
    )
    .await?;
    if interface.get_property::<String>("Ifname").await? != identity.interface {
        bail!("P2P interface ownership changed");
    }
    verify(identity)?;
    let device = Proxy::new(
        connection,
        identity.service_owner.as_str(),
        identity.interface_object.as_str(),
        DEVICE,
    )
    .await?;
    let group: OwnedObjectPath = device.get_property("Group").await?;
    if group.as_str() != identity.group_object {
        bail!("P2P group ownership changed");
    }
    device.call::<_, _, ()>("Disconnect", &()).await?;
    // A method reply is not the removal observation. A fresh group VIF must
    // disappear from the same pinned owner's live inventory before settling.
    let root = zbus::proxy::Builder::<Proxy<'_>>::new(connection)
        .destination(identity.service_owner.as_str())?
        .path("/fi/w1/wpa_supplicant1")?
        .interface(SERVICE)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await?;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let interfaces: Vec<OwnedObjectPath> = root.get_property("Interfaces").await?;
            if !interfaces
                .iter()
                .any(|object| object.as_str() == identity.interface_object)
            {
                return Ok::<_, anyhow::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .context("Supplicant still reports the disconnected P2P interface")??;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_bounded_wps_credentials_and_interface_names_are_accepted() {
        assert!(validate("wlan2", "Android", "12345670", 2437).is_ok());
        assert!(validate("wlan2", "Android", "", 2437).is_ok());
        assert!(validate("../wlan0", "Android", "12345670", 2437).is_err());
        assert!(validate("wlan0", "Android", "1234", 2437).is_err());
        assert!(validate("wlan0", "Android", "12345670", 99999).is_err());
        assert!(validate("wlan0", "Android\n", "12345670", 0).is_err());
    }
}

#[cfg(test)]
#[path = "p2p_host_tests.rs"]
mod host_tests;
