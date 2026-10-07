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

const SERVICE: &str = "fi.w1.wpa_supplicant1";
const DEVICE: &str = "fi.w1.wpa_supplicant1.Interface.P2PDevice";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GroupIdentity {
    pub interface: String,
    pub interface_object: String,
    pub group_object: String,
    pub parent_interface: String,
    pub peer_object: String,
}

/// Retain this guard until transfer completion. Drop disconnects only this group.
pub struct Group {
    pub identity: GroupIdentity,
    connection: Connection,
    armed: bool,
    verify: fn(&GroupIdentity) -> Result<()>,
}
impl Group {
    /// Called only after netd durably journals the identity for crash recovery.
    pub fn into_journaled(mut self) -> GroupIdentity {
        self.armed = false;
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
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                let _ = disconnect_checked(&connection, &identity, verify).await;
            });
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
    let connection = Connection::system().await?;
    let root = Proxy::new(&connection, SERVICE, "/fi/w1/wpa_supplicant1", SERVICE).await?;
    let parent: OwnedObjectPath = root
        .call("GetInterface", &(interface.as_str(),))
        .await
        .context("Reserved adapter is not managed by wpa_supplicant")?;
    let device = Proxy::new(&connection, SERVICE, parent.as_str(), DEVICE).await?;
    let group: OwnedObjectPath = device.get_property("Group").await?;
    if group.as_str() != "/" {
        bail!("Reserved adapter already has a P2P group");
    }
    let existing: Vec<OwnedObjectPath> = root.get_property("Interfaces").await?;
    let mut started = device.receive_signal("GroupStarted").await?;
    let mut find = HashMap::new();
    find.insert("Timeout", Value::from(20i32));
    find.insert("DiscoveryType", Value::from("start_with_full"));
    device.call::<_, _, ()>("Find", &(find,)).await?;
    let operation = async {
        let peer = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let paths: Vec<OwnedObjectPath> = device.get_property("Peers").await?;
                let mut matches = Vec::new();
                for path in paths.into_iter().take(256) {
                    let peer = Proxy::new(
                        &connection,
                        SERVICE,
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
            SERVICE,
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
        };
        let guard = Group {
            identity,
            connection: connection.clone(),
            armed: true,
            verify: verify_phy,
        };
        verify_phy(&guard.identity)?;
        Ok::<_, anyhow::Error>(guard)
    };
    let result = tokio::select! { _ = cancel.cancelled() => Err(anyhow::anyhow!("P2P connection cancelled")), result = operation => result };
    let _ = device.call::<_, _, ()>("StopFind", &()).await;
    if result.is_err() {
        let _ = device.call::<_, _, ()>("Cancel", &()).await;
    }
    result
}

/// Credentials come from the running group; they are never persisted in the
/// lease journal or supplied in process arguments. Debug deliberately redacts them.
pub struct HostedGroup {
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
            create_group_inner(connection, &interface, frequency, cancel, verify_phy).await
        }
        .await;
        let _ = sender.send(result);
    });
    let result = receiver.await.context("P2P group owner worker stopped")?;
    cancel_on_drop.disarm();
    result
}

async fn create_group_inner(
    connection: Connection,
    interface: &str,
    frequency: u32,
    cancel: CancellationToken,
    verify: fn(&GroupIdentity) -> Result<()>,
) -> Result<HostedGroup> {
    let root = Proxy::new(&connection, SERVICE, "/fi/w1/wpa_supplicant1", SERVICE).await?;
    let parent: OwnedObjectPath = root
        .call("GetInterface", &(interface,))
        .await
        .context("Reserved adapter is not managed by wpa_supplicant")?;
    let device = Proxy::new(&connection, SERVICE, parent.as_str(), DEVICE).await?;
    let group: OwnedObjectPath = device.get_property("Group").await?;
    anyhow::ensure!(
        group.as_str() == "/",
        "Reserved adapter already has a P2P group"
    );
    let existing: Vec<OwnedObjectPath> = root.get_property("Interfaces").await?;
    let mut started = device.receive_signal("GroupStarted").await?;
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
        let identity = started_identity(&connection, interface, &existing, properties).await?;
        // Own cleanup before reading any further properties, including invalid
        // credentials or an unexpected operating channel.
        let group = Group {
            identity,
            connection: connection.clone(),
            armed: true,
            verify,
        };
        verify(&group.identity)?;
        let details = Proxy::new(
            &connection,
            SERVICE,
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
        Ok::<_, anyhow::Error>(HostedGroup {
            group,
            ssid,
            password,
            frequency: actual,
        })
    };
    let result = tokio::select! {
        _ = cancel.cancelled() => Err(anyhow::anyhow!("P2P group creation cancelled")),
        result = tokio::time::timeout(Duration::from_secs(45), operation) => result.unwrap_or_else(|_| Err(anyhow::anyhow!("P2P group creation timed out"))),
    };
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
                if let Ok(identity) =
                    started_identity(&connection, interface, &existing, properties).await
                {
                    if verify(&identity).is_ok() {
                        let _ = disconnect_checked(&connection, &identity, verify).await;
                    }
                }
            }
        }
    }
    result
}

async fn started_identity(
    connection: &Connection,
    parent: &str,
    existing: &[OwnedObjectPath],
    mut properties: HashMap<String, OwnedValue>,
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
        role == "GO" && !existing.contains(&object) && group.as_str() != "/",
        "Supplicant returned an existing or unexpected P2P group"
    );
    let proxy = Proxy::new(
        connection,
        SERVICE,
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

/// Crash recovery may call this with a root-owned journal record. Both the
/// current interface name and group object must still match before disconnect.
pub async fn disconnect(connection: &Connection, identity: &GroupIdentity) -> Result<()> {
    disconnect_checked(connection, identity, verify_phy).await
}
async fn disconnect_checked(
    connection: &Connection,
    identity: &GroupIdentity,
    verify: fn(&GroupIdentity) -> Result<()>,
) -> Result<()> {
    let interface = Proxy::new(
        connection,
        SERVICE,
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
        SERVICE,
        identity.interface_object.as_str(),
        DEVICE,
    )
    .await?;
    let group: OwnedObjectPath = device.get_property("Group").await?;
    if group.as_str() != identity.group_object {
        bail!("P2P group ownership changed");
    }
    device.call::<_, _, ()>("Disconnect", &()).await?;
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
