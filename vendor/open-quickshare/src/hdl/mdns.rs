use std::sync::{Arc, Mutex};
use std::time::Duration;

use mdns_sd::{ServiceDaemon, ServiceInfo};
use tokio::sync::broadcast::Receiver;
use tokio::sync::watch;
use tokio::time::{Instant, interval_at};
use tokio_util::sync::CancellationToken;

use crate::DEVICE_NAME;
use crate::utils::{DeviceType, gen_mdns_endpoint_info, gen_mdns_name};

const INNER_NAME: &str = "MDnsServer";
const TICK_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Visibility {
    Visible = 0,
    Invisible = 1,
    Temporarily = 2,
}

#[allow(dead_code)]
impl Visibility {
    pub fn from_raw_value(value: u64) -> Self {
        match value {
            0 => Visibility::Visible,
            1 => Visibility::Invisible,
            2 => Visibility::Temporarily,
            _ => unreachable!(),
        }
    }
}

pub struct MDnsServer {
    daemon: ServiceDaemon,
    closed: bool,
    service_info: ServiceInfo,
    ble_receiver: Receiver<()>,
    visibility_sender: Arc<Mutex<watch::Sender<Visibility>>>,
    visibility_receiver: watch::Receiver<Visibility>,
    lan_state: watch::Receiver<crate::lan_policy::LanSnapshot>,
    endpoint_id: [u8; 4],
    service_port: u16,
    registered: bool,
}

impl Drop for MDnsServer {
    fn drop(&mut self) {
        if !self.closed {
            super::mdns_cleanup::abandoned(&self.daemon);
        }
    }
}

impl MDnsServer {
    pub fn new(
        endpoint_id: [u8; 4],
        service_port: u16,
        ble_receiver: Receiver<()>,
        visibility_sender: Arc<Mutex<watch::Sender<Visibility>>>,
        visibility_receiver: watch::Receiver<Visibility>,
        lan_state: watch::Receiver<crate::lan_policy::LanSnapshot>,
    ) -> Result<Self, anyhow::Error> {
        let snapshot = lan_state.borrow().clone();
        let service_info = Self::build_service_on(
            endpoint_id,
            service_port,
            DeviceType::Laptop,
            &snapshot.interfaces,
        )?;

        let daemon = ServiceDaemon::new()?;
        let this = Self {
            daemon,
            closed: false,
            service_info,
            ble_receiver,
            visibility_sender,
            visibility_receiver,
            lan_state,
            endpoint_id,
            service_port,
            registered: false,
        };
        crate::lan_policy::configure_mdns_on(&this.daemon, &snapshot.interfaces)?;
        Ok(this)
    }

    pub async fn run(&mut self, ctk: CancellationToken) -> Result<(), anyhow::Error> {
        let result = self.run_inner(ctk).await;
        let cleanup = super::mdns_cleanup::shutdown(&self.daemon).await;
        self.closed = cleanup.is_ok();
        result.and(cleanup)
    }

    async fn run_inner(&mut self, ctk: CancellationToken) -> Result<(), anyhow::Error> {
        info!("{INNER_NAME}: service starting");
        let monitor = self.daemon.monitor()?;
        let mut visibility = *self.visibility_receiver.borrow();
        if visibility != Visibility::Invisible && !self.service_info.get_addresses().is_empty() {
            self.daemon.register(self.service_info.clone())?;
            self.registered = true;
        }
        let mut interval = interval_at(Instant::now() + TICK_INTERVAL, TICK_INTERVAL);

        loop {
            tokio::select! {
                _ = ctk.cancelled() => {
                    info!("{INNER_NAME}: tracker cancelled, breaking");
                    break;
                }
                r = monitor.recv_async() => {
                    match r {
                        Ok(mdns_sd::DaemonEvent::Error(err)) => return Err(err.into()),
                        Ok(_) => continue,
                        Err(err) => return Err(err.into()),
                    }
                },
                changed = self.lan_state.changed() => {
                    if changed.is_err() { break; }
                    let snapshot = self.lan_state.borrow_and_update().clone();
                    self.unregister().await?;
                    crate::lan_policy::configure_mdns_on(&self.daemon, &snapshot.interfaces)?;
                    self.service_info = Self::build_service_on(self.endpoint_id, self.service_port, DeviceType::Laptop, &snapshot.interfaces)?;
                    if *self.visibility_receiver.borrow() != Visibility::Invisible && !self.service_info.get_addresses().is_empty() {
                        self.daemon.register(self.service_info.clone())?;
                        self.registered = true;
                    }
                },
                changed = self.visibility_receiver.changed() => {
                    if changed.is_err() { break; }
                    visibility = *self.visibility_receiver.borrow_and_update();

                    debug!("{INNER_NAME}: visibility changed: {visibility:?}");
                    if visibility != Visibility::Invisible && !self.service_info.get_addresses().is_empty() {
                        self.daemon.register(self.service_info.clone())?;
                        self.registered = true;
                    } else {
                        self.unregister().await?;
                    }
                    if visibility == Visibility::Temporarily { interval.reset(); }
                }
                signal = self.ble_receiver.recv() => {
                    if matches!(signal, Err(tokio::sync::broadcast::error::RecvError::Closed)) { break; }
                    if *self.visibility_receiver.borrow() == Visibility::Invisible || !self.registered {
                        continue;
                    }

                    debug!("{INNER_NAME}: ble_receiver: got event");
                    if visibility == Visibility::Visible || visibility == Visibility::Temporarily {
                        // Android can sometime not see the mDNS service if the service
                        // was running BEFORE Android started the Discovery phase for QuickShare.
                        // So resend a broadcast if there's a android device sending.
                        self.daemon.register_resend(self.service_info.get_fullname())?;
                    } else {
                        self.daemon.register(self.service_info.clone())?;
                    }
                },
                _ = interval.tick() => {
                    if visibility != Visibility::Temporarily {
                        continue;
                    }

                    self.unregister().await?;
                    visibility = Visibility::Invisible;
                    let _ = self.visibility_sender.lock().unwrap().send(Visibility::Invisible);
                }
            }
        }

        // Unregister the mDNS service - we're shutting down
        self.unregister().await?;

        Ok(())
    }

    async fn unregister(&mut self) -> Result<(), anyhow::Error> {
        if self.registered {
            let receiver = self.daemon.unregister(self.service_info.get_fullname())?;
            tokio::time::timeout(Duration::from_secs(2), receiver.recv_async()).await??;
            self.registered = false;
        }
        Ok(())
    }

    pub fn build_service(
        endpoint_id: [u8; 4],
        service_port: u16,
        device_type: DeviceType,
    ) -> Result<ServiceInfo, anyhow::Error> {
        Self::build_service_on(
            endpoint_id,
            service_port,
            device_type,
            &crate::lan_policy::interfaces(false)?,
        )
    }

    pub fn build_service_on(
        endpoint_id: [u8; 4],
        service_port: u16,
        device_type: DeviceType,
        interfaces: &[linuxdrop_network::InterfaceAddress],
    ) -> Result<ServiceInfo, anyhow::Error> {
        // This `name` is going to be random every time RQS service restarts.
        // If that is not desired, derive host_name, etc. via some other means
        let name = gen_mdns_name(endpoint_id);
        let device_name = DEVICE_NAME.read().unwrap().clone();
        info!("Broadcasting with: device_name={device_name}, host_name={name}");
        let endpoint_info = gen_mdns_endpoint_info(device_type as u8, &device_name);

        let properties = [("n", endpoint_info)];
        let addresses = interfaces
            .iter()
            .filter(|interface| {
                !interface.loopback && crate::lan_policy::valid_unicast(interface.address)
            })
            .map(|interface| interface.address.to_string())
            .collect::<Vec<_>>()
            .join(",");
        let si = ServiceInfo::new(
            "_FC9F5ED42C8A._tcp.local.",
            &name,
            &name, // Needs to be ASCII?
            addresses.as_str(),
            service_port,
            &properties[..],
        )?;

        Ok(si)
    }
}
