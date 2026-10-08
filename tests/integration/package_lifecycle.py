#!/usr/bin/env python3
"""Remove/reinstall a built package only on an explicitly disposable system."""
import json
import os
from pathlib import Path
import pwd
import subprocess
import sys

if os.geteuid() != 0 or os.environ.get('LINUXDROP_TEST_DISPOSABLE_SYSTEM') != '1':
    raise SystemExit('Explicit disposable-system root opt-in is required')
marker = Path('/etc/linuxdrop-disposable-acceptance')
if not marker.is_file() or marker.read_text().strip() != 'linuxdrop-systemd-acceptance-v1':
    raise SystemExit('Disposable-system marker is missing')
if Path('/proc/1/comm').read_text().strip() != 'systemd':
    raise SystemExit('Booted systemd is required')
if len(sys.argv) != 2:
    raise SystemExit('Pass exactly one built Debian package')
package = Path(sys.argv[1]).resolve(strict=True)
if not package.is_file():
    raise SystemExit('A regular package file is required')


def run(*args, timeout=120):
    return subprocess.run(args, check=True, capture_output=True, text=True,
        timeout=timeout, env=dict(os.environ, DEBIAN_FRONTEND='noninteractive'))


assert run('dpkg-deb', '-f', str(package), 'Package').stdout.strip() == 'linuxdrop'
version = run('dpkg-deb', '-f', str(package), 'Version').stdout.strip()
assert run('dpkg-query', '-W', '-f=${Version}', 'linuxdrop').stdout == version, 'Test the installed version'
assert subprocess.run(['pgrep', '-x', 'linuxdropd'], capture_output=True).returncode == 1, 'Stop test user daemons first'
journal = Path('/var/lib/linuxdrop-netd/leases.json')
assert json.loads(journal.read_text()) == [], 'Never remove a package with a reserved radio'
original_journal = journal.read_bytes()
original_uid = pwd.getpwnam('linuxdrop-netd').pw_uid
executables = [Path(path) for path in ('/usr/bin/linuxdrop', '/usr/bin/linuxdropd',
    '/usr/libexec/linuxdrop/linuxdrop-netd', '/usr/libexec/linuxdrop/filin')]
unit = Path('/usr/lib/systemd/system/linuxdrop-netd.service')
checks = []
removed = False
try:
    removed = True  # Any partial removal must also be repaired in finally.
    run('apt-get', 'remove', '-y', 'linuxdrop')
    assert all(not path.exists() for path in executables) and not unit.exists()
    assert subprocess.run(['systemctl', 'is-active', '--quiet', 'linuxdrop-netd.service']).returncode != 0
    assert not Path('/run/linuxdrop/netd.sock').exists()
    checks.append('package removal stops the helper and removes executables, unit and socket')
    assert journal.read_bytes() == original_journal
    assert pwd.getpwnam('linuxdrop-netd').pw_uid == original_uid
    checks.append('ordinary removal preserves the recovery journal and dedicated service identity')
finally:
    if removed:
        run('apt-get', 'install', '-y', '--no-install-recommends', str(package))

assert run('dpkg-query', '-W', '-f=${Version}', 'linuxdrop').stdout == version
assert all(path.is_file() for path in executables) and unit.is_file()
assert pwd.getpwnam('linuxdrop-netd').pw_uid == original_uid
assert json.loads(journal.read_text()) == []
checks.append('reinstallation restores the same version without changing service UID or losing journal data')
probe = Path(__file__).with_name('netd_systemd.py')
service = json.loads(run(sys.executable, str(probe)).stdout)
assert service['status'] == 'passed'
checks.append('reinstalled package passes the actual systemd identity, restart and journal recovery probe')
print(json.dumps({'status': 'passed', 'package': version, 'checks': checks,
    'open': ['physical-radio removal during a transfer', 'other distribution package-manager lifecycles']}, indent=2))
