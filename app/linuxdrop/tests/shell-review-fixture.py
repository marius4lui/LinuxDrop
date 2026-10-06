#!/usr/bin/env python3
"""Private-bus Shell rendering fixture; never starts protocol listeners."""
import json
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
info = Gio.DBusNodeInfo.new_for_xml(f"<node><interface name='{interface}'><method name='GetSnapshot'><arg type='s' direction='out'/></method></interface></node>")
def called(connection, sender, object_path, iface, method, params, invocation):
    invocation.return_value(GLib.Variant('(s)', (json.dumps(snapshot),)))
bus.register_object(path, info.interfaces[0], called, None, None)
print('fixture-ready', flush=True)
GLib.MainLoop().run()
