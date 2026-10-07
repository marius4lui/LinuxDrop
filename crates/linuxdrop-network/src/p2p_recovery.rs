//! Durable, credential-free provenance for an interrupted formation request.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FormationIdentity {
    pub service_owner: String,
    pub bus_guid: String,
    pub parent_interface: String,
    pub parent_object: String,
    pub existing_interfaces: Vec<String>,
}

impl FormationIdentity {
    pub(super) fn from_setup(
        device: &Proxy<'_>,
        existing: &[OwnedObjectPath],
        interface: &str,
    ) -> Self {
        let mut existing_interfaces: Vec<_> = existing.iter().map(ToString::to_string).collect();
        existing_interfaces.sort();
        Self {
            service_owner: device.destination().to_string(),
            bus_guid: device.connection().server_guid().to_string(),
            parent_interface: interface.into(),
            parent_object: device.path().to_string(),
            existing_interfaces,
        }
    }

    pub(super) fn verify_setup(
        &self,
        device: &Proxy<'_>,
        existing: &[OwnedObjectPath],
        interface: &str,
    ) -> Result<()> {
        anyhow::ensure!(
            *self == Self::from_setup(device, existing, interface),
            "Supplicant formation provenance changed before submission"
        );
        Ok(())
    }
}

async fn uncached<'a>(
    connection: &'a Connection,
    owner: &'a str,
    path: &'a str,
    interface: &'a str,
) -> Result<Proxy<'a>> {
    Ok(zbus::proxy::Builder::<Proxy<'_>>::new(connection)
        .destination(owner)?
        .path(path)?
        .interface(interface)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .await?)
}

pub async fn prepare_formation(interface: &str) -> Result<FormationIdentity> {
    validate(interface, "LinuxDrop", "", 0)?;
    let connection = Connection::system().await?;
    tokio::time::timeout(Duration::from_secs(5), prepare_on(&connection, interface)).await?
}

pub(super) async fn prepare_on(
    connection: &Connection,
    interface: &str,
) -> Result<FormationIdentity> {
    let owner = owner::current(connection).await?;
    let root = uncached(connection, &owner, "/fi/w1/wpa_supplicant1", SERVICE).await?;
    let parent: OwnedObjectPath = root.call("GetInterface", &(interface,)).await?;
    let device = uncached(connection, &owner, parent.as_str(), DEVICE).await?;
    let group: OwnedObjectPath = device.get_property("Group").await?;
    anyhow::ensure!(
        group.as_str() == "/",
        "Reserved adapter already has a P2P group"
    );
    let existing: Vec<OwnedObjectPath> = root.get_property("Interfaces").await?;
    anyhow::ensure!(
        existing.contains(&parent),
        "Reserved adapter is absent from supplicant inventory"
    );
    anyhow::ensure!(
        owner::current(connection).await? == owner,
        "Supplicant owner changed during preparation"
    );
    Ok(FormationIdentity::from_setup(&device, &existing, interface))
}

/// Cancel only the original reserved device's pending formation. Never infer
/// ownership of an unidentified group and never issue Disconnect here.
pub async fn recover_formation(identity: &FormationIdentity) -> Result<()> {
    let connection = Connection::system().await?;
    tokio::time::timeout(Duration::from_secs(10), recover_on(&connection, identity)).await?
}

pub(super) async fn recover_on(
    connection: &Connection,
    identity: &FormationIdentity,
) -> Result<()> {
    anyhow::ensure!(
        connection.server_guid().as_str() == identity.bus_guid,
        "Recovery bus differs from the recorded formation bus"
    );
    anyhow::ensure!(
        owner::current(connection).await? == identity.service_owner,
        "Recovery refuses a replacement supplicant owner"
    );
    let before = prepare_on(connection, &identity.parent_interface).await?;
    anyhow::ensure!(
        before == *identity,
        "Formation inventory or parent changed; ownership is uncertain"
    );
    let device = uncached(
        connection,
        &identity.service_owner,
        &identity.parent_object,
        DEVICE,
    )
    .await?;
    device
        .call::<_, _, ()>("Cancel", &())
        .await
        .context("Formation cancellation was not acknowledged")?;
    let after = prepare_on(connection, &identity.parent_interface).await?;
    anyhow::ensure!(
        after == *identity,
        "Formation inventory changed during recovery"
    );
    Ok(())
}
