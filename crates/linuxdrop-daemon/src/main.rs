mod notifications;
mod settings;
use anyhow::{Context, Result};
use linuxdrop_core::*;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, Mutex};
use uuid::Uuid;

const SERVICE: &str = "io.github.marius4lui.LinuxDrop";
const OBJECT: &str = "/io/github/marius4lui/LinuxDrop";
const INTERFACE: &str = "io.github.marius4lui.LinuxDrop.Manager1";

struct Draft {
    paths: Vec<PathBuf>,
    created: Instant,
}
struct Data {
    epoch: String,
    revision: u64,
    settings: Value,
    peers: HashMap<String, Peer>,
    transfers: HashMap<String, Transfer>,
    transfer_order: Vec<String>,
    backends: HashMap<String, BackendState>,
    hardware: Value,
    drafts: HashMap<String, Draft>,
    commands: HashMap<String, CommandSender>,
    visibility_since: Option<Instant>,
}
struct Shared {
    data: Mutex<Data>,
    connection: std::sync::OnceLock<zbus::Connection>,
    config_path: PathBuf,
    data_dir: PathBuf,
    restart: mpsc::Sender<()>,
    helper: Mutex<Option<linuxdrop_netd::Client>>,
    locked: std::sync::atomic::AtomicBool,
    download_offer: Mutex<Option<linuxdrop_localsend::reverse::ReverseOffer>>,
    history_io: Mutex<()>,
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
        let transfers: Vec<_> = d
            .transfer_order
            .iter()
            .rev()
            .filter_map(|id| d.transfers.get(id))
            .collect();
        json!({"epoch":d.epoch,"revision":d.revision,"peers":peers,"transfers":transfers,"backends":d.backends.values().collect::<Vec<_>>(),"hardware":d.hardware,"settings":d.settings}).to_string()
    }
    async fn action(&self, id: &str, action: &str) -> zbus::fdo::Result<()> {
        if action == "accept" && self.locked.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(failed("Unlock the session to accept files"));
        }
        let data = self.data.lock().await;
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
        if action == "accept" && !matches!(transfer.state.as_str(), "waiting" | "verification") {
            return Err(failed("Transfer is not awaiting a decision"));
        }
        let command = match action {
            "accept" => BackendCommand::Accept {
                transfer_id: id.into(),
            },
            "reject" => BackendCommand::Reject {
                transfer_id: id.into(),
            },
            _ => BackendCommand::Cancel {
                transfer_id: id.into(),
            },
        };
        drop(data);
        tx.send(command)
            .await
            .map_err(|_| failed("Backend stopped"))
    }
}
fn failed(message: impl ToString) -> zbus::fdo::Error {
    zbus::fdo::Error::Failed(message.to_string())
}
struct Manager(Arc<Shared>);
#[zbus::interface(name = "io.github.marius4lui.LinuxDrop.Manager1")]
impl Manager {
    async fn get_snapshot(&self) -> String {
        self.0.snapshot().await
    }
    async fn get_settings(&self) -> String {
        self.0.data.lock().await.settings.to_string()
    }
    async fn get_diagnostics(&self) -> String {
        let d = self.0.data.lock().await;
        json!({"version":env!("CARGO_PKG_VERSION"),"epoch":d.epoch,"backends":d.backends,"hardware":d.hardware,"active_transfers":d.transfers.values().filter(|t|!t.is_terminal()).count()}).to_string()
    }
    async fn prepare_send(&self, paths: Vec<String>) -> zbus::fdo::Result<String> {
        let mut d = self.0.data.lock().await;
        d.drafts
            .retain(|_, draft| draft.created.elapsed() < Duration::from_secs(1800));
        if d.drafts.len() >= 64
            || paths.is_empty()
            || paths.len() > d.settings["receive"]["max_files"].as_u64().unwrap_or(1000) as usize
        {
            return Err(failed("Invalid file selection"));
        }
        let mut total = 0u64;
        let mut files = Vec::new();
        for path in paths {
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err(failed("File paths must be absolute"));
            }
            let metadata = tokio::fs::metadata(&path).await.map_err(failed)?;
            if !metadata.is_file() {
                return Err(failed("Only regular files are currently supported"));
            }
            total = total
                .checked_add(metadata.len())
                .ok_or_else(|| failed("File size overflow"))?;
            files.push(path);
        }
        if total > d.settings["receive"]["max_bytes"].as_u64().unwrap() {
            return Err(failed("Selection exceeds configured size limit"));
        }
        let id = Uuid::new_v4().to_string();
        d.drafts.insert(
            id.clone(),
            Draft {
                paths: files,
                created: Instant::now(),
            },
        );
        Ok(id)
    }
    async fn discard_draft(&self, draft_id: String) {
        self.0.data.lock().await.drafts.remove(&draft_id);
    }
    async fn create_download_offer(&self, draft_id: String) -> zbus::fdo::Result<String> {
        let mut d = self.0.data.lock().await;
        let draft = d
            .drafts
            .remove(&draft_id)
            .ok_or_else(|| failed("File selection expired"))?;
        let config = linuxdrop_localsend::reverse::OfferConfig {
            alias: d.settings["general"]["device_name"]
                .as_str()
                .unwrap()
                .into(),
            bind: "0.0.0.0:53318".parse().unwrap(),
            expires_after: Duration::from_secs(600),
            max_files: d.settings["receive"]["max_files"].as_u64().unwrap() as usize,
            max_bytes: d.settings["receive"]["max_bytes"].as_u64().unwrap(),
        };
        drop(d);
        let mut current = self.0.download_offer.lock().await;
        current.take();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let offer = linuxdrop_localsend::reverse::start_offer(config, draft.paths)
            .await
            .map_err(failed)?;
        let ip = local_address().await.unwrap_or_else(|| "127.0.0.1".into());
        let result=json!({"url":format!("http://{}:{}",ip,offer.address.port()),"pin":offer.pin,"expires_in":600,"encrypted":false}).to_string();
        *current = Some(offer);
        Ok(result)
    }
    async fn stop_download_offer(&self) {
        self.0.download_offer.lock().await.take();
    }
    async fn start_send(
        &self,
        draft_id: String,
        peer_id: String,
        protocol: String,
    ) -> zbus::fdo::Result<String> {
        let mut d = self.0.data.lock().await;
        if d.transfers.values().filter(|t| !t.is_terminal()).count()
            >= d.settings["transfers"]["max_parallel"].as_u64().unwrap() as usize
        {
            return Err(failed("Parallel transfer limit reached"));
        }
        let peer = d
            .peers
            .get(&peer_id)
            .ok_or_else(|| failed("Device is no longer available"))?;
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
        for p in &draft.paths {
            let m = tokio::fs::metadata(p).await.map_err(failed)?;
            files.push(TransferFile {
                name: p.file_name().unwrap_or_default().to_string_lossy().into(),
                size: m.len(),
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
                files: draft.paths,
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
    async fn reject_transfer(&self, id: String) -> zbus::fdo::Result<()> {
        self.0.action(&id, "reject").await
    }
    async fn cancel_transfer(&self, id: String) -> zbus::fdo::Result<()> {
        self.0.action(&id, "cancel").await
    }
    async fn clear_history(&self) -> zbus::fdo::Result<()> {
        let mut d = self.0.data.lock().await;
        d.transfers.retain(|_, t| !t.is_terminal());
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
        d.settings["visibility"]["mode"] = json!(mode);
        d.visibility_since = if mode == "everyone" {
            Some(Instant::now())
        } else {
            None
        };
        let commands: Vec<_> = d.commands.values().cloned().collect();
        drop(d);
        for tx in commands {
            let _ = tx
                .send(BackendCommand::SetVisibility {
                    visible: mode == "everyone",
                })
                .await;
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
        if d.transfers.values().any(|t| !t.is_terminal()) {
            return Err(failed(
                "Finish or cancel active transfers before changing settings",
            ));
        }
        let mut next = d.settings.clone();
        settings::merge(&mut next, &patch).map_err(failed)?;
        settings::validate(&next).map_err(failed)?;
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
        d.visibility_since = if next["visibility"]["mode"] == "everyone" {
            Some(Instant::now())
        } else {
            None
        };
        d.settings = next;
        drop(d);
        self.0.restart.send(()).await.map_err(failed)?;
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

async fn local_address() -> Option<String> {
    // Route lookup via UDP connect does not send a datagram.
    let socket = tokio::net::UdpSocket::bind("0.0.0.0:0").await.ok()?;
    socket.connect("192.0.2.1:9").await.ok()?;
    let address = socket.local_addr().ok()?.ip();
    if address.is_unspecified() {
        None
    } else {
        Some(address.to_string())
    }
}

async fn boot_backends(shared: &Arc<Shared>, events: EventSender) {
    let (settings, previous) = {
        let mut d = shared.data.lock().await;
        d.peers.clear();
        (d.settings.clone(), std::mem::take(&mut d.commands))
    };
    for (_, tx) in previous {
        let _ = tx.send(BackendCommand::Shutdown).await;
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    shared.helper.lock().await.take();
    let name = settings["general"]["device_name"]
        .as_str()
        .unwrap()
        .to_string();
    let directory = PathBuf::from(settings["receive"]["directory"].as_str().unwrap());
    let visible = settings["visibility"]["mode"] == "everyone";
    if settings["localsend"]["enabled"] == true {
        let result = linuxdrop_localsend::start(
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
            },
            events.clone(),
        )
        .await;
        install_backend(shared, "localsend", result).await;
    } else {
        disabled(shared, "localsend").await;
    }
    if settings["quickshare"]["enabled"] == true {
        let preferred = settings["hardware"]["preferred_adapter"]
            .as_str()
            .unwrap_or("");
        let inventory = linuxdrop_hardware::inventory().await;
        let upgrade_interface = inventory
            .radios
            .iter()
            .find(|r| r.id == preferred && !r.protected && !r.rfkill)
            .and_then(|r| r.interfaces.first())
            .cloned();
        let result = linuxdrop_quickshare::start(
            linuxdrop_quickshare::Config {
                name: name.clone(),
                download_dir: directory.clone(),
                visible,
                port: None,
                ble: settings["quickshare"]["ble"] == true,
                max_receive_bytes: settings["receive"]["max_bytes"].as_u64().unwrap(),
                max_files: settings["receive"]["max_files"].as_u64().unwrap() as usize,
                upgrade_interface,
            },
            events.clone(),
        )
        .await;
        install_backend(shared, "quickshare", result).await;
    } else {
        disabled(shared, "quickshare").await;
    }
    if settings["airdrop"]["enabled"] == true {
        let result =
            start_airdrop(shared, &settings, name, directory, visible, events.clone()).await;
        install_backend(shared, "airdrop", result).await;
    } else {
        disabled(shared, "airdrop").await;
    }
    shared.changed().await;
}
async fn install_backend(shared: &Arc<Shared>, id: &str, result: Result<CommandSender>) {
    let mut data = shared.data.lock().await;
    match result {
        Ok(tx) => {
            data.commands.insert(id.into(), tx);
        }
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

async fn start_airdrop(
    shared: &Arc<Shared>,
    settings: &Value,
    name: String,
    directory: PathBuf,
    visible: bool,
    events: EventSender,
) -> Result<CommandSender> {
    let inventory = linuxdrop_hardware::inventory().await;
    let preferred = settings["hardware"]["preferred_adapter"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let mut chosen = None;
    for channel in [44, 6, 149] {
        let choice = linuxdrop_hardware::select_radio(
            &inventory,
            &linuxdrop_hardware::SelectionRequest {
                preferred: preferred.clone(),
                channel: Some(channel),
                require_tested_awdl: false,
                leased: vec![],
                prefer_usb: settings["hardware"]["prefer_usb"] == true,
            },
        );
        if let Some(id) = choice.selected {
            chosen = Some((id, channel));
            break;
        }
    }
    let (radio_id,channel)=chosen.context("No idle monitor-capable WLAN adapter with a permitted AWDL channel. Attach a dedicated adapter; physical compatibility is still experimental.")?;
    let mut client = linuxdrop_netd::Client::connect()
        .await
        .context("AirDrop network helper is not installed or running")?;
    let response = tokio::time::timeout(
        Duration::from_secs(90),
        client.request(&linuxdrop_netd::Request::AcquireAwdl { radio_id, channel }),
    )
    .await??;
    let interface = match response {
        linuxdrop_netd::Response::Acquired { lease } => lease
            .awdl_interface
            .context("Helper did not return an AWDL interface")?,
        linuxdrop_netd::Response::Error { message } => anyhow::bail!(message),
        _ => anyhow::bail!("Unexpected helper response"),
    };
    let tx = linuxdrop_airdrop::start(
        linuxdrop_airdrop::Config {
            name,
            download_dir: directory,
            interface,
            visible,
            ble_wake: settings["airdrop"]["ble_wakeup"] == true,
            max_receive_bytes: settings["receive"]["max_bytes"].as_u64().unwrap(),
            max_files: settings["receive"]["max_files"].as_u64().unwrap() as usize,
        },
        events,
    )
    .await?;
    *shared.helper.lock().await = Some(client);
    Ok(tx)
}

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::args().any(|a| a == "--version") {
        println!("linuxdropd {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "linuxdrop=info".into()),
        )
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
    settings["visibility"]["mode"] = json!("hidden");
    let history_path = data_dir.join("history.json");
    let history: Vec<Transfer> = if let Ok(bytes) = tokio::fs::read(&history_path).await {
        serde_json::from_slice(&bytes).unwrap_or_default()
    } else {
        vec![]
    };
    let history: Vec<_> = history
        .into_iter()
        .filter(Transfer::is_terminal)
        .rev()
        .take(settings["transfers"]["history_limit"].as_u64().unwrap() as usize)
        .collect();
    let transfer_order: Vec<_> = history.iter().rev().map(|t| t.id.clone()).collect();
    let transfers = history.into_iter().map(|t| (t.id.clone(), t)).collect();
    let (restart, mut restarts) = mpsc::channel(4);
    let shared = Arc::new(Shared {
        data: Mutex::new(Data {
            epoch: Uuid::new_v4().to_string(),
            revision: 0,
            settings,
            peers: HashMap::new(),
            transfers,
            transfer_order,
            backends: HashMap::new(),
            hardware: json!({"radios":[],"interfaces":[],"bluetooth":[],"warnings":[]}),
            drafts: HashMap::new(),
            commands: HashMap::new(),
            visibility_since: None,
        }),
        connection: std::sync::OnceLock::new(),
        config_path,
        data_dir,
        restart,
        helper: Mutex::new(None),
        locked: std::sync::atomic::AtomicBool::new(false),
        download_offer: Mutex::new(None),
        history_io: Mutex::new(()),
    });
    let connection = zbus::connection::Builder::session()?
        .name(SERVICE)?
        .serve_at(OBJECT, Manager(shared.clone()))?
        .build()
        .await?;
    shared.connection.set(connection.clone()).ok();
    notifications::start(shared.clone());
    let helper = shared.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let mut connection = helper.helper.lock().await;
            if let Some(client) = connection.as_mut() {
                if !matches!(tokio::time::timeout(Duration::from_secs(5),client.request(&linuxdrop_netd::Request::Status)).await,Ok(Ok(linuxdrop_netd::Response::State{leases,..})) if !leases.is_empty())
                {
                    connection.take();
                    drop(connection);
                    let mut d = helper.data.lock().await;
                    if let Some(tx) = d.commands.remove("airdrop") {
                        let _ = tx.send(BackendCommand::Shutdown).await;
                    }
                    d.backends.insert(
                        "airdrop".into(),
                        BackendState {
                            id: "airdrop".into(),
                            state: "error".into(),
                            detail: "AWDL helper connection lost; radio lease released".into(),
                        },
                    );
                    drop(d);
                    helper.changed().await;
                }
            }
        }
    });
    let (events, mut event_rx) = mpsc::channel(256);
    let supervisor = shared.clone();
    let event_tx = events.clone();
    tokio::spawn(async move {
        boot_backends(&supervisor, event_tx.clone()).await;
        while restarts.recv().await.is_some() {
            boot_backends(&supervisor, event_tx.clone()).await;
        }
    });
    let hardware = shared.clone();
    tokio::spawn(async move {
        let mut inventories = linuxdrop_hardware::watch_inventory();
        while inventories.changed().await.is_ok() {
            let value =
                serde_json::to_value(inventories.borrow_and_update().clone()).unwrap_or_default();
            let mut d = hardware.data.lock().await;
            let changed = d.hardware != value;
            let radios_changed = d.hardware["radios"] != value["radios"];
            let retry = radios_changed
                && d.settings["airdrop"]["enabled"] == true
                && d.backends
                    .get("airdrop")
                    .is_some_and(|b| b.state == "error")
                && !d.transfers.values().any(|t| !t.is_terminal());
            d.hardware = value;
            drop(d);
            if retry {
                let _ = hardware.restart.try_send(());
            }
            if changed {
                hardware.changed().await;
            }
        }
    });
    let visibility = shared.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
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
    loop {
        tokio::select! {
            _=tokio::signal::ctrl_c()=>break,
            Some(event)=event_rx.recv()=>{
                let mut d=shared.data.lock().await;
                let mut notification=None;
                if let BackendEvent::Incoming(t)|BackendEvent::TransferUpdated(t)=&event {
                    if !d.transfers.contains_key(&t.id){d.transfer_order.push(t.id.clone());}
                }
                match event {
                    BackendEvent::PeerUpsert(peer)=>{d.peers.insert(peer.id.clone(),peer);},
                    BackendEvent::PeerRemoved{peer_id}=>{d.peers.remove(&peer_id);},
                    BackendEvent::Incoming(transfer)=>{
                        if !d.transfers.get(&transfer.id).is_some_and(Transfer::is_terminal) {
                            let full=!d.transfers.contains_key(&transfer.id) && d.transfers.values().filter(|t|!t.is_terminal()).count() >= d.settings["transfers"]["max_parallel"].as_u64().unwrap() as usize;
                            if full || shared.locked.load(std::sync::atomic::Ordering::Relaxed) {
                                if let Some(tx)=d.commands.get(&transfer.protocol) {let _=tx.try_send(BackendCommand::Reject{transfer_id:transfer.id.clone()});}
                            } else {notification=Some(transfer.clone());}
                            d.transfers.insert(transfer.id.clone(),transfer);
                        }
                    },
                    BackendEvent::TransferUpdated(transfer)=>{
                        if d.transfers.get(&transfer.id).is_none_or(|old|old.accepts_update(&transfer)){
                            if transfer.is_terminal(){notification=Some(transfer.clone());}
                            d.transfers.insert(transfer.id.clone(),transfer);
                        }
                    },
                    BackendEvent::StateChanged(state)=>{d.backends.insert(state.id.clone(),state);}
                }
                let persist=notification.as_ref().is_some_and(Transfer::is_terminal);
                let history=if persist {
                    let keep=d.settings["transfers"]["history_limit"].as_u64().unwrap_or(100) as usize;
                    let finished:Vec<_>=d.transfer_order.iter().filter(|id|d.transfers.get(*id).is_some_and(Transfer::is_terminal)).cloned().collect();
                    for id in finished.iter().take(finished.len().saturating_sub(keep)){d.transfers.remove(id);}
                    let order:Vec<_>=d.transfer_order.iter().filter(|id|d.transfers.contains_key(*id)).cloned().collect();d.transfer_order=order;
                    Some(json!(d.transfer_order.iter().filter_map(|id|d.transfers.get(id)).filter(|t|t.is_terminal()).collect::<Vec<_>>()))
                }else{None};
                drop(d);shared.changed().await;
                if history.is_some() {if let Err(error)=shared.persist_history().await {tracing::warn!(%error,"Could not save transfer history");}}
                if let Some(transfer)=notification {
                    let shared=shared.clone();tokio::spawn(async move{notifications::notify(shared.clone(),transfer.clone()).await;notifications::after_receive(shared,transfer).await;});
                }
            }
        }
    }
    for tx in shared.data.lock().await.commands.values() {
        let _ = tx.send(BackendCommand::Shutdown).await;
    }
    Ok(())
}
