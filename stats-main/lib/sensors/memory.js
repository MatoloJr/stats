import {clampPercent} from '../numbers.js';
import {readText} from '../io.js';
import {Sensor} from './sensor.js';

/**
 * Parse /proc/meminfo. Used memory is MemTotal - MemAvailable, never MemFree.
 * Values in the file are KiB; the result is bytes.
 *
 * @param {string} text
 * @returns {object|null}
 */
export function parseMeminfo(text) {
    const fields = {};
    for (const line of text.split('\n')) {
        const match = /^(\w+):\s+(\d+)/.exec(line);
        if (match)
            fields[match[1]] = Number(match[2]) * 1024;
    }
    if (fields.MemTotal == null || fields.MemAvailable == null)
        return null;
    const total = fields.MemTotal;
    const available = fields.MemAvailable;
    const used = Math.max(0, total - available);
    const swapTotal = fields.SwapTotal ?? 0;
    const swapFree = fields.SwapFree ?? 0;
    const swapUsed = Math.max(0, swapTotal - swapFree);
    return {
        total,
        used,
        available,
        percent: total > 0 ? clampPercent((used / total) * 100) : 0,
        cached: fields.Cached ?? 0,
        buffers: fields.Buffers ?? 0,
        swapTotal,
        swapUsed,
        swapPercent: swapTotal > 0 ? clampPercent((swapUsed / swapTotal) * 100) : 0,
    };
}

export class MemorySensor extends Sensor {
    constructor() {
        super('memory', 'Memory');
    }

    /**
     * @param {object} context
     */
    async sample(context) {
        try {
            const text = await readText('/proc/meminfo', context.cancellable);
            const parsed = parseMeminfo(text);
            if (!parsed) {
                this.warnOnce('sample', '/proc/meminfo is missing MemTotal or MemAvailable');
                return null;
            }
            this.clearWarning('sample');
            return parsed;
        } catch (error) {
            if (this.isCancelled(error))
                throw error;
            this.warnOnce('sample', `unable to read /proc/meminfo (${error.message})`);
            return null;
        }
    }
}
