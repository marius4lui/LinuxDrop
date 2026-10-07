#!/usr/bin/env python3
"""Real Orca/AT-SPI and keyboard traversal, against explicitly simulated peers."""
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import pyatspi

assert os.environ.get('LINUXDROP_PRIVATE_A11Y') == '1', 'Use run-orca-smoke.sh'
root = Path(__file__).resolve().parents[3]
out = Path(os.environ['LINUXDROP_A11Y_OUTPUT']).resolve()
out.mkdir(parents=True, exist_ok=True)
(out/'RESULT.json').unlink(missing_ok=True)
children, handles = [], []

def launch(args, name):
    handle = open(out / (name + '.log'), 'w')
    handles.append(handle)
    child = subprocess.Popen(args, stdout=handle, stderr=subprocess.STDOUT)
    children.append(child)
    return child

def wait(check, message, seconds=20):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        result = check()
        if result: return result
        time.sleep(.1)
    raise AssertionError(message)

def descendants(node, depth=0):
    if depth > 40: return
    yield node
    try:
        for child in node:
            yield from descendants(child, depth+1)
    except Exception:
        # GTK rebuilds rows after selection and settings changes.
        return

def find(name, role=None):
    for node in descendants(pyatspi.Registry.getDesktop(0)):
        try:
            if (name is None or node.name == name) and (role is None or node.getRoleName() == role) and node.getState().contains(pyatspi.STATE_SHOWING):
                return node
        except Exception: pass
    return None

def key(value):
    subprocess.run(['xdotool', 'key', '--clearmodifiers', value], check=True, timeout=3)
    time.sleep(.15)

def focus(name, role=None):
    wait(lambda: find(name, role), 'Missing accessible control: '+name)
    for _ in range(60):
        node = find(name, role)
        if node and node.getState().contains(pyatspi.STATE_FOCUSED): return node
        key('Tab')
    raise AssertionError('Control is not reachable with Tab: '+name)

try:
    launch(['openbox'], 'window-manager')
    fixture = launch(['python3', '-u', str(root / 'app/linuxdrop/tests/shell-review-fixture.py'), '--a11y'], 'fixture')
    wait(lambda: 'fixture-ready' in (out/'fixture.log').read_text(), 'Private-bus fixture failed')
    (out/'orca-debug.log').unlink(missing_ok=True)
    orca = launch(['orca', '--enable=speech', '--disable=braille', '--debug-file', str(out/'orca-debug.log')], 'orca')
    wait(lambda: (out/'orca-debug.log').exists() and 'SPEECH: Speak' in (out/'orca-debug.log').read_text(), 'Orca did not initialize')
    app = launch([str(Path(sys.argv[1]).resolve())], 'application')
    def window_id():
        assert app.poll() is None, 'Application exited during accessibility startup'
        result = subprocess.run(['xdotool','search','--pid',str(app.pid),'--name','LinuxDrop'],capture_output=True,text=True,timeout=3)
        return result.stdout.splitlines()[0] if result.returncode == 0 else None
    window = wait(window_id, 'Application window did not map', 30)
    subprocess.run(['xdotool','windowactivate','--sync',window],check=True,timeout=25)
    focus('Dateien auswählen', 'push button')
    for _ in range(3): key('Tab')
    peer = wait(lambda: find('Orca Testgerät', 'toggle button'), 'Peer has no accessible name')
    assert peer.getState().contains(pyatspi.STATE_FOCUSED), 'Tab must reach the named device'
    assert 'LocalSend' in peer.description and 'Quick Share' in peer.description
    key('space')
    wait(lambda: find('Orca Testgerät', 'toggle button').getState().contains(pyatspi.STATE_PRESSED), 'Selected peer is not exposed as pressed')
    key('ctrl+2')
    progress = wait(lambda: find('Übertragungsfortschritt: Zweites Prüfgerät','progress bar'), 'Transfer progress is unnamed')
    value = progress.queryValue()
    assert abs((value.currentValue-value.minimumValue)/(value.maximumValue-value.minimumValue)-.43) < .01
    focus('Prüfen und annehmen','push button')
    # Review is reachable, but no simulated acceptance is sent to the fixture.
    key('ctrl+3')
    wait(lambda: find('Erkannte Hardware'), 'Hardware view did not open')
    key('ctrl+4')
    wait(lambda: find('Einstellungen suchen'), 'Settings search lacks its translated accessible name')
    key('ctrl+1')
    focus('Tastenkürzel','push button')
    key('space')
    wait(lambda: find(None,'dialog'), 'Native keyboard help is not exposed as a dialog')
    key('Escape')
    wait(lambda: find(None,'dialog') is None, 'Escape did not dismiss keyboard help')
    spoken = '\n'.join(line for line in (out/'orca-debug.log').read_text().splitlines() if 'SPEECH: Speak' in line)
    for label in ['Geräte verwalten','Geräte in der Nähe aktualisieren','Orca Testgerät','Tastenkürzel','Prüfen und annehmen']:
        assert label in spoken, 'Orca did not generate speech for '+label
    assert orca.poll() is None and fixture.poll() is None
    (out/'speech-evidence.txt').write_text(spoken)
    report = dict(status='passed', runtime='GTK4, AT-SPI, Openbox/X11 private session',
        orca_version=subprocess.check_output(['orca','--version'],text=True).strip(),
        checks=['actual Tab/Space navigation and named device selection', 'selected state and protocol description',
                'numeric transfer progress through AT-SPI', 'incoming review reachable without implicit acceptance',
                'four views and named settings search', 'native keyboard dialog and Escape dismissal',
                'real Orca speech generation for focused controls'],
        limits=['No physical audio output or Braille display assessed', 'GNOME Shell traversal remains separate'])
    (out/'RESULT.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report),flush=True)
finally:
    for child in reversed(children): child.terminate()
    for child in children:
        try: child.wait(timeout=5)
        except subprocess.TimeoutExpired: child.kill(); child.wait()
    for handle in handles: handle.close()
