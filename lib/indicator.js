import Clutter from 'gi://Clutter';
import GObject from 'gi://GObject';
import Shell from 'gi://Shell';
import St from 'gi://St';

import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';

import {METRICS} from './constants.js';
import {Dashboard} from './widgets/dashboard.js';
import {
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
            this.add_child(this._bar);
            this._items = new Map();
            this._barSignature = '';
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
                this._renderBar();
            });
            this._unsubscribe = poller.subscribe(snapshot => {
                this._snapshot = snapshot;
                this._render();
            });
            this.connect('destroy', () => this._onDestroy());
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

        _render() {
            if (!this._snapshot || !this._settings)
                return;
            this._renderBar();
            this._renderPopup();
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
            const showIcons = mode !== 'text';
            const parts = ids.map(id => `${id}:${metricText(id, this._snapshot, this._settings)}:${metricLevel(id, this._snapshot, this._settings)}`);
            const signature = `${mode}|${showIcons}|${expanded}|${parts.join('|')}`;
            if (signature === this._barSignature)
                return;
            this._barSignature = signature;
            this._bar.remove_all_children();
            for (const id of ids) {
                const item = this._ensureItem(id);
                const text = metricText(id, this._snapshot, this._settings);
                if (item.label.text !== text)
                    item.label.text = text;
                item.icon.visible = showIcons;
                applyLevel(item.box, metricLevel(id, this._snapshot, this._settings));
                this._bar.add_child(item.box);
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
            const label = new St.Label({
                text: '',
                style_class: 'stats-value',
                y_align: Clutter.ActorAlign.CENTER,
            });
            box.add_child(icon);
            box.add_child(label);
            const item = {box, icon, label};
            this._items.set(id, item);
            return item;
        }
    });

/**
 * @param {string} id
 * @param {object} snapshot
 * @param {Gio.Settings} settings
 * @returns {string}
 */
function metricText(id, snapshot, settings) {
    switch (id) {
    case 'cpu':
        return `CPU ${formatPercent(snapshot.cpu?.usage)}`;
    case 'memory':
        return `RAM ${formatPercent(snapshot.memory?.percent)}`;
    case 'storage':
        return `Disk ${formatPercent(snapshot.storage?.mounts?.[0]?.percent)}`;
    case 'gpu':
        return gpuBarText(snapshot.gpu?.selected);
    case 'temperature':
        return formatTemperature(
            snapshot.temperature?.headline?.celsius,
            settings.get_string('temperature-unit'));
    case 'refresh':
        return formatRefreshHz(snapshot.display?.primaryHz);
    default:
        return '';
    }
}

/**
 * @param {object|null|undefined} card
 * @returns {string}
 */
function gpuBarText(card) {
    if (!card)
        return 'GPU n/a';
    if (card.state === 'off')
        return 'GPU idle/off';
    if (card.util == null)
        return 'GPU n/a';
    return `GPU ${formatPercent(card.util)}`;
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
 * @param {St.Widget} box
 * @param {'normal'|'warning'|'critical'} level
 */
function applyLevel(box, level) {
    for (const name of LEVELS)
        box.remove_style_class_name(name);
    if (level === 'warning' || level === 'critical')
        box.add_style_class_name(`stats-${level}`);
}
