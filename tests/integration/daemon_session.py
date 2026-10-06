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
            call("UpdateSettings", "(s)", (json.dumps({"general": {"appearance": "dark"}}),))
            assert snapshot()["settings"]["general"]["appearance"] == "dark"
            assert not pending.done(), "Presentation settings interrupted incoming consent"
            call("AcceptTransfer", "(s)", (transfer["id"],))
            accepted = json.loads(pending.result())
            request(f'/upload?sessionId={accepted["sessionId"]}&fileId=file&token={accepted["files"]["file"]}', raw=payload)
            done = wait(lambda: next((t for t in snapshot()["transfers"] if t["state"] == "completed"), None))
            assert done["transferred_bytes"] == len(payload)
            assert (root / "received/integration.txt").read_bytes() == payload
            call("CancelTransfer", "(s)", (transfer["id"],))
            assert snapshot()["transfers"][0]["state"] == "completed"
        # Exercise the actual per-request destination/subset contract and the
        # duplicate-decision guard, rather than just serializing option values.
        offer["files"]["skip"] = {"id": "skip", "fileName": "excluded.txt", "size": 999, "fileType": "text/plain"}
        custom = root / "chosen-folder"
        with concurrent.futures.ThreadPoolExecutor() as executor:
            pending = executor.submit(request, "/prepare-upload", offer)
            transfer = wait(lambda: next((t for t in snapshot()["transfers"] if t["state"] == "waiting"), None))
            index = next(i for i, f in enumerate(transfer["files"]) if f["name"] == "integration.txt")
            call("AcceptTransferWithOptions", "(ss)", (transfer["id"], json.dumps({"directory": str(custom), "selected_indices": [index], "collision_policy": "reject"})))
            accepted = json.loads(pending.result())
            assert set(accepted["files"]) == {"file"}
            try:
                call("AcceptTransfer", "(s)", (transfer["id"],))
                raise AssertionError("Second client could accept the same request again")
            except GLib.Error:
                pass
            request(f'/upload?sessionId={accepted["sessionId"]}&fileId=file&token={accepted["files"]["file"]}', raw=payload)
            done = wait(lambda: next((t for t in snapshot()["transfers"] if t["id"] == transfer["id"] and t["state"] == "completed"), None))
            assert (custom / "integration.txt").read_bytes() == payload
            assert not (custom / "excluded.txt").exists()
        request("/register", offer["info"])
        peer = wait(lambda: next((p for p in snapshot()["peers"] if p["name"] == "Integration sender"), None))
        call("UpdatePeerPreferences", "(ss)", (peer["id"], json.dumps({"favorite": True, "display_name": "Private nickname", "blocked": True})))
        assert next(p for p in snapshot()["peers"] if p["id"] == peer["id"])["available"] is False
        redacted = call("ExportDiagnostics")
        assert "Private nickname" not in redacted and str(root) not in redacted and "integration.txt" not in redacted
        assert json.loads(call("GetDefaults"))["receive"]["collision_policy"] == "rename"
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
        assert next(p for p in restored["known_peers"] if p["id"] == peer["id"])["blocked"] is True
        call("ClearHistory")
        assert snapshot()["transfers"] == []
        assert json.loads(history.read_text()) == []
        assert (root / "received/integration.txt").read_bytes() == payload
        call("StopWhenIdle")
        assert daemon.wait(timeout=5) == 0
        print("PASS actual D-Bus/HTTPS consent, subset/custom destination, duplicate decision, private persisted preferences/history, redacted diagnostics, idle shutdown")
    finally:
        daemon.terminate()
        try:
            daemon.wait(timeout=5)
        except subprocess.TimeoutExpired:
            daemon.kill()
            daemon.wait()
