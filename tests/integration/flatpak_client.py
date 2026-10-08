#!/usr/bin/env python3
"""Installed Flatpak -> real chooser/portal -> host daemon, plus received-file actions.

Requires the private namespaces and dedicated test account from the shell runner.
A persisted receipt is seeded to isolate the completed-transfer UX; it is not a
claim of physical-device reception. The registered MIME reader is a real child
process. FileManager1 is a recording test service.
"""
import concurrent.futures
import json
import signal
import ssl
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
# Recording host GNOME service: validate sandbox routing and failure recovery.
notch_calls = []
notch_fail = [True]
notch_xml = Gio.DBusNodeInfo.new_for_xml('<node><interface name="org.gnome.Shell.Extensions"><method name="OpenExtensionPrefs"><arg type="s" direction="in"/><arg type="s" direction="in"/><arg type="a{sv}" direction="in"/></method></interface></node>')
def open_notch(connection, sender, path, interface, method, args, invocation):
    notch_calls.append(args.unpack())
    if notch_fail[0]:
        invocation.return_dbus_error('org.gnome.Shell.Extensions.Error', 'Fixture extension unavailable')
    else:
        invocation.return_value(GLib.Variant('()', ()))
notch_registration = bus.register_object('/org/gnome/Shell/Extensions', notch_xml.interfaces[0], open_notch, None, None)
bus.call_sync('org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'RequestName', GLib.Variant('(su)', ('org.gnome.Shell.Extensions', 0)), None, Gio.DBusCallFlags.NONE, 5000, None)
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

def nodes(portal_only=False):
    context = GLib.MainContext.default()
    while context.pending():
        context.iteration(False)
    desktop = pyatspi.Registry.getDesktop(0)
    roots = [desktop]
    if portal_only:
        roots = [desktop.getChildAtIndex(i) for i in range(desktop.childCount)]
        roots = [item for item in roots if item and item.name == 'xdg-desktop-portal-gtk']
    pending = [(item,0) for item in roots]
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
    print('UI', [(item.getRoleName(), item.name) for item, d in nodes() if item.name][:100], flush=True)
    print('PROCESSES', app.poll(), daemon.poll(), flush=True)
    raise AssertionError('Timed out')

# Ubuntu calls buttons 'push button'; newer AT-SPI calls them 'button'.
def named(name, role):
    roles=('button','push button') if role=='button' else (role,)
    for item,_ in nodes():
        if item.name != name or item.getRoleName() not in roles:continue
        # GTK may retain an accessible proxy for a row removed by a fresh snapshot.
        bounds=item.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
        if bounds.width>0 and bounds.height>0 and bounds.x>-100000 and bounds.y>-100000:
            return item
    return None

def choose_directory(destination):
    wait(lambda: named('Choose a receiving folder', 'file chooser'))
    time.sleep(1)
    # GTK 3's Recent view has no selectable folder. Enter its actual filesystem view.
    home=next(i for i,d in nodes(True) if i.name=='Home' and i.getRoleName()=='label')
    rect=home.queryComponent().getExtents(pyatspi.DESKTOP_COORDS)
    subprocess.run(['xdotool','mousemove',str(rect.x+rect.width//2),str(rect.y+rect.height//2),'click','1'],check=True,timeout=5)
    time.sleep(.5)
    window=subprocess.check_output(['xdotool','search','--name','^Choose a receiving folder$'],text=True).splitlines()[-1]
    subprocess.run(['xdotool','windowfocus','--sync',window,'key','ctrl+l'],check=True,timeout=5)
    time.sleep(.3)
    entries=[i for i,d in nodes(True) if i.getRoleName()=='text' and i.getState().contains(pyatspi.STATE_SHOWING)]
    assert entries[-1].queryComponent().grabFocus()
    subprocess.run(['xdotool','key','ctrl+a'],check=True,timeout=5)
    subprocess.run(['xdotool','type','--clearmodifiers','--',str(destination)+'/'],check=True,timeout=5)
    subprocess.run(['xdotool','key','Return'],check=True,timeout=5)
    time.sleep(.5)
    select=next((i for i,d in nodes(True) if i.name=='Select' and i.getRoleName()=='push button'),None)
    if select:assert select.queryAction().doAction(0)

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
    entries = [i for i, d in nodes(True) if i.getRoleName() == 'text' and i.getApplication().name == 'xdg-desktop-portal-gtk' and i.getState().contains(pyatspi.STATE_SHOWING)]
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
    # Persist the actual picker destination, not its expiring document path.
    destination = root / 'Receive directory & space'
    destination.mkdir()
    assert named('Settings', 'page tab').queryAction().doAction(0)
    choose_folder = wait(lambda: named('Save files to', 'button'))
    assert choose_folder.queryAction().doAction(0)
    choose_directory(destination)
    def saved_directory():
        response=bus.call_sync('io.github.marius4lui.LinuxDrop','/io/github/marius4lui/LinuxDrop','io.github.marius4lui.LinuxDrop.Manager1','GetSettings',None,None,Gio.DBusCallFlags.NO_AUTO_START,5000,None)
        return json.loads(response.unpack()[0])['receive']['directory']
    saved=wait(lambda: saved_directory() if destination.name in saved_directory() else None)
    assert saved == str(destination), ('Saved an expiring sandbox path', saved)

    def call(method, signature=None, values=()):
        args=GLib.Variant(signature,values) if signature else None
        result=bus.call_sync('io.github.marius4lui.LinuxDrop','/io/github/marius4lui/LinuxDrop','io.github.marius4lui.LinuxDrop.Manager1',method,args,None,Gio.DBusCallFlags.NO_AUTO_START,10000,None).unpack()
        return result[0] if result else None

    def docs(method, signature=None, values=()):
        return bus.call_sync('org.freedesktop.portal.Documents','/org/freedesktop/portal/documents','org.freedesktop.portal.Documents',method,GLib.Variant(signature,values) if signature else None,None,Gio.DBusCallFlags.NONE,5000,None).unpack()

    def revoke_documents():
        for doc_id in docs('List','(s)',('io.github.marius4lui.LinuxDrop.App',))[0]:
            docs('Delete','(s)',(doc_id,))

    # A real sandbox button calls only the host's fixed extension action.
    def notch_button():
        return named('Notch preferences', 'button')
    assert wait(notch_button).queryAction().doAction(0)
    wait(lambda:len(notch_calls)==1)
    wait(lambda:notch_button().getState().contains(pyatspi.STATE_SENSITIVE))
    assert wait(lambda:named('Install and enable the LinuxDrop GNOME extension first', 'label'))
    notch_fail[0]=False
    assert notch_button().queryAction().doAction(0)
    wait(lambda:len(notch_calls)==2)
    wait(lambda:notch_button().getState().contains(pyatspi.STATE_SENSITIVE))
    assert notch_calls==[('linuxdrop@marius4lui.github.io','',{}),('linuxdrop@marius4lui.github.io','',{})],notch_calls

    # A directory already visible inside the sandbox follows the FD-export path.
    private_folder=Path.home()/'.var/app/io.github.marius4lui.LinuxDrop.App/data/Host mapped receive'
    private_folder.mkdir(parents=True,exist_ok=True)
    assert subprocess.run(['flatpak','run','--user','--command=test','io.github.marius4lui.LinuxDrop.App','-d',str(private_folder)]).returncode==0
    assert wait(lambda:named('Save files to','button')).queryAction().doAction(0)
    choose_directory(private_folder)
    wait(lambda:saved_directory()==str(private_folder))
    call('UpdateSettings','(s)',(json.dumps({'receive':{'directory':str(destination)}}),))

    # Receipts grant read only; neither malformed IDs nor file grants are folders.
    for document_id, relative in [('..',''),('', ''),(exported.split('/doc/')[1].split('/')[0], '')]:
        try:
            call('ResolveReceiveDirectory','(ss)',(document_id,relative))
            raise AssertionError('Invalid folder authority was accepted')
        except GLib.Error:pass
    directory_docs=docs('List','(s)',('io.github.marius4lui.LinuxDrop.App',))[0]
    directory_id=next(key for key,value in directory_docs.items() if bytes(value).rstrip(b'\0').decode()==str(destination))
    for relative in ('../escape',destination.name+'/../../escape','/absolute','different-name'):
        try:
            call('ResolveReceiveDirectory','(ss)',(directory_id,relative))
            raise AssertionError('Invalid relative folder was accepted')
        except GLib.Error:pass
    child=destination/'child';child.mkdir()
    assert call('ResolveReceiveDirectory','(ss)',(directory_id,destination.name+'/child'))==str(child)
    (destination/'escape').symlink_to(root,target_is_directory=True)
    try:
        call('ResolveReceiveDirectory','(ss)',(directory_id,destination.name+'/escape'))
        raise AssertionError('Folder symlink escaped its grant')
    except GLib.Error:pass

    # Revoke the grant, then restart both the document portal and host service.
    subprocess.run(['flatpak','kill','io.github.marius4lui.LinuxDrop.App'],check=True)
    app.wait(timeout=10)
    revoke_documents()
    portal_pid=bus.call_sync('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','GetConnectionUnixProcessID',GLib.Variant('(s)',('org.freedesktop.portal.Documents',)),None,Gio.DBusCallFlags.NONE,5000,None).unpack()[0]
    os.kill(portal_pid,signal.SIGTERM)
    wait(lambda:not bus.call_sync('org.freedesktop.DBus','/org/freedesktop/DBus','org.freedesktop.DBus','NameHasOwner',GLib.Variant('(s)',('org.freedesktop.portal.Documents',)),None,Gio.DBusCallFlags.NONE,5000,None).unpack()[0])
    docs('GetMountPoint')
    assert not docs('List','(s)',('io.github.marius4lui.LinuxDrop.App',))[0]
    daemon.terminate();daemon.wait(timeout=10)
    daemon=subprocess.Popen([sys.argv[1]],stdout=log,stderr=subprocess.STDOUT,env={**os.environ,'DBUS_SYSTEM_BUS_ADDRESS':'unix:path=/nonexistent'})
    def snapshot():
        try:return json.loads(call('GetSnapshot'))
        except GLib.Error:return None
    wait(snapshot)
    assert saved_directory()==str(destination)
    call('UpdateSettings','(s)',(json.dumps({'localsend':{'enabled':True,'multicast':False}}),))
    wait(lambda: not snapshot()['restarting'] and any(b['id']=='localsend' and b['state']=='ready' for b in snapshot()['backends']))
    call('SetVisibility','(s)',('everyone',))
    app=subprocess.Popen(['flatpak','run','--user','io.github.marius4lui.LinuxDrop.App'],stdout=app_log,stderr=subprocess.STDOUT)
    wait(lambda:named('Transfers','page tab')).queryAction().doAction(0)
    https=urllib.request.build_opener(urllib.request.ProxyHandler({}),urllib.request.HTTPSHandler(context=ssl._create_unverified_context()))
    def request(route, body):
        # Private test receiver uses its generated certificate; this is not product TLS policy.
        with https.open(urllib.request.Request('https://198.18.0.1:53317/api/localsend/v2'+route,data=body,method='POST',headers={'Content-Type':'application/json'}),timeout=35) as response:return response.read()
    for override in (False,True):
        target=(root/'Per request & directory') if override else destination
        target.mkdir(exist_ok=True)
        filename='overridden.txt' if override else 'persisted.txt'
        payload=b'Actual receive after portal and daemon restart'
        offer={'info':{'alias':'Portal receive fixture','version':'2.1','fingerprint':'f'*64,'port':53317,'protocol':'https','deviceType':'desktop'},'files':{'file':{'id':'file','fileName':filename,'size':len(payload),'fileType':'text/plain'}}}
        with concurrent.futures.ThreadPoolExecutor() as executor:
            pending=executor.submit(request,'/prepare-upload',json.dumps(offer).encode())
            transfer=wait(lambda:next((t for t in snapshot()['transfers'] if t['state']=='waiting'),None))
            assert not (target/filename).exists(), 'Payload saved before consent'
            review=wait(lambda:named('Review and accept','button'));assert review.queryAction().doAction(0)
            wait(lambda:named('Accept selected files','button'))
            if override:
                assert wait(lambda:named('Save files to','button')).queryAction().doAction(0)
                choose_directory(target)
                wait(lambda:named(str(target),'label'))
                revoke_documents()
            assert named('Accept selected files','button').queryAction().doAction(0)
            accepted=json.loads(pending.result(timeout=10))
            request('/upload?'+urllib.parse.urlencode({'sessionId':accepted['sessionId'],'fileId':'file','token':accepted['files']['file']}),payload)
            wait(lambda:next((t for t in snapshot()['transfers'] if t['id']==transfer['id'] and t['state']=='completed'),None))
            assert (target/filename).read_bytes()==payload
            assert saved_directory()==str(destination), 'Per-request folder changed the default'
            if override:assert not (destination/filename).exists()
    print('PASS: installed sandbox selection, portal export, descriptor handoff, exact bytes, link shutdown, receipt actions, persisted and per-request real receive after portal restart', flush=True)
finally:
    subprocess.run(['flatpak', 'kill', 'io.github.marius4lui.LinuxDrop.App'], check=False)
    app.wait(timeout=10)
    daemon.terminate()
    daemon.wait(timeout=10)
