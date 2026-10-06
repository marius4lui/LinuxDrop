use anyhow::{Context, Result, bail};
use bytes::Bytes;
use futures_util::StreamExt;
use http_body_util::{BodyExt, Full, StreamBody, combinators::UnsyncBoxBody};
use hyper::{Request, body::Frame};
use hyper_util::rt::TokioIo;
use linuxdrop_core::{BackendEvent, Transfer, TransferFile};
use rustls::{
    DigitallySignedStruct, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use std::{
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tokio_util::{io::ReaderStream, sync::CancellationToken};
type Body = UnsyncBoxBody<Bytes, std::io::Error>;

/// AirDrop Everyone accepts self-signed certs. Verify TLS proof of possession
/// and pin the first certificate throughout Discover/Ask/Upload. This is TOFU,
/// not an Apple account identity, and is deliberately scoped to one transfer.
#[derive(Debug)]
struct SessionVerifier {
    pin: Mutex<Option<Vec<u8>>>,
}
impl ServerCertVerifier for SessionVerifier {
    fn verify_server_cert(
        &self,
        end: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let mut pin = self
            .pin
            .lock()
            .map_err(|_| rustls::Error::General("Certificate state unavailable".into()))?;
        match &*pin {
            Some(first) if first.as_slice() != end.as_ref() => {
                return Err(rustls::Error::General(
                    "AirDrop receiver certificate changed during transfer".into(),
                ));
            }
            None => *pin = Some(end.as_ref().to_vec()),
            _ => {}
        }
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
fn full(bytes: Vec<u8>) -> Body {
    Full::new(Bytes::from(bytes))
        .map_err(|never| match never {})
        .boxed_unsync()
}
struct ConnectionTask(tokio::task::JoinHandle<Result<(), hyper::Error>>);
impl Drop for ConnectionTask {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn post(
    address: SocketAddr,
    config: Arc<rustls::ClientConfig>,
    path: &str,
    mime: &str,
    body: Body,
    length: Option<u64>,
) -> Result<Vec<u8>> {
    let tcp = tokio::time::timeout(Duration::from_secs(10), connect_on_awdl(address)).await??;
    let tls = tokio::time::timeout(
        Duration::from_secs(10),
        tokio_rustls::TlsConnector::from(config).connect(ServerName::try_from("airdrop")?, tcp),
    )
    .await??;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls)).await?;
    let task = ConnectionTask(tokio::spawn(connection));
    let mut request = Request::builder()
        .method("POST")
        .uri(path)
        .header("host", "airdrop._tcp.local.")
        .header("content-type", mime);
    if let Some(length) = length {
        request = request.header("content-length", length);
    }
    let result = async {
        let response = sender.send_request(request.body(body)?).await?;
        if response.status() != hyper::StatusCode::OK {
            bail!("AirDrop {path} returned {}", response.status());
        }
        let mut body = response.into_body();
        let mut bytes = vec![];
        while let Some(frame) = body.frame().await {
            if let Ok(data) = frame?.into_data() {
                if bytes.len() + data.len() > 1024 * 1024 {
                    bail!("AirDrop response exceeded metadata limit");
                }
                bytes.extend_from_slice(&data);
            }
        }
        Ok(bytes)
    }
    .await;
    drop(task);
    result
}

async fn connect_on_awdl(address: SocketAddr) -> Result<tokio::net::TcpStream> {
    match address {
        SocketAddr::V4(peer) if peer.ip().is_loopback() => {
            // Local protocol harnesses only; production discovery supplies an
            // IPv6 address scoped to the netd-leased AWDL interface.
            Ok(tokio::net::TcpStream::connect(address).await?)
        }
        SocketAddr::V6(peer) if peer.scope_id() != 0 => {
            let interfaces = linuxdrop_network::interfaces(
                &linuxdrop_core::TransferPolicy {
                    allow_virtual_interfaces: true,
                    ..Default::default()
                },
                false,
            )?;
            let local = interfaces.into_iter().find(|interface| interface.index == peer.scope_id() && matches!(interface.address, std::net::IpAddr::V6(ip) if ip.is_unicast_link_local()))
                .context("The leased AWDL interface no longer has its IPv6 address")?;
            let std::net::IpAddr::V6(ip) = local.address else {
                unreachable!()
            };
            let socket = tokio::net::TcpSocket::new_v6()?;
            socket.bind_device(Some(local.name.as_bytes()))?;
            socket.bind(SocketAddr::V6(std::net::SocketAddrV6::new(
                ip,
                0,
                0,
                local.index,
            )))?;
            Ok(socket.connect(address).await?)
        }
        _ => bail!("AirDrop requires an IPv6 peer on the leased AWDL interface"),
    }
}

pub async fn send(
    address: SocketAddr,
    name: &str,
    files: Vec<PathBuf>,
    transfer: &mut Transfer,
    events: &mpsc::Sender<BackendEvent>,
    cancel: CancellationToken,
    bandwidth: linuxdrop_network::BandwidthLimiter,
) -> Result<()> {
    tokio::select! {_=cancel.cancelled()=>bail!("Transfer cancelled"),result=send_inner(address,name,files,transfer,events,cancel.clone(),bandwidth)=>result}
}
async fn send_inner(
    address: SocketAddr,
    name: &str,
    files: Vec<PathBuf>,
    transfer: &mut Transfer,
    events: &mpsc::Sender<BackendEvent>,
    cancel: CancellationToken,
    bandwidth: linuxdrop_network::BandwidthLimiter,
) -> Result<()> {
    if files.is_empty() || files.len() > 1000 {
        bail!("Choose between 1 and 1000 files");
    }
    let mut metadata = vec![];
    let mut names = std::collections::HashSet::new();
    for path in &files {
        let stat = tokio::fs::metadata(path).await?;
        if !stat.is_file() {
            bail!("Only regular files can be shared");
        }
        let name = path
            .file_name()
            .context("Missing filename")?
            .to_str()
            .context("AirDrop needs UTF-8 filenames")?;
        linuxdrop_storage::validate_name(name)?;
        if !names.insert(name.to_owned()) {
            bail!("Files in one AirDrop transfer must have distinct names");
        }
        transfer.total_bytes = transfer
            .total_bytes
            .checked_add(stat.len())
            .context("Size overflow")?;
        transfer.files.push(TransferFile {
            name: name.into(),
            size: stat.len(),
            transferred: 0,
        });
        let mut file = plist::Dictionary::new();
        file.insert("FileName".into(), plist::Value::String(name.into()));
        file.insert(
            "FileType".into(),
            plist::Value::String("public.data".into()),
        );
        file.insert("FileSize".into(), plist::Value::Integer(stat.len().into()));
        file.insert(
            "FileBomPath".into(),
            plist::Value::String(format!("./{name}")),
        );
        metadata.push(plist::Value::Dictionary(file));
    }
    transfer.state = "preparing".into();
    events
        .send(BackendEvent::TransferUpdated(transfer.clone()))
        .await
        .ok();
    let archive_cancel = cancel.clone();
    let (archive, size) =
        tokio::task::spawn_blocking(move || super::archive::encode(&files, Some(&archive_cancel)))
            .await??;
    let config = Arc::new(
        rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(SessionVerifier {
                pin: Mutex::new(None),
            }))
            .with_no_client_auth(),
    );
    let sender =
        luftlift_rs::client::SenderConfig::new(name, "Linux", "io.github.marius4lui.LinuxDrop");
    let discover = luftlift_rs::client::build_discover_request(&sender);
    let discovered = tokio::time::timeout(
        Duration::from_secs(20),
        post(
            address,
            config.clone(),
            "/Discover",
            "application/x-apple-binary-plist",
            full(discover.body),
            None,
        ),
    )
    .await??;
    let response = luftlift_rs::http::HttpResponse {
        status: 200,
        headers: Default::default(),
        body: discovered,
    };
    transfer.peer_name = luftlift_rs::client::parse_discover_response(&response)?.computer_name;
    let mut ask = plist::Dictionary::new();
    ask.insert(
        "TransferID".into(),
        plist::Value::String(transfer.id.clone()),
    );
    ask.insert("TransferType".into(), plist::Value::String("files".into()));
    ask.insert(
        "SenderComputerName".into(),
        plist::Value::String(name.into()),
    );
    ask.insert(
        "SenderModelName".into(),
        plist::Value::String("Linux".into()),
    );
    ask.insert(
        "SenderID".into(),
        plist::Value::Data(uuid::Uuid::new_v4().as_bytes()[..6].to_vec()),
    );
    ask.insert(
        "BundleID".into(),
        plist::Value::String("io.github.marius4lui.LinuxDrop".into()),
    );
    ask.insert("Files".into(), plist::Value::Array(metadata));
    ask.insert("Items".into(), plist::Value::Array(vec![]));
    let mut bytes = vec![];
    plist::Value::Dictionary(ask).to_writer_binary(&mut bytes)?;
    transfer.state = "waiting".into();
    events
        .send(BackendEvent::TransferUpdated(transfer.clone()))
        .await
        .ok();
    tokio::time::timeout(
        Duration::from_secs(130),
        post(
            address,
            config.clone(),
            "/Ask",
            "application/x-apple-binary-plist",
            full(bytes),
            None,
        ),
    )
    .await??;
    transfer.state = "transferring".into();
    events
        .send(BackendEvent::TransferUpdated(transfer.clone()))
        .await
        .ok();
    let mut snapshot = transfer.clone();
    let progress = events.clone();
    let mut transferred = 0u64;
    let mut last = Instant::now();
    let stream = ReaderStream::with_capacity(tokio::fs::File::from_std(archive), 128 * 1024)
        .then(move |chunk| {
            let bandwidth = bandwidth.clone();
            let cancel = cancel.clone();
            async move {
                let bytes = chunk?;
                bandwidth
                    .acquire(bytes.len(), &cancel)
                    .await
                    .map_err(std::io::Error::other)?;
                Ok::<_, std::io::Error>(bytes)
            }
        })
        .map(move |chunk| {
            if let Ok(bytes) = &chunk {
                transferred += bytes.len() as u64;
                if last.elapsed() > Duration::from_millis(200) {
                    snapshot.transferred_bytes =
                        ((transferred as u128 * snapshot.total_bytes as u128)
                            / (size.max(1) as u128)) as u64;
                    let _ = progress.try_send(BackendEvent::TransferUpdated(snapshot.clone()));
                    last = Instant::now();
                }
            }
            chunk.map(Frame::data)
        });
    let body = StreamBody::new(stream).boxed_unsync();
    tokio::time::timeout(
        Duration::from_secs(1800),
        post(
            address,
            config,
            "/Upload",
            "application/x-dvzip",
            body,
            Some(size),
        ),
    )
    .await??;
    Ok(())
}

pub async fn ble_wake(name: Option<&str>) -> Result<bluer::adv::AdvertisementHandle> {
    let adapter = linuxdrop_network::bluetooth_adapter(name).await?;
    if !adapter.is_powered().await? {
        bail!("Bluetooth is switched off");
    }
    let advertisement = bluer::adv::Advertisement {
        advertisement_type: bluer::adv::Type::Broadcast,
        manufacturer_data: std::collections::BTreeMap::from([(
            0x004c,
            vec![
                0x05, 0x12, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            ],
        )]),
        ..Default::default()
    };
    linuxdrop_network::advertise(&adapter, advertisement).await
}

#[cfg(test)]
pub(super) async fn unsolicited_upload(address: SocketAddr) -> Result<Vec<u8>> {
    let config = Arc::new(
        rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(SessionVerifier {
                pin: Mutex::new(None),
            }))
            .with_no_client_auth(),
    );
    post(
        address,
        config,
        "/Upload",
        "application/x-dvzip",
        full(vec![0, 0, 0, 0]),
        Some(4),
    )
    .await
}

#[cfg(test)]
mod scope_tests {
    #[tokio::test]
    async fn unscoped_remote_airdrop_addresses_cannot_use_the_default_route() {
        for address in ["192.0.2.1:8771", "[2001:db8::1]:8771", "[fe80::1]:8771"] {
            assert!(
                super::connect_on_awdl(address.parse().unwrap())
                    .await
                    .is_err()
            );
        }
    }
}
