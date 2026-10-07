//! One orderly exit path for desktop idle exit, SIGINT and service-manager SIGTERM.
use super::*;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

pub(super) async fn begin_stop(shared: &Arc<Shared>, stop: &CancellationToken) {
    {
        let mut data = shared.data.lock().await;
        data.stop_when_idle = true;
        data.settings["visibility"]["mode"] = json!("hidden");
        data.visibility_since = None;
        data.drafts.clear();
        if let Some(offer) = shared.download_offer.lock().await.as_ref() {
            offer.quiesce();
        }
    }
    stop.cancel();
    shared.changed().await;
}

pub(super) async fn finish(
    shared: &Arc<Shared>,
    supervisor: JoinHandle<()>,
    workers: Vec<JoinHandle<()>>,
) -> Result<()> {
    // Do not cancel an in-flight constructor or a helper request: either may own
    // a resource whose reply still needs draining. Late actors are retired by
    // install_backend after begin_stop has closed admission.
    let mut failures = Vec::new();
    if let Err(error) = supervisor.await {
        failures.push(format!("Backend supervisor: {error}"));
    }
    // A health poll may be waiting behind a backend P2P request. Stop actors
    // concurrently with watchers so that cancellation can unblock that request.
    let (backend_errors, link_error, workers) = tokio::join!(
        stop_backends(shared),
        stop_link(shared),
        futures_util::future::join_all(workers),
    );
    for result in workers {
        if let Err(error) = result {
            failures.push(format!("Lifecycle worker: {error}"));
        }
    }
    failures.extend(backend_errors);
    if let Err(error) = link_error {
        failures.push(format!("Download link: {error}"));
    }
    // Radio leases outlive the actors using their interfaces. Explicit release
    // confirms restoration; simply closing the socket only schedules recovery.
    let (airdrop, quickshare) = tokio::join!(
        helper::release(&shared.helper),
        helper::release(&shared.quickshare_helper),
    );
    for (name, result) in [("AirDrop", airdrop), ("Quick Share", quickshare)] {
        if let Err(error) = result {
            failures.push(format!("{name} radio restoration: {error}"));
        }
    }
    shared.event_forwarders.close();
    // The main loop continues consuming the output during this wait. Once all
    // forwarders finish, draining that queue cannot miss a completed transfer.
    if failures.is_empty() {
        shared.event_forwarders.wait().await;
        Ok(())
    } else {
        anyhow::bail!("LinuxDrop cleanup incomplete: {}", failures.join("; "))
    }
}

async fn stop_backends(shared: &Arc<Shared>) -> Vec<String> {
    let commands = {
        let mut data = shared.data.lock().await;
        let active = std::mem::take(&mut data.commands);
        data.retiring.extend(active);
        data.retiring.clone()
    };
    let mut tasks = tokio::task::JoinSet::new();
    for (id, commands) in commands {
        tasks.spawn(async move { (id, commands.shutdown().await) });
    }
    let mut errors = Vec::new();
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok((id, Ok(()))) => {
                shared.data.lock().await.retiring.remove(&id);
            }
            Ok((id, Err(error))) => errors.push(format!("{id}: {error}")),
            Err(error) => errors.push(format!("Backend cleanup worker: {error}")),
        }
    }
    errors
}

async fn stop_link(shared: &Arc<Shared>) -> std::result::Result<(), String> {
    let mut offer = shared.download_offer.lock().await;
    if let Some(current) = offer.as_ref() {
        current.shutdown().await?;
    }
    offer.take();
    Ok(())
}

pub(super) async fn finish_history(shared: &Arc<Shared>) -> Result<()> {
    let mut data = shared.data.lock().await;
    let mut interrupted = Vec::new();
    for transfer in data.transfers.values_mut().filter(|t| !t.is_terminal()) {
        transfer.state = "failed".into();
        transfer.error = Some("LinuxDrop stopped before this transfer finished.".into());
        interrupted.push(transfer.id.clone());
    }
    for id in interrupted {
        data.completed_at.insert(id, history::now());
    }
    data.decisions.clear();
    drop(data);
    shared.persist_history().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::helper::tests::Fixture;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn exit_waits_for_late_startup_cleanup_and_all_queued_events() {
        let fixture = Fixture::new();
        let shared = &fixture.0;
        let stop = CancellationToken::new();
        let (start, started) = oneshot::channel();
        let (cleanup, cleaned) = oneshot::channel();
        let (entered, entering) = oneshot::channel();
        let (commands, mut receiver) = mpsc::channel(4);
        let (events, mut output) = mpsc::channel(1);
        let (backend_events, mut forward) = mpsc::channel(1);
        shared.event_forwarders.spawn(async move {
            while let Some(event) = forward.recv().await {
                events.send(event).await.unwrap();
            }
        });
        let backend = tokio::spawn(async move {
            assert!(matches!(
                receiver.recv().await,
                Some(BackendCommand::Shutdown)
            ));
            entered.send(()).unwrap();
            cleaned.await.unwrap();
            for index in 0..8 {
                backend_events.send(index).await.unwrap();
            }
            Ok(())
        });
        let startup = {
            let shared = shared.clone();
            tokio::spawn(async move {
                started.await.unwrap();
                install_backend(
                    &shared,
                    "localsend",
                    Ok(CommandSender::track(commands, backend)),
                )
                .await;
            })
        };
        begin_stop(shared, &stop).await;
        assert!(stop.is_cancelled());
        assert!(shared.data.lock().await.accepting_transfers().is_err());
        assert!(Manager(shared.clone())
            .set_visibility("everyone".into())
            .await
            .is_err());
        let finishing = {
            let shared = shared.clone();
            tokio::spawn(async move { finish(&shared, startup, vec![]).await })
        };
        assert!(!finishing.is_finished());
        start.send(()).unwrap();
        entering.await.unwrap();
        assert!(!finishing.is_finished());
        assert!(shared.data.lock().await.commands.is_empty());
        assert!(shared.data.lock().await.retiring.contains_key("localsend"));
        cleanup.send(()).unwrap();
        // This deliberately exceeds both bounded queues. The real main loop must
        // keep consuming terminal updates while awaiting shutdown as well.
        let mut received = Vec::new();
        while let Some(event) = output.recv().await {
            received.push(event);
        }
        finishing.await.unwrap().unwrap();
        assert_eq!(received, (0..8).collect::<Vec<_>>());
        assert!(shared.data.lock().await.retiring.is_empty());
    }

    #[tokio::test]
    async fn exit_reports_backend_failure_and_still_drains_other_services() {
        let fixture = Fixture::new();
        let shared = &fixture.0;
        let mut stopped = Vec::new();
        for (id, fails) in [("localsend", true), ("airdrop", false)] {
            let (commands, mut receiver) = mpsc::channel(1);
            let (done, observed) = oneshot::channel();
            let actor = tokio::spawn(async move {
                assert!(matches!(
                    receiver.recv().await,
                    Some(BackendCommand::Shutdown)
                ));
                done.send(()).unwrap();
                if fails {
                    Err("restoration failed".into())
                } else {
                    Ok(())
                }
            });
            shared
                .data
                .lock()
                .await
                .commands
                .insert(id.into(), CommandSender::track(commands, actor));
            stopped.push(observed);
        }
        let error = finish(shared, tokio::spawn(async {}), vec![])
            .await
            .unwrap_err();
        assert!(error.to_string().contains("localsend: restoration failed"));
        for observed in stopped {
            observed.await.unwrap();
        }
        let data = shared.data.lock().await;
        assert!(data.retiring.contains_key("localsend"));
        assert!(!data.retiring.contains_key("airdrop"));
    }

    #[tokio::test]
    async fn helper_release_waits_for_restore_reply_and_propagates_failure() {
        for fails in [false, true] {
            let fixture = Fixture::new();
            let shared = &fixture.0;
            let (client, server) = tokio::net::UnixStream::pair().unwrap();
            let expected = linuxdrop_netd::Lease {
                id: "release-this-lease".into(),
                uid: 1000,
                phy: "phy1".into(),
                interface: "wlan1".into(),
                channel: 6,
                boot_id: "test-boot".into(),
                awdl_interface: Some("awdl0".into()),
                allowed_frequencies: vec![],
                kind: linuxdrop_netd::LeaseKind::Monitor,
                connection_uuid: None,
                p2p_group: None,
                direct_capabilities: Default::default(),
            };
            *shared.helper.lock().await = Some(helper::HelperLease::new(
                linuxdrop_netd::Client::from_stream(client),
                expected,
                shared,
            ));
            let (entered, entering) = oneshot::channel();
            let (restore, restored) = oneshot::channel();
            let helper = tokio::spawn(async move {
                let (read, mut write) = server.into_split();
                let mut read = BufReader::new(read);
                let mut line = String::new();
                read.read_line(&mut line).await.unwrap();
                assert!(
                    matches!(serde_json::from_str::<linuxdrop_netd::Request>(&line).unwrap(),
                    linuxdrop_netd::Request::Release { lease_id } if lease_id == "release-this-lease")
                );
                entered.send(()).unwrap();
                restored.await.unwrap();
                let response = if fails {
                    linuxdrop_netd::Response::Error {
                        message: "radio still in recovery".into(),
                    }
                } else {
                    linuxdrop_netd::Response::Ok
                };
                let mut bytes = serde_json::to_vec(&response).unwrap();
                bytes.push(b'\n');
                write.write_all(&bytes).await.unwrap();
            });
            let finishing = {
                let shared = shared.clone();
                tokio::spawn(async move { finish(&shared, tokio::spawn(async {}), vec![]).await })
            };
            entering.await.unwrap();
            assert!(!finishing.is_finished());
            restore.send(()).unwrap();
            let result = finishing.await.unwrap();
            if fails {
                assert!(result
                    .unwrap_err()
                    .to_string()
                    .contains("radio still in recovery"));
            } else {
                result.unwrap();
            }
            helper.await.unwrap();
        }
    }
}
