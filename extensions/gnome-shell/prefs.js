import Adw from 'gi://Adw';
import Gtk from 'gi://Gtk';
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
        for (const [key, title, subtitle, lower, upper] of [
            ['monitor', 'Monitor', '−1 follows the primary monitor; otherwise use a monitor index', -1, 32],
            ['top-offset', 'Top spacing', 'Logical pixels below the panel', 0, 200],
            ['auto-collapse', 'Close after inactivity', 'Seconds; 0 keeps the expanded menu open', 0, 120],
        ]) {
            const row = new Adw.SpinRow({title: t(title), subtitle: t(subtitle), adjustment: new Gtk.Adjustment({lower, upper, step_increment: 1, page_increment: 10})});
            settings.bind(key, row, 'value', Gio.SettingsBindFlags.DEFAULT);
            position.add(row);
        }
        const info = new Adw.PreferencesGroup({title: t('Files and privacy'), description: t('Click the LinuxDrop icon in the top panel first. Then choose Drop files or drag files over the open bubble. Click outside or press Escape to close. Requests and progress never open it automatically.')});
        page.add(info);
        window.add(page);
    }
}
