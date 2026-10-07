"""Actual netd restarts with private kernel inventory and supplicant fixtures."""
import json
import multiprocessing
import os
from pathlib import Path
import socket
import subprocess
import sys
import time

for namespace in ('mnt', 'net'):
    assert os.readlink(f'/proc/self/ns/{namespace}') != os.readlink(f'/proc/1/ns/{namespace}')
assert os.environ['DBUS_SESSION_BUS_ADDRESS'].startswith('unix:')
os.environ['DBUS_SYSTEM_BUS_ADDRESS'] = os.environ['DBUS_SESSION_BUS_ADDRESS']
for path in ('/run', '/var/lib', '/sys'):
    subprocess.run(['mount', '-t', 'tmpfs', '-o', 'mode=0755', 'linuxdrop-formation-fixture', path], check=True)
for path in ('/run/linuxdrop', '/var/lib/linuxdrop-netd', '/sys/class/ieee80211/phy-test', '/sys/class/net/testwifi0'):
    Path(path).mkdir(parents=True, exist_ok=True)
interface = Path('/sys/class/net/testwifi0')
(interface / 'phy80211').symlink_to('/sys/class/ieee80211/phy-test')
(interface / 'ifindex').write_text('42')
state_file = Path('/run/formation-state.json')
calls_file = Path('/run/formation-calls.json')
journal = Path('/var/lib/linuxdrop-netd/leases.json')
SERVICE = 'fi.w1.wpa_supplicant1'
ROOT = '/fi/w1/wpa_supplicant1'
PARENT = ROOT + '/Interfaces/0'
VIF = ROOT + '/Interfaces/1'


def state(**values):
    temporary = state_file.with_suffix('.tmp')
    temporary.write_text(json.dumps(values))
    temporary.replace(state_file)


def fixture(queue):
    import dbus
    import dbus.service
    from dbus.mainloop.glib import DBusGMainLoop
    from gi.repository import GLib
    DBusGMainLoop(set_as_default=True)
    bus = dbus.SessionBus()
    name = dbus.service.BusName(SERVICE, bus)

    class Root(dbus.service.Object):
        @dbus.service.method(SERVICE, in_signature='s', out_signature='o')
        def GetInterface(self, name):
            assert name == 'testwifi0'
            return dbus.ObjectPath(PARENT)

        @dbus.service.method('org.freedesktop.DBus.Properties', in_signature='ss', out_signature='v')
        def Get(self, interface, prop):
            assert interface == SERVICE and prop == 'Interfaces'
            paths = [PARENT]
            if json.loads(state_file.read_text()).get('group'):
                paths.append(VIF)
            return dbus.Array(paths, signature='o')

    class Device(dbus.service.Object):
        @dbus.service.method('org.freedesktop.DBus.Properties', in_signature='ss', out_signature='v')
        def Get(self, interface, prop):
            assert interface == SERVICE + '.Interface.P2PDevice' and prop == 'Group'
            return dbus.ObjectPath('/')

        @dbus.service.method(SERVICE + '.Interface.P2PDevice', in_signature='', out_signature='')
        def Cancel(self):
            calls = json.loads(calls_file.read_text())
            calls['cancel'] += 1
            calls_file.write_text(json.dumps(calls))
            if json.loads(state_file.read_text()).get('reject'):
                raise dbus.exceptions.DBusException('fixture rejection', name=SERVICE + '.UnknownError')

        @dbus.service.method(SERVICE + '.Interface.P2PDevice', in_signature='', out_signature='')
        def Disconnect(self):
            raise AssertionError('Recovery must never disconnect an unidentified group')

    root = Root(bus, ROOT)
    device = Device(bus, PARENT)
    # Match zbus's authenticated server GUID, not the separate GetId bus ID.
    bus_id = next(part.split('=', 1)[1] for part in os.environ['DBUS_SESSION_BUS_ADDRESS'].split(',') if part.startswith('guid='))
    queue.put((bus.get_unique_name(), bus_id))
    GLib.MainLoop().run()


def request(operation):
    with socket.socket(socket.AF_UNIX) as client:
        client.settimeout(2)
        client.connect('/run/linuxdrop/netd.sock')
        client.sendall(json.dumps({'operation': operation}).encode() + b'\n')
        return json.loads(client.makefile('rb').readline())


def wait_for(check, process, seconds=6):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        assert process.poll() is None, 'Helper stopped unexpectedly'
        try:
            if check():
                return
        except (FileNotFoundError, ConnectionRefusedError, json.JSONDecodeError):
            pass
        time.sleep(0.03)
    raise AssertionError(f'Recovery timed out: {request("recovery_status")}')


def run(check):
    process = subprocess.Popen([sys.argv[1]], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    try:
        wait_for(lambda: request('status')['status'] == 'state', process)
        check(process)
        process.terminate()
        output = process.communicate(timeout=6)
        assert process.returncode == 0, output
    finally:
        if process.poll() is None:
            process.kill()
            process.communicate()


state()
calls_file.write_text('{"cancel": 0}')
queue = multiprocessing.Queue()
server = multiprocessing.Process(target=fixture, args=(queue,))
server.start()
try:
    owner, bus_id = queue.get(timeout=5)
    lease = dict(id='b' * 32, uid=os.getuid(), phy='phy-test', interface='testwifi0',
        channel=0, boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
        awdl_interface=None, kind='direct_wifi', p2p_pending=True, p2p_group=None,
        connection_uuid=None, p2p_recovery=dict(kernel_interfaces={'testwifi0': 42},
            formation=dict(service_owner=owner, bus_guid=bus_id, parent_interface='testwifi0',
                parent_object=PARENT, existing_interfaces=[PARENT])))

    def cleared(process):
        wait_for(lambda: json.loads(journal.read_text()) == [], process)
        assert request('status')['leases'] == []

    def retained(process):
        wait_for(lambda: bool(request('recovery_status')['recent_errors']), process)
        assert json.loads(journal.read_text())[0]['p2p_pending']
        assert request('status')['leases'] == []

    journal.write_text(json.dumps([lease]))
    run(cleared)
    assert json.loads(calls_file.read_text())['cancel'] == 1
    for case in ('foreign-owner', 'different-parent', 'new-kernel-interface', 'unknown-group'):
        changed = json.loads(json.dumps(lease))
        if case == 'foreign-owner': changed['p2p_recovery']['formation']['service_owner'] = ':99999.1'
        if case == 'different-parent': changed['p2p_recovery']['formation']['parent_interface'] = 'foreign0'
        if case == 'new-kernel-interface': (interface / 'ifindex').write_text('43')
        state(group=case == 'unknown-group')
        before = json.loads(calls_file.read_text())['cancel']
        journal.write_text(json.dumps([changed]))
        run(retained)
        assert json.loads(calls_file.read_text())['cancel'] == before, case
        (interface / 'ifindex').write_text('42')
    state(reject=True)
    journal.write_text(json.dumps([lease]))
    run(retained)
    # A second actual helper process reloads the retained record, then its own
    # periodic recovery completes when the original service acknowledges Cancel.
    def retry(process):
        retained(process)
        state()
        wait_for(lambda: json.loads(journal.read_text()) == [], process, seconds=36)
    run(retry)
    print('LINUXDROP_FORMATION_RECOVERY_PASSED: real helper restart, durable provenance, pinned cancellation, automatic retry, foreign owner/group/kernel refusal')
finally:
    server.terminate()
    server.join(timeout=5)
