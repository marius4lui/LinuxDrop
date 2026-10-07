"""Actual netd IPC/watchdog/retirement with simulated radio and real child/TAP.

No physical radio or authorization acceptance is claimed. Only private mounts,
network namespace, session bus, logind fixture and executable substitutions.
"""
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import subprocess
import sys
import time

for namespace in ('mnt', 'net'):
    assert os.readlink(f'/proc/self/ns/{namespace}') != os.readlink(f'/proc/1/ns/{namespace}')
assert os.environ['DBUS_SESSION_BUS_ADDRESS'].startswith('unix:')
os.environ['DBUS_SYSTEM_BUS_ADDRESS'] = os.environ['DBUS_SESSION_BUS_ADDRESS']
fixture = Path(__file__).with_name('awdl-radio-fixture.py').resolve()
binary = Path(sys.argv[1]).resolve(strict=True)
daemon_binary = Path(sys.argv[2]).resolve(strict=True)
for path in ('/run', '/var/lib'):
    subprocess.run(['mount', '-t', 'tmpfs', '-o', 'mode=0755', 'linuxdrop-watchdog', path], check=True)
Path('/run/kernel-sys').mkdir()
subprocess.run(['mount', '-t', 'sysfs', '-o', 'ro', 'sysfs', '/run/kernel-sys'], check=True)
subprocess.run(['mount', '-t', 'tmpfs', '-o', 'mode=0755', 'linuxdrop-radio', '/sys'], check=True)
for path in ('/run/linuxdrop', '/var/lib/linuxdrop-netd', '/sys/class/net',
             '/sys/class/ieee80211/phy0/device', '/sys/class/ieee80211/phy0/rfkill0',
             '/sys/drivers/fixture'):
    Path(path).mkdir(parents=True, exist_ok=True)
Path('/run/linuxdrop-watchdog-fixture').write_text('isolated-v1')
phy = Path('/sys/class/ieee80211/phy0')
(phy / 'device/driver').symlink_to('/sys/drivers/fixture')
(phy / 'rfkill0/soft').write_text('0')
radio_id = 'platform:unknown:unknown:/sys/class/ieee80211/phy0/device'
journal = Path('/var/lib/linuxdrop-netd/leases.json')
links = Path('/sys/class/net')
kernel = Path('/run/kernel-sys/class/net')
# Private overmounts never modify the installed iw, pkcheck or Filin binaries.
shutil.copyfile(fixture, '/run/fixture-iw')
Path('/run/fixture-iw').chmod(0o755)
subprocess.run(['mount', '--bind', '/run/fixture-iw', '/usr/sbin/iw'], check=True)
subprocess.run(['mount', '--bind', '/usr/bin/true', '/usr/bin/pkcheck'], check=True)
helper_dir = Path('/usr/libexec/linuxdrop')
assert helper_dir.is_dir(), 'Install helper directory before this fixture'
subprocess.run(['mount', '-t', 'tmpfs', '-o', 'mode=0755', 'linuxdrop-filin', str(helper_dir)], check=True)
shutil.copyfile(fixture, helper_dir / 'filin')
(helper_dir / 'filin').chmod(0o755)

def wait_for(check, process=None, seconds=8):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if process is not None:
            assert process.poll() is None, 'Helper exited unexpectedly'
        try:
            if check():
                return
        except (FileNotFoundError, ConnectionRefusedError, json.JSONDecodeError):
            pass
        time.sleep(0.03)
    raise AssertionError('Timed out waiting for AWDL lifecycle condition')

class Client:
    def __init__(self):
        self.socket = socket.socket(socket.AF_UNIX)
        self.socket.settimeout(10)
        self.socket.connect('/run/linuxdrop/netd.sock')
        self.reader = self.socket.makefile('rb')

    def request(self, operation, **fields):
        self.socket.sendall(json.dumps(dict(operation=operation, **fields)).encode() + b'\n')
        return json.loads(self.reader.readline())

    def close(self):
        self.reader.close()
        self.socket.close()

logind = subprocess.Popen([sys.executable, '-m', 'dbusmock', '--session', '-t', 'logind'],
                          stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
helper = None
daemon = None
clients = []
try:
    import dbus
    bus = dbus.SessionBus()
    wait_for(lambda: bus.name_has_owner('org.freedesktop.login1'), logind)
    mock = dbus.Interface(bus.get_object('org.freedesktop.login1', '/org/freedesktop/login1'),
                          'org.freedesktop.DBus.Mock')
    mock.AddSession('watchdog', 'seat0', dbus.UInt32(os.getuid()), 'root', True)
    mock.AddMethod('org.freedesktop.login1.Manager', 'GetUser', 'u', 'o',
                   "ret = '/org/freedesktop/login1/user/' + str(args[0])")
    helper = subprocess.Popen([str(binary)], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    wait_for(lambda: Path('/run/linuxdrop/netd.sock').exists(), helper)
    observer = Client()
    clients.append(observer)

    for case in ('child-exit', 'tap-loss', 'monitor-loss', 'radio-loss', 'rfkill',
                 'regulatory', 'competing-link', 'ownership-change', 'socket-eof'):
        owner = Client()
        clients.append(owner)
        response = owner.request('acquire_awdl', radio_id=radio_id, channel=6)
        assert response['status'] == 'acquired', (case, response)
        lease = response['lease']
        monitor, tap = lease['interface'], lease['awdl_interface']
        child = int(Path('/run/filin-pid').read_text())
        assert (kernel / tap).exists() and (kernel / monitor).exists()
        assert observer.request('status')['leases'][0]['id'] == lease['id']
        Path('/run/delete-started').unlink(missing_ok=True)
        hold = case == 'child-exit'
        if hold:
            Path('/run/hold-delete').touch()
        if case == 'child-exit':
            os.kill(child, signal.SIGKILL)
        elif case == 'tap-loss':
            subprocess.run(['/usr/sbin/ip', 'link', 'del', tap], check=True)
        elif case == 'monitor-loss':
            subprocess.run(['/usr/sbin/ip', 'link', 'del', monitor], check=True)
            shutil.rmtree(links / monitor)
        elif case == 'radio-loss':
            phy.rename(phy.with_name('unplugged'))
        elif case == 'rfkill':
            (phy / 'rfkill0/soft').write_text('1')
        elif case == 'regulatory':
            Path('/run/restrict-channel').touch()
        elif case == 'competing-link':
            subprocess.run(['/usr/sbin/ip', 'link', 'add', 'foreign0', 'type', 'dummy'], check=True)
            foreign = links / 'foreign0'
            foreign.mkdir()
            (foreign / 'phy80211').symlink_to(phy)
            (foreign / 'operstate').write_text('up')
        elif case == 'ownership-change':
            subprocess.run(['/usr/sbin/ip', 'link', 'set', monitor, 'alias', 'foreign-owner'], check=True)
        elif case == 'socket-eof':
            owner.close()
            clients.remove(owner)

        wait_for(lambda: not observer.request('status')['leases'], helper)
        wait_for(lambda: not Path(f'/proc/{child}').exists(), helper)
        assert not (kernel / tap).exists(), (case, 'TAP leaked after child reaping')
        if hold:
            wait_for(lambda: Path('/run/delete-started').exists(), helper)
            assert json.loads(journal.read_text())[0]['id'] == lease['id']
            assert observer.request('acquire_awdl', radio_id=radio_id, channel=6)['status'] == 'error'
            Path('/run/hold-delete').unlink()
        if case in ('ownership-change', 'radio-loss'):
            wait_for(lambda: bool(observer.request('recovery_status')['issues']), helper)
            assert (kernel / monitor).exists(), 'Unverified interface must not be deleted'
            assert json.loads(journal.read_text())[0]['id'] == lease['id']
            if case == 'radio-loss':
                phy.with_name('unplugged').rename(phy)
            else:
                subprocess.run(['/usr/sbin/ip', 'link', 'set', monitor, 'alias',
                                'linuxdrop:' + lease['id']], check=True)
            # Wait for the first cleanup worker to publish its failed receipt.
            wait_for(lambda: all('progress' not in i['detail'] for i in
                                 observer.request('recovery_status')['issues']), helper)
            assert observer.request('retry_recovery')['status'] == 'recovery'
        wait_for(lambda: json.loads(journal.read_text()) == [], helper)
        assert not (kernel / monitor).exists(), (case, 'owned monitor leaked')
        (links / tap).unlink(missing_ok=True)
        (phy / 'rfkill0/soft').write_text('0')
        Path('/run/restrict-channel').unlink(missing_ok=True)
        if case == 'competing-link':
            assert (kernel / 'foreign0').exists(), 'Foreign interface must survive'
            subprocess.run(['/usr/sbin/ip', 'link', 'del', 'foreign0'], check=True)
            shutil.rmtree(links / 'foreign0')
        if owner in clients:
            owner.close()
            clients.remove(owner)
        print('PASS actual netd AWDL watchdog:', case, flush=True)

    # Now connect the real user daemon and AirDrop listener to this same helper.
    # Only the unavailable radio transport and authorization setup are stand-ins.
    from gi.repository import Gio
    subprocess.run(['/usr/sbin/ip', 'link', 'set', 'lo', 'up'], check=True)
    root = Path('/run/daemon-fixture')
    config = root / 'config/linuxdrop'
    config.mkdir(parents=True)
    (config / 'settings.json').write_text(json.dumps({
        'general': {'device_name': 'AWDL lifecycle fixture'},
        'receive': {'directory': str(root / 'received')},
        'localsend': {'enabled': False},
        'quickshare': {'enabled': False},
        'airdrop': {'enabled': True, 'ble_wakeup': False},
    }))
    env = dict(os.environ, HOME=str(root), XDG_CONFIG_HOME=str(root / 'config'),
               XDG_DATA_HOME=str(root / 'data'), XDG_RUNTIME_DIR=str(root / 'runtime'))
    (root / 'runtime').mkdir(mode=0o700)
    daemon_log = (root / 'daemon.log').open('w')
    daemon = subprocess.Popen([str(daemon_binary)], env=env, stdout=daemon_log,
                              stderr=subprocess.STDOUT)
    wait_for(lambda: bus.name_has_owner('io.github.marius4lui.LinuxDrop'), daemon)
    gio = Gio.bus_get_sync(Gio.BusType.SESSION, None)

    def call(method):
        result = gio.call_sync('io.github.marius4lui.LinuxDrop', '/io/github/marius4lui/LinuxDrop',
                              'io.github.marius4lui.LinuxDrop.Manager1', method, None, None,
                              Gio.DBusCallFlags.NO_AUTO_START, 15000, None).unpack()
        return result[0] if result else None

    def backend_state():
        snapshot = json.loads(call('GetSnapshot'))
        return next((item['state'] for item in snapshot['backends'] if item['id'] == 'airdrop'), None)

    previous_lease = None
    for case in ('regulatory', 'helper-stop'):
        wait_for(lambda: backend_state() == 'ready', daemon, seconds=15)
        acquired = observer.request('status')['leases']
        assert len(acquired) == 1 and acquired[0]['id'] != previous_lease
        previous_lease = acquired[0]['id']
        with socket.socket(socket.AF_INET6) as stream:
            stream.settimeout(2)
            stream.connect(('fd42:77::1', 8771))
        if case == 'regulatory':
            Path('/run/restrict-channel').touch()
        else:
            helper.terminate()
            output = helper.communicate(timeout=8)
            assert helper.returncode == 0, output
        wait_for(lambda: backend_state() == 'error', daemon, seconds=15)
        assert not any(peer['protocols'] == ['airdrop'] for peer in json.loads(call('GetSnapshot'))['peers'])
        wait_for(lambda: json.loads(journal.read_text()) == [], daemon)
        assert not (kernel / acquired[0]['awdl_interface']).exists()
        print('PASS actual daemon/helper/AirDrop state retirement:', case, flush=True)
        if case == 'regulatory':
            Path('/run/restrict-channel').unlink()
            call('RestartBackends')
    daemon.terminate()
    assert daemon.wait(timeout=10) == 0
    daemon_log.close()
finally:
    if daemon is not None and daemon.poll() is None:
        daemon.kill()
        daemon.wait(timeout=5)
        print((Path('/run/daemon-fixture/daemon.log')).read_text()[-5000:], file=sys.stderr)
    for client in clients:
        client.close()
    if helper is not None and helper.poll() is None:
        helper.kill()
        output = helper.communicate(timeout=5)
        print(output, file=sys.stderr)
    logind.terminate()
    logind.communicate(timeout=5)
