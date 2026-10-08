#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
#[tokio::main]
async fn main() -> std::io::Result<()> {
    linux::run().await
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("linuxdrop-netd requires Linux");
    std::process::exit(1);
}
