#!/usr/bin/env python3
"""Installed netd lifecycle, only inside an explicitly disposable systemd OS."""
import json
import os
from pathlib import Path
import pwd
import socket
import stat
import subprocess
import sys
import time

MARKER = Path('/etc/linuxdrop-disposable-acceptance')
if os.geteuid() != 0 or Path('/proc/1/comm').read_text().strip() != 'systemd':
    raise SystemExit('A disposable booted systemd OS is required')
if os.environ.get('LINUXDROP_TEST_DISPOSABLE_SYSTEM') != '1' or not MARKER.is_file():
    raise SystemExit('Explicit disposable-system opt-in and marker are required')
if MARKER.read_text().strip() != 'linuxdrop-systemd-acceptance-v1':
    raise SystemExit('Unexpected disposable-system marker')

UNIT = 'linuxdrop-netd.service'
SOCKET = Path('/run/linuxdrop/netd.sock')
JOURNAL = Path('/var/lib/linuxdrop-netd/leases.json')
checks = []

def run(*args, check=True):
    return subprocess.run(args, check=check, text=True, capture_output=True, timeout=20)

def property_value(name):
    return run('systemctl', 'show', UNIT, '--property=' + name, '--value').stdout.strip()

def request(payload):
    with socket.socket(socket.AF_UNIX) as client:
        client.settimeout(4)
        client.connect(str(SOCKET))
        client.sendall(json.dumps(payload).encode() + b'\n')
        data = bytearray()
        while not data.endswith(b'\n'):
            chunk = client.recv(4096)
            if not chunk:
                raise AssertionError('Helper closed before its response')
            data.extend(chunk)
            if len(data) > 65536:
                raise AssertionError('Unexpected response size')
        return json.loads(data)

def ready():
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        try:
            if property_value('ActiveState') == 'active':
                response = request({'operation': 'status'})
                assert response['status'] == 'state'
                return response
        except (OSError, subprocess.CalledProcessError):
            pass
        time.sleep(.05)
    raise AssertionError('Installed helper did not become ready')

run('systemctl', 'start', UNIT)
state = ready()
assert state['leases'] == [], 'Never mutate a journal containing live leases'
assert json.loads(JOURNAL.read_text()) == [], 'Only an empty disposable journal may be tested'
service_user = pwd.getpwnam('linuxdrop-netd')
pid = int(property_value('MainPID'))
process = dict(line.split(':', 1) for line in Path(f'/proc/{pid}/status').read_text().splitlines() if ':' in line)
assert set(process['Uid'].split()) == {str(service_user.pw_uid)}
assert service_user.pw_uid != 0
assert process['NoNewPrivs'].strip() == '1'
expected_caps = (1 << 10) | (1 << 12) | (1 << 13)
assert int(process['CapEff'], 16) == expected_caps
assert int(process['CapBnd'], 16) == expected_caps
assert stat.S_IMODE(JOURNAL.stat().st_mode) == 0o600
assert JOURNAL.stat().st_uid == service_user.pw_uid
assert stat.S_IMODE(JOURNAL.parent.stat().st_mode) == 0o700
assert stat.S_IMODE(SOCKET.stat().st_mode) == 0o666
checks.append('real systemd unit: service UID, capabilities, no-new-privileges and journal/socket modes')

username = 'linuxdrop-acceptance-reader'
try:
    pwd.getpwnam(username)
except KeyError:
    run('useradd', '--system', '--no-create-home', '--shell', '/usr/sbin/nologin', username)
# Executed as an actual unprivileged process over the installed Unix socket.
client_source = '''import json,socket,sys
with socket.socket(socket.AF_UNIX) as client:
 client.settimeout(4); client.connect('/run/linuxdrop/netd.sock')
 client.sendall(json.dumps(json.loads(sys.argv[1])).encode()+b'\\n')
 print(client.recv(65536).decode())
'''
for payload, expected in [({'operation': 'status'}, 'state'), ({'operation': 'reserve', 'radio_id': 'acceptance-no-radio'}, 'error')]:
    response = json.loads(run('runuser', '-u', username, '--', 'python3', '-c', client_source, json.dumps(payload)).stdout)
    assert response['status'] == expected, response
    if expected == 'error':
        assert 'local desktop session' in response['message'].lower(), response
checks.append('unprivileged status allowed; radio mutation without a local logind session denied')

before = int(property_value('MainPID'))
run('systemctl', 'restart', UNIT)
ready()
assert int(property_value('MainPID')) != before
run('systemctl', 'stop', UNIT)
assert property_value('ActiveState') == 'inactive'
assert not SOCKET.exists()
assert json.loads(JOURNAL.read_text()) == []
run('systemctl', 'start', UNIT)
ready()
checks.append('real service restart, orderly stop/socket removal, retained journal and start')

# Corruption must fail startup and leave the evidence intact. Restore in finally,
# so a failed assertion cannot leave even this disposable service in a bad state.
run('systemctl', 'stop', UNIT)
original = JOURNAL.read_bytes()
try:
    JOURNAL.write_bytes(b'{invalid-journal')
    run('systemctl', 'start', UNIT, check=False)
    deadline = time.monotonic() + 5
    failed = False
    while time.monotonic() < deadline:
        if property_value('ExecMainStatus') != '0':
            failed = True
            break
        time.sleep(.05)
    assert failed, 'Corrupted journal did not fail the installed service'
    assert JOURNAL.read_bytes() == b'{invalid-journal'
finally:
    run('systemctl', 'stop', UNIT, check=False)
    JOURNAL.write_bytes(original)
    run('systemctl', 'reset-failed', UNIT)
    run('systemctl', 'start', UNIT)
    ready()
checks.append('malformed journal fails real service startup without overwriting it; restored journal starts')

print(json.dumps({'status': 'passed', 'checks': checks, 'package': run('dpkg-query', '-W', '-f=${Version}', 'linuxdrop').stdout,
    'open': ['interactive Polkit authorization and active/remote session matrix', 'real radios and desktop integration']}, indent=2))
