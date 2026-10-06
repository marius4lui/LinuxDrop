#[tokio::main(flavor = "current_thread")]
async fn main() {
    println!(
        "{}",
        serde_json::to_string_pretty(&linuxdrop_hardware::inventory().await).unwrap()
    );
}
