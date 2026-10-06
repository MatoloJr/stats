import GLib from 'gi://GLib';

import {clampPercent, counterDelta} from '../numbers.js';
import {queryFilesystem, readText} from '../io.js';
import {Sensor} from './sensor.js';

const SECTOR_BYTES = 512;
const FS_ATTRIBUTES = 'filesystem::size,filesystem::used,filesystem::free';

/**
* @param {string} text
* @returns {{name: string, readSectors: number, writeSectors: number}[]}
*/
export function parseDiskstats(text) {
    const devices = [];
    for (const line of text.split('\n')) {
        const parts = line.trim().split(/\s+/);
        if (parts.length < 14)
            continue;
        const readSectors = Number(parts[5]);
        const writeSectors = Number(parts[9]);
        if (!Number.isFinite(readSectors) || !Number.isFinite(writeSectors))
            continue;
        devices.push({
            name: parts[2],
            readSectors,
            writeSectors,
        });
    }
    return devices;
}

/**
* @param {string} name
* @returns {boolean}
*/
export function isVirtualDevice(name) {
    return name.startsWith('loop') ||
        name.startsWith('ram') ||
        name.startsWith('dm-') ||
        name.startsWith('zram') ||
        name.startsWith('fd');
}

/**
* Parent of a partition, or null for a whole disk.
* nvme0n1 is a disk; nvme0n1p1 is a partition.
*
* @param {string} name
* @returns {string|null}
*/
export function partitionParent(name) {
    const nvme = /^(nvme\d+n\d+)p\d+$/.exec(name);
    if (nvme)
        return nvme[1];
    const mmc = /^(mmcblk\d+)p\d+$/.exec(name);
    if (mmc)
        return mmc[1];
    const scsi = /^(\D+)\d+$/.exec(name);
    if (scsi)
        return scsi[1];
    return null;
}

/**
* Keep whole physical disks. Drop loop/ram/dm and partitions whose parent
* is already present, so a disk is not counted twice.
*
* @param {{name: string}[]} devices
* @returns {typeof devices}
*/
export function physicalDevices(devices) {
    const names = new Set(devices.map(device => device.name));
    return devices.filter(device => {
        if (isVirtualDevice(device.name))
            return false;
        const parent = partitionParent(device.name);
        return !(parent && names.has(parent));
    });
}

/**
* @param {object|null} previous
* @param {object|null} next
* @param {number} elapsedSeconds
* @returns {{readBps: number, writeBps: number}|null}
*/
export function throughputFromDeltas(previous, next, elapsedSeconds) {
    if (!previous || !next || !(elapsedSeconds > 0))
        return null;
    const readSectors = counterDelta(previous.readSectors, next.readSectors);
    const writeSectors = counterDelta(previous.writeSectors, next.writeSectors);
    if (readSectors == null || writeSectors == null)
        return null;
    return {
        readBps: (readSectors * SECTOR_BYTES) / elapsedSeconds,
        writeBps: (writeSectors * SECTOR_BYTES) / elapsedSeconds,
    };
}

/**
* Sum sector counters for a device list.
*
* @param {{readSectors: number, writeSectors: number}[]} devices
*/
export function sumSectors(devices) {
    return devices.reduce((total, device) => ({
        readSectors: total.readSectors + device.readSectors,
        writeSectors: total.writeSectors + device.writeSectors,
    }), {readSectors: 0, writeSectors: 0});
}

export class StorageSensor extends Sensor {
    constructor() {
        super('storage', 'Storage');
        this._previous = null;
        this._previousUs = 0;
    }

    /**
    * @param {object} context
    */
    async sample(context) {
        const mounts = await this._readMounts(context);
        const throughput = await this._readThroughput(context);
        return {
            mounts,
            readBps: throughput?.readBps ?? null,
            writeBps: throughput?.writeBps ?? null,
        };
    }

    destroy() {
        super.destroy();
        this._previous = null;
        this._previousUs = 0;
    }

    /**
    * @param {object} context
    */
    async _readMounts(context) {
        const points = context.settings?.get_strv('mount-points') ?? ['/'];
        const mounts = [];
        await Promise.all(points.map(async point => {
            try {
                const info = await queryFilesystem(point, FS_ATTRIBUTES, context.cancellable);
                const size = info.get_attribute_uint64('filesystem::size');
                const used = info.get_attribute_uint64('filesystem::used');
                const free = info.get_attribute_uint64('filesystem::free');
                mounts.push({
                    path: point,
                    size,
                    used,
                    free,
                    percent: size > 0 ? clampPercent((used / size) * 100) : 0,
                });
                this.clearWarning(`mount:${point}`);
            } catch (error) {
                if (this.isCancelled(error))
                    throw error;
                this.warnOnce(`mount:${point}`, `mount ${point} is unreadable (${error.message})`);
            }
        }));
        mounts.sort((a, b) => points.indexOf(a.path) - points.indexOf(b.path));
        return mounts;
    }

    /**
    * @param {object} context
    */
    async _readThroughput(context) {
        try {
            const text = await readText('/proc/diskstats', context.cancellable);
            const now = GLib.get_monotonic_time();
            const totals = sumSectors(physicalDevices(parseDiskstats(text)));
            const elapsed = this._previousUs > 0 ? (now - this._previousUs) / 1e6 : 0;
            const throughput = throughputFromDeltas(this._previous, totals, elapsed);
            this._previous = totals;
            this._previousUs = now;
            this.clearWarning('diskstats');
            return throughput;
        } catch (error) {
            if (this.isCancelled(error))
                throw error;
            this.warnOnce('diskstats', `unable to read /proc/diskstats (${error.message})`);
            return null;
        }
    }
}
