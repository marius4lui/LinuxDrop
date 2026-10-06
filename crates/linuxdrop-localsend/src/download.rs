//! Explicit LocalSend download offers. Metadata is reviewed before fetching files.
use super::*;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Offer {
    info: DeviceInfo,
    session_id: String,
    files: HashMap<String, WireFile>,
}

/// Limit this local sharing operation to a literal on-link address. No DNS,
/// redirects, URL credentials, proxy, path, query or fragment authority is used.
pub fn offer_address(url: &str, policy: &TransferPolicy) -> Result<(SocketAddr, String)> {
    if url.len() > 2048 {
        bail!("Download link is too long");
    }
    let parsed =
        reqwest::Url::parse(url).context("Enter a full LocalSend link starting with http://")?;
    if parsed.scheme() != "http"
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        bail!("Use the LocalSend HTTP address without a path, credentials, PIN or tracking parameters");
    }
    let ip: IpAddr = parsed
        .host_str()
        .context("Missing local address")?
        .trim_matches(['[', ']'])
        .parse()
        .context("Use the sender's local IP address, not a website name")?;
    let port = parsed.port_or_known_default().context("Missing port")?;
    if port == 0 {
        bail!("Invalid download port");
    }
    let interface = linuxdrop_network::interfaces(policy, true)?
        .into_iter()
        .find(|interface| discovery::permits(interface, ip))
        .context("Download link is outside the enabled local networks")?;
    Ok((SocketAddr::new(ip, port), interface.name))
}

pub fn pending_transfer(id: String, address: SocketAddr) -> Transfer {
    Transfer {
        id,
        peer_id: format!("localsend-download:{address}"),
        peer_name: address.to_string(),
        protocol: "localsend".into(),
        direction: "incoming".into(),
        state: "connecting".into(),
        files: vec![],
        total_bytes: 0,
        transferred_bytes: 0,
        error: None,
        verification_code: None,
        saved_paths: vec![],
    }
}

pub(super) async fn run(s: SharedState, id: String, url: String, cancel: CancellationToken) {
    let address = offer_address(&url, &s.config.policy);
    let mut transfer = pending_transfer(
        id.clone(),
        address
            .as_ref()
            .map(|value| value.0)
            .unwrap_or_else(|_| SocketAddr::from((Ipv4Addr::LOCALHOST, 0))),
    );
    let result = async {
        let (address, interface) = address?;
        receive(&s, &mut transfer, address, &interface, &cancel).await
    }
    .await;
    match result {
        Ok(true) => transfer.state = "completed".into(),
        Ok(false) => transfer.state = "rejected".into(),
        Err(error) => {
            transfer.state = if cancel.is_cancelled() {
                "cancelled"
            } else {
                "failed"
            }
            .into();
            transfer.error = Some(error.to_string());
        }
    }
    s.download_decisions.lock().await.remove(&id);
    s.pin_requests.lock().await.remove(&id);
    s.outgoing.lock().await.remove(&id);
    let _ = s.events.send(BackendEvent::TransferUpdated(transfer)).await;
}

async fn receive(
    s: &SharedState,
    transfer: &mut Transfer,
    address: SocketAddr,
    interface: &str,
    cancel: &CancellationToken,
) -> Result<bool> {
    let client = tls::client_on("http", "", Some(interface))?;
    let base = format!("http://{address}/api/localsend/v2");
    let mut pin = None;
    let mut attempts = 0;
    let mut response = loop {
        let mut request = client
            .post(format!("{base}/prepare-download"))
            .timeout(Duration::from_secs(120));
        if let Some(pin) = &pin {
            request = request.query(&[("pin", pin)]);
        }
        let response = tokio::select! {
            _ = cancel.cancelled() => bail!("Download cancelled"),
            response = request.send() => response.map_err(|error| anyhow::anyhow!("Could not open download offer: {}", error.without_url()))?,
        };
        if response.status() != StatusCode::UNAUTHORIZED {
            break response;
        }
        attempts += 1;
        if attempts > 3 {
            bail!("The sender did not accept the PIN");
        }
        let (reply, input) = oneshot::channel();
        s.pin_requests
            .lock()
            .await
            .insert(transfer.id.clone(), reply);
        transfer.state = "pin_required".into();
        transfer.error = if attempts > 1 {
            Some("Incorrect PIN; enter the sender's current PIN".into())
        } else {
            None
        };
        let _ = s
            .events
            .send(BackendEvent::TransferUpdated(transfer.clone()))
            .await;
        pin = Some(tokio::select! {
            _ = cancel.cancelled() => bail!("Download cancelled"),
            input = tokio::time::timeout(Duration::from_secs(120), input) => input.context("PIN entry expired")?.context("PIN entry closed")?,
        });
        s.pin_requests.lock().await.remove(&transfer.id);
    };
    if !response.status().is_success() {
        bail!(
            "Download offer unavailable (HTTP {})",
            response.status().as_u16()
        );
    }
    let mut body = Vec::new();
    loop {
        let chunk = tokio::select! {
            _ = cancel.cancelled() => bail!("Download cancelled"),
            chunk = response.chunk() => chunk.map_err(|error| anyhow::anyhow!("Could not read offer: {}", error.without_url()))?,
        };
        let Some(chunk) = chunk else {
            break;
        };
        if body.len().saturating_add(chunk.len()) > 1024 * 1024 {
            bail!("Download metadata exceeds limit");
        }
        body.extend_from_slice(&chunk);
    }
    let mut offer: Offer =
        serde_json::from_slice(&body).context("Invalid LocalSend download offer")?;
    if offer.session_id.is_empty() || offer.session_id.len() > 256 {
        bail!("Invalid download session");
    }
    // Download API info may omit its port/protocol. Use the explicit URL's
    // authority, never metadata supplied by the remote endpoint, for requests.
    offer.info.port = address.port();
    offer.info.protocol = "http".into();
    let metadata = Prepare {
        info: offer.info,
        files: offer.files,
    };
    transfer.total_bytes = validate_offer(&metadata, s.config.max_files, s.config.max_bytes)
        .map_err(|_| anyhow::anyhow!("Download offer has invalid names, sizes or checksums"))?;
    validate_info(&metadata.info)?;
    transfer.peer_name = metadata.info.alias;
    transfer.peer_id = format!("localsend:{}", metadata.info.fingerprint);
    let mut files: Vec<_> = metadata.files.into_iter().collect();
    files.sort_by(|left, right| left.0.cmp(&right.0));
    transfer.files = files
        .iter()
        .map(|(_, file)| TransferFile {
            name: file.file_name.clone(),
            size: file.size,
            transferred: 0,
        })
        .collect();
    transfer.state = "waiting".into();
    transfer.error = None;
    let (reply, decision) = oneshot::channel();
    s.download_decisions
        .lock()
        .await
        .insert(transfer.id.clone(), reply);
    let _ = s
        .events
        .send(BackendEvent::Incoming(transfer.clone()))
        .await;
    let options = tokio::select! {
        _ = cancel.cancelled() => bail!("Download cancelled"),
        decision = tokio::time::timeout(Duration::from_secs(120), decision) => decision.context("Download review expired")?.context("Download review closed")?,
    };
    let Some(options) = options else {
        return Ok(false);
    };
    let selection = options
        .selected_indices
        .unwrap_or_else(|| (0..files.len()).collect());
    let selected: std::collections::HashSet<_> = selection.iter().copied().collect();
    if selected.is_empty()
        || selected.len() != selection.len()
        || selected.iter().any(|index| *index >= files.len())
    {
        bail!("Invalid file selection");
    }
    let store = match options.directory {
        Some(directory) => ReceiveStore::open(directory)?,
        None => s.store.clone(),
    };
    transfer.total_bytes = files
        .iter()
        .enumerate()
        .filter(|(index, _)| selected.contains(index))
        .map(|(_, (_, file))| file.size)
        .sum();
    store.ensure_space(transfer.total_bytes)?;
    transfer.files = transfer
        .files
        .iter()
        .enumerate()
        .filter(|(index, _)| selected.contains(index))
        .map(|(_, file)| file.clone())
        .collect();
    transfer.state = "transferring".into();
    let _ = s
        .events
        .send(BackendEvent::TransferUpdated(transfer.clone()))
        .await;
    let mut published_index = 0;
    for (index, (file_id, file)) in files.into_iter().enumerate() {
        if !selected.contains(&index) {
            continue;
        }
        if cancel.is_cancelled() {
            bail!("Download cancelled");
        }
        let request = client
            .get(format!("{base}/download"))
            .query(&[("sessionId", &offer.session_id), ("fileId", &file_id)]);
        let response = tokio::select! {
            _ = cancel.cancelled() => bail!("Download cancelled"),
            response = tokio::time::timeout(Duration::from_secs(30), request.send()) => response.context("Download response timed out")?.map_err(|error| anyhow::anyhow!("Download failed: {}", error.without_url()))?,
        };
        if !response.status().is_success() {
            bail!(
                "File download refused (HTTP {})",
                response.status().as_u16()
            );
        }
        if response
            .content_length()
            .is_some_and(|size| size != file.size)
        {
            bail!("Downloaded file length differs from offer");
        }
        let mut pending = store.create(&file.file_name)?;
        let mut stream = response.bytes_stream();
        let mut received = 0u64;
        let mut digest = Sha256::new();
        let mut last_update = Instant::now();
        loop {
            let chunk = tokio::select! {
                _ = cancel.cancelled() => bail!("Download cancelled"),
                chunk = tokio::time::timeout(Duration::from_secs(30), stream.next()) => chunk.context("Download stalled")?,
            };
            let Some(chunk) = chunk else {
                break;
            };
            let chunk = chunk.map_err(|error| {
                anyhow::anyhow!("Download interrupted: {}", error.without_url())
            })?;
            received = received
                .checked_add(chunk.len() as u64)
                .context("Download size overflow")?;
            if received > file.size {
                bail!("Downloaded file exceeds offer");
            }
            s.bandwidth.acquire(chunk.len(), cancel).await?;
            pending.file.write_all(&chunk).await?;
            digest.update(&chunk);
            transfer.files[published_index].transferred = received;
            transfer.transferred_bytes = transfer.files.iter().map(|file| file.transferred).sum();
            if last_update.elapsed() >= Duration::from_millis(100) || received == file.size {
                let _ = s
                    .events
                    .send(BackendEvent::TransferUpdated(transfer.clone()))
                    .await;
                last_update = Instant::now();
            }
        }
        if received != file.size {
            bail!("Downloaded file was truncated");
        }
        if file
            .sha256
            .as_ref()
            .is_some_and(|expected| expected.to_ascii_lowercase() != hex::encode(digest.finalize()))
        {
            bail!("Downloaded file checksum mismatch");
        }
        if cancel.is_cancelled() {
            bail!("Download cancelled");
        }
        if let Some(metadata) = &file.metadata {
            metadata.apply(&mut pending).await?;
        }
        transfer.saved_paths.push(
            pending
                .commit_with_policy(options.collision_policy)
                .await?
                .to_string_lossy()
                .into_owned(),
        );
        published_index += 1;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{config, next_transfer};

    #[tokio::test]
    async fn pin_protected_offer_requires_review_and_downloads_only_selected_files() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        for (name, data) in [
            ("keep.txt", b"exact bytes".as_slice()),
            ("empty.txt", b""),
            ("skip.txt", b"unselected"),
        ] {
            std::fs::write(source.path().join(name), data).unwrap();
            crate::metadata::tests::set_test_times(&source.path().join(name));
        }
        let offer = crate::reverse::start_offer(
            crate::reverse::OfferConfig {
                alias: "Explicit sender".into(),
                bind: "127.0.0.1:0".parse().unwrap(),
                expires_after: Duration::from_secs(60),
                max_files: 3,
                max_bytes: 100,
            },
            ["keep.txt", "empty.txt", "skip.txt"]
                .iter()
                .map(|name| source.path().join(name))
                .collect(),
        )
        .await
        .unwrap();
        let mut settings = config(target.path(), "Receiver");
        settings.visible = false; // Explicit download does not enable unsolicited reception.
        let (events, mut received) = mpsc::channel(128);
        let commands = start(settings.clone(), events).await.unwrap();
        commands
            .send(BackendCommand::ReceiveOffer {
                transfer_id: "download".into(),
                url: format!("http://{}", offer.address),
            })
            .await
            .unwrap();
        next_transfer(&mut received, "pin_required").await;
        let wrong = if offer.pin == "000000" {
            "111111"
        } else {
            "000000"
        };
        commands
            .send(BackendCommand::ProvidePin {
                transfer_id: "download".into(),
                pin: wrong.into(),
            })
            .await
            .unwrap();
        let retry = next_transfer(&mut received, "pin_required").await;
        assert!(retry.error.unwrap().contains("Incorrect PIN"));
        commands
            .send(BackendCommand::ProvidePin {
                transfer_id: "download".into(),
                pin: offer.pin.clone(),
            })
            .await
            .unwrap();
        let request = next_transfer(&mut received, "waiting").await;
        assert_eq!(request.direction, "incoming");
        assert_eq!(request.peer_name, "Explicit sender");
        assert_eq!(
            std::fs::read_dir(&settings.download_dir).unwrap().count(),
            0
        );
        let destination = target.path().join("chosen");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("keep.txt"), b"original").unwrap();
        commands
            .send(BackendCommand::AcceptWithOptions {
                transfer_id: "download".into(),
                options: ReceiveOptions {
                    directory: Some(destination.clone()),
                    selected_indices: Some(
                        request
                            .files
                            .iter()
                            .enumerate()
                            .filter_map(|(index, file)| (file.name != "skip.txt").then_some(index))
                            .collect(),
                    ),
                    collision_policy: CollisionPolicy::Rename,
                },
            })
            .await
            .unwrap();
        let completed = next_transfer(&mut received, "completed").await;
        assert_eq!(completed.saved_paths.len(), 2);
        assert_eq!(completed.transferred_bytes, 11);
        assert_eq!(completed.total_bytes, 11);
        assert_eq!(
            std::fs::read(destination.join("keep.txt")).unwrap(),
            b"original"
        );
        assert!(!destination.join("skip.txt").exists());
        for path in completed.saved_paths {
            crate::metadata::tests::assert_test_times(std::path::Path::new(&path));
            let contents = std::fs::read(path).unwrap();
            assert!(contents.is_empty() || contents == b"exact bytes");
        }
        commands.send(BackendCommand::Shutdown).await.unwrap();
    }

    #[tokio::test]
    async fn hostile_downloads_and_cancellation_never_publish_partial_files() {
        for scenario in ["traversal", "checksum", "truncated", "oversized", "cancel"] {
            let target = tempfile::tempdir().unwrap();
            let mut settings = config(target.path(), "Receiver");
            if scenario == "cancel" {
                settings.policy.bandwidth_bytes_per_second = Some(1);
            }
            let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let counter = hits.clone();
            let metadata = serde_json::json!({
                "sessionId": "session", "info": { "alias": "Fixture", "fingerprint": "fixture" },
                "files": { "one": { "id": "one", "fileName": if scenario == "traversal" { "../escape" } else { "safe.bin" },
                    "size": 5, "fileType": "application/octet-stream", "sha256": if scenario == "checksum" { Some("00".repeat(32)) } else { None } } }
            });
            let router = Router::new()
                .route(
                    "/api/localsend/v2/prepare-download",
                    post(move || {
                        let metadata = metadata.clone();
                        async move { Json(metadata) }
                    }),
                )
                .route(
                    "/api/localsend/v2/download",
                    get(move || {
                        counter.fetch_add(1, Ordering::Relaxed);
                        async move {
                            match scenario {
                                "truncated" => "tiny",
                                "oversized" => "too long",
                                _ => "bytes",
                            }
                        }
                    }),
                );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            });
            let (events, mut received) = mpsc::channel(128);
            let commands = start(settings.clone(), events).await.unwrap();
            commands
                .send(BackendCommand::ReceiveOffer {
                    transfer_id: "download".into(),
                    url: format!("http://{address}"),
                })
                .await
                .unwrap();
            if scenario != "traversal" {
                next_transfer(&mut received, "waiting").await;
                assert_eq!(
                    hits.load(Ordering::Relaxed),
                    0,
                    "File traffic before consent"
                );
                commands
                    .send(BackendCommand::Accept {
                        transfer_id: "download".into(),
                    })
                    .await
                    .unwrap();
                if scenario == "cancel" {
                    next_transfer(&mut received, "transferring").await;
                    commands
                        .send(BackendCommand::Cancel {
                            transfer_id: "download".into(),
                        })
                        .await
                        .unwrap();
                }
            }
            let ended = next_transfer(
                &mut received,
                if scenario == "cancel" {
                    "cancelled"
                } else {
                    "failed"
                },
            )
            .await;
            assert!(ended.saved_paths.is_empty());
            assert_eq!(
                std::fs::read_dir(&settings.download_dir).unwrap().count(),
                0,
                "{scenario}"
            );
            if scenario == "traversal" {
                assert_eq!(hits.load(Ordering::Relaxed), 0);
            }
            commands.send(BackendCommand::Shutdown).await.unwrap();
            server.abort();
        }
    }

    #[test]
    fn links_cannot_redirect_authority_or_escape_the_selected_lan() {
        let policy = TransferPolicy {
            allowed_interfaces: vec!["lo".into()],
            ..Default::default()
        };
        assert!(offer_address("http://127.0.0.1:53317", &policy).is_ok());
        for url in [
            "https://127.0.0.1:53317",
            "http://example.com",
            "http://192.0.2.1:53317",
            "http://127.0.0.1/file",
            "http://user:password@127.0.0.1",
            "http://127.0.0.1?pin=1234",
            "http://127.0.0.1/#secret",
            "http://127.0.0.1:0",
        ] {
            assert!(offer_address(url, &policy).is_err(), "{url}");
        }
    }
}
