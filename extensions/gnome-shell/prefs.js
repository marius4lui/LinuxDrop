import Adw from 'gi://Adw';
import Gtk from 'gi://Gtk';
import Gdk from 'gi://Gdk';
import Gio from 'gi://Gio';
import {t} from './locale.js';
import {ExtensionPreferences} from 'resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js';

export default class LinuxDropPreferences extends ExtensionPreferences {
    fillPreferencesWindow(window) {
        const settings = this.getSettings();
        const page = new Adw.PreferencesPage({title: t('Desktop notch'), icon_name: 'document-send-symbolic'});
        const display = new Adw.PreferencesGroup({title: t('Sharing controls'), description: t('Sharing controls that stay out of your way.')});
        page.add(display);
        for (const [key, title, subtitle] of [
            ['show-notch', 'Show panel button', 'Click the LinuxDrop icon to open the bubble'],
            ['hide-fullscreen', 'Hide in full screen', 'Keep videos and presentations unobstructed'],
            ['animations', 'Animate transitions', 'Also respects the desktop animation setting'],
        ]) {
            const row = new Adw.SwitchRow({title: t(title), subtitle: t(subtitle)});
            settings.bind(key, row, 'active', Gio.SettingsBindFlags.DEFAULT);
            display.add(row);
        }
        const position = new Adw.PreferencesGroup({title: t('Position and behavior')});
        page.add(position);
        const modes = ['primary', 'pointer', 'fixed'];
        const monitorMode = new Adw.ComboRow({title: t('Open bubble on'), model: Gtk.StringList.new(['Primary monitor', 'Pointer monitor', 'Fixed monitor'].map(t)), selected: Math.max(0, modes.indexOf(settings.get_string('monitor-mode')))});
        monitorMode.connect('notify::selected', () => {
            const mode = modes[monitorMode.selected];
            if (mode && mode !== settings.get_string('monitor-mode')) settings.set_string('monitor-mode', mode);
        });
        position.add(monitorMode);
        let fixedMonitorRow;
        for (const [key, title, subtitle, lower, upper] of [
            ['monitor', 'Fixed monitor index', 'Only used for Fixed monitor. 0 is the first display; −1 or an unavailable display uses the primary monitor.', -1, 32],
            ['top-offset', 'Top spacing', 'Logical pixels below the panel', 0, 200],
            ['auto-collapse', 'Close after inactivity', 'Seconds; 0 keeps the expanded menu open', 0, 120],
            ['drag-hover-delay', 'Drag hover delay', 'Milliseconds before the open bubble accepts file drops', 100, 2000],
        ]) {
            const row = new Adw.SpinRow({title: t(title), subtitle: t(subtitle), adjustment: new Gtk.Adjustment({lower, upper, step_increment: 1, page_increment: 10})});
            settings.bind(key, row, 'value', Gio.SettingsBindFlags.DEFAULT);
            position.add(row);
            if (key === 'monitor') fixedMonitorRow = row;
        }
        const syncMonitorMode = () => {
            const mode = settings.get_string('monitor-mode');
            monitorMode.selected = Math.max(0, modes.indexOf(mode));
            fixedMonitorRow.sensitive = mode === 'fixed';
        };
        syncMonitorMode();
        const monitorChanged = settings.connect('changed::monitor-mode', syncMonitorMode);
        const shortcut = new Adw.ActionRow({title: t('Keyboard shortcut')});
        const shortcutLabel = new Gtk.ShortcutLabel({valign: Gtk.Align.CENTER});
        shortcut.add_suffix(shortcutLabel);
        const choose = new Gtk.Button({label: t('Set shortcut'), valign: Gtk.Align.CENTER});
        shortcut.add_suffix(choose);
        shortcut.activatable_widget = choose;
        position.add(shortcut);
        const syncShortcut = () => {
            shortcutLabel.accelerator = settings.get_string('toggle-shortcut');
            const status = settings.get_string('shortcut-status');
            shortcut.subtitle = t(status === 'active' ? 'Opens or closes the bubble' : status === 'unavailable' ? 'Shortcut is unavailable or already in use; choose another' : status === 'inactive' ? 'Enable the extension to use this shortcut' : 'No shortcut assigned');
        };
        const shortcutChanged = settings.connect('changed::toggle-shortcut', syncShortcut);
        const shortcutStatus = settings.connect('changed::shortcut-status', syncShortcut);
        syncShortcut();
        choose.connect('clicked', () => {
            let selected = settings.get_string('toggle-shortcut');
            const preview = new Gtk.ShortcutLabel({accelerator: selected, halign: Gtk.Align.CENTER});
            const dialog = new Adw.AlertDialog({heading: t('Set shortcut'), body: t('Press a key combination. Escape cancels; Backspace removes the shortcut.'), extra_child: preview});
            dialog.add_response('cancel', t('Cancel'));
            dialog.add_response('apply', t('Apply'));
            dialog.set_close_response('cancel');
            const keys = new Gtk.EventControllerKey();
            keys.set_propagation_phase(Gtk.PropagationPhase.CAPTURE);
            keys.connect('key-pressed', (_, key, _code, state) => {
                if (key === Gdk.KEY_Escape) { dialog.close(); return true; }
                if (key === Gdk.KEY_BackSpace) selected = '';
                else {
                    const modifiers = state & Gtk.accelerator_get_default_mod_mask();
                    const commandModifiers = modifiers & (Gdk.ModifierType.CONTROL_MASK | Gdk.ModifierType.ALT_MASK | Gdk.ModifierType.SUPER_MASK);
                    if (!commandModifiers && [Gdk.KEY_Tab, Gdk.KEY_ISO_Left_Tab, Gdk.KEY_Return, Gdk.KEY_space].includes(key)) return false;
                    if (!Gtk.accelerator_valid(key, modifiers) || !commandModifiers) return true;
                    selected = Gtk.accelerator_name(key, modifiers);
                }
                preview.accelerator = selected;
                return true;
            });
            dialog.add_controller(keys);
            dialog.connect('response', (_, response) => {
                if (response === 'apply') settings.set_string('toggle-shortcut', selected);
            });
            dialog.present(window);
        });
        window.connect('close-request', () => {
            for (const id of [monitorChanged, shortcutChanged, shortcutStatus]) settings.disconnect(id);
            return false;
        });
        const info = new Adw.PreferencesGroup({title: t('Files and privacy'), description: t('Click the LinuxDrop icon in the top panel first. Then choose Drop files or drag files over the open bubble. Click outside or press Escape to close. Requests and progress never open it automatically.')});
        page.add(info);
        window.add(page);
    }
}
