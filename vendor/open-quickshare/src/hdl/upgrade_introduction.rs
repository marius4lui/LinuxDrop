//! Plaintext routing checks precede, but never replace, encrypted session continuity.
use crate::location_nearby_connections::{
    BandwidthUpgradeNegotiationFrame, OfflineFrame, V1Frame,
    bandwidth_upgrade_negotiation_frame::{ClientIntroductionAck, EventType},
    v1_frame::FrameType,
};
use anyhow::{Context, Result, ensure};
use prost::Message;

pub(super) fn validate(bytes: &[u8], expected: &[u8; 4]) -> Result<()> {
    let frame = OfflineFrame::decode(bytes)?;
    ensure!(
        frame.version == Some(1),
        "Unsupported upgrade introduction version"
    );
    let v1 = frame.v1.context("Missing introduction frame")?;
    ensure!(
        v1.r#type() == FrameType::BandwidthUpgradeNegotiation,
        "Invalid introduction frame type"
    );
    let bwu = v1
        .bandwidth_upgrade_negotiation
        .context("Missing introduction negotiation")?;
    ensure!(
        bwu.event_type() == EventType::ClientIntroduction,
        "Invalid introduction event"
    );
    let intro = bwu
        .client_introduction
        .context("Missing client introduction")?;
    ensure!(
        intro.endpoint_id().as_bytes() == expected,
        "Introduction belongs to another endpoint"
    );
    // supports_disabling_encryption is a capability, never permission to disable it.
    Ok(())
}

#[cfg(all(feature = "experimental", target_os = "linux"))]
pub(super) async fn accept(
    listener: &crate::lan_policy::LanListeners,
    expected: [u8; 4],
    lan: bool,
) -> Result<tokio::net::TcpStream> {
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    loop {
        let (mut socket, peer) = listener.accept().await?;
        if lan
            && !socket
                .local_addr()
                .is_ok_and(|local| crate::lan_policy::permits(local.ip(), peer.ip()))
        {
            continue;
        }
        let introduction = async {
            let length = socket.read_u32().await? as usize;
            ensure!(length > 0 && length <= 1024, "Oversized introduction");
            let mut bytes = vec![0; length];
            socket.read_exact(&mut bytes).await?;
            validate(&bytes, &expected)?;
            let ack = OfflineFrame {
                version: Some(1),
                v1: Some(V1Frame {
                    r#type: Some(FrameType::BandwidthUpgradeNegotiation.into()),
                    bandwidth_upgrade_negotiation: Some(BandwidthUpgradeNegotiationFrame {
                        event_type: Some(EventType::ClientIntroductionAck.into()),
                        client_introduction_ack: Some(ClientIntroductionAck {}),
                        ..Default::default()
                    }),
                    ..Default::default()
                }),
            }
            .encode_to_vec();
            socket.write_u32(ack.len() as u32).await?;
            socket.write_all(&ack).await?;
            socket.flush().await?;
            Ok::<_, anyhow::Error>(())
        };
        if matches!(
            tokio::time::timeout(Duration::from_secs(5), introduction).await,
            Ok(Ok(()))
        ) {
            return Ok(socket);
        }
        // A stray/malformed connection does not close the established transfer.
        // The caller enforces the total offer deadline and consumes BLE traffic.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn introduction_requires_version_type_event_body_and_exact_endpoint() {
        let valid = OfflineFrame { version: Some(1), v1: Some(V1Frame {
            r#type: Some(FrameType::BandwidthUpgradeNegotiation.into()),
            bandwidth_upgrade_negotiation: Some(BandwidthUpgradeNegotiationFrame {
                event_type: Some(EventType::ClientIntroduction.into()),
                client_introduction: Some(crate::location_nearby_connections::bandwidth_upgrade_negotiation_frame::ClientIntroduction {
                    endpoint_id: Some("PEER".into()), supports_disabling_encryption: Some(true)
                }), ..Default::default()
            }), ..Default::default()
        })};
        assert!(validate(&valid.encode_to_vec(), b"PEER").is_ok());
        assert!(validate(&valid.encode_to_vec(), b"NOPE").is_err());
        for case in 0..5 {
            let mut invalid = valid.clone();
            match case {
                0 => invalid.version = Some(99),
                1 => invalid.v1.as_mut().unwrap().r#type = Some(FrameType::KeepAlive.into()),
                2 => {
                    invalid
                        .v1
                        .as_mut()
                        .unwrap()
                        .bandwidth_upgrade_negotiation
                        .as_mut()
                        .unwrap()
                        .event_type = Some(EventType::ClientIntroductionAck.into())
                }
                3 => {
                    invalid
                        .v1
                        .as_mut()
                        .unwrap()
                        .bandwidth_upgrade_negotiation
                        .as_mut()
                        .unwrap()
                        .client_introduction = None
                }
                _ => invalid.v1 = None,
            }
            assert!(validate(&invalid.encode_to_vec(), b"PEER").is_err());
        }
        assert!(validate(&[0xff], b"PEER").is_err());
    }
    #[test]
    fn discovery_identity_comes_from_service_bytes_not_display_name_or_address() {
        let mut endpoint = crate::hdl::EndpointInfo {
            fullname: format!(
                "{}._FC9F5ED42C8A._tcp.local.",
                crate::utils::gen_mdns_name(*b"PEER")
            ),
            name: Some("Friendly name".into()),
            id: "192.0.2.1:99".into(),
            ..Default::default()
        };
        assert_eq!(endpoint.endpoint_id(), Some(*b"PEER"));
        endpoint.fullname = "PEER".into();
        assert_eq!(endpoint.endpoint_id(), None);
        endpoint.fullname = "ble://PEER".into();
        assert_eq!(endpoint.endpoint_id(), None);
    }
}
