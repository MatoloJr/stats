import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

import {logError} from './log.js';
import {isCancelled} from './io.js';
import {PressureWatch} from './pressure.js';
import {CpuSensor} from './sensors/cpu.js';
import {DisplaySensor} from './sensors/display.js';
import {GpuSensor} from './sensors/gpu/index.js';
import {MemorySensor} from './sensors/memory.js';
import {NetworkSensor} from './sensors/network.js';
import {StorageSensor} from './sensors/storage.js';
import {TemperatureSensor} from './sensors/temperature.js';

const STARTUP_IDS = ['cpu', 'memory', 'storage', 'network'];

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
            new NetworkSensor(),
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
            netDown: new Ring(60),
            netUp: new Ring(60),
        };
        this._progress = 0;
        this._startupDone = false;
        this._seenStartup = new Set();
        this._pressure = new PressureWatch();
        this._pressureHandler = null;
        this._latest = {};
        this._snapshot = null;
        this._timeoutId = 0;
        this._inFlight = false;
        this._paused = false;
        this._started = false;
        this._cancellable = new Gio.Cancellable();
        this._requestUpdate = () => {
            const display = this._sensors.find(sensor => sensor.id === 'display');
            if (display?.current)
                this._latest.display = display.current();
            this._publish();
        };
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

    /**
     * @param {(notice: object) => void} handler
     */
    setPressureHandler(handler) {
        this._pressureHandler = handler;
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
        this._pressure.resetStreak();
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
        this._pressureHandler = null;
        this._snapshot = null;
        this._settings = null;
    }

    _context() {
        return {
            settings: this._settings,
            cancellable: this._cancellable,
            requestUpdate: this._requestUpdate,
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
        const context = this._context();
        context.cancellable = cancellable;
        try {
            const pairs = await Promise.all(this._sensors.map(async sensor => {
                let value;
                try {
                    value = await sensor.sample(context);
                } catch (error) {
                    if (sensor.isCancelled(error) || cancellable.is_cancelled())
                        value = this._latest[sensor.id] ?? null;
                    else {
                        sensor.warnOnce('tick', error.message);
                        logError(`${sensor.id} sample failed: ${error.message}`);
                        value = null;
                    }
                }
                this._noteStartup(sensor.id, value);
                return [sensor.id, value];
            }));
            if (!this._settings || cancellable.is_cancelled())
                return;
            for (const [id, value] of pairs)
                this._latest[id] = value;
            this._pushHistory();
            this._publish();
            this._watchPressure(cancellable);
        } finally {
            this._inFlight = false;
        }
    }

    /**
     * @param {string} id
     * @param {object|null} value
     */
    _noteStartup(id, value) {
        if (this._startupDone || !STARTUP_IDS.includes(id) || this._seenStartup.has(id))
            return;
        this._seenStartup.add(id);
        this._latest[id] = value;
        this._progress = this._seenStartup.size / STARTUP_IDS.length;
        if (this._seenStartup.size >= STARTUP_IDS.length)
            this._startupDone = true;
        this._publish();
    }

    /**
     * @param {Gio.Cancellable} cancellable
     */
    _watchPressure(cancellable) {
        const settings = this._settings;
        const snapshot = this._snapshot;
        if (!settings || !snapshot || !this._pressureHandler)
            return;
        this._pressure.consider(snapshot, settings, cancellable).then(notice => {
            if (notice && this._pressureHandler)
                this._pressureHandler(notice);
        }).catch(error => {
            if (!isCancelled(error))
                logError(`pressure scan failed: ${error.message}`);
        });
    }

    _pushHistory() {
        const gpu = this._latest.gpu?.selected;
        this._history.cpu.push(this._latest.cpu?.usage ?? null);
        this._history.memory.push(this._latest.memory?.percent ?? null);
        this._history.gpu.push(gpu?.state === 'off' ? null : (gpu?.util ?? null));
        this._history.diskRead.push(this._latest.storage?.readBps ?? null);
        this._history.diskWrite.push(this._latest.storage?.writeBps ?? null);
        this._history.temperature.push(this._latest.temperature?.headline?.celsius ?? null);
        this._history.netDown.push(this._latest.network?.downBps ?? null);
        this._history.netUp.push(this._latest.network?.upBps ?? null);
    }

    _publish() {
        if (!this._settings)
            return;
        const history = {};
        for (const [key, ring] of Object.entries(this._history))
            history[key] = ring.values();
        this._snapshot = {
            ...this._latest,
            history,
            progress: this._startupDone ? 1 : this._progress,
        };
        for (const listener of this._listeners)
            listener(this._snapshot);
    }
}
