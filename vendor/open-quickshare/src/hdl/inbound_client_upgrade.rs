//! Receiver-initiated role reversal: the sender hosts, the receiver joins.
use super::*;
use crate::hdl::{JoinGuard, MigratableStream};
use location_nearby_connections::bandwidth_upgrade_negotiation_frame::{
    ClientIntroduction, EventType, UpgradePathInfo,
    upgrade_path_info::{Medium, UpgradePathRequest},
};
use location_nearby_connections::v1_frame::FrameType;
use std::future::Future;
use tokio::time::Instant;

fn negotiation(
    frame: &OfflineFrame,
) -> Option<&location_nearby_connections::BandwidthUpgradeNegotiationFrame> {
    frame
        .v1
        .as_ref()
        .filter(|v| v.r#type() == FrameType::BandwidthUpgradeNegotiation)
        .and_then(|v| v.bandwidth_upgrade_negotiation.as_ref())
}

impl InboundRequest<MigratableStream> {
    /// Read encrypted traffic while retaining local cancellation and partial
    /// frames. The caller can race this future against network configuration.
    pub(super) async fn next_upgrade_frame(
        &mut self,
        deadline: Instant,
    ) -> anyhow::Result<Option<OfflineFrame>> {
        loop {
            tokio::select! {
                biased;
                message = self.receiver.recv() => {
                    let message = message.context("Upgrade control channel closed or lagged")?;
                    if message.id != self.state.id && message.id != "*" { continue; }
                    match message.msg {
                        channel::Message::Lib { action: TransferAction::TransferCancel | TransferAction::ConsentDecline } => {
                            self.update_state(|state| state.state = TransferState::Cancelled, true).await;
                            let _ = tokio::time::timeout(Duration::from_secs(2), self.disconnection()).await;
                            return Err(anyhow!(crate::errors::AppError::NotAnError));
                        }
                        _ => continue,
                    }
                }
                _ = tokio::time::sleep_until(deadline) => return Ok(None),
                bytes = self.framing.read(&mut self.socket) => {
                    return Ok(Some(self.decrypt_offline_frame(&bytes?).await?));
                }
            }
        }
    }

    /// Continue consuming BLE payload/control while the dedicated radio joins,
    /// TCP connects, or the peer acknowledges our introduction. The outer error
    /// ends a damaged/cancelled session; the inner error permits BLE fallback.
    async fn during_upgrade<T>(
        &mut self,
        operation: impl Future<Output = anyhow::Result<T>>,
        deadline: Instant,
    ) -> anyhow::Result<anyhow::Result<T>> {
        tokio::pin!(operation);
        loop {
            tokio::select! {
                result = &mut operation => {
                    if result.is_err() && self.bwu_peer_last_write {
                        anyhow::bail!("New channel failed after the peer ended its prior channel");
                    }
                    return Ok(result);
                },
                frame = self.next_upgrade_frame(deadline) => {
                    let Some(frame) = frame? else {
                        anyhow::ensure!(!self.bwu_peer_last_write, "Peer ended its prior channel before upgrade timed out");
                        return Ok(Err(anyhow!("Dedicated-network upgrade timed out")));
                    };
                    if negotiation(&frame).is_some_and(|bwu| bwu.event_type() == EventType::LastWriteToPriorChannel) {
                        self.bwu_peer_last_write = true;
                        continue;
                    }
                    anyhow::ensure!(!self.bwu_peer_last_write, "Traffic after prior-channel LAST_WRITE before handoff");
                    if negotiation(&frame).is_some_and(|bwu| bwu.event_type() == EventType::UpgradeFailure) {
                        return Ok(Err(anyhow!("Peer declined the bandwidth upgrade")));
                    }
                    self.process_offline_frame(frame).await?;
                }
            }
        }
    }

    async fn decline_peer_upgrade(&mut self, medium: Option<i32>) -> anyhow::Result<()> {
        self.encrypt_and_send(&Self::bwu_frame(
            EventType::UpgradeFailure,
            Some(UpgradePathInfo {
                medium,
                ..Default::default()
            }),
            None,
        ))
        .await?;
        self.schedule_bwu_retry();
        Ok(())
    }

    pub(super) async fn request_peer_upgrade(&mut self) -> anyhow::Result<()> {
        // Radio changes require accepted file consent and our actual advertised
        // endpoint identity. Never substitute the session UUID or peer identity.
        if self.state.state != TransferState::ReceivingFiles {
            return Ok(());
        }
        let Some(endpoint_id) = self.local_endpoint_id else {
            self.schedule_bwu_retry();
            return Ok(());
        };
        let mediums = crate::hdl::client_mediums_for(
            &crate::hdl::upgrade_capabilities(),
            &self.remote_mediums,
            self.remote_metadata.as_ref(),
        );
        if mediums.is_empty() {
            self.schedule_bwu_retry();
            return Ok(());
        }
        self.encrypt_and_send(&Self::bwu_frame(
            EventType::UpgradePathRequest,
            Some(UpgradePathInfo {
                upgrade_path_request: Some(UpgradePathRequest {
                    mediums: mediums.clone(),
                    medium_meta_data: Some(crate::hdl::upgrade_metadata()),
                }),
                ..Default::default()
            }),
            None,
        ))
        .await?;

        let deadline = Instant::now() + Duration::from_secs(45);
        let offer = loop {
            let Some(frame) = self.next_upgrade_frame(deadline).await? else {
                return self.decline_peer_upgrade(None).await;
            };
            if let Some(bwu) = negotiation(&frame) {
                match bwu.event_type() {
                    EventType::UpgradePathAvailable => {
                        if let Some(offer) = &bwu.upgrade_path_info {
                            if offer.medium.is_some_and(|medium| mediums.contains(&medium)) {
                                break offer.clone();
                            }
                        }
                        return self
                            .decline_peer_upgrade(
                                bwu.upgrade_path_info.as_ref().and_then(|info| info.medium),
                            )
                            .await;
                    }
                    EventType::UpgradeFailure => {
                        self.schedule_bwu_retry();
                        return Ok(());
                    }
                    _ => {}
                }
            }
            self.process_offline_frame(frame).await?;
        };
        let medium = offer.medium;
        let setup = async {
            let (guard, candidates) = match offer.medium() {
                Medium::WifiDirect => {
                    let credentials = offer
                        .wifi_direct_credentials
                        .as_ref()
                        .ok_or_else(|| anyhow!("Direct upgrade has no credentials"))?;
                    let candidates = crate::hdl::direct_candidates(credentials)?;
                    let guard = if credentials.ssid().is_empty() {
                        anyhow::ensure!(
                            !credentials.device_name().is_empty(),
                            "Direct upgrade has no device name"
                        );
                        crate::hdl::join_p2p(
                            credentials.device_name(),
                            credentials.pin(),
                            u32::try_from(credentials.frequency()).unwrap_or(0),
                        )
                        .await?
                    } else {
                        crate::hdl::join_wifi(
                            credentials.ssid(),
                            credentials.password(),
                            &candidates,
                        )
                        .await?
                    };
                    (guard, candidates)
                }
                Medium::WifiHotspot => {
                    let credentials = offer
                        .wifi_hotspot_credentials
                        .as_ref()
                        .ok_or_else(|| anyhow!("Hotspot upgrade has no credentials"))?;
                    let candidates = crate::hdl::hotspot_candidates(credentials)?;
                    let guard = crate::hdl::join_wifi(
                        credentials.ssid(),
                        credentials.password(),
                        &candidates,
                    )
                    .await?;
                    (guard, candidates)
                }
                _ => anyhow::bail!("Unrequested bandwidth upgrade medium"),
            };
            let mut tcp = crate::hdl::connect_joined(&guard.interface, &candidates).await?;
            let intro = Self::bwu_frame(
                EventType::ClientIntroduction,
                None,
                Some(ClientIntroduction {
                    endpoint_id: Some(std::str::from_utf8(&endpoint_id)?.to_owned()),
                    supports_disabling_encryption: Some(false),
                }),
            );
            send_frame_on(&mut tcp, &intro.encode_to_vec()).await?;
            if offer.supports_client_introduction_ack() {
                let bytes = tokio::time::timeout(Duration::from_secs(5), read_frame_from(&mut tcp))
                    .await??;
                let frame = OfflineFrame::decode(bytes.as_slice())?;
                anyhow::ensure!(
                    frame.version == Some(1)
                        && negotiation(&frame).is_some_and(|bwu| bwu.event_type()
                            == EventType::ClientIntroductionAck
                            && bwu.client_introduction_ack.is_some()),
                    "Invalid bandwidth upgrade introduction acknowledgment"
                );
            }
            Ok::<(JoinGuard, TcpStream), anyhow::Error>((guard, tcp))
        };
        let (guard, tcp) = match self
            .during_upgrade(setup, Instant::now() + Duration::from_secs(80))
            .await?
        {
            Ok(joined) => joined,
            Err(error) => {
                debug!("Receiver client upgrade unavailable: {error}");
                return self.decline_peer_upgrade(medium).await;
            }
        };
        // From LAST_WRITE onward it is unsafe to resume the old stream if the
        // handoff fails. Propagate that failure so the session closes honestly.
        self.drain_prior_channel().await?;
        let disconnect = OfflineFrame {
            version: Some(1),
            v1: Some(location_nearby_connections::V1Frame {
                r#type: Some(FrameType::Disconnection.into()),
                disconnection: Some(location_nearby_connections::DisconnectionFrame {
                    request_safe_to_disconnect: Some(false),
                    ack_safe_to_disconnect: Some(false),
                }),
                ..Default::default()
            }),
        };
        let _ = self.send_frame(disconnect.encode_to_vec()).await;
        self.join_guard = Some(guard);
        self.socket = MigratableStream::Tcp(tcp);
        self.bwu_retry_at = None;
        self.send_keepalive(false).await?;
        info!("BWU: receiver joined sender-hosted network; encrypted payload continues over TCP");
        Ok(())
    }
}
