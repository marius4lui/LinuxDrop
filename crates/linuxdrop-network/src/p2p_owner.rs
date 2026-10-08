//! Subscribe before resolving the owner so loss cannot hide between the two.
use super::*;

pub(super) struct Watch {
    pub name: String,
    changes: zbus::MessageStream,
}

pub(super) async fn current(connection: &Connection) -> Result<String> {
    let bus = zbus::fdo::DBusProxy::new(connection).await?;
    Ok(tokio::time::timeout(
        Duration::from_secs(3),
        bus.get_name_owner(SERVICE.try_into()?),
    )
    .await??
    .to_string())
}

impl Watch {
    pub async fn bind(connection: &Connection) -> Result<Self> {
        let rule = zbus::MatchRule::builder()
            .msg_type(zbus::message::Type::Signal)
            .sender("org.freedesktop.DBus")?
            .path("/org/freedesktop/DBus")?
            .interface("org.freedesktop.DBus")?
            .member("NameOwnerChanged")?
            .add_arg(SERVICE)?
            .build();
        let changes = zbus::MessageStream::for_match_rule(rule, connection, Some(16)).await?;
        let name = current(connection).await?;
        Ok(Self { name, changes })
    }
    pub async fn lost(&mut self) {
        while let Some(Ok(message)) = self.changes.next().await {
            if let Ok((name, old, new)) = message.body().deserialize::<(String, String, String)>() {
                if name == SERVICE && old == self.name && new != self.name {
                    return;
                }
            }
        }
    }
}
