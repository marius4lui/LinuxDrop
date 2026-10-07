// Run only with the private settings and display created by run-bubble-smoke.sh.
import Adw from 'gi://Adw';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Gtk from 'gi://Gtk';
import Gdk from 'gi://Gdk';
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
window.present(); flush();
const shortcut = find(window, 'Keyboard shortcut');
check(shortcut instanceof Adw.ActionRow, 'Shortcut preference must exist');
shortcut.activatable_widget.emit('clicked'); flush();
let dialog = window.get_visible_dialog();
check(dialog instanceof Adw.AlertDialog, 'Shortcut capture must be a native dialog');
const controllers = dialog.observe_controllers();
let keys;
for (let index = 0; index < controllers.get_n_items(); index++) {
    const controller = controllers.get_item(index);
    if (controller instanceof Gtk.EventControllerKey) keys = controller;
}
check(keys, 'Shortcut capture needs a key controller');
check(!keys.emit('key-pressed', Gdk.KEY_Tab, 0, 0), 'Tab must remain available for keyboard navigation');
keys.emit('key-pressed', Gdk.KEY_d, 0, Gdk.ModifierType.CONTROL_MASK | Gdk.ModifierType.ALT_MASK);
dialog.emit('response', 'apply'); dialog.force_close(); flush();
check(settings.get_string('toggle-shortcut') === Gtk.accelerator_name(Gdk.KEY_d, Gdk.ModifierType.CONTROL_MASK | Gdk.ModifierType.ALT_MASK), 'Captured shortcut must persist');
settings.set_string('shortcut-status', 'unavailable'); flush();
check(shortcut.subtitle === t('Shortcut is unavailable or already in use; choose another'), 'Conflict feedback must appear in preferences');
shortcut.activatable_widget.emit('clicked'); flush();
dialog = window.get_visible_dialog();
const removalControllers = dialog.observe_controllers();
for (let index = 0; index < removalControllers.get_n_items(); index++) {
    const controller = removalControllers.get_item(index);
    if (controller instanceof Gtk.EventControllerKey) controller.emit('key-pressed', Gdk.KEY_BackSpace, 0, 0);
}
dialog.emit('response', 'apply'); dialog.force_close(); flush();
check(settings.get_string('toggle-shortcut') === '', 'Backspace must remove the shortcut');
window.emit('close-request');
window.destroy();
print('LINUXDROP_PREFS_SMOKE_PASSED: monitor dependency, external synchronization, native shortcut capture/removal and conflict feedback');
