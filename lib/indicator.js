import Clutter from 'gi://Clutter';
import GObject from 'gi://GObject';
import Pango from 'gi://Pango';
import Shell from 'gi://Shell';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

import {METRICS} from './constants.js';
import {naturalWidth, sideBudget} from './panel_fit.js';
import {Dashboard} from './widgets/dashboard.js';
import {Meter} from './widgets/meter.js';
import {
    formatBytes,
    formatBytesPerSecond,
    formatGigahertz,
    formatMegahertz,
    formatPercent,
    formatRefreshHz,
    formatTemperature,
    thresholdLevel,
} from './widgets/formatters.js';

const SYSTEM_MONITOR_IDS = [
    'org.gnome.SystemMonitor.desktop',
    'gnome-system-monitor.desktop',
];

const LEVELS = ['stats-warning', 'stats-critical'];

export const StatsIndicator = GObject.registerClass(
    class StatsIndicator extends PanelMenu.Button {
    /**
     * @param {import('./poller.js').Poller} poller
     * @param {Gio.Settings} settings
     * @param {() => void} openPreferences
     */
        _init(poller, settings, openPreferences) {
            const side = settings.get_string('position');
            super._init(side === 'left' ? 0 : 1, 'Stats', false);
            this._poller = poller;
            this._settings = settings;
            this._openPreferences = openPreferences;
            this.track_hover = true;
            this.add_style_class_name('stats-indicator');

            this._bar = new St.BoxLayout({style_class: 'stats-bar'});
            this._startup = new St.BoxLayout({style_class: 'stats-startup'});
            this._startupMeter = new Meter();
            this._startup.add_child(this._startupMeter);
            // PanelMenu.ButtonBox measures and allocates its first child and
            // nothing else, so the meter and the metric row share one root box.
            this._root = new St.BoxLayout({style_class: 'stats-root'});
            this._root.add_child(this._startup);
            this._root.add_child(this._bar);
            this.add_child(this._root);
            this._bar.visible = false;
            this._items = new Map();
            this._separators = [];
            this._barIds = [];
            this._barSignature = '';
            this._mode = 'icon-text';
            this._fitting = false;
            this._snapshot = null;

            this._dashboard = new Dashboard();
            const item = new PopupMenu.PopupBaseMenuItem({
                reactive: false,
                can_focus: false,
                style_class: 'stats-dashboard-item',
            });
            item.add_child(this._dashboard);
            this.menu.addMenuItem(item);
            this.menu.addAction('Preferences', () => this._openPreferences?.());
            this._systemItem = null;

            this._settingId = settings.connect('changed', () => this._render());
            this._hoverId = this.connect('notify::hover', () => this._renderBar());
            this._menuId = this.menu.connect('open-state-changed', () => {
                this._syncSystemMonitor();
                this._capPopup();
                this._renderBar();
            });
            this._allocId = this.connect('notify::allocation', () => this._fitBar());
            this._layoutManager = Main.layoutManager;
            this._monitorId = this._layoutManager?.connect('monitors-changed', () => {
                this._capPopup();
                this._fitBar();
            }) ?? 0;
            this._unsubscribe = poller.subscribe(snapshot => {
                this._snapshot = snapshot;
                this._render();
            });
            this.connect('destroy', () => this._onDestroy());
            this._capPopup();
            this._syncSystemMonitor();
        }

        _onDestroy() {
            if (this._hoverId) {
                this.disconnect(this._hoverId);
                this._hoverId = 0;
            }
            if (this._menuId) {
                this.menu.disconnect(this._menuId);
                this._menuId = 0;
            }
            if (this._allocId) {
                this.disconnect(this._allocId);
                this._allocId = 0;
            }
            if (this._monitorId && this._layoutManager) {
                this._layoutManager.disconnect(this._monitorId);
                this._monitorId = 0;
            }
            this._layoutManager = null;
            if (this._settingId && this._settings) {
                this._settings.disconnect(this._settingId);
                this._settingId = 0;
            }
            this._unsubscribe?.();
            this._unsubscribe = null;
            this._poller = null;
            this._settings = null;
            this._openPreferences = null;
            this._snapshot = null;
            this._items.clear();
            this._separators = [];
            this._barIds = [];
        }

        _syncSystemMonitor() {
            if (this._systemItem)
                return;
            const appSystem = Shell.AppSystem.get_default();
            const app = SYSTEM_MONITOR_IDS
                .map(id => appSystem.lookup_app(id))
                .find(found => found);
            if (!app)
                return;
            this._systemItem = this.menu.addAction('Open System Monitor', () => {
                app.activate();
            });
        }

        _capPopup() {
            const monitor = Main.layoutManager?.primaryMonitor;
            const screen = monitor?.width ?? 480;
            const width = Math.min(480, Math.max(0, screen - 24));
            this._dashboard.setWidthCap(width);
            if (this.menu.box)
                this.menu.box.style = `max-width: ${width}px;`;
        }

        _render() {
            if (!this._snapshot || !this._settings)
                return;
            if ((this._snapshot.progress ?? 0) < 1) {
                this.setProgress(this._snapshot.progress ?? 0);
                return;
            }
            this._showLive();
            this._renderBar();
            this._renderPopup();
        }

        /**
         * @param {number} fraction
         */
        setProgress(fraction) {
            const percent = Math.max(0, Math.min(1, fraction)) * 100;
            this._startup.visible = true;
            this._bar.visible = false;
            this._startupMeter.setPercent(percent);
        }

        _showLive() {
            if (this._bar.visible && !this._startup.visible)
                return;
            this._startup.visible = false;
            this._bar.visible = true;
        }

        _renderPopup() {
            if (!this._snapshot || !this._settings)
                return;
            this._dashboard.update(this._snapshot, this._settings);
        }

        /**
         * Compact mode keeps the first enabled metric until hover or until the
         * menu is open. That metric is the highest-priority one in bar-metrics.
         */
        _renderBar() {
            if (!this._snapshot || !this._settings)
                return;
            const mode = this._settings.get_string('display-mode');
            let ids = this._settings.get_strv('bar-metrics').filter(id => METRICS.some(metric => metric.id === id));
            const expanded = this.hover || this.menu.isOpen;
            if (mode === 'compact' && !expanded)
                ids = ids.slice(0, 1);
            const parts = ids.map(id => {
                const part = metricParts(id, this._snapshot, this._settings);
                const level = metricLevel(id, this._snapshot, this._settings);
                return `${id}:${part.name}:${part.primary}:${part.speed}:${level}`;
            });
            const structure = `${mode}|${ids.join(',')}`;
            const signature = `${structure}|${parts.join('|')}`;
            this._mode = mode;
            if (signature === this._barSignature)
                return;
            const structureChanged = this._barIds.join(',') !== ids.join(',');
            this._barSignature = signature;
            if (structureChanged)
                this._rebuild(ids);
            for (const id of ids) {
                const item = this._ensureItem(id);
                const part = metricParts(id, this._snapshot, this._settings);
                setText(item.name, part.name);
                setText(item.primary, part.primary);
                setText(item.speed, part.speed);
                applyLevel(item.box, metricLevel(id, this._snapshot, this._settings));
            }
            this._fitBar();
        }

        /**
         * @param {string[]} ids
         */
        _rebuild(ids) {
            this._bar.remove_all_children();
            for (const separator of this._separators)
                separator.destroy();
            this._separators = [];
            this._barIds = ids;
            ids.forEach((id, index) => {
                if (index > 0) {
                    const separator = plainLabel('stats-sep');
                    separator.text = ' | ';
                    this._separators.push(separator);
                    this._bar.add_child(separator);
                }
                this._bar.add_child(this._ensureItem(id).box);
            });
        }

        /**
         * Richest first: icon, name, value and speed. Then drop speed.
         * Then drop the icon so a narrow panel shows the name and the value.
         * If that still overflows, drop refresh, temperature, then percent
         * metrics from the right, leaving at least one.
         *
         * @returns {{detail: number, hidden: Set<string>}[]}
         */
        _levels() {
            const ids = this._barIds;
            const levels = [
                {detail: 2, hidden: new Set()},
                {detail: 1, hidden: new Set()},
                {detail: 0, hidden: new Set()},
            ];
            const drop = [];
            if (ids.includes('refresh'))
                drop.push('refresh');
            if (ids.includes('temperature'))
                drop.push('temperature');
            const percents = ids.filter(id => id !== 'refresh' && id !== 'temperature');
            for (let index = percents.length - 1; index >= 1; index--)
                drop.push(percents[index]);
            const hidden = new Set();
            for (const id of drop) {
                hidden.add(id);
                levels.push({detail: 0, hidden: new Set(hidden)});
            }
            return levels;
        }

        /**
         * Use the richest level that fits the panel side budget.
         */
        _fitBar() {
            if (this._fitting || this._barIds.length === 0)
                return;
            const available = sideBudget(this);
            if (!(available > 0)) {
                this._applyLevel({detail: 2, hidden: new Set()});
                return;
            }
            this._fitting = true;
            try {
                const levels = this._levels();
                let chosen = levels[levels.length - 1];
                for (const level of levels) {
                    this._applyLevel(level);
                    if (this._contentWidth() <= available) {
                        chosen = level;
                        break;
                    }
                }
                this._applyLevel(chosen);
            } finally {
                this._fitting = false;
            }
        }

        /**
         * Sum the natural widths of what is actually visible, including the
         * 4px gap between parts of one metric.
         *
         * @returns {number}
         */
        _contentWidth() {
            let total = 0;
            let previousShown = false;
            for (let index = 0; index < this._barIds.length; index++) {
                const item = this._items.get(this._barIds[index]);
                if (!item?.box.visible) {
                    previousShown = false;
                    continue;
                }
                if (previousShown && this._separators[index - 1])
                    total += naturalWidth(this._separators[index - 1]);
                const parts = [item.icon, item.name, item.primary, item.speed]
                    .filter(actor => actor.visible);
                for (const actor of parts)
                    total += naturalWidth(actor);
                if (parts.length > 1)
                    total += 4 * (parts.length - 1);
                previousShown = true;
            }
            return total + this._buttonChrome();
        }

        /**
         * Horizontal padding of the panel button, outside the metric row.
         *
         * @returns {number}
         */
        _buttonChrome() {
            const node = this.get_theme_node?.();
            if (!node?.get_padding)
                return 12;
            let chrome = node.get_padding(St.Side.LEFT) + node.get_padding(St.Side.RIGHT);
            if (node.get_margin)
                chrome += node.get_margin(St.Side.LEFT) + node.get_margin(St.Side.RIGHT);
            return chrome;
        }

        /**
         * @param {{detail: number, hidden: Set<string>}} level
         */
        _applyLevel(level) {
            const showIcons = this._mode !== 'text' && level.detail >= 1;
            const showName = true;
            const showSpeed = level.detail >= 2;
            let previousShown = false;
            for (let index = 0; index < this._barIds.length; index++) {
                const id = this._barIds[index];
                const item = this._items.get(id);
                const shown = !level.hidden.has(id);
                item.box.visible = shown;
                item.icon.visible = showIcons;
                item.name.visible = showName && item.name.text.length > 0;
                item.speed.visible = showSpeed && item.speed.text.length > 0;
                item.primary.visible = item.primary.text.length > 0;
                if (index > 0)
                    this._separators[index - 1].visible = shown && previousShown;
                previousShown = shown;
            }
        }

        /**
         * @param {string} id
         */
        _ensureItem(id) {
            const existing = this._items.get(id);
            if (existing)
                return existing;
            const spec = METRICS.find(metric => metric.id === id);
            const box = new St.BoxLayout({style_class: `stats-item stats-${id}`});
            const icon = new St.Icon({
                icon_name: spec?.icon ?? 'computer-symbolic',
                style_class: 'system-status-icon stats-icon',
            });
            const name = plainLabel('stats-name');
            const primary = plainLabel('stats-value');
            const speed = plainLabel('stats-speed');
            box.add_child(icon);
            box.add_child(name);
            box.add_child(primary);
            box.add_child(speed);
            const item = {box, icon, name, primary, speed};
            this._items.set(id, item);
            return item;
        }
    });

/**
 * @param {string} styleClass
 * @returns {St.Label}
 */
function plainLabel(styleClass) {
    const label = new St.Label({
        text: '',
        style_class: styleClass,
        y_align: Clutter.ActorAlign.CENTER,
    });
    label.clutter_text.ellipsize = Pango.EllipsizeMode.NONE;
    return label;
}

/**
 * @param {string} id
 * @param {object} snapshot
 * @param {Gio.Settings} settings
 * @returns {{name: string, primary: string, speed: string}}
 */
function metricParts(id, snapshot, settings) {
    switch (id) {
    case 'cpu':
        return {
            name: 'CPU',
            primary: formatPercent(snapshot.cpu?.usage),
            speed: formatGigahertz(snapshot.cpu?.frequency?.averageMhz),
        };
    case 'memory':
        return {
            name: 'RAM',
            primary: formatPercent(snapshot.memory?.percent),
            speed: formatBytes(snapshot.memory?.used),
        };
    case 'storage':
        return {
            name: 'Disk',
            primary: formatPercent(snapshot.storage?.mounts?.[0]?.percent),
            speed: formatBytesPerSecond(combinedBps(snapshot.storage)),
        };
    case 'gpu':
        return gpuParts(snapshot.gpu?.selected);
    case 'temperature':
        return {
            name: '',
            primary: formatTemperature(
                snapshot.temperature?.headline?.celsius,
                settings.get_string('temperature-unit')),
            speed: '',
        };
    case 'refresh':
        return {
            name: '',
            primary: formatRefreshHz(snapshot.display?.primaryHz),
            speed: '',
        };
    default:
        return {name: '', primary: '', speed: ''};
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
 * @param {object|null|undefined} card
 * @returns {{name: string, primary: string, speed: string}}
 */
function gpuParts(card) {
    if (!card)
        return {name: 'GPU', primary: 'n/a', speed: ''};
    if (card.state === 'off')
        return {name: 'GPU', primary: 'idle/off', speed: ''};
    return {
        name: 'GPU',
        primary: formatPercent(card.util),
        speed: card.frequencyMhz == null ? '' : formatMegahertz(card.frequencyMhz),
    };
}

/**
 * @param {string} id
 * @param {object} snapshot
 * @param {Gio.Settings} settings
 * @returns {'normal'|'warning'|'critical'}
 */
function metricLevel(id, snapshot, settings) {
    const keys = thresholdKey(id);
    if (!keys)
        return 'normal';
    return thresholdLevel(
        metricNumber(id, snapshot),
        settings.get_int(keys[0]),
        settings.get_int(keys[1]));
}

/**
 * @param {string} id
 * @returns {[string, string]|null}
 */
function thresholdKey(id) {
    switch (id) {
    case 'cpu':
    case 'memory':
    case 'storage':
    case 'gpu':
        return [`${id}-warning`, `${id}-critical`];
    case 'temperature':
        return ['temp-warning', 'temp-critical'];
    default:
        return null;
    }
}

/**
 * @param {string} id
 * @param {object} snapshot
 * @returns {number|null}
 */
function metricNumber(id, snapshot) {
    switch (id) {
    case 'cpu':
        return snapshot.cpu?.usage ?? null;
    case 'memory':
        return snapshot.memory?.percent ?? null;
    case 'storage':
        return snapshot.storage?.mounts?.[0]?.percent ?? null;
    case 'gpu':
        return snapshot.gpu?.selected?.state === 'off' ? null : (snapshot.gpu?.selected?.util ?? null);
    case 'temperature':
        return snapshot.temperature?.headline?.celsius ?? null;
    default:
        return null;
    }
}

/**
 * @param {St.Label} label
 * @param {string} text
 */
function setText(label, text) {
    if (label.text !== text)
        label.text = text;
}

/**
 * @param {St.Widget} box
 * @param {'normal'|'warning'|'critical'} level
 */
function applyLevel(box, level) {
    for (const name of LEVELS)
        box.remove_style_class_name(name);
    if (level === 'warning' || level === 'critical')
        box.add_style_class_name(`stats-${level}`);
}
