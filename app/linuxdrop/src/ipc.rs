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
            30_000,
        )
        .await
        .map_err(|e| e.to_string())
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
