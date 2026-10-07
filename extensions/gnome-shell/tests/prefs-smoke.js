// Run only with the private settings and display created by run-bubble-smoke.sh.
import Adw from 'gi://Adw';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import {t} from '../locale.js';

Gio.resources_register(Gio.Resource.load('/usr/share/gnome-shell/org.gnome.Shell.Extensions.src.gresource'));
const extensionPath = ARGV[0];
const {default: Preferences} = await import(GLib.filename_to_uri(`${extensionPath}/prefs.js`, null));
const [, bytes] = GLib.file_get_contents(`${extensionPath}/metadata.json`);
const metadata = JSON.parse(new TextDecoder().decode(bytes));
metadata.path = extensionPath;
metadata.dir = Gio.File.new_for_path(extensionPath);
Adw.init();
const prefs = new Preferences(metadata);
const settings = prefs.getSettings();
const window = new Adw.PreferencesWindow();
prefs.fillPreferencesWindow(window);
const find = (widget, title) => {
    if (widget instanceof Adw.PreferencesRow && widget.title === t(title)) return widget;
    for (let child = widget.get_first_child(); child; child = child.get_next_sibling()) {
        const result = find(child, title);
        if (result) return result;
    }
    return null;
};
const check = (condition, message) => { if (!condition) throw new Error(message); };
const flush = () => { while (GLib.MainContext.default().pending()) GLib.MainContext.default().iteration(false); };
const mode = find(window, 'Open bubble on');
const index = find(window, 'Fixed monitor index');
check(mode instanceof Adw.ComboRow && index instanceof Adw.SpinRow, 'Native monitor controls must exist');
check(!index.sensitive, 'Fixed index must be disabled in primary mode');
mode.selected = 2; flush();
check(settings.get_string('monitor-mode') === 'fixed' && index.sensitive, 'Fixed selection must enable its index');
settings.set_string('monitor-mode', 'pointer'); flush();
check(mode.selected === 1 && !index.sensitive, 'External mode changes must update both native controls');
settings.set_string('monitor-mode', 'fixed'); flush();
check(mode.selected === 2 && index.sensitive, 'Returning to fixed mode must restore the index control');
window.emit('close-request');
window.destroy();
print('LINUXDROP_PREFS_SMOKE_PASSED: fixed monitor dependency and external settings synchronization');
