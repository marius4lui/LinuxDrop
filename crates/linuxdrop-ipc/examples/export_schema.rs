fn main() {
    println!(
        "{}",
        serde_json::to_string_pretty(&linuxdrop_ipc::snapshot_schema()).unwrap()
    );
}
