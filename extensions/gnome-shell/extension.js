import Clutter from 'gi://Clutter';
import Atk from 'gi://Atk';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Pango from 'gi://Pango';
import St from 'gi://St';
import Shell from 'gi://Shell';
import {t, nearby, filesStatus, transferError} from './locale.js';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as QuickSettings from 'resource:///org/gnome/shell/ui/quickSettings.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';

import {BUS, PATH, INTERFACE as IFACE, MANAGER_XML, METHODS} from './contract.js';
const INTERFACE_INFO = Gio.DBusNodeInfo.new_for_xml(MANAGER_XML).lookup_interface(IFACE);
const TERMINAL = new Set(['completed', 'failed', 'cancelled', 'rejected']);

const SharingToggle = GObject.registerClass(class SharingToggle extends QuickSettings.QuickMenuToggle {
    constructor(extension) {
        super({title: 'LinuxDrop', subtitle: t('Connecting…'), iconName: 'document-send-symbolic', toggleMode: true});
        this.menu.setHeader('document-send-symbolic', 'LinuxDrop', t('Nearby sharing'));
        this.connect('clicked', () => extension.call('SetVisibility', new GLib.Variant('(s)', [this.checked ? 'everyone' : 'hidden'])));
        this.devices = new PopupMenu.PopupMenuSection();
        this.menu.addMenuItem(this.devices);
        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
        this.menu.addAction(t('Send files…'), () => extension.openApp());
        this.menu.addAction(t('LinuxDrop settings'), () => extension.openApp('--settings'));
        this.menu.addAction(t('Notch preferences'), () => extension.openPreferences());
    }
});

const SharingIndicator = GObject.registerClass(class SharingIndicator extends QuickSettings.SystemIndicator {
    constructor(extension) {
        super();
        this.toggle = new SharingToggle(extension);
        this.quickSettingsItems.push(this.toggle);
    }
    destroy() {
        this.quickSettingsItems.forEach(item => item.destroy());
        super.destroy();
    }
});

function textLabel(text, style = 'linuxdrop-notch-subtitle', singleLine = false, translate = true) {
    const value = translate ? t(String(text ?? '')) : String(text ?? '');
    const label = new St.Label({text: !singleLine && value.length > 240 ? `${value.slice(0, 237)}…` : value, accessible_name: value, style_class: style, x_expand: true});
    label.clutter_text.line_wrap = !singleLine;
    label.clutter_text.line_wrap_mode = Pango.WrapMode.WORD_CHAR;
    label.clutter_text.ellipsize = singleLine ? Pango.EllipsizeMode.MIDDLE : Pango.EllipsizeMode.NONE;
    if (style === 'linuxdrop-notch-code') label.clutter_text.line_alignment = Pango.Alignment.CENTER;
    return label;
}

export default class LinuxDropExtension extends Extension {
    enable() {
        this._alive = true;
        this._expanded = false;
        this._openAfterOverview = false;
        this._overviewOpenIdle = 0;
        this._dropRequested = false;
        this._dropLaunchPending = false;
        this._activeCount = 0;
        this._selectedTransfer = null;
        this._interactionMonitor = null;
        this._snapshot = null;
        this._serviceState = 'connecting';
        this._actionPending = false;
        this._actionAwaitingSnapshot = false;
        this._actionError = null;
        this._pending = null;
        this._refreshDirty = false;
        this._ownerGeneration = 0;
        this._revision = '';
        this._peerMenuSignature = null;
        this._signals = [];
        this._settings = this.getSettings();
        this._cancellable = new Gio.Cancellable();
        this._indicator = new SharingIndicator(this);
        Main.panel.statusArea.quickSettings.addExternalIndicator(this._indicator);
        this._panelButton = new PanelMenu.Button(0.5, t('Open LinuxDrop'), true);
        this._panelIcon = new St.Icon({icon_name: 'document-send-symbolic', style_class: 'system-status-icon'});
        this._panelButton.add_child(this._panelIcon);
        const toggle = () => { this._setExpanded(!(this._expanded || this._openAfterOverview)); return Clutter.EVENT_STOP; };
        this._panelButton.connect('button-press-event', (_, event) => event.get_button() === 1 ? toggle() : Clutter.EVENT_PROPAGATE);
        this._panelButton.connect('touch-event', (_, event) => event.type() === Clutter.EventType.TOUCH_BEGIN ? toggle() : Clutter.EVENT_PROPAGATE);
        this._panelButton.connect('key-press-event', (_, event) => {
            if (event.get_key_symbol() === Clutter.KEY_Down) { this._setExpanded(true); return Clutter.EVENT_STOP; }
            if ([Clutter.KEY_Return, Clutter.KEY_KP_Enter, Clutter.KEY_space].includes(event.get_key_symbol())) return toggle();
            return Clutter.EVENT_PROPAGATE;
        });
        Main.panel.addToStatusArea('linuxdrop', this._panelButton, 0, 'right');
        this._notch = new St.BoxLayout({vertical: true, request_mode: Clutter.RequestMode.HEIGHT_FOR_WIDTH, style_class: 'linuxdrop-notch', reactive: true, can_focus: true, track_hover: true, visible: false});
        this._header = new St.Button({style_class: 'linuxdrop-notch-header', can_focus: true, accessible_name: t('Close LinuxDrop'), x_expand: true});
        const row = new St.BoxLayout({style_class: 'linuxdrop-notch-actions', x_expand: true});
        row.add_child(new St.Icon({icon_name: 'document-send-symbolic', style_class: 'linuxdrop-notch-icon'}));
        this._title = new St.Label({text: 'LinuxDrop', style_class: 'linuxdrop-notch-title', x_expand: true, y_align: Clutter.ActorAlign.CENTER});
        this._title.clutter_text.ellipsize = Pango.EllipsizeMode.END;
        row.add_child(this._title);
        this._detail = new St.Label({text: '', style_class: 'linuxdrop-notch-detail', y_align: Clutter.ActorAlign.CENTER}); row.add_child(this._detail);
        this._detail.clutter_text.ellipsize = Pango.EllipsizeMode.END;
        row.add_child(new St.Icon({icon_name: 'window-close-symbolic', style_class: 'linuxdrop-notch-close', y_align: Clutter.ActorAlign.CENTER}));
        this._header.set_child(row); this._notch.add_child(this._header);
        this._scroll = new St.ScrollView({hscrollbar_policy: St.PolicyType.NEVER, vscrollbar_policy: St.PolicyType.AUTOMATIC, overlay_scrollbars: true});
        this._body = new St.BoxLayout({vertical: true, request_mode: Clutter.RequestMode.HEIGHT_FOR_WIDTH, style_class: 'linuxdrop-notch-expanded', visible: false});
        this._scroll.set_child(this._body); this._notch.add_child(this._scroll);
        this._header.connect('clicked', () => this._setExpanded(!this._expanded));
        this._notch.connect('key-press-event', (_, event) => {
            if (event.get_key_symbol() === Clutter.KEY_Escape) { this._setExpanded(false); return Clutter.EVENT_STOP; }
            if ([Clutter.KEY_Tab, Clutter.KEY_ISO_Left_Tab].includes(event.get_key_symbol())) {
                const backwards = event.get_key_symbol() === Clutter.KEY_ISO_Left_Tab || (event.get_state() & Clutter.ModifierType.SHIFT_MASK);
                const direction = backwards ? St.DirectionType.TAB_BACKWARD : St.DirectionType.TAB_FORWARD;
                if (!this._notch.navigate_focus(global.stage.get_key_focus(), direction, false)) this._notch.navigate_focus(null, direction, false);
                return Clutter.EVENT_STOP;
            }
            return Clutter.EVENT_PROPAGATE;
        });
        this._notch.connect('notify::hover', () => this._scheduleCollapse());
        // Input-region tracking is already the default on older shells; GNOME
        // 50 removed that X11-specific option entirely.
        Main.layoutManager.addChrome(this._notch, {trackFullscreen: true});
        this._connect(global.stage, 'captured-event', (_, event) => this._dismissFromEvent(event));
        this._connect(global.stage, 'notify::key-focus', () => {
            this._ensureFocusVisible();
            this._scheduleCollapse();
        });
        this._body.connect('notify::allocation', () => this._ensureFocusVisible());
        this._connect(Main.layoutManager, 'monitors-changed', () => this._position());
        this._connect(St.ThemeContext.get_for_stage(global.stage), 'notify::scale-factor', () => this._position());
        this._connect(Main.sessionMode, 'updated', () => this._visibility());
        this._connect(Main.overview, 'showing', () => this._visibility());
        this._connect(Main.overview, 'hidden', () => {
            if (this._openAfterOverview && !this._overviewOpenIdle) {
                // Overview releases its modal focus after emitting hidden.
                this._overviewOpenIdle = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
                    this._overviewOpenIdle = 0;
                    if (this._alive && this._openAfterOverview) this._setExpanded(true);
                    return GLib.SOURCE_REMOVE;
                });
            } else this._visibility();
        });
        this._connect(global.display, 'in-fullscreen-changed', () => this._visibility());
        this._connect(global.display, 'window-created', (_, window) => this._watchDropSurface(window));
        const dnd = global.backend.get_dnd();
        this._connect(dnd, 'dnd-position-change', (_, x, y) => this._dragPosition(x, y));
        this._connect(dnd, 'dnd-leave', () => this._cancelDragHover());
        this._connect(this._settings, 'changed', () => { this._position(); this._visibility(); });
        this._connect(this._settings, 'changed::auto-collapse', () => this._scheduleCollapse());
        this._position(); this._render(); this._visibility();
        const session = this._cancellable;
        Gio.DBusProxy.new_for_bus(Gio.BusType.SESSION, Gio.DBusProxyFlags.NONE, INTERFACE_INFO, BUS, PATH, IFACE, session, (source, result) => {
            if (!this._alive || this._cancellable !== session) return;
            try {
                this._proxy = Gio.DBusProxy.new_for_bus_finish(result);
                this._connect(this._proxy, 'g-signal', (_, _sender, signal) => { if (signal === 'Changed') this._refresh(); });
                this._connect(this._proxy, 'notify::g-name-owner', () => this._ownerChanged());
                this._ownerChanged();
            } catch (error) { this._serviceState = 'offline'; this._render(); this._error(error.message); }
        });
        this._poll = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 3, () => { this._refresh(); return GLib.SOURCE_CONTINUE; });
    }

    _connect(object, signal, callback) { const id=object.connect(signal, callback); this._signals.push([object,id]); return id; }

    _disconnectObject(object) {
        const remaining=[];
        for (const [owner,id] of this._signals) {
            if (owner===object) owner.disconnect(id); else remaining.push([owner,id]);
        }
        this._signals=remaining;
    }

    call(method, parameters = null, callback = null) {
        if (!this._proxy || !this._alive || this._actionPending || this._serviceState !== 'ready' || !this._proxy.get_name_owner()) return;
        if (!Object.hasOwn(METHODS, method) || (parameters?.get_type_string() ?? '()') !== METHODS[method][0]) {
            this._error(t('Sharing service contract mismatch')); this._render(); return;
        }
        const session = this._cancellable;
        const generation = this._ownerGeneration;
        this._actionPending = true; this._actionError = null; this._render();
        this._proxy.call(method, parameters, Gio.DBusCallFlags.NONE, 30000, this._cancellable, (proxy, result) => {
            if (!this._alive || this._cancellable !== session || generation !== this._ownerGeneration) return;
            try {
                const value = proxy.call_finish(result);
                if (callback) {
                    this._actionPending = false;
                    callback(value.deep_unpack());
                } else {
                    // A successful receipt is not the new transfer state. Keep
                    // consent/cancel disabled until a read started after it returns.
                    this._actionAwaitingSnapshot = true;
                    this._refresh();
                }
            } catch (error) {
                this._actionPending = false;
                this._actionAwaitingSnapshot = false;
                this._error(error.message);
            }
            this._render();
        });
    }

    _ownerChanged() {
        // A replacement daemon owns a different set of consent requests. Retire
        // both the old view and its in-flight callbacks before showing actions.
        this._ownerGeneration++;
        this._pending = null;
        this._refreshDirty = false;
        this._snapshot = null;
        this._revision = '';
        this._selectedTransfer = null;
        this._actionPending = false;
        this._actionAwaitingSnapshot = false;
        this._actionError = null;
        this._serviceState = this._proxy.get_name_owner() ? 'connecting' : 'offline';
        this._render();
        this._refresh();
    }

    _refresh() {
        if (!this._proxy || !this._alive || !this._proxy.get_name_owner()) return;
        if (this._pending) { this._refreshDirty = true; return; }
        const request = {afterAction: this._actionAwaitingSnapshot};
        const session = this._cancellable;
        this._pending = request;
        this._refreshDirty = false;
        this._proxy.call('GetSnapshot', null, Gio.DBusCallFlags.NONE, 5000, this._cancellable, (proxy, result) => {
            if (!this._alive || this._cancellable !== session || this._pending !== request) return;
            this._pending = null;
            try {
                const [json] = proxy.call_finish(result).deep_unpack();
                const snapshot = JSON.parse(json);
                const revision = `${snapshot.epoch}:${snapshot.revision}`;
                const actionSettled = request.afterAction;
                if (actionSettled) {
                    this._actionPending = false;
                    this._actionAwaitingSnapshot = false;
                }
                this._serviceState = 'ready';
                if (revision !== this._revision || actionSettled) { this._snapshot = snapshot; this._revision = revision; this._render(); }
            } catch (_error) {
                if (this._actionAwaitingSnapshot) {
                    this._actionPending = false;
                    this._actionAwaitingSnapshot = false;
                }
                this._snapshot = null; this._revision = ''; this._serviceState = 'offline'; this._actionError = null; this._render();
            }
            // Changed can arrive while GetSnapshot is in flight. Read again so
            // completed transfers and new requests need not wait for polling.
            if (this._refreshDirty) this._refresh();
        });
    }

    _error(message) {
        if (!this._alive) return;
        this._actionError = message;
        this._render();
    }

    _render() {
        const snapshot = this._snapshot ?? {peers: [], transfers: [], backends: []};
        const transfers = snapshot.transfers ?? [];
        const active = transfers.filter(t => !TERMINAL.has(t.state));
        const pending = active.find(t => t.state === 'verification' || (t.state === 'waiting' && ['incoming', 'receive'].includes(t.direction)));
        const selected = transfers.find(transfer => transfer.id === this._selectedTransfer);
        const current = selected && (!TERMINAL.has(selected.state) || this._expanded) ? selected : pending ?? active[0];
        if (current) this._selectedTransfer = current.id;
        this._activeCount = active.length;
        this._panelIcon.icon_name = pending ? 'mail-unread-symbolic' : 'document-send-symbolic';
        this._panelButton.accessible_name = `LinuxDrop · ${pending ? t('Request') : active.length ? t('Transfer in progress') : t('Open LinuxDrop')}`;
        if (active.length) this._panelIcon.add_style_class_name('linuxdrop-panel-active');
        else this._panelIcon.remove_style_class_name('linuxdrop-panel-active');
        this._title.text = current ? `${['incoming', 'receive'].includes(current.direction) ? '↓' : '↑'} ${current.peer_name}` : 'LinuxDrop';
        this._detail.text = current ? (TERMINAL.has(current.state) ? t(current.state) : current.state === 'transferring' && current.total_bytes > 0 ? `${Math.floor(100 * current.transferred_bytes / current.total_bytes)}%` : t(current.state === 'verification' ? 'Compare code' : current.state === 'pin_required' ? 'Enter PIN' : current.state === 'waiting' && ['incoming', 'receive'].includes(current.direction) ? 'Request' : 'Waiting')) : nearby((snapshot.peers ?? []).length);
        const visible = snapshot.settings?.visibility?.mode === 'everyone';
        this._indicator.toggle.checked = visible;
        this._indicator.toggle.reactive = this._serviceState === 'ready' && !this._actionPending;
        this._indicator.toggle.can_focus = this._indicator.toggle.reactive;
        this._indicator.toggle.subtitle = t(visible ? 'Visible to everyone' : 'Hidden');
        const menuPeers = (snapshot.peers ?? []).slice(0, 6);
        const peerMenuSignature = JSON.stringify(menuPeers.map(peer => [peer.id, peer.name]));
        if (peerMenuSignature !== this._peerMenuSignature) {
            this._peerMenuSignature = peerMenuSignature;
            this._indicator.toggle.devices.removeAll();
            for (const peer of menuPeers) this._indicator.toggle.devices.addAction(peer.name, () => this.openApp());
            if (!menuPeers.length) this._indicator.toggle.devices.addMenuItem(new PopupMenu.PopupMenuItem(t('No devices nearby'), {reactive: false}));
        }
        if (this._serviceState !== 'ready') {
            const status = t(this._serviceState === 'connecting' ? 'Connecting…' : 'Service unavailable');
            this._detail.text = status;
            this._indicator.toggle.subtitle = status;
            this._panelIcon.icon_name = 'network-offline-symbolic';
            this._panelButton.accessible_name = `LinuxDrop · ${status}`;
        } else if (this._actionError) {
            this._detail.text = t('Needs attention');
            this._indicator.toggle.subtitle = t('Needs attention');
        } else if (this._actionPending) {
            this._detail.text = t('Working…');
            this._indicator.toggle.subtitle = t('Working…');
        }
        // Rebuild only expanded contents. Compact progress has a stable actor/focus tree.
        if (this._expanded) this._renderBody(current, active);
        this._position();
    }

    _renderBody(current, active) {
        // Preserve keyboard focus by keeping action rows unless the transfer/state changes.
        const signature = JSON.stringify([this._serviceState, this._actionPending, this._actionError, this._dropRequested, current?.id, current?.state, current?.peer_name, current?.files?.map(file => [file.name, file.size]), current?.error, current?.verification_code, current?.saved_paths, this._snapshot?.settings?.receive?.ask_directory, active.map(transfer => transfer.id)]);
        if (this._bodySignature === signature) {
            if (this._progress && current) {
                const fraction = Math.max(0, Math.min(1, (current.transferred_bytes ?? 0) / Math.max(1, current.total_bytes ?? 1)));
                this._progressFraction = fraction;
                this._progress.width = this._progress.get_parent().width * fraction;
                this._progress.get_parent().accessible_name = `${current.peer_name}: ${Math.floor(100 * fraction)}%`;
            }
            return;
        }
        this._bodySignature = signature;
        const focus = global.stage.get_key_focus();
        const focusedAction = focus && this._body.contains(focus) ? focus._linuxdropAction : null;
        const context = JSON.stringify([this._serviceState, current?.id, current?.state]);
        const sameContext = this._bodyContext === context;
        this._bodyContext = context;
        if (focus && this._body.contains(focus)) this._header.grab_key_focus();
        this._body.destroy_all_children(); this._progress = null;
        this._populateBody(current, active);
        if (this._serviceState === 'ready' && current) {
            const shortcuts = new St.BoxLayout({style_class: 'linuxdrop-notch-actions'});
            this._button(shortcuts, this._dropRequested ? 'Opening…' : 'Drop files', () => this.openApp('--notch-drop'), false, this._dropRequested);
            this._iconButton(shortcuts, 'Settings', 'emblem-system-symbolic', () => this.openApp('--settings'));
            this._body.add_child(shortcuts);
        }
        // An unrelated transfer joining/leaving must not interrupt a keyboard
        // action. A new request/state must never inherit consent or cancel focus.
        const navigation = ['Previous transfer', 'Next transfer'].includes(focusedAction);
        if (focusedAction && (sameContext || navigation)) {
            const restore = actor => {
                if (actor._linuxdropAction === focusedAction && actor.can_focus && actor.reactive) {
                    actor.grab_key_focus();
                    return true;
                }
                return actor.get_children().some(restore);
            };
            restore(this._body);
        }
    }

    _populateBody(current, active) {
        if (this._actionError) this._body.add_child(textLabel(this._actionError, 'linuxdrop-notch-error', false, false));
        if (this._serviceState !== 'ready') {
            this._body.add_child(textLabel(this._serviceState === 'connecting' ? 'Connecting…' : 'Service unavailable', 'linuxdrop-notch-heading'));
            this._body.add_child(textLabel('Open LinuxDrop to check the sharing service.'));
            const actions = new St.BoxLayout({style_class: 'linuxdrop-notch-actions'});
            this._button(actions, 'Open LinuxDrop', () => this.openApp(), true);
            this._body.add_child(actions);
            return;
        }
        if (active.length > 1 || (active.length && current && TERMINAL.has(current.state))) {
            const chooser = new St.BoxLayout({style_class: 'linuxdrop-notch-actions'});
            const index = active.findIndex(transfer => transfer.id === current?.id);
            const select = offset => {
                const next = index < 0 ? (offset > 0 ? 0 : active.length - 1) : (index + offset + active.length) % active.length;
                this._selectedTransfer = active[next].id;
                this._render();
            };
            this._iconButton(chooser, 'Previous transfer', 'go-previous-symbolic', () => select(-1));
            const count = textLabel(index < 0 ? t('Active transfers') : `${index + 1} / ${active.length}`);
            count.y_align = Clutter.ActorAlign.CENTER;
            chooser.add_child(count);
            this._iconButton(chooser, 'Next transfer', 'go-next-symbolic', () => select(1));
            this._body.add_child(chooser);
        }
        if (current) {
            this._body.add_child(textLabel(current.peer_name, 'linuxdrop-notch-heading', true, false));
            const files = current.files ?? [];
            this._body.add_child(textLabel(files.length === 1 ? files[0].name : filesStatus(files.length, active.length), 'linuxdrop-notch-subtitle', files.length === 1, false));
            if (TERMINAL.has(current.state)) {
                this._body.add_child(textLabel(current.error ? transferError(current.error) : t(current.state), current.state === 'failed' ? 'linuxdrop-notch-error' : 'linuxdrop-notch-subtitle', false, false));
                const actions = new St.BoxLayout({style_class: 'linuxdrop-notch-actions'});
                const saved = current.saved_paths?.[0];
                if (current.state === 'completed' && saved) {
                    this._button(actions, 'Open folder', () => {
                        try {
                            const parent = Gio.File.new_for_path(saved).get_parent();
                            if (parent) Gio.AppInfo.launch_default_for_uri(parent.get_uri(), global.create_app_launch_context(0, -1));
                            this._setExpanded(false);
                        } catch (error) { this._error(error.message); }
                    });
                }
                this._button(actions, 'Details', () => this.openApp('--transfers'));
                this._body.add_child(actions);
                const done = new St.BoxLayout({style_class: 'linuxdrop-notch-actions'});
                this._button(done, 'Done', () => this._setExpanded(false), true);
                this._body.add_child(done);
                this._scheduleCollapse();
                return;
            }
            if (current.state === 'verification') {
                this._body.add_child(textLabel('Compare this code on both devices'));
                this._body.add_child(textLabel(current.verification_code ?? '', 'linuxdrop-notch-code', false, false));
            }
            if (current.state === 'transferring') {
                this._progressFraction = Math.max(0, Math.min(1, (current.transferred_bytes ?? 0) / Math.max(1, current.total_bytes ?? 1)));
                const track = new St.Widget({style_class: 'linuxdrop-notch-track', x_expand: true, accessible_role: Atk.Role.PROGRESS_BAR, accessible_name: `${current.peer_name}: ${Math.floor(100 * this._progressFraction)}%`});
                const fill = new St.Widget({style_class: 'linuxdrop-notch-fill', width: 0});
                this._progress = fill;
                track.connect('notify::width', () => { fill.width = track.width * this._progressFraction; });
                track.add_child(this._progress); this._body.add_child(track);
            }
            const actions = new St.BoxLayout({style_class: 'linuxdrop-notch-actions'});
            const id = new GLib.Variant('(s)', [current.id]);
            if (current.state === 'verification' || (current.state === 'waiting' && ['incoming', 'receive'].includes(current.direction))) {
                const primary = new St.BoxLayout({style_class: 'linuxdrop-notch-actions'});
                const needsDestination = ['incoming', 'receive'].includes(current.direction) && this._snapshot?.settings?.receive?.ask_directory;
                if (current.state === 'verification' && !needsDestination) this._button(primary, 'Codes match', () => this.call('AcceptTransfer', id), true, true);
                else this._button(primary, 'Review files', () => this.openApp('--transfers'), true);
                this._body.add_child(primary);
                this._button(actions, 'Decline', () => this.call('RejectTransfer', id), false, true);
            } else this._button(actions, 'Cancel', () => this.call('CancelTransfer', id), false, true);
            this._button(actions, current.state === 'pin_required' ? 'Enter PIN' : 'Details', () => this.openApp('--transfers'));
            this._body.add_child(actions);
        } else {
            this._body.add_child(textLabel('Send files', 'linuxdrop-notch-heading'));
            this._body.add_child(textLabel('Open the drop area, add your files, and choose a nearby device.'));
            const actions = new St.BoxLayout({style_class: 'linuxdrop-notch-actions'});
            this._button(actions, this._dropRequested ? 'Opening…' : 'Drop files', () => this.openApp('--notch-drop'), true, this._dropRequested);
            this._iconButton(actions, 'Settings', 'emblem-system-symbolic', () => this.openApp('--settings'));
            this._body.add_child(actions);
        }
    }

    _button(box, label, callback, primary = false, mutation = false) {
        const disabled = mutation && (this._actionPending || this._dropRequested);
        const button = new St.Button({can_focus: !disabled, reactive: !disabled, accessible_name: t(label), x_expand: true, style_class: `linuxdrop-notch-action${primary ? ' primary' : ''}`});
        button._linuxdropAction = label;
        const caption = textLabel(label, 'linuxdrop-notch-action-label');
        caption.clutter_text.line_alignment = Pango.Alignment.CENTER;
        button.set_child(caption);
        if (disabled) button.add_style_pseudo_class('insensitive');
        button.connect('clicked', callback); box.add_child(button);
    }

    _iconButton(box, label, icon, callback) {
        const button = new St.Button({can_focus: true, accessible_name: t(label), style_class: 'linuxdrop-notch-action linuxdrop-notch-navigation'});
        button._linuxdropAction = label;
        button.set_child(new St.Icon({icon_name: icon, style_class: 'linuxdrop-notch-icon'}));
        button.connect('clicked', callback); box.add_child(button);
        return button;
    }

    _setExpanded(expanded) {
        if (expanded && (!this._settings.get_boolean('show-notch') || Main.sessionMode.isLocked || Main.sessionMode.isGreeter)) {
            this._openAfterOverview = false;
            return;
        }
        if (expanded && Main.overview.visible) {
            // The panel remains available in Overview. Honor that explicit
            // activation once its modal overview has finished relinquishing focus.
            this._openAfterOverview = true;
            Main.overview.hide();
            return;
        }
        this._openAfterOverview = false;
        if (this._overviewOpenIdle) { GLib.source_remove(this._overviewOpenIdle); this._overviewOpenIdle = 0; }
        const focus = global.stage.get_key_focus();
        const restoreFocus = !expanded && focus && this._notch.contains(focus);
        if (!expanded) {
            this._dropRequested = false;
            if (this._dropTimeout) { GLib.source_remove(this._dropTimeout); this._dropTimeout = 0; }
            this._cancelDragHover();
            this._closeDropSurface();
        } else {
            if (!this._expanded) this._interactionMonitor = this._monitor()?.index ?? null;
            Main.panel.statusArea.quickSettings.menu.close();
        }
        if (!expanded) { this._interactionMonitor = null; this._selectedTransfer = null; }
        this._expanded = expanded; this._body.visible = expanded;
        if (expanded) this._panelButton.add_style_pseudo_class('active');
        else this._panelButton.remove_style_pseudo_class('active');
        this._bodySignature = null; this._render(); this._visibility(); this._scheduleCollapse();
        if (expanded && this._settings.get_boolean('animations') && St.Settings.get().enable_animations) {
            this._body.opacity = 0;
            this._body.ease({opacity: 255, duration: 180, mode: Clutter.AnimationMode.EASE_OUT_QUAD});
        } else this._body.opacity = 255;
        if (expanded) this._header.grab_key_focus();
        else if (restoreFocus && this._panelButton.visible) this._panelButton.grab_key_focus();
    }

    _scheduleCollapse() {
        if (this._collapse) { GLib.source_remove(this._collapse); this._collapse = 0; }
        const seconds = this._settings.get_int('auto-collapse');
        const focus = global.stage.get_key_focus();
        if (!this._expanded || this._dropWindow || this._dropRequested || !seconds || this._notch.hover || (focus && this._notch.contains(focus))) return;
        this._collapse = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, seconds, () => {
            this._collapse = 0;
            const focus = global.stage.get_key_focus();
            if (!this._notch.hover && (!focus || !this._notch.contains(focus))) this._setExpanded(false);
            return GLib.SOURCE_REMOVE;
        });
    }

    _ensureFocusVisible() {
        const focus = global.stage.get_key_focus();
        if (!this._expanded || !focus || !this._body.contains(focus) || !focus.has_allocation()) return;
        const [, top] = focus.get_transformed_position();
        const [, height] = focus.get_transformed_size();
        const [, viewportTop] = this._scroll.get_transformed_position();
        const [, viewportHeight] = this._scroll.get_transformed_size();
        const adjustment = this._scroll.vadjustment;
        if (top < viewportTop) adjustment.value += top - viewportTop;
        else if (top + height > viewportTop + viewportHeight) adjustment.value += top + height - viewportTop - viewportHeight;
    }

    _position() {
        if (!this._alive) return;
        if (this._interactionMonitor !== null && !Main.layoutManager.monitors[this._interactionMonitor]) this._setExpanded(false);
        const monitor = this._monitor();
        if (!monitor) return;
        const scale = St.ThemeContext.get_for_stage(global.stage).scale_factor;
        const width = Math.min(350 * scale, monitor.width - 32 * scale);
        const top = Main.panel.height + this._settings.get_int('top-offset') * scale;
        this._detail.visible = width >= 300 * scale;
        this._notch.width = width;
        const scrollStyle = `max-height: ${Math.max(80, (monitor.height - top) / scale - 76)}px;`;
        if (this._scroll.get_style() !== scrollStyle) this._scroll.set_style(scrollStyle);
        this._notch.set_position(Math.round(monitor.x + (monitor.width - width) / 2), monitor.y + top);
        this._positionDropSurface();
    }

    _positionDropSurface() {
        if (!this._alive || !this._dropWindow) return;
        const monitor = this._monitor();
        if (!monitor) return;
        const rect = this._dropWindow.get_frame_rect();
        if (rect.width <= 0 || rect.height <= 0) return;
        const scale = St.ThemeContext.get_for_stage(global.stage).scale_factor;
        const panelBottom = monitor.y + Main.panel.height;
        const lastTop = Math.max(panelBottom, monitor.y + monitor.height - rect.height - 8 * scale);
        const y = Math.round(Math.min(panelBottom + this._settings.get_int('top-offset') * scale, lastTop));
        const x = Math.round(monitor.x + (monitor.width - rect.width) / 2);
        // GTK chooses its natural size (including text scaling). Reposition after
        // that size is committed instead of centering an assumed 350px frame.
        if (rect.x !== x || rect.y !== y) this._dropWindow.move_frame(false, x, y);
    }

    _visibility() {
        if (!this._alive) return;
        const monitor = this._monitor();
        const locked = Main.sessionMode.isLocked || Main.sessionMode.isGreeter;
        const fullscreen = monitor && global.display.get_monitor_in_fullscreen(monitor.index);
        const enabled = this._settings.get_boolean('show-notch');
        if (!enabled || locked) this._openAfterOverview = false;
        const blocked = locked || Main.overview.visible || (this._settings.get_boolean('hide-fullscreen') && fullscreen);
        this._panelButton.visible = enabled && !locked;
        this._notch.visible = this._expanded && !this._dropWindow && enabled && !blocked;
        if ((!enabled || blocked) && this._expanded) this._setExpanded(false);
    }

    _monitor() {
        const mode = this._settings.get_string('monitor-mode');
        const index = this._interactionMonitor ?? (mode === 'pointer' ? global.display.get_current_monitor() : mode === 'fixed' ? this._settings.get_int('monitor') : -1);
        return Main.layoutManager.monitors[index] ?? Main.layoutManager.primaryMonitor;
    }

    _closeDropSurface() {
        if (!this._dropWindow) return;
        const window = this._dropWindow;
        this._dropWindow = null;
        window.unmake_above();
        window.delete(global.get_current_time());
    }

    _dismissFromEvent(event) {
        if (!this._expanded && !this._openAfterOverview) return Clutter.EVENT_PROPAGATE;
        if (event.type() === Clutter.EventType.KEY_PRESS && event.get_key_symbol() === Clutter.KEY_Escape) {
            this._setExpanded(false);
            this._panelButton.grab_key_focus();
            return Clutter.EVENT_STOP;
        }
        if (![Clutter.EventType.BUTTON_PRESS, Clutter.EventType.TOUCH_BEGIN].includes(event.type())) return Clutter.EVENT_PROPAGATE;
        const [x, y] = event.get_coords();
        const inside = actor => {
            const [left, top] = actor.get_transformed_position();
            const [width, height] = actor.get_transformed_size();
            return x >= left && x < left + width && y >= top && y < top + height;
        };
        if (inside(this._panelButton) || (this._notch.visible && inside(this._notch))) return Clutter.EVENT_PROPAGATE;
        if (this._dropWindow) {
            const rect = this._dropWindow.get_frame_rect();
            if (x >= rect.x && x < rect.x + rect.width && y >= rect.y && y < rect.y + rect.height) return Clutter.EVENT_PROPAGATE;
        }
        this._setExpanded(false);
        return Clutter.EVENT_PROPAGATE;
    }

    _cancelDragHover() {
        if (this._dragHover) { GLib.source_remove(this._dragHover); this._dragHover = 0; }
    }

    _dragPosition(x, y) {
        if (!this._alive || !this._expanded || this._dropWindow || this._dropRequested || !this._notch.visible) return;
        const [left, top] = this._notch.get_transformed_position();
        const [width, height] = this._notch.get_transformed_size();
        if (x < left - 12 || x > left + width + 12 || y < top - 8 || y > top + height + 12) { this._cancelDragHover(); return; }
        if (this._dragHover) return;
        this._dragHover = GLib.timeout_add(GLib.PRIORITY_DEFAULT, this._settings.get_int('drag-hover-delay'), () => {
            this._dragHover = 0;
            if (this._expanded && this._notch.visible) this.openApp('--notch-drop');
            return GLib.SOURCE_REMOVE;
        });
    }

    _watchDropSurface(window) {
        const attach = () => {
            if (!this._alive || window.get_title() !== 'LinuxDrop Drop Surface' || window.get_gtk_application_id() !== `${BUS}.App`) return;
            if (this._dropWindow === window) return;
            if (!this._dropLaunchPending) return;
            this._dropLaunchPending = false;
            if (this._dropTimeout) { GLib.source_remove(this._dropTimeout); this._dropTimeout = 0; }
            if (!this._expanded || !this._dropRequested) { window.delete(global.get_current_time()); return; }
            this._dropRequested = false;
            this._dropWindow = window;
            const monitor = this._monitor();
            if (!monitor) return;
            window.make_above();
            this._connect(window, 'size-changed', () => this._positionDropSurface());
            this._positionDropSurface();
            this._visibility();
            this._connect(window, 'unmanaged', () => { this._disconnectObject(window); if (this._dropWindow === window) { this._dropWindow = null; this._setExpanded(false); } });
        };
        attach();
        const actor = window.get_compositor_private();
        if (actor) {
            this._connect(actor, 'first-frame', () => { this._disconnectObject(actor); attach(); });
            this._connect(actor, 'destroy', () => this._disconnectObject(actor));
        }
    }

    openApp(option = '') {
        const selectedTransfer = this._selectedTransfer;
        if (option === '--notch-drop') {
            if (!this._expanded || this._dropRequested || this._dropWindow) return;
            this._dropRequested = true;
            this._dropLaunchPending = true;
            this._dropTimeout = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 15, () => {
                this._dropTimeout = 0;
                if (this._dropRequested) {
                    this._dropRequested = false;
                    // Keep watching so a delayed window is dismissed after timeout.
                    this._error(t('Could not open the drop area. Try again.'));
                    this._scheduleCollapse();
                }
                return GLib.SOURCE_REMOVE;
            });
            this._render();
            if (this._collapse) { GLib.source_remove(this._collapse); this._collapse = 0; }
        } else this._setExpanded(false);
        Main.panel.statusArea.quickSettings.menu.close();
        try {
            this._launchApp(option);
            // A retry accepted by the launcher retires the previous failure.
            // Drop-area startup still reports a new error if its window times out.
            if (this._actionError) { this._actionError = null; this._render(); }
        } catch (error) {
            this._dropRequested = false; this._dropLaunchPending = false;
            if (this._dropTimeout) { GLib.source_remove(this._dropTimeout); this._dropTimeout = 0; }
            this._selectedTransfer = selectedTransfer;
            this._setExpanded(true);
            this._error(error.message); this._scheduleCollapse();
        }
    }

    _launchApp(option) {
        if (!option) {
            const app = Shell.AppSystem.get_default().lookup_app(`${BUS}.desktop`);
            if (app) { app.activate(); return; }
        }
        Gio.Subprocess.new(['linuxdrop', ...(option ? [option] : ['open'])], Gio.SubprocessFlags.NONE);
    }

    disable() {
        this._alive = false;
        this._openAfterOverview = false;
        if (this._overviewOpenIdle) { GLib.source_remove(this._overviewOpenIdle); this._overviewOpenIdle = 0; }
        this._cancellable?.cancel();
        this._cancelDragHover();
        this._closeDropSurface();
        if (this._poll) GLib.source_remove(this._poll);
        if (this._collapse) GLib.source_remove(this._collapse);
        if (this._dropTimeout) { GLib.source_remove(this._dropTimeout); this._dropTimeout = 0; }
        for (const [object, id] of this._signals ?? []) { try { object.disconnect(id); } catch (_) { /* A window can already be disposed. */ } }
        this._signals = [];
        if (this._notch) { Main.layoutManager.removeChrome(this._notch); this._notch.destroy(); }
        this._indicator?.destroy();
        this._panelButton?.destroy();
        this._notch = null; this._indicator = null; this._proxy = null; this._settings = null;
        this._snapshot = null; this._poll = 0; this._collapse = 0;
        this._panelButton = null; this._panelIcon = null; this._dropRequested = false; this._dropLaunchPending = false;
    }
}
