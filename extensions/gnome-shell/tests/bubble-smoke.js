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
    const capture = async path => {
        if (!path) return;
        const output = Gio.File.new_for_path(path).replace(null, false, Gio.FileCreateFlags.REPLACE_DESTINATION, null);
        try { await new Shell.Screenshot().screenshot(false, output); }
        finally { output.close(null); }
        console.log(`LINUXDROP_SMOKE_CAPTURED: ${path}`);
    };
    try {
        GLib.source_remove(this._poll); this._poll = 0;
        check(!this._expanded && !this._notch.visible, 'Bubble must start hidden');
        Main.overview.hide();
        this._setExpanded(true);
        later(async () => {
            check(this._notch.visible, 'Explicit open must reveal the bubble');
            this._setExpanded(false);
            Main.overview.show();
            await settle();
            check(Main.overview.visible && !this._notch.visible, 'Overview alone must leave the bubble closed');
            const pointer = Clutter.get_default_backend().get_default_seat().create_virtual_device(Clutter.InputDeviceType.POINTER_DEVICE);
            const clickPanel = () => {
                const [x, y] = this._panelButton.get_transformed_position();
                const [width, height] = this._panelButton.get_transformed_size();
                pointer.notify_absolute_motion(GLib.get_monotonic_time(), x + width / 2, y + height / 2);
                pointer.notify_button(GLib.get_monotonic_time(), 1, Clutter.ButtonState.PRESSED);
                pointer.notify_button(GLib.get_monotonic_time(), 1, Clutter.ButtonState.RELEASED);
            };
            clickPanel();
            await settle();
            check(!Main.overview.visible && this._expanded && this._notch.visible, 'Panel click from Overview must dismiss Overview and open the bubble');
            check(global.stage.get_key_focus() === this._header, 'Overview handoff must focus the visible bubble header');
            Main.overview.show();
            await settle();
            Main.overview.hide();
            await settle();
            check(!this._expanded && !this._notch.visible, 'Unrequested Overview transitions must not reopen the bubble');
            Main.overview.show();
            await settle();
            this._setExpanded(true);
            check(this._openAfterOverview, 'Opening from Overview must wait for its modal focus release');
            this._setExpanded(false);
            await settle();
            check(!this._expanded && !this._openAfterOverview && !this._overviewOpenIdle, 'Cancelling the queued activation must leave the bubble closed');
            clickPanel();
            await settle();
            check(this._expanded && this._notch.visible, 'A later panel click must remain available after cancellation');
            check(this._scroll.get_child() === this._body, 'Native ScrollView must own the body');
            check(find(this._body, 'Codes match'), 'Verification action must be available');
            check(find(this._body, 'Drop files') && find(this._body, 'Settings'), 'Incoming verification must retain send and settings access');
            check(this._body.width <= this._notch.width, 'Body must fit the bubble');
            for (const name of ['Codes match', 'Decline', 'Details']) {
                const button = find(this._body, name);
                check(button.width <= this._body.width, `${name} must fit the body`);
            }
            check(find(this._body, 'Previous transfer') && find(this._body, 'Next transfer'), 'Transfer chooser must expose specific named controls');
            await capture(GLib.getenv('LINUXDROP_SMOKE_CAPTURE'));
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
                const peerMenuItem = this._indicator.toggle.devices.box.get_first_child();
                const current = this._snapshot.transfers.find(item => item.id === 'other-transfer');
                const extra = {id: 'unrelated', state: 'waiting', direction: 'outgoing', peer_name: 'Other device', files: []};
                this._snapshot.transfers.push(extra); this._render();
                check(global.stage.get_key_focus() === find(this._body, 'Cancel'), 'An unrelated transfer arriving must preserve the selected transfer action focus');
                this._snapshot.transfers.pop(); this._render();
                check(global.stage.get_key_focus() === find(this._body, 'Cancel'), 'An unrelated transfer leaving must preserve the selected transfer action focus');
                const originalName = current.peer_name;
                const originalFile = current.files[0].name;
                current.peer_name = 'Settings'; current.files[0].name = 'Details'; this._render();
                check(this._body.get_children().some(child => child instanceof St.Label && child.text === 'Settings'), 'External peer names must remain literal in every locale');
                check(this._body.get_children().some(child => child instanceof St.Label && child.text === 'Details'), 'External filenames must remain literal in every locale');
                current.peer_name = originalName; current.files[0].name = originalFile; this._render();
                check(find(this._body, 'Drop files') && find(this._body, 'Settings'), 'Active transfers must retain send and settings access');
                const stableCancel = find(this._body, 'Cancel');
                const stableFill = this._progress;
                current.transferred_bytes = 67; current.files[0].transferred = 67; this._render();
                check(this._progress === stableFill && global.stage.get_key_focus() === stableCancel, 'Progress must preserve actor and keyboard focus');
                check(this._indicator.toggle.devices.box.get_first_child() === peerMenuItem, 'Progress must preserve Quick Settings peer actors');
                later(async () => {
                    check(Math.abs(stableFill.width - stableFill.get_parent().width * 0.67) < 1, `Progress must use the allocated track width: ${stableFill.width}/${stableFill.get_parent().width}`);
                    check(Math.abs(stableFill.get_transformed_position()[0] - stableFill.get_parent().get_transformed_position()[0]) < 1, 'Progress must begin at the left edge, not the center of its track');
                    const settings = find(this._body, 'Settings'); settings.grab_key_focus();
                    await settle();
                    const [, actionY] = settings.get_transformed_position();
                    const [, scrollY] = this._scroll.get_transformed_position();
                    check(actionY >= scrollY && actionY + settings.height <= scrollY + this._scroll.height + 1, 'Persistent settings action must remain reachable in the constrained viewport');
                    await capture(GLib.getenv('LINUXDROP_SMOKE_CAPTURE_PROGRESS'));
                    find(this._body, 'Cancel').grab_key_focus();
                    this._actionPending = true; this._render();
                    check(!find(this._body, 'Cancel').reactive, 'Mutation must be disabled while pending');
                    check(this._indicator.toggle.subtitle === t('Working…') && !this._indicator.toggle.reactive, 'Quick Settings must expose pending state and prevent repeated mutations');
                    check(find(this._body, 'Details').reactive, 'Details must remain available while pending');
                    check(global.stage.get_key_focus() === this._header, 'Status rebuild must retain focus inside the bubble');
                    this._actionPending = false;
                    current.state = 'pin_required'; this._render();
                    check(this._detail.text === t('Enter PIN') && find(this._body, 'Enter PIN'), 'PIN state must explain the next action in the header and body');
                    current.state = 'completed';
                    current.saved_paths = ['/tmp/LinuxDrop received/photo.png'];
                    this._snapshot.transfers.reverse(); this._render();
                    check(this._selectedTransfer === current.id && this._title.text.includes(current.peer_name), 'Completion and reordered snapshots must preserve the chosen peer');
                    check(find(this._body, 'Open folder') && find(this._body, 'Done') && !find(this._body, 'Cancel'), 'Completed receive exposes the folder and dismissal without stale cancellation');
                    const completedDone = find(this._body, 'Done'); completedDone.grab_key_focus();
                    this._snapshot.transfers.reverse(); this._render();
                    check(this._selectedTransfer === current.id && global.stage.get_key_focus() === find(this._body, 'Done'), 'Other transfer ordering must not move completion focus or selection');
                    delete current.saved_paths;
                    current.state = 'failed';
                    current.error = 'Network helper is unavailable. The sharing service stopped; radio cleanup may still be running. Reconnect the adapter and restart sharing services.';
                    this._render();
                    check(this._body.get_children().some(child => child instanceof St.Label && child.text === transferError(current.error)), 'Helper failure must show its translated recovery explanation');
                    const helperError = current.error;
                    current.error = 'Settings'; this._render();
                    check(this._body.get_children().some(child => child instanceof St.Label && child.text === 'Settings'), 'Unknown transfer error text must remain literal');
                    current.error = helperError; this._render();
                    check(!find(this._body, 'Cancel') && find(this._body, 'Details') && find(this._body, 'Done'), 'Failed transfer must replace stale cancellation with recovery navigation');
                    const launchApp = this._launchApp;
                    this._launchApp = () => { throw new Error('Launch failure for smoke test'); };
                    try {
                        this.openApp('--transfers');
                        check(this._expanded && this._notch.visible && this._selectedTransfer === current.id, 'Launch failure must restore the bubble and selected transfer');
                        check(this._body.get_children().some(child => child instanceof St.Label && child.text === 'Launch failure for smoke test'), 'Launch failure must remain visible with retry navigation');
                        check(this._indicator.toggle.subtitle === t('Needs attention'), 'Quick Settings must expose action failures');
                        check(find(this._body, 'Details').reactive, 'Failed launch must allow retry');
                        this._launchApp = () => {};
                        this.openApp('--transfers');
                        this._setExpanded(true);
                        check(!this._actionError && this._indicator.toggle.subtitle !== t('Needs attention'), 'Successful launch retry must retire its old error when the bubble reopens');
                        check(!this._body.get_children().some(child => child instanceof St.Label && child.text === 'Launch failure for smoke test'), 'Successful launch retry must remove stale failure text');
                    } finally { this._launchApp = launchApp; }
                    const fixtureSettings = this._snapshot.settings;
                    this._actionPending = false; this._serviceState = 'offline'; this._snapshot = null; this._render();
                    check(!find(this._body, 'Cancel') && find(this._body, 'Open LinuxDrop'), 'Offline state must replace stale transfer actions');
                    const realProxy = this._proxy;
                    const requests = [];
                    let owner = ':smoke.old';
                    const snapshot = revision => ({epoch: owner, revision, restarting: false, download_link_active: false, hardware: {observed_unix: 0, radios: [], interfaces: [], bluetooth: [], warnings: []}, settings: fixtureSettings, known_peers: [], peers: [], backends: [], transfers: [{id: 'reconnect', peer_id: 'fixture-peer', protocol: 'quickshare', state: 'verification', direction: 'incoming', peer_name: 'Reconnect peer', verification_code: '123456', saved_paths: [], total_bytes: 10, transferred_bytes: 0, files: [{name: 'review.txt', size: 10, transferred: 0}]}]});
                    const reply = (request, value) => request.callback(this._proxy, {deep_unpack: () => [JSON.stringify(value)]});
                    this._proxy = {
                        get_name_owner: () => owner,
                        call: (method, _parameters, _flags, _timeout, _cancellable, callback) => requests.push({method, callback}),
                        call_finish: result => { if (result instanceof Error) throw result; return result; },
                    };
                    try {
                        this._ownerChanged();
                        check(this._serviceState === 'connecting' && !find(this._body, 'Codes match'), 'Connecting must not expose stale consent');
                        const firstRead = requests.shift();
                        this._serviceState = 'ready';
                        this.call('SetVisibility', new GLib.Variant('(u)', [1]));
                        this.call('NonexistentManagerMethod');
                        check(requests.length === 0 && !this._actionPending, 'Invalid wire types and unknown methods must not reach the service');
                        this._serviceState = 'connecting'; this._actionError = null;

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
                        this._refresh();
                        reply(requests.shift(), {...snapshot(2), restarting: 'false'});
                        check(!this._snapshot && this._serviceState === 'offline' && !this._indicator.toggle.reactive && !find(this._body, 'Codes match'), 'Malformed status must retire consent and remote actions');
                        this._refresh();
                        reply(requests.shift(), snapshot(2));
                        check(this._serviceState === 'ready' && this._indicator.toggle.reactive && find(this._body, 'Codes match'), 'Valid status after malformed data must recover without stale pending state');
                        find(this._body, 'Codes match').grab_key_focus();
                        this._snapshot.transfers.push(extra); this._render();
                        check(global.stage.get_key_focus() === find(this._body, 'Codes match'), 'Unrelated arrivals must preserve consent focus for the same request');
                        this._snapshot.transfers[0].id = 'replacement-request'; this._selectedTransfer = null; this._render();
                        check(global.stage.get_key_focus() === this._header, 'A different request must never inherit consent focus');
                        this._snapshot.transfers[0].id = 'reconnect'; this._selectedTransfer = null; this._render();
                        this._refresh();
                        const preActionRead = requests.shift();
                        this.call('AcceptTransfer', new GLib.Variant('(s)', ['reconnect']));
                        const newAction = requests.shift();
                        oldAction.callback(this._proxy, {deep_unpack: () => []});
                        check(this._actionPending && requests.length === 0, 'Old action completion must not unlock a new owner action');
                        newAction.callback(this._proxy, {deep_unpack: () => []});
                        check(this._actionPending && requests.length === 0 && !find(this._body, 'Codes match').reactive, 'Successful receipt must keep consent disabled while an older read is in flight');
                        this.call('AcceptTransfer', new GLib.Variant('(s)', ['reconnect']));
                        check(requests.length === 0, 'Repeated consent must not be sent before the post-action snapshot');
                        reply(preActionRead, snapshot(1));
                        check(this._actionPending && requests.length === 1, 'A pre-action snapshot must not unlock consent and must start a fresh read');
                        reply(requests.shift(), snapshot(1));
                        check(!this._actionPending && find(this._body, 'Codes match').reactive && this._indicator.toggle.reactive, 'A post-action snapshot must settle controls even when revision is unchanged');
                        this._refresh();
                        reply(requests.shift(), {...snapshot(2), transfers: []});
                        check(!find(this._body, 'Codes match') && find(this._body, 'Drop files'), 'Fresh empty state must retire the completed request');
                        this.call('SetVisibility', new GLib.Variant('(s)', ['everyone']));
                        requests.shift().callback(this._proxy, new Error('Visibility change failed'));
                        check(!this._actionPending && this._indicator.toggle.reactive && !this._indicator.toggle.checked && this._indicator.toggle.subtitle === t('Needs attention'), 'Failed visibility change must restore retry and show attention without claiming the requested mode');
                        this.call('SetVisibility', new GLib.Variant('(s)', ['everyone']));
                        requests.shift().callback(this._proxy, {deep_unpack: () => []});
                        check(this._actionPending && requests.length === 1, 'Successful visibility change must wait for its own snapshot');
                        requests.shift().callback(this._proxy, new Error('Snapshot unavailable'));
                        check(!this._actionPending && !this._actionAwaitingSnapshot && this._serviceState === 'offline' && !this._indicator.toggle.reactive, 'Failed post-action snapshot must leave the UI recoverable and remote actions unavailable');
                        this._refresh();
                        reply(requests.shift(), {...snapshot(3), transfers: []});
                        check(this._serviceState === 'ready' && this._indicator.toggle.reactive, 'Recovery must restore actions without a stuck pending flag');
                        this.call('SetVisibility', new GLib.Variant('(s)', ['everyone']));
                        const pendingVisibility = requests.shift();
                        this._refresh();
                        requests.shift().callback(this._proxy, new Error('Concurrent snapshot unavailable'));
                        this._refresh();
                        reply(requests.shift(), {...snapshot(4), transfers: []});
                        check(this._actionPending && !this._indicator.toggle.reactive, 'An unrelated read failure and recovery must not unlock a mutation still in flight');
                        pendingVisibility.callback(this._proxy, {deep_unpack: () => []});
                        reply(requests.shift(), {...snapshot(5), transfers: []});
                        check(!this._actionPending && this._indicator.toggle.reactive, 'An in-flight mutation must settle after recovery and its post-action read');
                    } finally { this._proxy = realProxy; }
                    this._setExpanded(false);
                    check(!this._notch.visible && global.stage.get_key_focus() === this._panelButton, 'Close must hide and restore panel focus');
                    const collapseSeconds = this._settings.get_int('auto-collapse');
                    this._setExpanded(true);
                    this._panelButton.grab_key_focus();
                    this._settings.set_int('auto-collapse', 0);
                    check(!this._collapse, 'Disabling inactivity dismissal must cancel an already running timer');
                    this._settings.set_int('auto-collapse', 1);
                    check(this._collapse, 'Enabling inactivity dismissal must start a timer for an unfocused bubble');
                    this._header.grab_key_focus();
                    check(!this._collapse, 'Keyboard interaction must suspend inactivity dismissal');
                    for (let tick = 0; tick < 10; tick++) await settle();
                    check(this._expanded, 'A focused bubble must stay open beyond the inactivity interval');
                    this._panelButton.grab_key_focus();
                    check(this._collapse, 'Leaving keyboard interaction must rearm inactivity dismissal');
                    for (let tick = 0; tick < 12 && this._expanded; tick++) await settle();
                    check(!this._expanded && !this._notch.visible, 'Unfocused bubble must close after the configured inactivity interval');
                    this._settings.set_int('auto-collapse', collapseSeconds);
                    this._setExpanded(true);
                    Main.layoutManager.emit('monitors-changed');
                    check(!this._expanded && this._interactionMonitor === null, 'Display layout changes must retire the pinned monitor interaction');
                    this._setExpanded(true);
                    check(this._notch.visible && this._monitor(), 'Panel interaction must recover on the available monitor');
                    this._setExpanded(false);
                    const shortcut = '<Control><Alt><Super>F12';
                    const occupied = global.display.grab_accelerator(shortcut, Meta.KeyBindingFlags.NONE);
                    check(occupied, 'Fixture shortcut must initially be free');
                    try {
                        this._settings.set_string('toggle-shortcut', shortcut);
                        check(!this._shortcutAction && this._settings.get_string('shortcut-status') === 'unavailable', 'An occupied shortcut must not replace another action');
                    } finally { global.display.ungrab_accelerator(occupied); }
                    this._bindShortcut();
                    check(this._shortcutAction && this._settings.get_string('shortcut-status') === 'active', 'An available shortcut must register after conflict removal');
                    const toggleAction = this._shortcutAction;
                    global.display.emit('accelerator-activated', toggleAction, null, global.get_current_time());
                    check(this._expanded && this._notch.visible, 'Registered shortcut must open the bubble');
                    global.display.emit('accelerator-activated', toggleAction, null, global.get_current_time());
                    check(!this._expanded, 'Registered shortcut must close the bubble');
                    this._settings.set_string('toggle-shortcut', 'd');
                    check(!this._shortcutAction, 'Unmodified typing keys must never be grabbed');
                    this._settings.set_string('toggle-shortcut', '');
                    const released = global.display.grab_accelerator(shortcut, Meta.KeyBindingFlags.NONE);
                    check(released, 'Removing the preference must release the shortcut');
                    global.display.ungrab_accelerator(released);
                    if (GLib.getenv('LINUXDROP_SMOKE_REAL_DROP') === '1') {
                        for (let attempt = 0; attempt < 2; attempt++) {
                            this._setExpanded(true);
                            this.openApp('--notch-drop');
                            for (let tick = 0; tick < 24 && !this._dropWindow; tick++) await settle();
                            check(this._dropWindow, 'Installed GTK drop surface must attach to the requesting bubble');
                            await settle();
                            const rect = this._dropWindow.get_frame_rect();
                            const monitor = this._monitor();
                            check(Math.abs(rect.x + rect.width / 2 - (monitor.x + monitor.width / 2)) <= 3,
                                `Drop surface must remain centered at its actual size: ${rect.x},${rect.y} ${rect.width}x${rect.height}`);
                            check(rect.y >= monitor.y + Main.panel.height && rect.y + rect.height <= monitor.y + monitor.height,
                                'Drop surface must fit below the panel on the selected monitor');
                            check(!this._notch.visible && !this._dropRequested && !this._dropLaunchPending,
                                'Attached GTK surface must replace the bubble without a pending launch');
                            const offset = this._settings.get_int('top-offset');
                            this._settings.set_int('top-offset', offset + 12);
                            await settle();
                            const moved = this._dropWindow.get_frame_rect();
                            const scale = St.ThemeContext.get_for_stage(global.stage).scale_factor;
                            check(Math.abs(moved.y - rect.y - 12 * scale) <= 1,
                                'An open GTK drop surface must follow the configured top offset');
                            this._settings.set_int('top-offset', offset);
                            await settle();
                            await capture(GLib.getenv('LINUXDROP_SMOKE_CAPTURE_DROP'));
                            this._setExpanded(false);
                            for (let tick = 0; tick < 12 && this._dropWindow; tick++) await settle();
                            check(!this._dropWindow && !this._expanded, 'Closing the bubble must dismiss the installed drop surface');
                        }
                        console.log('LINUXDROP_REAL_DROP_PASSED: installed Wayland surface launch, measured centering, close and reopen');
                    }
                    console.log('LINUXDROP_SMOKE_PASSED: hidden/open, keyboard scrolling, verification, chooser focus, stable action focus, consent focus isolation, literal external names, persistent send/settings, PIN, progress, busy, helper failure, launch recovery, offline, owner replacement, stale consent callbacks, coalesced refresh, post-action snapshot gating, Quick Settings feedback, focus, inactivity preference and keyboard dismissal');
                });
            });
        });
    } catch (error) { console.error(`LINUXDROP_SMOKE_FAILED: ${error.message}\n${error.stack}`); }
    return GLib.SOURCE_REMOVE;
});
