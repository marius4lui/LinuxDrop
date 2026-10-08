#!/usr/bin/env python3
"""Private-session real SIGTERM/SIGINT exit with an accepted, stalled TLS upload."""
import concurrent.futures
import json
import os
from pathlib import Path
import signal
import socket
import ssl
import subprocess
import sys
import tempfile
import time
import urllib.request

from gi.repository import Gio, GLib


with tempfile.TemporaryDirectory(prefix="linuxdrop-shutdown-") as temporary:
    root = Path(temporary)
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    config = root / "config/linuxdrop"
    config.mkdir(parents=True)
    received = root / "received"
    (config / "settings.json").write_text(json.dumps({
        "general": {"device_name": "Shutdown test"},
        "receive": {"directory": str(received)},
        "localsend": {"port": port, "multicast": False},
        "quickshare": {"enabled": False},
    }))
    environment = dict(os.environ, XDG_CONFIG_HOME=str(root / "config"),
                       XDG_DATA_HOME=str(root / "data"), HOME=str(root))
    bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
    # Test-only trust of the receiver certificate; pinning is separately covered.
    context = ssl._create_unverified_context()

    def call(method, signature=None, values=()):
        response = bus.call_sync("io.github.marius4lui.LinuxDrop",
            "/io/github/marius4lui/LinuxDrop", "io.github.marius4lui.LinuxDrop.Manager1",
            method, GLib.Variant(signature, values) if signature else None,
            None, Gio.DBusCallFlags.NO_AUTO_START, 10000, None)
        fields = response.unpack()
        return fields[0] if fields else None

    def snapshot():
        return json.loads(call("GetSnapshot"))

    def wait(predicate):
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            if daemon.poll() is not None:
                raise AssertionError(daemon.stdout.read())
            try:
                result = predicate()
                if result:
                    return result
            except GLib.Error:
                pass
            time.sleep(0.025)
        raise AssertionError("Shutdown fixture did not reach its expected state")

    ids = []
    for shutdown_signal in [signal.SIGTERM, signal.SIGINT]:
        daemon = subprocess.Popen([sys.argv[1]], env=environment,
                                  stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
        stream = None
        try:
            wait(lambda: not snapshot()["restarting"] and
                 any(b["id"] == "localsend" and b["state"] == "ready" for b in snapshot()["backends"]))
            for old_id in ids:
                assert any(t["id"] == old_id and t["state"] in ["failed", "cancelled"]
                           for t in snapshot()["transfers"]), "Interrupted transfer history was lost"
            call("SetVisibility", "(s)", ("everyone",))
            payload = {
                "info": {"alias": "Stalled sender", "version": "2.1", "fingerprint": "a" * 64,
                         "port": 53317, "protocol": "https", "deviceType": "desktop"},
                "files": {"file": {"id": "file", "fileName": "incomplete.txt", "size": 65536,
                                   "fileType": "text/plain"}},
            }
            prepare = urllib.request.Request(f"https://127.0.0.1:{port}/api/localsend/v2/prepare-upload",
                json.dumps(payload).encode(), headers={"Content-Type": "application/json"})

            def offer():
                with urllib.request.urlopen(prepare, context=context, timeout=10) as response:
                    return json.load(response)

            with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
                future = executor.submit(offer)
                transfer = wait(lambda: next((t for t in snapshot()["transfers"] if t["state"] == "waiting"), None))
                ids.append(transfer["id"])
                call("AcceptTransfer", "(s)", (transfer["id"],))
                accepted = future.result(timeout=10)
            stream = context.wrap_socket(socket.create_connection(("127.0.0.1", port), timeout=5),
                                         server_hostname="localhost")
            route = f'/api/localsend/v2/upload?sessionId={accepted["sessionId"]}&fileId=file&token={accepted["files"]["file"]}'
            stream.sendall((f"POST {route} HTTP/1.1\r\nHost: localhost\r\n"
                            "Content-Length: 65536\r\nContent-Type: application/octet-stream\r\n\r\n").encode()
                           + b"partial payload")
            wait(lambda: list(received.glob(".linuxdrop-*.part")))
            daemon.send_signal(shutdown_signal)
            output, _ = daemon.communicate(timeout=10)
            assert daemon.returncode == 0, (shutdown_signal, daemon.returncode, output)
            assert not list(received.iterdir()), "Exit left a partial or falsely completed file"
            history = json.loads((root / "data/linuxdrop/history.json").read_text())
            assert any(t["id"] == transfer["id"] and t["state"] in ["failed", "cancelled"] for t in history)
            # The next loop starts a new daemon on this exact listener port and
            # reloads the persisted interrupted-transfer history.
        finally:
            if stream is not None:
                stream.close()
            if daemon.poll() is None:
                daemon.kill()
                daemon.communicate()
    print("PASS SIGTERM and SIGINT: stalled accepted TLS upload drained, partial removed, terminal history persisted/reloaded, same-port restart")
