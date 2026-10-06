//! Opt-in local demonstration target; never installed or started by the product.
#[path = "../src/tls.rs"]
mod tls;

use anyhow::{bail, Context, Result};
use linuxdrop_core::{BackendCommand, BackendEvent};
use linuxdrop_localsend::{start_bound, Config};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{io::BufReader, net::Ipv4Addr, path::PathBuf, time::Duration};
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut port: u16 = std::env::var("LINUXDROP_DEMO_PORT")
        .unwrap_or_else(|_| "53319".into())
        .parse()?;
    let mut daemon_port: u16 = std::env::var("LINUXDROP_DEMO_DAEMON_PORT")
        .unwrap_or_else(|_| "53317".into())
        .parse()?;
    let mut autoaccept = false;
    let mut send_back = false;
    let mut options = args.iter();
    while let Some(option) = options.next() {
        match option.as_str() {
            "--enable-autoaccept"=>autoaccept=true,
            "--send-back"=>send_back=true,
            "--port"=>port=options.next().context("--port requires a number")?.parse()?,
            "--daemon-port"=>daemon_port=options.next().context("--daemon-port requires a number")?.parse()?,
            _=>bail!("Unknown argument: {option}. Use --enable-autoaccept, optional --send-back, --port NUMBER and --daemon-port NUMBER."),
        }
    }
    if !autoaccept {
        bail!("Explicit opt-in required: demo_peer --enable-autoaccept. This LOCAL DEMO automatically accepts files on loopback only.");
    }
    if port == 0 || daemon_port == 0 || port == daemon_port {
        bail!("Demo and daemon ports must be distinct nonzero ports");
    }
    let home = PathBuf::from(std::env::var("HOME").context("HOME is missing")?);
    let root = std::env::var_os("LINUXDROP_DEMO_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("LinuxDrop-Demo"));
    let identity = root.join("identity");
    let received = root.join("received");
    std::fs::create_dir_all(&received)?;
    let sample = root.join("Zum-Testen.txt");
    if !sample.exists() {
        std::fs::write(&sample, "Hallo von LinuxDrop!\n\nDiese Datei wird mit dem echten LocalSend-Protokoll über HTTPS übertragen.\nDer Demo-Empfänger ist nur auf diesem Ubuntu-System erreichbar.\n")?;
    }
    let daemon_data = std::env::var_os("LINUXDROP_DEMO_DAEMON_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".local/share"))
                .join("linuxdrop")
        });
    let public_cert = std::fs::read(daemon_data.join("localsend-cert.pem"))
        .context("Start the real LinuxDrop daemon first; its public TLS certificate is required")?;
    let der = rustls_pemfile::certs(&mut BufReader::new(public_cert.as_slice()))
        .next()
        .transpose()?
        .context("Daemon certificate missing")?;
    let daemon_fingerprint = hex::encode(Sha256::digest(der.as_ref()));
    let (events, mut receiver) = mpsc::channel(256);
    let commands = start_bound(
        Config {
            name: "Demo-Empfänger (Ubuntu)".into(),
            download_dir: received.clone(),
            identity_dir: identity.clone(),
            visible: true,
            port,
            https: true,
            multicast: false,
            max_files: 100,
            max_bytes: 1024 * 1024 * 1024,
        },
        events,
        Ipv4Addr::LOCALHOST,
    )
    .await?;
    let (_, _, fingerprint) = tls::identity(&identity)?;
    let own_client = tls::client("https", &fingerprint)?;
    let client = tls::client("https", &daemon_fingerprint)?;
    let info = json!({"alias":"Demo-Empfänger (Ubuntu)","version":"2.1","deviceModel":"LinuxDrop local demo","deviceType":"desktop","fingerprint":fingerprint,"port":port,"protocol":"https","download":false});
    let daemon_info = json!({"alias":"LinuxDrop (echte Oberfläche)","version":"2.1","deviceType":"desktop","fingerprint":daemon_fingerprint,"port":daemon_port,"protocol":"https","download":false});
    let daemon_register = format!("https://127.0.0.1:{daemon_port}/api/localsend/v2/register");
    own_client
        .post(format!(
            "https://127.0.0.1:{port}/api/localsend/v2/register"
        ))
        .json(&daemon_info)
        .send()
        .await?
        .error_for_status()?;
    println!("LOKALE DEMO: automatische Annahme ausschließlich auf 127.0.0.1:{port}; LinuxDrop auf Port {daemon_port}.");
    println!(
        "Gerät: Demo-Empfänger (Ubuntu)\nDatei zum Ziehen: {}\nEmpfangsordner: {}",
        sample.display(),
        received.display()
    );
    let mut registration = tokio::time::interval(Duration::from_secs(20));
    loop {
        tokio::select! {
            _=tokio::signal::ctrl_c()=>break,
            _=registration.tick()=>{
                match client.post(&daemon_register).json(&info).timeout(Duration::from_secs(5)).send().await {
                    Ok(response) if response.status().is_success()=>{
                        let remote:Value=response.json().await?;
                        println!("Mit LinuxDrop verbunden: {}",remote["alias"].as_str().unwrap_or("LinuxDrop"));
                        if send_back {
                            send_back=false;
                            commands.send(BackendCommand::Send{transfer_id:"demo-send-back".into(),peer_id:format!("localsend:{daemon_fingerprint}"),files:vec![sample.clone()]}).await?;
                            println!("Explizit angeforderter Empfangstest gestartet; bitte in LinuxDrop annehmen.");
                        }
                    },
                    Ok(response) if response.status()==reqwest::StatusCode::FORBIDDEN=>println!("Demo-Gerät registriert. LinuxDrop ist verborgen; Senden an die Demo ist trotzdem möglich."),
                    Ok(response)=>eprintln!("Registrierung: {}",response.status()),
                    Err(error)=>eprintln!("LinuxDrop noch nicht erreichbar: {error}"),
                }
            },
            event=receiver.recv()=>match event {
                Some(BackendEvent::Incoming(transfer))=>{
                    println!("DEMO nimmt {} Datei(en) von {} an.",transfer.files.len(),transfer.peer_name);
                    commands.send(BackendCommand::Accept{transfer_id:transfer.id}).await?;
                },
                Some(BackendEvent::TransferUpdated(transfer)) if transfer.is_terminal()=>{
                    println!("Übertragung {}: {} ({} / {} Bytes)",transfer.id,transfer.state,transfer.transferred_bytes,transfer.total_bytes);
                    for path in transfer.saved_paths {println!("Gespeichert: {path}");}
                    if let Some(error)=transfer.error {eprintln!("{error}");}
                },
                Some(BackendEvent::StateChanged(state)) if state.state=="error"=>eprintln!("Demo-Backend: {}",state.detail),
                None=>break,
                _=>{},
            }
        }
    }
    commands.send(BackendCommand::Shutdown).await.ok();
    Ok(())
}
