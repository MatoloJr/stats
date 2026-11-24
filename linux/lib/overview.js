/**
 * What the dashboard shows, worked out from a snapshot: the status chip, the
 * six tiles, and programs grouped by name. Plain JavaScript with no GNOME
 * imports, so the same tests run under gjs and Node.
 */

import {
    formatBytes,
    formatBytesPerSecond,
    formatPercent,
    formatTemperature,
    thresholdLevel,
} from './widgets/formatters.js';

const LEVEL_RANK = {normal: 0, warning: 1, critical: 2};

/**
 * Parts of the desktop that must never be offered a Close button: closing
 * them ends the session or breaks it.
 */
const PROTECTED_NAMES = new Set([
    'gnome-shell', 'gnome-session', 'gnome-session-b', 'gnome-session-c', 'mutter',
    'xwayland', 'xorg', 'x', 'systemd', 'dbus-daemon', 'dbus-broker', 'dbus-broker-lau',
    'pipewire', 'pipewire-pulse', 'wireplumber', 'pulseaudio', 'polkitd', 'gnome-keyring-d',
    'ibus-daemon', 'ibus-x11', 'ibus-portal', 'at-spi-bus-laun', 'at-spi2-registr',
    'xdg-desktop-por', 'xdg-document-po', 'xdg-permission-', 'gvfsd', 'evolution-sourc',
    'evolution-alarm', 'evolution-calen', 'evolution-addre', 'gjs', 'tracker-miner-f',
    'localsearch-3', 'goa-daemon', 'goa-identity-se', 'sshd', 'bash', 'zsh', 'fish', 'sh',
]);
const PROTECTED_PREFIXES = ['gsd-', 'gvfs', 'gnome-session', 'xdg-', 'ibus-', 'at-spi', 'dbus-', 'systemd', 'pipewire'];

/** Friendlier names for programs whose process name is cryptic. */
const DISPLAY_NAMES = new Map([
    ['chrome', 'Chrome'],
    ['chromium', 'Chromium'],
    ['firefox', 'Firefox'],
    ['firefox-bin', 'Firefox'],
    ['code', 'VS Code'],
    ['electron', 'Electron'],
    ['java', 'Java'],
    ['node', 'Node.js'],
    ['python3', 'Python'],
    ['nautilus', 'Files'],
    ['gnome-text-edit', 'Text Editor'],
    ['gnome-calculato', 'Calculator'],
    ['gnome-terminal-', 'Terminal'],
    ['ptyxis', 'Terminal'],
    ['slack', 'Slack'],
    ['discord', 'Discord'],
    ['spotify', 'Spotify'],
    ['thunderbird', 'Thunderbird'],
    ['qemu-system-x86', 'QEMU'],
]);

/**
 * @param {string} comm Process name from /proc (15 characters at most).
 * @returns {boolean}
 */
export function isProtected(comm) {
    const name = String(comm ?? '').toLowerCase();
    if (!name)
        return true;
    if (PROTECTED_NAMES.has(name))
        return true;
    return PROTECTED_PREFIXES.some(prefix => name.startsWith(prefix));
}

/**
 * @param {string} comm
 * @returns {string}
 */
export function displayName(comm) {
    const name = String(comm ?? '').trim();
    const known = DISPLAY_NAMES.get(name.toLowerCase());
    if (known)
        return known;
    if (!name)
        return 'Unknown';
    return name.charAt(0).toUpperCase() + name.slice(1);
}

/**
 * The fields Stats needs from /proc/<pid>/status.
 *
 * @param {string|null} text
 * @returns {{name: string, uid: number|null, pid: number|null, rss: number|null}|null}
 */
export function parseStatus(text) {
    if (!text)
        return null;
    const field = key => {
        const match = new RegExp(`^${key}:\\s*(.*)$`, 'm').exec(text);
        return match ? match[1] : null;
    };
    const name = field('Name');
    if (name == null)
        return null;
    const uid = Number((field('Uid') ?? '').split(/\s+/)[0]);
    const pid = Number(field('Pid'));
    const rss = /^(\d+)\s*kB/.exec(field('VmRSS') ?? '');
    return {
        name: name.trim(),
        uid: Number.isFinite(uid) ? uid : null,
        pid: Number.isFinite(pid) ? pid : null,
        rss: rss ? Number(rss[1]) * 1024 : null,
    };
}

/**
 * Group processes by name, like "Firefox ×12": memory and CPU are summed and
 * every pid is kept, so Close reaches all of them. Biggest memory first.
 *
 * @param {{pid: number, comm: string, rss: number|null, cpu: number|null}[]} processes
 * @returns {{comm: string, name: string, count: number, rss: number, cpu: number|null, pids: number[], protected: boolean}[]}
 */
export function groupProcesses(processes) {
    const groups = new Map();
    for (const process of processes ?? []) {
        if (!process?.comm)
            continue;
        let group = groups.get(process.comm);
        if (!group) {
            group = {
                comm: process.comm,
                name: displayName(process.comm),
                count: 0,
                rss: 0,
                cpu: null,
                pids: [],
                protected: isProtected(process.comm),
            };
            groups.set(process.comm, group);
        }
        group.count += 1;
        group.rss += process.rss ?? 0;
        if (process.cpu != null && Number.isFinite(process.cpu))
            group.cpu = (group.cpu ?? 0) + process.cpu;
        if (process.pid != null)
            group.pids.push(process.pid);
    }
    return [...groups.values()].sort((a, b) => b.rss - a.rss || a.name.localeCompare(b.name));
}

/**
 * Warning levels for each metric. `get` reads an integer setting.
 *
 * @param {object} snapshot
 * @param {(key: string) => number} get
 * @returns {{cpu: string, memory: string, storage: string, gpu: string, temperature: string}}
 */
export function levels(snapshot, get) {
    const gpu = snapshot?.gpu?.selected;
    return {
        memory: thresholdLevel(snapshot?.memory?.percent, get('memory-warning'), get('memory-critical')),
        storage: thresholdLevel(snapshot?.storage?.mounts?.[0]?.percent, get('storage-warning'), get('storage-critical')),
        cpu: thresholdLevel(snapshot?.cpu?.usage, get('cpu-warning'), get('cpu-critical')),
        temperature: thresholdLevel(snapshot?.temperature?.headline?.celsius, get('temp-warning'), get('temp-critical')),
        gpu: gpu?.state === 'off' ? 'normal' : thresholdLevel(gpu?.util, get('gpu-warning'), get('gpu-critical')),
    };
}

/**
 * One phrase for the header chip: the most serious thing going on.
 *
 * @param {object} snapshot
 * @param {(key: string) => number} get
 * @returns {{text: string, level: string}}
 */
export function status(snapshot, get) {
    const level = levels(snapshot, get);
    const mount = snapshot?.storage?.mounts?.[0]?.path ?? 'Disk';
    // Most likely to make the machine misbehave first.
    const candidates = [
        [level.memory, 'RAM high'],
        [level.storage, `${mount} almost full`],
        [level.cpu, 'CPU busy'],
        [level.temperature, 'Running hot'],
        [level.gpu, 'GPU busy'],
    ];
    let best = null;
    for (const [value, text] of candidates) {
        if (value !== 'normal' && (!best || LEVEL_RANK[value] > LEVEL_RANK[best.level]))
            best = {text, level: value};
    }
    return best ?? {text: 'All good', level: 'normal'};
}

/**
 * @param {object|null|undefined} storage
 * @returns {number|null}
 */
function combinedBps(storage) {
    const read = storage?.readBps;
    const write = storage?.writeBps;
    if (read == null && write == null)
        return null;
    return (read ?? 0) + (write ?? 0);
}

/**
 * The six overview tiles. `series` names the history arrays drawn under each
 * tile; `max` is the graph ceiling (null scales to the data).
 *
 * @param {object} snapshot
 * @param {(key: string) => number} get
 * @param {string} unit 'celsius' or 'fahrenheit'
 * @param {string|null} busiest Display name of the busiest program, if known.
 * @returns {{id: string, label: string, value: string, detail: string, level: string, series: string[], max: number|null}[]}
 */
export function tileModels(snapshot, get, unit, busiest = null) {
    const level = levels(snapshot, get);
    const cpu = snapshot?.cpu;
    const memory = snapshot?.memory;
    const mount = snapshot?.storage?.mounts?.[0];
    const gpu = snapshot?.gpu?.selected;
    const headline = snapshot?.temperature?.headline;
    const network = snapshot?.network;

    let gpuValue = formatPercent(gpu?.util);
    let gpuDetail = gpu ? `${gpu.vendor ?? ''}`.trim() || gpu.id || 'Graphics' : 'Not found';
    if (gpu?.state === 'off') {
        gpuValue = 'idle';
        gpuDetail = 'Sleeping, not woken';
    } else if (gpu?.vramTotal) {
        gpuDetail = `${formatBytes(gpu.vramUsed)} of ${formatBytes(gpu.vramTotal)}`;
    }

    return [
        {
            id: 'cpu',
            label: 'CPU',
            value: formatPercent(cpu?.usage),
            detail: busiest ? `Busiest: ${busiest}` : `${cpu?.cores?.length ?? '?'} cores`,
            level: level.cpu,
            series: ['cpu'],
            max: 100,
        },
        {
            id: 'memory',
            label: 'RAM',
            value: formatPercent(memory?.percent),
            detail: memory ? `${formatBytes(memory.used)} of ${formatBytes(memory.total)}` : 'n/a',
            level: level.memory,
            series: ['memory'],
            max: 100,
        },
        {
            id: 'storage',
            label: mount ? `Disk ${mount.path}` : 'Disk',
            value: formatPercent(mount?.percent),
            detail: mount
                ? `${formatBytes(Math.max(0, (mount.size ?? 0) - (mount.used ?? 0)))} free · ${formatBytesPerSecond(combinedBps(snapshot.storage))}`
                : 'n/a',
            level: level.storage,
            series: ['diskRead', 'diskWrite'],
            max: null,
        },
        {
            id: 'gpu',
            label: 'GPU',
            value: gpuValue,
            detail: gpuDetail,
            level: level.gpu,
            series: ['gpu'],
            max: 100,
        },
        {
            id: 'temperature',
            label: 'Temperature',
            value: headline?.state === 'off' ? 'idle' : formatTemperature(headline?.celsius, unit),
            detail: headline?.label ?? 'No sensor',
            level: level.temperature,
            series: ['temperature'],
            max: null,
        },
        {
            id: 'network',
            label: 'Network',
            value: `↓ ${formatBytesPerSecond(network?.downBps)}`,
            detail: `↑ ${formatBytesPerSecond(network?.upBps)}`,
            level: 'normal',
            series: ['netDown', 'netUp'],
            max: null,
        },
    ];
}
