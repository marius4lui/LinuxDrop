#!/usr/bin/python3
"""udhcpc hook restricted to netd's newly created, ownership-marked P2P VIF."""
import ipaddress
import json
import os
from pathlib import Path
import re
import subprocess
import sys


def validated_lease(values):
    interface = values.get("interface", "")
    if not re.fullmatch(r"[A-Za-z0-9_-]{1,15}", interface) or interface != values.get("LINUXDROP_P2P_INTERFACE"):
        raise ValueError("Unexpected DHCP interface")
    token = values.get("LINUXDROP_LEASE_ID", "")
    if not re.fullmatch(r"[a-f0-9]{32}", token):
        raise ValueError("Invalid lease identity")
    address = ipaddress.IPv4Address(values.get("ip", ""))
    if address.is_unspecified or address.is_multicast or address.is_loopback:
        raise ValueError("Invalid DHCP address")
    network = ipaddress.IPv4Network(f"{address}/{values.get('subnet', '')}", strict=False)
    if network.prefixlen < 8 or address in (network.network_address, network.broadcast_address):
        raise ValueError("Unsafe DHCP subnet")
    return interface, token, str(address), network.prefixlen


def main():
    if len(sys.argv) != 2 or sys.argv[1] not in ("bound", "renew"):
        return
    interface, token, address, prefix = validated_lease(os.environ)
    alias = Path(f"/sys/class/net/{interface}/ifalias").read_text().strip()
    if alias != f"linuxdrop:p2p:{token}":
        raise ValueError("P2P interface ownership changed")
    # A kernel-connected route is needed to reach the group owner. Never install
    # offered routes, DNS servers, hostnames, MTU or a default route from DHCP.
    subprocess.run(["/usr/sbin/ip", "-4", "address", "replace", f"{address}/{prefix}", "dev", interface], check=True, timeout=5)
    result = Path(f"/run/linuxdrop/{token}.ipv4.json")
    temporary = result.with_suffix(".tmp")
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, "w") as stream:
        json.dump({"interface": interface, "ipv4_address": address}, stream)
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, result)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.SubprocessError):
        # Do not echo network-controlled DHCP fields into privileged logs.
        sys.exit("LinuxDrop: rejected P2P DHCP configuration")
