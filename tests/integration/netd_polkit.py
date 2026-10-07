#!/usr/bin/env python3
"""Real Polkit/PAM challenges in an explicitly disposable booted Ubuntu OS.
Requires python3-pexpect. Synthetic passwords stay in memory/PTY and are never
included in logs, arguments or result reports; the test account is locked last.
"""
import json
import os
from pathlib import Path
import pwd
import re
import secrets
import subprocess
import pexpect

marker = Path('/etc/linuxdrop-disposable-acceptance')
if os.geteuid() != 0 or os.environ.get('LINUXDROP_TEST_DISPOSABLE_SYSTEM') != '1':
    raise SystemExit('Explicit disposable-system root opt-in is required')
if not marker.is_file() or marker.read_text().strip() != 'linuxdrop-systemd-acceptance-v1':
    raise SystemExit('Disposable-system marker is missing')
if Path('/proc/1/comm').read_text().strip() != 'systemd':
    raise SystemExit('Booted systemd is required')

username = 'linuxdrop-auth-test'
try:
    pwd.getpwnam(username)
except KeyError:
    subprocess.run(['useradd', '-m', '-G', 'sudo', '-s', '/bin/bash', username], check=True, capture_output=True)
client_path = str(Path(__file__).with_name('netd_polkit_client.py').resolve())
checks = []
active_console = Path('/sys/class/tty/tty0/active').read_text().strip()
console_match = re.fullmatch(r'tty(\d+)', active_console)
if not console_match:
    raise SystemExit('A real virtual console seat is required for the Polkit matrix')
active_vt = int(console_match.group(1))
inactive_vt = 2 if active_vt != 2 else 3
password = secrets.token_hex(16)
subprocess.run(['chpasswd'], input=username+':'+password+'\n', text=True, check=True, capture_output=True)

def scenario(name, seat, vt, agent, wrong=False):
    unit = 'linuxdrop-polkit-' + name + '-' + str(os.getpid())
    args = ['--unit='+unit, '--quiet', '--collect', '--wait', '--pty', '--uid='+username,
        '--property=PAMName=login', '--setenv=LC_ALL=C']
    if seat:
        args.extend(['--setenv=XDG_SEAT='+seat, '--setenv=XDG_VTNR='+str(vt)])
    args.extend(['/usr/bin/python3', '-u', client_path])
    if agent:
        args.append('--agent')
    child = pexpect.spawn('systemd-run', args, encoding='utf-8', timeout=70, echo=False)
    prompts, session, result = 0, None, None
    try:
        while True:
            event = child.expect([r'LINUXDROP_AUTH_SESSION:([^\r\n]+)',
                r'Choose identity to authenticate as[^:]*:', r'Password:',
                r'LINUXDROP_AUTH_RESULT:([^\r\n]+)', pexpect.EOF, pexpect.TIMEOUT])
            if event == 0:
                session = json.loads(child.match.group(1))
            elif event == 1:
                choices = re.findall(r'(?m)^\s*(\d+)\.([^\r\n]+)', child.before)
                choice = [number for number, text in choices if username in text]
                if len(choice) != 1:
                    raise RuntimeError('Test administrator not uniquely listed')
                child.sendline(choice[0])
            elif event == 2:
                prompts += 1
                if prompts > 3 or not agent:
                    raise RuntimeError('Unexpected authentication challenge count')
                child.sendline('deliberately-invalid-test-password' if wrong else password)
            elif event == 3:
                result = json.loads(child.match.group(1))
                break
            else:
                raise RuntimeError('Authentication probe ended without a result; terminal contents suppressed')
        if child.expect([pexpect.EOF, pexpect.TIMEOUT]) != 0:
            raise RuntimeError('Authentication client did not terminate')
        child.close()
        assert child.exitstatus == 0, 'Authentication client failed'
        assert session is not None and session['Remote'] == 'no', 'Missing real PAM/logind session'
        assert result['status'] == 'error', 'Nonexistent test radio must not create a lease'
        assert 'Only trusted callers' not in result['message'], 'Mechanism is not the Polkit action owner'
        return prompts, session, result['message']
    finally:
        if child.isalive():
            child.terminate(force=True)
        subprocess.run(['systemctl', 'stop', unit+'.service'], capture_output=True, check=False)

try:
    prompts, session, message = scenario('inactive', 'seat0', inactive_vt, True)
    assert session['Active'] == 'no' and prompts == 0 and 'local desktop session' in message.lower()
    checks.append('inactive seat session rejected before authentication')
    prompts, session, message = scenario('seatless', None, 0, True)
    assert session['Seat'] == '' and prompts == 0 and message != 'radio not found'
    checks.append('seatless PAM session denied by the unchanged Polkit policy')
    prompts, session, message = scenario('noagent', 'seat0', active_vt, False)
    assert session['Active'] == 'yes' and prompts == 0 and message != 'radio not found'
    checks.append('active local session without an authentication agent denied')
    prompts, session, message = scenario('wrongpassword', 'seat0', active_vt, True, wrong=True)
    assert prompts > 0 and message != 'radio not found'
    checks.append('real PAM administrator challenge rejects an incorrect password')
    prompts, session, message = scenario('authorized', 'seat0', active_vt, True)
    assert session['Active'] == 'yes' and session['Seat'] == 'seat0' and prompts > 0
    assert message == 'radio not found', 'Successful authentication did not reach the permitted operation'
    checks.append('correct administrator authentication reaches radio selection through the non-root installed helper')
    print(json.dumps({'status': 'passed', 'checks': checks,
        'open': ['graphical authentication agent and actual user-service invocation', 'remote session matrix', 'physical adapter mutation']}, indent=2))
finally:
    subprocess.run(['passwd', '-l', username], check=True, capture_output=True)
