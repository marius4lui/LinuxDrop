//! Preserve engine cleanup classification around the shared protocol scheduler.
pub(super) use linuxdrop_network::bluetooth_airtime::sender_requested;
pub(super) struct Turn(linuxdrop_network::bluetooth_airtime::Turn);
impl Turn {
    pub(super) async fn acquire(
        sender: bool,
        cancel: &tokio_util::sync::CancellationToken,
    ) -> anyhow::Result<Option<Self>> {
        linuxdrop_network::bluetooth_airtime::Turn::acquire(sender, cancel)
            .await
            .map(|turn| turn.map(Self))
            .map_err(crate::lifecycle::cleanup_failure)
    }
    pub(super) fn registering(&mut self) {
        self.0.registering();
    }
    pub(super) fn cleared(&mut self) {
        self.0.cleared();
    }
}
