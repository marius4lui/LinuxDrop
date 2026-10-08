// Test-only extension: operates the real GNOME/Polkit dialog in a disposable session.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Shell from 'gi://Shell';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as ModalDialog from 'resource:///org/gnome/shell/ui/modalDialog.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

export default class PolkitProbe extends Extension {
    enable() {
        this._handled = false;
        this._ready = false;
        this._timer = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 100, () => {
            const agent = Main.componentManager?._allComponents.polkitAgent;
            if (!agent)
                return GLib.SOURCE_CONTINUE;
            if (!this._ready) {
                if (Main.layoutManager._startingUp)
                    return GLib.SOURCE_CONTINUE;
                if (Main.overview.visible) {
                    Main.overview.hide();
                    return GLib.SOURCE_CONTINUE;
                }
                this._ready = true;
                console.log('LINUXDROP_GUI_READY');
            }
            const dialog = agent._currentDialog;
            if (!this._handled && dialog?.state === ModalDialog.State.OPENED &&
                dialog._passwordEntry.visible && dialog._passwordEntry.reactive) {
                this._handled = true;
                this._respond(dialog).catch(() => console.error('LINUXDROP_GUI_FAILED'));
            }
            return GLib.SOURCE_CONTINUE;
        });
    }

    async _respond(dialog) {
        if (dialog.actionId !== 'io.github.marius4lui.LinuxDrop.manage-radio' ||
            !dialog.message.includes('LinuxDrop') || dialog._passwordEntry.password_visible ||
            global.stage.get_key_focus() !== dialog._passwordEntry.clutter_text)
            throw new Error('Unexpected authorization dialog');
        // Wait for a rendered frame before capturing; the password is still empty.
        await new Promise(resolve => GLib.timeout_add(GLib.PRIORITY_DEFAULT, 300, () => {
            resolve();
            return GLib.SOURCE_REMOVE;
        }));
        const output = Gio.File.new_for_path(GLib.getenv('LINUXDROP_GUI_CAPTURE'))
            .replace(null, false, Gio.FileCreateFlags.PRIVATE, null);
        try { await new Shell.Screenshot().screenshot(false, output); }
        finally { output.close(null); }
        if (GLib.getenv('LINUXDROP_GUI_MODE') === 'cancel') {
            dialog.cancel();
        } else {
            dialog._passwordEntry.set_text(GLib.getenv('LINUXDROP_GUI_TEST_PASSWORD'));
            if (!dialog._okButton.reactive)
                throw new Error('Authentication button did not enable');
            dialog._passwordEntry.clutter_text.emit('activate');
        }
        console.log('LINUXDROP_GUI_SUBMITTED');
    }

    disable() {
        if (this._timer)
            GLib.source_remove(this._timer);
        this._timer = 0;
    }
}
