//! Restart receiver radio resources together; migrated payloads outlive a generation.
use super::{BluetoothTasks, L2capServer, ReceiverAdvertiser, ReceiverGattServer, Visibility};
use crate::channel::{ChannelMessage, Message};
use linuxdrop_network::bluetooth_lifetime::ControllerMonitor;
use std::time::Duration;
use tokio::sync::{broadcast, watch};
use tokio_util::sync::CancellationToken;

pub async fn supervise_receiver(
    endpoint: [u8; 4],
    device_type: u8,
    name: String,
    port: u16,
    visibility: watch::Receiver<Visibility>,
    status: broadcast::Sender<ChannelMessage>,
    cancel: CancellationToken,
) -> anyhow::Result<()> {
    let sessions = BluetoothTasks::new(&cancel);
    let _cancel_sessions = sessions.cancellation().drop_guard();
    let mut retry = Duration::from_secs(2);
    let result = loop {
        if cancel.is_cancelled() {
            break Ok(());
        }
        for component in ["bluetooth-gatt", "bluetooth-receiver"] {
            crate::backend_failure(
                &status,
                component,
                "Waiting for the selected Bluetooth controller",
            );
        }
        let setup = async {
            let adapter = crate::bluetooth_adapter().await?;
            let monitor = ControllerMonitor::new(adapter.name()).await?;
            let l2cap =
                if std::env::var("PACKET_BLE_L2CAP").is_ok_and(|v| v.eq_ignore_ascii_case("off")) {
                    None
                } else {
                    match L2capServer::bind_adapter(adapter.clone()).await {
                        Ok(listener) => Some(listener),
                        Err(error) => {
                            warn!("L2CAP unavailable; receiving through GATT: {error}");
                            None
                        }
                    }
                };
            let psm = l2cap.as_ref().map(|(_, psm)| *psm);
            let advert = super::receiver_service_data(endpoint, device_type, &name, psm);
            let gatt = ReceiverGattServer::with_adapter(
                adapter.clone(),
                advert.clone(),
                status.clone(),
                port,
            )
            .await?;
            let advertiser =
                ReceiverAdvertiser::with_adapter(adapter, endpoint, device_type, &name, psm)
                    .await?;
            monitor.check().await?;
            Ok::<_, anyhow::Error>((monitor, l2cap, advert, gatt, advertiser))
        };
        let setup = tokio::select! {
            _ = cancel.cancelled() => break Ok(()),
            result = tokio::time::timeout(Duration::from_secs(10), setup) => result.map_err(anyhow::Error::from).and_then(|r| r),
        };
        let mut ready = false;
        let outcome = match setup {
            Err(error) => Err(error),
            Ok((monitor, l2cap, advert, gatt, advertiser)) => {
                let generation = cancel.child_token();
                let _cancel_generation = generation.clone().drop_guard();
                let mut workers = tokio::task::JoinSet::new();
                let mut states = status.subscribe();
                let token = generation.clone();
                let inbound = sessions.clone();
                workers.spawn(async move {
                    (
                        "bluetooth-gatt",
                        gatt.run_with_sessions(token, inbound).await,
                    )
                });
                let token = generation.clone();
                let visible = visibility.clone();
                let events = status.clone();
                workers.spawn(async move {
                    (
                        "bluetooth-receiver",
                        advertiser.run_reported(visible, token, Some(events)).await,
                    )
                });
                if let Some((listener, _)) = l2cap {
                    let token = generation.clone();
                    let inbound = sessions.clone();
                    let events = status.clone();
                    workers.spawn(async move {
                        (
                            "bluetooth-l2cap",
                            listener
                                .run_with_sessions(advert, events, port, token, inbound)
                                .await,
                        )
                    });
                }
                let lost = monitor.lost();
                tokio::pin!(lost);
                let mut failure = loop {
                    tokio::select! {
                        _ = cancel.cancelled() => break None,
                        error = &mut lost => break Some(error),
                        event = states.recv() => {
                            if let Ok(ChannelMessage {msg: Message::BluetoothServiceReady {component}, ..}) = event
                                && component == "bluetooth-gatt" { ready = true; }
                        },
                        ended = workers.join_next() => {
                            break Some(worker_error(ended, &status));
                        },
                    }
                };
                generation.cancel();
                if let Some(error) = &failure {
                    for component in ["bluetooth-gatt", "bluetooth-receiver"] {
                        crate::backend_failure(&status, component, error);
                    }
                }
                // Do not abort: each role must acknowledge owned-resource cleanup.
                while let Some(ended) = workers.join_next().await {
                    let error = match ended {
                        Ok((_, Ok(()))) => continue,
                        Ok((component, Err(error))) => {
                            crate::backend_failure(
                                &status,
                                if component == "bluetooth-gatt" {
                                    component
                                } else {
                                    "bluetooth-receiver"
                                },
                                &error,
                            );
                            error
                        }
                        Err(error) => crate::lifecycle::cleanup_failure(error),
                    };
                    if failure.is_none() || crate::lifecycle::cleanup_unconfirmed(&error) {
                        failure = Some(error);
                    }
                }
                failure.map_or(Ok(()), Err)
            }
        };
        if let Err(error) = outcome {
            crate::backend_failure(&status, "bluetooth-receiver", &error);
            if crate::lifecycle::cleanup_unconfirmed(&error) {
                // Keep migrated transfers alive, but never open a duplicate receiver.
                cancel.cancelled().await;
                break Err(error);
            }
        }
        if cancel.is_cancelled() {
            break Ok(());
        }
        if ready {
            retry = Duration::from_secs(2);
        }
        tokio::select! {
            _ = cancel.cancelled() => break Ok(()),
            _ = tokio::time::sleep(retry) => {},
        }
        retry = (retry * 2).min(Duration::from_secs(15));
    };
    sessions.shutdown().await;
    result
}

fn worker_error(
    ended: Option<Result<(&'static str, anyhow::Result<()>), tokio::task::JoinError>>,
    status: &broadcast::Sender<ChannelMessage>,
) -> anyhow::Error {
    match ended {
        Some(Ok((component, Ok(())))) => anyhow::anyhow!("{component} stopped unexpectedly"),
        Some(Ok((component, Err(error)))) => {
            crate::backend_failure(
                status,
                if component == "bluetooth-gatt" {
                    component
                } else {
                    "bluetooth-receiver"
                },
                &error,
            );
            error
        }
        Some(Err(error)) => crate::lifecycle::cleanup_failure(error),
        None => anyhow::anyhow!("Bluetooth receiver workers ended"),
    }
}
