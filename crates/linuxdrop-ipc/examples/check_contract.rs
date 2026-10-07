//! Called only on the isolated daemon integration bus.
use futures_util::StreamExt;
use linuxdrop_ipc::ManagerProxy;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(std::env::var("LINUXDROP_TEST_CONTRACT").as_deref(), Ok("1"));
    let bus = zbus::Connection::session().await?;
    let proxy = ManagerProxy::new(&bus).await?;
    let snapshot: serde_json::Value = serde_json::from_str(&proxy.get_snapshot().await?)?;
    assert!(snapshot["epoch"].is_string());
    let mut changed = proxy.receive_changed().await?;
    assert!(proxy
        .set_visibility("invalid-contract-test".into())
        .await
        .is_err());
    proxy.set_visibility("hidden".into()).await?;
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), changed.next())
        .await?
        .ok_or("Missing Changed signal")?;
    assert!(*event.args()?.revision() > snapshot["revision"].as_u64().ok_or("Missing revision")?);
    assert!(proxy.get_defaults().await?.contains("visibility"));
    assert!(proxy
        .prepare_send_files(String::new(), Vec::new())
        .await
        .is_err());
    println!(
        "Typed Manager1 proxy: snapshot, defaults, signal and rejected invalid requests passed"
    );
    Ok(())
}
