use gtk::prelude::*;
use gtk::{gio, glib};
use serde_json::Value;

pub use linuxdrop_ipc::{APP_ID, BUS, INTERFACE, PATH};

pub async fn connect() -> Result<gio::DBusProxy, String> {
    let node = gio::DBusNodeInfo::for_xml(linuxdrop_ipc::MANAGER_XML).map_err(|e| e.to_string())?;
    let info = node
        .lookup_interface(INTERFACE)
        .ok_or("Invalid service contract")?;
    gio::DBusProxy::for_bus_future(
        gio::BusType::Session,
        gio::DBusProxyFlags::NONE,
        Some(&info),
        BUS,
        PATH,
        INTERFACE,
    )
    .await
    .map_err(|e| e.to_string())
}

pub async fn call(
    proxy: &gio::DBusProxy,
    method: &str,
    parameters: Option<glib::Variant>,
) -> Result<glib::Variant, String> {
    let signatures = if proxy.interface_name() == INTERFACE {
        let method = linuxdrop_ipc::ManagerMethod::from_name(method)
            .ok_or_else(|| crate::i18n::tr("Sharing service contract mismatch"))?;
        let signatures = method.signatures();
        if parameters
            .as_ref()
            .map_or("()", |value| value.type_().as_str())
            != signatures.0
        {
            return Err(crate::i18n::tr("Sharing service contract mismatch"));
        }
        Some(signatures)
    } else {
        None
    };
    let result = proxy
        .call_future(
            method,
            parameters.as_ref(),
            gio::DBusCallFlags::NONE,
            if method == "RunHardwareDiagnostic" {
                120_000
            } else {
                30_000
            },
        )
        .await
        .map_err(|mut error| {
            // The transport namespace is diagnostic metadata, not user guidance.
            gio::DBusError::strip_remote_error(&mut error);
            crate::i18n::tr(error.message())
        })?;
    if signatures.is_some_and(|(_, output)| result.type_().as_str() != output) {
        return Err(crate::i18n::tr("Sharing service contract mismatch"));
    }
    Ok(result)
}

pub async fn json(proxy: &gio::DBusProxy, method: &str) -> Result<Value, String> {
    let result = call(proxy, method, None).await?;
    let (text,) = result
        .get::<(String,)>()
        .ok_or("Invalid service response")?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

pub fn string_result(value: glib::Variant) -> Result<String, String> {
    value
        .get::<(String,)>()
        .map(|v| v.0)
        .ok_or_else(|| "Invalid service response".into())
}

/// Pass opened sources, so the host daemon need not resolve sandbox paths.
/// Small batches stay within ordinary session-bus descriptor limits.
pub async fn prepare_files(proxy: &gio::DBusProxy, paths: Vec<String>) -> Result<String, String> {
    if paths.is_empty() {
        return Err(crate::i18n::tr("Select files first"));
    }
    let mut draft = String::new();
    let result = async {
        for paths in paths.chunks(16) {
            let paths = paths.to_vec();
            let files = gio::spawn_blocking(move || {
                paths
                    .into_iter()
                    .map(|path| {
                        let source = linuxdrop_core::SendSource::open(path)?;
                        Ok((source.name().to_owned(), source.reader()?))
                    })
                    .collect::<std::io::Result<Vec<_>>>()
            })
            .await
            .map_err(|_| "File preparation failed".to_owned())?
            .map_err(|error| error.to_string())?;
            let descriptors = gio::UnixFDList::new();
            let entries = files
                .into_iter()
                .map(|(name, file)| {
                    descriptors
                        .append(file)
                        .map(|index| (name, glib::variant::Handle(index)))
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| error.to_string())?;
            let parameters = (draft.clone(), entries).to_variant();
            let (reply, _) = proxy
                .call_with_unix_fd_list_future(
                    "PrepareSendFiles",
                    Some(&parameters),
                    gio::DBusCallFlags::NONE,
                    30000,
                    Some(&descriptors),
                )
                .await
                .map_err(|mut error| {
                    gio::DBusError::strip_remote_error(&mut error);
                    crate::i18n::tr(error.message())
                })?;
            draft = string_result(reply)?;
        }
        Ok(draft.clone())
    }
    .await;
    if result.is_err() && !draft.is_empty() {
        let _ = call(proxy, "DiscardDraft", Some((draft,).to_variant())).await;
    }
    result
}

/// Resolve chooser grants before persisting a destination used by the host daemon.
/// AddFull also handles directories already visible through a sandbox bind mount.
pub async fn receive_directory(file: gio::File) -> Result<String, String> {
    let path = file
        .path()
        .ok_or_else(|| crate::i18n::tr("Choose a local folder"))?;
    if !std::path::Path::new("/.flatpak-info").is_file() {
        return Ok(path.to_string_lossy().into_owned());
    }
    let portal = gio::DBusProxy::for_bus_future(
        gio::BusType::Session,
        gio::DBusProxyFlags::NONE,
        None,
        "org.freedesktop.portal.Documents",
        "/org/freedesktop/portal/documents",
        "org.freedesktop.portal.Documents",
    )
    .await
    .map_err(|error| error.to_string())?;
    // A chooser already exported this directory. Older portals reject exporting
    // its FUSE descriptor again, so use that existing grant directly.
    let mount = call(&portal, "GetMountPoint", None).await?;
    let (mount,) = mount
        .get::<(Vec<u8>,)>()
        .ok_or("Invalid folder portal response")?;
    let mount = std::str::from_utf8(mount.strip_suffix(&[0]).unwrap_or(&mount))
        .map_err(|_| "Invalid folder portal response")?;
    if let Ok(relative) = path.strip_prefix(mount) {
        let mut components = relative.components();
        let Some(std::path::Component::Normal(id)) = components.next() else {
            return Err(crate::i18n::tr("Invalid folder portal response"));
        };
        let proxy = connect().await?;
        return string_result(
            call(
                &proxy,
                "ResolveReceiveDirectory",
                Some(
                    (
                        id.to_string_lossy().as_ref(),
                        components.as_path().to_string_lossy().as_ref(),
                    )
                        .to_variant(),
                ),
            )
            .await?,
        );
    }
    let directory = gio::spawn_blocking(move || {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC)
            .open(path)
    })
    .await
    .map_err(|_| crate::i18n::tr("Could not check the receiving folder"))?
    .map_err(|error| error.to_string())?;
    let descriptors = gio::UnixFDList::new();
    let handle = descriptors
        .append(directory)
        .map_err(|error| error.to_string())?;
    // export-directory | reuse-existing; never invent authority from a pathname.
    let args = (
        vec![glib::variant::Handle(handle)],
        9u32,
        APP_ID,
        vec!["read", "write"],
    )
        .to_variant();
    let (reply, _) = portal
        .call_with_unix_fd_list_future(
            "AddFull",
            Some(&args),
            gio::DBusCallFlags::NONE,
            10000,
            Some(&descriptors),
        )
        .await
        .map_err(|error| error.to_string())?;
    let ids = reply
        .child_value(0)
        .get::<Vec<String>>()
        .ok_or("Invalid folder portal response")?;
    let id = ids
        .first()
        .filter(|id| !id.is_empty())
        .ok_or("Invalid folder portal response")?;
    let proxy = connect().await?;
    string_result(
        call(
            &proxy,
            "ResolveReceiveDirectory",
            Some((id, "").to_variant()),
        )
        .await?,
    )
}
