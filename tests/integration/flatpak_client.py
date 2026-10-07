#!/usr/bin/env python3
"""Installed Flatpak -> real chooser/portal -> host daemon, plus received-file actions.

Requires the private namespaces and dedicated test account from the shell runner.
A persisted receipt is seeded to isolate the completed-transfer UX; it is not a
claim of physical-device reception. The registered MIME reader is a real child
process. FileManager1 is a recording test service.
"""
import json
import os
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from gi.repository import Gio, GLib
import pyatspi
assert os.environ.get('LINUXDROP_FP_PRIVATE') == '1'
assert os.readlink('/proc/self/ns/net') != os.environ['LINUXDROP_FP_PARENT_NET']
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)
folder_calls = []
manager_xml = Gio.DBusNodeInfo.new_for_xml('<node><interface name="org.freedesktop.FileManager1"><method name="ShowItems"><arg type="as" direction="in"/><arg type="s" direction="in"/></method></interface></node>')

def show_items(connection, sender, path, interface, method, args, invocation):
    folder_calls.append(args.unpack()[0])
    invocation.return_value(GLib.Variant('()', ()))
manager_registration = bus.register_object('/org/freedesktop/FileManager1', manager_xml.interfaces[0], show_items, None, None)
bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'RequestName', GLib.Variant('(su)', ('org.freedesktop.FileManager1', 0)), None, Gio.DBusCallFlags.NONE, 5000, None)
root = Path(os.environ['XDG_RUNTIME_DIR'])
config = Path(os.environ['XDG_CONFIG_HOME']) / 'linuxdrop'
config.mkdir(parents=True)
(config / 'settings.json').write_text(json.dumps({'general': {'device_name': 'Flatpak integration', 'appearance': 'light', 'language': 'en'}, 'localsend': {'enabled': False}, 'quickshare': {'enabled': False}, 'airdrop': {'enabled': False}, 'network': {'allowed_interfaces': ['ld-fp-test']}}))
applications = Path(os.environ['XDG_DATA_HOME']) / 'applications'
applications.mkdir(parents=True)
opener_script = root / 'record-open.py'
opener_script.write_text("from pathlib import Path\nimport json,sys\np=Path(sys.argv[1]);Path('" + str(root / 'opened.json') + "').write_text(json.dumps({'path':str(p),'bytes':list(p.read_bytes())}))\n")
desktop = applications / 'linuxdrop-test-reader.desktop'
desktop.write_text('[Desktop Entry]\nType=Application\nName=LinuxDrop test reader\nExec=/usr/bin/python3 ' + str(opener_script) + ' %f\nMimeType=text/plain;\nNoDisplay=false\n')
assert Gio.DesktopAppInfo.new_from_filename(str(desktop)).set_as_default_for_type('text/plain')
source = root / 'Portal selection & literal.txt'
source.write_bytes(b'Flatpak portal original bytes')
received = root / 'Received literal & file.txt'
received.write_bytes(b'Received original bytes')
data = Path(os.environ['XDG_DATA_HOME']) / 'linuxdrop'
data.mkdir(parents=True)
(data / 'history.json').write_text(json.dumps([{'id': 'received-fixture', 'peer_id': 'fixture', 'peer_name': 'Fixture sender', 'protocol': 'localsend', 'direction': 'incoming', 'state': 'completed', 'files': [{'name': received.name, 'size': 23, 'transferred': 23}], 'total_bytes': 23, 'transferred_bytes': 23, 'error': None, 'verification_code': None, 'saved_paths': [str(received)], 'completed_at': int(time.time())}]))
assert subprocess.run(['flatpak', 'run', '--user', '--command=test', 'io.github.marius4lui.LinuxDrop.App', '-r', str(source)]).returncode == 1, 'Host source leaked into sandbox'

def nodes():
    context = GLib.MainContext.default()
    while context.pending():
        context.iteration(False)
    pending = [(pyatspi.Registry.getDesktop(0), 0)]
    while pending:
        item, depth = pending.pop(0)
        if item is None or depth > 40:
            continue
        try:
            item.clearCache()
            yield (item, depth)
            pending.extend(((item.getChildAtIndex(i), depth + 1) for i in range(min(item.childCount, 150))))
        except GLib.Error:
            pass

def wait(test):
    until = time.monotonic() + 20
    while time.monotonic() < until:
        context = GLib.MainContext.default()
        while context.pending():
            context.iteration(False)
        result = test()
        if result:
            return result
        time.sleep(0.2)
    print('UI', [(item.getRoleName(), item.name) for item, d in nodes() if item.name], flush=True)
    print('PROCESSES', app.poll(), daemon.poll(), flush=True)
    raise AssertionError('Timed out')

# Ubuntu calls buttons 'push button'; newer AT-SPI calls them 'button'.
def named(name, role):
    return next((item for item, d in nodes() if item.name == name and item.getRoleName() in (('button', 'push button') if role == 'button' else (role,))), None)
log = (root / 'daemon.log').open('w')
daemon = subprocess.Popen([sys.argv[1]], stdout=log, stderr=subprocess.STDOUT, env={**os.environ, 'DBUS_SYSTEM_BUS_ADDRESS': 'unix:path=/nonexistent'})
app_log = (root / 'app.log').open('w')
app = subprocess.Popen(['flatpak', 'run', '--user', 'io.github.marius4lui.LinuxDrop.App'], stdout=app_log, stderr=subprocess.STDOUT)
try:
    choose = wait(lambda: named('Choose files', 'button'))
    assert choose.queryAction().doAction(0)
    chooser = wait(lambda: named('Choose files to share', 'file chooser'))
    time.sleep(1)
    window = subprocess.check_output(['xdotool', 'search', '--name', '^Choose files to share$'], text=True).splitlines()[-1]
    subprocess.run(['xdotool', 'windowfocus', '--sync', window, 'key', 'ctrl+l'], check=True)
    time.sleep(0.3)
    entries = [i for i, d in nodes() if i.getRoleName() == 'text' and i.getState().contains(pyatspi.STATE_SHOWING)]
    entry = entries[-1]
    assert entry.queryEditableText().setTextContents(str(source))
    time.sleep(0.5)
    assert named('Add files', 'button').queryAction().doAction(0)
    wait(lambda: named('Add more files', 'button'))
    wait(lambda: named('Remove file: ' + source.name, 'button'))
    wait(lambda: named('1 file ready · choose a device', 'label'))
    share = wait(lambda: named('Share with a link', 'button'))
    assert share.queryAction().doAction(0)
    create = wait(lambda: named('Create link', 'button'))
    assert create.queryAction().doAction(0)
    wait(lambda: named('Stop sharing', 'button'))
    names = [i.name for i, d in nodes() if i.getRoleName() == 'label']
    url = next((n for n in names if n.startswith('http://198.18.0.1:')))
    pin = next((n for n in names if len(n) == 6 and n.isdigit()))
    documents = bus.call_sync('org.freedesktop.portal.Documents', '/org/freedesktop/portal/documents', 'org.freedesktop.portal.Documents', 'List', GLib.Variant('(s)', ('io.github.marius4lui.LinuxDrop.App',)), None, Gio.DBusCallFlags.NONE, 5000, None).unpack()[0]
    assert documents, 'Chooser did not export a document'
    # Revoke the portal and replace its original pathname after preparing the offer.
    for doc_id in documents:
        bus.call_sync('org.freedesktop.portal.Documents', '/org/freedesktop/portal/documents', 'org.freedesktop.portal.Documents', 'Delete', GLib.Variant('(s)', (doc_id,)), None, Gio.DBusCallFlags.NONE, 5000, None)
    source.rename(source.with_suffix('.moved'))
    source.write_bytes(b'Replacement must not be sent')
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    request = urllib.request.Request(url + '/api/localsend/v2/prepare-download?' + urllib.parse.urlencode({'pin': pin}), data=b'', method='POST')
    with opener.open(request, timeout=5) as response:
        metadata = json.load(response)
    assert len(metadata['files']) == 1
    item = next(iter(metadata['files'].values()))
    assert item['fileName'] == source.name, item
    download = url + '/api/localsend/v2/download?' + urllib.parse.urlencode({'sessionId': metadata['sessionId'], 'fileId': item['id']})
    with opener.open(download, timeout=5) as response:
        assert response.read() == b'Flatpak portal original bytes'
    assert named('Stop sharing', 'button').queryAction().doAction(0)
    time.sleep(0.5)
    try:
        opener.open(download, timeout=1)
        raise AssertionError('Revoked link remained usable')
    except urllib.error.URLError:
        pass
    assert named('Transfers', 'page tab').queryAction().doAction(0)
    open_file = wait(lambda: named('Open file', 'button'))
    assert open_file.queryAction().doAction(0)
    reader = wait(lambda: named('LinuxDrop test reader', 'label'))
    # GTK portal exposes label -> filler -> list item.
    row = reader.parent.parent
    assert row.parent.querySelection().selectChild(row.getIndexInParent())
    assert named('Open', 'button').queryAction().doAction(0)
    wait(lambda: (root / 'opened.json').exists())
    opened = json.loads((root / 'opened.json').read_text())
    assert opened['path'] == str(received), opened
    assert bytes(opened['bytes']) == b'Received original bytes', opened
    assert named('Show the destination folder', 'button').queryAction().doAction(0)
    wait(lambda: folder_calls)
    assert len(folder_calls) == 1 and len(folder_calls[0]) == 1
    assert urllib.parse.unquote(urllib.parse.urlsplit(folder_calls[0][0]).path) == str(received), folder_calls

    def export(path):
        return bus.call_sync('io.github.marius4lui.LinuxDrop', '/io/github/marius4lui/LinuxDrop', 'io.github.marius4lui.LinuxDrop.Manager1', 'ExportReceivedFile', GLib.Variant('(s)', (str(path),)), None, Gio.DBusCallFlags.NO_AUTO_START, 5000, None).unpack()[0]
    for invalid in (source, received.parent, 'relative.txt'):
        try:
            export(invalid)
            raise AssertionError('Unreceived file was exported')
        except GLib.Error:
            pass
    exported = export(received)
    assert subprocess.check_output(['flatpak', 'run', '--user', '--command=cat', 'io.github.marius4lui.LinuxDrop.App', exported]) == b'Received original bytes'
    assert subprocess.run(['flatpak', 'run', '--user', '--command=test', 'io.github.marius4lui.LinuxDrop.App', '-w', exported]).returncode == 1
    received.unlink()
    received.symlink_to(source)
    try:
        export(received)
        raise AssertionError('Symlink replacement was exported')
    except GLib.Error:
        pass
    print('PASS: installed sandbox selection, portal export, descriptor handoff, exact bytes, link shutdown, received file launch and folder reveal', flush=True)
finally:
    subprocess.run(['flatpak', 'kill', 'io.github.marius4lui.LinuxDrop.App'], check=False)
    app.wait(timeout=10)
    daemon.terminate()
    daemon.wait(timeout=10)
