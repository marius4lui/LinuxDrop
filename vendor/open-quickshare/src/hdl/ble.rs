use std::time::{Duration, SystemTime};

use anyhow::{Context, anyhow};
use btleplug::api::{
    AddressType, Central, CentralEvent, Manager as _, Peripheral as _, ScanFilter,
};
use btleplug::platform::{Adapter, Manager};
use futures::stream::StreamExt;
use tokio::sync::broadcast::Sender;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use uuid::{Uuid, uuid};

const SERVICE_UUID_SHARING: Uuid = uuid!("0000fe2c-0000-1000-8000-00805f9b34fb");

const INNER_NAME: &str = "BleListener";

// The radio cannot scan and advertise at the same time.
//
// With LL privacy enabled (BlueZ's KernelExperimental UUID
// 15c0a148-c273-11ea-b3de-0242ac130004) the kernel pauses *every* advertising
// instance for as long as an active scan is running, because active scanning
// turns off address resolution and RPA generation depends on it. Even without
// it, one antenna time-shares scanning against advertising and against any live
// connection.
//
// Scanning back-to-back therefore keeps us permanently off the air as a Quick
// Share receiver: the phone lists us from a cached advertisement but has no
// window to connect into. So duty-cycle instead. A phone that is sharing
// repeats its 0xFE2C beacon for as long as its share sheet is open, so a short
// window every few seconds still catches it well within the 10s alert rate
// limit below.
const SCAN_WINDOW: Duration = Duration::from_secs(3);
const SCAN_PAUSE: Duration = Duration::from_secs(7);
/// Hard cap on any single D-Bus round-trip. The BLE D-Bus client machinery
/// wedged silently once, taking the whole stack quiet with it -- an unbounded
/// await here would freeze the listener loop for good.
const DBUS_CALL_TIMEOUT: Duration = Duration::from_secs(5);

/// Quick Share seems to emit an LE advert every 10 seconds, so don't alert more
/// often than that.
const ALERT_RATE_LIMIT: Duration = Duration::from_secs(10);

pub use linuxdrop_network::bluetooth_airtime::BleScanSuppressor;
pub(crate) use linuxdrop_network::bluetooth_airtime::scanning_suppressed;

pub struct BleListener {
    adapter: Adapter,
    controller: linuxdrop_network::bluetooth_lifetime::ControllerMonitor,
    sender: Sender<()>,
}

impl BleListener {
    pub async fn new(sender: Sender<()>) -> Result<Self, anyhow::Error> {
        let selected = crate::bluetooth_adapter().await?.name().to_owned();
        let controller =
            linuxdrop_network::bluetooth_lifetime::ControllerMonitor::new(&selected).await?;
        let adapter = selected_scanner(&selected).await?;
        Ok(Self {
            adapter,
            controller,
            sender,
        })
    }

    /// Recreate only the scanner; never cancel independent transfer workers.
    pub async fn supervise(
        sender: Sender<()>,
        status: Sender<crate::channel::ChannelMessage>,
        ctk: CancellationToken,
    ) -> Result<(), anyhow::Error> {
        let mut retry = Duration::from_secs(2);
        loop {
            let mut ready = false;
            let created = tokio::select! {
                _ = ctk.cancelled() => return Ok(()),
                result = tokio::time::timeout(DBUS_CALL_TIMEOUT, Self::new(sender.clone())) =>
                    result.context("Bluetooth scanner setup timed out").and_then(|r| r),
            };
            let result = match created {
                Ok(scanner) => {
                    scanner
                        .run_reported(ctk.clone(), Some(status.clone()), &mut ready)
                        .await
                }
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                crate::backend_failure(&status, "bluetooth-listener", &error);
                if crate::lifecycle::cleanup_unconfirmed(&error) {
                    return Err(error);
                }
            }
            if ctk.is_cancelled() {
                return Ok(());
            }
            if ready {
                retry = Duration::from_secs(2);
            }
            tokio::select! {
                _ = ctk.cancelled() => return Ok(()),
                _ = tokio::time::sleep(retry) => {},
            }
            retry = (retry * 2).min(Duration::from_secs(15));
        }
    }

    pub async fn run(self, ctk: CancellationToken) -> Result<(), anyhow::Error> {
        self.run_reported(ctk, None, &mut false).await
    }

    async fn run_reported(
        self,
        ctk: CancellationToken,
        status: Option<Sender<crate::channel::ChannelMessage>>,
        made_ready: &mut bool,
    ) -> Result<(), anyhow::Error> {
        info!("{INNER_NAME}: service starting");

        let mut events = self.adapter.events().await?;
        // Filter on the NearyShare/QuickShare services UUID

        // Not using the ScanFilter here to filter out advertisements
        // not matching the Nearby Share service UUID, it seems to
        // exclude Nearby Share advertisements despite its UUID being
        // in the filter.
        //
        // Perhaps broken?
        //
        // ...The filtering is being done only here now.

        let mut last_alert: SystemTime = SystemTime::UNIX_EPOCH;
        let mut scanning = false;
        let mut scan_turn = None;
        let mut reported = None;
        let mut report = |paused: bool| {
            *made_ready = true;
            if reported == Some(paused) {
                return;
            }
            reported = Some(paused);
            if let Some(status) = &status {
                let _ = status.send(crate::channel::ChannelMessage {
                    id: "backend".into(),
                    msg: crate::channel::Message::BluetoothScannerReady {
                        adapter: self.controller.name().into(),
                        paused,
                    },
                });
            }
        };
        let loss = self.controller.lost();
        tokio::pin!(loss);
        // Fires immediately, which opens the first scan window.
        let mut phase_deadline = Instant::now();

        let outcome: Result<(), anyhow::Error> = async {
        loop {
            // While a window is open, wake early every 500ms so a suppressor
            // appearing mid-window (a slot-0 fetch or weave session starting)
            // aborts the scan immediately instead of at the window's end.
            let wake = if scanning {
                phase_deadline.min(Instant::now() + Duration::from_millis(500))
            } else {
                phase_deadline
            };
            tokio::select! {
                error = &mut loss => return Err(error),
                _ = ctk.cancelled() => {
                    info!("{INNER_NAME}: tracker cancelled, breaking");
                    break;
                }
                _ = tokio::time::sleep_until(wake) => {
                    if scanning && Instant::now() < phase_deadline && !scanning_suppressed() {
                        // Mid-window early check: nothing changed, keep going.
                        continue;
                    }
                    if scanning {
                        tokio::time::timeout(DBUS_CALL_TIMEOUT, self.adapter.stop_scan()).await
                            .context("Bluetooth stop-scan acknowledgement timed out")?
                            .context("Bluetooth scan could not stop")?;
                        scanning = false;
                        if let Some(mut turn) = scan_turn.take() { super::scan_turn::ScanTurn::cleared(&mut turn); }
                        phase_deadline = Instant::now() + SCAN_PAUSE;
                    } else if scanning_suppressed() {
                        report(true);
                        // Something else needs the radio; skip this window.
                        phase_deadline = Instant::now() + SCAN_PAUSE;
                    } else if self.phone_connected().await {
                        report(true);
                        // A phone is connected to us -- almost certainly a
                        // Quick Share GATT fetch or transfer in flight. Scan
                        // windows were measured stretching its ATT round-trips
                        // from ~30ms to ~370ms, enough to blow the phone-side
                        // fetch timeout, so stay off the air until it's done.
                        phase_deadline = Instant::now() + SCAN_PAUSE;
                    } else {
                        // The service may process StartDiscovery even if its
                        // reply fails. Keep cleanup required until StopDiscovery
                        // acknowledges it; do not report another scan as active.
                        let turn = tokio::select! {
                            error = &mut loss => return Err(error),
                            turn = super::scan_turn::ScanTurn::acquire(&ctk) => turn?,
                        };
                        let Some(mut turn) = turn else { break; };
                        self.controller.check().await?;
                        if scanning_suppressed() { report(true); phase_deadline = Instant::now() + SCAN_PAUSE; continue; }
                        turn.started();
                        scan_turn = Some(turn);
                        scanning = true;
                        // The D-Bus client bounds the request itself. Keep the
                        // future until its receipt before issuing StopDiscovery.
                        self.adapter.start_scan(ScanFilter::default()).await
                            .context("Bluetooth scan could not start")?;
                        if !ctk.is_cancelled() { report(false); }
                        phase_deadline = Instant::now() + SCAN_WINDOW;
                    }
                }
                event = events.next() => {
                    let Some(e) = event else { anyhow::bail!("Bluetooth scanner event stream ended"); };
                    match e {
                        CentralEvent::ServiceDataAdvertisement { id, service_data } => {
                            // Sanity check as per: https://github.com/Martichou/rquickshare/issues/74
                            // Seems like the filtering is not enough, so we'll add a check before
                            // proceeding with the service_data.
                            if let Some(service_data) = service_data.get(&SERVICE_UUID_SHARING) {
                                if self.alert(&mut last_alert) {
                                    debug!("{INNER_NAME}: A device ({id}) is sharing ({}) nearby", hex::encode(service_data));
                                }
                            }
                        },
                        CentralEvent::DeviceDiscovered(id) => {
                            // BlueZ reports the first advertisement of a device
                            // it isn't already caching through InterfacesAdded,
                            // which btleplug turns into DeviceDiscovered and
                            // *not* into a ServiceDataAdvertisement. Every scan
                            // window that follows a cache eviction would
                            // otherwise drop its first beacon, so read the
                            // service data off the peripheral instead.
                            let Ok(peripheral) = self.adapter.peripheral(&id).await else {
                                continue;
                            };
                            let Ok(Some(props)) = peripheral.properties().await else {
                                continue;
                            };
                            if let Some(service_data) = props.service_data.get(&SERVICE_UUID_SHARING) {
                                if self.alert(&mut last_alert) {
                                    debug!("{INNER_NAME}: A device ({id}) is sharing ({}) nearby", hex::encode(service_data));
                                }
                            }
                        },
                        // Not interesting for us
                        _ => {
                            // trace!("{INNER_NAME}: Another CentralEvent got the same services: {:?}", e);
                        }
                    }
                }
            }
        }

        Ok(())
        }.await;
        let cleanup = if scanning {
            match tokio::time::timeout(DBUS_CALL_TIMEOUT, self.adapter.stop_scan()).await {
                Ok(result) => result.context("Bluetooth scan cleanup failed"),
                Err(error) => Err(anyhow::Error::new(error)
                    .context("Bluetooth stop-scan acknowledgement timed out")),
            }
        } else {
            Ok(())
        };
        if let Some(mut turn) = scan_turn {
            if cleanup.is_ok() {
                turn.cleared();
            } else {
                super::scan_turn::ScanTurn::defer_cleanup(self.adapter.clone(), turn);
            }
        }
        crate::lifecycle::finish(outcome, cleanup)
    }

    /// Is a phone currently connected to us over LE?
    ///
    /// Phones use resolvable private addresses: type Random with the top two
    /// bits of the most significant octet reading 0b01. Static-random gadgets
    /// (mice, pads) read 0b11 there and public-address devices are excluded by
    /// the type, so neither keeps this true while idle-connected.
    async fn phone_connected(&self) -> bool {
        // Bounded as a whole: on a wedged D-Bus round-trip, "no phone" and a
        // normal scan window beat freezing the listener loop forever.
        tokio::time::timeout(DBUS_CALL_TIMEOUT, self.phone_connected_inner())
            .await
            .unwrap_or(false)
    }

    async fn phone_connected_inner(&self) -> bool {
        let Ok(peripherals) = self.adapter.peripherals().await else {
            return false;
        };
        for peripheral in peripherals {
            if !matches!(peripheral.is_connected().await, Ok(true)) {
                continue;
            }
            let Ok(Some(props)) = peripheral.properties().await else {
                continue;
            };
            if props.address_type == Some(AddressType::Random)
                && props.address.into_inner()[0] & 0xC0 == 0x40
            {
                return true;
            }
        }
        false
    }

    /// Pokes the mDNS server so it re-announces us, at most once per
    /// [`ALERT_RATE_LIMIT`]. Returns whether the alert was actually sent.
    fn alert(&self, last_alert: &mut SystemTime) -> bool {
        let now = SystemTime::now();
        // A clock that went backwards shouldn't wedge the listener, so treat an
        // un-orderable pair of timestamps as "long enough ago".
        if matches!(now.duration_since(*last_alert), Ok(d) if d <= ALERT_RATE_LIMIT) {
            return false;
        }

        *last_alert = now;
        // The only receiver is the mDNS server; if it's gone there's nothing to
        // poke, but that's no reason to tear the listener down.
        self.sender.send(()).is_ok()
    }
}

/// Open one scanner generation on the explicitly selected controller.
pub(crate) async fn selected_scanner(selected: &str) -> anyhow::Result<Adapter> {
    let manager = Manager::new().await?;
    let adapters = manager.adapters().await?;
    if adapters.is_empty() {
        return Err(anyhow!("no bluetooth adapter"));
    }

    // Resolve through the same powered-controller policy as advertising,
    // GATT and L2CAP; never silently take the first scanner adapter.
    let mut chosen = None;
    for adapter in adapters {
        let information = adapter.adapter_info().await?;
        let name = information
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .rsplit('/')
            .next()
            .unwrap_or_default();
        if name == selected {
            chosen = Some(adapter);
            break;
        }
    }
    chosen.ok_or_else(|| anyhow!("Selected Bluetooth controller is unavailable to the scanner"))
}
