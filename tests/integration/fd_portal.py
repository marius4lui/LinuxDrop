#!/usr/bin/env python3
"""Real descriptor/portal integration; run only through run-fd-portal.sh."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request
from gi.repository import Gio, GLib

assert os.environ.get("LINUXDROP_FD_TEST_PRIVATE") == "1"
assert os.readlink("/proc/self/ns/net") != os.readlink("/proc/1/ns/net")
BUS = "io.github.marius4lui.LinuxDrop"
OBJECT = "/io/github/marius4lui/LinuxDrop"
IFACE = BUS + ".Manager1"
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)


def call(method, signature=None, values=(), fds=None, destination=BUS, path=OBJECT, interface=IFACE):
    args = GLib.Variant(signature, values) if signature else None
    reply, _ = bus.call_with_unix_fd_list_sync(destination, path, interface,
        method, args, None, Gio.DBusCallFlags.NO_AUTO_START if destination == BUS else Gio.DBusCallFlags.NONE, 10000, fds, None)
    return reply.unpack()


def prepare(draft, entries):
    fds = Gio.UnixFDList.new()
    values = [(name, fds.append(fd)) for name, fd in entries]
    return call("PrepareSendFiles", "(sa(sh))", (draft, values), fds)[0]


def rejected(fn):
    try:
        fn()
    except GLib.Error:
        return
    raise AssertionError("Invalid descriptor batch was accepted")


with tempfile.TemporaryDirectory(prefix="linuxdrop-fd-sources-") as directory:
    root = Path(directory)
    config = Path(os.environ["XDG_CONFIG_HOME"]) / "linuxdrop"
    config.mkdir(parents=True)
    (config / "settings.json").write_text(json.dumps({
        "localsend": {"enabled": False}, "quickshare": {"enabled": False},
        "airdrop": {"enabled": False},
        "receive": {"max_files": 32, "max_bytes": 4096, "directory": str(root / "received")},
        "network": {"allowed_interfaces": ["ld-fd-test"]},
    }))
    daemon = subprocess.Popen([sys.argv[1]], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    try:
        for _ in range(100):
            if daemon.poll() is not None:
                raise AssertionError(daemon.stderr.read().decode())
            try:
                call("GetSnapshot")
                break
            except GLib.Error:
                time.sleep(.05)
        else:
            raise AssertionError("Daemon did not start")
        source = root / "original.txt"
        source.write_bytes(b"selected bytes")
        with source.open("rb") as file:
            # An actual document portal exports the file; no synthetic /doc path.
            portal_fds = Gio.UnixFDList.new()
            handle = portal_fds.append(file.fileno())
            portal_args = dict(destination="org.freedesktop.portal.Documents",
                path="/org/freedesktop/portal/documents", interface="org.freedesktop.portal.Documents")
            doc_id = call("Add", "(hbb)", (handle, True, False), portal_fds, **portal_args)[0]
            mount = bytes(call("GetMountPoint", **portal_args)[0]).rstrip(b"\0").decode()
            exported = Path(mount) / doc_id / source.name
            assert exported.read_bytes() == b"selected bytes"
            with exported.open("rb") as portal_file:
                draft = prepare("", [("portal.txt", portal_file.fileno())])
            call("Delete", "(s)", (doc_id,), **portal_args)
            assert not exported.exists(), "Portal export must actually be revoked"
            source.rename(root / "moved.txt")
            source.write_bytes(b"replacement must not be sent")
            # Multiple messages exceed the usual single-message descriptor budget.
            draft = prepare(draft, [(f"file-{n}.txt", file.fileno()) for n in range(16)])
            draft = prepare(draft, [(f"file-{n}.txt", file.fileno()) for n in range(16, 24)])
            rejected(lambda: prepare(draft, [("../escape", file.fileno())]))
            rejected(lambda: prepare(draft, [(f"overflow-{n}", file.fileno()) for n in range(8)]))
            oversized = root / "oversized.bin"
            oversized.write_bytes(b"x" * 4097)
            with oversized.open("rb") as large:
                rejected(lambda: prepare(draft, [("valid.txt", file.fileno()), ("large.bin", large.fileno())]))
            write_fd = os.open(source, os.O_WRONLY)
            dir_fd = os.open(root, os.O_RDONLY | os.O_DIRECTORY)
            read_pipe, write_pipe = os.pipe()
            try:
                for bad in (write_fd, dir_fd, read_pipe):
                    rejected(lambda: prepare(draft, [("valid.txt", file.fileno()), ("bad.txt", bad)]))
            finally:
                for fd in (write_fd, dir_fd, read_pipe, write_pipe):
                    os.close(fd)
        # All client file objects and the portal export are gone before payload.
        offer = json.loads(call("CreateDownloadOffer", "(s)", (draft,))[0])
        opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        request = urllib.request.Request(offer["url"] + "/api/localsend/v2/prepare-download?" +
            urllib.parse.urlencode({"pin": offer["pin"]}), data=b"", method="POST")
        with opener.open(request) as response:
            metadata = json.load(response)
        assert len(metadata["files"]) == 25, "Rejected batches must not partially append"
        for item in metadata["files"].values():
            url = offer["url"] + "/api/localsend/v2/download?" + urllib.parse.urlencode({
                "sessionId": metadata["sessionId"], "fileId": item["id"]})
            with opener.open(url) as response:
                assert response.read() == b"selected bytes"
        call("StopDownloadOffer")
        print("PASS real document-portal export/revocation, descriptor ownership, 25-file batching, invalid-descriptor rejection, atomic append and exact payload")
    finally:
        daemon.terminate()
        try:
            daemon.wait(timeout=5)
        except subprocess.TimeoutExpired:
            daemon.kill()
            daemon.wait()
