//! Narrow document grants and host directory resolution for the sandboxed client.
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

/// Info exists on older supported portals where GetHostPaths is not available.
pub async fn resolve_directory(
    connection: &zbus::Connection,
    id: String,
    relative: String,
) -> Result<String> {
    anyhow::ensure!(
        !id.is_empty() && id.len() <= 64 && id.bytes().all(|c| c.is_ascii_hexdigit()),
        "Invalid folder portal response"
    );
    let proxy = zbus::Proxy::new(
        connection,
        "org.freedesktop.portal.Documents",
        "/org/freedesktop/portal/documents",
        "org.freedesktop.portal.Documents",
    )
    .await?;
    let (path, applications): (Vec<u8>, HashMap<String, Vec<String>>) =
        proxy.call("Info", &(id,)).await?;
    let permissions = applications
        .get("io.github.marius4lui.LinuxDrop.App")
        .context("Choose the receiving folder again")?;
    anyhow::ensure!(
        ["read", "write"]
            .iter()
            .all(|required| permissions.iter().any(|p| p == required)),
        "Choose a writable receiving folder"
    );
    let path = std::str::from_utf8(path.strip_suffix(&[0]).unwrap_or(&path))?;
    anyhow::ensure!(
        std::path::Path::new(path).is_absolute(),
        "Invalid folder portal response"
    );
    let root = std::path::Path::new(path);
    anyhow::ensure!(
        relative.len() <= 4096
            && !std::path::Path::new(&relative).is_absolute()
            && std::path::Path::new(&relative)
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_))),
        "Invalid folder portal response"
    );
    let selected = if relative.is_empty() {
        root.to_owned()
    } else {
        let mut parts = std::path::Path::new(&relative).components();
        let name = parts.next().context("Invalid folder portal response")?;
        anyhow::ensure!(
            root.file_name() == Some(name.as_os_str()),
            "Invalid folder portal response"
        );
        root.join(parts.as_path())
    };
    let root = tokio::fs::canonicalize(root).await?;
    let path = tokio::fs::canonicalize(selected).await?;
    anyhow::ensure!(path.starts_with(&root), "Choose the receiving folder again");
    anyhow::ensure!(
        tokio::fs::metadata(&path).await?.is_dir(),
        "Choose a local folder"
    );
    Ok(path
        .to_str()
        .context("Choose a folder with a UTF-8 name")?
        .to_owned())
}
