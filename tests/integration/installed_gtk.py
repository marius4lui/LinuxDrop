#!/usr/bin/env python3
"""Check an installed GTK package through desktop launch and AT-SPI.

Run only in an isolated desktop/network fixture, as an unprivileged user:
  GTK_A11Y=atspi GDK_BACKEND=x11 GSK_RENDERER=cairo \
    dbus-run-session -- xvfb-run -a python3 tests/integration/installed_gtk.py
Requires the installed package, PyGObject, pyatspi, Xvfb, and a usable isolated
network interface. This uses real installed binaries with temporary preferences;
it checks accessible UI state, not screen-reader speech or physical transfers.
"""
import json, os, socket, subprocess, tempfile, time
from pathlib import Path
from gi.repository import Gio, GLib
import pyatspi

bus=Gio.bus_get_sync(Gio.BusType.SESSION,None)
APP='io.github.marius4lui.LinuxDrop.App'
if os.geteuid() == 0:
    raise SystemExit('Run the installed GUI check as an unprivileged user')
for name in (APP, 'io.github.marius4lui.LinuxDrop'):
    occupied=bus.call_sync('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','NameHasOwner',GLib.Variant('(s)',(name,)),None,Gio.DBusCallFlags.NONE,1000,None).unpack()[0]
    if occupied:
        raise SystemExit('Refusing to touch an existing LinuxDrop session; use a private bus')

def owner():
    return bus.call_sync('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','GetNameOwner',GLib.Variant('(s)',(APP,)),None,Gio.DBusCallFlags.NONE,1000,None).unpack()[0]
def names():
    values=[]
    pending=[pyatspi.Registry.getDesktop(0)]
    seen=0
    while pending and seen<2500:
        item=pending.pop(); seen+=1
        try:
            if item.name and item.getState().contains(pyatspi.STATE_SHOWING): values.append(item.name)
            pending.extend(item.getChildAtIndex(i) for i in range(min(item.childCount,200)))
        except Exception:
            pass
    return values
def wait(check):
    deadline=time.monotonic()+20
    while time.monotonic()<deadline:
        try:
            if check(): return
        except GLib.Error: pass
        time.sleep(.15)
    raise AssertionError('Installed GUI condition timed out; accessible names: '+repr(names()))
with tempfile.TemporaryDirectory(prefix='linuxdrop-installed-ui-') as tmp:
    tmp=Path(tmp)
    for key,leaf in [('XDG_CONFIG_HOME','config'),('XDG_DATA_HOME','data'),('XDG_CACHE_HOME','cache')]:
        os.environ[key]=str(tmp/leaf)
    conf=tmp/'config/linuxdrop'; conf.mkdir(parents=True)
    with socket.socket() as s:
        s.bind(('127.0.0.1',0)); port=s.getsockname()[1]
    (conf/'settings.json').write_text(json.dumps({'general':{'device_name':'Installed package test'},'localsend':{'port':port,'multicast':False},'quickshare':{'enabled':False},'airdrop':{'enabled':False}}))
    # Start the installed daemon with the same private XDG environment; the
    # accessibility session bus was already running before these directories.
    with (tmp/'daemon.log').open('w') as log:
        daemon=subprocess.Popen(['/usr/bin/linuxdropd'],stdout=log,stderr=subprocess.STDOUT)
        try:
            sample=tmp/'Package selection & spaces.txt'; sample.write_text('Installed desktop launch')
            desktop=Gio.DesktopAppInfo.new_from_filename('/usr/share/applications/io.github.marius4lui.LinuxDrop.desktop')
            assert desktop is not None
            assert desktop.launch([Gio.File.new_for_path(str(sample))],None)
            wait(lambda:any(sample.name in text for text in names()))
            original_owner=owner()
            subprocess.run(['/usr/bin/linuxdrop','--settings'],check=True,timeout=15)
            wait(lambda:'Device name' in names())
            assert owner()==original_owner, 'CLI navigation must reuse the installed app'
            subprocess.run(['/usr/bin/linuxdrop','--hardware'],check=True,timeout=15)
            wait(lambda:'No Bluetooth controllers detected' in names())
            assert owner()==original_owner
            subprocess.run(['/usr/bin/linuxdrop','open'],check=True,timeout=15)
            wait(lambda:any(sample.name in text for text in names()))
            print('LINUXDROP_INSTALLED_GTK_PASSED desktop-file file selection, settings, hardware, single-instance reuse, retained draft')
        finally:
            try:
                bus.call_sync(owner(),'/io/github/marius4lui/LinuxDrop/App','org.gtk.Actions','Activate',GLib.Variant('(sava{sv})',('quit',[],{})),None,Gio.DBusCallFlags.NO_AUTO_START,3000,None)
            except GLib.Error: pass
            daemon.terminate()
            try: daemon.wait(timeout=10)
            except subprocess.TimeoutExpired: daemon.kill(); daemon.wait()
