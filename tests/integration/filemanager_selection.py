#!/usr/bin/env python3
"""Actual installed file-manager menu -> GTK draft, in a private X11 session.

Run through run-filemanager-selection.sh in an isolated installed-package root.
The Dolphin keyboard sequence targets the observed Fedora 44 / 26.08 menu; the
Nautilus unnamed-menu fallback targets 50.3. Unexpected menus fail acceptance.
"""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time

import gi
import pyatspi
from gi.repository import Gio, GLib

manager = sys.argv[1]
assert manager in ('nautilus', 'thunar', 'dolphin')
assert os.geteuid() != 0
assert os.environ.get('XDG_RUNTIME_DIR', '').startswith('/tmp/linuxdrop-filemanager.')
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
app_name = 'io.github.marius4lui.LinuxDrop.App'
assert not bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus',
    'org.freedesktop.DBus', 'NameHasOwner', GLib.Variant('(s)', (app_name,)),
    None, Gio.DBusCallFlags.NONE, 2000, None).unpack()[0]


def nodes(root=None):
    context = GLib.MainContext.default()
    while context.pending():
        context.iteration(False)
    pending = [(root or pyatspi.Registry.getDesktop(0), 0)]
    visited = 0
    while pending and visited < 3000:
        item, depth = pending.pop(0)
        visited += 1
        if item is None or depth > 40:
            continue
        try:
            item.clearCache()
            yield item
            pending.extend((item.getChildAtIndex(i), depth + 1)
                           for i in range(min(item.childCount, 150)))
        except (GLib.Error, RuntimeError):
            pass


def wait(check):
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        value = check()
        if value:
            return value
        time.sleep(.15)
    raise AssertionError('Native file-manager condition timed out')


def named(name, role):
    return next((item for item in nodes() if item.name == name and item.getRoleName() == role), None)


def capture(name):
    directory = os.environ.get('LINUXDROP_FILEMANAGER_CAPTURES')
    if not directory:
        return
    gi.require_version('Gdk', '3.0')
    from gi.repository import Gdk
    root = Gdk.get_default_root_window()
    target = Path(directory)
    target.mkdir(parents=True, exist_ok=True)
    Gdk.pixbuf_get_from_window(root, 0, 0, root.get_width(), root.get_height()).savev(
        str(target / (manager + '-' + name + '.png')), 'png', [], [])


with tempfile.TemporaryDirectory(prefix='linuxdrop-file-selection-') as temporary:
    root = Path(temporary)
    filenames = ["01 space & quote ' file.txt", '02 ; $(touch INJECTION) file.txt', '-03 Gr\u00fc\u00dfe [literal].txt']
    for name in filenames:
        (root / name).write_text('test bytes')
    wm_log = (Path(os.environ['XDG_RUNTIME_DIR']) / 'openbox.log').open('w')
    wm = subprocess.Popen(['openbox'], stdout=wm_log, stderr=subprocess.STDOUT)
    def wm_ready():
        if wm.poll() is not None:
            wm_log.flush()
            raise AssertionError((Path(os.environ['XDG_RUNTIME_DIR']) / 'openbox.log').read_text())
        result = subprocess.run(['xprop', '-root', '_NET_SUPPORTING_WM_CHECK'], capture_output=True, text=True)
        return 'window id' in result.stdout
    wait(wm_ready)
    command = [manager, *(['--new-window'] if manager == 'nautilus' else []), str(root)]
    process = subprocess.Popen(command, cwd=root)
    try:
        def window_id():
            result = subprocess.run(['xdotool', 'search', '--name', root.name], capture_output=True, text=True)
            return result.stdout.splitlines()[-1] if result.returncode == 0 else None
        window = wait(window_id)
        # Let the view load before selecting; all subsequent assertions inspect
        # actual UI state and exact selected filenames, not subprocess success.
        time.sleep(3)
        subprocess.run(['xdotool', 'windowactivate', '--sync', window, 'key', 'ctrl+a'], check=True)
        time.sleep(.3)
        if manager == 'dolphin':
            # Controlled default Dolphin layout. The fixture screenshot verifies
            # this is the selected first file; no host desktop is involved.
            subprocess.run(['xdotool', 'mousemove', '--window', window, '235', '130', 'click', '3'], check=True)
            time.sleep(2)
            capture('menu')
            subprocess.run(['xdotool', 'key', '--delay', '150', 'Up', 'Up', 'Return'], check=True)
        else:
            subprocess.run(['xdotool', 'key', 'Menu'], check=True)
            time.sleep(.5)
            if manager == 'thunar':
                assert wait(lambda: named('Send To', 'menu')).queryAction().doAction(0)
                action = wait(lambda: named('LinuxDrop', 'menu item'))
            else:
                action = named('Send with LinuxDrop', 'menu item')
                if action is None:
                    # GTK 4.22 exposes these menu items with empty names. Fail if
                    # the reviewed, otherwise unextended menu shape changes.
                    menus = [item for item in nodes() if item.getRoleName() == 'menu item']
                    assert len(menus) == 12 and all(not item.name for item in menus)
                    action = menus[-2]
            capture('menu')
            assert action.queryAction().doAction(0)
        app = wait(lambda: named('linuxdrop', 'application'))
        expected = {'Remove file: ' + name for name in filenames}
        def draft_ready():
            labels = {item.name for item in nodes(app) if item.getState().contains(pyatspi.STATE_SHOWING)}
            actual = {name for name in labels if name.startswith('Remove file: ')}
            return actual == expected and '3 files ready \u00b7 choose a device' in labels
        wait(draft_ready)
        for name in filenames:
            assert (root / name).read_text() == 'test bytes'
        assert not (root / 'INJECTION').exists()
        assert not (Path.home() / 'INJECTION').exists()
        capture('draft')
        print(f'LINUXDROP_FILEMANAGER_PASSED {manager}: real menu, 3 literal files, ready draft', flush=True)
    finally:
        try:
            bus.call_sync(app_name, '/io/github/marius4lui/LinuxDrop/App', 'org.gtk.Actions',
                'Activate', GLib.Variant('(sava{sv})', ('quit', [], {})), None,
                Gio.DBusCallFlags.NO_AUTO_START, 2000, None)
        except GLib.Error:
            pass
        process.terminate()
        wm.terminate()
        for child in (process, wm):
            try:
                child.wait(timeout=3)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait()
        wm_log.close()
