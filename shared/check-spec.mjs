#!/usr/bin/env node
// Checks the GNOME extension's GSettings defaults against shared/defaults.ini
// ([shared] plus [linux]) and that the spec itself is well formed.
// The Windows side is checked by `cargo test -p stats-core`.
// Usage: node shared/check-spec.mjs   (exit 1 on any mismatch)

import { readFileSync, readdirSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const PLATFORMS = ['linux', 'windows'];

/** @returns {Map<string, Map<string, string>>} section -> key -> value */
export function parseSpec(text) {
    const sections = new Map();
    let current = null;
    for (const [i, raw] of text.split(/\r?\n/).entries()) {
        const line = raw.trim();
        if (!line || line.startsWith('#'))
            continue;
        const header = line.match(/^\[(\w+)\]$/);
        if (header) {
            current = header[1];
            if (!sections.has(current))
                sections.set(current, new Map());
            continue;
        }
        const eq = line.indexOf('=');
        if (eq < 0 || !current)
            throw new Error(`defaults.ini line ${i + 1}: expected key = value inside a [section]`);
        sections.get(current).set(line.slice(0, eq).trim(), normalize(line.slice(eq + 1)));
    }
    return sections;
}

/** Lists compare as comma-separated trimmed items, everything else as text. */
function normalize(value) {
    return value.split(',').map(s => s.trim()).filter(Boolean).join(', ');
}

/** GVariant text from a <default> element -> spec text. */
function fromGVariant(text) {
    const t = text.trim();
    if (t.startsWith('[') || t.startsWith('@as'))
        return normalize([...t.matchAll(/'([^']*)'/g)].map(m => m[1]).join(','));
    return t.replace(/^'(.*)'$/, '$1');
}

export function gschemaDefaults(xml) {
    const out = new Map();
    for (const m of xml.matchAll(/<key name="([^"]+)"[^>]*>[\s\S]*?<default>([\s\S]*?)<\/default>/g))
        out.set(m[1], fromGVariant(m[2]));
    return out;
}

function main() {
    const errors = [];
    const spec = parseSpec(readFileSync(join(root, 'shared', 'defaults.ini'), 'utf8'));
    for (const name of spec.keys()) {
        if (name !== 'shared' && !PLATFORMS.includes(name))
            errors.push(`defaults.ini: unknown section [${name}]`);
    }
    const shared = spec.get('shared') ?? new Map();
    for (const platform of PLATFORMS) {
        for (const key of (spec.get(platform) ?? new Map()).keys()) {
            if (shared.has(key))
                errors.push(`defaults.ini: ${key} is in [shared] and [${platform}]`);
        }
    }

    const schemaDir = join(root, 'linux', 'schemas');
    const xml = readdirSync(schemaDir).filter(f => f.endsWith('.gschema.xml'))
        .map(f => readFileSync(join(schemaDir, f), 'utf8')).join('\n');
    const actual = gschemaDefaults(xml);
    const expected = new Map([...shared, ...(spec.get('linux') ?? new Map())]);
    for (const [key, value] of expected) {
        if (!actual.has(key))
            errors.push(`linux: ${key} is in the spec but not in the gschema`);
        else if (actual.get(key) !== value)
            errors.push(`linux: ${key} defaults to '${actual.get(key)}' in the gschema but '${value}' in defaults.ini`);
    }

    if (errors.length) {
        console.error(errors.map(e => `FAIL ${e}`).join('\n'));
        process.exit(1);
    }
    console.log(`ok: ${expected.size} Linux defaults match shared/defaults.ini`);
}

main();
