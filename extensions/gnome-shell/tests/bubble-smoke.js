// Injected only into the isolated test copy by run-bubble-smoke.sh.
GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1200, () => {
    const check = (condition, message) => { if (!condition) throw new Error(message); };
    const later = callback => GLib.timeout_add(GLib.PRIORITY_DEFAULT, 250, () => {
        Promise.resolve().then(callback).catch(error => console.error(`LINUXDROP_SMOKE_FAILED: ${error.message}\n${error.stack}`));
        return GLib.SOURCE_REMOVE;
    });
    const find = (actor, name) => {
        if (actor instanceof St.Button && actor.accessible_name === t(name)) return actor;
        for (const child of actor.get_children()) { const found = find(child, name); if (found) return found; }
        return null;
    };
    try {
        GLib.source_remove(this._poll); this._poll = 0;
        check(!this._expanded && !this._notch.visible, 'Bubble must start hidden');
        Main.overview.hide();
        this._setExpanded(true);
        later(async () => {
            check(this._notch.visible, 'Explicit open must reveal the bubble');
            check(this._scroll.get_child() === this._body, 'Native ScrollView must own the body');
            check(find(this._body, 'Codes match'), 'Verification action must be available');
            check(this._body.width <= this._notch.width, 'Body must fit the bubble');
            for (const name of ['Codes match', 'Decline', 'Details']) {
                const button = find(this._body, name);
                check(button.width <= this._body.width, `${name} must fit the body`);
            }
            check(find(this._body, 'Previous') && find(this._body, 'Next'), 'Transfer chooser must expose named controls');
            const capture = GLib.getenv('LINUXDROP_SMOKE_CAPTURE');
            if (capture) {
                const output = Gio.File.new_for_path(capture).replace(null, false, Gio.FileCreateFlags.REPLACE_DESTINATION, null);
                try { await new Shell.Screenshot().screenshot(false, output); }
                finally { output.close(null); }
                console.log(`LINUXDROP_SMOKE_CAPTURED: ${capture}`);
            }
            this._selectedTransfer = 'other-transfer'; this._render();
            later(() => {
                const cancel = find(this._body, 'Cancel'); cancel.grab_key_focus();
                const fill = this._progress;
                const peerMenuItem = this._indicator.toggle.devices.box.get_first_child();
                const current = this._snapshot.transfers.find(item => item.id === 'other-transfer');
                current.transferred_bytes = 67; current.files[0].transferred = 67; this._render();
                check(this._progress === fill && global.stage.get_key_focus() === cancel, 'Progress must preserve actor and keyboard focus');
                check(this._indicator.toggle.devices.box.get_first_child() === peerMenuItem, 'Progress must preserve Quick Settings peer actors');
                later(() => {
                    check(Math.abs(fill.width - fill.get_parent().width * 0.67) < 1, `Progress must use the allocated track width: ${fill.width}/${fill.get_parent().width}`);
                    this._actionPending = true; this._render();
                    check(!find(this._body, 'Cancel').reactive, 'Mutation must be disabled while pending');
                    check(find(this._body, 'Details').reactive, 'Details must remain available while pending');
                    check(global.stage.get_key_focus() === this._header, 'Status rebuild must retain focus inside the bubble');
                    this._actionPending = false; this._serviceState = 'offline'; this._snapshot = null; this._render();
                    check(!find(this._body, 'Cancel') && find(this._body, 'Open LinuxDrop'), 'Offline state must replace stale transfer actions');
                    this._setExpanded(false);
                    check(!this._notch.visible && global.stage.get_key_focus() === this._panelButton, 'Close must hide and restore panel focus');
                    console.log('LINUXDROP_SMOKE_PASSED: hidden/open, native scroll, verification, chooser, progress, busy, offline, focus');
                });
            });
        });
    } catch (error) { console.error(`LINUXDROP_SMOKE_FAILED: ${error.message}\n${error.stack}`); }
    return GLib.SOURCE_REMOVE;
});
