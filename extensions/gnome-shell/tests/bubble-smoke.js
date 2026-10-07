// Injected only into the isolated test copy by run-bubble-smoke.sh.
GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1200, () => {
    const check = (condition, message) => { if (!condition) throw new Error(message); };
    const settle = () => new Promise(resolve => GLib.timeout_add(GLib.PRIORITY_DEFAULT, 250, () => { resolve(); return GLib.SOURCE_REMOVE; }));
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
            check(find(this._body, 'Previous transfer') && find(this._body, 'Next transfer'), 'Transfer chooser must expose specific named controls');
            const capture = GLib.getenv('LINUXDROP_SMOKE_CAPTURE');
            if (capture) {
                const output = Gio.File.new_for_path(capture).replace(null, false, Gio.FileCreateFlags.REPLACE_DESTINATION, null);
                try { await new Shell.Screenshot().screenshot(false, output); }
                finally { output.close(null); }
                console.log(`LINUXDROP_SMOKE_CAPTURED: ${capture}`);
            }
            this._scroll.set_style('max-height: 120px;');
            await settle();
            const details = find(this._body, 'Details'); details.grab_key_focus();
            await settle();
            const [, actionY] = details.get_transformed_position();
            const [, scrollY] = this._scroll.get_transformed_position();
            check(actionY >= scrollY && actionY + details.height <= scrollY + this._scroll.height + 1, 'Keyboard-focused action must scroll fully into view');
            const first = find(this._body, 'Previous transfer'); first.grab_key_focus();
            await settle();
            const [, firstY] = first.get_transformed_position();
            check(firstY >= scrollY && firstY + first.height <= scrollY + this._scroll.height + 1, 'Keyboard navigation must also scroll back to the transfer chooser');
            this._position();
            await settle();
            let next = find(this._body, 'Next transfer'); next.grab_key_focus(); next.emit('clicked', 1);
            check(this._selectedTransfer === 'other-transfer' && global.stage.get_key_focus() === find(this._body, 'Next transfer'), 'Next must preserve keyboard focus on the rebuilt chooser');
            const previous = find(this._body, 'Previous transfer'); previous.grab_key_focus(); previous.emit('clicked', 1);
            check(this._selectedTransfer === 'verification' && global.stage.get_key_focus() === find(this._body, 'Previous transfer'), 'Previous must preserve keyboard focus on the rebuilt chooser');
            next = find(this._body, 'Next transfer'); next.grab_key_focus(); next.emit('clicked', 1);
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
                    this._actionPending = false;
                    current.state = 'pin_required'; this._render();
                    check(this._detail.text === t('Enter PIN') && find(this._body, 'Enter PIN'), 'PIN state must explain the next action in the header and body');
                    current.state = 'failed';
                    current.error = 'Network helper is unavailable. The sharing service stopped; radio cleanup may still be running. Reconnect the adapter and restart sharing services.';
                    this._render();
                    check(this._body.get_children().some(child => child instanceof St.Label && child.text === t(current.error)), 'Helper failure must show its translated recovery explanation');
                    check(!find(this._body, 'Cancel') && find(this._body, 'Details') && find(this._body, 'Done'), 'Failed transfer must replace stale cancellation with recovery navigation');
                    const launchApp = this._launchApp;
                    this._launchApp = () => { throw new Error('Launch failure for smoke test'); };
                    try {
                        this.openApp('--transfers');
                        check(this._expanded && this._notch.visible && this._selectedTransfer === current.id, 'Launch failure must restore the bubble and selected transfer');
                        check(this._body.get_children().some(child => child instanceof St.Label && child.text === 'Launch failure for smoke test'), 'Launch failure must remain visible with retry navigation');
                        check(find(this._body, 'Details').reactive, 'Failed launch must allow retry');
                    } finally { this._launchApp = launchApp; }
                    this._actionPending = false; this._serviceState = 'offline'; this._snapshot = null; this._render();
                    check(!find(this._body, 'Cancel') && find(this._body, 'Open LinuxDrop'), 'Offline state must replace stale transfer actions');
                    const realProxy = this._proxy;
                    const requests = [];
                    let owner = ':smoke.old';
                    const snapshot = revision => ({epoch: owner, revision, peers: [], backends: [], transfers: [{id: 'reconnect', state: 'verification', direction: 'incoming', peer_name: 'Reconnect peer', verification_code: '123456', files: [{name: 'review.txt', size: 10}]}]});
                    const reply = (request, value) => request.callback(this._proxy, {deep_unpack: () => [JSON.stringify(value)]});
                    this._proxy = {
                        get_name_owner: () => owner,
                        call: (method, _parameters, _flags, _timeout, _cancellable, callback) => requests.push({method, callback}),
                        call_finish: result => result,
                    };
                    try {
                        this._ownerChanged();
                        check(this._serviceState === 'connecting' && !find(this._body, 'Codes match'), 'Connecting must not expose stale consent');
                        const firstRead = requests.shift();
                        this._refresh(); this._refresh();
                        check(requests.length === 0, 'Concurrent changes must coalesce behind the pending snapshot');
                        reply(firstRead, snapshot(1));
                        check(requests.length === 1 && find(this._body, 'Codes match'), 'Dirty snapshot must immediately refresh while restoring current consent');
                        const oldRead = requests.shift();
                        this.call('AcceptTransfer', new GLib.Variant('(s)', ['reconnect']));
                        const oldAction = requests.shift();
                        check(this._actionPending, 'Consent action must enter pending state');
                        owner = null; this._ownerChanged();
                        check(!this._snapshot && !this._actionPending && this._serviceState === 'offline' && !find(this._body, 'Codes match'), 'Owner loss must immediately retire pending consent and transfer data');
                        reply(oldRead, snapshot(99));
                        check(!this._snapshot && this._serviceState === 'offline', 'Late old snapshot must not resurrect disconnected transfers');
                        owner = ':smoke.new'; this._ownerChanged();
                        const newRead = requests.shift();
                        reply(newRead, snapshot(1));
                        check(this._serviceState === 'ready' && find(this._body, 'Codes match'), 'New owner must recover without requiring the polling timer');
                        this.call('AcceptTransfer', new GLib.Variant('(s)', ['reconnect']));
                        const newAction = requests.shift();
                        oldAction.callback(this._proxy, {deep_unpack: () => []});
                        check(this._actionPending && requests.length === 0, 'Old action completion must not unlock a new owner action');
                        newAction.callback(this._proxy, {deep_unpack: () => []});
                        check(!this._actionPending && requests.length === 1, 'Current action completion must refresh normally');
                        reply(requests.shift(), {...snapshot(2), transfers: []});
                        check(!find(this._body, 'Codes match') && find(this._body, 'Drop files'), 'Fresh empty state must retire the completed request');
                    } finally { this._proxy = realProxy; }
                    this._setExpanded(false);
                    check(!this._notch.visible && global.stage.get_key_focus() === this._panelButton, 'Close must hide and restore panel focus');
                    console.log('LINUXDROP_SMOKE_PASSED: hidden/open, keyboard scrolling, verification, chooser focus, PIN, progress, busy, helper failure, launch recovery, offline, owner replacement, stale consent callbacks, coalesced refresh, focus');
                });
            });
        });
    } catch (error) { console.error(`LINUXDROP_SMOKE_FAILED: ${error.message}\n${error.stack}`); }
    return GLib.SOURCE_REMOVE;
});
