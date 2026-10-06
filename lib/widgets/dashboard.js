import GObject from 'gi://GObject';
import St from 'gi://St';

import {
    formatBytes,
    formatBytesPerSecond,
    formatGigahertz,
    formatMegahertz,
    formatPercent,
    formatRefreshHz,
    formatTemperature,
    formatWatts,
} from './formatters.js';
import {Meter} from './meter.js';
import {Sparkline} from './sparkline.js';

/**
 * Click-to-open dashboard. Widgets are created once and updated in place.
 */
export const Dashboard = GObject.registerClass(
    class Dashboard extends St.BoxLayout {
        _init() {
            super._init({
                vertical: true,
                style_class: 'stats-dashboard',
                x_expand: true,
            });

            this.add_child(new St.Label({
                text: 'Stats',
                style_class: 'stats-popup-header',
            }));

            this._cpuLabel = new St.Label({style_class: 'stats-detail'});
            this._cpuSpark = new Sparkline();
            this._coreBox = new St.BoxLayout({vertical: true, style_class: 'stats-stack'});
            this._coreRows = [];
            const cpu = this._addSection('CPU');
            cpu.add_child(this._cpuLabel);
            cpu.add_child(this._cpuSpark);
            cpu.add_child(this._coreBox);

            this._memLabel = new St.Label({style_class: 'stats-detail'});
            this._memSpark = new Sparkline();
            const memory = this._addSection('Memory');
            memory.add_child(this._memLabel);
            memory.add_child(this._memSpark);

            this._mountBox = new St.BoxLayout({vertical: true, style_class: 'stats-stack'});
            this._mountRows = [];
            this._diskLabel = new St.Label({style_class: 'stats-detail'});
            this._diskSpark = new Sparkline();
            const storage = this._addSection('Storage');
            storage.add_child(this._mountBox);
            storage.add_child(this._diskLabel);
            storage.add_child(this._diskSpark);

            this._gpuBox = new St.BoxLayout({vertical: true, style_class: 'stats-stack'});
            this._gpuRows = [];
            this._gpuSpark = new Sparkline();
            const gpu = this._addSection('GPU');
            gpu.add_child(this._gpuBox);
            gpu.add_child(this._gpuSpark);

            this._tempBox = new St.BoxLayout({vertical: true, style_class: 'stats-stack'});
            this._tempRows = [];
            this._tempSpark = new Sparkline();
            const temps = this._addSection('Temperatures');
            temps.add_child(this._tempBox);
            temps.add_child(this._tempSpark);

            this._displayBox = new St.BoxLayout({vertical: true, style_class: 'stats-stack'});
            this._displayRows = [];
            const displays = this._addSection('Displays');
            displays.add_child(this._displayBox);
            displays.add_child(new St.Label({
                text: 'Refresh rate is the nominal mode. Variable-refresh displays do not report the instantaneous rate.',
                style_class: 'stats-muted',
            }));
        }

        /**
         * @param {object} snapshot
         * @param {Gio.Settings} settings
         */
        update(snapshot, settings) {
            const unit = settings.get_string('temperature-unit');
            const sparks = settings.get_boolean('show-sparklines');
            const history = snapshot.history ?? {};
            this._updateCpu(snapshot.cpu, history.cpu, sparks, settings.get_boolean('show-per-core'));
            this._updateMemory(snapshot.memory, history.memory, sparks);
            this._updateStorage(snapshot.storage, history, sparks);
            this._updateGpu(snapshot.gpu, history.gpu, sparks, unit);
            this._updateTemperatures(snapshot.temperature, history.temperature, sparks, unit);
            this._updateDisplays(snapshot.display);
        }

        /**
         * @param {string} title
         * @returns {St.BoxLayout}
         */
        _addSection(title) {
            this.add_child(new St.Label({
                text: title,
                style_class: 'stats-section-title',
            }));
            const body = new St.BoxLayout({
                vertical: true,
                style_class: 'stats-section',
                x_expand: true,
            });
            this.add_child(body);
            return body;
        }

        /**
         * @param {object|null} cpu
         * @param {(number|null)[]} history
         * @param {boolean} sparks
         * @param {boolean} perCore
         */
        _updateCpu(cpu, history, sparks, perCore) {
            const usage = formatPercent(cpu?.usage);
            let text = usage;
            if (cpu?.frequency) {
                text += `   avg ${formatGigahertz(cpu.frequency.averageMhz)}   max ${formatGigahertz(cpu.frequency.maxMhz)}`;
            }
            setText(this._cpuLabel, text);
            this._cpuSpark.visible = sparks;
            if (sparks)
                this._cpuSpark.setSeries([history ?? []], 100);
            this._coreBox.visible = perCore;
            if (!perCore)
                return;
            syncRows(this._coreRows, this._coreBox, cpu?.cores ?? [], () => {
                const row = labeledMeter();
                return row;
            }, (row, core) => {
                setText(row.label, `${core.id}  ${formatPercent(core.usage)}`);
                row.meter.setPercent(core.usage);
            });
        }

        /**
         * @param {object|null} memory
         * @param {(number|null)[]} history
         * @param {boolean} sparks
         */
        _updateMemory(memory, history, sparks) {
            if (!memory) {
                setText(this._memLabel, 'n/a');
            } else {
                const swap = memory.swapTotal > 0
                    ? `${formatBytes(memory.swapUsed)} / ${formatBytes(memory.swapTotal)}`
                    : 'none';
                setText(
                    this._memLabel,
                    `${formatBytes(memory.used)} / ${formatBytes(memory.total)} (${formatPercent(memory.percent)})\nCached ${formatBytes(memory.cached)}    Swap ${swap}`);
            }
            this._memSpark.visible = sparks;
            if (sparks)
                this._memSpark.setSeries([history ?? []], 100);
        }

        /**
         * @param {object|null} storage
         * @param {object} history
         * @param {boolean} sparks
         */
        _updateStorage(storage, history, sparks) {
            const mounts = storage?.mounts ?? [];
            syncRows(this._mountRows, this._mountBox, mounts, () => labeledMeter(), (row, mount) => {
                setText(
                    row.label,
                    `${mount.path}  ${formatBytes(mount.used)} / ${formatBytes(mount.size)} (${formatPercent(mount.percent)})`);
                row.meter.setPercent(mount.percent);
            });
            if (mounts.length === 0)
                setText(this._diskLabel, 'n/a');
            else {
                setText(
                    this._diskLabel,
                    `Read ${formatBytesPerSecond(storage.readBps)}    Write ${formatBytesPerSecond(storage.writeBps)}`);
            }
            this._diskSpark.visible = sparks;
            if (sparks)
                this._diskSpark.setSeries([history.diskRead ?? [], history.diskWrite ?? []], null);
        }

        /**
         * @param {object|null} gpu
         * @param {(number|null)[]} history
         * @param {boolean} sparks
         * @param {string} unit
         */
        _updateGpu(gpu, history, sparks, unit) {
            const cards = gpu?.cards ?? [];
            const gpuItems = cards.length > 0 ? cards : [null];
            syncRows(this._gpuRows, this._gpuBox, gpuItems, () => {
                return {box: new St.Label({style_class: 'stats-detail'})};
            }, (row, card) => {
                setText(row.box, card ? gpuText(card, unit) : 'n/a');
            });
            this._gpuSpark.visible = sparks;
            if (sparks)
                this._gpuSpark.setSeries([history ?? []], 100);
        }

        /**
         * @param {object|null} temperature
         * @param {(number|null)[]} history
         * @param {boolean} sparks
         * @param {string} unit
         */
        _updateTemperatures(temperature, history, sparks, unit) {
            const sensors = temperature?.sensors ?? [];
            const tempItems = sensors.length > 0 ? sensors : [null];
            syncRows(this._tempRows, this._tempBox, tempItems, () => {
                return {box: new St.Label({style_class: 'stats-detail'})};
            }, (row, sensor) => {
                if (!sensor) {
                    setText(row.box, 'n/a');
                    return;
                }
                const value = sensor.state === 'off'
                    ? 'idle/off'
                    : formatTemperature(sensor.celsius, unit);
                setText(row.box, `${sensor.chip} ${sensor.label}  ${value}`);
            });
            this._tempSpark.visible = sparks;
            if (sparks)
                this._tempSpark.setSeries([history ?? []], null);
        }

        /**
         * @param {object|null} display
         */
        _updateDisplays(display) {
            const monitors = display?.monitors ?? [];
            const displayItems = monitors.length > 0 ? monitors : [null];
            syncRows(this._displayRows, this._displayBox, displayItems, () => {
                return {box: new St.Label({style_class: 'stats-detail'})};
            }, (row, monitor) => {
                if (!monitor) {
                    setText(row.box, 'n/a');
                    return;
                }
                const resolution = monitor.width && monitor.height
                    ? `${monitor.width}×${monitor.height}`
                    : 'n/a';
                const vrr = monitor.vrr ? '  VRR' : '';
                setText(
                    row.box,
                    `${monitor.connector}  ${resolution}  ${formatRefreshHz(monitor.refresh)}${vrr}`);
            });
        }
    });

/**
 * @param {St.Label} label
 * @param {string} text
 */
function setText(label, text) {
    if (label.text !== text)
        label.text = text;
}

/**
 * @returns {{box: St.BoxLayout, label: St.Label, meter: Meter}}
 */
function labeledMeter() {
    const box = new St.BoxLayout({vertical: true, style_class: 'stats-meter-row'});
    const label = new St.Label({style_class: 'stats-detail'});
    const meter = new Meter();
    box.add_child(label);
    box.add_child(meter);
    return {box, label, meter};
}

/**
 * @param {{box: St.Widget}[]} rows
 * @param {St.BoxLayout} container
 * @param {object[]} items
 * @param {() => {box: St.Widget}} build
 * @param {(row: object, item: object) => void} update
 */
function syncRows(rows, container, items, build, update) {
    while (rows.length > items.length) {
        const row = rows.pop();
        container.remove_child(row.box);
        row.box.destroy();
    }
    for (let index = 0; index < items.length; index++) {
        if (!rows[index]) {
            rows[index] = build();
            container.add_child(rows[index].box);
        }
        update(rows[index], items[index]);
    }
}

/**
 * @param {object} card
 * @param {string} unit
 * @returns {string}
 */
function gpuText(card, unit) {
    const name = `${card.id}  ${card.vendor}`;
    if (card.state === 'off')
        return `${name}  idle/off`;
    const util = card.util == null ? 'n/a' : formatPercent(card.util);
    const freq = card.frequencyMhz == null ? '' : `    ${formatMegahertz(card.frequencyMhz)}`;
    const vram = card.vramTotal
        ? `${formatBytes(card.vramUsed)} / ${formatBytes(card.vramTotal)}`
        : 'n/a';
    const temp = formatTemperature(card.tempC, unit);
    const power = formatWatts(card.powerW);
    return `${name}\nUtil ${util}${freq}    VRAM ${vram}\nTemp ${temp}    Power ${power}`;
}
