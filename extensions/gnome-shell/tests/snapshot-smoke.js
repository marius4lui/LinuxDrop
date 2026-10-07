// Pure GJS contract regression; no display, session bus or service required.
import Gio from 'gi://Gio';
import {parseSnapshot, snapshotValidator} from '../snapshot.js';
import {snapshotSchema} from '../snapshot-schema.js';

const settingsPath = ARGV[0] ?? 'crates/linuxdrop-ipc/settings.defaults.json';
const [, data] = Gio.File.new_for_path(settingsPath).load_contents(null);
const defaults = JSON.parse(new TextDecoder().decode(data));
const fresh = () => ({epoch: 'test', revision: 1, restarting: false, download_link_active: false,
    settings: JSON.parse(JSON.stringify(defaults)), peers: [], known_peers: [], hardware: {observed_unix: 0, radios: [], interfaces: [], bluetooth: [], warnings: []}, backends: [],
    transfers: [{id: 'transfer', peer_id: 'peer', peer_name: 'Phone', protocol: 'quickshare', direction: 'incoming', state: 'verification',
        files: [{name: 'file.txt', size: 10, transferred: 0}], total_bytes: 10, transferred_bytes: 0, saved_paths: [], verification_code: '1234'}]});
let assertions = 0;
const check = (condition, message) => { assertions++; if (!condition) throw new Error(message); };
const rejected = (mutate, label) => {
    const value = fresh(); mutate(value);
    let failed = false;
    try { parseSnapshot(JSON.stringify(value)); } catch (_) { failed = true; }
    check(failed, `Accepted invalid status: ${label}`);
};
check(parseSnapshot(JSON.stringify(fresh())).transfers[0].id === 'transfer', 'Valid status');
for (const [section, fields] of Object.entries(defaults)) {
    if (typeof fields !== 'object') continue;
    for (const [key, value] of Object.entries(fields)) {
        rejected(s => { s.settings[section][key] = typeof value === 'string' ? false : 'false'; }, `${section}.${key} type`);
        rejected(s => { delete s.settings[section][key]; }, `${section}.${key} missing`);
    }
}
for (const key of Object.keys(fresh())) rejected(s => { delete s[key]; }, `missing ${key}`);
for (const value of [-1, 1.5, Number.MAX_SAFE_INTEGER + 1]) rejected(s => { s.revision = value; }, 'revision');
rejected(s => { s.restarting = 'false'; }, 'boolean string');
rejected(s => { s.transfers.push({...s.transfers[0]}); }, 'duplicate transfer');
rejected(s => { s.transfers[0].transferred_bytes = 11; }, 'total progress');
rejected(s => { s.transfers[0].files[0].transferred = 11; }, 'file progress');
rejected(s => { s.transfers[0].selection_mode = 'accept_all'; }, 'selection mode');
rejected(s => { s.settings.general.device_name = 'x'.repeat(81); }, 'name bytes');
rejected(s => { s.settings.general.device_name = String.fromCharCode(0x85); }, 'name control');
rejected(s => { s.settings.general.device_name = String.fromCharCode(0xd800); }, 'unpaired surrogate');
rejected(s => { s.settings.receive.directory = 'relative'; }, 'directory');
rejected(s => { s.settings.receive.max_bytes = 10995116277761; }, 'byte limit');
rejected(s => { s.settings.localsend.port = s.settings.quickshare.port; }, 'port collision');
rejected(s => { s.settings.localsend.require_pin = true; }, 'missing PIN');
rejected(s => { s.settings.localsend.pin = '1234\n'; }, 'PIN terminator');
rejected(s => { s.settings.bluetooth.adapter = 'hci1\n'; }, 'controller terminator');
rejected(s => { s.settings.network.allowed_interfaces = ['wlan0\n']; }, 'interface terminator');
rejected(s => { s.settings.network.allowed_interfaces = Array(33).fill('wlan0'); }, 'interface count');
rejected(s => { s.settings.hardware.protect_active_connection = false; }, 'active connection protection');
rejected(s => { s.settings.visibility.mode = 'contacts'; }, 'unsupported visibility');
const future = fresh(); future.future = {trusted: true}; future.settings.future = {auto_accept: true};
future.transfers[0].selection_mode = null;
check(parseSnapshot(JSON.stringify(future)).future.trusted === true, 'Opaque additive data preserved');
const hardwarePath = Gio.File.new_for_path(settingsPath).get_parent().get_child('tests/hardware.fixture.json');
const [, hardwareBytes] = hardwarePath.load_contents(null);
const hardwareFixture = JSON.parse(new TextDecoder().decode(hardwareBytes));
const withHardware = fresh(); withHardware.hardware = hardwareFixture;
check(parseSnapshot(JSON.stringify(withHardware)).hardware.radios[0].driver_details.firmware === 'test-firmware', 'Hardware evidence survives validation');
const badHardware = (mutate, label) => rejected(s => {
    s.hardware = JSON.parse(JSON.stringify(hardwareFixture)); mutate(s.hardware);
}, label);
for (const key of ['protected', 'rfkill', 'monitor', 'awdl', 'reserved_for', 'phy']) {
    badHardware(h => { delete h.radios[0][key]; }, `missing radio ${key}`);
}
badHardware(h => { h.radios[0].protected = 'false'; }, 'protected type');
badHardware(h => { h.radios[0].awdl.value = 'supported'; }, 'unknown capability enum');
badHardware(h => { h.radios[0].reserved_for = 'anything'; }, 'unknown reservation');
badHardware(h => { h.bluetooth[0].supported_advertisements = 256; }, 'u8 bound');
badHardware(h => { h.radios.push({...h.radios[0]}); }, 'duplicate radio');
badHardware(h => { h.interfaces = [{name: 'wlan-test', ifindex: 3, phy: 'phy-test', state: 'up', default_route: true, nm_state: 100, nm_managed: true, active_connection: 'active'}]; }, 'active radio must stay protected');
const schema = JSON.parse(JSON.stringify(snapshotSchema)); schema.pattern = '.*';
let unsupported = false;
try { snapshotValidator(schema); } catch (_) { unsupported = true; }
check(unsupported, 'New assertion keywords must not be silently ignored');
print(`LINUXDROP_SNAPSHOT_PASSED: ${assertions} assertions`);

if (ARGV[1]) {
    const [, data] = Gio.File.new_for_path(ARGV[1]).load_contents(null);
    const responses = JSON.parse(new TextDecoder().decode(data));
    check(responses.length > 0, 'Live status responses required');
    for (const response of responses) parseSnapshot(response);
    print(`LINUXDROP_LIVE_STATUS_PASSED: ${responses.length} actual daemon responses`);
}
