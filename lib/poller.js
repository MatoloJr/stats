import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

import {logError} from './log.js';
import {CpuSensor} from './sensors/cpu.js';
import {DisplaySensor} from './sensors/display.js';
import {GpuSensor} from './sensors/gpu/index.js';
import {MemorySensor} from './sensors/memory.js';
import {StorageSensor} from './sensors/storage.js';
import {TemperatureSensor} from './sensors/temperature.js';

/**
 * Fixed-capacity sample history. The newest value is at the end.
 */
export class Ring {
    /**
     * @param {number} capacity
     */
    constructor(capacity) {
        this._capacity = Math.max(2, capacity);
        this._values = [];
    }

    /**
     * @param {number} capacity
     */
    setCapacity(capacity) {
        this._capacity = Math.max(2, capacity);
        if (this._values.length > this._capacity)
            this._values.splice(0, this._values.length - this._capacity);
    }

    /**
     * @param {number|null} value
     */
    push(value) {
        this._values.push(value);
        if (this._values.length > this._capacity)
            this._values.shift();
    }

    /**
     * @returns {(number|null)[]}
     */
    values() {
        return this._values;
    }
}

/**
 * One GLib timeout that samples every enabled sensor in parallel.
 */
export class Poller {
    /**
     * @param {Gio.Settings} settings
     */
    constructor(settings) {
        this._settings = settings;
        this._sensors = [
            new CpuSensor(),
            new MemorySensor(),
            new StorageSensor(),
            new TemperatureSensor(),
            new GpuSensor(),
            new DisplaySensor(),
        ];
        this._listeners = new Set();
        this._history = {
            cpu: new Ring(60),
            memory: new Ring(60),
            gpu: new Ring(60),
            diskRead: new Ring(60),
            diskWrite: new Ring(60),
            temperature: new Ring(60),
        };
        this._latest = {};
        this._snapshot = null;
        this._timeoutId = 0;
        this._inFlight = false;
        this._paused = false;
        this._started = false;
        this._cancellable = new Gio.Cancellable();
        this._settingIds = [
            settings.connect('changed::poll-interval-ms', () => this._arm()),
            settings.connect('changed::history-seconds', () => this._resizeHistory()),
        ];
        this._resizeHistory();
    }

    /**
     * @param {(snapshot: object) => void} listener
     * @returns {() => void}
     */
    subscribe(listener) {
        this._listeners.add(listener);
        if (this._snapshot)
            listener(this._snapshot);
        return () => this._listeners.delete(listener);
    }

    start() {
        if (this._started)
            return;
        this._started = true;
        const context = this._context();
        for (const sensor of this._sensors) {
            sensor.init(context).catch(error => {
                if (!sensor.isCancelled(error))
                    sensor.warnOnce('init', error.message);
            });
        }
        this._arm();
        this._tick();
    }

    pause() {
        this._paused = true;
        this._disarm();
        this._cancellable.cancel();
        this._cancellable = new Gio.Cancellable();
        for (const sensor of this._sensors)
            sensor.setPaused(true);
    }

    resume() {
        if (!this._started)
            return;
        const wasPaused = this._paused;
        this._paused = false;
        for (const sensor of this._sensors)
            sensor.setPaused(false);
        if (wasPaused || !this._timeoutId) {
            this._tick();
            this._arm();
        }
    }

    destroy() {
        this._paused = true;
        this._started = false;
        this._disarm();
        this._cancellable.cancel();
        for (const id of this._settingIds)
            this._settings.disconnect(id);
        this._settingIds = [];
        for (const sensor of this._sensors)
            sensor.destroy();
        this._sensors = [];
        this._listeners.clear();
        this._snapshot = null;
        this._settings = null;
    }

    _context() {
        return {
            settings: this._settings,
            cancellable: this._cancellable,
            requestUpdate: () => this._publish(),
        };
    }

    _resizeHistory() {
        if (!this._settings)
            return;
        const seconds = this._settings.get_int('history-seconds');
        const interval = this._settings.get_int('poll-interval-ms');
        const capacity = Math.max(2, Math.round(seconds / (interval / 1000)));
        for (const ring of Object.values(this._history))
            ring.setCapacity(capacity);
    }

    _arm() {
        this._disarm();
        if (!this._settings || this._paused || !this._started)
            return;
        this._resizeHistory();
        const interval = this._settings.get_int('poll-interval-ms');
        this._timeoutId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, interval, () => {
            this._tick();
            return GLib.SOURCE_CONTINUE;
        });
        GLib.Source.set_name_by_id(this._timeoutId, '[stats] poll');
    }

    _disarm() {
        if (this._timeoutId) {
            GLib.source_remove(this._timeoutId);
            this._timeoutId = 0;
        }
    }

    async _tick() {
        if (!this._settings || this._paused || this._inFlight)
            return;
        this._inFlight = true;
        const cancellable = this._cancellable;
        const context = {
            settings: this._settings,
            cancellable,
            requestUpdate: () => this._publish(),
        };
        try {
            const pairs = await Promise.all(this._sensors.map(async sensor => {
                if (sensor.id === 'display')
                    return [sensor.id, await sensor.sample(context)];
                try {
                    return [sensor.id, await sensor.sample(context)];
                } catch (error) {
                    if (sensor.isCancelled(error) || cancellable.is_cancelled())
                        return [sensor.id, this._latest[sensor.id] ?? null];
                    sensor.warnOnce('tick', error.message);
                    logError(`${sensor.id} sample failed: ${error.message}`);
                    return [sensor.id, null];
                }
            }));
            if (!this._settings || cancellable.is_cancelled())
                return;
            for (const [id, value] of pairs)
                this._latest[id] = value;
            this._pushHistory();
            this._publish();
        } finally {
            this._inFlight = false;
        }
    }

    _pushHistory() {
        const gpu = this._latest.gpu?.selected;
        this._history.cpu.push(this._latest.cpu?.usage ?? null);
        this._history.memory.push(this._latest.memory?.percent ?? null);
        this._history.gpu.push(gpu?.state === 'off' ? null : (gpu?.util ?? null));
        this._history.diskRead.push(this._latest.storage?.readBps ?? null);
        this._history.diskWrite.push(this._latest.storage?.writeBps ?? null);
        this._history.temperature.push(this._latest.temperature?.headline?.celsius ?? null);
    }

    _publish() {
        if (!this._settings)
            return;
        const history = {};
        for (const [key, ring] of Object.entries(this._history))
            history[key] = ring.values();
        this._snapshot = {...this._latest, history};
        for (const listener of this._listeners)
            listener(this._snapshot);
    }
}
