#[macro_use]
extern crate log;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use anyhow::anyhow;
use channel::ChannelMessage;
#[cfg(all(feature = "experimental", target_os = "linux"))]
use hdl::BleAdvertiser;
use hdl::MDnsDiscovery;
use lifecycle::Workers;
use once_cell::sync::Lazy;
use rand::Rng;
use rand::distr::Alphanumeric;
use tokio::sync::{broadcast, mpsc, watch};
use tokio_util::sync::CancellationToken;

#[cfg(feature = "experimental")]
use crate::hdl::BleListener;
use crate::hdl::MDnsServer;
use crate::manager::TcpServer;

pub mod channel;
pub mod errors;
pub mod hdl;
pub mod lan_policy;
#[doc(hidden)]
pub mod lifecycle;
pub mod manager;
pub mod payload_budget;
pub mod utils;

static BLUETOOTH_ADAPTER: RwLock<Option<String>> = RwLock::new(None);
pub fn set_bluetooth_adapter(name: Option<String>) {
    *BLUETOOTH_ADAPTER.write().unwrap() = name;
}
pub fn bluetooth_adapter_name() -> Option<String> {
    BLUETOOTH_ADAPTER.read().unwrap().clone()
}
#[cfg(all(feature = "experimental", target_os = "linux"))]
pub async fn bluetooth_adapter() -> Result<bluer::Adapter, anyhow::Error> {
    let name = bluetooth_adapter_name();
    linuxdrop_network::bluetooth_adapter(name.as_deref()).await
}

static SESSION_SHUTDOWN: Lazy<RwLock<CancellationToken>> =
    Lazy::new(|| RwLock::new(CancellationToken::new()));
pub fn session_shutdown() -> CancellationToken {
    SESSION_SHUTDOWN.read().unwrap().clone()
}

static RECEIVE_LIMITS: RwLock<(u64, usize)> = RwLock::new((10 * 1024 * 1024 * 1024, 1000));
pub fn set_receive_limits(bytes: u64, files: usize) {
    *RECEIVE_LIMITS.write().unwrap() = (bytes.min(1 << 40), files.min(1000));
}

fn backend_failure(
    sender: &broadcast::Sender<ChannelMessage>,
    component: &str,
    detail: impl std::fmt::Display,
) {
    let _ = sender.send(ChannelMessage {
        id: "backend".into(),
        msg: channel::Message::Backend {
            component: component.into(),
            detail: detail.to_string(),
        },
    });
}

pub use hdl::{EndpointInfo, OutboundPayload, TransferState, Visibility};
pub use manager::SendInfo;
pub use utils::DeviceType;

pub mod sharing_nearby {
    include!(concat!(env!("OUT_DIR"), "/sharing.nearby.rs"));
}

pub mod securemessage {
    include!(concat!(env!("OUT_DIR"), "/securemessage.rs"));
}

pub mod securegcm {
    include!(concat!(env!("OUT_DIR"), "/securegcm.rs"));
}

pub mod location_nearby_connections {
    include!(concat!(env!("OUT_DIR"), "/location.nearby.connections.rs"));
}

static CUSTOM_DOWNLOAD: Lazy<RwLock<Option<PathBuf>>> = Lazy::new(|| RwLock::new(None));
static DEVICE_NAME: Lazy<RwLock<String>> = Lazy::new(|| RwLock::new(hostname()));
fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "LinuxDrop".into())
}

#[derive(Debug)]
pub struct RQS {
    pub ble_enabled: bool,
    tracker: Option<Workers>,
    stop_error: Option<String>,
    ctoken: Option<CancellationToken>,
    session_ctoken: Option<CancellationToken>,
    // Discovery token is different than ctoken because he is on his own
    // - can be cancelled while the ctoken is still active
    discovery_ctk: Option<CancellationToken>,
    lan_state: Option<watch::Receiver<lan_policy::LanSnapshot>>,

    // Used to trigger a change in the mDNS visibility (and later on, BLE)
    pub visibility_sender: Arc<Mutex<watch::Sender<Visibility>>>,
    visibility_receiver: watch::Receiver<Visibility>,

    // Only used to send the info "a nearby device is sharing"
    ble_sender: broadcast::Sender<()>,

    pub port_number: Option<u32>,

    pub message_sender: broadcast::Sender<ChannelMessage>,
}

impl Default for RQS {
    fn default() -> Self {
        Self::new(Visibility::Visible, None, None, Some(hostname()))
    }
}

impl RQS {
    pub fn new(
        visibility: Visibility,
        port_number: Option<u32>,
        download_path: Option<PathBuf>,
        device_name: Option<String>,
    ) -> Self {
        {
            let mut guard = CUSTOM_DOWNLOAD.write().unwrap();
            *guard = download_path;
        }
        {
            let mut guard = DEVICE_NAME.write().unwrap();
            if let Some(device_name) = device_name {
                *guard = device_name.clone();
            }
        }

        let (message_sender, _) = broadcast::channel(1024);
        let (ble_sender, _) = broadcast::channel(5);

        // Define default visibility as per the args inside the new()
        let (visibility_sender, visibility_receiver) = watch::channel(Visibility::Invisible);
        let _ = visibility_sender.send(visibility);

        Self {
            ble_enabled: true,
            tracker: None,
            stop_error: None,
            ctoken: None,
            session_ctoken: None,
            discovery_ctk: None,
            lan_state: None,
            visibility_sender: Arc::new(Mutex::new(visibility_sender)),
            visibility_receiver,
            ble_sender,
            port_number,
            message_sender,
        }
    }

    pub async fn run(
        &mut self,
    ) -> Result<(mpsc::Sender<SendInfo>, broadcast::Receiver<()>), anyhow::Error> {
        if self.ctoken.is_some() {
            return Err(anyhow!("Quick Share engine is already started"));
        }
        let sessions = CancellationToken::new();
        self.session_ctoken = Some(sessions.clone());
        *SESSION_SHUTDOWN.write().unwrap() = sessions;
        if let Some(error) = &self.stop_error {
            return Err(anyhow!("Previous cleanup is incomplete: {error}"));
        }
        let tracker = Workers::with_events(self.message_sender.clone());
        let ctoken = CancellationToken::new();
        self.tracker = Some(tracker.clone());
        self.ctoken = Some(ctoken.clone());

        let endpoint_id: Vec<u8> = rand::rng()
            .sample_iter(Alphanumeric)
            .take(4)
            .map(u8::from)
            .collect();
        let tcp_listeners =
            lan_policy::LanListeners::new(u16::try_from(self.port_number.unwrap_or(0))?).await?;
        let service_port = tcp_listeners.port();
        let lan_state = tcp_listeners.subscribe();
        self.lan_state = Some(lan_state.clone());
        info!("TCP listeners on port: {}", service_port);

        // So the random port can be accessed from the user if needed.
        // This does have a difference in behaviour however when port_number is Some.
        // .stop() and .run() will reuse the port number instead of generating a new one.
        self.port_number = Some(service_port as u32);

        // MPSC for the TcpServer
        let send_channel = mpsc::channel(10);
        // Start TcpServer in own "task"
        let mut server = TcpServer::new(
            endpoint_id[..4].try_into()?,
            tcp_listeners,
            self.message_sender.clone(),
            send_channel.1,
        )?;
        let ctk = ctoken.clone();
        let lifetime = ctk.clone();
        let status = self.message_sender.clone();
        tracker.spawn("tcp", async move {
            let result = server.run(ctk).await;
            if !lifetime.is_cancelled() {
                let _ = status.send(channel::ChannelMessage {
                    id: "backend".into(),
                    msg: channel::Message::Backend {
                        component: "tcp".into(),
                        detail: result
                            .as_ref()
                            .err()
                            .map(ToString::to_string)
                            .unwrap_or_else(|| "Service task ended unexpectedly".into()),
                    },
                });
            }
            result
        });

        #[cfg(feature = "experimental")]
        if self.ble_enabled {
            let sender = self.ble_sender.clone();
            let status = self.message_sender.clone();
            let ctk = ctoken.clone();
            tracker.spawn("bluetooth-listener", async move {
                BleListener::supervise(sender, status, ctk).await
            });
        }

        // Start MDnsServer in own "task"
        let mut mdns = MDnsServer::new(
            endpoint_id[..4].try_into()?,
            service_port,
            self.ble_sender.subscribe(),
            self.visibility_sender.clone(),
            self.visibility_receiver.clone(),
            lan_state,
        )?;
        let ctk = ctoken.clone();
        let lifetime = ctk.clone();
        let status = self.message_sender.clone();
        tracker.spawn("mdns", async move {
            let result = mdns.run(ctk).await;
            if !lifetime.is_cancelled() {
                let _ = status.send(channel::ChannelMessage {
                    id: "backend".into(),
                    msg: channel::Message::Backend {
                        component: "mdns".into(),
                        detail: result
                            .as_ref()
                            .err()
                            .map(ToString::to_string)
                            .unwrap_or_else(|| "Service task ended unexpectedly".into()),
                    },
                });
            }
            result
        });

        // Recreate receiver resources as one generation so its advertised L2CAP
        // port always belongs to the current listener. Migrated Wi-Fi sessions
        // are owned by the supervisor, not a short-lived radio generation.
        #[cfg(all(feature = "experimental", target_os = "linux"))]
        if self.ble_enabled {
            let endpoint = endpoint_id[..4].try_into()?;
            let name = DEVICE_NAME.read().unwrap().clone();
            let status = self.message_sender.clone();
            let visibility = self.visibility_receiver.clone();
            let ctk = ctoken.clone();
            tracker.spawn("bluetooth-receiver", async move {
                crate::hdl::supervise_receiver(
                    endpoint,
                    crate::utils::DeviceType::Laptop as u8,
                    name,
                    service_port,
                    visibility,
                    status,
                    ctk,
                )
                .await
            });
        }

        tracker.close();

        Ok((send_channel.0, self.ble_sender.subscribe()))
    }

    pub fn discovery(
        &mut self,
        sender: broadcast::Sender<EndpointInfo>,
    ) -> Result<(), anyhow::Error> {
        if self.discovery_ctk.is_some() {
            return Err(anyhow!(
                "Discovery is already started; stop it before replacing it"
            ));
        }
        let tracker = self
            .tracker
            .as_ref()
            .ok_or_else(|| anyhow!("The service wasn't first started"))?;

        let ctk = CancellationToken::new();
        self.discovery_ctk = Some(ctk.clone());

        #[cfg(all(feature = "experimental", target_os = "linux"))]
        if self.ble_enabled {
            let ctk_blea = ctk.clone();
            let status = self.message_sender.clone();
            tracker.spawn("bluetooth-discovery", async move {
                BleAdvertiser::supervise(status, ctk_blea).await
            });

            // Discover phones on their Quick Share receive screen over BLE, so
            // they can be sent to without both devices being on the same Wi-Fi.
            // `PACKET_BLE_SEND=off` disables it.
            if std::env::var("PACKET_BLE_SEND")
                .map(|v| v.eq_ignore_ascii_case("off"))
                .unwrap_or(false)
            {
                info!("BLE recipient discovery: disabled by PACKET_BLE_SEND=off");
            } else {
                let ble_sender = sender.clone();
                let ctk_bled = ctk.clone();
                let status = self.message_sender.clone();
                tracker.spawn("bluetooth-peer-discovery", async move {
                    crate::hdl::ble_discovery(ble_sender, status, ctk_bled).await
                });
            }
        }

        // `PACKET_PREFER_BLE=on` disables mDNS discovery so recipients are found
        // over BLE only — the send then starts on L2CAP and (with the phone on
        // Wi-Fi) exercises the BLE→Wi-Fi bandwidth upgrade. Test/demo toggle.
        if std::env::var("PACKET_PREFER_BLE")
            .map(|v| v.eq_ignore_ascii_case("on") || v == "1")
            .unwrap_or(false)
        {
            info!("MDnsDiscovery: disabled by PACKET_PREFER_BLE=on (BLE-only recipient discovery)");
        } else {
            let discovery = MDnsDiscovery::new(sender, self.lan_state()?)?;
            let status = self.message_sender.clone();
            tracker.spawn("discovery", async move {
                let result = discovery.run(ctk.clone()).await;
                if !ctk.is_cancelled() {
                    let _ = status.send(channel::ChannelMessage {
                        id: "backend".into(),
                        msg: channel::Message::Backend {
                            component: "discovery".into(),
                            detail: result
                                .as_ref()
                                .err()
                                .map(ToString::to_string)
                                .unwrap_or_else(|| "Discovery task ended unexpectedly".into()),
                        },
                    });
                }
                result
            });
        }

        Ok(())
    }

    pub fn lan_state(&self) -> Result<watch::Receiver<lan_policy::LanSnapshot>, anyhow::Error> {
        self.lan_state
            .clone()
            .ok_or_else(|| anyhow!("LAN service is not running"))
    }

    pub fn stop_discovery(&mut self) {
        if let Some(discovert_ctk) = &self.discovery_ctk {
            discovert_ctk.cancel();
            self.discovery_ctk = None;
        }
    }

    pub fn change_visibility(&mut self, nv: Visibility) {
        self.visibility_sender
            .lock()
            .unwrap()
            .send_modify(|state| *state = nv);
    }

    pub async fn stop(&mut self) -> Result<(), String> {
        // An old/cancelled startup must never cancel a newer engine generation.
        if let Some(sessions) = &self.session_ctoken {
            sessions.cancel();
        }
        let _ = self.message_sender.send(channel::ChannelMessage {
            id: "*".into(),
            msg: channel::Message::Lib {
                action: channel::TransferAction::TransferCancel,
            },
        });
        self.stop_discovery();

        if let Some(ctoken) = &self.ctoken {
            ctoken.cancel();
        }

        let result = if let Some(tracker) = &self.tracker {
            // Inorder for TaskTracker::wait to return, close() must be called
            // and the count of tasks being watched should be 0 (i.e. they've all closed).
            //
            // If not, the TaskTracker may forever wait if task count is 0 when wait() was called
            tracker.close();
            tracker.wait().await
        } else {
            self.stop_error.clone().map_or(Ok(()), Err)
        };
        self.stop_error = result.as_ref().err().cloned();

        self.ctoken = None;
        self.session_ctoken = None;
        self.tracker = None;
        self.lan_state = None;
        result
    }

    // Setting None here will resume the default settings
    pub fn set_download_path(&self, p: Option<PathBuf>) {
        debug!("Setting the download path to {:?}", p);
        let mut guard = CUSTOM_DOWNLOAD.write().unwrap();
        *guard = p;
    }

    /// For this to properly take effect,
    /// `MdnsServer` would need to be reset which is done by `RQS::stop` followed by `RQS::run`.
    ///
    /// So only do this when no data transfer is going on.
    pub fn set_device_name(&self, name: String) {
        debug!("Setting the device name {:?}", name);
        let mut guard = DEVICE_NAME.write().unwrap();
        *guard = name;
    }

    pub fn get_device_name(&self) -> String {
        DEVICE_NAME.read().unwrap().clone()
    }
}
