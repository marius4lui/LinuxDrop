//! Channel changes share the same producer/retirement ordering as acquisition.
use super::*;
use tokio_util::sync::CancellationToken;

fn validated_lease(state: &State, owned: &[String], id: &str) -> Result<Lease, String> {
    if !state.attached.contains(id) {
        return Err("lease is revoked; complete radio recovery first".into());
    }
    if !owned.iter().any(|owned| owned == id) {
        return Err("lease does not belong to this connection".into());
    }
    if state.producers.contains_key(id) {
        return Err("another radio operation is still in progress".into());
    }
    let lease = state.leases.get(id).ok_or("lease not found")?;
    if lease.kind == LeaseKind::DirectWifi {
        return Err("direct Wi-Fi reservation cannot change monitor channels".into());
    }
    if lease.awdl_interface.is_some() {
        return Err("AWDL channel scheduling belongs to the link helper".into());
    }
    Ok(lease.clone())
}

pub(super) async fn set_channel(
    shared: &Shared,
    owned: &mut Vec<String>,
    id: String,
    channel: u16,
) -> Result<Response, String> {
    {
        let state = shared.lock().await;
        validated_lease(&state, owned, &id)?;
    }
    let inventory = inventory().await;
    let mut state = shared.lock().await;
    // Ownership/revocation can change while inventory is awaiting external I/O.
    let lease = validated_lease(&state, owned, &id)?;
    let radio = inventory
        .radios
        .iter()
        .find(|radio| radio.phy == lease.phy)
        .ok_or("radio unplugged")?;
    if inventory.interfaces.iter().any(|interface| {
        interface.phy.as_deref() == Some(&lease.phy)
            && interface.name != lease.interface
            && interface.in_use()
    }) {
        return Err("radio became active outside the lease".into());
    }
    if !radio.channels.iter().any(|candidate| {
        candidate.number == channel && !candidate.disabled && !candidate.no_ir && !candidate.radar
    }) {
        return Err("channel is restricted or unavailable".into());
    }
    verify_owned(&lease)?;
    let cancel = CancellationToken::new();
    let stop_on_drop = cancel.clone().drop_guard();
    let reply = spawn_channel_with(
        &mut state,
        shared,
        lease,
        channel,
        cancel,
        move |lease| async move {
            // Recheck the kernel marker immediately before the mutation too.
            verify_owned(&lease)?;
            run_command(
                "/usr/sbin/iw",
                &[
                    "dev",
                    &lease.interface,
                    "set",
                    "channel",
                    &channel.to_string(),
                ],
                5,
            )
            .await
        },
        persist,
    );
    drop(state);
    let result = reply
        .await
        .map_err(|_| "Channel supervisor stopped without a reply".to_owned())?;
    stop_on_drop.disarm();
    if let Err(error) = result {
        let (exists, attached) = {
            let state = shared.lock().await;
            (state.leases.contains_key(&id), state.attached.contains(&id))
        };
        if !exists {
            owned.retain(|owned| owned != &id);
        }
        if exists && !attached {
            return Err(match cleanup_lease(shared, &id).await {
                Ok(()) => {
                    owned.retain(|owned| owned != &id);
                    error
                }
                Err(cleanup) => format!("{error}; radio recovery required: {cleanup}"),
            });
        }
        return Err(error);
    }
    result
}

pub(super) fn spawn_channel_with<F, Fut, P>(
    state: &mut State,
    shared: &Shared,
    lease: Lease,
    channel: u16,
    cancel: CancellationToken,
    operation: F,
    journal: P,
) -> tokio::sync::oneshot::Receiver<Result<Response, String>>
where
    F: FnOnce(Lease) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<(), String>> + Send + 'static,
    P: FnOnce(&State) -> io::Result<()> + Send + 'static,
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
            if worker_cancel.is_cancelled() {
                return Err("Channel change cancelled".into());
            }
            // Never select cancellation against the command: its explicit timeout
            // path terminates/reaps it before this producer can settle.
            let changed = operation(lease.clone()).await;
            let mut state = worker_shared.lock().await;
            state.radio_changed();
            if let Err(error) = changed {
                state.attached.remove(&lease.id);
                state.record_recovery_error(format!(
                    "{}: channel mutation failed: {error}",
                    lease.id
                ));
                return Err(error);
            }
            let current = state
                .leases
                .get_mut(&lease.id)
                .ok_or("lease disappeared during channel mutation")?;
            current.channel = channel;
            if let Err(error) = journal(&state) {
                state.attached.remove(&lease.id);
                state.record_recovery_error(format!(
                    "{}: channel journal failed: {error}",
                    lease.id
                ));
                return Err(format!("Channel journal failed: {error}"));
            }
            if worker_cancel.is_cancelled() || state.cleanups.contains_key(&lease.id) {
                return Err("Channel change cancelled".into());
            }
            Ok(Response::Ok)
        },
    )
}
