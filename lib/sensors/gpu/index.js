import GLib from 'gi://GLib';

import {ensureIo, listChildren, readTextOrNull} from '../../io.js';
import {LineProcess} from '../../line_process.js';
import {Sensor} from '../sensor.js';
import {isSuspended, parseVendor, selectCard} from './detect.js';
import {consumeJsonObjects, parseIntelGpuTop} from './intel.js';
import {parseNvidiaCsvLine} from './nvidia.js';

/**
 * List DRM cards and their vendors. Safe to call from the preferences process.
 *
 * @param {Gio.Cancellable|null} cancellable
 * @returns {Promise<{id: string, vendor: string}[]>}
 */
export async function listGpuCards(cancellable) {
    ensureIo();
    let names = [];
    try {
        names = await listChildren('/sys/class/drm', cancellable);
    } catch {
        return [];
    }
    const cards = [];
    await Promise.all(names.filter(name => /^card\d+$/.test(name)).map(async name => {
        const vendorText = await readTextOrNull(
            `/sys/class/drm/${name}/device/vendor`,
            cancellable);
        const vendor = vendorText ? parseVendor(vendorText) : null;
        if (vendor)
            cards.push({id: name, vendor});
    }));
    cards.sort((a, b) => a.id.localeCompare(b.id, undefined, {numeric: true}));
    return cards;
}

export class GpuSensor extends Sensor {
    constructor() {
        super('gpu', 'GPU');
        this._nvidia = null;
        this._nvidiaCount = 0;
        this._nvidiaBatch = [];
        this._nvidiaSamples = [];
        this._intel = null;
        this._intelBuffer = '';
        this._intelSample = null;
        this._latest = null;
    }

    /**
     * @param {boolean} paused
     */
    setPaused(paused) {
        super.setPaused(paused);
        if (paused) {
            this._nvidia?.stop();
            this._intel?.stop();
        }
    }

    destroy() {
        this._nvidia?.stop();
        this._intel?.stop();
        this._nvidia = null;
        this._intel = null;
        super.destroy();
    }

    /**
     * @param {object} context
     */
    async sample(context) {
        if (this._paused)
            return this._latest;
        try {
            const detected = await listGpuCards(context.cancellable);
            const cards = [];
            let nvidiaIndex = 0;
            for (const card of detected) {
                const sample = await this._sampleCard(card, context.cancellable, nvidiaIndex);
                if (card.vendor === 'nvidia' && sample.state !== 'off')
                    nvidiaIndex += 1;
                cards.push(sample);
            }
            this._applyIntelUtil(cards);
            this._syncStreams(cards);
            if (this._intel?.gaveUp)
                this.warnOnce('intel-exit', 'intel_gpu_top failed; showing frequency only');
            const preferred = context.settings?.get_string('gpu-id') ?? '';
            const selected = selectCard(cards, preferred);
            this._latest = {cards, selected};
            this.clearWarning('sample');
            return this._latest;
        } catch (error) {
            if (this.isCancelled(error))
                throw error;
            this.warnOnce('sample', `GPU scan failed (${error.message})`);
            return null;
        }
    }

    /**
     * @param {{id: string, vendor: string}} card
     * @param {Gio.Cancellable} cancellable
     * @param {number} nvidiaIndex
     */
    async _sampleCard(card, cancellable, nvidiaIndex) {
        const base = `/sys/class/drm/${card.id}`;
        const status = await readTextOrNull(`${base}/device/power/runtime_status`, cancellable);
        if (isSuspended(status))
            return emptyCard(card, 'off');
        if (card.vendor === 'nvidia')
            return this._sampleNvidia(card, nvidiaIndex);
        if (card.vendor === 'amd')
            return this._sampleAmd(card, base, cancellable);
        return this._sampleIntel(card, base, cancellable);
    }

    /**
     * Do not start nvidia-smi when any NVIDIA card is suspended: querying it
     * wakes a discrete GPU that runtime PM has put to sleep.
     *
     * @param {object[]} cards
     */
    _syncStreams(cards) {
        const nvidia = cards.filter(card => card.vendor === 'nvidia');
        const nvidiaActive = nvidia.filter(card => card.state !== 'off');
        this._nvidiaCount = nvidiaActive.length;
        if (nvidia.length > 0 && nvidiaActive.length === nvidia.length)
            this._ensureNvidia();
        else
            this._nvidia?.stop();

        const intelActive = cards.some(card => card.vendor === 'intel' && card.state !== 'off');
        if (intelActive && GLib.find_program_in_path('intel_gpu_top'))
            this._ensureIntel();
        else
            this._intel?.stop();
    }

    _ensureNvidia() {
        if (this._nvidia?.gaveUp)
            return;
        const binary = GLib.find_program_in_path('nvidia-smi');
        if (!binary) {
            this.warnOnce('nvidia-smi', 'nvidia-smi is not installed');
            return;
        }
        if (!this._nvidia) {
            this._nvidia = new LineProcess([
                binary,
                '--query-gpu=utilization.gpu,temperature.gpu,memory.used,memory.total,power.draw',
                '--format=csv,noheader,nounits',
                '-l',
                '1',
            ], line => this._onNvidiaLine(line));
        }
        this._nvidia.start();
    }

    /**
     * @param {string} line
     */
    _onNvidiaLine(line) {
        const parsed = parseNvidiaCsvLine(line);
        if (!parsed)
            return;
        this._nvidiaBatch.push(parsed);
        const count = Math.max(1, this._nvidiaCount);
        if (this._nvidiaBatch.length >= count) {
            this._nvidiaSamples = this._nvidiaBatch.slice(0, count);
            this._nvidiaBatch = [];
        }
    }

    /**
     * @param {{id: string, vendor: string}} card
     * @param {number} index
     */
    _sampleNvidia(card, index) {
        if (!GLib.find_program_in_path('nvidia-smi')) {
            this.warnOnce('nvidia-smi', 'nvidia-smi is not installed');
            return emptyCard(card, 'active');
        }
        const sample = this._nvidiaSamples[index] ?? null;
        if (!sample)
            return emptyCard(card, 'active');
        return {
            ...emptyCard(card, 'active'),
            util: sample.util,
            tempC: sample.tempC,
            vramUsed: sample.vramUsed,
            vramTotal: sample.vramTotal,
            powerW: sample.powerW,
        };
    }

    /**
     * @param {{id: string, vendor: string}} card
     * @param {string} base
     * @param {Gio.Cancellable} cancellable
     */
    async _sampleAmd(card, base, cancellable) {
        const device = `${base}/device`;
        const busy = Number((await readTextOrNull(`${device}/gpu_busy_percent`, cancellable))?.trim());
        const used = Number((await readTextOrNull(`${device}/mem_info_vram_used`, cancellable))?.trim());
        const total = Number((await readTextOrNull(`${device}/mem_info_vram_total`, cancellable))?.trim());
        const temp = await this._amdTemperature(device, cancellable);
        return {
            ...emptyCard(card, 'active'),
            util: Number.isFinite(busy) ? Math.min(100, Math.max(0, busy)) : null,
            vramUsed: Number.isFinite(used) ? used : null,
            vramTotal: Number.isFinite(total) ? total : null,
            tempC: temp,
        };
    }

    /**
     * @param {string} device
     * @param {Gio.Cancellable} cancellable
     */
    async _amdTemperature(device, cancellable) {
        try {
            const hwmon = await listChildren(`${device}/hwmon`, cancellable);
            const dir = hwmon.find(name => name.startsWith('hwmon'));
            if (!dir)
                return null;
            const milli = Number((await readTextOrNull(
                `${device}/hwmon/${dir}/temp1_input`,
                cancellable))?.trim());
            return Number.isFinite(milli) ? milli / 1000 : null;
        } catch (error) {
            if (this.isCancelled(error))
                throw error;
            return null;
        }
    }

    /**
     * @param {{id: string, vendor: string}} card
     * @param {string} base
     * @param {Gio.Cancellable} cancellable
     */
    async _sampleIntel(card, base, cancellable) {
        const frequency = await readIntelFrequency(`${base}/device`, cancellable);
        if (!GLib.find_program_in_path('intel_gpu_top'))
            this.warnOnce('intel_gpu_top', 'intel_gpu_top is not installed; utilisation is n/a');
        return {
            ...emptyCard(card, 'active'),
            frequencyMhz: frequency,
        };
    }

    /**
     * intel_gpu_top reports one device. Apply it to the first active Intel card.
     *
     * @param {object[]} cards
     */
    _applyIntelUtil(cards) {
        if (!this._intelSample)
            return;
        const primary = cards.find(card => card.vendor === 'intel' && card.state !== 'off');
        if (!primary)
            return;
        primary.util = this._intelSample.util;
        if (this._intelSample.frequencyMhz != null)
            primary.frequencyMhz = this._intelSample.frequencyMhz;
    }

    _ensureIntel() {
        if (this._intel?.gaveUp)
            return;
        const binary = GLib.find_program_in_path('intel_gpu_top');
        if (!binary)
            return;
        if (!this._intel) {
            this._intel = new LineProcess(
                [binary, '-J', '-s', '1000'],
                line => this._onIntelLine(line),
                {maxFailures: 3});
        }
        this._intel.start();
    }

    /**
     * @param {string} line
     */
    _onIntelLine(line) {
        this._intelBuffer += `${line}\n`;
        if (this._intelBuffer.length > 200000)
            this._intelBuffer = this._intelBuffer.slice(-100000);
        const consumed = consumeJsonObjects(this._intelBuffer);
        this._intelBuffer = consumed.rest;
        const last = consumed.objects.at(-1);
        if (last)
            this._intelSample = parseIntelGpuTop(last);
    }
}

/**
 * Newer i915/xe sysfs uses gt/gt0/rps_act_freq_mhz. Older layouts only have
 * gt_act_freq_mhz beside the DRM device. Prefer the first file that exists.
 *
 * @param {string} device
 * @param {Gio.Cancellable|null} cancellable
 * @returns {Promise<number|null>}
 */
export async function readIntelFrequency(device, cancellable) {
    const paths = [
        `${device}/gt/gt0/rps_act_freq_mhz`,
        `${device}/gt_act_freq_mhz`,
    ];
    for (const path of paths) {
        const text = await readTextOrNull(path, cancellable);
        if (text == null)
            continue;
        const value = Number(text.trim());
        if (Number.isFinite(value))
            return value;
    }
    return null;
}

/**
 * @param {{id: string, vendor: string}} card
 * @param {'active'|'off'} state
 */
function emptyCard(card, state) {
    return {
        id: card.id,
        vendor: card.vendor,
        state,
        util: null,
        vramUsed: null,
        vramTotal: null,
        tempC: null,
        powerW: null,
        frequencyMhz: null,
    };
}
