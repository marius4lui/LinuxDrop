"""Isolated real GNOME agent; no mocked logind/Polkit or authentication rules."""
import getpass
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time


class GraphicalAgent:
    def __init__(self):
        self.process = None
        self.directory = tempfile.TemporaryDirectory(prefix='linuxdrop-gnome-auth-')
        self.root = Path(self.directory.name)
        self.log = self.root / 'shell.log'
        self.output = None

    def start(self, mode, capture):
        # getpass disables echo in the inner systemd PTY too. Only the synthetic
        # disposable account password is passed in process memory/environment.
        password = getpass.getpass('LINUXDROP_GUI_PASSWORD:')
        env = dict(os.environ, XDG_CONFIG_HOME=str(self.root / 'config'),
            XDG_DATA_HOME=str(self.root / 'data'), XDG_CACHE_HOME=str(self.root / 'cache'),
            GSETTINGS_BACKEND='keyfile', LIBGL_ALWAYS_SOFTWARE='1', GSK_RENDERER='cairo',
            XDG_CURRENT_DESKTOP='GNOME', XDG_SESSION_TYPE='wayland')
        env['LC_ALL'] = env['LANG'] = os.environ.get('LINUXDROP_GUI_LANGUAGE', 'C.UTF-8')
        extension = self.root / 'data/gnome-shell/extensions/linuxdrop-auth-test@example.invalid'
        extension.mkdir(parents=True)
        (extension / 'metadata.json').write_text(json.dumps({'uuid': extension.name,
            'name': 'LinuxDrop disposable authorization probe', 'description': 'Test only',
            'shell-version': ['46']}))
        (extension / 'extension.js').write_bytes(Path(__file__).with_name('gnome_polkit.js').read_bytes())
        for key, value in [('org.gnome.shell enabled-extensions', "['linuxdrop-auth-test@example.invalid']"),
            ('org.gnome.desktop.interface enable-animations', 'false'),
            ('org.gnome.desktop.notifications show-banners', 'false'),
            ('org.gnome.desktop.session idle-delay', '0')]:
            subprocess.run(['gsettings', 'set', *key.split(), value], env=env,
                check=True, capture_output=True, timeout=5)
        # Refuse to replace an existing shell on this test user's bus.
        present = subprocess.check_output(['busctl', '--user', 'call', 'org.freedesktop.DBus',
            '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'NameHasOwner', 's', 'org.gnome.Shell'], text=True)
        assert present.strip() == 'b false', 'Existing desktop must not be replaced'
        env.update(LINUXDROP_GUI_MODE=mode, LINUXDROP_GUI_CAPTURE=capture,
            LINUXDROP_GUI_TEST_PASSWORD=password)
        self.output = self.log.open('w')
        self.process = subprocess.Popen(['gnome-shell', '--headless', '--wayland', '--no-x11',
            '--wayland-display=linuxdrop-auth-test', '--virtual-monitor=1280x800'], env=env,
            stdout=self.output, stderr=subprocess.STDOUT)
        # Do not print shell logs: unrelated components can contain session data.
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise RuntimeError('Isolated GNOME stopped')
            if 'LINUXDROP_GUI_READY' in self.log.read_text():
                return
            time.sleep(0.1)
        raise RuntimeError('Isolated GNOME did not register its probe')

    def verify(self):
        log = self.log.read_text()
        assert 'LINUXDROP_GUI_SUBMITTED' in log and 'LINUXDROP_GUI_FAILED' not in log

    def close(self):
        if self.process is not None:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        if self.output is not None:
            self.output.close()
        self.directory.cleanup()
