#!/usr/bin/env python3
"""Exercise actual D-Bus + HTTPS consent without external devices or radio changes.
Run: dbus-run-session -- python3 tests/integration/daemon_session.py /path/linuxdropd
"""
import concurrent.futures
import http.server
import json
import os
from pathlib import Path
import socket
import ssl
import subprocess
import sys
import tempfile
import time
import threading
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
        wait(lambda: not snapshot()["restarting"])
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
        # Reserving a restart closes admission before the supervisor begins teardown.
        selected = root / "restart-selection.txt"
        selected.write_bytes(b"retain across restart")
        draft = call("PrepareSend", "(as)", ([str(selected)],))
        # Hold an accepted TLS handshake open. This makes admission testing
        # deterministic while exercising real connection drain, not a restart sleep.
        stalled = socket.create_connection(("127.0.0.1", port))
        remote_port = stalled.getsockname()[1]
        def accepted_stalled_connection():
            inodes = set()
            for line in Path(f"/proc/{daemon.pid}/net/tcp").read_text().splitlines()[1:]:
                fields = line.split()
                if (int(fields[1].split(":")[1], 16) == port
                        and int(fields[2].split(":")[1], 16) == remote_port):
                    inodes.add(f"socket:[{fields[9]}]")
            return any(os.readlink(fd) in inodes for fd in Path(f"/proc/{daemon.pid}/fd").iterdir())
        wait(accepted_stalled_connection)
        call("RestartBackends")
        assert snapshot()["restarting"]
        for method, signature, args in [
            ("StartSend", "(sss)", (draft, "absent-peer", "localsend")),
            ("CreateDownloadOffer", "(s)", (draft,)),
            ("ReceiveDownloadOffer", "(s)", ("http://127.0.0.1:53318",)),
            ("RestartBackends", None, ()),
            ("UpdateSettings", "(s)", (json.dumps({"general": {"device_name": "must-not-apply"}}),)),
        ]:
            try:
                call(method, signature, args)
                raise AssertionError(f"{method} bypassed restart admission")
            except GLib.Error as error:
                assert "restarting" in str(error), str(error)
        stalled.close()
        wait(lambda: not snapshot()["restarting"])
        assert snapshot()["settings"]["general"]["device_name"] == "Session test"
        # The selected descriptor is still owned after every rejected admission.
        assert any(os.readlink(fd) == str(selected) for fd in Path(f"/proc/{daemon.pid}/fd").iterdir())
        # Once ready, ordinary peer validation resumes.
        try:
            call("StartSend", "(sss)", (draft, "absent-peer", "localsend"))
        except GLib.Error as error:
            assert "Device is no longer available" in str(error), str(error)
        call("DiscardDraft", "(s)", (draft,))
        call("SetVisibility", "(s)", ("everyone",))
        payload = b"D-Bus consent to HTTPS transfer\n"
        offer = {
            "info": {"alias": "Integration sender", "version": "2.1", "fingerprint": "f" * 64,
                     "port": 53317, "protocol": "https", "deviceType": "desktop"},
            "files": {"file": {"id": "file", "fileName": "integration.txt", "size": len(payload), "fileType": "text/plain",
                               "metadata": {"modified": "2021-01-01T14:34:56.123456789+02:00",
                                            "accessed": "2021-01-01T12:34:57.987654321Z", "futureOptional": True},
                               "futureFileField": {"ignored": True}}},
        }
        with concurrent.futures.ThreadPoolExecutor() as executor:
            pending = executor.submit(request, "/prepare-upload", offer)
            transfer = wait(lambda: next((t for t in snapshot()["transfers"] if t["state"] == "waiting"), None))
            assert list((root / "received").iterdir()) == []
            try:
                call("RestartBackends")
                raise AssertionError("Restart interrupted an active receive request")
            except GLib.Error as error:
                assert "Finish or cancel active transfers" in str(error), str(error)
            assert not snapshot()["restarting"]
            call("UpdateSettings", "(s)", (json.dumps({"general": {"appearance": "dark"}}),))
            assert snapshot()["settings"]["general"]["appearance"] == "dark"
            assert not pending.done(), "Presentation settings interrupted incoming consent"
            call("AcceptTransfer", "(s)", (transfer["id"],))
            accepted = json.loads(pending.result())
            request(f'/upload?sessionId={accepted["sessionId"]}&fileId=file&token={accepted["files"]["file"]}', raw=payload)
            done = wait(lambda: next((t for t in snapshot()["transfers"] if t["state"] == "completed"), None))
            assert done["transferred_bytes"] == len(payload)
            saved_stat = (root / "received/integration.txt").stat()
            assert saved_stat.st_mtime_ns == 1609504496123456789
            assert saved_stat.st_atime_ns == 1609504497987654321
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
        wait(lambda: not snapshot()["restarting"])
        assert restored["settings"]["visibility"]["mode"] == "hidden"
        assert restored["transfers"][0]["id"] == done["id"]
        assert restored["transfers"][0]["state"] == "completed"
        assert next(p for p in restored["known_peers"] if p["id"] == peer["id"])["blocked"] is True
        call("ClearHistory")
        assert snapshot()["transfers"] == []
        assert json.loads(history.read_text()) == []
        assert (root / "received/integration.txt").read_bytes() == payload
        # An explicit download link must obey the same network allowlist as
        # discovery; refusing it must preserve the user's prepared file draft.
        call("UpdateSettings", "(s)", (json.dumps({"network": {"allowed_interfaces": ["not-present0"]}}),))
        wait(lambda: not snapshot()["restarting"])
        draft = call("PrepareSend", "(as)", ([str(root / "received/integration.txt")],))
        for _ in range(2):
            try:
                call("CreateDownloadOffer", "(s)", (draft,))
                raise AssertionError("Download offer escaped the network allowlist")
            except GLib.Error as error:
                assert "No enabled IPv4 LAN interface" in str(error), str(error)
        call("DiscardDraft", "(s)", (draft,))
        wait(lambda: not snapshot()["restarting"] and any(b["id"] == "localsend" and b["state"] == "unavailable" for b in snapshot()["backends"]))
        # Explicit reverse reception remains available while hidden, but the
        # daemon must require the PIN and actual receive consent before payload.
        downloads = []
        class OfferHandler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass
            def do_POST(self):
                if self.path != "/api/localsend/v2/prepare-download?pin=424242":
                    self.send_response(401)
                    self.end_headers()
                    return
                body = json.dumps({"info": {"alias": "Download sender", "fingerprint": "download-fixture"},
                    "sessionId": "download-session", "files": {"one": {"id": "one", "fileName": "downloaded.txt",
                    "size": 8, "fileType": "text/plain"}}}).encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)
            def do_GET(self):
                downloads.append(self.path)
                self.send_response(200)
                self.send_header("Content-Length", "8")
                self.end_headers()
                self.wfile.write(b"download")
        offer = http.server.ThreadingHTTPServer(("127.0.0.1", 0), OfferHandler)
        thread = threading.Thread(target=offer.serve_forever, daemon=True)
        thread.start()
        try:
            transfer_id = call("ReceiveDownloadOffer", "(s)", (f"http://127.0.0.1:{offer.server_port}",))
            def download_state(state):
                return next((t for t in snapshot()["transfers"] if t["id"] == transfer_id and t["state"] == state), None)
            wait(lambda: download_state("pin_required"))
            assert not downloads
            call("ProvideTransferPin", "(ss)", (transfer_id, "424242"))
            waiting = wait(lambda: download_state("waiting"))
            assert waiting["direction"] == "incoming" and waiting["peer_name"] == "Download sender"
            assert not downloads
            chosen = root / "reverse-received"
            call("AcceptTransferWithOptions", "(ss)", (transfer_id, json.dumps({"directory": str(chosen), "selected_indices": [0]})))
            received = wait(lambda: download_state("completed"))
            assert received["transferred_bytes"] == 8
            assert (chosen / "downloaded.txt").read_bytes() == b"download"
            assert len(downloads) == 1
            assert snapshot()["settings"]["visibility"]["mode"] == "hidden"
        finally:
            offer.shutdown()
            offer.server_close()
            thread.join(timeout=2)
        call("StopWhenIdle")
        assert daemon.wait(timeout=5) == 0
        print("PASS restart admission/quiescing and active-request protection, actual D-Bus/HTTPS consent and timestamp/forward-field interoperability, subset/custom destination, duplicate decision, private persisted preferences/history, redacted diagnostics, download-link network restriction with draft retention, reverse-download PIN and consent while hidden, idle shutdown")
    finally:
        daemon.terminate()
        try:
            daemon.wait(timeout=5)
        except subprocess.TimeoutExpired:
            daemon.kill()
            daemon.wait()
