#!/usr/bin/env python3
"""Exercise actual D-Bus + HTTPS consent without external devices or radio changes.
Run: dbus-run-session -- python3 tests/integration/daemon_session.py /path/linuxdropd
"""
import concurrent.futures
import json
import os
from pathlib import Path
import socket
import ssl
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from gi.repository import Gio, GLib


with tempfile.TemporaryDirectory(prefix="linuxdrop-session-") as root:
    root = Path(root)
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    config_dir = root / "config/linuxdrop"
    config_dir.mkdir(parents=True)
    (config_dir / "settings.json").write_text(json.dumps({
        "general": {"device_name": "Session test"},
        "receive": {"directory": str(root / "received")},
        "localsend": {"port": port, "multicast": False},
        "quickshare": {"enabled": False},
    }))
    env = dict(os.environ, XDG_CONFIG_HOME=str(root / "config"),
               XDG_DATA_HOME=str(root / "data"), HOME=str(root))
    daemon = subprocess.Popen([sys.argv[1]], env=env, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True)
    try:
        bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)

        def call(method, signature=None, values=()):
            args = GLib.Variant(signature, values) if signature else None
            result = bus.call_sync("io.github.marius4lui.LinuxDrop",
                "/io/github/marius4lui/LinuxDrop", "io.github.marius4lui.LinuxDrop.Manager1",
                method, args, None, Gio.DBusCallFlags.NO_AUTO_START, 15000, None)
            data = result.unpack()
            return data[0] if data else None

        def wait(predicate, seconds=10):
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                if daemon.poll() is not None:
                    raise RuntimeError(daemon.stdout.read())
                try:
                    result = predicate()
                    if result:
                        return result
                except GLib.Error:
                    pass
                time.sleep(0.05)
            raise AssertionError("Timed out waiting for daemon state")

        def snapshot():
            return json.loads(call("GetSnapshot"))

        wait(lambda: any(b["id"] == "localsend" and b["state"] == "ready"
                         for b in snapshot()["backends"]))
        first = snapshot()
        assert first["settings"]["visibility"]["mode"] == "hidden"
        # The test client deliberately accepts the generated certificate. Product
        # client certificate pinning is covered by the Rust TLS integration test.
        context = ssl._create_unverified_context()
        base = f"https://127.0.0.1:{port}/api/localsend/v2"

        def request(route, payload=None, raw=None):
            body = raw if raw is not None else json.dumps(payload).encode() if payload is not None else None
            req = urllib.request.Request(base + route, body,
                headers={"Content-Type": "application/json" if raw is None else "application/octet-stream"})
            with urllib.request.urlopen(req, context=context, timeout=10) as response:
                return response.read()

        try:
            request("/info")
            raise AssertionError("Hidden receiver disclosed info")
        except urllib.error.HTTPError as error:
            assert error.code == 403
        call("SetVisibility", "(s)", ("everyone",))
        payload = b"D-Bus consent to HTTPS transfer\n"
        offer = {
            "info": {"alias": "Integration sender", "version": "2.1", "fingerprint": "f" * 64,
                     "port": 53317, "protocol": "https", "deviceType": "desktop"},
            "files": {"file": {"id": "file", "fileName": "integration.txt", "size": len(payload), "fileType": "text/plain"}},
        }
        with concurrent.futures.ThreadPoolExecutor() as executor:
            pending = executor.submit(request, "/prepare-upload", offer)
            transfer = wait(lambda: next((t for t in snapshot()["transfers"] if t["state"] == "waiting"), None))
            assert list((root / "received").iterdir()) == []
            call("AcceptTransfer", "(s)", (transfer["id"],))
            accepted = json.loads(pending.result())
            request(f'/upload?sessionId={accepted["sessionId"]}&fileId=file&token={accepted["files"]["file"]}', raw=payload)
            done = wait(lambda: next((t for t in snapshot()["transfers"] if t["state"] == "completed"), None))
            assert done["transferred_bytes"] == len(payload)
            assert (root / "received/integration.txt").read_bytes() == payload
            call("CancelTransfer", "(s)", (transfer["id"],))
            assert snapshot()["transfers"][0]["state"] == "completed"
        try:
            call("UpdateSettings", "(s)", (json.dumps({"localsend": {"port": 1}}),))
            raise AssertionError("Unsafe settings accepted")
        except GLib.Error:
            pass
        assert snapshot()["revision"] > first["revision"]
        history = root / "data/linuxdrop/history.json"
        wait(lambda: history.exists() and json.loads(history.read_text()))
        assert history.stat().st_mode & 0o777 == 0o600
        daemon.terminate()
        daemon.wait(timeout=5)
        daemon = subprocess.Popen([sys.argv[1]], env=env, stdout=subprocess.PIPE,
                                  stderr=subprocess.STDOUT, text=True)
        restored = wait(lambda: snapshot() if snapshot()["epoch"] != first["epoch"] else None)
        assert restored["settings"]["visibility"]["mode"] == "hidden"
        assert restored["transfers"][0]["id"] == done["id"]
        assert restored["transfers"][0]["state"] == "completed"
        call("ClearHistory")
        assert snapshot()["transfers"] == []
        assert json.loads(history.read_text()) == []
        assert (root / "received/integration.txt").read_bytes() == payload
        print("PASS actual D-Bus snapshot, visibility, consent, HTTPS bytes, terminal state, settings validation, private history restart and clear")
    finally:
        daemon.terminate()
        try:
            daemon.wait(timeout=5)
        except subprocess.TimeoutExpired:
            daemon.kill()
            daemon.wait()
