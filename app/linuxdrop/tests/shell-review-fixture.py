#!/usr/bin/env python3
"""Private-bus Shell rendering fixture; never starts protocol listeners."""
import json
import sys
from pathlib import Path
from gi.repository import Gio, GLib

bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
service = 'io.github.marius4lui.LinuxDrop'
path = '/io/github/marius4lui/LinuxDrop'
interface = service + '.Manager1'
result = bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'RequestName', GLib.Variant('(su)', (service, 0)), None, Gio.DBusCallFlags.NONE, 2000, None)
assert result.unpack()[0] == 1, 'Fixture requires a private unused session bus'
snapshot = dict(epoch='shell-review-fixture', revision=1, peers=[], backends=[], settings=dict(visibility=dict(mode='hidden')),
                transfers=[dict(id='verification', peer_name='Prüfgerät · keine echte Verbindung', state='verification', direction='incoming', protocol='quickshare', verification_code='1234', files=[dict(name='A' * 100 + '.jpg', size=120)], total_bytes=120, transferred_bytes=0),
                           dict(id='other-transfer', peer_name='Zweites Prüfgerät', state='transferring', direction='outgoing', protocol='localsend', files=[dict(name='fixture.txt', size=100)], total_bytes=100, transferred_bytes=43)])
# Use the real settings schema and complete transfer records on the wire.
snapshot.update(restarting=False, download_link_active=False, known_peers=[], hardware=dict(observed_unix=0, radios=[], interfaces=[], bluetooth=[], warnings=[]))
snapshot['settings'] = json.loads((Path(__file__).resolve().parents[3] / 'crates/linuxdrop-ipc/settings.defaults.json').read_text())
if '--a11y' in sys.argv:
    snapshot['peers'] = [dict(id='fixture-peer', name='Orca Testgerät',
        platform='android', available=True, protocols=['localsend', 'quickshare'],
        address='192.0.2.1:53317', identity_scope='Simulated accessibility fixture; no real device.')]
for transfer in snapshot['transfers']:
    transfer.update(peer_id='fixture-peer', saved_paths=[])
    for file in transfer['files']:
        file['transferred'] = transfer['transferred_bytes']
info = Gio.DBusNodeInfo.new_for_xml((Path(__file__).resolve().parents[3] / 'crates/linuxdrop-ipc/manager1.xml').read_text())
def called(connection, sender, object_path, iface, method, params, invocation):
    value = snapshot['settings'] if method in ('GetSettings', 'GetDefaults') else snapshot
    invocation.return_value(GLib.Variant('(s)', (json.dumps(value),)))
bus.register_object(path, info.interfaces[0], called, None, None)
print('fixture-ready', flush=True)
GLib.MainLoop().run()
