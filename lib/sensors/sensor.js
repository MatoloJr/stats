import Gio from 'gi://Gio';

import {logWarn} from '../log.js';

/**
* Shared sensor interface. A metric is one subclass: id, label, init, sample, destroy.
*/
export class Sensor {
    /**
    * @param {string} id
    * @param {string} label
    */
    constructor(id, label) {
        this.id = id;
        this.label = label;
        this._warnings = new Set();
        this._paused = false;
    }

    /**
    * @param {object} _context
    * @returns {Promise<void>}
    */
    async init(_context) {}

    /**
    * @param {object} _context
    * @returns {Promise<object|null>}
    */
    async sample(_context) {
        return null;
    }

    /**
    * Stop background work. The poller calls this from disable().
    */
    destroy() {
        this._paused = true;
    }

    /**
    * @param {boolean} paused
    */
    setPaused(paused) {
        this._paused = paused;
    }

    /**
    * Log a sensor failure once until the key succeeds again.
    *
    * @param {string} key
    * @param {string} message
    */
    warnOnce(key, message) {
        if (this._warnings.has(key))
            return;
        this._warnings.add(key);
        logWarn(`${this.id}: ${message}`);
    }

    /**
    * @param {string} key
    */
    clearWarning(key) {
        this._warnings.delete(key);
    }

    /**
    * @param {Error} error
    * @returns {boolean}
    */
    isCancelled(error) {
        return !!error?.matches?.(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED);
    }
}
