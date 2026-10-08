# Local live transfer demo

The normal LinuxDrop UI can send real files to a clearly labelled receiver on the same Ubuntu system. The receiver uses the actual LocalSend backend, HTTPS, certificate pinning, and regular storage code. It does not add simulated devices to the application.

Start the installed LinuxDrop desktop first, then run as the same user:

```sh
sh packaging/dev/start-demo-peer.sh
```

For the prepared WSL development environment, add `CARGO_TARGET_DIR=/opt/linuxdrop-target` before this command. The launcher builds the Rust example if needed; Cargo must be available for that first build.

In LinuxDrop choose **Demo-Empfänger (Ubuntu)** and send `~/LinuxDrop-Demo/Zum-Testen.txt` or drag any test file into the main window. For the desktop bubble, first click the LinuxDrop icon in the top panel, then choose **Drop files** or drag over the open bubble. The bubble stays hidden at startup and closes on an outside click or Escape. Received bytes appear in `~/LinuxDrop-Demo/received`. The terminal prints outcomes and saved paths. Existing files are preserved using the normal collision handling.

The demo receiver automatically accepts files, is explicitly enabled by the launcher, and listens only on **127.0.0.1:53319**. It does not advertise to the LAN. Stop it with Ctrl+C; it is never installed as a background service or included in package autostart.

To also test incoming consent in the real UI, pass `--send-back`. Once LinuxDrop is visible, the demo sends its sample file once; approve or reject it in LinuxDrop. The real application always retains its normal consent flow.

Parallel isolated desktop sessions can select separate ports with `--daemon-port 53327 --port 53329`. The matching environment variables are `LINUXDROP_DEMO_DAEMON_PORT` and `LINUXDROP_DEMO_PORT`; explicit arguments take precedence. Each Linux user uses their own certificate and demo directory.

This proves local application/protocol operation. Android, Apple, radio, and cross-device interoperability require separate physical-device acceptance.
