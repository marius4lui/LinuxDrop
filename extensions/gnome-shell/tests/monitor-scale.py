#!/usr/bin/env python3
"""Set and verify actual Mutter scale only in the runner's private session."""
import json
import os
from pathlib import Path
import time

from gi.repository import Gio, GLib

scale = float(os.environ['LINUXDROP_SMOKE_SCALE'])
assert scale in (1.25, 1.5, 2.0)
root = Path(os.environ['LINUXDROP_SMOKE_ROOT'])
assert os.environ['XDG_RUNTIME_DIR'] == str(root / 'runtime')
bus = Gio.bus_get_sync(Gio.BusType.SESSION, None)


def call(method, args=None):
    return bus.call_sync('org.gnome.Mutter.DisplayConfig',
                         '/org/gnome/Mutter/DisplayConfig',
                         'org.gnome.Mutter.DisplayConfig', method, args, None,
                         Gio.DBusCallFlags.NO_AUTO_START, 2000, None).unpack()


for attempt in range(60):
    try:
        serial, monitors, logical, properties = call('GetCurrentState')
        break
    except GLib.Error:
        time.sleep(0.1)
else:
    raise RuntimeError('Private Mutter DisplayConfig unavailable')
assert len(monitors) == 1, monitors
spec, modes, _ = monitors[0]
mode = next(mode for mode in modes if mode[6].get('is-current'))
assert scale in mode[5], f'Requested {scale} unsupported: {mode[5]}'
call('ApplyMonitorsConfig', GLib.Variant('(uua(iiduba(ssa{sv}))a{sv})',
     (serial, 1, [(0, 0, scale, 0, True, [(spec[0], mode[0], {})])], {})))
for attempt in range(30):
    _, _, logical, properties = call('GetCurrentState')
    if len(logical) == 1 and logical[0][2] == scale:
        break
    time.sleep(0.1)
else:
    raise RuntimeError(f'Mutter did not apply scale {scale}: {logical}')
evidence = {'scale': scale, 'physical_size': mode[1:3],
            'logical_size': [round(mode[1] / scale), round(mode[2] / scale)],
            'layout_mode': properties.get('layout-mode'), 'connector': spec[0]}
assert evidence['layout_mode'] == 1, evidence
(root / 'monitor-scale.json').write_text(json.dumps(evidence))
print('LINUXDROP_MONITOR_SCALE: ' + json.dumps(evidence), flush=True)
