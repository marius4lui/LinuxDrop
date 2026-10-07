#!/usr/bin/python3
"""Radio-only stand-ins bind-mounted by awdl-watchdog.py in private namespaces."""
import fcntl
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import time

assert os.readlink('/proc/self/ns/mnt') != os.readlink('/proc/1/ns/mnt')
assert os.readlink('/proc/self/ns/net') != os.readlink('/proc/1/ns/net')
assert Path('/run/linuxdrop-watchdog-fixture').read_text() == 'isolated-v1'
args = sys.argv[1:]
kernel = Path('/run/kernel-sys/class/net')
links = Path('/sys/class/net')
phy = Path('/sys/class/ieee80211/phy0')

if '-i' in args:  # Filin substitute: actual nonpersistent TAP, owned by this child.
    assert os.environ['LINUXDROP_MANAGED_LEASE'] == '1'
    assert os.environ['LINUXDROP_ALLOWED_FREQUENCIES'] == '2437'
    name = args[args.index('-h') + 1]
    fd = os.open('/dev/net/tun', os.O_RDWR)
    fcntl.ioctl(fd, 0x400454ca, struct.pack('16sH', name.encode(), 0x1002))
    subprocess.run(['/usr/sbin/ip', 'link', 'set', name, 'up'], check=True)
    subprocess.run(['/usr/sbin/ip', '-6', 'addr', 'add', 'fd42:77::1/64',
                    'dev', name, 'nodad'], check=True)
    (links / name).symlink_to(kernel / name)
    Path('/run/filin-pid').write_text(str(os.getpid()))
    print('LINUXDROP_AWDL_READY_V1', flush=True)
    while True:
        time.sleep(1)
elif args == ['phy', 'phy0', 'info']:
    restricted = ' (no IR)' if Path('/run/restrict-channel').exists() else ''
    print('Supported interface modes:\n * managed\n * monitor\nBand 1:\n'
          ' * 2437 MHz [6] (20.0 dBm)' + restricted)
elif args[:4] == ['phy', 'phy0', 'interface', 'add']:
    name = args[4]
    assert name.startswith('ld') and args[5:] == ['type', 'monitor']
    subprocess.run(['/usr/sbin/ip', 'link', 'add', name, 'type', 'dummy'], check=True)
    path = links / name
    path.mkdir()
    (path / 'phy80211').symlink_to(phy)
    for field in ('ifindex', 'ifalias', 'operstate'):
        (path / field).symlink_to(kernel / name / field)
elif args[0] == 'dev' and args[2:] == ['set', 'channel', '6']:
    pass
elif args[0] == 'dev' and args[2:] == ['del']:
    name = args[1]
    assert name.startswith('ld')
    Path('/run/delete-started').write_text(name)
    # Hold the ACK briefly to prove status and reservations while cleanup runs.
    deadline = time.monotonic() + 4
    while Path('/run/hold-delete').exists() and time.monotonic() < deadline:
        time.sleep(0.02)
    subprocess.run(['/usr/sbin/ip', 'link', 'del', name], check=True)
    shutil.rmtree(links / name)
else:
    raise AssertionError(args)
