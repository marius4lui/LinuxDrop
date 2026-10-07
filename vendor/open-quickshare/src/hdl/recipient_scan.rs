//! Owned, acknowledged recipient scans; dropping a caller cannot drop cleanup.
use super::{BleTarget, decode_receiver_advert, scan_turn::ScanTurn, selected_scanner};
use btleplug::api::{AddressType, Central, CentralEvent, Peripheral as _, ScanFilter};
use futures::{FutureExt, StreamExt};
use std::{collections::HashMap, time::Duration};
use tokio_util::sync::CancellationToken;
use uuid::uuid;
static SCANS: once_cell::sync::Lazy<tokio_util::task::TaskTracker> =
    once_cell::sync::Lazy::new(|| {
        let tracker = tokio_util::task::TaskTracker::new();
        tracker.close();
        tracker
    });
pub async fn wait_for_scans() -> anyhow::Result<()> {
    tokio::time::timeout(Duration::from_secs(15), SCANS.wait())
        .await
        .map_err(crate::lifecycle::cleanup_failure)?;
    super::scan_turn::ScanTurn::confirm_idle().await
}

pub(super) async fn scan(
    name: String,
    window: Duration,
    wanted: Option<[u8; 4]>,
    parent: CancellationToken,
    background: bool,
) -> anyhow::Result<Vec<BleTarget>> {
    let cancel = parent.child_token();
    let _cancel_on_drop = cancel.clone().drop_guard();
    let Some(turn) = ScanTurn::acquire(&cancel).await? else {
        anyhow::bail!("Bluetooth scan cancelled");
    };
    // The worker owns StartDiscovery and StopDiscovery even when a transfer
    // task is aborted. No new scan can take its turn before acknowledgement.
    SCANS
        .spawn(worker(name, window, wanted, cancel, background, turn))
        .await
        .map_err(crate::lifecycle::cleanup_failure)?
}
async fn worker(
    name: String,
    window: Duration,
    wanted: Option<[u8; 4]>,
    cancel: CancellationToken,
    background: bool,
    mut turn: ScanTurn,
) -> anyhow::Result<Vec<BleTarget>> {
    let setup = async {
        let monitor = linuxdrop_network::bluetooth_lifetime::ControllerMonitor::new(&name).await?;
        let adapter = selected_scanner(&name).await?;
        monitor.check().await?;
        Ok::<_, anyhow::Error>((adapter, monitor))
    };
    let (adapter, monitor) = tokio::select! {
        _ = cancel.cancelled() => anyhow::bail!("Bluetooth scan cancelled"),
        result = tokio::time::timeout(Duration::from_secs(5), setup) => result??,
    };
    if background && super::scanning_suppressed() {
        return Ok(Vec::new());
    }
    let mut events = adapter.events().await?;
    // btleplug synthesizes DeviceDiscovered for cached peripherals. Drain those
    // before StartDiscovery so stale advertisements cannot refresh presence.
    let mut cached = 0;
    while let Some(Some(_)) = events.next().now_or_never() {
        cached += 1;
        if cached > 4096 {
            anyhow::bail!("Bluetooth event backlog exceeded its limit");
        }
    }
    if cancel.is_cancelled() {
        anyhow::bail!("Bluetooth scan cancelled");
    }
    turn.started();
    let mut found = HashMap::new();
    let operation = async {
        // Await the method receipt: cancelling the caller never abandons a
        // StartDiscovery request that bluetoothd may already have processed.
        adapter.start_scan(ScanFilter::default()).await?;
        monitor.check().await?;
        let lost = monitor.lost(); tokio::pin!(lost);
        let deadline = tokio::time::sleep(window); tokio::pin!(deadline);
        let mut pause = tokio::time::interval(Duration::from_millis(250));
        loop {
            let id = tokio::select! {
                biased;
                _ = cancel.cancelled() => anyhow::bail!("Bluetooth scan cancelled"),
                error = &mut lost => return Err(error),
                _ = &mut deadline => break,
                _ = pause.tick() => { if background && super::scanning_suppressed() { break; } else { continue; } },
                event = events.next() => match event {
                    Some(CentralEvent::DeviceDiscovered(id) | CentralEvent::DeviceUpdated(id) | CentralEvent::ServiceDataAdvertisement {id, ..}) => id,
                    Some(_) => continue,
                    None => anyhow::bail!("Bluetooth recipient event stream ended"),
                }
            };
            let read = async { adapter.peripheral(&id).await?.properties().await };
            let properties = tokio::select! {
                _ = cancel.cancelled() => anyhow::bail!("Bluetooth scan cancelled"),
                result = tokio::time::timeout(Duration::from_secs(5), read) => match result { Ok(Ok(Some(properties))) => properties, _ => continue },
            };
            let Some(data) = properties.service_data.get(&uuid!("0000fef3-0000-1000-8000-00805f9b34fb")) else { continue; };
            let Some(advert) = decode_receiver_advert(data) else { continue; };
            if !advert.visible || wanted.is_some_and(|wanted| wanted != advert.endpoint_id) { continue; }
            let address_type = match properties.address_type {
                Some(AddressType::Public) => bluer::AddressType::LePublic,
                Some(AddressType::Random) => bluer::AddressType::LeRandom,
                None => continue,
            };
            let Some(target) = advert.into_target(properties.address.to_string().parse()?, address_type) else { continue; };
            // Accept current scan events, not a stale BlueZ cache snapshot.
            if found.len() < 512 || found.contains_key(&target.endpoint_id) { found.insert(target.endpoint_id, target); }
            if wanted.is_some() && !found.is_empty() { break; }
        }
        Ok(())
    }.await;
    let cleanup = tokio::time::timeout(Duration::from_secs(5), adapter.stop_scan())
        .await
        .map_err(anyhow::Error::from)
        .and_then(|result| result.map_err(anyhow::Error::from));
    if cleanup.is_ok() {
        turn.cleared();
    } else {
        ScanTurn::defer_cleanup(adapter, turn);
    }
    crate::lifecycle::finish(operation, cleanup)?;
    Ok(found.into_values().collect())
}
