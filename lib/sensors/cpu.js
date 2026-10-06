import {clampPercent, counterDelta} from '../numbers.js';
import {listChildren, readText} from '../io.js';
import {Sensor} from './sensor.js';

/**
* Parse one /proc/stat snapshot. Guest jiffies are already included in user/nice.
*
* @param {string} text
* @returns {{aggregate: object|null, cores: object[]}}
*/
export function parseProcStat(text) {
    const rows = [];
    for (const line of text.split('\n')) {
        const match = /^(cpu\d*)\s+(\d+(?:\s+\d+)*)$/.exec(line);
        if (!match)
            continue;
        const parts = match[2].split(/\s+/).map(Number);
        const user = parts[0] ?? 0;
        const nice = parts[1] ?? 0;
        const system = parts[2] ?? 0;
        const idle = parts[3] ?? 0;
        const iowait = parts[4] ?? 0;
        const irq = parts[5] ?? 0;
        const softirq = parts[6] ?? 0;
        const steal = parts[7] ?? 0;
        rows.push({
            id: match[1],
            idleAll: idle + iowait,
            total: user + nice + system + idle + iowait + irq + softirq + steal,
        });
    }
    return {
        aggregate: rows.find(row => row.id === 'cpu') ?? null,
        cores: rows.filter(row => row.id !== 'cpu'),
    };
}

/**
* Busy percent between two parseProcStat rows.
* Returns null on the first sample and when counters wrap or reset.
*
* @param {{idleAll: number, total: number}|null} previous
* @param {{idleAll: number, total: number}|null} next
* @returns {number|null}
*/
export function usagePercent(previous, next) {
    if (!previous || !next)
        return null;
    const total = counterDelta(previous.total, next.total);
    const idle = counterDelta(previous.idleAll, next.idleAll);
    if (total == null || idle == null)
        return null;
    if (total === 0)
        return 0;
    const idleClamped = Math.min(idle, total);
    return clampPercent(((total - idleClamped) / total) * 100);
}

/**
* @param {string} text kHz contents of scaling_cur_freq
* @returns {number|null} MHz
*/
export function parseFrequencyKhz(text) {
    const khz = Number(String(text).trim());
    if (!Number.isFinite(khz) || khz < 0)
        return null;
    return khz / 1000;
}

export class CpuSensor extends Sensor {
    constructor() {
        super('cpu', 'CPU');
        this._previous = null;
        this._corePaths = null;
    }

    /**
    * @param {object} context
    */
    async init(context) {
        this._corePaths = await this._frequencyPaths(context.cancellable);
    }

    /**
    * @param {object} context
    */
    async sample(context) {
        try {
            const text = await readText('/proc/stat', context.cancellable);
            const parsed = parseProcStat(text);
            const usage = usagePercent(this._previous?.aggregate, parsed.aggregate);
            const cores = parsed.cores.map(core => ({
                id: core.id,
                usage: usagePercent(this._previous?.cores.get(core.id), core),
            }));
            const coreMap = new Map(parsed.cores.map(core => [core.id, core]));
            this._previous = {aggregate: parsed.aggregate, cores: coreMap};
            const frequency = await this._readFrequency(context.cancellable);
            this.clearWarning('sample');
            return {usage, cores, frequency};
        } catch (error) {
            if (this.isCancelled(error))
                throw error;
            this.warnOnce('sample', `unable to read /proc/stat (${error.message})`);
            return null;
        }
    }

    destroy() {
        super.destroy();
        this._previous = null;
        this._corePaths = null;
    }

    /**
    * @param {Gio.Cancellable} cancellable
    * @returns {Promise<string[]>}
    */
    async _frequencyPaths(cancellable) {
        try {
            const names = await listChildren('/sys/devices/system/cpu', cancellable);
            return names
                .filter(name => /^cpu\d+$/.test(name))
                .sort((a, b) => Number(a.slice(3)) - Number(b.slice(3)))
                .map(name => `/sys/devices/system/cpu/${name}/cpufreq/scaling_cur_freq`);
        } catch (error) {
            if (this.isCancelled(error))
                throw error;
            return [];
        }
    }

    /**
    * @param {Gio.Cancellable} cancellable
    * @returns {Promise<{averageMhz: number, maxMhz: number}|null>}
    */
    async _readFrequency(cancellable) {
        if (!this._corePaths)
            this._corePaths = await this._frequencyPaths(cancellable);
        if (this._corePaths.length === 0)
            return null;
        const values = [];
        await Promise.all(this._corePaths.map(async path => {
            try {
                const text = await readText(path, cancellable);
                const mhz = parseFrequencyKhz(text);
                if (mhz != null)
                    values.push(mhz);
            } catch (error) {
                if (this.isCancelled(error))
                    throw error;
            }
        }));
        if (values.length === 0)
            return null;
        const sum = values.reduce((total, value) => total + value, 0);
        return {
            averageMhz: sum / values.length,
            maxMhz: Math.max(...values),
        };
    }
}
