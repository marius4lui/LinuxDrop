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
import shlex
import shutil
import subprocess
import sys
import tempfile
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
# The dedicated user cannot necessarily traverse a CI runner's checkout/home.
# Copy only this non-secret probe into a root-owned readable temporary directory.
probe_dir = tempfile.TemporaryDirectory(prefix='linuxdrop-auth-')
os.chmod(probe_dir.name, 0o755)
client_file = Path(probe_dir.name) / 'client.py'
client_file.write_bytes(Path(__file__).with_name('netd_polkit_client.py').read_bytes())
client_file.chmod(0o644)
client_path = str(client_file)
graphical = '--graphical' in sys.argv
if graphical:
    for filename in ('gnome_polkit_driver.py', 'gnome_polkit.js'):
        target = Path(probe_dir.name) / filename
        target.write_bytes(Path(__file__).with_name(filename).read_bytes())
        target.chmod(0o644)
    captures = Path(probe_dir.name) / 'captures'
    captures.mkdir(mode=0o700)
    account = pwd.getpwnam(username)
    os.chown(captures, account.pw_uid, account.pw_gid)
    capture_destination = Path(os.environ.get('LINUXDROP_GUI_CAPTURE_DIR', 'dist/polkit-gnome')).resolve()
    capture_destination.mkdir(parents=True, exist_ok=True)
checks = []
active_console = Path('/sys/class/tty/tty0/active').read_text().strip()
console_match = re.fullmatch(r'tty(\d+)', active_console)
if not console_match:
    raise SystemExit('A real virtual console seat is required for the Polkit matrix')
active_vt = int(console_match.group(1))
inactive_vt = 2 if active_vt != 2 else 3
password = secrets.token_hex(16)
subprocess.run(['chpasswd'], input=username+':'+password+'\n', text=True, check=True, capture_output=True)

def scenario(name, seat, vt, agent, wrong=False, daemon=False, remote=False, gui=None):
    unit = 'linuxdrop-polkit-' + name + '-' + str(os.getpid())
    args = ['--unit='+unit, '--quiet', '--collect', '--wait', '--pty', '--setenv=LC_ALL=C']
    if not remote:
        args.extend(['--uid='+username, '--property=PAMName=login'])
    if seat:
        args.extend(['--setenv=XDG_SEAT='+seat, '--setenv=XDG_VTNR='+str(vt)])
    if daemon:
        # Real logind graphical-session metadata enables Polkit's documented
        # user-service-to-display-session association; no GUI is simulated.
        args.append('--setenv=XDG_SESSION_TYPE=wayland')
    if gui:
        args.append('--setenv=LINUXDROP_GUI_LANGUAGE='+os.environ.get('LINUXDROP_GUI_LANGUAGE', 'C.UTF-8'))
    client_args = ['/usr/bin/python3', '-u', client_path]
    if daemon:
        client_args.append('--daemon')
    if agent:
        client_args.append('--agent')
    if gui:
        client_args.extend(['--gui', gui, '--capture', str(captures / (name+'.png'))])
    args.extend(['/bin/login', '-f', '-h', '127.0.0.1', username] if remote else client_args)
    child = pexpect.spawn('systemd-run', args, encoding='utf-8', timeout=70, echo=False)
    prompts, session, result = 0, None, None
    try:
        if remote:
            if child.expect([r'[$#] ', pexpect.EOF, pexpect.TIMEOUT]) != 0:
                raise RuntimeError('Remote PAM login did not start')
            child.sendline('exec ' + shlex.join(client_args))
        while True:
            event = child.expect([r'LINUXDROP_AUTH_SESSION:([^\r\n]+)',
                r'Choose identity to authenticate as[^:]*:', r'Password:',
                r'LINUXDROP_AUTH_RESULT:([^\r\n]+)', r'LINUXDROP_GUI_PASSWORD:', pexpect.EOF, pexpect.TIMEOUT])
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
                if result.get('status') == 'probe_failed':
                    raise RuntimeError(f'{name}: client failed: {result["message"]}')
                break
            elif event == 4:
                assert gui, 'Unexpected graphical probe credential request'
                child.sendline(password)
            else:
                # Session metadata and systemd states are safe; never expose PTY
                # buffers, which can contain entered authentication credentials.
                state = subprocess.check_output(['systemctl', 'show', unit+'.service',
                    '-p', 'ActiveState', '-p', 'SubState', '-p', 'Result', '-p', 'ExecMainStatus'], text=True)
                raise RuntimeError(f'{name}: probe ended without result; session={session}, prompts={prompts}; {state}')
        if child.expect([pexpect.EOF, pexpect.TIMEOUT]) != 0:
            raise RuntimeError('Authentication client did not terminate')
        child.close()
        assert child.exitstatus == 0, 'Authentication client failed'
        assert session is not None and session['Remote'] == ('yes' if remote else 'no'), 'Missing real PAM/logind session'
        assert result['status'] == 'error', 'Nonexistent test radio must not create a lease'
        assert 'Only trusted callers' not in result['message'], 'Mechanism is not the Polkit action owner'
        if daemon and result['message'] == 'radio not found':
            assert result.get('via_user_service') is True
        if gui:
            shutil.copyfile(captures / (name+'.png'), capture_destination / (name+'.png'))
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
    prompts, session, message = scenario('user-service', 'seat0', active_vt, True, daemon=True)
    assert prompts > 0 and message == 'radio not found', 'Installed user-service authorization failed'
    checks.append('installed linuxdropd user service passes D-Bus diagnostic through real Polkit authentication')
    checks.append('user service has no effective/permitted/ambient capabilities, retains seccomp/no-new-privileges and opens desktop-selected temporary files')
    prompts, session, message = scenario('remote', None, 0, True, remote=True)
    assert prompts == 0 and 'local desktop session' in message.lower()
    checks.append('real remote PAM login rejected before authentication')
    hold_unit = 'linuxdrop-polkit-local-hold-' + str(os.getpid())
    hold = pexpect.spawn('systemd-run', ['--unit='+hold_unit, '--quiet', '--collect', '--wait', '--pty',
        '--uid='+username, '--property=PAMName=login', '--setenv=LC_ALL=C',
        '--setenv=XDG_SEAT=seat0', '--setenv=XDG_VTNR='+str(active_vt),
        '/usr/bin/python3', '-u', client_path, '--hold'], encoding='utf-8', timeout=20, echo=False)
    try:
        hold.expect(r'LINUXDROP_AUTH_SESSION:([^\r\n]+)')
        local = json.loads(hold.match.group(1))
        assert local['Active'] == 'yes' and local['Remote'] == 'no' and local['Seat'] == 'seat0'
        prompts, session, message = scenario('remote-with-local', None, 0, True, remote=True)
        assert prompts == 0 and message != 'radio not found'
        checks.append('remote caller cannot borrow an active local session of the same user')
    finally:
        subprocess.run(['systemctl', 'stop', hold_unit+'.service'], capture_output=True, check=False)
        if hold.isalive():
            hold.terminate(force=True)
    if graphical:
        prompts, session, message = scenario('gui-cancel', 'seat0', active_vt, False, daemon=True, gui='cancel')
        assert prompts == 0 and message != 'radio not found'
        checks.append('real GNOME dialog is focused and masked; Cancel denies the installed daemon diagnostic')
        prompts, session, message = scenario('gui-authenticate', 'seat0', active_vt, False, daemon=True, gui='authenticate')
        assert prompts == 0 and message == 'radio not found'
        checks.append('real GNOME password dialog authenticates the installed daemon and renders before submission')
    print(json.dumps({'status': 'passed', 'checks': checks,
        'open': ([] if graphical else ['graphical authentication agent/rendered dialog']) + ['physical adapter mutation']}, indent=2))
finally:
    subprocess.run(['passwd', '-l', username], check=True, capture_output=True)
    probe_dir.cleanup()
