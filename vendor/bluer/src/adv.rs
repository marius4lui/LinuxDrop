//! Bluetooth LE advertising.

use dbus::{
    arg::{PropMap, RefArg, Variant},
    nonblock::Proxy,
};
use dbus_crossroads::{Crossroads, IfaceBuilder, IfaceToken};
use futures::channel::oneshot;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt,
    sync::Arc,
    time::Duration,
};
use strum::{Display, EnumString};
use uuid::Uuid;

use crate::{read_dict, Adapter, Result, SessionInner, SERVICE_NAME};

pub(crate) const MANAGER_INTERFACE: &str = "org.bluez.LEAdvertisingManager1";
pub(crate) const ADVERTISEMENT_INTERFACE: &str = "org.bluez.LEAdvertisement1";
pub(crate) const ADVERTISEMENT_PREFIX: &str = publish_path!("advertising/");

/// Determines the type of advertising packet requested.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Hash, Display, EnumString)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Type {
    /// Broadcast
    #[strum(serialize = "broadcast")]
    Broadcast,
    /// Peripheral
    #[strum(serialize = "peripheral")]
    Peripheral,
}

impl Default for Type {
    fn default() -> Self {
        Self::Peripheral
    }
}

/// Secondary channel for advertisement.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Hash, Display, EnumString)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum SecondaryChannel {
    /// 1M
    #[strum(serialize = "1M")]
    OneM,
    /// 2M
    #[strum(serialize = "2M")]
    TwoM,
    /// Coded
    #[strum(serialize = "Coded")]
    Coded,
}

impl Default for SecondaryChannel {
    fn default() -> Self {
        Self::OneM
    }
}

/// Advertisement feature.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Hash, Display, EnumString)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Feature {
    /// TX power.
    #[strum(serialize = "tx-power")]
    TxPower,
    /// Appearance.
    #[strum(serialize = "appearance")]
    Appearance,
    /// Local name.
    #[strum(serialize = "local-name")]
    LocalName,
}

/// LE advertising platform feature.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord, Hash, Display, EnumString)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum PlatformFeature {
    /// Indicates whether platform can
    /// specify TX power on each
    /// advertising instance.
    #[strum(serialize = "CanSetTxPower")]
    CanSetTxPower,
    /// Indicates whether multiple
    /// advertising will be offloaded
    /// to the controller.
    #[strum(serialize = "HardwareOffload")]
    HardwareOffload,
}

/// Advertising-related controller capabilities.
#[derive(Clone, Debug, Default, Eq, PartialEq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct Capabilities {
    /// Maximum advertising data length.
    pub max_advertisement_length: u8,
    /// Maximum advertising scan response length.
    pub max_scan_response_length: u8,
    /// Minimum advertising TX power (dBm).
    pub min_tx_power: i16,
    /// Maximum advertising TX power (dBm).
    pub max_tx_power: i16,
}

impl Capabilities {
    pub(crate) fn from_dict(
        dict: &HashMap<String, Variant<Box<dyn RefArg + 'static>>>,
    ) -> Result<Self> {
        Ok(Self {
            max_advertisement_length: *read_dict(dict, "MaxAdvLen")?,
            max_scan_response_length: *read_dict(dict, "MaxScnRspLen")?,
            min_tx_power: *read_dict(dict, "MinTxPower")?,
            max_tx_power: *read_dict(dict, "MaxTxPower")?,
        })
    }
}

/// Bluetooth LE advertisement data definition.
///
/// Specifies the Advertisement Data to be broadcast and some advertising
/// parameters.  Properties which are not present will not be included in the
/// data.  Required advertisement data types will always be included.
/// All UUIDs are 128-bit versions in the API, and 16 or 32-bit
/// versions of the same UUID will be used in the advertising data as appropriate.
///
/// Use [Adapter::advertise] to register a new advertisement.
#[derive(Clone, Debug, Default, Eq, PartialEq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Advertisement {
    /// Determines the type of advertising packet requested.
    pub advertisement_type: Type,
    /// List of UUIDs to include in the "Service UUID" field of
    /// the Advertising Data.
    pub service_uuids: BTreeSet<Uuid>,
    /// Manufacturer Data fields to include in
    /// the Advertising Data.
    ///
    /// Keys are the Manufacturer ID
    /// to associate with the data.
    pub manufacturer_data: BTreeMap<u16, Vec<u8>>,
    /// Array of UUIDs to include in "Service Solicitation"
    /// Advertisement Data.
    pub solicit_uuids: BTreeSet<Uuid>,
    /// Service Data elements to include.
    ///
    /// The keys are the
    /// UUID to associate with the data.
    pub service_data: BTreeMap<Uuid, Vec<u8>>,
    /// Advertising Type to include in the Advertising
    /// Data.
    ///
    /// Key is the advertising type and value is the
    /// data as byte array.
    ///
    /// Note: Types already handled by other properties shall
    /// not be used.
    pub advertising_data: BTreeMap<u8, Vec<u8>>,
    /// Advertise as general discoverable.
    ///
    /// When present this
    /// will override adapter Discoverable property.
    ///
    /// Note: This property shall not be set when Type is set
    /// to broadcast. Additionally, Types that are official
    /// Bluetooth assigned numbers cannot be used. So for
    /// example the Type value of 0x0a cannot be used because
    /// it is assigned as the TX Power Level Type. But,
    /// currently the Type 0x0c is unassigned and can be used.
    pub discoverable: Option<bool>,
    /// The discoverable timeout in seconds.
    ///
    /// A value of zero
    /// means that the timeout is disabled and it will stay in
    /// discoverable/limited mode forever.
    ///
    /// Note: This property shall not be set when Type is set
    /// to broadcast.
    pub discoverable_timeout: Option<Duration>,
    /// List of system features to be included in the advertising
    /// packet.
    pub system_includes: BTreeSet<Feature>,
    /// Local name to be used in the advertising report.
    ///
    /// If the
    /// string is too big to fit into the packet it will be
    /// truncated.
    pub local_name: Option<String>,
    /// Appearance to be used in the advertising report.
    pub appearance: Option<u16>,
    /// Duration of the advertisement in seconds.
    ///
    /// If there are
    /// other applications advertising no duration is set the
    /// default is 2 seconds.
    pub duration: Option<Duration>,
    /// Timeout of the advertisement in seconds.
    ///
    /// This defines
    /// the lifetime of the advertisement.
    pub timeout: Option<Duration>,
    /// Secondary channel to be used.
    ///
    /// Primary channel is
    /// always set to "1M" except when "Coded" is set.
    pub secondary_channel: Option<SecondaryChannel>,
    /// Minimum advertising interval to be used by the
    /// advertising set, in milliseconds.
    ///
    /// Acceptable values
    /// are in the range [20ms, 10,485s]. If the provided
    /// MinInterval is larger than the provided MaxInterval,
    /// the registration will return failure.
    pub min_interval: Option<Duration>,
    /// Maximum advertising interval to be used by the
    /// advertising set, in milliseconds.
    ///
    /// Acceptable values
    /// are in the range [20ms, 10,485s]. If the provided
    /// MinInterval is larger than the provided MaxInterval,
    /// the registration will return failure.
    pub max_interval: Option<Duration>,
    /// Requested transmission power of this advertising set.
    ///
    /// The provided value is used only if the "CanSetTxPower"
    /// feature is enabled on the Advertising Manager. The
    /// provided value must be in range [-127 to +20], where
    /// units are in dBm.
    pub tx_power: Option<i16>,
    #[doc(hidden)]
    pub _non_exhaustive: (),
}

pub(crate) struct RegisteredAdvertisement {
    advertisement: Advertisement,
    owner: String,
    released: tokio::sync::watch::Sender<bool>,
}
impl std::ops::Deref for RegisteredAdvertisement {
    type Target = Advertisement;
    fn deref(&self) -> &Advertisement {
        &self.advertisement
    }
}

impl Advertisement {
    pub(crate) fn register_interface(cr: &mut Crossroads) -> IfaceToken<RegisteredAdvertisement> {
        cr.register(ADVERTISEMENT_INTERFACE, |ib: &mut IfaceBuilder<RegisteredAdvertisement>| {
            ib.method("Release", (), (), |ctx, advertisement, ()| {
                if !ctx.message().sender().is_some_and(|sender| sender.as_ref() == advertisement.owner) {
                    return Err(dbus::MethodErr::failed("Release is only accepted from the registering BlueZ owner"));
                }
                advertisement.released.send_replace(true);
                Ok(())
            });
            cr_property!(ib, "Type", la => {
                Some(la.advertisement_type.to_string())
            });
            cr_property!(ib, "ServiceUUIDs", la => {
                Some(la.service_uuids.iter().map(|uuid| uuid.to_string()).collect::<Vec<_>>())
            });
            cr_property!(ib, "ManufacturerData", la => {
                Some(la.manufacturer_data.clone().into_iter().map(|(k, v)| (k, Variant(v))).collect::<HashMap<_, _>>())
            });
            cr_property!(ib, "SolicitUUIDs", la => {
                Some(la.solicit_uuids.iter().map(|uuid| uuid.to_string()).collect::<Vec<_>>())
            });
            cr_property!(ib, "ServiceData", la => {
                Some(la.service_data.iter().map(|(k, v)| (k.to_string(), Variant(v.clone()))).collect::<HashMap<_, _>>())
            });
            cr_property!(ib, "Data", la => {
                Some(la.advertising_data.iter().map(|(k, v)| (*k, Variant(v.clone()))).collect::<HashMap<_, _>>())
            });
            cr_property!(ib, "Discoverable", la => {
                la.discoverable
            });
            cr_property!(ib, "DiscoverableTimeout", la => {
                la.discoverable_timeout.map(|t| t.as_secs().min(u16::MAX as _) as u16)
            });
            cr_property!(ib, "Includes", la => {
                Some(la.system_includes.iter().map(|v| v.to_string()).collect::<Vec<_>>())
            });
            cr_property!(ib, "LocalName", la => {
                la.local_name.clone()
            });
            cr_property!(ib, "Appearance", la => {
                la.appearance
            });
            cr_property!(ib, "Duration", la => {
                la.duration.map(|t| t.as_secs().min(u16::MAX as _) as u16)
            });
            cr_property!(ib, "Timeout", la => {
                la.timeout.map(|t| t.as_secs().min(u16::MAX as _) as u16)
            });
            cr_property!(ib, "SecondaryChannel", la => {
                la.secondary_channel.map(|v| v.to_string())
            });
            cr_property!(ib, "MinInterval", la => {
                la.min_interval.map(|t| t.as_millis().min(u32::MAX as _) as u32)
            });
            cr_property!(ib, "MaxInterval", la => {
                la.max_interval.map(|t| t.as_millis().min(u32::MAX as _) as u32)
            });
            cr_property!(ib, "TxPower", la => {
                la.tx_power
            });
        })
    }

    pub(crate) async fn register(
        self,
        inner: Arc<SessionInner>,
        adapter_name: Arc<String>,
    ) -> Result<AdvertisementHandle> {
        let name = dbus::Path::new(format!(
            "{}{}",
            ADVERTISEMENT_PREFIX,
            Uuid::new_v4().as_simple()
        ))
        .unwrap();
        log::trace!("Publishing advertisement at {}", &name);

        let bus = Proxy::new(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            Duration::from_secs(15),
            inner.connection.clone(),
        );
        let (owner,): (String,) = bus
            .method_call("org.freedesktop.DBus", "GetNameOwner", (SERVICE_NAME,))
            .await?;
        let (lost_tx, lost) = tokio::sync::watch::channel(false);
        let owner_signal = owner.clone();
        let owner_lost = lost_tx.clone();
        let owner_match = inner
            .connection
            .add_match(
                dbus::message::MatchRule::new_signal("org.freedesktop.DBus", "NameOwnerChanged")
                    .with_strict_sender("org.freedesktop.DBus")
                    .with_path("/org/freedesktop/DBus"),
            )
            .await?
            .msg_cb(move |message| {
                if let Ok((name, old, new)) = message.read3::<String, String, String>() {
                    if name == SERVICE_NAME && old == owner_signal && new != owner_signal {
                        owner_lost.send_replace(true);
                    }
                }
                true
            });
        let power_lost = lost_tx.clone();
        let power_match = inner
            .connection
            .add_match(
                dbus::message::MatchRule::new_signal(
                    "org.freedesktop.DBus.Properties",
                    "PropertiesChanged",
                )
                .with_strict_sender(owner.clone())
                .with_path(Adapter::dbus_path(&adapter_name)?),
            )
            .await?
            .msg_cb(move |message| {
                if let Ok((interface, changed, invalidated)) =
                    message.read3::<String, PropMap, Vec<String>>()
                {
                    if interface == "org.bluez.Adapter1"
                        && (changed.get("Powered").and_then(|value| value.0.as_i64()) == Some(0)
                            || invalidated.iter().any(|name| name == "Powered"))
                    {
                        power_lost.send_replace(true);
                    }
                }
                true
            });
        let removed_lost = lost_tx.clone();
        let adapter_path = Adapter::dbus_path(&adapter_name)?;
        let removed_match = inner
            .connection
            .add_match(
                dbus::message::MatchRule::new_signal(
                    "org.freedesktop.DBus.ObjectManager",
                    "InterfacesRemoved",
                )
                .with_strict_sender(owner.clone()),
            )
            .await?
            .msg_cb(move |message| {
                if let Ok((path, interfaces)) = message.read2::<dbus::Path<'static>, Vec<String>>()
                {
                    if path == adapter_path
                        && interfaces.iter().any(|name| name == "org.bluez.Adapter1")
                    {
                        removed_lost.send_replace(true);
                    }
                }
                true
            });
        let (released_tx, released) = tokio::sync::watch::channel(false);
        {
            let mut cr = inner.crossroads.lock().await;
            cr.insert(
                name.clone(),
                &[inner.le_advertisment_token],
                RegisteredAdvertisement {
                    advertisement: self,
                    owner: owner.clone(),
                    released: released_tx,
                },
            );
        }

        // Bind this lifecycle to one bluetoothd instance. A restarted daemon
        // must not receive cleanup requests for another owner's registration.
        let proxy = Proxy::new(
            owner.clone(),
            Adapter::dbus_path(&adapter_name)?,
            Duration::from_secs(15),
            inner.connection.clone(),
        );
        let (registered_tx, registered_rx) = oneshot::channel();
        let (drop_tx, drop_rx) = oneshot::channel();
        let (finished, completion) = tokio::sync::watch::channel(None);
        let unreg_name = name.clone();
        let mut released_rx = released.clone();
        let mut lost_rx = lost.clone();
        // Own registration until its reply, even if the caller's future is
        // cancelled. A late successful reply must still be unregistered.
        tokio::spawn(async move {
            // Keep subscriptions until this registration's cleanup is complete.
            let _owner_match = owner_match;
            let _power_match = power_match;
            let _removed_match = removed_match;
            log::trace!("Registering advertisement at {}", &unreg_name);
            let registered: Result<()> = proxy
                .method_call(
                    MANAGER_INTERFACE,
                    "RegisterAdvertisement",
                    (unreg_name.clone(), PropMap::new()),
                )
                .await
                .map_err(Into::into);
            let mut registered_tx = Some(registered_tx);
            if registered.is_ok() {
                let _ = registered_tx.take().unwrap().send(Ok(()));
                // Close the subscribe/register race, including a controller
                // removed before it could emit its final PropertiesChanged.
                let current_owner: std::result::Result<(String,), dbus::Error> = bus
                    .method_call("org.freedesktop.DBus", "GetNameOwner", (SERVICE_NAME,))
                    .await;
                let powered: std::result::Result<(Variant<bool>,), dbus::Error> = proxy
                    .method_call(
                        "org.freedesktop.DBus.Properties",
                        "Get",
                        ("org.bluez.Adapter1", "Powered"),
                    )
                    .await;
                if !matches!(current_owner, Ok((current,)) if current == owner)
                    || !matches!(powered, Ok((Variant(true),)))
                {
                    lost_tx.send_replace(true);
                }
                tokio::select! {
                    _ = drop_rx => {},
                    _ = async {
                        while !*lost_rx.borrow() {
                            if lost_rx.changed().await.is_err() { break; }
                        }
                    } => {},
                    _ = async {
                        while !*released_rx.borrow() {
                            if released_rx.changed().await.is_err() { break; }
                        }
                    } => {},
                }
            }
            // A failed/timed-out registration may have reached bluetoothd.
            // Keep the object alive and retry cleanup until absence is confirmed.
            loop {
                // Release itself confirms removal; BlueZ explicitly says not
                // to unregister again after that callback.
                if *released_rx.borrow() {
                    break;
                }
                let result: std::result::Result<(), dbus::Error> = proxy
                    .method_call(
                        MANAGER_INTERFACE,
                        "UnregisterAdvertisement",
                        (unreg_name.clone(),),
                    )
                    .await;
                match result {
                    Ok(()) => break,
                    Err(error)
                        if matches!(
                            error.name(),
                            Some(
                                "org.bluez.Error.DoesNotExist"
                                    | "org.freedesktop.DBus.Error.ServiceUnknown"
                                    | "org.freedesktop.DBus.Error.NameHasNoOwner"
                                    | "org.freedesktop.DBus.Error.UnknownObject"
                                    | "org.freedesktop.DBus.Error.UnknownMethod"
                            )
                        ) =>
                    {
                        break
                    }
                    Err(error) => {
                        log::warn!(
                            "Advertisement cleanup pending at {}: {}",
                            &unreg_name,
                            error
                        );
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
            log::trace!("Unpublishing advertisement at {}", &unreg_name);
            let mut cr = inner.crossroads.lock().await;
            let _: Option<RegisteredAdvertisement> = cr.remove(&unreg_name);
            finished.send_replace(Some(Ok(())));
            if let Some(reply) = registered_tx {
                let _ = reply.send(registered);
            }
        });
        registered_rx.await.map_err(|_| crate::Error {
            kind: crate::ErrorKind::Failed,
            message: "Advertisement registration worker stopped".into(),
        })??;
        Ok(AdvertisementHandle {
            name,
            _drop_tx: Some(drop_tx),
            completion,
            released,
            lost,
        })
    }
}

/// Handle to active Bluetooth LE advertisement.
///
/// Drop to unregister advertisement.
#[must_use = "AdvertisementHandle must be held for advertisement to be broadcasted"]
pub struct AdvertisementHandle {
    name: dbus::Path<'static>,
    _drop_tx: Option<oneshot::Sender<()>>,
    completion: tokio::sync::watch::Receiver<Option<Result<()>>>,
    released: tokio::sync::watch::Receiver<bool>,
    lost: tokio::sync::watch::Receiver<bool>,
}

impl AdvertisementHandle {
    /// Wait for external Release, controller/owner loss or completed local removal without initiating
    /// removal. Safe to cancel and restart this wait.
    pub async fn released(&self) {
        let mut released = self.released.clone();
        let mut lost = self.lost.clone();
        tokio::select! {
            _ = async { while !*released.borrow() { if released.changed().await.is_err() { break; } } } => {},
            _ = async { while !*lost.borrow() { if lost.changed().await.is_err() { break; } } } => {},
        }
    }

    /// Unregister and wait until BlueZ confirms removal and the local object
    /// is unpublished. Cancelling this wait does not cancel cleanup; it can be
    /// called again. Dropping the handle retains the original background cleanup.
    pub async fn unregister(&mut self) -> Result<()> {
        if let Some(stop) = self._drop_tx.take() {
            let _ = stop.send(());
        }
        let mut completion = self.completion.clone();
        loop {
            if let Some(result) = completion.borrow().clone() {
                return result;
            }
            completion.changed().await.map_err(|_| crate::Error {
                kind: crate::ErrorKind::Failed,
                message: "Advertisement cleanup worker stopped".into(),
            })?;
        }
    }
}

impl Drop for AdvertisementHandle {
    fn drop(&mut self) {
        // required for drop order
    }
}

impl fmt::Debug for AdvertisementHandle {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "AdvertisementHandle {{ {} }}", &self.name)
    }
}
