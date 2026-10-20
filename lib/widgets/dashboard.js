import Clutter from 'gi://Clutter';
import GObject from 'gi://GObject';
import Pango from 'gi://Pango';
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
 * Every section heading is a button that folds its body away; the choice is
 * kept in GSettings and a folded section is not refreshed while it is closed.
 */
export const Dashboard = GObject.registerClass(
    class Dashboard extends St.BoxLayout {
        /**
         * @param {Gio.Settings} settings
         */
        _init(settings) {
            super._init({
                vertical: true,
                style_class: 'stats-dashboard',
                x_expand: true,
            });
            this._settings = settings;
            this._wrapping = [];
            this._sections = new Map();
            this._cap = 0;

            this.add_child(new St.Label({
                text: 'Stats',
                style_class: 'stats-popup-header',
            }));

            this._cpuLabel = this._detailLabel();
            this._cpuSpark = new Sparkline();
            this._coreBox = new St.BoxLayout({vertical: true, style_class: 'stats-stack'});
            this._coreRows = [];
            const cpu = this._addSection('CPU', 'cpu');
            cpu.add_child(this._cpuLabel);
            cpu.add_child(this._cpuSpark);
            cpu.add_child(this._coreBox);

            this._memLabel = this._detailLabel();
            this._memSpark = new Sparkline();
            const memory = this._addSection('Memory', 'memory');
            memory.add_child(this._memLabel);
            memory.add_child(this._memSpark);

            this._mountBox = new St.BoxLayout({vertical: true, style_class: 'stats-stack'});
            this._mountRows = [];
            this._diskLabel = this._detailLabel();
            this._diskSpark = new Sparkline();
            const storage = this._addSection('Storage', 'storage');
            storage.add_child(this._diskLabel);
            storage.add_child(this._mountBox);
            storage.add_child(this._diskSpark);

            this._netLabel = this._detailLabel();
            this._netSpark = new Sparkline();
            const network = this._addSection('Network', 'network');
            network.add_child(this._netLabel);
            network.add_child(this._netSpark);

            this._gpuBox = new St.BoxLayout({vertical: true, style_class: 'stats-stack'});
            this._gpuRows = [];
            this._gpuSpark = new Sparkline();
            const gpu = this._addSection('GPU', 'gpu');
            gpu.add_child(this._gpuBox);
            gpu.add_child(this._gpuSpark);

            this._tempHeadline = this._detailLabel();
            this._tempBox = new St.BoxLayout({vertical: true, style_class: 'stats-stack'});
            this._tempRows = [];
            this._tempSpark = new Sparkline();
            const temps = this._addSection('Temperatures', 'temperature');
            temps.add_child(this._tempHeadline);
            temps.add_child(this._tempBox);
            temps.add_child(this._tempSpark);

            this._displayHeadline = this._detailLabel();
            this._displayBox = new St.BoxLayout({vertical: true, style_class: 'stats-stack'});
            this._displayRows = [];
            const displays = this._addSection('Displays', 'display');
            displays.add_child(this._displayHeadline);
            displays.add_child(this._displayBox);
            displays.add_child(this._detailLabelNote(
                'Refresh rate is the nominal mode. Variable-refresh displays do not report the instantaneous rate.'));

            this._applyCollapsed();
        }

        /**
         * @param {string} key
         * @returns {boolean}
         */
        _isOpen(key) {
            return this._sections.get(key)?.body.visible ?? true;
        }

        /**
         * Flip the section first and persist after. Writing the key makes the
         * indicator re-render, and by then the body it should fill is already
         * visible, so a section opens with live figures instead of stale ones.
         *
         * @param {string} key
         */
        _toggleSection(key) {
            const collapsed = new Set(this._settings?.get_strv('collapsed-sections') ?? []);
            if (collapsed.has(key))
                collapsed.delete(key);
            else
                collapsed.add(key);
            const next = [...collapsed];
            this._applyCollapsed(next);
            this._settings?.set_strv('collapsed-sections', next);
        }

        /**
         * @param {string[]} [keys]
         */
        _applyCollapsed(keys) {
            const collapsed = new Set(keys ?? this._settings?.get_strv('collapsed-sections') ?? []);
            for (const [key, section] of this._sections) {
                const open = !collapsed.has(key);
                section.body.visible = open;
                section.arrow.icon_name = open ? 'pan-down-symbolic' : 'pan-end-symbolic';
            }
        }

        /**
         * @param {object} snapshot
         * @param {Gio.Settings} settings
         */
        update(snapshot, settings) {
            const unit = settings.get_string('temperature-unit');
            const sparks = settings.get_boolean('show-sparklines');
            const history = snapshot.history ?? {};
            // A folded section draws nothing, so there is nothing to refresh.
            if (this._isOpen('cpu'))
                this._updateCpu(snapshot.cpu, history.cpu, sparks, settings.get_boolean('show-per-core'));
            if (this._isOpen('memory'))
                this._updateMemory(snapshot.memory, history.memory, sparks);
            if (this._isOpen('storage'))
                this._updateStorage(snapshot.storage, history, sparks);
            if (this._isOpen('network'))
                this._updateNetwork(snapshot.network, history, sparks);
            if (this._isOpen('gpu'))
                this._updateGpu(snapshot.gpu, history.gpu, sparks, unit);
            if (this._isOpen('temperature'))
                this._updateTemperatures(snapshot.temperature, history.temperature, sparks, unit);
            if (this._isOpen('display'))
                this._updateDisplays(snapshot.display);
            if (this._cap)
                this.setWidthCap(this._cap);
        }

        /**
         * Cap the menu to the monitor. Detail lines wrap inside that width.
         *
         * @param {number} width
         */
        setWidthCap(width) {
            this._cap = width;
            const textWidth = Math.max(120, width - 24);
            this.style = `max-width: ${width}px;`;
            for (const label of this._wrapping) {
                label.style = `max-width: ${textWidth}px;`;
                label.clutter_text.width = textWidth;
            }
        }

        /**
         * @param {string} [styleClass]
         * @returns {St.Label}
         */
        _detailLabel(styleClass = 'stats-detail') {
            const label = new St.Label({
                style_class: styleClass,
                x_expand: true,
            });
            const text = label.clutter_text;
            text.line_wrap = true;
            text.line_wrap_mode = Pango.WrapMode.WORD_CHAR;
            text.ellipsize = Pango.EllipsizeMode.NONE;
            this._wrapping.push(label);
            label.connect('destroy', () => {
                const index = this._wrapping.indexOf(label);
                if (index >= 0)
                    this._wrapping.splice(index, 1);
            });
            return label;
        }

        /**
         * @param {string} note
         * @returns {St.Label}
         */
        _detailLabelNote(note) {
            const label = this._detailLabel('stats-muted');
            label.text = note;
            return label;
        }

        /**
         * @returns {{box: St.BoxLayout, label: St.Label, meter: Meter}}
         */
        _labeledMeter() {
            const box = new St.BoxLayout({
                vertical: true,
                style_class: 'stats-meter-row',
                x_expand: true,
            });
            const label = this._detailLabel();
            const meter = new Meter();
            box.add_child(label);
            box.add_child(meter);
            return {box, label, meter};
        }

        /**
         * @param {string} title
         * @param {string} key
         * @returns {St.BoxLayout}
         */
        _addSection(title, key) {
            const arrow = new St.Icon({
                icon_name: 'pan-down-symbolic',
                style_class: 'stats-section-arrow',
                y_align: Clutter.ActorAlign.CENTER,
            });
            const heading = new St.BoxLayout({
                style_class: 'stats-section-heading',
                x_expand: true,
            });
            heading.add_child(arrow);
            heading.add_child(new St.Label({
                text: title,
                style_class: 'stats-section-title',
                y_align: Clutter.ActorAlign.CENTER,
                x_expand: true,
            }));
            const header = new St.Button({
                style_class: 'stats-section-header',
                can_focus: true,
                x_expand: true,
                child: heading,
            });
            header.connect('clicked', () => this._toggleSection(key));
            const body = new St.BoxLayout({
                vertical: true,
                style_class: 'stats-section',
                x_expand: true,
            });
            this.add_child(header);
            this.add_child(body);
            this._sections.set(key, {arrow, body});
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
            const speed = formatGigahertz(cpu?.frequency?.averageMhz);
            let text = `CPU ${usage} ${speed}`;
            if (cpu?.frequency)
                text += `  max ${formatGigahertz(cpu.frequency.maxMhz)}`;
            setText(this._cpuLabel, text);
            this._cpuSpark.visible = sparks;
            if (sparks)
                this._cpuSpark.setSeries([history ?? []], 100);
            this._coreBox.visible = perCore;
            if (!perCore)
                return;
            syncRows(this._coreRows, this._coreBox, cpu?.cores ?? [], () => {
                return this._labeledMeter();
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
                    `RAM ${formatPercent(memory.percent)} ${formatBytes(memory.used)}\n${formatBytes(memory.used)} / ${formatBytes(memory.total)}    Cached ${formatBytes(memory.cached)}    Swap ${swap}`);
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
            syncRows(this._mountRows, this._mountBox, mounts, () => this._labeledMeter(), (row, mount) => {
                setText(
                    row.label,
                    `${mount.path}  ${formatBytes(mount.used)} / ${formatBytes(mount.size)} (${formatPercent(mount.percent)})`);
                row.meter.setPercent(mount.percent);
            });
            if (mounts.length === 0) {
                setText(this._diskLabel, 'Disk n/a');
            } else {
                const percent = formatPercent(mounts[0].percent);
                const speed = formatBytesPerSecond(combinedBps(storage));
                setText(
                    this._diskLabel,
                    `Disk ${percent} ${speed}\nRead ${formatBytesPerSecond(storage.readBps)}    Write ${formatBytesPerSecond(storage.writeBps)}`);
            }
            this._diskSpark.visible = sparks;
            if (sparks)
                this._diskSpark.setSeries([history.diskRead ?? [], history.diskWrite ?? []], null);
        }

        /**
         * @param {object|null} network
         * @param {object} history
         * @param {boolean} sparks
         */
        _updateNetwork(network, history, sparks) {
            const down = formatBytesPerSecond(network?.downBps);
            const up = formatBytesPerSecond(network?.upBps);
            setText(this._netLabel, `Down ${down}    Up ${up}`);
            this._netSpark.visible = sparks;
            if (sparks)
                this._netSpark.setSeries([history.netDown ?? [], history.netUp ?? []], null);
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
                return {box: this._detailLabel()};
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
            const headline = temperature?.headline;
            if (!headline) {
                setText(this._tempHeadline, 'n/a');
            } else {
                const value = headline.state === 'off'
                    ? 'idle/off'
                    : formatTemperature(headline.celsius, unit);
                setText(this._tempHeadline, `${headline.label} ${value}`);
            }
            const sensors = temperature?.sensors ?? [];
            const tempItems = sensors.length > 0 ? sensors : [null];
            syncRows(this._tempRows, this._tempBox, tempItems, () => {
                return {box: this._detailLabel()};
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
            const primary = (display?.monitors ?? []).find(monitor => monitor.connector === display?.primaryConnector)
                ?? display?.monitors?.[0]
                ?? null;
            if (!primary)
                setText(this._displayHeadline, formatRefreshHz(display?.primaryHz));
            else
                setText(this._displayHeadline, `${primary.connector} ${formatRefreshHz(primary.refresh)}`);
            const monitors = display?.monitors ?? [];
            const displayItems = monitors.length > 0 ? monitors : [null];
            syncRows(this._displayRows, this._displayBox, displayItems, () => {
                return {box: this._detailLabel()};
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
 * @param {object|null|undefined} storage
 * @returns {number|null}
 */
function combinedBps(storage) {
    const read = storage?.readBps;
    const write = storage?.writeBps;
    if (read == null && write == null)
        return null;
    return (read ?? 0) + (write ?? 0);
}

/**
 * @param {object} card
 * @param {string} unit
 * @returns {string}
 */
function gpuText(card, unit) {
    const name = `${card.id}  ${card.vendor}`;
    if (card.state === 'off')
        return `GPU idle/off\n${name}`;
    const util = card.util == null ? 'n/a' : formatPercent(card.util);
    const freq = card.frequencyMhz == null ? 'n/a' : formatMegahertz(card.frequencyMhz);
    const vram = card.vramTotal
        ? `${formatBytes(card.vramUsed)} / ${formatBytes(card.vramTotal)}`
        : 'n/a';
    const temp = formatTemperature(card.tempC, unit);
    const power = formatWatts(card.powerW);
    return `GPU ${util} ${freq}\n${name}\nVRAM ${vram}\nTemp ${temp}    Power ${power}`;
}
