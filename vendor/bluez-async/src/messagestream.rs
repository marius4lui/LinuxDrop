use dbus::Message;
use dbus::nonblock::{MsgMatch, SyncConnection};
use futures::{Stream, task::AtomicWaker};
use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

// One budget across all signal matches in a subscription. Serialized bytes bound
// retained payloads; libdbus/allocator metadata and one incoming message are extra.
const MAX_MESSAGES: usize = 256;
const MAX_BYTES: usize = 2 * 1024 * 1024;

#[derive(Default)]
struct Pending {
    messages: VecDeque<(Message, usize)>,
    bytes: usize,
    closed: bool,
}
#[derive(Default)]
struct SignalQueue {
    pending: Mutex<Pending>,
    waker: AtomicWaker,
}
impl SignalQueue {
    fn push(&self, message: Message) -> bool {
        let mut pending = self.pending.lock().unwrap();
        if pending.closed {
            return false;
        }
        let mut size = 0;
        let measured = message.marshal(|data| {
            size += data.len();
            if size > MAX_BYTES { Err(()) } else { Ok(()) }
        });
        if measured.is_err()
            || pending.messages.len() >= MAX_MESSAGES
            || size > MAX_BYTES - pending.bytes
        {
            // Any dropped signal can invalidate device state. End the entire
            // subscription, discard its stale backlog, and let its owner rebuild.
            pending.closed = true;
            pending.messages.clear();
            pending.bytes = 0;
            log::warn!("Bluetooth signal queue exceeded its resource limit; subscription ended");
            drop(pending);
            self.waker.wake();
            return false;
        }
        pending.bytes += size;
        pending.messages.push_back((message, size));
        drop(pending);
        self.waker.wake();
        true
    }
    fn poll(&self, cx: &mut Context<'_>) -> Poll<Option<Message>> {
        self.waker.register(cx.waker());
        let mut pending = self.pending.lock().unwrap();
        if pending.closed {
            return Poll::Ready(None);
        }
        match pending.messages.pop_front() {
            Some((message, size)) => {
                pending.bytes -= size;
                Poll::Ready(Some(message))
            }
            None => Poll::Pending,
        }
    }
    fn close(&self) {
        let mut pending = self.pending.lock().unwrap();
        pending.closed = true;
        pending.messages.clear();
        pending.bytes = 0;
        drop(pending);
        self.waker.wake();
    }
}

/// A bounded subscription shared by all matching rules. An overflow terminates
/// the stream rather than silently dropping events from only one rule.
pub struct MessageStream {
    matches: Vec<MsgMatch>,
    events: Arc<SignalQueue>,
    connection: Arc<SyncConnection>,
    resource: Arc<crate::ResourceLifetime>,
}
impl MessageStream {
    pub(crate) fn new(
        connection: Arc<SyncConnection>,
        resource: Arc<crate::ResourceLifetime>,
    ) -> Self {
        Self {
            matches: Vec::new(),
            events: Arc::default(),
            connection,
            resource,
        }
    }
    pub(crate) fn add_match(&mut self, msg_match: MsgMatch) {
        let queue = self.events.clone();
        self.matches
            .push(msg_match.msg_cb(move |message| queue.push(message)));
    }
}
impl Stream for MessageStream {
    type Item = Message;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Message>> {
        self.events.poll(cx)
    }
}
impl Drop for MessageStream {
    fn drop(&mut self) {
        self.events.close();
        let connection = self.connection.clone();
        let matches = std::mem::take(&mut self.matches);
        let resource = self.resource.clone();
        tokio::spawn(async move {
            let _resource = resource;
            for msg_match in matches {
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
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn signal(value: Vec<u8>) -> Message {
        Message::new_signal(
            "/org/bluez/hci0",
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
        )
        .unwrap()
        .append1(value)
    }
    #[test]
    fn overflow_ends_subscription_and_discards_stale_backlog() {
        let queue = SignalQueue::default();
        for _ in 0..MAX_MESSAGES {
            assert!(queue.push(signal(vec![7])));
        }
        assert!(!queue.push(signal(vec![8])));
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        assert!(matches!(queue.poll(&mut cx), Poll::Ready(None)));
        assert_eq!(queue.pending.lock().unwrap().bytes, 0);
        assert!(!queue.push(signal(vec![9])));
    }
    #[test]
    fn byte_limit_is_shared_and_consumption_releases_budget_in_order() {
        let queue = SignalQueue::default();
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        // Sustained consumption must not accumulate lifetime byte accounting.
        for n in 0..600u32 {
            assert!(queue.push(signal(n.to_le_bytes().to_vec())));
            let Poll::Ready(Some(message)) = queue.poll(&mut cx) else {
                panic!("missing signal")
            };
            assert_eq!(message.read1::<Vec<u8>>().unwrap(), n.to_le_bytes());
        }
        assert_eq!(queue.pending.lock().unwrap().bytes, 0);
        assert!(queue.push(signal(vec![0; MAX_BYTES / 2])));
        assert!(!queue.push(signal(vec![0; MAX_BYTES / 2])));
        assert!(matches!(queue.poll(&mut cx), Poll::Ready(None)));
    }
    #[test]
    fn oversized_single_signal_is_never_retained() {
        let queue = SignalQueue::default();
        assert!(!queue.push(signal(vec![0; MAX_BYTES + 1])));
        assert!(queue.pending.lock().unwrap().messages.is_empty());
    }
}
