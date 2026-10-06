import {listChildren, readTextOrNull} from '../io.js';
import {Sensor} from './sensor.js';

/**
 * @param {number} millidegree
 * @returns {number|null}
 */
export function celsiusFromMilli(millidegree) {
    if (!Number.isFinite(millidegree))
        return null;
    return millidegree / 1000;
}

/**
 * Turn hwmon chips and thermal zones into labelled sensors.
 * `preferred` marks the headline candidate for that chip (package / Tctl / composite).
 *
 * @param {{id: string, name: string, suspended?: boolean, temps: {label: string, millidegree: number, index: number}[]}[]} chips
 * @param {{type: string, millidegree: number, id: string}[]} zones
 */
export function buildTemperatureCatalog(chips, zones) {
    const sensors = [];
    for (const chip of chips) {
        const role = roleForChip(chip.name);
        if (chip.suspended) {
            if (role === 'gpu') {
                sensors.push({
                    id: `${chip.id}:off`,
                    chip: chip.name,
                    label: chip.name,
                    role,
                    celsius: null,
                    state: 'off',
                    preferred: false,
                });
            }
            continue;
        }
        for (const temp of chip.temps) {
            const label = temp.label || `temp${temp.index}`;
            sensors.push({
                id: `${chip.id}:${label}`,
                chip: chip.name,
                label,
                role,
                celsius: celsiusFromMilli(temp.millidegree),
                state: 'active',
                preferred: isPreferredLabel(chip.name, label),
            });
        }
    }
    for (const zone of zones) {
        if (zone.type !== 'x86_pkg_temp')
            continue;
        sensors.push({
            id: zone.id,
            chip: zone.type,
            label: zone.type,
            role: 'cpu',
            celsius: celsiusFromMilli(zone.millidegree),
            state: 'active',
            preferred: true,
        });
    }
    return sensors;
}

/**
 * @param {string} name hwmon name
 * @returns {'cpu'|'disk'|'gpu'|'other'}
 */
export function roleForChip(name) {
    if (name === 'coretemp' || name === 'k10temp')
        return 'cpu';
    if (name.includes('nvme'))
        return 'disk';
    if (name === 'amdgpu' || name === 'nvidia')
        return 'gpu';
    return 'other';
}

/**
 * @param {string} chip
 * @param {string} label
 * @returns {boolean}
 */
export function isPreferredLabel(chip, label) {
    if (chip === 'coretemp')
        return label === 'Package id 0';
    if (chip === 'k10temp')
        return label === 'Tctl' || label === 'Tdie';
    if (chip.includes('nvme'))
        return label === 'Composite' || label === 'temp1';
    if (chip === 'amdgpu' || chip === 'nvidia')
        return label === 'edge' || label === 'temp1';
    return false;
}

/**
 * Automatic headline: preferred CPU package, then any CPU sensor, then the first sensor.
 * An explicit id from preferences wins when it is still present.
 *
 * @param {object[]} sensors
 * @param {string} preferredId
 */
export function chooseHeadline(sensors, preferredId) {
    if (preferredId) {
        const chosen = sensors.find(sensor => sensor.id === preferredId && sensor.celsius != null);
        if (chosen)
            return chosen;
    }
    const preferredCpu = sensors.find(sensor => sensor.role === 'cpu' && sensor.preferred && sensor.celsius != null);
    if (preferredCpu)
        return preferredCpu;
    const anyCpu = sensors.find(sensor => sensor.role === 'cpu' && sensor.celsius != null);
    if (anyCpu)
        return anyCpu;
    return sensors.find(sensor => sensor.celsius != null) ?? null;
}

export class TemperatureSensor extends Sensor {
    constructor() {
        super('temperature', 'Temperature');
    }

    /**
     * @param {object} context
     */
    async sample(context) {
        try {
            const catalog = await this.discover(context.cancellable);
            const preferred = context.settings?.get_string('headline-temp') ?? '';
            const headline = chooseHeadline(catalog, preferred);
            this.clearWarning('scan');
            return {sensors: catalog, headline};
        } catch (error) {
            if (this.isCancelled(error))
                throw error;
            this.warnOnce('scan', `temperature scan failed (${error.message})`);
            return null;
        }
    }

    /**
     * @param {Gio.Cancellable|null} cancellable
     */
    async discover(cancellable) {
        const chips = await this._readHwmons(cancellable);
        const zones = await this._readZones(cancellable);
        return buildTemperatureCatalog(chips, zones);
    }

    /**
     * @param {Gio.Cancellable|null} cancellable
     */
    async _readHwmons(cancellable) {
        let names = [];
        try {
            names = await listChildren('/sys/class/hwmon', cancellable);
        } catch (error) {
            if (this.isCancelled(error))
                throw error;
            this.warnOnce('hwmon', 'no hwmon sensors');
            return [];
        }
        const chips = [];
        await Promise.all(names.filter(name => name.startsWith('hwmon')).map(async name => {
            const base = `/sys/class/hwmon/${name}`;
            const chipName = (await readTextOrNull(`${base}/name`, cancellable))?.trim();
            if (!chipName)
                return;
            const status = (await readTextOrNull(`${base}/device/power/runtime_status`, cancellable))?.trim();
            if (status === 'suspended') {
                chips.push({id: name, name: chipName, suspended: true, temps: []});
                return;
            }
            const temps = await this._readTemps(base, cancellable);
            chips.push({id: name, name: chipName, suspended: false, temps});
        }));
        return chips;
    }

    /**
     * @param {string} base
     * @param {Gio.Cancellable|null} cancellable
     */
    async _readTemps(base, cancellable) {
        let entries = [];
        try {
            entries = await listChildren(base, cancellable);
        } catch (error) {
            if (this.isCancelled(error))
                throw error;
            return [];
        }
        const indexes = entries
            .map(entry => /^temp(\d+)_input$/.exec(entry))
            .filter(Boolean)
            .map(match => Number(match[1]));
        const temps = [];
        await Promise.all(indexes.map(async index => {
            const milli = Number((await readTextOrNull(`${base}/temp${index}_input`, cancellable))?.trim());
            if (!Number.isFinite(milli))
                return;
            const label = (await readTextOrNull(`${base}/temp${index}_label`, cancellable))?.trim() ?? '';
            temps.push({index, label, millidegree: milli});
        }));
        temps.sort((a, b) => a.index - b.index);
        return temps;
    }

    /**
     * @param {Gio.Cancellable|null} cancellable
     */
    async _readZones(cancellable) {
        let names = [];
        try {
            names = await listChildren('/sys/class/thermal', cancellable);
        } catch (error) {
            if (this.isCancelled(error))
                throw error;
            return [];
        }
        const zones = [];
        await Promise.all(names.filter(name => name.startsWith('thermal_zone')).map(async name => {
            const base = `/sys/class/thermal/${name}`;
            const type = (await readTextOrNull(`${base}/type`, cancellable))?.trim();
            const milli = Number((await readTextOrNull(`${base}/temp`, cancellable))?.trim());
            if (!type || !Number.isFinite(milli))
                return;
            zones.push({id: name, type, millidegree: milli});
        }));
        return zones;
    }
}
