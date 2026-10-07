// Status data is validated before it can enable Shell actions. The generated
// structure is shared with the Rust daemon/client; this is not a general JSON
// Schema implementation. New assertion keywords must be implemented explicitly.
import {snapshotSchema} from './snapshot-schema.js';

const own = (object, key) => Object.prototype.hasOwnProperty.call(object, key);
const object = value => value !== null && typeof value === 'object' && !Array.isArray(value);
const keywords = new Set(['$schema', '$defs', '$ref', 'title', 'type', 'properties', 'required', 'items', 'anyOf', 'enum', 'const', 'minimum', 'maximum', 'minLength', 'format']);
const types = new Set(['object', 'array', 'string', 'integer', 'boolean', 'null']);
const full = (pattern, value) => pattern.exec(value)?.[0] === value;
const invalid = () => { throw new Error('Invalid sharing service status'); };

export function snapshotValidator(schema) {
    const resolve = ref => {
        if (typeof ref !== 'string' || !/^#\/\$defs\/[A-Za-z0-9_]+$/.test(ref)) throw new Error('Unsupported status schema reference');
        const name = ref.slice('#/$defs/'.length);
        if (!own(schema.$defs, name)) throw new Error('Missing status schema definition');
        return schema.$defs[name];
    };
    const inspect = node => {
        if (!object(node)) throw new Error('Unsupported status schema node');
        for (const key of Object.keys(node)) {
            if (!keywords.has(key)) throw new Error(`Unsupported status schema keyword: ${key}`);
        }
        if (node.$ref) resolve(node.$ref);
        if (node.format && node.format !== 'uint64') throw new Error('Unsupported status format');
        if (node.type && ![node.type].flat().every(type => types.has(type))) throw new Error('Unsupported status type');
        for (const child of Object.values(node.$defs ?? {})) inspect(child);
        for (const child of Object.values(node.properties ?? {})) inspect(child);
        for (const child of node.anyOf ?? []) inspect(child);
        if (node.items) inspect(node.items);
    };
    inspect(schema);
    return text => {
        // Bound Shell work before parsing; additive fields remain opaque.
        if (typeof text !== 'string' || text.length > 32 * 1024 * 1024) invalid();
        const value = JSON.parse(text);
        let budget = 1000000;
        const matches = (value, node, depth = 0) => {
            if (--budget < 0 || depth > 64) invalid();
            if (node.$ref && !matches(value, resolve(node.$ref), depth + 1)) return false;
            if (node.anyOf && !node.anyOf.some(child => matches(value, child, depth + 1))) return false;
            if (node.type && ![node.type].flat().some(type => {
                if (type === 'null') return value === null;
                if (type === 'object') return object(value);
                if (type === 'array') return Array.isArray(value);
                // JSON.parse rounds larger integers. Never use that rounded value
                // as a revision or transfer counter in the desktop controls.
                if (type === 'integer') return Number.isSafeInteger(value);
                return typeof value === type;
            })) return false;
            if (own(node, 'const') && value !== node.const) return false;
            if (node.enum && !node.enum.includes(value)) return false;
            if (typeof value === 'number' && ((own(node, 'minimum') && value < node.minimum) || (own(node, 'maximum') && value > node.maximum))) return false;
            if (typeof value === 'string') {
                if (/[\uD800-\uDFFF]/u.test(value)) return false; // Rust strings cannot contain unpaired surrogates.
                if (node.minLength && [...value].length < node.minLength) return false;
            }
            if (Array.isArray(value) && node.items && !value.every(child => matches(child, node.items, depth + 1))) return false;
            if (object(value)) {
                if (node.required && !node.required.every(key => own(value, key))) return false;
                for (const [key, child] of Object.entries(node.properties ?? {})) {
                    if (own(value, key) && !matches(value[key], child, depth + 1)) return false;
                }
            }
            return true;
        };
        if (!matches(value, schema)) invalid();
        validateSemantics(value);
        return value;
    };
}

function validateSemantics(snapshot) {
    const settings = snapshot.settings;
    const bytes = value => new TextEncoder().encode(value).length;
    const name = settings.general.device_name;
    if (!name.trim() || bytes(name) > 80 || /[\u0000-\u001f\u007f-\u009f]/u.test(name)) invalid();
    if (!settings.receive.directory.startsWith('/') || bytes(settings.receive.directory) > 4096) invalid();
    if (settings.localsend.port === settings.quickshare.port) invalid();
    const pin = settings.localsend.pin;
    if ((pin !== '' && !full(/^[0-9]{4,12}$/, pin)) || (settings.localsend.require_pin && !pin)) invalid();
    const adapter = settings.bluetooth.adapter;
    if (adapter !== '' && !full(/^hci[0-9]{1,4}$/, adapter)) invalid();
    const interfaces = settings.network.allowed_interfaces;
    if (interfaces.length > 32 || !interfaces.every(name => full(/^[A-Za-z0-9_.:-]{1,15}$/, name))) invalid();
    for (const records of [snapshot.peers, snapshot.transfers]) {
        const ids = new Set();
        for (const record of records) {
            if (ids.has(record.id)) invalid();
            ids.add(record.id);
        }
    }
    for (const transfer of snapshot.transfers) {
        if (transfer.transferred_bytes > transfer.total_bytes || transfer.files.some(file => file.transferred > file.size)) invalid();
    }
}

export const parseSnapshot = snapshotValidator(snapshotSchema);
