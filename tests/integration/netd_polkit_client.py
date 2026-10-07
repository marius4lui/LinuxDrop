#!/usr/bin/env python3
"""Client-side real Polkit probe, run in a disposable PAM/logind session."""
import json
import os
import select
import socket
import subprocess
import sys
import shlex
import tempfile
from pathlib import Path

agent = None
daemon_mode = '--daemon' in sys.argv
daemon_started = False
settings_path = Path.home() / '.config/linuxdrop/settings.json'
original_settings = None
settings_existed = False
stage = 'session lookup'
try:
    # Resolve the calling PID through real logind rather than trusting its env.
    session_path = shlex.split(subprocess.check_output(['busctl', 'call',
        'org.freedesktop.login1', '/org/freedesktop/login1', 'org.freedesktop.login1.Manager',
        'GetSessionByPID', 'u', str(os.getpid())], text=True))[1]
    session = shlex.split(subprocess.check_output(['busctl', 'get-property',
        'org.freedesktop.login1', session_path, 'org.freedesktop.login1.Session', 'Id'], text=True))[1]
    metadata = subprocess.check_output(['loginctl', 'show-session', session,
        '-p', 'Active', '-p', 'Remote', '-p', 'Seat', '-p', 'VTNr', '-p', 'Leader', '-p', 'User'], text=True)
    details = dict(line.split('=', 1) for line in metadata.splitlines())
    assert details['User'] == str(os.getuid()), 'Client must belong to its actual PAM session'
    print('LINUXDROP_AUTH_SESSION:' + json.dumps(details), flush=True)
    if '--hold' in sys.argv:
        input()
        sys.exit(0)
    subprocess.run(['pkcheck', '--revoke-temp'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    subject_pid = os.getpid()
    if daemon_mode:
        stage = 'user service preparation'
        active = subprocess.run(['systemctl', '--user', 'is-active', 'linuxdropd.service'], capture_output=True, text=True)
        assert active.stdout.strip() != 'active', 'Do not replace an existing user daemon'
        settings_existed = settings_path.exists()
        if settings_existed:
            original_settings = settings_path.read_bytes()
        settings_path.parent.mkdir(parents=True, exist_ok=True)
        settings_path.write_text(json.dumps({
            'localsend': {'enabled': False}, 'quickshare': {'enabled': False},
            'airdrop': {'enabled': False}, 'hardware': {'auto_use_usb': False}}))
        daemon_started = True
        stage = 'user service start'
        subprocess.run(['systemctl', '--user', 'start', 'linuxdropd.service'], check=True, capture_output=True, timeout=30)
        stage = 'user service identity'
        subject_pid = int(subprocess.check_output(['systemctl', '--user', 'show', 'linuxdropd.service', '--property=MainPID', '--value'], text=True))
        assert subject_pid > 0 and subject_pid != os.getpid()
        cgroup = Path(f'/proc/{subject_pid}/cgroup').read_text()
        assert 'user@' in cgroup and 'linuxdropd.service' in cgroup and 'session-' not in cgroup, 'Daemon must run as a real user service'
        assert Path(f'/proc/{subject_pid}').stat().st_uid == os.getuid()
        status = dict(line.split(':', 1) for line in Path(f'/proc/{subject_pid}/status').read_text().splitlines())
        assert all(int(status[field].strip(), 16) == 0 for field in ('CapEff', 'CapPrm', 'CapAmb'))
        assert status['NoNewPrivs'].strip() == '1' and status['Seccomp'].strip() == '2'
        stage = 'desktop temporary-file handoff'
        with tempfile.TemporaryDirectory(prefix='linuxdrop-send-', dir='/tmp') as folder:
            source = Path(folder) / 'selected file.txt'
            source.write_text('File selected by the desktop user.\n')
            prepared = subprocess.check_output(['busctl', '--user', '--timeout=10', 'call',
                'io.github.marius4lui.LinuxDrop', '/io/github/marius4lui/LinuxDrop',
                'io.github.marius4lui.LinuxDrop.Manager1', 'PrepareSend', 'as', '1', str(source)], text=True, timeout=15)
            signature, draft = shlex.split(prepared)
            assert signature == 's' and draft
            subprocess.run(['busctl', '--user', '--timeout=10', 'call',
                'io.github.marius4lui.LinuxDrop', '/io/github/marius4lui/LinuxDrop',
                'io.github.marius4lui.LinuxDrop.Manager1', 'DiscardDraft', 's', draft],
                check=True, capture_output=True, timeout=15)
    if '--agent' in sys.argv:
        stage = 'agent registration'
        read_fd, write_fd = os.pipe()
        agent = subprocess.Popen(['pkttyagent', '--process', str(subject_pid), '--notify-fd', str(write_fd)], pass_fds=(write_fd,))
        os.close(write_fd)
        if not select.select([read_fd], [], [], 10)[0]:
            raise RuntimeError('Agent registration timed out')
        os.read(read_fd, 1)
        os.close(read_fd)
        if agent.poll() is not None:
            raise RuntimeError('Agent could not register')
    print('LINUXDROP_AUTH_CLIENT_READY', flush=True)
    stage = 'authorized diagnostic' if daemon_mode else 'authorized reserve'
    if daemon_mode:
        called = subprocess.run(['busctl', '--user', '--timeout=90', 'call',
            'io.github.marius4lui.LinuxDrop', '/io/github/marius4lui/LinuxDrop',
            'io.github.marius4lui.LinuxDrop.Manager1', 'RunHardwareDiagnostic', 'sq',
            'acceptance-no-radio', '6'], capture_output=True, text=True, timeout=95)
        if called.returncode != 0:
            result = {'status': 'error', 'message': called.stderr.strip()}
        else:
            signature, payload = shlex.split(called.stdout)
            assert signature == 's'
            report = json.loads(payload)
            assert report['radio_id'] == 'acceptance-no-radio' and report['transmitted_frames'] == 0
            assert report['restored'] and len(report['steps']) == 1
            result = {'status': 'error', 'message': report['steps'][0]['detail'], 'via_user_service': True}
    else:
        with socket.socket(socket.AF_UNIX) as client:
            client.settimeout(65)
            client.connect('/run/linuxdrop/netd.sock')
            client.sendall(b'{"operation":"reserve","radio_id":"acceptance-no-radio"}\n')
            response = bytearray()
            while not response.endswith(b'\n'):
                chunk = client.recv(4096)
                if not chunk:
                    raise RuntimeError('Helper closed before authorization result')
                response.extend(chunk)
                if len(response) > 65536:
                    raise RuntimeError('Oversized helper response')
        result = json.loads(response)
    print('LINUXDROP_AUTH_RESULT:' + json.dumps(result), flush=True)
except Exception as error:
    # Deliberately omit exception text/tracebacks and PTY buffers: only fixed
    # stages, exception class and systemd's non-secret result properties escape.
    diagnostic = {'stage': stage, 'exception': type(error).__name__}
    if daemon_started:
        diagnostic['unit'] = subprocess.run(['systemctl', '--user', 'show',
            'linuxdropd.service', '-p', 'Result', '-p', 'ExecMainStatus', '-p', 'SubState'],
            capture_output=True, text=True, timeout=5).stdout.strip()
    print('LINUXDROP_AUTH_RESULT:' + json.dumps({'status': 'probe_failed', 'message': diagnostic}), flush=True)
    sys.exit(1)
finally:
    if agent is not None:
        agent.terminate()
        try: agent.wait(timeout=2)
        except subprocess.TimeoutExpired:
            agent.kill(); agent.wait()
    if daemon_started:
        subprocess.run(['systemctl', '--user', 'stop', 'linuxdropd.service'], check=False, capture_output=True, timeout=20)
        if settings_existed:
            settings_path.write_bytes(original_settings)
        else:
            settings_path.unlink(missing_ok=True)
