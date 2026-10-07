#!/usr/bin/env python3
"""Build exact test artifacts, then run radio-free kernel network regressions."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
with tempfile.TemporaryFile(mode="w+", encoding="utf-8") as output:
    subprocess.run([
        "cargo", "test", "--locked", "--no-run", "--message-format=json",
        "-p", "linuxdrop-netd", "-p", "linuxdrop-quickshare", "-p", "rqs_lib", "-p", "linuxdrop-daemon",
    ], cwd=ROOT, stdout=output, check=True)
    output.seek(0)
    binaries = {}
    for line in output:
        artifact = json.loads(line)
        if artifact.get("reason") != "compiler-artifact" or not artifact.get("executable"):
            continue
        if not artifact.get("profile", {}).get("test"):
            continue
        target = artifact["target"]
        binaries[(target["name"], tuple(target["kind"]))] = artifact["executable"]

cases = [
    ("crates/linuxdrop-daemon/tests/run-awdl-lifecycle.sh", ("linuxdropd", ("bin",))),
    ("crates/linuxdrop-netd/tests/run-p2p-addresses.sh", ("linuxdrop-netd", ("bin",))),
    ("crates/linuxdrop-netd/tests/run-p2p-host-network.sh", ("linuxdrop-netd", ("bin",))),
    ("crates/linuxdrop-quickshare/tests/run-lan-lifecycle.sh", ("lan_lifecycle", ("test",))),
    ("crates/linuxdrop-quickshare/tests/run-direct-upgrade.sh", ("rqs_lib", ("lib",))),
]
privilege = [] if os.geteuid() == 0 else ["sudo", "-n"]
for script, target in cases:
    executable = binaries.get(target)
    if not executable:
        raise SystemExit(f"No built test artifact for {target!r}")
    print(f"Running isolated network fixture: {script}", flush=True)
    # Each wrapper creates a fresh network namespace; each test independently
    # checks its namespace before changing a link or starting DHCP.
    subprocess.run([*privilege, "sh", str(ROOT / script), executable], cwd=ROOT, check=True, timeout=90)

# The actual AWDL executable must honor netd's readiness/retirement contract.
# Build its separately pinned workspace and locate the exact emitted artifact.
with tempfile.TemporaryFile(mode="w+", encoding="utf-8") as output:
    subprocess.run([
        "cargo", "build", "--locked", "--manifest-path", "vendor/opendrop-rs/Cargo.toml",
        "-p", "filin-rs", "--message-format=json",
    ], cwd=ROOT, stdout=output, check=True)
    output.seek(0)
    filin = None
    for line in output:
        artifact = json.loads(line)
        if artifact.get("reason") == "compiler-artifact" and artifact.get("target", {}).get("name") == "filin":
            filin = artifact.get("executable") or filin
    if not filin:
        raise SystemExit("No built filin executable")
subprocess.run([*privilege, "sh", str(ROOT / "crates/linuxdrop-netd/tests/run-awdl-link.sh"),
                binaries[("linuxdrop-netd", ("bin",))], filin], cwd=ROOT, check=True, timeout=30)
