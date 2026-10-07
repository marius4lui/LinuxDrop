// Injected into the private extension copy only; no consent is submitted.
GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1000, () => {
    const sleep = ms => new Promise(resolve => GLib.timeout_add(GLib.PRIORITY_DEFAULT, ms, () => { resolve(); return GLib.SOURCE_REMOVE; }));
    const check = (value, message) => { if (!value) throw new Error(message); };
    const find = (actor, label) => {
        if (actor instanceof St.Button && actor.accessible_name === t(label)) return actor;
        for (const child of actor.get_children()) { const match = find(child, label); if (match) return match; }
        return null;
    };
    const log = `${GLib.getenv('LINUXDROP_SMOKE_ROOT')}/orca-debug.log`;
    const speech = () => {
        try { return new TextDecoder().decode(GLib.file_get_contents(log)[1]).split('\n').filter(line => line.includes('SPEECH: Speak')).join('\n'); }
        catch (_) { return ''; }
    };
    (async () => {
        for (let attempts = 0; attempts < 100 && (!speech() || this._serviceState !== 'ready'); attempts++) await sleep(100);
        check(speech() && this._serviceState === 'ready', 'Orca and private service must be ready');
        GLib.source_remove(this._poll); this._poll = 0;
        Main.overview.hide();
        await sleep(300);
        const keyboard = Clutter.get_default_backend().get_default_seat().create_virtual_device(Clutter.InputDeviceType.KEYBOARD_DEVICE);
        const key = async code => {
            keyboard.notify_key(GLib.get_monotonic_time(), code, Clutter.KeyState.PRESSED);
            keyboard.notify_key(GLib.get_monotonic_time(), code, Clutter.KeyState.RELEASED);
            await sleep(250);
        };
        this._panelButton.grab_key_focus();
        await sleep(400);
        await key(28); // Enter opens the focused panel button.
        check(this._expanded, 'Panel Enter must open the Notch');
        for (const label of ['Previous transfer', 'Next transfer', 'Decline', 'Codes match', 'Details', 'Drop files', 'Settings']) {
            for (let attempts = 0; attempts < 20 && global.stage.get_key_focus() !== find(this._body, label); attempts++) await key(15);
            check(global.stage.get_key_focus() === find(this._body, label), `Tab must reach ${label}`);
            await sleep(150);
        }
        find(this._body, 'Next transfer').grab_key_focus();
        await key(57); // Space chooses a transfer, never accepts it.
        check(this._selectedTransfer === 'other-transfer', 'Space must change selected transfer');
        check(this._progress?.get_parent().get_accessible().get_role() === Atk.Role.PROGRESS_BAR, 'Progress must expose its role');
        check(this._progress.get_parent().accessible_name.includes('43%'), 'Progress must expose its percentage');
        find(this._body, 'Cancel').grab_key_focus();
        await sleep(400);
        await key(1); // Escape dismisses without cancelling the transfer.
        check(!this._expanded, 'Escape must dismiss the Notch');
        for (const label of ['Close LinuxDrop', 'Previous transfer', 'Next transfer', 'Codes match', 'Details', 'Settings', 'Cancel'])
            check(speech().includes(t(label)), `Orca must speak ${label}`);
        const request = this._snapshot.transfers.find(transfer => transfer.id === 'verification');
        check(speech().includes(request.peer_name) && speech().includes(request.verification_code), 'Orca must announce the request identity and comparison code');
        check(speech().includes('43%'), 'Orca must announce the selected transfer progress');
        console.log('LINUXDROP_SMOKE_PASSED: real Orca panel/Notch speech, Tab navigation, transfer selection, progress role/value label and Escape');
    })().catch(error => console.error(`LINUXDROP_SMOKE_FAILED: ${error.message}\n${error.stack}`));
    return GLib.SOURCE_REMOVE;
});
