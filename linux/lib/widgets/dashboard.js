import Clutter from 'gi://Clutter';
import GObject from 'gi://GObject';
import St from 'gi://St';

import {status, tileModels} from '../overview.js';
import {
    label,
    makeButton,
    makeCard,
    makeControlRow,
    makeRow,
    makeSegmented,
    makeSwitchRow,
    makeTile,
    setLevel,
    setRow,
    setText,
    wrapLabel,
} from './cards.js';
import {
    formatBytes,
    formatBytesPerSecond,
    formatGigahertz,
    formatMegahertz,
    formatPercent,
    formatRefreshHz,
    formatTemperature,
    formatWatts,
    thresholdLevel,
} from './formatters.js';
import {Meter} from './meter.js';
import {Sparkline} from './sparkline.js';

const PROGRAM_ROWS = 6;

/**
 * The dashboard, laid out like the Windows app: a header with a status chip,
 * six live tiles, then folding cards of label/value rows (including the
 * programs using memory, each with Close), and quick controls at the end.
 *
 * Widgets are created once and updated in place. A folded card is not
 * refreshed at all, and its state is kept in `collapsed-sections`.
 */
export const Dashboard = GObject.registerClass(
    class Dashboard extends St.BoxLayout {
        /**
         * @param {Gio.Settings} settings
         * @param {{
         *   openPreferences: () => void,
         *   openSystemMonitor: (() => void)|null,
         *   closeProgram: (group: object, force: boolean) => void,
         *   scrollTo: (actor: Clutter.Actor) => void,
         * }} actions
         */
        _init(settings, actions) {
            super._init({vertical: true, style_class: 'stats-dashboard', x_expand: true});
            this._settings = settings;
            this._actions = actions;
            this._cards = new Map();
            this._cap = 0;
            this._groups = [];
            this._forceNames = new Set();
            this._busiest = null;

            this._buildHeader();
            this._notice = wrapLabel('stats-notice', {visible: false});
            this.add_child(this._notice);
            this._buildTiles();

            this._buildCpu();
            this._buildMemory();
            this._buildPrograms();
            this._buildStorage();
            this._buildNetwork();
            this._buildGpu();
            this._buildTemperatures();
            this._buildDisplays();
            this._buildControls();
            this._applyCollapsed();
        }

        // ----- structure ---------------------------------------------------

        _buildHeader() {
            const header = new St.BoxLayout({style_class: 'stats-header', x_expand: true});
            header.add_child(label('stats-title', {text: 'Stats'}));
            this._chip = new St.BoxLayout({style_class: 'stats-chip', y_align: Clutter.ActorAlign.CENTER});
            this._chip.add_child(new St.Widget({style_class: 'stats-chip-dot', y_align: Clutter.ActorAlign.CENTER}));
            this._chipText = label('stats-chip-text', {text: 'All good'});
            this._chip.add_child(this._chipText);
            header.add_child(this._chip);
            header.add_child(new St.Widget({x_expand: true}));
            const gear = new St.Button({
                style_class: 'stats-icon-button',
                can_focus: true,
                child: new St.Icon({icon_name: 'emblem-system-symbolic', style_class: 'stats-icon-button-icon'}),
                accessible_name: 'Preferences',
            });
            gear.connect('clicked', () => this._actions.openPreferences());
            header.add_child(gear);
            this.add_child(header);
        }

        _buildTiles() {
            this._tiles = [];
            const grid = new St.BoxLayout({vertical: true, style_class: 'stats-tiles', x_expand: true});
            for (let line = 0; line < 2; line++) {
                const row = new St.BoxLayout({style_class: 'stats-tile-row', x_expand: true});
                for (let column = 0; column < 3; column++) {
                    const index = line * 3 + column;
                    const tile = makeTile(() => this._openFromTile(index));
                    this._tiles.push(tile);
                    row.add_child(tile.button);
                }
                grid.add_child(row);
            }
            this.add_child(grid);
        }

        /**
         * @param {string} key
         * @param {string} title
         * @returns {St.BoxLayout} The card body.
         */
        _addCard(key, title) {
            const card = makeCard(title, () => this._toggleSection(key));
            this._cards.set(key, card);
            this.add_child(card.card);
            return card.body;
        }

        _buildCpu() {
            const body = this._addCard('cpu', 'CPU');
            this._cpuUsage = makeRow();
            this._cpuFreq = makeRow();
            this._cpuSpark = new Sparkline();
            this._coreBox = new St.BoxLayout({vertical: true, style_class: 'stats-stack'});
            this._coreRows = [];
            body.add_child(this._cpuUsage.box);
            body.add_child(this._cpuFreq.box);
            body.add_child(this._cpuSpark);
            body.add_child(this._coreBox);
        }

        _buildMemory() {
            const body = this._addCard('memory', 'Memory');
            this._memUsed = makeRow();
            this._memAvailable = makeRow();
            this._memCached = makeRow();
            this._memSwap = makeRow();
            this._memSpark = new Sparkline();
            for (const row of [this._memUsed, this._memAvailable, this._memCached, this._memSwap])
                body.add_child(row.box);
            body.add_child(this._memSpark);
        }

        _buildPrograms() {
            const body = this._addCard('programs', 'Programs');
            this._programBox = new St.BoxLayout({vertical: true, style_class: 'stats-stack'});
            this._programRows = [];
            this._programEmpty = label('stats-muted', {text: 'Reading programs…'});
            body.add_child(this._programEmpty);
            body.add_child(this._programBox);
            const buttons = new St.BoxLayout({style_class: 'stats-button-row'});
            if (this._actions.openSystemMonitor)
                buttons.add_child(makeButton('System Monitor', 'utilities-system-monitor-symbolic', () => this._actions.openSystemMonitor()));
            body.add_child(buttons);
            body.add_child(wrapLabel('stats-muted', {
                text: 'Close asks a program to quit, as when you log out: save your work first. Parts of the desktop are never offered.',
            }));
        }

        _buildStorage() {
            const body = this._addCard('storage', 'Storage');
            this._mountBox = new St.BoxLayout({vertical: true, style_class: 'stats-stack'});
            this._mountRows = [];
            this._diskRead = makeRow();
            this._diskWrite = makeRow();
            this._diskSpark = new Sparkline();
            body.add_child(this._mountBox);
            body.add_child(this._diskRead.box);
            body.add_child(this._diskWrite.box);
            body.add_child(this._diskSpark);
        }

        _buildNetwork() {
            const body = this._addCard('network', 'Network');
            this._netDown = makeRow();
            this._netUp = makeRow();
            this._netSpark = new Sparkline();
            body.add_child(this._netDown.box);
            body.add_child(this._netUp.box);
            body.add_child(this._netSpark);
        }

        _buildGpu() {
            const body = this._addCard('gpu', 'GPU');
            this._gpuBox = new St.BoxLayout({vertical: true, style_class: 'stats-stack'});
            this._gpuRows = [];
            this._gpuSpark = new Sparkline();
            body.add_child(this._gpuBox);
            body.add_child(this._gpuSpark);
        }

        _buildTemperatures() {
            const body = this._addCard('temperature', 'Temperatures');
            this._tempHeadline = makeRow();
            this._tempBox = new St.BoxLayout({vertical: true, style_class: 'stats-stack'});
            this._tempRows = [];
            this._tempSpark = new Sparkline();
            body.add_child(this._tempHeadline.box);
            body.add_child(this._tempBox);
            body.add_child(this._tempSpark);
        }

        _buildDisplays() {
            const body = this._addCard('display', 'Displays');
            this._displayBox = new St.BoxLayout({vertical: true, style_class: 'stats-stack'});
            this._displayRows = [];
            body.add_child(this._displayBox);
            body.add_child(wrapLabel('stats-muted', {
                text: 'Refresh rate is the nominal mode. Variable-refresh displays do not report the instantaneous rate.',
            }));
        }

        _buildControls() {
            const body = this._addCard('controls', 'Quick controls');
            const set = (key, value) => {
                if (this._settings.get_string(key) !== value)
                    this._settings.set_string(key, value);
            };
            this._positionControl = makeSegmented(
                [{label: 'Left', value: 'left'}, {label: 'Right', value: 'right'}],
                value => {
                    if (value === 'left' && this._settings.get_int('position-index') === 0)
                        this._settings.set_int('position-index', 20);
                    set('position', value);
                });
            body.add_child(makeControlRow('Top-bar side', 'Where the readings sit next to the clock', this._positionControl.box));
            this._modeControl = makeSegmented(
                [
                    {label: 'Icons', value: 'icon-text'},
                    {label: 'Text', value: 'text'},
                    {label: 'Compact', value: 'compact'},
                ],
                value => set('display-mode', value));
            body.add_child(makeControlRow('Top-bar style', 'Compact shows one reading until you point at it', this._modeControl.box));
            this._switches = [
                ['show-sparklines', 'Graphs', 'History graphs in tiles and cards'],
                ['show-per-core', 'Per-core bars', 'One bar for each CPU core'],
                ['pressure-notifications', 'Pressure alerts', 'Name the program behind high CPU or memory'],
                ['pause-when-locked', 'Pause when locked', 'Stop reading sensors while the screen is locked'],
                ['frosted-popup', 'Frosted glass', 'Blur what is behind this dashboard'],
            ].map(([key, title, subtitle]) => {
                const row = makeSwitchRow(title, subtitle, on => this._settings.set_boolean(key, on));
                body.add_child(row.button);
                return {key, row};
            });
            const buttons = new St.BoxLayout({style_class: 'stats-button-row'});
            buttons.add_child(makeButton('All settings…', 'emblem-system-symbolic', () => this._actions.openPreferences()));
            if (this._actions.openSystemMonitor)
                buttons.add_child(makeButton('System Monitor', 'utilities-system-monitor-symbolic', () => this._actions.openSystemMonitor()));
            body.add_child(buttons);
        }

        // ----- folding -----------------------------------------------------

        /**
         * @param {string} key
         * @returns {boolean}
         */
        _isOpen(key) {
            return this._cards.get(key)?.body.visible ?? true;
        }

        /**
         * Flip the card first and persist after: writing the key re-renders,
         * and by then the body is visible, so it opens with live figures.
         *
         * @param {string} key
         * @param {boolean} [forceOpen]
         */
        _toggleSection(key, forceOpen = false) {
            const collapsed = new Set(this._settings?.get_strv('collapsed-sections') ?? []);
            if (forceOpen) {
                if (!collapsed.has(key))
                    return;
                collapsed.delete(key);
            } else if (collapsed.has(key)) {
                collapsed.delete(key);
            } else {
                collapsed.add(key);
            }
            const next = [...collapsed];
            this._applyCollapsed(next);
            this._settings?.set_strv('collapsed-sections', next);
        }

        /**
         * @param {string[]} [keys]
         */
        _applyCollapsed(keys) {
            const collapsed = new Set(keys ?? this._settings?.get_strv('collapsed-sections') ?? []);
            for (const [key, card] of this._cards) {
                const open = !collapsed.has(key);
                card.body.visible = open;
                card.arrow.icon_name = open ? 'pan-down-symbolic' : 'pan-end-symbolic';
                if (open)
                    card.card.remove_style_class_name('folded');
                else
                    card.card.add_style_class_name('folded');
            }
        }

        /**
         * A tile opens its card and scrolls to it.
         *
         * @param {number} index
         */
        _openFromTile(index) {
            const key = ['cpu', 'memory', 'storage', 'gpu', 'temperature', 'network'][index];
            this._toggleSection(key, true);
            const card = this._cards.get(key);
            if (card)
                this._actions.scrollTo(card.card);
        }

        // ----- updates -----------------------------------------------------

        /**
         * A short message at the top, for example the result of Close.
         *
         * @param {string|null} text
         * @param {string} [level]
         */
        setNotice(text, level = 'normal') {
            this._notice.visible = !!text;
            if (text)
                setText(this._notice, text);
            setLevel(this._notice, level);
        }

        /**
         * Programs from the sampler, and which of them still run after Close.
         *
         * @param {object[]} groups
         * @param {Set<string>} forceNames
         */
        setPrograms(groups, forceNames) {
            this._groups = groups;
            this._forceNames = forceNames;
            const busiest = groups
                .filter(group => (group.cpu ?? 0) >= 1)
                .sort((a, b) => (b.cpu ?? 0) - (a.cpu ?? 0))[0];
            this._busiest = busiest?.name ?? null;
            if (this._isOpen('programs'))
                this._updatePrograms();
        }

        /**
         * @param {object} snapshot
         * @param {Gio.Settings} settings
         */
        update(snapshot, settings) {
            const unit = settings.get_string('temperature-unit');
            const sparks = settings.get_boolean('show-sparklines');
            const history = snapshot.history ?? {};
            const get = key => settings.get_int(key);

            const chip = status(snapshot, get);
            setText(this._chipText, chip.text);
            setLevel(this._chip, chip.level);

            tileModels(snapshot, get, unit, this._busiest).forEach((model, index) => {
                const tile = this._tiles[index];
                setText(tile.name, model.label);
                setText(tile.value, model.value);
                setText(tile.detail, model.detail);
                setLevel(tile.value, model.level);
                setLevel(tile.button, model.level);
                tile.spark.visible = sparks;
                if (sparks)
                    tile.spark.setSeries(model.series.map(name => history[name] ?? []), model.max);
            });

            this._updateSummaries(snapshot, unit);
            // A folded card draws nothing, so there is nothing to refresh.
            if (this._isOpen('cpu'))
                this._updateCpu(snapshot.cpu, history.cpu, sparks, settings.get_boolean('show-per-core'), get);
            if (this._isOpen('memory'))
                this._updateMemory(snapshot.memory, history.memory, sparks, get);
            if (this._isOpen('programs'))
                this._updatePrograms();
            if (this._isOpen('storage'))
                this._updateStorage(snapshot.storage, history, sparks, get);
            if (this._isOpen('network'))
                this._updateNetwork(snapshot.network, history, sparks);
            if (this._isOpen('gpu'))
                this._updateGpu(snapshot.gpu, history.gpu, sparks, unit, get);
            if (this._isOpen('temperature'))
                this._updateTemperatures(snapshot.temperature, history.temperature, sparks, unit, get);
            if (this._isOpen('display'))
                this._updateDisplays(snapshot.display);
            if (this._isOpen('controls'))
                this._updateControls(settings);
            if (this._cap)
                this.setWidthCap(this._cap);
        }

        /**
         * One-line summaries on the card headers, so a folded card still says
         * something useful.
         */
        _updateSummaries(snapshot, unit) {
            const summary = (key, text) => setText(this._cards.get(key).summary, text);
            summary('cpu', formatPercent(snapshot.cpu?.usage));
            summary('memory', formatPercent(snapshot.memory?.percent));
            const top = this._groups[0];
            summary('programs', top ? `${top.name} ${formatBytes(top.rss)}` : '');
            const mount = snapshot.storage?.mounts?.[0];
            summary('storage', mount ? `${mount.path} ${formatPercent(mount.percent)}` : 'n/a');
            summary('network', `↓ ${formatBytesPerSecond(snapshot.network?.downBps)}`);
            const gpu = snapshot.gpu?.selected;
            summary('gpu', gpu?.state === 'off' ? 'idle' : formatPercent(gpu?.util));
            summary('temperature', formatTemperature(snapshot.temperature?.headline?.celsius, unit));
            summary('display', formatRefreshHz(snapshot.display?.primaryHz));
            summary('controls', '');
        }

        _updateCpu(cpu, history, sparks, perCore, get) {
            setRow(this._cpuUsage, 'Busy', formatPercent(cpu?.usage),
                thresholdLevel(cpu?.usage, get('cpu-warning'), get('cpu-critical')));
            const freq = cpu?.frequency;
            setRow(this._cpuFreq, 'Frequency', freq
                ? `${formatGigahertz(freq.averageMhz)} · max ${formatGigahertz(freq.maxMhz)}`
                : 'n/a');
            this._cpuSpark.visible = sparks;
            if (sparks)
                this._cpuSpark.setSeries([history ?? []], 100);
            this._coreBox.visible = perCore;
            if (!perCore)
                return;
            syncRows(this._coreRows, this._coreBox, cpu?.cores ?? [], () => labeledMeter(), (row, core) => {
                setRow(row, core.id, formatPercent(core.usage));
                row.meter.setPercent(core.usage);
            });
        }

        _updateMemory(memory, history, sparks, get) {
            if (!memory) {
                for (const row of [this._memUsed, this._memAvailable, this._memCached, this._memSwap])
                    setRow(row, '', 'n/a');
            } else {
                setRow(this._memUsed, 'In use',
                    `${formatBytes(memory.used)} / ${formatBytes(memory.total)} (${formatPercent(memory.percent)})`,
                    thresholdLevel(memory.percent, get('memory-warning'), get('memory-critical')));
                setRow(this._memAvailable, 'Available', formatBytes(Math.max(0, memory.total - memory.used)));
                setRow(this._memCached, 'Cached', formatBytes(memory.cached));
                setRow(this._memSwap, 'Swap', memory.swapTotal > 0
                    ? `${formatBytes(memory.swapUsed)} / ${formatBytes(memory.swapTotal)}`
                    : 'none');
            }
            this._memSpark.visible = sparks;
            if (sparks)
                this._memSpark.setSeries([history ?? []], 100);
        }

        _updatePrograms() {
            const groups = this._groups.slice(0, PROGRAM_ROWS);
            this._programEmpty.visible = groups.length === 0;
            syncRows(this._programRows, this._programBox, groups, () => this._programRow(), (row, group) => {
                const count = group.count > 1 ? ` ×${group.count}` : '';
                setText(row.key, `${group.name}${count}`);
                const cpu = group.cpu == null ? '' : ` · ${Math.round(group.cpu)}%`;
                setText(row.value, `${formatBytes(group.rss)}${cpu}`);
                row.group = group;
                row.button.visible = !group.protected;
                const force = this._forceNames.has(group.comm);
                setText(row.buttonLabel, force ? 'Force end' : 'Close');
                setLevel(row.button, force ? 'critical' : 'normal');
            });
        }

        _programRow() {
            const row = makeRow();
            const buttonLabel = new St.Label({text: 'Close', y_align: Clutter.ActorAlign.CENTER});
            const button = new St.Button({style_class: 'stats-button stats-button-small', can_focus: true, child: buttonLabel});
            const entry = {box: row.box, key: row.key, value: row.value, button, buttonLabel, group: null};
            button.connect('clicked', () => {
                if (entry.group)
                    this._actions.closeProgram(entry.group, this._forceNames.has(entry.group.comm));
            });
            row.box.add_child(button);
            return entry;
        }

        _updateStorage(storage, history, sparks, get) {
            const mounts = storage?.mounts ?? [];
            const items = mounts.length > 0 ? mounts : [null];
            syncRows(this._mountRows, this._mountBox, items, () => labeledMeter(), (row, mount) => {
                if (!mount) {
                    setRow(row, 'Disk', 'n/a');
                    row.meter.setPercent(null);
                    return;
                }
                setRow(row, mount.path,
                    `${formatBytes(mount.used)} / ${formatBytes(mount.size)} (${formatPercent(mount.percent)})`,
                    thresholdLevel(mount.percent, get('storage-warning'), get('storage-critical')));
                row.meter.setPercent(mount.percent);
            });
            setRow(this._diskRead, 'Read', formatBytesPerSecond(storage?.readBps));
            setRow(this._diskWrite, 'Write', formatBytesPerSecond(storage?.writeBps));
            this._diskSpark.visible = sparks;
            if (sparks)
                this._diskSpark.setSeries([history.diskRead ?? [], history.diskWrite ?? []], null);
        }

        _updateNetwork(network, history, sparks) {
            setRow(this._netDown, 'Download', formatBytesPerSecond(network?.downBps));
            setRow(this._netUp, 'Upload', formatBytesPerSecond(network?.upBps));
            this._netSpark.visible = sparks;
            if (sparks)
                this._netSpark.setSeries([history.netDown ?? [], history.netUp ?? []], null);
        }

        _updateGpu(gpu, history, sparks, unit, get) {
            const cards = gpu?.cards ?? [];
            const items = cards.length > 0 ? cards : [null];
            syncRows(this._gpuRows, this._gpuBox, items, () => gpuBlock(), (block, card) => {
                if (!card) {
                    setRow(block.rows[0], 'GPU', 'n/a');
                    for (const row of block.rows.slice(1))
                        row.box.visible = false;
                    return;
                }
                for (const row of block.rows)
                    row.box.visible = true;
                const off = card.state === 'off';
                setRow(block.rows[0], `${card.id} · ${card.vendor}`, off ? 'idle/off' : formatPercent(card.util),
                    off ? 'normal' : thresholdLevel(card.util, get('gpu-warning'), get('gpu-critical')));
                setRow(block.rows[1], 'Frequency', card.frequencyMhz == null ? 'n/a' : formatMegahertz(card.frequencyMhz));
                setRow(block.rows[2], 'VRAM', card.vramTotal
                    ? `${formatBytes(card.vramUsed)} / ${formatBytes(card.vramTotal)}`
                    : 'n/a');
                setRow(block.rows[3], 'Temperature', formatTemperature(card.tempC, unit));
                setRow(block.rows[4], 'Power', formatWatts(card.powerW));
                for (const row of block.rows.slice(1))
                    row.box.visible = !off;
            });
            this._gpuSpark.visible = sparks;
            if (sparks)
                this._gpuSpark.setSeries([history ?? []], 100);
        }

        _updateTemperatures(temperature, history, sparks, unit, get) {
            const headline = temperature?.headline;
            if (!headline) {
                setRow(this._tempHeadline, 'Headline', 'n/a');
            } else {
                const value = headline.state === 'off' ? 'idle/off' : formatTemperature(headline.celsius, unit);
                setRow(this._tempHeadline, headline.label, value,
                    thresholdLevel(headline.celsius, get('temp-warning'), get('temp-critical')));
            }
            const sensors = temperature?.sensors ?? [];
            syncRows(this._tempRows, this._tempBox, sensors, () => makeRow(), (row, sensor) => {
                const value = sensor.state === 'off' ? 'idle/off' : formatTemperature(sensor.celsius, unit);
                setRow(row, `${sensor.chip} ${sensor.label}`, value);
            });
            this._tempSpark.visible = sparks;
            if (sparks)
                this._tempSpark.setSeries([history ?? []], null);
        }

        _updateDisplays(display) {
            const monitors = display?.monitors ?? [];
            const items = monitors.length > 0 ? monitors : [null];
            syncRows(this._displayRows, this._displayBox, items, () => makeRow(), (row, monitor) => {
                if (!monitor) {
                    setRow(row, 'Display', formatRefreshHz(display?.primaryHz));
                    return;
                }
                const resolution = monitor.width && monitor.height ? `${monitor.width}×${monitor.height}` : 'n/a';
                const vrr = monitor.vrr ? ' · VRR' : '';
                setRow(row, monitor.connector, `${resolution} · ${formatRefreshHz(monitor.refresh)}${vrr}`);
            });
        }

        _updateControls(settings) {
            this._positionControl.select(settings.get_string('position'));
            this._modeControl.select(settings.get_string('display-mode'));
            for (const {key, row} of this._switches)
                row.setState(settings.get_boolean(key));
        }

        /**
         * Cap the dashboard to the monitor; labels ellipsize inside it.
         *
         * @param {number} width
         */
        setWidthCap(width) {
            this._cap = width;
            const style = `max-width: ${width}px;`;
            if (this.style !== style)
                this.style = style;
        }
    });

/**
 * A label/value row with a meter under it.
 *
 * @returns {{box: St.BoxLayout, key: St.Label, value: St.Label, meter: Meter}}
 */
function labeledMeter() {
    const box = new St.BoxLayout({vertical: true, style_class: 'stats-meter-row', x_expand: true});
    const row = makeRow();
    const meter = new Meter();
    box.add_child(row.box);
    box.add_child(meter);
    return {box, key: row.key, value: row.value, meter};
}

/**
 * Five rows for one graphics card.
 *
 * @returns {{box: St.BoxLayout, rows: ReturnType<typeof makeRow>[]}}
 */
function gpuBlock() {
    const box = new St.BoxLayout({vertical: true, style_class: 'stats-stack'});
    const rows = [0, 1, 2, 3, 4].map(() => makeRow());
    for (const row of rows)
        box.add_child(row.box);
    return {box, rows};
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
