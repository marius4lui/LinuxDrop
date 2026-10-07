use dbus::Message;
use dbus::nonblock::{MsgMatch, SyncConnection};
use futures::Stream;
use futures::channel::mpsc::UnboundedReceiver;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

/// Wrapper for a stream of D-Bus messages which automatically removes the `MsgMatch` from the D-Bus
/// connection when it is dropped.
pub struct MessageStream {
    msg_match: Option<MsgMatch>,
    events: UnboundedReceiver<Message>,
    connection: Arc<SyncConnection>,
    resource: Arc<crate::ResourceLifetime>,
}

impl MessageStream {
    pub(crate) fn new(
        msg_match: MsgMatch,
        connection: Arc<SyncConnection>,
        resource: Arc<crate::ResourceLifetime>,
    ) -> Self {
        let (msg_match, events) = msg_match.msg_stream();
        Self {
            msg_match: Some(msg_match),
            events,
            connection,
            resource,
        }
    }
}

impl Stream for MessageStream {
    type Item = Message;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.events).poll_next(cx)
    }
}

impl Drop for MessageStream {
    fn drop(&mut self) {
        let connection = self.connection.clone();
        let msg_match = self.msg_match.take().unwrap();
        let resource = self.resource.clone();
        tokio::spawn(async move {
            let _resource = resource;
            if let Ok(Err(error)) = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                connection.remove_match(msg_match.token()),
            )
            .await
            {
                if error.name() != Some("org.freedesktop.DBus.Error.MatchRuleNotFound") {
                    log::warn!("Bluetooth signal subscription cleanup: {error}");
                }
            }
        });
    }
}
