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
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| format!("AWDL helper unavailable: {error}"))?;
    let ready = async {
        for _ in 0..30 {
            check_cancel(&cancel)?;
            if child
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_some()
            {
                return Err("AWDL helper exited before creating its interface".into());
            }
            if Path::new("/sys/class/net").join(tap).exists() {
                return Ok(());
            }
            tokio::select! {
                _ = cancel.cancelled() => return Err("Radio preparation cancelled".into()),
                _ = tokio::time::sleep(Duration::from_millis(100)) => {},
            }
        }
        Err("AWDL helper did not create its interface".into())
    }
    .await;
    if let Err(error) = ready {
        reap_child(Some(child)).await?;
        return Err(error);
    }
    Ok(Some(child))
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
