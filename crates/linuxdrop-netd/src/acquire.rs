//! Monitor/AWDL preparation is a persistent radio producer. No network process
//! or readiness wait owns the shared State lock; retirement waits for settlement.
use super::*;
use tokio_util::sync::CancellationToken;

pub(super) async fn acquire_radio(
    shared: &Shared,
    uid: u32,
    owned: &mut Vec<String>,
    radio_id: String,
    channel: u16,
    awdl: bool,
) -> Result<Response, String> {
    if owned.len() >= 2 {
        return Err("two radio leases per client maximum".into());
    }
    let inventory = inventory().await;
    let mut state = shared.lock().await;
    let radio = inventory
        .radios
        .iter()
        .find(|r| r.id == radio_id)
        .ok_or("radio not found")?;
    if !radio.phy.starts_with("phy") || !radio.phy[3..].chars().all(|c| c.is_ascii_digit()) {
        return Err("invalid kernel radio name".into());
    }
    let leased = inventory
        .radios
        .iter()
        .filter(|r| state.leases.values().any(|l| l.phy == r.phy))
        .map(|r| r.id.clone())
        .collect();
    let decision = select_radio(
        &inventory,
        &SelectionRequest {
            preferred: Some(radio_id.clone()),
            leased,
            require_tested_awdl: false,
            channel: Some(channel),
            prefer_usb: true,
        },
    );
    let candidate = decision
        .candidates
        .iter()
        .find(|c| c.id == radio_id)
        .ok_or("radio disappeared")?;
    if !candidate.exclusions.is_empty() {
        return Err(candidate.exclusions.join("; "));
    }
    let id = uuid::Uuid::new_v4().simple().to_string();
    let lease = Lease {
        id: id.clone(),
        uid,
        phy: radio.phy.clone(),
        interface: format!("ld{}", &id[..10]),
        channel,
        boot_id: std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .map_err(|e| e.to_string())?
            .trim()
            .into(),
        awdl_interface: awdl.then(|| format!("la{}", &id[..10])),
        kind: LeaseKind::Monitor,
        connection_uuid: None,
        p2p_group: None,
        p2p_pending: false,
        p2p_recovery: None,
        direct_capabilities: Default::default(),
        allowed_frequencies: radio
            .channels
            .iter()
            .filter(|c| !c.disabled && !c.no_ir && !c.radar)
            .map(|c| c.frequency_mhz)
            .collect(),
    };
    state.leases.insert(id.clone(), lease.clone());
    state.radio_changed();
    if let Err(e) = persist(&state) {
        state.leases.remove(&id);
        return Err(e.to_string());
    }

    // Record socket ownership before the first await after durable reservation.
    // EOF, task abortion and shutdown can now find and retire this producer.
    owned.push(id.clone());
    let cancel = CancellationToken::new();
    let stop_on_drop = cancel.clone().drop_guard();
    let response = spawn_acquisition_with(&mut state, shared, lease, cancel, prepare_monitor);
    drop(state);
    let result = response
        .await
        .map_err(|_| "Radio supervisor stopped without a reply".to_owned())?;
    stop_on_drop.disarm();
    if let Err(error) = result {
        let cleanup = cleanup_lease(shared, &id).await;
        if cleanup.is_ok() {
            owned.retain(|owned| owned != &id);
        }
        return Err(match cleanup {
            Ok(()) => error,
            Err(cleanup) => format!("{error}; radio recovery required: {cleanup}"),
        });
    }
    result
}

// The same path is exercised with a delayed producer in regression tests. A
// late-created child is retained by State even if its caller has disappeared.
pub(super) fn spawn_acquisition_with<F, Fut>(
    state: &mut State,
    shared: &Shared,
    lease: Lease,
    cancel: CancellationToken,
    prepare: F,
) -> tokio::sync::oneshot::Receiver<Result<Response, String>>
where
    F: FnOnce(Lease, CancellationToken) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<Option<tokio::process::Child>, String>>
        + Send
        + 'static,
{
    let worker_shared = shared.clone();
    let worker_cancel = cancel.clone();
    spawn_radio_with(
        state,
        shared,
        lease.id.clone(),
        cancel,
        Default::default(),
        move || async move {
            let child = prepare(lease.clone(), worker_cancel.clone()).await?;
            let mut state = worker_shared.lock().await;
            state.radio_changed();
            if let Some(child) = child {
                state.children.insert(lease.id.clone(), child);
            }
            if worker_cancel.is_cancelled() || state.cleanups.contains_key(&lease.id) {
                return Err("Radio preparation cancelled".into());
            }
            state.attached.insert(lease.id.clone());
            Ok(Response::Acquired {
                lease: Box::new(lease),
            })
        },
    )
}

fn check_cancel(cancel: &CancellationToken) -> Result<(), String> {
    if cancel.is_cancelled() {
        Err("Radio preparation cancelled".into())
    } else {
        Ok(())
    }
}

async fn prepare_monitor(
    lease: Lease,
    cancel: CancellationToken,
) -> Result<Option<tokio::process::Child>, String> {
    check_cancel(&cancel)?;
    // Original interfaces and NetworkManager properties remain untouched.
    run_command(
        "/usr/sbin/iw",
        &[
            "phy",
            &lease.phy,
            "interface",
            "add",
            &lease.interface,
            "type",
            "monitor",
        ],
        5,
    )
    .await?;
    // Mark this newly created interface even if cancellation arrived during iw;
    // cleanup then has a verifiable identity before it can retire the lease.
    run_command(
        "/usr/sbin/ip",
        &[
            "link",
            "set",
            "dev",
            &lease.interface,
            "alias",
            &format!("linuxdrop:{}", lease.id),
        ],
        5,
    )
    .await?;
    check_cancel(&cancel)?;
    let current = linuxdrop_hardware::inventory().await;
    if current.interfaces.iter().any(|interface| {
        interface.phy.as_deref() == Some(&lease.phy)
            && interface.name != lease.interface
            && interface.in_use()
    }) {
        return Err("radio became active while preparing its lease".into());
    }
    check_cancel(&cancel)?;
    run_command(
        "/usr/sbin/iw",
        &[
            "dev",
            &lease.interface,
            "set",
            "channel",
            &lease.channel.to_string(),
        ],
        5,
    )
    .await?;
    check_cancel(&cancel)?;
    run_command(
        "/usr/sbin/ip",
        &["link", "set", "dev", &lease.interface, "up"],
        5,
    )
    .await?;
    check_cancel(&cancel)?;
    let Some(tap) = &lease.awdl_interface else {
        return Ok(None);
    };
    if Path::new("/sys/class/net")
        .join(tap)
        .try_exists()
        .map_err(|error| error.to_string())?
    {
        return Err("AWDL interface name is already in use".into());
    }
    let allowed = lease
        .allowed_frequencies
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let mut child = Command::new(format!("{HELPER_DIRECTORY}/filin"))
        .args([
            "-i",
            &lease.interface,
            "-h",
            tap,
            "-c",
            &lease.channel.to_string(),
            "-N",
            "--no-http",
            "--no-force-master",
        ])
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin")
        .env("LC_ALL", "C")
        .env("LINUXDROP_ALLOWED_FREQUENCIES", allowed)
        .env("LINUXDROP_MANAGED_LEASE", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("AWDL helper unavailable: {error}"))?;
    let ready = wait_awdl_ready(&mut child, tap, &cancel)
        .await
        .and_then(|()| verify_owned(&lease));
    if let Err(error) = ready {
        reap_child(Some(child)).await?;
        return Err(error);
    }
    Ok(Some(child))
}

async fn wait_awdl_ready(
    child: &mut tokio::process::Child,
    tap: &str,
    cancel: &CancellationToken,
) -> Result<(), String> {
    const READY: &[u8] = b"LINUXDROP_AWDL_READY_V1\n";
    let mut stdout = child
        .stdout
        .take()
        .ok_or("AWDL readiness pipe unavailable")?;
    let mut receipt = [0_u8; READY.len()];
    tokio::select! {
        biased;
        _ = cancel.cancelled() => return Err("Radio preparation cancelled".into()),
        result = timeout(Duration::from_secs(3), stdout.read_exact(&mut receipt)) => {
            result.map_err(|_| "AWDL helper readiness timed out")?
                .map_err(|_| "AWDL helper stopped before confirming ready")?;
        }
    }
    check_cancel(cancel)?;
    if receipt != READY
        || child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_some()
    {
        return Err("AWDL helper did not confirm an active link".into());
    }
    if !Path::new("/sys/class/net")
        .join(tap)
        .try_exists()
        .map_err(|error| error.to_string())?
    {
        return Err("AWDL interface disappeared before readiness".into());
    }
    Ok(())
}

// Passive inventory can involve D-Bus/netlink timeouts; only the final choice
// and durable reservation serialize with other helper state changes.
pub(super) async fn reserve_radio(
    shared: &Shared,
    uid: u32,
    owned: &mut Vec<String>,
    radio_id: String,
) -> Result<Response, String> {
    if owned.len() >= 2 {
        return Err("two radio leases per client maximum".into());
    }
    let inv = inventory().await;
    let mut state = shared.lock().await;
    state.radio_changed();
    let radio = inv
        .radios
        .iter()
        .find(|r| r.id == radio_id)
        .ok_or("radio not found")?;
    if radio.protected || radio.rfkill || radio.driver.is_none() {
        return Err("radio is active, blocked, or has no driver".into());
    }
    if state.leases.values().any(|l| l.phy == radio.phy) {
        return Err("radio is already leased".into());
    }
    if !radio.modes.iter().any(|m| m == "managed") {
        return Err("managed station mode is unavailable".into());
    }
    let interface = inv
        .interfaces
        .iter()
        .find(|i| i.phy.as_deref() == Some(&radio.phy) && !i.in_use())
        .ok_or("no idle interface on selected radio")?;
    let id = uuid::Uuid::new_v4().simple().to_string();
    let lease = Lease {
        id: id.clone(),
        uid,
        phy: radio.phy.clone(),
        interface: interface.name.clone(),
        channel: 0,
        boot_id: std::fs::read_to_string("/proc/sys/kernel/random/boot_id")
            .map_err(|e| e.to_string())?
            .trim()
            .into(),
        awdl_interface: None,
        allowed_frequencies: vec![],
        kind: LeaseKind::DirectWifi,
        connection_uuid: Some(uuid::Uuid::new_v4().to_string()),
        p2p_group: None,
        p2p_pending: false,
        p2p_recovery: None,
        direct_capabilities: direct_capabilities(&radio.modes, &radio.channels),
    };
    state.leases.insert(id.clone(), lease.clone());
    if let Err(e) = persist(&state) {
        state.leases.remove(&id);
        return Err(e.to_string());
    }
    state.attached.insert(id.clone());
    owned.push(id);
    Ok(Response::Acquired {
        lease: Box::new(lease),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn readiness_requires_receipt_and_observes_cancellation_and_child_exit() {
        for (script, success, cancel_early) in [
            (
                "printf 'LINUXDROP_AWDL_READY_V1\\n'; exec sleep 30",
                true,
                false,
            ),
            (
                "printf 'invalid readiness receipt\\n'; exec sleep 30",
                false,
                false,
            ),
            ("exit 1", false, false),
            ("exec sleep 30", false, true),
        ] {
            let mut child = Command::new("/bin/sh")
                .args(["-c", script])
                .stdout(std::process::Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .unwrap();
            let cancel = CancellationToken::new();
            if cancel_early {
                let stop = cancel.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    stop.cancel();
                });
            }
            // lo exists in every test namespace. Its mere existence must not
            // substitute for the process's explicit configuration receipt.
            let result = wait_awdl_ready(&mut child, "lo", &cancel).await;
            assert_eq!(result.is_ok(), success, "{result:?}");
            reap_child(Some(child)).await.unwrap();
        }
    }

    #[tokio::test]
    #[ignore = "private network namespace and actual filin binary required"]
    async fn real_managed_filin_rejects_channel_failure_without_false_readiness() {
        assert_eq!(
            std::env::var("LINUXDROP_TEST_PRIVATE_AWDL").as_deref(),
            Ok("1")
        );
        assert_ne!(
            std::fs::read_link("/proc/self/ns/net").unwrap(),
            std::fs::read_link("/proc/1/ns/net").unwrap()
        );
        let binary = std::env::var("LINUXDROP_TEST_FILIN").unwrap();
        run_command(
            "/usr/sbin/ip",
            &["link", "add", "ldmon-test", "type", "dummy"],
            5,
        )
        .await
        .unwrap();
        let mut child = Command::new(binary)
            .args([
                "-i",
                "ldmon-test",
                "-h",
                "latap-test",
                "-c",
                "6",
                "-N",
                "--no-http",
                "--no-force-master",
            ])
            .env_clear()
            .env("PATH", "/usr/sbin:/usr/bin")
            .env("LC_ALL", "C")
            .env("LINUXDROP_ALLOWED_FREQUENCIES", "2437")
            .env("LINUXDROP_MANAGED_LEASE", "1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let result = wait_awdl_ready(&mut child, "latap-test", &CancellationToken::new()).await;
        assert!(
            result.is_err(),
            "A created TAP cannot report a failed channel setup as ready"
        );
        let output = timeout(Duration::from_secs(2), child.wait_with_output())
            .await
            .expect("Managed filin must exit rather than reopen the leased interface")
            .unwrap();
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains("Channel"),
            "The real channel failure must be reached: {error}"
        );
        let inventory = Command::new("/usr/sbin/ip")
            .args(["-j", "link", "show"])
            .output()
            .await
            .unwrap();
        assert!(inventory.status.success());
        let links: Vec<serde_json::Value> = serde_json::from_slice(&inventory.stdout).unwrap();
        assert!(
            !links.iter().any(|link| link["ifname"] == "latap-test"),
            "Failed startup must release its nonpersistent TAP"
        );
    }
}
