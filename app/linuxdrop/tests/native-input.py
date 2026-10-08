#!/usr/bin/env python3
"""Drive the isolated GNOME review session through Mutter's native input API.

This is test-only process automation, not a product dependency. The session dies
with the caller, so no input device or permission remains on the host desktop.
"""
import sys
import time
from gi.repository import Gio, GLib

bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
if sys.argv[1] == 'close-overview':
    bus.call_sync('org.gnome.Shell','/org/gnome/Shell','org.freedesktop.DBus.Properties','Set',GLib.Variant('(ssv)',('org.gnome.Shell','OverviewActive',GLib.Variant('b',False))),None,Gio.DBusCallFlags.NONE,-1,None)
    raise SystemExit(0)
name = 'org.gnome.Mutter.RemoteDesktop'
interface = name + '.Session'
path = bus.call_sync(name, '/org/gnome/Mutter/RemoteDesktop', name, 'CreateSession', None, None, Gio.DBusCallFlags.NONE, -1, None).unpack()[0]

def call(method, signature=None, values=None):
    return bus.call_sync(name, path, interface, method,
                         GLib.Variant(signature, values) if signature else None,
                         None, Gio.DBusCallFlags.NONE, -1, None)

call('Start')
try:
    if sys.argv[1] == 'inspect':
        print(bus.call_sync(name,path,'org.freedesktop.DBus.Introspectable','Introspect',None,None,Gio.DBusCallFlags.NONE,-1,None).unpack()[0])
    elif sys.argv[1] == 'key':
        key = int(sys.argv[2], 0)
        call('NotifyKeyboardKeysym', '(ub)', (key, True))
        time.sleep(.1)
        call('NotifyKeyboardKeysym', '(ub)', (key, False))
    elif sys.argv[1] == 'chord':
        keys = [int(key, 0) for key in sys.argv[2:]]
        for key in keys:
            call('NotifyKeyboardKeysym', '(ub)', (key, True))
            time.sleep(.05)
        for key in reversed(keys):
            call('NotifyKeyboardKeysym', '(ub)', (key, False))
    elif sys.argv[1] == 'move':
        call('NotifyPointerMotionRelative', '(dd)', (float(sys.argv[2]), float(sys.argv[3])))
    elif sys.argv[1] == 'at':
        call('NotifyPointerMotionRelative', '(dd)', (-10000.0, 10000.0))
        time.sleep(.1)
        call('NotifyPointerMotionRelative', '(dd)', (float(sys.argv[2]), float(sys.argv[3])-899.0))
    elif sys.argv[1] in ('click', 'clickat'):
        if sys.argv[1] == 'clickat':
            call('NotifyPointerMotionRelative', '(dd)', (-10000.0, 10000.0))
            time.sleep(.1)
            call('NotifyPointerMotionRelative', '(dd)', (float(sys.argv[2]), float(sys.argv[3])-899.0))
            time.sleep(.2)
        button = int(sys.argv[2]) if sys.argv[1] == 'click' and len(sys.argv) > 2 else 272
        call('NotifyPointerButton', '(ib)', (button, True))
        time.sleep(.1)
        call('NotifyPointerButton', '(ib)', (button, False))
    elif sys.argv[1] in ('drag', 'dragfrom'):
        if sys.argv[1] == 'dragfrom':
            call('NotifyPointerMotionRelative', '(dd)', (-10000.0, 10000.0))
            time.sleep(.1)
            call('NotifyPointerMotionRelative', '(dd)', (float(sys.argv[2]), float(sys.argv[3])-899.0))
            time.sleep(.3)
            dx, dy = float(sys.argv[4]), float(sys.argv[5])
        else:
            dx, dy = float(sys.argv[2]), float(sys.argv[3])
        call('NotifyPointerButton', '(ib)', (272, True))
        time.sleep(.2)
        for _ in range(20):
            call('NotifyPointerMotionRelative', '(dd)', (dx / 20, dy / 20))
            time.sleep(.04)
        time.sleep(.5)
        if len(sys.argv) > 6 and sys.argv[6] == 'cancel':
            call('NotifyKeyboardKeysym', '(ub)', (0xff1b, True))
            time.sleep(.1)
            call('NotifyKeyboardKeysym', '(ub)', (0xff1b, False))
        call('NotifyPointerButton', '(ib)', (272, False))
finally:
    time.sleep(.15)
    call('Stop')
