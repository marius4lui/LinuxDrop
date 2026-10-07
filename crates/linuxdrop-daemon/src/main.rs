mod helper;
mod history;
mod lifecycle;
mod notifications;
mod preferences;
mod receive;
mod settings;
use anyhow::{Context, Result};
use linuxdrop_core::*;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, Mutex};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

const SERVICE: &str = "io.github.marius4lui.LinuxDrop";
const OBJECT: &str = "/io/github/marius4lui/LinuxDrop";
const INTERFACE: &str = "io.github.marius4lui.LinuxDrop.Manager1";

struct Draft {
    files: Vec<SendSource>,
    created: Instant,
}
struct Data {
    epoch: String,
    revision: u64,
    settings: Value,
    peers: HashMap<String, Peer>,
    peer_preferences: HashMap<String, preferences::PeerPreferences>,
    transfers: HashMap<String, Transfer>,
    transfer_order: Vec<String>,
    completed_at: HashMap<String, u64>,
    backends: HashMap<String, BackendState>,
    hardware: Value,
    drafts: HashMap<String, Draft>,
    commands: HashMap<String, CommandSender>,
    retiring: HashMap<String, CommandSender>,
    failed_backends: HashSet<String>,
    visibility_since: Option<Instant>,
    decisions: HashSet<String>,
    stop_when_idle: bool,
    restarting: bool,
}
impl Data {
    fn accepting_transfers(&self) -> zbus::fdo::Result<()> {
        if self.stop_when_idle {
            return Err(failed("LinuxDrop is stopping"));
        }
        if self.restarting {
            return Err(failed("Sharing services are restarting; try again shortly"));
        }
        Ok(())
    }
}
struct Shared {
    bandwidth: Mutex<linuxdrop_network::BandwidthLimiter>,
    data: Mutex<Data>,
    connection: std::sync::OnceLock<zbus::Connection>,
    config_path: PathBuf,
    data_dir: PathBuf,
    restart: mpsc::Sender<()>,
    helper: Mutex<Option<helper::HelperLease>>,
    quickshare_helper: Mutex<Option<helper::HelperLease>>,
    backend_generation: std::sync::atomic::AtomicU64,
    event_forwarders: tokio_util::task::TaskTracker,
    locked: std::sync::atomic::AtomicBool,
    download_offer: Mutex<Option<linuxdrop_localsend::reverse::ReverseOffer>>,
    history_io: Mutex<()>,
    log_filter: tracing_subscriber::reload::Handle<
        tracing_subscriber::EnvFilter,
        tracing_subscriber::Registry,
    >,
}
impl Shared {
    async fn persist_history(&self) -> Result<()> {
        let _guard = self.history_io.lock().await;
        let d = self.data.lock().await;
        let history = json!(d
            .transfer_order
            .iter()
            .filter_map(|id| d.transfers.get(id))
            .filter(|t| t.is_terminal())
            .map(|t| history::Entry {
                transfer: t.clone(),
                completed_at: d
                    .completed_at
                    .get(&t.id)
                    .copied()
                    .unwrap_or_else(history::now)
            })
            .collect::<Vec<_>>());
        drop(d);
        save_config(&self.data_dir.join("history.json"), &history).await
    }
    async fn changed(&self) {
        let revision = {
            let mut d = self.data.lock().await;
            d.revision += 1;
            d.revision
        };
        if let Some(connection) = self.connection.get() {
            let _ = connection
                .emit_signal(None::<&str>, OBJECT, INTERFACE, "Changed", &(revision,))
                .await;
        }
    }
    async fn snapshot(&self) -> String {
        let d = self.data.lock().await;
        let mut peers: Vec<_> = d.peers.values().collect();
        peers.sort_by(|a, b| a.name.cmp(&b.name));
        let peers:Vec<_>=peers.into_iter().map(|peer| {
            let mut value=serde_json::to_value(peer).unwrap_or_default();
            if let Some(prefs)=d.peer_preferences.get(&peer.id) {
                for (key,field) in serde_json::to_value(prefs).unwrap().as_object().unwrap() {value[key]=field.clone();}
                if prefs.blocked {value["available"]=json!(false);}
            }
            value["identity_scope"]=json!("Preferences apply to this protocol identifier; they do not authenticate a person. Discovery identifiers can change.");
            value
        }).collect();
        let known_peers:Vec<_>=d.peer_preferences.iter().map(|(id,prefs)|json!({"id":id,"favorite":prefs.favorite,"display_name":prefs.display_name,"blocked":prefs.blocked,"preferred_protocol":prefs.preferred_protocol,"available":d.peers.contains_key(id)})).collect();
        let transfers: Vec<_> = d
            .transfer_order
            .iter()
            .rev()
            .filter_map(|id| d.transfers.get(id))
            .map(|transfer| {
                let mut value = serde_json::to_value(transfer).unwrap_or_default();
                if transfer.direction == "incoming" && !transfer.is_terminal() {
                    if let Ok(options) = receive::options(&d.settings, transfer, None) {
                        value["receive_directory"] = json!(options.directory);
                    }
                    value["selection_mode"] = json!(if transfer.protocol == "localsend" {
                        "native"
                    } else {
                        "publish_selected"
                    });
                }
                value
            })
            .collect();
        let download_link_active = self
            .download_offer
            .lock()
            .await
            .as_ref()
            .is_some_and(|offer| offer.is_active());
        json!({"download_link_active":download_link_active,"restarting":d.restarting,"epoch":d.epoch,"revision":d.revision,"peers":peers,"known_peers":known_peers,"transfers":transfers,"backends":d.backends.values().collect::<Vec<_>>(),"hardware":d.hardware,"settings":d.settings}).to_string()
    }
    async fn action(&self, id: &str, action: &str) -> zbus::fdo::Result<()> {
        if action == "accept" {
            return self.accept(id, None).await;
        }
        let mut data = self.data.lock().await;
        let transfer = data
            .transfers
            .get(id)
            .ok_or_else(|| failed("Unknown transfer"))?;
        if transfer.is_terminal() {
            return Ok(());
        }
        let tx = data
            .commands
            .get(&transfer.protocol)
            .cloned()
            .ok_or_else(|| failed("Backend unavailable"))?;
        if action == "reject" && data.decisions.contains(id) {
            return Err(failed(
                "A decision has already been submitted; cancel the transfer instead",
            ));
        }
        let command = match action {
            "reject" => BackendCommand::Reject {
                transfer_id: id.into(),
            },
            _ => BackendCommand::Cancel {
                transfer_id: id.into(),
            },
        };
        tx.try_send(command).map_err(failed)?;
        data.decisions.insert(id.into());
        Ok(())
    }

    async fn accept(&self, id: &str, patch: Option<Value>) -> zbus::fdo::Result<()> {
        if self.locked.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(failed("Unlock the session to accept files"));
        }
        let mut d = self.data.lock().await;
        let transfer = d
            .transfers
            .get(id)
            .ok_or_else(|| failed("Unknown transfer"))?;
        if transfer.is_terminal() {
            return Err(failed("Transfer has already ended"));
        }
        if d.decisions.contains(id) {
            return Err(failed("A decision has already been submitted"));
        }
        if !matches!(transfer.state.as_str(), "waiting" | "verification") {
            return Err(failed("Transfer is not awaiting a decision"));
        }
        if d.peer_preferences
            .get(&transfer.peer_id)
            .is_some_and(|p| p.blocked)
        {
            return Err(failed("This device is blocked"));
        }
        let tx = d
            .commands
            .get(&transfer.protocol)
            .cloned()
            .ok_or_else(|| failed("Backend unavailable"))?;
        let command = if transfer.direction == "incoming" {
            if d.settings["receive"]["ask_directory"] == true
                && !patch.as_ref().is_some_and(|p| p["directory"].is_string())
            {
                return Err(failed("Choose a destination in LinuxDrop before accepting"));
            }
            let options = receive::options(&d.settings, transfer, patch).map_err(failed)?;
            let directory = options.directory.clone().unwrap();
            let bytes = if transfer.protocol == "localsend" {
                options
                    .selected_indices
                    .as_ref()
                    .map(|indices| indices.iter().map(|i| transfer.files[*i].size).sum())
                    .unwrap_or(transfer.total_bytes)
            } else {
                transfer.total_bytes
            };
            tokio::task::spawn_blocking(move || {
                linuxdrop_storage::ReceiveStore::open(directory)?.ensure_space(bytes)
            })
            .await
            .map_err(failed)?
            .map_err(failed)?;
            BackendCommand::AcceptWithOptions {
                transfer_id: id.into(),
                options,
            }
        } else {
            BackendCommand::Accept {
                transfer_id: id.into(),
            }
        };
        if self.locked.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(failed("Unlock the session to accept files"));
        }
        tx.try_send(command).map_err(failed)?;
        d.decisions.insert(id.into());
        Ok(())
    }
}
fn failed(message: impl ToString) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(message.to_string())
}
struct Manager(Arc<Shared>);
#[derive(Clone)]
struct HelperP2p {
    shared: std::sync::Weak<Shared>,
    lease_id: String,
}
impl HelperP2p {
    async fn request(&self, request: linuxdrop_netd::Request) -> Result<linuxdrop_netd::Response> {
        let client = self.clone();
        let cancel = tokio_util::sync::CancellationToken::new();
        let cancel_on_drop = cancel.clone().drop_guard();
        let joining = matches!(
            &request,
            linuxdrop_netd::Request::JoinP2p { .. } | linuxdrop_netd::Request::HostP2p { .. }
        );
        let (sender, receiver) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let pending = client.request_inner(request);
            tokio::pin!(pending);
            let result = tokio::select! {
                result = &mut pending => result,
                _ = cancel.cancelled(), if joining => {
                    // Cancel on a separate authorized socket, while the original
                    // worker drains its response and preserves request framing.
                    let _ = tokio::time::timeout(Duration::from_secs(10), async {
                        let mut control = linuxdrop_netd::Client::connect().await?;
                        control.request(&linuxdrop_netd::Request::CancelP2p { lease_id: client.lease_id.clone() }).await
                    }).await;
                    pending.await
                }
            };
            let _ = sender.send(result);
        });
        let result = receiver
            .await
            .context("P2P helper request worker stopped")?;
        cancel_on_drop.disarm();
        result
    }
    async fn request_inner(
        &self,
        request: linuxdrop_netd::Request,
    ) -> Result<linuxdrop_netd::Response> {
        let shared = self
            .shared
            .upgrade()
            .context("LinuxDrop is shutting down")?;
        let mut slot = shared.quickshare_helper.lock().await;
        let lease = slot
            .as_mut()
            .context("Direct Wi-Fi helper lease is unavailable")?;
        anyhow::ensure!(
            lease.expected.id == self.lease_id,
            "Direct Wi-Fi lease was replaced"
        );
        let generation = lease.generation;
        let result =
            match tokio::time::timeout(Duration::from_secs(110), lease.client.request(&request))
                .await
            {
                Ok(Ok(response)) => return Ok(response),
                Ok(Err(error)) => Err(error.into()),
                Err(error) => Err(error.into()),
            };
        slot.take();
        drop(slot);
        helper::failed(&shared, "quickshare", generation).await;
        result
    }
}
impl linuxdrop_network::P2pConnector for HelperP2p {
    fn host(&self) -> futures_util::future::BoxFuture<'_, Result<linuxdrop_network::P2pHosted>> {
        Box::pin(async move {
            match self
                .request(linuxdrop_netd::Request::HostP2p {
                    lease_id: self.lease_id.clone(),
                })
                .await?
            {
                linuxdrop_netd::Response::P2pHosted {
                    interface,
                    ssid,
                    password,
                    frequency,
                    ipv4_address,
                    ipv6_address,
                } => Ok(linuxdrop_network::P2pHosted {
                    interface,
                    ssid,
                    password,
                    frequency,
                    ipv4_address,
                    ipv6_address,
                }),
                linuxdrop_netd::Response::Error { message } => anyhow::bail!(message),
                _ => anyhow::bail!("Unexpected P2P group-owner response"),
            }
        })
    }
    fn connect(
        &self,
        peer_name: String,
        pin: String,
        frequency: u32,
    ) -> futures_util::future::BoxFuture<'_, Result<linuxdrop_network::P2pConnection>> {
        Box::pin(async move {
            match self
                .request(linuxdrop_netd::Request::JoinP2p {
                    lease_id: self.lease_id.clone(),
                    peer_name,
                    pin,
                    frequency,
                })
                .await?
            {
                linuxdrop_netd::Response::P2pJoined {
                    interface,
                    ipv4_address,
                    ipv6_address,
                } => Ok(linuxdrop_network::P2pConnection {
                    interface,
                    ipv4_address,
                    ipv6_address,
                }),
                linuxdrop_netd::Response::Error { message } => anyhow::bail!(message),
                _ => anyhow::bail!("Unexpected P2P helper response"),
            }
        })
    }
    fn disconnect(&self) -> futures_util::future::BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            match self
                .request(linuxdrop_netd::Request::LeaveP2p {
                    lease_id: self.lease_id.clone(),
                })
                .await?
            {
                linuxdrop_netd::Response::Ok => Ok(()),
                linuxdrop_netd::Response::Error { message } => anyhow::bail!(message),
                _ => anyhow::bail!("Unexpected P2P cleanup response"),
            }
        })
    }
}
#[zbus::interface(name = "io.github.marius4lui.LinuxDrop.Manager1")]
impl Manager {
    async fn get_snapshot(&self) -> String {
        self.0.snapshot().await
    }
    async fn get_settings(&self) -> String {
        self.0.data.lock().await.settings.to_string()
    }
    async fn get_defaults(&self) -> String {
        settings::defaults().to_string()
    }

    async fn update_peer_preferences(
        &self,
        peer_id: String,
        patch: String,
    ) -> zbus::fdo::Result<()> {
        if peer_id.len() > 512 || patch.len() > 4096 {
            return Err(failed("Peer preferences exceed limits"));
        }
        let mut d = self.0.data.lock().await;
        if !d.peers.contains_key(&peer_id) && !d.peer_preferences.contains_key(&peer_id) {
            return Err(failed("Unknown peer"));
        }
        if d.peer_preferences.len() >= 1000 && !d.peer_preferences.contains_key(&peer_id) {
            return Err(failed("Saved device limit reached"));
        }
        let next = d
            .peer_preferences
            .get(&peer_id)
            .cloned()
            .unwrap_or_default()
            .patch(serde_json::from_str(&patch).map_err(failed)?)
            .map_err(failed)?;
        let mut prefs = d.peer_preferences.clone();
        prefs.insert(peer_id, next);
        save_config(&self.0.data_dir.join("peers.json"), &json!(prefs))
            .await
            .map_err(failed)?;
        d.peer_preferences = prefs;
        drop(d);
        self.0.changed().await;
        Ok(())
    }
    async fn forget_peer(&self, peer_id: String) -> zbus::fdo::Result<()> {
        let mut d = self.0.data.lock().await;
        let mut prefs = d.peer_preferences.clone();
        prefs.remove(&peer_id);
        save_config(&self.0.data_dir.join("peers.json"), &json!(prefs))
            .await
            .map_err(failed)?;
        d.peer_preferences = prefs;
        drop(d);
        self.0.changed().await;
        Ok(())
    }
    async fn export_diagnostics(&self) -> String {
        let d = self.0.data.lock().await;
        let backends: Vec<_> = d
            .backends
            .values()
            .map(|b| json!({"id":b.id,"state":b.state}))
            .collect();
        let radios:Vec<_>=d.hardware["radios"].as_array().into_iter().flatten().map(|r|json!({"driver":r["driver"],"firmware":r["firmware"],"bands":r["bands"],"rfkill":r["rfkill"],"protected":r["protected"],"monitor":r["monitor"]})).collect();
        json!({"version":env!("CARGO_PKG_VERSION"),"platform":"linux","backends":backends,"radios":radios,"active_transfers":d.transfers.values().filter(|t|!t.is_terminal()).count(),"redacted":true,"omitted":["names","addresses","serials","paths","keys","PINs","file names","error details"]}).to_string()
    }
    async fn restart_backends(&self) -> zbus::fdo::Result<()> {
        let mut d = self.0.data.lock().await;
        d.accepting_transfers()?;
        if d.transfers.values().any(|t| !t.is_terminal()) {
            return Err(failed(
                "Finish or cancel active transfers before restarting",
            ));
        }
        if self
            .0
            .download_offer
            .lock()
            .await
            .as_ref()
            .is_some_and(|offer| offer.is_active())
        {
            return Err(failed(
                "Stop the download link before restarting sharing or changing network settings",
            ));
        }
        let permit = self.0.restart.try_reserve().map_err(failed)?;
        d.restarting = true;
        permit.send(());
        drop(d);
        self.0.changed().await;
        Ok(())
    }
    async fn stop_when_idle(&self) -> zbus::fdo::Result<()> {
        {
            let mut d = self.0.data.lock().await;
            if let Some(offer) = self.0.download_offer.lock().await.as_ref() {
                offer.quiesce();
            }
            d.stop_when_idle = true;
        }
        self.set_visibility("hidden".into()).await?;
        Ok(())
    }
    async fn reset_settings(&self) -> zbus::fdo::Result<()> {
        self.update_settings(settings::defaults().to_string()).await
    }
    async fn get_diagnostics(&self) -> String {
        let d = self.0.data.lock().await;
        json!({"version":env!("CARGO_PKG_VERSION"),"epoch":d.epoch,"backends":d.backends,"hardware":d.hardware,"active_transfers":d.transfers.values().filter(|t|!t.is_terminal()).count()}).to_string()
    }
    async fn prepare_send(&self, paths: Vec<String>) -> zbus::fdo::Result<String> {
        let mut d = self.0.data.lock().await;
        if d.stop_when_idle {
            return Err(failed(
                "LinuxDrop is finishing active transfers before closing",
            ));
        }
        d.drafts
            .retain(|_, draft| draft.created.elapsed() < Duration::from_secs(1800));
        if d.drafts.len() >= 64
            || paths.is_empty()
            || paths.len() > d.settings["receive"]["max_files"].as_u64().unwrap_or(1000) as usize
        {
            return Err(failed("Invalid file selection"));
        }
        let max_bytes = d.settings["receive"]["max_bytes"].as_u64().unwrap();
        let files = tokio::task::spawn_blocking(move || -> Result<Vec<SendSource>> {
            let mut total = 0u64;
            let mut files = Vec::new();
            for path in paths {
                let path = PathBuf::from(path);
                if !path.is_absolute() {
                    anyhow::bail!("File paths must be absolute");
                }
                let source = SendSource::open(&path)?;
                total = total
                    .checked_add(source.size())
                    .context("File size overflow")?;
                if total > max_bytes {
                    anyhow::bail!("Selection exceeds configured size limit");
                }
                files.push(source);
            }
            Ok(files)
        })
        .await
        .map_err(failed)?
        .map_err(failed)?;
        let id = Uuid::new_v4().to_string();
        d.drafts.insert(
            id.clone(),
            Draft {
                files,
                created: Instant::now(),
            },
        );
        Ok(id)
    }
    /// Empty draft_id creates a selection; subsequent bounded batches append atomically.
    async fn prepare_send_files(
        &self,
        draft_id: String,
        entries: Vec<(String, zbus::zvariant::OwnedFd)>,
    ) -> zbus::fdo::Result<String> {
        if entries.is_empty() || entries.len() > 16 {
            return Err(failed("Send between 1 and 16 file descriptors per batch"));
        }
        let files = entries
            .into_iter()
            .map(|(name, fd)| {
                let fd: std::os::fd::OwnedFd = fd.into();
                SendSource::from_file(name, std::fs::File::from(fd)).map_err(failed)
            })
            .collect::<zbus::fdo::Result<Vec<_>>>()?;
        let mut d = self.0.data.lock().await;
        if d.stop_when_idle {
            return Err(failed(
                "LinuxDrop is finishing active transfers before closing",
            ));
        }
        d.drafts
            .retain(|_, draft| draft.created.elapsed() < Duration::from_secs(1800));
        let old = if draft_id.is_empty() {
            if d.drafts.len() >= 64 {
                return Err(failed("Too many prepared selections"));
            }
            None
        } else {
            Some(
                d.drafts
                    .get(&draft_id)
                    .ok_or_else(|| failed("File selection expired"))?,
            )
        };
        let existing = old.map(|draft| draft.files.as_slice()).unwrap_or_default();
        if existing.len() + files.len()
            > d.settings["receive"]["max_files"].as_u64().unwrap_or(1000) as usize
        {
            return Err(failed("Invalid file selection"));
        }
        let total = existing
            .iter()
            .chain(&files)
            .try_fold(0u64, |total, source| total.checked_add(source.size()))
            .ok_or_else(|| failed("File size overflow"))?;
        if total > d.settings["receive"]["max_bytes"].as_u64().unwrap() {
            return Err(failed("Selection exceeds configured size limit"));
        }
        let id = if draft_id.is_empty() {
            Uuid::new_v4().to_string()
        } else {
            draft_id
        };
        d.drafts
            .entry(id.clone())
            .or_insert_with(|| Draft {
                files: Vec::new(),
                created: Instant::now(),
            })
            .files
            .extend(files);
        Ok(id)
    }
    async fn discard_draft(&self, draft_id: String) {
        self.0.data.lock().await.drafts.remove(&draft_id);
    }
    async fn create_download_offer(&self, draft_id: String) -> zbus::fdo::Result<String> {
        let mut d = self.0.data.lock().await;
        d.accepting_transfers()?;
        let local = linuxdrop_network::interfaces(&transfer_policy(&d.settings), false)
            .map_err(failed)?
            .into_iter()
            .find(|interface| interface.address.is_ipv4())
            .ok_or_else(|| {
                failed("No enabled IPv4 LAN interface for a download link; check network settings")
            })?;
        let sources = d
            .drafts
            .get(&draft_id)
            .ok_or_else(|| failed("File selection expired"))?
            .files
            .clone();
        let config = linuxdrop_localsend::reverse::OfferConfig {
            alias: d.settings["general"]["device_name"]
                .as_str()
                .unwrap()
                .into(),
            bind: std::net::SocketAddr::new(local.address, 53318),
            expires_after: Duration::from_secs(600),
            max_files: d.settings["receive"]["max_files"].as_u64().unwrap() as usize,
            max_bytes: d.settings["receive"]["max_bytes"].as_u64().unwrap(),
        };
        let mut current = self.0.download_offer.lock().await;
        if let Some(offer) = current.as_ref() {
            if !offer.quiesce_if_idle() {
                return Err(failed(
                    "Finish or stop the current download before creating another link",
                ));
            }
            offer.shutdown().await.map_err(failed)?;
        }
        current.take();
        let budget = self.0.bandwidth.lock().await.clone();
        let offer =
            linuxdrop_localsend::reverse::start_sources_with_budget(config, sources, budget)
                .await
                .map_err(failed)?;
        let result=json!({"url":format!("http://{}",offer.address),"pin":offer.pin,"expires_in":600,"encrypted":false}).to_string();
        *current = Some(offer);
        d.drafts.remove(&draft_id);
        drop(current);
        drop(d); // Offer publication is serialized with restart admission.
        self.0.changed().await;
        Ok(result)
    }
    async fn stop_download_offer(&self) -> zbus::fdo::Result<()> {
        let mut current = self.0.download_offer.lock().await;
        if let Some(offer) = current.as_ref() {
            offer.shutdown().await.map_err(failed)?;
        }
        current.take();
        drop(current);
        self.0.changed().await;
        Ok(())
    }
    async fn receive_download_offer(&self, url: String) -> zbus::fdo::Result<String> {
        if self.0.locked.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(failed("Unlock the session to receive files"));
        }
        let mut d = self.0.data.lock().await;
        d.accepting_transfers()?;
        if d.transfers
            .values()
            .filter(|transfer| !transfer.is_terminal())
            .count()
            >= d.settings["transfers"]["max_parallel"].as_u64().unwrap() as usize
        {
            return Err(failed("Parallel transfer limit reached"));
        }
        let (address, _) =
            linuxdrop_localsend::download::offer_address(&url, &transfer_policy(&d.settings))
                .map_err(failed)?;
        if d.settings["localsend"]["enabled"] != true {
            return Err(failed("Enable LocalSend before receiving a download offer"));
        }
        let sender = d
            .commands
            .get("localsend")
            .ok_or_else(|| failed("LocalSend is restarting or unavailable; wait for its status or restart it from Settings"))?;
        let id = Uuid::new_v4().to_string();
        sender
            .try_send(BackendCommand::ReceiveOffer {
                transfer_id: id.clone(),
                url,
            })
            .map_err(failed)?;
        d.transfer_order.push(id.clone());
        d.transfers.insert(
            id.clone(),
            linuxdrop_localsend::download::pending_transfer(id.clone(), address),
        );
        drop(d);
        self.0.changed().await;
        Ok(id)
    }
    async fn start_send(
        &self,
        draft_id: String,
        peer_id: String,
        protocol: String,
    ) -> zbus::fdo::Result<String> {
        let mut d = self.0.data.lock().await;
        d.accepting_transfers()?;
        if d.transfers.values().filter(|t| !t.is_terminal()).count()
            >= d.settings["transfers"]["max_parallel"].as_u64().unwrap() as usize
        {
            return Err(failed("Parallel transfer limit reached"));
        }
        let peer = d
            .peers
            .get(&peer_id)
            .ok_or_else(|| failed("Device is no longer available"))?;
        if d.peer_preferences.get(&peer_id).is_some_and(|p| p.blocked) {
            return Err(failed("This device is blocked"));
        }
        if protocol == "airdrop" && d.settings["airdrop"]["send"] != true {
            return Err(failed("AirDrop sending is disabled"));
        }
        if !peer.protocols.contains(&protocol) || !peer.available {
            return Err(failed("Protocol unavailable for this device"));
        }
        let peer_name = peer.name.clone();
        let tx = d
            .commands
            .get(&protocol)
            .cloned()
            .ok_or_else(|| failed("Backend not ready"))?;
        let draft = d
            .drafts
            .remove(&draft_id)
            .ok_or_else(|| failed("File selection expired"))?;
        let id = Uuid::new_v4().to_string();
        let mut files = Vec::new();
        for source in &draft.files {
            source.verify().map_err(failed)?;
            files.push(TransferFile {
                name: source.name().into(),
                size: source.size(),
                transferred: 0,
            });
        }
        let total_bytes = files.iter().map(|f| f.size).sum();
        d.transfer_order.push(id.clone());
        d.transfers.insert(
            id.clone(),
            Transfer {
                id: id.clone(),
                peer_id: peer_id.clone(),
                peer_name,
                protocol: protocol.clone(),
                direction: "outgoing".into(),
                state: "waiting".into(),
                files,
                total_bytes,
                transferred_bytes: 0,
                error: None,
                verification_code: None,
                saved_paths: vec![],
            },
        );
        drop(d);
        if tx
            .send(BackendCommand::Send {
                transfer_id: id.clone(),
                peer_id,
                files: draft.files,
            })
            .await
            .is_err()
        {
            if let Some(transfer) = self.0.data.lock().await.transfers.get_mut(&id) {
                transfer.state = "failed".into();
                transfer.error = Some("Backend stopped before sending".into());
            }
            let _ = self.0.persist_history().await;
            self.0.changed().await;
            return Err(failed("Backend stopped"));
        }
        self.0.changed().await;
        Ok(id)
    }
    async fn accept_transfer(&self, id: String) -> zbus::fdo::Result<()> {
        self.0.action(&id, "accept").await
    }
    async fn accept_transfer_with_options(
        &self,
        id: String,
        options: String,
    ) -> zbus::fdo::Result<()> {
        if options.len() > 131072 {
            return Err(failed("Receive options too large"));
        }
        self.0
            .accept(&id, Some(serde_json::from_str(&options).map_err(failed)?))
            .await
    }
    async fn provide_transfer_pin(&self, id: String, pin: String) -> zbus::fdo::Result<()> {
        if !(4..=12).contains(&pin.len()) || !pin.bytes().all(|b| b.is_ascii_digit()) {
            return Err(failed("PIN must contain 4 to 12 digits"));
        }
        let d = self.0.data.lock().await;
        let t = d
            .transfers
            .get(&id)
            .ok_or_else(|| failed("Unknown transfer"))?;
        if t.state != "pin_required" || t.protocol != "localsend" {
            return Err(failed("Transfer is not awaiting a PIN"));
        }
        d.commands
            .get(&t.protocol)
            .ok_or_else(|| failed("Backend unavailable"))?
            .try_send(BackendCommand::ProvidePin {
                transfer_id: id,
                pin,
            })
            .map_err(failed)
    }
    async fn run_hardware_diagnostic(
        &self,
        radio_id: String,
        channel: u16,
    ) -> zbus::fdo::Result<String> {
        if self
            .0
            .data
            .lock()
            .await
            .transfers
            .values()
            .any(|t| !t.is_terminal())
        {
            return Err(failed("Finish transfers before testing an adapter"));
        }
        let mut client = linuxdrop_netd::Client::connect().await.map_err(failed)?;
        let response = tokio::time::timeout(
            Duration::from_secs(90),
            client.request(&linuxdrop_netd::Request::Diagnose { radio_id, channel }),
        )
        .await
        .map_err(failed)?
        .map_err(failed)?;
        match response {
            linuxdrop_netd::Response::Diagnostic { report } => {
                serde_json::to_string(&report).map_err(failed)
            }
            linuxdrop_netd::Response::Error { message } => Err(failed(message)),
            _ => Err(failed("Unexpected diagnostic response")),
        }
    }
    async fn get_recovery_status(&self) -> zbus::fdo::Result<String> {
        let mut client = linuxdrop_netd::Client::connect().await.map_err(failed)?;
        let response = client
            .request(&linuxdrop_netd::Request::RecoveryStatus)
            .await
            .map_err(failed)?;
        serde_json::to_string(&response).map_err(failed)
    }
    async fn reject_transfer(&self, id: String) -> zbus::fdo::Result<()> {
        self.0.action(&id, "reject").await
    }
    async fn cancel_transfer(&self, id: String) -> zbus::fdo::Result<()> {
        self.0.action(&id, "cancel").await
    }
    async fn clear_history(&self) -> zbus::fdo::Result<()> {
        let mut d = self.0.data.lock().await;
        d.transfers.retain(|_, t| !t.is_terminal());
        d.completed_at.clear();
        let active: Vec<_> = d
            .transfer_order
            .iter()
            .filter(|id| d.transfers.contains_key(*id))
            .cloned()
            .collect();
        d.transfer_order = active;
        drop(d);
        self.0.persist_history().await.map_err(failed)?;
        self.0.changed().await;
        Ok(())
    }
    async fn set_visibility(&self, mode: String) -> zbus::fdo::Result<()> {
        if mode == "everyone" && self.0.locked.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(failed("Unlock the session before making it visible"));
        }
        if !matches!(mode.as_str(), "hidden" | "everyone") {
            return Err(failed("Invalid visibility"));
        }
        let mut d = self.0.data.lock().await;
        if mode == "everyone" && d.stop_when_idle {
            return Err(failed("LinuxDrop is stopping"));
        }
        d.settings["visibility"]["mode"] = json!(mode);
        d.visibility_since = if mode == "everyone" {
            Some(Instant::now())
        } else {
            None
        };
        let commands: Vec<_> = d
            .commands
            .iter()
            .map(|(id, tx)| {
                (
                    tx.clone(),
                    mode == "everyone"
                        && (id != "airdrop" || d.settings["airdrop"]["receive"] == true),
                )
            })
            .collect();
        drop(d);
        for (tx, visible) in commands {
            let _ = tx.send(BackendCommand::SetVisibility { visible }).await;
        }
        self.0.changed().await;
        Ok(())
    }
    async fn update_settings(&self, patch: String) -> zbus::fdo::Result<()> {
        if patch.len() > 65536 {
            return Err(failed("Settings patch too large"));
        }
        let patch: Value = serde_json::from_str(&patch).map_err(failed)?;
        let mut d = self.0.data.lock().await;
        let mut next = d.settings.clone();
        settings::merge(&mut next, &patch).map_err(failed)?;
        settings::validate(&next).map_err(failed)?;
        let restart = settings::needs_backend_restart(&d.settings, &next);
        if restart {
            d.accepting_transfers()?;
            if self
                .0
                .download_offer
                .lock()
                .await
                .as_ref()
                .is_some_and(|offer| offer.is_active())
            {
                return Err(failed(
                    "Stop the download link before restarting sharing or changing network settings",
                ));
            }
        }
        let restart_permit = if restart {
            Some(self.0.restart.try_reserve().map_err(failed)?)
        } else {
            None
        };
        if restart && d.transfers.values().any(|t| !t.is_terminal()) {
            return Err(failed(
                "Finish or cancel active transfers before changing network settings",
            ));
        }
        if next["visibility"]["mode"] == "everyone"
            && self.0.locked.load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(failed("Unlock the session before making it visible"));
        }
        save_config(&self.0.config_path, &next)
            .await
            .map_err(failed)?;
        apply_autostart(next["general"]["autostart"].as_bool().unwrap())
            .await
            .map_err(failed)?;
        self.0
            .log_filter
            .reload(tracing_subscriber::EnvFilter::new(format!(
                "warn,linuxdrop={}",
                next["diagnostics"]["log_level"].as_str().unwrap_or("info")
            )))
            .map_err(failed)?;
        let visibility_changed = d.settings["visibility"]["mode"] != next["visibility"]["mode"];
        if visibility_changed {
            d.visibility_since = if next["visibility"]["mode"] == "everyone" {
                Some(Instant::now())
            } else {
                None
            };
        }
        let mode = next["visibility"]["mode"].as_str().unwrap().to_owned();
        d.settings = next;
        if let Some(permit) = restart_permit {
            d.restarting = true;
            permit.send(());
        }
        drop(d);
        if !restart && visibility_changed {
            self.set_visibility(mode).await?;
        }
        self.0.changed().await;
        Ok(())
    }
    async fn open_application(&self) -> zbus::fdo::Result<()> {
        tokio::process::Command::new("linuxdrop")
            .arg("open")
            .spawn()
            .map_err(failed)?;
        Ok(())
    }
}

async fn save_config(path: &std::path::Path, value: &Value) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    let parent = path.parent().context("Config parent missing")?;
    tokio::fs::create_dir_all(parent).await?;
    let temporary = parent.join(format!(".settings-{}.tmp", Uuid::new_v4()));
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .await?;
    file.write_all(&serde_json::to_vec_pretty(value)?).await?;
    file.sync_all().await?;
    drop(file);
    tokio::fs::rename(temporary, path).await?;
    Ok(())
}
async fn apply_autostart(enabled: bool) -> Result<()> {
    let base = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(std::env::var("HOME").unwrap()).join(".config"));
    let file = base.join("autostart/io.github.marius4lui.LinuxDrop.desktop");
    if enabled {
        tokio::fs::create_dir_all(file.parent().unwrap()).await?;
        tokio::fs::write(file,"[Desktop Entry]\nType=Application\nName=LinuxDrop\nExec=linuxdropd\nNoDisplay=true\nX-GNOME-Autostart-enabled=true\n").await?;
    } else if file.exists() {
        tokio::fs::remove_file(file).await?;
    }
    Ok(())
}

async fn boot_backends(shared: &Arc<Shared>, output: mpsc::Sender<(u64, BackendEvent)>) {
    // Advance the generation under the same lock used to apply fault/events.
    let generation = {
        let mut data = shared.data.lock().await;
        if data.stop_when_idle {
            return;
        }
        let generation = shared
            .backend_generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
            + 1;
        data.failed_backends.clear();
        generation
    };
    let (events, mut receiver) = mpsc::channel(256);
    shared.event_forwarders.spawn(async move {
        while let Some(event) = receiver.recv().await {
            if output.send((generation, event)).await.is_err() {
                break;
            }
        }
    });
    let (settings, previous) = {
        let mut d = shared.data.lock().await;
        d.peers.clear();
        for id in ["localsend", "quickshare", "airdrop"] {
            d.backends.insert(
                id.into(),
                BackendState {
                    id: id.into(),
                    state: "starting".into(),
                    detail: "Applying sharing settings".into(),
                },
            );
        }
        let mut previous = std::mem::take(&mut d.retiring);
        previous.extend(std::mem::take(&mut d.commands));
        (d.settings.clone(), previous)
    };
    shared.changed().await;
    shared.data.lock().await.retiring = previous.clone();
    let mut stopping = tokio::task::JoinSet::new();
    for (id, commands) in previous {
        stopping.spawn(async move {
            let result = commands.shutdown().await;
            (id, commands, result)
        });
    }
    let mut failures = Vec::new();
    while let Some(stopped) = stopping.join_next().await {
        match stopped {
            Ok((id, commands, Err(error))) => {
                failures.push(format!("{id}: {error}"));
                shared.data.lock().await.retiring.insert(id, commands);
            }
            Err(error) => failures.push(format!("Backend shutdown failed: {error}")),
            Ok((id, _, Ok(()))) => {
                shared.data.lock().await.retiring.remove(&id);
            }
        }
    }
    {
        let mut current = shared.download_offer.lock().await;
        if let Some(offer) = current.as_ref() {
            match offer.shutdown().await {
                Ok(()) => {
                    current.take();
                }
                Err(error) => failures.push(format!("Download link: {error}")),
            }
        }
    }
    if failures.is_empty() {
        let (airdrop, quickshare) = tokio::join!(
            helper::release(&shared.helper),
            helper::release(&shared.quickshare_helper),
        );
        for (name, result) in [("AirDrop", airdrop), ("Quick Share", quickshare)] {
            if let Err(error) = result {
                failures.push(format!("{name} radio restoration: {error}"));
            }
        }
    }
    if !failures.is_empty() {
        let mut d = shared.data.lock().await;
        for state in d.backends.values_mut() {
            state.state = "error".into();
            state.detail = format!("Sharing restart could not finish: {}", failures.join("; "));
        }
        d.restarting = false;
        drop(d);
        shared.changed().await;
        return;
    }
    let bandwidth = linuxdrop_network::BandwidthLimiter::new(
        transfer_policy(&settings).bandwidth_bytes_per_second,
    );
    *shared.bandwidth.lock().await = bandwidth.clone();
    let name = settings["general"]["device_name"]
        .as_str()
        .unwrap()
        .to_string();
    let directory = PathBuf::from(settings["receive"]["directory"].as_str().unwrap());
    // Do not accept offers on a partially installed generation. Apply the latest
    // visibility only after every command channel is registered below.
    let visible = false;
    if shared.data.lock().await.stop_when_idle {
        return;
    }
    if settings["localsend"]["enabled"] == true {
        let result = linuxdrop_localsend::start_with_budget(
            linuxdrop_localsend::Config {
                name: name.clone(),
                download_dir: directory.clone(),
                identity_dir: shared.data_dir.clone(),
                visible,
                port: settings["localsend"]["port"].as_u64().unwrap() as u16,
                https: settings["localsend"]["https"] == true,
                multicast: settings["localsend"]["multicast"] == true,
                max_files: settings["receive"]["max_files"].as_u64().unwrap() as usize,
                max_bytes: settings["receive"]["max_bytes"].as_u64().unwrap(),
                policy: transfer_policy(&settings),
                receive_pin: (settings["localsend"]["require_pin"] == true).then(|| {
                    settings["localsend"]["pin"]
                        .as_str()
                        .unwrap_or_default()
                        .into()
                }),
            },
            events.clone(),
            bandwidth.clone(),
        )
        .await;
        install_backend(shared, "localsend", result).await;
    } else {
        disabled(shared, "localsend").await;
    }
    if shared.data.lock().await.stop_when_idle {
        return;
    }
    // Allocate jointly before either backend reserves a radio. AirDrop needs
    // its radio; Quick Share can continue on LAN when only one is available.
    let inventory = linuxdrop_hardware::inventory().await;
    let mut allocation = radio_allocation_request(&settings);
    let plan = linuxdrop_hardware::allocation::allocate(&inventory, &allocation);
    if settings["airdrop"]["enabled"] == true {
        let result = start_airdrop(
            shared,
            &settings,
            plan.airdrop,
            name.clone(),
            directory.clone(),
            visible,
            events.clone(),
        )
        .await;
        install_backend(shared, "airdrop", result).await;
    } else {
        disabled(shared, "airdrop").await;
    }
    if shared.data.lock().await.stop_when_idle {
        return;
    }
    if settings["quickshare"]["enabled"] == true {
        // A failed AirDrop startup can release its radio for Direct Wi-Fi. A
        // retained failed-cleanup lease must remain excluded until restoration.
        allocation.airdrop = false;
        if let Some(lease) = shared.helper.lock().await.as_ref() {
            allocation.leased.push(lease.expected.phy.clone());
        }
        let upgrade_radio =
            linuxdrop_hardware::allocation::allocate(&inventory, &allocation).quickshare;
        let upgrade_lease = if let Some(radio_id) = upgrade_radio {
            match reserve_direct_wifi(shared, radio_id).await {
                Ok(lease) => Some(lease),
                Err(error) => {
                    tracing::warn!(%error,"Quick Share direct Wi-Fi unavailable; continuing on LAN");
                    None
                }
            }
        } else {
            None
        };
        let result = linuxdrop_quickshare::start_with_budget(
            linuxdrop_quickshare::Config {
                name: name.clone(),
                download_dir: directory.clone(),
                visible,
                port: Some(settings["quickshare"]["port"].as_u64().unwrap() as u16),
                ble: settings["quickshare"]["ble"] == true,
                max_receive_bytes: settings["receive"]["max_bytes"].as_u64().unwrap(),
                max_files: settings["receive"]["max_files"].as_u64().unwrap() as usize,
                p2p_connector: upgrade_lease.as_ref().map(|lease| {
                    Arc::new(HelperP2p {
                        shared: Arc::downgrade(shared),
                        lease_id: lease.lease_id.clone(),
                    }) as Arc<dyn linuxdrop_network::P2pConnector>
                }),
                upgrade_lease,
                policy: transfer_policy(&settings),
            },
            events.clone(),
            bandwidth.clone(),
        )
        .await;
        install_backend(shared, "quickshare", result).await;
    } else {
        disabled(shared, "quickshare").await;
    }
    if shared.data.lock().await.stop_when_idle {
        return;
    }
    {
        let mut d = shared.data.lock().await;
        let visible = d.settings["visibility"]["mode"] == "everyone"
            && !shared.locked.load(std::sync::atomic::Ordering::Relaxed)
            && !d.stop_when_idle;
        for (id, tx) in &d.commands {
            let _ = tx.try_send(BackendCommand::SetVisibility {
                visible: visible && (id != "airdrop" || d.settings["airdrop"]["receive"] == true),
            });
        }
        d.restarting = false;
    }
    shared.changed().await;
}
async fn install_backend(shared: &Arc<Shared>, id: &str, result: Result<CommandSender>) {
    // Startup can acquire a radio before sockets/discovery are ready. Release
    // that ownership before publishing failure or allowing the next backend
    // to select hardware. Never await helper I/O while holding Data.
    let result = match id {
        "quickshare" => helper::release_after_failure(&shared.quickshare_helper, result).await,
        "airdrop" => helper::release_after_failure(&shared.helper, result).await,
        _ => result,
    };
    let mut data = shared.data.lock().await;
    match result {
        Ok(tx) => {
            if data.failed_backends.contains(id) || data.stop_when_idle {
                data.retiring.insert(id.into(), tx.clone());
                helper::drain(id.to_owned(), tx);
            } else {
                data.commands.insert(id.into(), tx);
            }
        }
        Err(_) if data.failed_backends.contains(id) => {}
        Err(error) => {
            data.backends.insert(
                id.into(),
                BackendState {
                    id: id.into(),
                    state: "error".into(),
                    detail: error.to_string(),
                },
            );
        }
    }
}
async fn disabled(shared: &Arc<Shared>, id: &str) {
    shared.data.lock().await.backends.insert(
        id.into(),
        BackendState {
            id: id.into(),
            state: "disabled".into(),
            detail: "Disabled in settings".into(),
        },
    );
}

fn radio_allocation_request(settings: &Value) -> linuxdrop_hardware::allocation::Request {
    linuxdrop_hardware::allocation::Request {
        airdrop: settings["airdrop"]["enabled"] == true,
        quickshare: settings["quickshare"]["enabled"] == true,
        preferred: settings["hardware"]["preferred_adapter"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(str::to_owned),
        prefer_usb: settings["hardware"]["prefer_usb"] == true,
        auto_use_usb: settings["hardware"]["auto_use_usb"] == true,
        leased: vec![],
    }
}

async fn start_airdrop(
    shared: &Arc<Shared>,
    settings: &Value,
    choice: Option<linuxdrop_hardware::allocation::AwdlChoice>,
    name: String,
    directory: PathBuf,
    visible: bool,
    events: EventSender,
) -> Result<CommandSender> {
    let choice = choice.context("No idle monitor-capable WLAN adapter with a permitted AWDL channel. Attach a dedicated adapter; physical compatibility is still experimental.")?;
    let radio_id = choice.radio_id;
    let channel = choice.channel;
    let mut client = linuxdrop_netd::Client::connect()
        .await
        .context("AirDrop network helper is not installed or running")?;
    let response = tokio::time::timeout(
        Duration::from_secs(90),
        client.request(&linuxdrop_netd::Request::AcquireAwdl { radio_id, channel }),
    )
    .await??;
    let lease = match response {
        linuxdrop_netd::Response::Acquired { lease } => *lease,
        linuxdrop_netd::Response::Error { message } => anyhow::bail!(message),
        _ => anyhow::bail!("Unexpected helper response"),
    };
    let interface = lease.awdl_interface.clone();
    *shared.helper.lock().await = Some(helper::HelperLease::new(client, lease, shared));
    let interface = interface.context("Helper did not return an AWDL interface")?;
    let bandwidth = shared.bandwidth.lock().await.clone();
    linuxdrop_airdrop::start_with_budget(
        linuxdrop_airdrop::Config {
            name,
            download_dir: directory,
            interface,
            visible: visible && settings["airdrop"]["receive"] == true,
            ble_wake: settings["airdrop"]["ble_wakeup"] == true,
            max_receive_bytes: settings["receive"]["max_bytes"].as_u64().unwrap(),
            max_files: settings["receive"]["max_files"].as_u64().unwrap() as usize,
            policy: transfer_policy(settings),
        },
        events,
        bandwidth,
    )
    .await
}

async fn reserve_direct_wifi(
    shared: &Arc<Shared>,
    radio_id: String,
) -> Result<linuxdrop_quickshare::DirectWifiLease> {
    let mut client = linuxdrop_netd::Client::connect().await?;
    let response = tokio::time::timeout(
        Duration::from_secs(90),
        client.request(&linuxdrop_netd::Request::Reserve { radio_id }),
    )
    .await??;
    let lease = match response {
        linuxdrop_netd::Response::Acquired { lease } => lease,
        linuxdrop_netd::Response::Error { message } => anyhow::bail!(message),
        _ => anyhow::bail!("Unexpected direct Wi-Fi helper response"),
    };
    let result = lease
        .connection_uuid
        .clone()
        .context("Direct Wi-Fi lease missing its connection ownership marker")
        .map(|connection_uuid| linuxdrop_quickshare::DirectWifiLease {
            interface: lease.interface.clone(),
            lease_id: lease.id.clone(),
            connection_uuid,
            capabilities: lease.direct_capabilities.clone(),
        });
    *shared.quickshare_helper.lock().await = Some(helper::HelperLease::new(client, *lease, shared));
    helper::release_after_failure(&shared.quickshare_helper, result).await
}

fn transfer_policy(settings: &Value) -> TransferPolicy {
    let limit = settings["transfers"]["bandwidth_limit_mbps"]
        .as_u64()
        .unwrap_or(0);
    TransferPolicy {
        allowed_interfaces: settings["network"]["allowed_interfaces"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        allow_virtual_interfaces: settings["network"]["allow_virtual_interfaces"] == true,
        bandwidth_bytes_per_second: (limit > 0).then_some(limit * 125_000),
        bluetooth_adapter: settings["bluetooth"]["adapter"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::args().any(|a| a == "--version") {
        println!("linuxdropd {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    let (filter, log_filter) = tracing_subscriber::reload::Layer::new(
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "warn,linuxdrop=info".into()),
    );
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .init();
    let home = PathBuf::from(std::env::var("HOME")?);
    let config_dir = std::env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".config"))
        .join("linuxdrop");
    let data_dir = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".local/share"))
        .join("linuxdrop");
    let config_path = config_dir.join("settings.json");
    let mut settings = settings::defaults();
    if config_path.exists() {
        let value = serde_json::from_slice(&tokio::fs::read(&config_path).await?)?;
        settings::merge(&mut settings, &value)?;
        settings::validate(&settings)?;
    }
    // Public visibility never survives a daemon restart unnoticed.
    if std::env::var_os("RUST_LOG").is_none() {
        log_filter.reload(tracing_subscriber::EnvFilter::new(format!(
            "warn,linuxdrop={}",
            settings["diagnostics"]["log_level"]
                .as_str()
                .unwrap_or("info")
        )))?;
    }
    settings["visibility"]["mode"] = json!("hidden");
    let history_path = data_dir.join("history.json");
    let history: Vec<history::Entry> = if let Ok(bytes) = tokio::fs::read(&history_path).await {
        serde_json::from_slice(&bytes).unwrap_or_default()
    } else {
        vec![]
    };
    let history: Vec<_> = history
        .into_iter()
        .filter(|entry| {
            entry.transfer.is_terminal()
                && history::retained(
                    entry.completed_at,
                    settings["transfers"]["history_days"].as_u64().unwrap_or(30),
                    history::now(),
                )
        })
        .rev()
        .take(settings["transfers"]["history_limit"].as_u64().unwrap() as usize)
        .collect();
    let transfer_order: Vec<_> = history
        .iter()
        .rev()
        .map(|t| t.transfer.id.clone())
        .collect();
    let completed_at = history
        .iter()
        .map(|entry| (entry.transfer.id.clone(), entry.completed_at))
        .collect();
    let transfers = history
        .into_iter()
        .map(|entry| (entry.transfer.id.clone(), entry.transfer))
        .collect();
    let peer_preferences = tokio::fs::read(data_dir.join("peers.json"))
        .await
        .ok()
        .and_then(|bytes| {
            serde_json::from_slice::<HashMap<String, preferences::PeerPreferences>>(&bytes).ok()
        })
        .filter(|prefs| prefs.len() <= 1000)
        .unwrap_or_default();
    let (restart, mut restarts) = mpsc::channel(4);
    let bandwidth = linuxdrop_network::BandwidthLimiter::new(
        transfer_policy(&settings).bandwidth_bytes_per_second,
    );
    let shared = Arc::new(Shared {
        bandwidth: Mutex::new(bandwidth),
        data: Mutex::new(Data {
            epoch: Uuid::new_v4().to_string(),
            revision: 0,
            settings,
            peers: HashMap::new(),
            peer_preferences,
            transfers,
            transfer_order,
            completed_at,
            backends: HashMap::new(),
            hardware: json!({"radios":[],"interfaces":[],"bluetooth":[],"warnings":[]}),
            drafts: HashMap::new(),
            commands: HashMap::new(),
            retiring: HashMap::new(),
            failed_backends: HashSet::new(),
            visibility_since: None,
            decisions: HashSet::new(),
            stop_when_idle: false,
            restarting: true,
        }),
        connection: std::sync::OnceLock::new(),
        config_path,
        data_dir,
        restart,
        helper: Mutex::new(None),
        quickshare_helper: Mutex::new(None),
        backend_generation: std::sync::atomic::AtomicU64::new(0),
        event_forwarders: tokio_util::task::TaskTracker::new(),
        locked: std::sync::atomic::AtomicBool::new(false),
        download_offer: Mutex::new(None),
        history_io: Mutex::new(()),
        log_filter,
    });
    let connection = zbus::connection::Builder::session()?
        .name(SERVICE)?
        .serve_at(OBJECT, Manager(shared.clone()))?
        .build()
        .await?;
    shared.connection.set(connection.clone()).ok();
    notifications::start(shared.clone());
    let stop = tokio_util::sync::CancellationToken::new();
    let mut terminating =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    let mut interrupting =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
    let helper = shared.clone();
    let helper_stop = stop.clone();
    let helper_watch = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = helper_stop.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_secs(30)) => {}
            }
            for (backend, socket) in [
                ("airdrop", &helper.helper),
                ("quickshare", &helper.quickshare_helper),
            ] {
                helper::poll(&helper, backend, socket).await;
            }
        }
    });
    let (events, mut event_rx) = mpsc::channel(256);
    let supervisor = shared.clone();
    let event_tx = events.clone();
    let supervisor_stop = stop.clone();
    let supervisor = tokio::spawn(async move {
        boot_backends(&supervisor, event_tx.clone()).await;
        loop {
            tokio::select! {
                biased;
                _ = supervisor_stop.cancelled() => break,
                request = restarts.recv() => {
                    if request.is_none() { break; }
                    boot_backends(&supervisor, event_tx.clone()).await;
                }
            }
        }
    });
    let hardware = shared.clone();
    let hardware_stop = stop.clone();
    let hardware_watch = tokio::spawn(async move {
        let mut inventories = linuxdrop_hardware::watch_inventory();
        let mut initialized = false;
        let mut radio_changes = linuxdrop_hardware::allocation::HotplugAllocation::default();
        loop {
            tokio::select! {
                biased;
                _ = hardware_stop.cancelled() => break,
                result = inventories.changed() => { if result.is_err() { break; } }
            }
            let inventory = inventories.borrow_and_update().clone();
            let mut value = serde_json::to_value(&inventory).unwrap_or_default();
            // Read lease ownership before Data; helper I/O never runs under Data.
            let awdl_phy = hardware
                .helper
                .lock()
                .await
                .as_ref()
                .map(|lease| lease.expected.phy.clone());
            let direct_phy = hardware
                .quickshare_helper
                .lock()
                .await
                .as_ref()
                .map(|lease| lease.expected.phy.clone());
            if let Some(radios) = value["radios"].as_array_mut() {
                for radio in radios {
                    radio["reserved_for"] = if radio["phy"].as_str() == awdl_phy.as_deref() {
                        json!("AirDrop")
                    } else if radio["phy"].as_str() == direct_phy.as_deref() {
                        json!("Quick Share")
                    } else {
                        json!("Not reserved")
                    };
                }
            }
            let link_active = hardware
                .download_offer
                .lock()
                .await
                .as_ref()
                .is_some_and(|offer| offer.is_active());
            let mut d = hardware.data.lock().await;
            let changed = d.hardware != value;
            let show_adapter = initialized
                && d.settings["hardware"]["open_on_adapter"] == true
                && !hardware.locked.load(std::sync::atomic::Ordering::Relaxed)
                && value["radios"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|radio| {
                        radio["bus"] == "usb"
                            && !d.hardware["radios"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .any(|old| old["id"] == radio["id"])
                    });
            let mut missing = radio_allocation_request(&d.settings);
            missing.airdrop &= awdl_phy.is_none();
            missing.quickshare &= direct_phy.is_none();
            missing
                .leased
                .extend(awdl_phy.into_iter().chain(direct_phy));
            let busy = d.restarting
                || d.stop_when_idle
                || link_active
                || d.transfers.values().any(|t| !t.is_terminal());
            let retry = radio_changes.observe(&inventory, &missing, busy);
            d.hardware = value;
            initialized = true;
            drop(d);
            if retry && Manager(hardware.clone()).restart_backends().await.is_ok() {
                radio_changes.queued();
            }
            if changed {
                hardware.changed().await;
            }
            if show_adapter {
                let _ = tokio::process::Command::new("linuxdrop")
                    .arg("--hardware")
                    .spawn();
            }
        }
    });
    let visibility = shared.clone();
    let visibility_stop = stop.clone();
    let visibility_watch = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = visibility_stop.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_secs(1)) => {}
            }
            let d = visibility.data.lock().await;
            let duration = d.settings["visibility"]["duration_minutes"]
                .as_u64()
                .unwrap_or(10);
            let expired = duration > 0
                && d.visibility_since
                    .is_some_and(|since| since.elapsed() > Duration::from_secs(duration * 60));
            drop(d);
            if expired {
                let _ = Manager(visibility.clone())
                    .set_visibility("hidden".into())
                    .await;
            }
        }
    });
    let mut shutdown_tick = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _=terminating.recv()=>break,
            _=interrupting.recv()=>break,
            _=shutdown_tick.tick()=>{
                let mut d=shared.data.lock().await;
                d.drafts.retain(|_, draft| draft.created.elapsed() < Duration::from_secs(1800));
                if d.stop_when_idle && !d.transfers.values().any(|t|!t.is_terminal())
                    && shared.download_offer.lock().await.as_ref().is_none_or(|offer| offer.active_downloads() == 0) {break;}
            },
            Some((generation,event))=event_rx.recv()=>{
                handle_event(&shared, generation, event).await;
            }
        }
    }
    lifecycle::begin_stop(&shared, &stop).await;
    let cleanup = tokio::time::timeout(
        Duration::from_secs(120),
        lifecycle::finish(
            &shared,
            supervisor,
            vec![helper_watch, hardware_watch, visibility_watch],
        ),
    );
    tokio::pin!(cleanup);
    // Backends can publish more than one channel's capacity while draining.
    // Keep processing their terminal events until their forwarders also stop.
    let outcome = loop {
        tokio::select! {
            result = &mut cleanup => break result,
            Some((generation, event)) = event_rx.recv() => handle_event(&shared, generation, event).await,
        }
    };
    while let Ok((generation, event)) = event_rx.try_recv() {
        handle_event(&shared, generation, event).await;
    }
    lifecycle::finish_history(&shared).await?;
    outcome.context(
        "LinuxDrop cleanup exceeded 120 seconds; some resources could not be confirmed stopped",
    )??;
    Ok(())
}

async fn handle_event(shared: &Arc<Shared>, generation: u64, event: BackendEvent) {
    let mut d = shared.data.lock().await;
    if generation
        != shared
            .backend_generation
            .load(std::sync::atomic::Ordering::Acquire)
    {
        return;
    }
    let Some(event) = helper::filter_event(&d, event) else {
        return;
    };
    let mut notification = None;
    if let BackendEvent::Incoming(t) | BackendEvent::TransferUpdated(t) = &event {
        if !d.transfers.contains_key(&t.id) {
            d.transfer_order.push(t.id.clone());
        }
    }
    match event {
        BackendEvent::PeerUpsert(peer) => {
            d.peers.insert(peer.id.clone(), peer);
        }
        BackendEvent::PeerRemoved { peer_id } => {
            d.peers.remove(&peer_id);
        }
        BackendEvent::Incoming(mut transfer) => {
            if !d
                .transfers
                .get(&transfer.id)
                .is_some_and(Transfer::is_terminal)
            {
                let full = !d.transfers.contains_key(&transfer.id)
                    && d.transfers.values().filter(|t| !t.is_terminal()).count()
                        >= d.settings["transfers"]["max_parallel"].as_u64().unwrap() as usize;
                let blocked = d
                    .peer_preferences
                    .get(&transfer.peer_id)
                    .is_some_and(|p| p.blocked);
                let receive_disabled =
                    transfer.protocol == "airdrop" && d.settings["airdrop"]["receive"] != true;
                if full
                    || blocked
                    || receive_disabled
                    || d.failed_backends.contains(&transfer.protocol)
                    || d.restarting
                    || d.stop_when_idle
                    || shared.locked.load(std::sync::atomic::Ordering::Relaxed)
                {
                    if let Some(tx) = d.commands.get(&transfer.protocol) {
                        let _ = tx.try_send(BackendCommand::Reject {
                            transfer_id: transfer.id.clone(),
                        });
                    }
                    transfer.state = "rejected".into();
                    transfer.error = Some("LinuxDrop is not accepting this request".into());
                    d.completed_at.insert(transfer.id.clone(), history::now());
                } else {
                    notification = Some(transfer.clone());
                }
                d.transfers.insert(transfer.id.clone(), transfer);
            }
        }
        BackendEvent::TransferUpdated(transfer) => {
            if transfer.is_terminal() {
                d.decisions.remove(&transfer.id);
                d.completed_at
                    .entry(transfer.id.clone())
                    .or_insert_with(history::now);
            }
            if d.transfers
                .get(&transfer.id)
                .is_none_or(|old| old.accepts_update(&transfer))
            {
                if transfer.is_terminal() {
                    notification = Some(transfer.clone());
                }
                d.transfers.insert(transfer.id.clone(), transfer);
            }
        }
        BackendEvent::StateChanged(state) => {
            d.backends.insert(state.id.clone(), state);
        }
    }
    let persist = notification.as_ref().is_some_and(Transfer::is_terminal);
    let history = if persist {
        let keep = d.settings["transfers"]["history_limit"]
            .as_u64()
            .unwrap_or(100) as usize;
        let finished: Vec<_> = d
            .transfer_order
            .iter()
            .filter(|id| d.transfers.get(*id).is_some_and(Transfer::is_terminal))
            .cloned()
            .collect();
        for id in finished.iter().take(finished.len().saturating_sub(keep)) {
            d.transfers.remove(id);
        }
        let expired: Vec<_> = d
            .completed_at
            .iter()
            .filter(|(_, timestamp)| {
                !history::retained(
                    **timestamp,
                    d.settings["transfers"]["history_days"]
                        .as_u64()
                        .unwrap_or(30),
                    history::now(),
                )
            })
            .map(|(id, _)| id.clone())
            .collect();
        for id in expired {
            d.transfers.remove(&id);
            d.completed_at.remove(&id);
        }
        let order: Vec<_> = d
            .transfer_order
            .iter()
            .filter(|id| d.transfers.contains_key(*id))
            .cloned()
            .collect();
        d.transfer_order = order;
        Some(json!(d
            .transfer_order
            .iter()
            .filter_map(|id| d.transfers.get(id))
            .filter(|t| t.is_terminal())
            .collect::<Vec<_>>()))
    } else {
        None
    };
    drop(d);
    shared.changed().await;
    if history.is_some() {
        if let Err(error) = shared.persist_history().await {
            tracing::warn!(%error,"Could not save transfer history");
        }
    }
    if let Some(transfer) = notification {
        let shared = shared.clone();
        tokio::spawn(async move {
            notifications::notify(shared.clone(), transfer.clone()).await;
            notifications::after_receive(shared, transfer).await;
        });
    }
}
