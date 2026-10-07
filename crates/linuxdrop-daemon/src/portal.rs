//! Read-only session exports for the optional sandboxed GTK client.
use anyhow::{Context, Result};
use linuxdrop_core::SendSource;
use std::{collections::HashMap, path::PathBuf};
use zbus::zvariant::{Fd, OwnedValue};

pub async fn export_received(connection: &zbus::Connection, path: String) -> Result<String> {
    // Use the same nonblocking, no-follow regular-file validation as sending.
    // Opening happens outside the shared daemon lock and cannot block its runtime.
    let source = tokio::task::spawn_blocking(move || SendSource::open(path)).await??;
    let file = source.reader()?;
    let proxy = zbus::Proxy::new(
        connection,
        "org.freedesktop.portal.Documents",
        "/org/freedesktop/portal/documents",
        "org.freedesktop.portal.Documents",
    )
    .await?;
    let mount: Vec<u8> = proxy.call("GetMountPoint", &()).await?;
    let mount = std::str::from_utf8(mount.strip_suffix(&[0]).unwrap_or(&mount))?;
    anyhow::ensure!(
        std::path::Path::new(mount).is_absolute(),
        "Invalid document portal mount"
    );
    // Reuse an export, do not persist it or grant write/delete/directory access.
    let (ids, _): (Vec<String>, HashMap<String, OwnedValue>) = proxy
        .call(
            "AddFull",
            &(
                vec![Fd::from(&file)],
                1u32,
                "io.github.marius4lui.LinuxDrop.App",
                vec!["read"],
            ),
        )
        .await?;
    let id = ids
        .first()
        .filter(|id| !id.is_empty() && id.bytes().all(|c| c.is_ascii_hexdigit()))
        .context("Invalid document portal export")?;
    Ok(PathBuf::from(mount)
        .join(id)
        .join(source.name())
        .to_string_lossy()
        .into_owned())
}
