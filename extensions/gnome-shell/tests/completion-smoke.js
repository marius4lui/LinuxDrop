// Only injected into the isolated desktop fixture.
GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1500, () => {
    const sleep = ms => new Promise(resolve => GLib.timeout_add(GLib.PRIORITY_DEFAULT, ms, () => { resolve(); return GLib.SOURCE_REMOVE; }));
    const check = (value, message) => { if (!value) throw new Error(message); };
    const find = (actor, label) => {
        if (actor instanceof St.Button && actor.accessible_name === t(label)) return actor;
        for (const child of actor.get_children()) { const match = find(child, label); if (match) return match; }
        return null;
    };
    const locations = () => new Promise(resolve => Gio.DBus.session.call(
        'org.freedesktop.FileManager1', '/org/freedesktop/FileManager1',
        'org.freedesktop.DBus.Properties', 'Get',
        new GLib.Variant('(ss)', ['org.freedesktop.FileManager1', 'OpenLocations']),
        new GLib.VariantType('(v)'), Gio.DBusCallFlags.NO_AUTO_START, 1000, null,
        (bus, result) => { try { resolve(bus.call_finish(result).deep_unpack()[0].deep_unpack()); } catch (_) { resolve([]); } }));
    (async () => {
        for (let count = 0; count < 50 && this._serviceState !== 'ready'; count++) await sleep(100);
        check(this._serviceState === 'ready', 'Private service must be ready');
        GLib.source_remove(this._poll); this._poll = 0;
        const manager = Gio.DesktopAppInfo.new('org.gnome.Nautilus.desktop');
        check(manager, 'Nautilus is required for the real folder-launch check');
        manager.set_as_default_for_type('inode/directory'); // Private XDG_CONFIG_HOME.
        const folder = `${GLib.getenv('LINUXDROP_SMOKE_ROOT')}/received files`;
        GLib.mkdir_with_parents(folder, 0o700);
        const saved = `${folder}/photo with spaces.txt`;
        GLib.file_set_contents(saved, 'Explicitly simulated received file.');
        Main.overview.hide();
        await sleep(250);
        this._setExpanded(true);
        const transfer = this._snapshot.transfers.find(item => item.id === 'other-transfer');
        transfer.state = 'completed'; transfer.direction = 'incoming'; transfer.saved_paths = [saved];
        this._selectedTransfer = transfer.id;
        this._render();
        const open = find(this._body, 'Open folder');
        check(open && !find(this._body, 'Cancel'), 'Completion must offer folder access without Cancel');
        open.grab_key_focus();
        const keyboard = Clutter.get_default_backend().get_default_seat().create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
        keyboard.notify_key(GLib.get_monotonic_time(), 57, Clutter.KeyState.PRESSED);
        keyboard.notify_key(GLib.get_monotonic_time(), 57, Clutter.KeyState.RELEASED);
        const uri = Gio.File.new_for_path(folder).get_uri();
        let opened = false;
        for (let count = 0; count < 80; count++) {
            if ((await locations()).includes(uri) && global.get_window_actors().some(actor => actor.visible && actor.meta_window.get_gtk_application_id() === 'org.gnome.Nautilus')) { opened = true; break; }
            await sleep(100);
        }
        check(opened, 'Real Nautilus must expose the exact received folder and a visible window');
        check(!this._expanded && !this._actionError, 'Successful folder launch must dismiss the Bubble');
        console.log('LINUXDROP_SMOKE_PASSED: completed transfer opens real Nautilus at the exact space-containing destination through GIO and keyboard activation');
    })().catch(error => console.error(`LINUXDROP_SMOKE_FAILED: ${error.message}\n${error.stack}`));
    return GLib.SOURCE_REMOVE;
});
