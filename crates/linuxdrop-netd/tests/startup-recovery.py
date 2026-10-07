"""Real helper startup/journal/shutdown probes in private mount + net namespaces."""
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import time

for namespace in ("mnt", "net"):
    assert os.readlink(f"/proc/self/ns/{namespace}") != os.readlink(f"/proc/1/ns/{namespace}"), "Private namespaces required"
subprocess.run(["mount", "-t", "tmpfs", "-o", "mode=0755", "linuxdrop-test-run", "/run"], check=True)
subprocess.run(["mount", "-t", "tmpfs", "-o", "mode=0755", "linuxdrop-test-state", "/var/lib"], check=True)
# Rebind sysfs to this network namespace so helper interface checks observe
# the private dummy link rather than the host network namespace.
subprocess.run(["mount", "-t", "sysfs", "-o", "ro", "sysfs", "/sys"], check=True)
Path("/run/linuxdrop").mkdir()
Path("/var/lib/linuxdrop-netd").mkdir(mode=0o700)
journal = Path("/var/lib/linuxdrop-netd/leases.json")
socket_path = "/run/linuxdrop/netd.sock"
binary = sys.argv[1]
env = dict(os.environ, DBUS_SYSTEM_BUS_ADDRESS="unix:path=/nonexistent")
lease = {
    "id": "a" * 32, "uid": os.getuid(), "phy": "phy-test", "interface": "ld-recovery0",
    "channel": 6, "boot_id": "synthetic-previous-boot", "awdl_interface": None,
    "allowed_frequencies": [], "kind": "monitor", "connection_uuid": None, "p2p_group": None,
}

def request(operation):
    with socket.socket(socket.AF_UNIX) as client:
        client.settimeout(1)
        client.connect(socket_path)
        client.sendall(json.dumps({"operation": operation}).encode() + b"\n")
        return json.loads(client.makefile("rb").readline())


def wait_for(check, process):
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        assert process.poll() is None, "Helper stopped before its startup check"
        try:
            if check():
                return
        except (FileNotFoundError, ConnectionRefusedError):
            pass
        time.sleep(0.02)
    raise AssertionError("Helper startup/recovery check timed out")


def run_helper(check):
    process = subprocess.Popen([binary], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        wait_for(lambda: request("status")["status"] == "state", process)
        check(process)
        process.terminate()
        stdout, stderr = process.communicate(timeout=5)
        assert process.returncode == 0, (stdout, stderr)
        assert not Path(socket_path).exists(), "Normal stop must remove the socket"
    finally:
        if process.poll() is None:
            process.kill()
            process.communicate(timeout=5)


journal.write_text(json.dumps([lease]))
run_helper(lambda process: wait_for(lambda: json.loads(journal.read_text()) == [], process))
assert json.loads(journal.read_text()) == []

# A current-boot interface without the helper's marker must survive recovery.
# It is a dummy in the private namespace, not a WLAN device or live connection.
subprocess.run(["/usr/sbin/ip", "link", "add", "ld-recovery0", "type", "dummy"], check=True)
assert Path("/sys/class/net/ld-recovery0").exists()
lease["boot_id"] = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
journal.write_text(json.dumps([lease]))

def failed_recovery(process):
    wait_for(lambda: bool(request("recovery_status")["recent_errors"]), process)
    assert request("status")["leases"] == [], "Unrestored radio cannot appear healthy"
    recovery = request("recovery_status")
    assert len(recovery["issues"]) == 1
    assert recovery["issues"][0]["ownership_verified"] is False
    assert json.loads(journal.read_text())[0]["id"] == lease["id"]
    subprocess.run(["/usr/sbin/ip", "link", "show", "dev", "ld-recovery0"], stdout=subprocess.DEVNULL, check=True)

run_helper(failed_recovery)
assert json.loads(journal.read_text())[0]["id"] == lease["id"]
# Restart sees and retains the failed reservation too.
run_helper(failed_recovery)

# A durable formation intent without any group identity remains reserved across
# actual helper process restarts, even when its parent link has no owner marker.
pending = dict(lease, kind="direct_wifi", p2p_pending=True, connection_uuid=None)
journal.write_text(json.dumps([pending]))

def uncertain_formation(process):
    wait_for(lambda: any("outcome is unknown" in error for error in
        request("recovery_status")["recent_errors"]), process)
    assert request("status")["leases"] == []
    recovery = request("recovery_status")
    assert len(recovery["issues"]) == 1
    assert not recovery["issues"][0]["ownership_verified"]
    assert "outcome is unknown" in recovery["issues"][0]["detail"]
    assert json.loads(journal.read_text())[0]["p2p_pending"] is True

run_helper(uncertain_formation)
run_helper(uncertain_formation)
# Simulated old-boot journal: no live formation can cross a kernel reboot.
pending["boot_id"] = "synthetic-previous-boot"
journal.write_text(json.dumps([pending]))
run_helper(lambda process: wait_for(lambda: json.loads(journal.read_text()) == [], process))

for malformed in [b"broken journal", json.dumps([lease, lease]).encode()]:
    journal.write_bytes(malformed)
    result = subprocess.run([binary], env=env, capture_output=True, timeout=5)
    assert result.returncode != 0, "Invalid recovery journal must fail closed"
    assert journal.read_bytes() == malformed, "Invalid journal must not be overwritten"

journal.unlink()
journal.mkdir()
result = subprocess.run([binary], env=env, capture_output=True, timeout=5)
assert result.returncode != 0 and journal.is_dir(), "An unreadable journal must fail closed"
print("LINUXDROP_NETD_STARTUP_RECOVERY_PASSED")
