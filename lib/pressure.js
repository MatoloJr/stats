import GLib from 'gi://GLib';

import {listChildren, readTextOrNull} from './io.js';
import {counterDelta} from './numbers.js';

const QUIET_US = 10 * 60 * 1000000;
const USER_HZ = 100;
const PAGE_SIZE = 4096;
const SHARE_CAP = 8;
const COMM_CAP = 32;
const READ_BATCH = 24;

/**
 * `comm` may contain spaces. utime and stime follow the closing parenthesis.
 *
 * @param {string} text
 * @returns {{comm: string, ticks: number}|null}
 */
export function parseProcessStat(text) {
    const open = text.indexOf('(');
    const close = text.lastIndexOf(')');
    if (open < 1 || close < open)
        return null;
    const comm = text.slice(open + 1, close);
    const rest = text.slice(close + 1).trim().split(/\s+/);
    if (rest.length < 13)
        return null;
    const utime = Number(rest[11]);
    const stime = Number(rest[12]);
    if (!comm || !Number.isFinite(utime) || !Number.isFinite(stime))
        return null;
    return {comm, ticks: utime + stime};
}

/**
 * Resident set is the second field of `/proc/pid/statm`, in pages.
 *
 * @param {string} text
 * @returns {number|null}
 */
export function parseResidentPages(text) {
    const pages = Number(text.trim().split(/\s+/)[1]);
    return Number.isFinite(pages) ? pages : null;
}

/**
 * @param {number} previousTicks
 * @param {number} nextTicks
 * @param {number} elapsedSeconds
 * @param {number} [userHz]
 * @returns {number|null}
 */
export function cpuShare(previousTicks, nextTicks, elapsedSeconds, userHz = USER_HZ) {
    const delta = counterDelta(previousTicks, nextTicks);
    if (delta == null || !(elapsedSeconds > 0) || !(userHz > 0))
        return null;
    return (delta / userHz / elapsedSeconds) * 100;
}

/**
 * @param {object} snapshot
 * @param {number} cpuWarning
 * @param {number} memoryWarning
 * @returns {'cpu'|'memory'|'either'|null}
 */
export function isHot(snapshot, cpuWarning, memoryWarning) {
    const cpu = snapshot.cpu?.usage;
    const memory = snapshot.memory?.percent;
    const cpuHot = cpu != null && cpu >= cpuWarning;
    const memoryHot = memory != null && memory >= memoryWarning;
    if (cpuHot && memoryHot)
        return 'either';
    if (cpuHot)
        return 'cpu';
    if (memoryHot)
        return 'memory';
    return null;
}

/**
 * @param {{comm: string, cpu: number|null, memory: number|null}[]} processes
 * @param {'cpu'|'memory'|'either'} reason
 */
export function chooseOffender(processes, reason) {
    let best = null;
    let bestScore = -1;
    for (const process of processes) {
        let score = null;
        if (reason === 'memory')
            score = process.memory;
        else if (reason === 'cpu')
            score = process.cpu;
        else
            score = Math.max(process.cpu ?? -1, process.memory ?? -1);
        if (score == null || !Number.isFinite(score) || score < 0)
            continue;
        if (score > bestScore) {
            best = process;
            bestScore = score;
        }
    }
    return best;
}

/**
 * @param {number|null} current
 * @param {number[]} history
 * @returns {boolean}
 */
export function isAboveBaseline(current, history) {
    if (current == null || !history || history.length < 2)
        return false;
    const mean = history.reduce((sum, value) => sum + value, 0) / history.length;
    return current >= 25 && current > mean * 2;
}

/**
 * @param {number[]} history
 * @returns {number}
 */
export function average(history) {
    if (!history?.length)
        return 0;
    return history.reduce((sum, value) => sum + value, 0) / history.length;
}

/**
 * @param {{comm: string, cpu: number|null, memory: number|null}} offender
 * @param {'cpu'|'memory'|'either'} reason
 * @param {number} baseline
 * @param {boolean} offerMonitor
 * @returns {{title: string, body: string}}
 */
export function pressureCopy(offender, reason, baseline, offerMonitor) {
    const usual = Math.round(baseline);
    const action = offerMonitor ? 'Quit it, or open System Monitor.' : 'Quit it.';
    const memoryLed = reason === 'memory' ||
        (reason === 'either' && (offender.memory ?? 0) >= (offender.cpu ?? 0));
    if (memoryLed) {
        return {
            title: 'Stats',
            body: `${offender.comm} is using ${Math.round(offender.memory)}% of memory (usual is about ${usual}%). The machine may feel slow. ${action}`,
        };
    }
    return {
        title: 'Stats',
        body: `${offender.comm} is using ${Math.round(offender.cpu)}% CPU (usual is about ${usual}%). The machine may feel slow. ${action}`,
    };
}

/**
 * Looks at processes only after CPU or memory has stayed hot, and only on
 * every third hot poll. The map of recent shares stays small.
 */
export class PressureWatch {
    constructor() {
        this._streak = 0;
        this._quietUntil = 0;
        this._scanning = false;
        this._previousTicks = new Map();
        this._previousUs = 0;
        this._shares = new Map();
    }

    resetStreak() {
        this._streak = 0;
    }

    /**
     * @param {object} snapshot
     * @param {Gio.Settings} settings
     * @param {Gio.Cancellable} cancellable
     * @returns {Promise<object|null>}
     */
    async consider(snapshot, settings, cancellable) {
        if (!settings.get_boolean('pressure-notifications')) {
            this.resetStreak();
            return null;
        }
        const reason = isHot(
            snapshot,
            settings.get_int('cpu-warning'),
            settings.get_int('memory-warning'));
        if (!reason) {
            this.resetStreak();
            return null;
        }
        this._streak += 1;
        if (this._streak < 3 || this._streak % 3 !== 0 || this._scanning)
            return null;
        const now = GLib.get_monotonic_time();
        if (now < this._quietUntil)
            return null;
        this._scanning = true;
        try {
            return await this._scan(snapshot, reason, now, cancellable);
        } finally {
            this._scanning = false;
        }
    }

    /**
     * @param {string} comm
     * @param {number} value
     */
    _remember(comm, value) {
        if (!this._shares.has(comm) && this._shares.size >= COMM_CAP) {
            const oldest = this._shares.keys().next().value;
            this._shares.delete(oldest);
        }
        const history = this._shares.get(comm) ?? [];
        history.push(value);
        if (history.length > SHARE_CAP)
            history.shift();
        this._shares.set(comm, history);
    }

    /**
     * @param {object} snapshot
     * @param {'cpu'|'memory'|'either'} reason
     * @param {number} now
     * @param {Gio.Cancellable} cancellable
     */
    async _scan(snapshot, reason, now, cancellable) {
        const names = await listChildren('/proc', cancellable);
        const pids = names.filter(name => /^\d+$/.test(name));
        const processes = [];
        const nextTicks = new Map();
        const elapsed = this._previousUs > 0 ? (now - this._previousUs) / 1e6 : 0;
        const total = snapshot.memory?.total;
        for (let index = 0; index < pids.length; index += READ_BATCH) {
            const batch = pids.slice(index, index + READ_BATCH);
            const rows = await Promise.all(batch.map(async pid => {
                const [stat, statm] = await Promise.all([
                    readTextOrNull(`/proc/${pid}/stat`, cancellable),
                    readTextOrNull(`/proc/${pid}/statm`, cancellable),
                ]);
                return {pid, stat, statm};
            }));
            for (const row of rows) {
                if (!row.stat)
                    continue;
                const parsed = parseProcessStat(row.stat);
                if (!parsed)
                    continue;
                nextTicks.set(row.pid, parsed.ticks);
                const previous = this._previousTicks.get(row.pid);
                const cpu = previous == null
                    ? null
                    : cpuShare(previous, parsed.ticks, elapsed);
                const pages = row.statm ? parseResidentPages(row.statm) : null;
                const memory = pages != null && total > 0
                    ? (pages * PAGE_SIZE / total) * 100
                    : null;
                processes.push({comm: parsed.comm, cpu, memory});
            }
        }
        this._previousTicks = nextTicks;
        this._previousUs = now;
        const offender = chooseOffender(processes, reason);
        if (!offender)
            return null;
        const memoryLed = reason === 'memory' ||
            (reason === 'either' && (offender.memory ?? 0) >= (offender.cpu ?? 0));
        const value = memoryLed ? offender.memory : offender.cpu;
        if (value == null)
            return null;
        const prior = (this._shares.get(offender.comm) ?? []).slice();
        this._remember(offender.comm, value);
        if (!isAboveBaseline(value, prior))
            return null;
        this._quietUntil = now + QUIET_US;
        return {
            comm: offender.comm,
            cpu: offender.cpu,
            memory: offender.memory,
            reason: memoryLed ? 'memory' : 'cpu',
            baseline: average(prior),
        };
    }
}
