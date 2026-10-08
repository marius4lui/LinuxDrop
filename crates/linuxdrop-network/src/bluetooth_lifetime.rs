//! Read-only controller generation checks, independent of a transport's cleanup.
use anyhow::{bail, Result};
use std::time::Duration;

pub struct ControllerMonitor {
    connection: zbus::Connection,
    owner: String,
    name: String,
}

impl ControllerMonitor {
    pub async fn new(name: &str) -> Result<Self> {
        let connection = zbus::Connection::system().await?;
        let bus = zbus::fdo::DBusProxy::new(&connection).await?;
        let owner = bus
            .get_name_owner("org.bluez".try_into()?)
            .await?
            .to_string();
        let monitor = Self {
            connection,
            owner,
            name: name.into(),
        };
        monitor.check().await?;
        Ok(monitor)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub async fn check(&self) -> Result<()> {
        tokio::time::timeout(Duration::from_secs(3), async {
            let bus = zbus::fdo::DBusProxy::new(&self.connection).await?;
            // A daemon restart includes a period without any owner. Treat that
            // as loss of this generation, just like a replacement owner; retain
            // other bus failures instead of disguising transport errors.
            let owner = match bus.get_name_owner("org.bluez".try_into()?).await {
                Ok(owner) => owner,
                Err(zbus::fdo::Error::NameHasNoOwner(_)) => {
                    bail!("Bluetooth daemon owner changed (service disappeared)");
                }
                Err(error) => return Err(error.into()),
            };
            if owner.as_str() != self.owner {
                bail!("Bluetooth daemon owner changed");
            }
            let path = format!("/org/bluez/{}", self.name);
            let adapter = zbus::Proxy::new(
                &self.connection,
                self.owner.as_str(),
                path.as_str(),
                "org.bluez.Adapter1",
            )
            .await?;
            if !adapter.get_property::<bool>("Powered").await? {
                bail!("Bluetooth controller {} is switched off", self.name);
            }
            Ok(())
        })
        .await?
    }

    pub async fn lost(&self) -> anyhow::Error {
        loop {
            if let Err(error) = self.check().await {
                return error;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
}
