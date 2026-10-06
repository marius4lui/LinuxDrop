use crate::{Manager, Shared};
use futures_util::StreamExt;
use linuxdrop_core::Transfer;
use std::{
    collections::HashMap,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};

pub async fn notify(shared: Arc<Shared>, transfer: Transfer) {
    if shared.locked.load(Ordering::Relaxed) {
        return;
    }
    let settings = shared.data.lock().await.settings.clone();
    let decision = matches!(transfer.state.as_str(), "waiting" | "verification")
        && (transfer.direction == "incoming" || transfer.verification_code.is_some());
    let failure = matches!(transfer.state.as_str(), "failed" | "rejected");
    if decision && settings["notifications"]["incoming"] != true {
        return;
    }
    if !decision
        && !(transfer.state == "completed" && settings["notifications"]["completed"] == true)
        && !(failure && settings["notifications"]["errors"] == true)
    {
        return;
    }
    let Some(connection) = shared.connection.get() else {
        return;
    };
    let Ok(proxy) = zbus::Proxy::new(
        connection,
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        "org.freedesktop.Notifications",
    )
    .await
    else {
        return;
    };
    let german = settings["general"]["language"] == "de"
        || (settings["general"]["language"] == "system"
            && std::env::var("LANG").is_ok_and(|lang| lang.starts_with("de")));
    let label = |en: &'static str, de: &'static str| if german { de } else { en };
    let summary = if decision {
        if transfer.verification_code.is_some() {
            label("Compare the sharing code", "Verbindungscode vergleichen")
        } else {
            label("Incoming files", "Eingehende Dateien")
        }
    } else if transfer.state == "completed" {
        label("Transfer complete", "Übertragung abgeschlossen")
    } else {
        label("Transfer stopped", "Übertragung beendet")
    };
    let private = settings["notifications"]["private_content"] == true;
    let body = if private {
        label(
            "Open LinuxDrop to view details.",
            "Details in LinuxDrop ansehen.",
        )
        .into()
    } else if let Some(code) = &transfer.verification_code {
        format!("{} · Code {}", escape(&transfer.peer_name), escape(code))
    } else {
        format!(
            "{} · {} file(s)",
            escape(&transfer.peer_name),
            transfer.files.len()
        )
    };
    let capabilities: Vec<String> = proxy.call("GetCapabilities", &()).await.unwrap_or_default();
    let actions = if !capabilities.iter().any(|s| s == "actions") {
        vec![]
    } else if decision && !private && settings["receive"]["ask_directory"] != true {
        vec![
            format!("accept:{}", transfer.id),
            if transfer.verification_code.is_some() {
                label("Codes match", "Codes stimmen überein")
            } else {
                label("Accept", "Annehmen")
            }
            .into(),
            format!("reject:{}", transfer.id),
            label("Reject", "Ablehnen").into(),
        ]
    } else {
        vec![
            "default".into(),
            label("Open LinuxDrop", "LinuxDrop öffnen").into(),
        ]
    };
    let mut hints: HashMap<&str, zbus::zvariant::Value<'_>> = HashMap::from([(
        "desktop-entry",
        zbus::zvariant::Value::from("io.github.marius4lui.LinuxDrop"),
    )]);
    hints.insert(
        "suppress-sound",
        zbus::zvariant::Value::from(settings["notifications"]["sound"] != true),
    );
    if settings["notifications"]["sound"] == true {
        hints.insert(
            "sound-name",
            zbus::zvariant::Value::from(if failure {
                "dialog-warning"
            } else {
                "message-new-instant"
            }),
        );
    }
    let result: Result<u32, _> = proxy
        .call(
            "Notify",
            &(
                "LinuxDrop",
                0u32,
                "io.github.marius4lui.LinuxDrop",
                summary,
                body,
                actions,
                hints,
                if decision { 0i32 } else { 8000 },
            ),
        )
        .await;
    if let Err(error) = result {
        tracing::debug!(%error,"Notification unavailable");
    }
}
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn start(shared: Arc<Shared>) {
    let actions = shared.clone();
    tokio::spawn(async move {
        let Some(connection) = actions.connection.get() else {
            return;
        };
        let Ok(proxy) = zbus::Proxy::new(
            connection,
            "org.freedesktop.Notifications",
            "/org/freedesktop/Notifications",
            "org.freedesktop.Notifications",
        )
        .await
        else {
            return;
        };
        let Ok(mut signals) = proxy.receive_signal("ActionInvoked").await else {
            return;
        };
        while let Some(message) = signals.next().await {
            let Ok((_notification, action)) = message.body().deserialize::<(u32, String)>() else {
                continue;
            };
            if actions.locked.load(Ordering::Relaxed) {
                continue;
            }
            if let Some((kind, id)) = action.split_once(':') {
                if matches!(kind, "accept" | "reject") {
                    let _ = actions.action(id, kind).await;
                }
            } else if action == "default" {
                let _ = Manager(actions.clone()).open_application().await;
            }
        }
    });
    tokio::spawn(async move {
        let Some(connection) = shared.connection.get() else {
            return;
        };
        loop {
            let active = async {
                let bus = zbus::fdo::DBusProxy::new(connection).await?;
                if !bus
                    .name_has_owner("org.gnome.Shell".try_into().unwrap())
                    .await?
                {
                    return Ok::<bool, zbus::Error>(false);
                }
                let proxy = zbus::Proxy::new(
                    connection,
                    "org.gnome.ScreenSaver",
                    "/org/gnome/ScreenSaver",
                    "org.gnome.ScreenSaver",
                )
                .await?;
                proxy.call::<_, _, bool>("GetActive", &()).await
            };
            if let Ok(Ok(locked)) = tokio::time::timeout(Duration::from_secs(2), active).await {
                let was = shared.locked.swap(locked, Ordering::Relaxed);
                if locked
                    && !was
                    && shared.data.lock().await.settings["visibility"]["hide_on_lock"] == true
                {
                    let _ = Manager(shared.clone())
                        .set_visibility("hidden".into())
                        .await;
                }
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
}

pub async fn after_receive(shared: Arc<Shared>, transfer: Transfer) {
    if shared.locked.load(Ordering::Relaxed)
        || transfer.direction != "incoming"
        || transfer.state != "completed"
    {
        return;
    }
    let settings = shared.data.lock().await.settings.clone();
    let paths = if settings["receive"]["open_after"] == true {
        transfer.saved_paths
    } else if settings["receive"]["open_folder"] == true {
        transfer
            .saved_paths
            .first()
            .and_then(|p| std::path::Path::new(p).parent())
            .map(|p| vec![p.to_string_lossy().into_owned()])
            .unwrap_or_default()
    } else {
        vec![]
    };
    for path in paths.into_iter().take(16) {
        let _ = tokio::process::Command::new("xdg-open").arg(path).spawn();
    }
}
