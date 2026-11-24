import GLib from 'gi://GLib';

import {counterDelta} from '../numbers.js';
import {readText} from '../io.js';
import {Sensor} from './sensor.js';

/**
 * @param {string} text
 * @returns {{name: string, rxBytes: number, txBytes: number}[]}
 */
export function parseNetDev(text) {
    const ifaces = [];
    for (const line of text.split('\n')) {
        const colon = line.indexOf(':');
        if (colon < 0)
            continue;
        const name = line.slice(0, colon).trim();
        const parts = line.slice(colon + 1).trim().split(/\s+/);
        if (!name || parts.length < 9)
            continue;
        const rxBytes = Number(parts[0]);
        const txBytes = Number(parts[8]);
        if (!Number.isFinite(rxBytes) || !Number.isFinite(txBytes))
            continue;
        ifaces.push({name, rxBytes, txBytes});
    }
    return ifaces;
}

/**
 * @param {{name: string, rxBytes: number, txBytes: number}[]} ifaces
 * @returns {{rxBytes: number, txBytes: number}}
 */
export function sumTraffic(ifaces) {
    return ifaces.reduce((total, iface) => {
        if (iface.name === 'lo')
            return total;
        return {
            rxBytes: total.rxBytes + iface.rxBytes,
            txBytes: total.txBytes + iface.txBytes,
        };
    }, {rxBytes: 0, txBytes: 0});
}

/**
 * @param {object|null} previous
 * @param {object|null} next
 * @param {number} elapsedSeconds
 * @returns {{downBps: number, upBps: number}|null}
 */
export function ratesFromDeltas(previous, next, elapsedSeconds) {
    if (!previous || !next || !(elapsedSeconds > 0))
        return null;
    const down = counterDelta(previous.rxBytes, next.rxBytes);
    const up = counterDelta(previous.txBytes, next.txBytes);
    if (down == null || up == null)
        return null;
    return {
        downBps: down / elapsedSeconds,
        upBps: up / elapsedSeconds,
    };
}

export class NetworkSensor extends Sensor {
    constructor() {
        super('network', 'Network');
        this._previous = null;
        this._previousUs = 0;
    }

    /**
     * @param {object} context
     */
    async sample(context) {
        try {
            const text = await readText('/proc/net/dev', context.cancellable);
            const now = GLib.get_monotonic_time();
            const totals = sumTraffic(parseNetDev(text));
            const elapsed = this._previousUs > 0 ? (now - this._previousUs) / 1e6 : 0;
            const rates = ratesFromDeltas(this._previous, totals, elapsed);
            this._previous = totals;
            this._previousUs = now;
            this.clearWarning('netdev');
            return {
                downBps: rates?.downBps ?? null,
                upBps: rates?.upBps ?? null,
            };
        } catch (error) {
            if (this.isCancelled(error))
                throw error;
            this.warnOnce('netdev', `unable to read /proc/net/dev (${error.message})`);
            return null;
        }
    }

    destroy() {
        super.destroy();
        this._previous = null;
        this._previousUs = 0;
    }
}
