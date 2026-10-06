use gtk::prelude::*;
use gtk::{gio, glib};
use serde_json::Value;

pub const APP_ID: &str = "io.github.marius4lui.LinuxDrop.App";
pub const BUS: &str = "io.github.marius4lui.LinuxDrop";
pub const PATH: &str = "/io/github/marius4lui/LinuxDrop";
pub const INTERFACE: &str = "io.github.marius4lui.LinuxDrop.Manager1";

pub async fn connect() -> Result<gio::DBusProxy, String> {
    gio::DBusProxy::for_bus_future(
        gio::BusType::Session,
        gio::DBusProxyFlags::NONE,
        None,
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
    proxy
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
        })
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
