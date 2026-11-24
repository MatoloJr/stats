import Adw from 'gi://Adw';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Gtk from 'gi://Gtk';

import {ExtensionPreferences} from 'resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js';

import {METRICS} from './lib/constants.js';
import {listGpuCards} from './lib/sensors/gpu/index.js';
import {TemperatureSensor} from './lib/sensors/temperature.js';

export default class StatsPreferences extends ExtensionPreferences {
    /**
     * @param {Adw.PreferencesWindow} window
     */
    fillPreferencesWindow(window) {
        this._closed = false;
        this._deferId = 0;
        this._metricsLock = false;
        this._metricRows = [];
        this._mountRows = [];
        this._gpuIds = [''];
        this._tempIds = [''];
        window.set_title('Stats');
        window.set_search_enabled(true);
        window.connect('destroy', () => {
            this._closed = true;
            if (this._deferId) {
                GLib.source_remove(this._deferId);
                this._deferId = 0;
            }
        });

        const settings = this.getSettings();
        window.add(this._generalPage(settings));
        window.add(this._metricsPage(settings));
        window.add(this._thresholdPage(settings));
        window.add(this._popupPage(settings));
        window.add(this._aboutPage(settings));
    }

    /**
     * @param {() => void} callback
     */
    _defer(callback) {
        if (this._deferId)
            GLib.source_remove(this._deferId);
        this._deferId = GLib.idle_add(GLib.PRIORITY_DEFAULT, () => {
            this._deferId = 0;
            if (!this._closed)
                callback();
            return GLib.SOURCE_REMOVE;
        });
    }

    /**
     * @param {Gio.Settings} settings
     */
    _generalPage(settings) {
        const page = new Adw.PreferencesPage({
            title: 'General',
            icon_name: 'preferences-system-symbolic',
        });
        const group = new Adw.PreferencesGroup({title: 'Panel'});
        page.add(group);

        addCombo(
            group,
            settings,
            'position',
            'Position',
            ['Left', 'Right'],
            ['left', 'right'],
            nick => {
                if (nick === 'left' && settings.get_int('position-index') === 0)
                    settings.set_int('position-index', 20);
            });
        addSpin(
            group,
            settings,
            'position-index',
            'Position index',
            'Use 20 on the left to sit after the workspace indicator, just before the clock',
            0,
            20,
            1);
        addSpin(
            group,
            settings,
            'poll-interval-ms',
            'Poll interval',
            'Milliseconds between sensor reads',
            500,
            10000,
            100);
        addCombo(
            group,
            settings,
            'temperature-unit',
            'Temperature unit',
            ['Celsius', 'Fahrenheit'],
            ['celsius', 'fahrenheit']);
        addCombo(
            group,
            settings,
            'display-mode',
            'Display mode',
            ['Icon and text', 'Text only', 'Compact'],
            ['icon-text', 'text', 'compact']);
        addSwitch(
            group,
            settings,
            'pause-when-locked',
            'Pause while locked or idle',
            'Stop polling when the screen is locked or the session is idle');
        addSwitch(
            group,
            settings,
            'pressure-notifications',
            'Resource warnings',
            'After CPU or memory stays high, name the program that is above its recent share');
        return page;
    }

    /**
     * @param {Gio.Settings} settings
     */
    _metricsPage(settings) {
        const page = new Adw.PreferencesPage({
            title: 'Metrics',
            icon_name: 'view-list-symbolic',
        });
        this._metricsGroup = new Adw.PreferencesGroup({
            title: 'Top bar',
            description: 'Enabled metrics are shown in this order. Compact mode shows the first one until hover.',
        });
        page.add(this._metricsGroup);
        this._buildMetrics(settings);
        settings.connect('changed::bar-metrics', () => {
            this._defer(() => this._buildMetrics(settings));
        });

        const device = new Adw.PreferencesGroup({title: 'Devices'});
        page.add(device);
        this._gpuRow = new Adw.ComboRow({
            title: 'Top-bar GPU',
            subtitle: 'Automatic prefers the active discrete GPU',
        });
        device.add(this._gpuRow);
        this._tempRow = new Adw.ComboRow({
            title: 'Headline temperature',
            subtitle: 'Automatic uses the CPU package sensor',
        });
        device.add(this._tempRow);
        this._bindIdCombo(settings, this._gpuRow, 'gpu-id', () => this._gpuIds);
        this._bindIdCombo(settings, this._tempRow, 'headline-temp', () => this._tempIds);
        this._loadGpuChoices(settings);
        this._loadTempChoices(settings);

        const mounts = new Adw.PreferencesGroup({
            title: 'Mount points',
            description: 'Disk space for each path. Throughput still comes from physical disks.',
        });
        page.add(mounts);
        const entry = new Adw.EntryRow({title: 'Add a mount point'});
        const addButton = new Gtk.Button({
            icon_name: 'list-add-symbolic',
            valign: Gtk.Align.CENTER,
        });
        entry.add_suffix(addButton);
        addButton.connect('clicked', () => {
            const path = entry.get_text().trim();
            if (!path.startsWith('/'))
                return;
            const current = settings.get_strv('mount-points');
            if (!current.includes(path))
                settings.set_strv('mount-points', [...current, path]);
            entry.set_text('');
        });
        mounts.add(entry);
        this._mountGroup = mounts;
        this._buildMounts(settings);
        settings.connect('changed::mount-points', () => {
            this._defer(() => this._buildMounts(settings));
        });
        return page;
    }

    /**
     * @param {Gio.Settings} settings
     */
    _buildMetrics(settings) {
        if (this._closed)
            return;
        this._metricsLock = true;
        for (const row of this._metricRows)
            this._metricsGroup.remove(row);
        this._metricRows = [];
        const enabled = settings.get_strv('bar-metrics')
            .filter(id => METRICS.some(metric => metric.id === id));
        const ordered = [
            ...enabled.map(id => METRICS.find(metric => metric.id === id)),
            ...METRICS.filter(metric => !enabled.includes(metric.id)),
        ];
        for (const metric of ordered) {
            const row = new Adw.ActionRow({title: metric.label});
            const toggle = new Gtk.Switch({
                active: enabled.includes(metric.id),
                valign: Gtk.Align.CENTER,
            });
            const up = iconButton('go-up-symbolic');
            const down = iconButton('go-down-symbolic');
            const index = enabled.indexOf(metric.id);
            up.sensitive = index > 0;
            down.sensitive = index >= 0 && index < enabled.length - 1;
            toggle.connect('notify::active', () => {
                if (this._metricsLock)
                    return;
                let order = settings.get_strv('bar-metrics')
                    .filter(id => METRICS.some(item => item.id === id));
                if (toggle.active) {
                    if (!order.includes(metric.id))
                        order = [...order, metric.id];
                } else {
                    order = order.filter(id => id !== metric.id);
                }
                settings.set_strv('bar-metrics', order);
            });
            up.connect('clicked', () => moveMetric(settings, metric.id, -1));
            down.connect('clicked', () => moveMetric(settings, metric.id, 1));
            row.add_suffix(up);
            row.add_suffix(down);
            row.add_suffix(toggle);
            this._metricsGroup.add(row);
            this._metricRows.push(row);
        }
        this._metricsLock = false;
    }

    /**
     * @param {Gio.Settings} settings
     */
    _buildMounts(settings) {
        if (this._closed)
            return;
        for (const row of this._mountRows)
            this._mountGroup.remove(row);
        this._mountRows = [];
        for (const path of settings.get_strv('mount-points')) {
            const row = new Adw.ActionRow({title: path});
            const remove = iconButton('list-remove-symbolic');
            remove.connect('clicked', () => {
                const next = settings.get_strv('mount-points').filter(item => item !== path);
                settings.set_strv('mount-points', next.length > 0 ? next : ['/']);
            });
            row.add_suffix(remove);
            this._mountGroup.add(row);
            this._mountRows.push(row);
        }
    }

    /**
     * @param {Gio.Settings} settings
     * @param {Adw.ComboRow} row
     * @param {string} key
     * @param {() => string[]} ids
     */
    _bindIdCombo(settings, row, key, ids) {
        row.connect('notify::selected', () => {
            if (row._statsLock)
                return;
            const list = ids();
            const value = list[row.selected];
            if (value === undefined || settings.get_string(key) === value)
                return;
            settings.set_string(key, value);
        });
        settings.connect(`changed::${key}`, () => {
            const list = ids();
            const index = Math.max(0, list.indexOf(settings.get_string(key)));
            if (row.selected !== index)
                row.set_selected(index);
        });
    }

    /**
     * @param {Gio.Settings} settings
     */
    _loadGpuChoices(settings) {
        listGpuCards(null).then(cards => {
            if (this._closed)
                return;
            const labels = ['Automatic'];
            const ids = [''];
            for (const card of cards) {
                ids.push(card.id);
                labels.push(`${card.id} (${card.vendor})`);
            }
            this._gpuIds = ids;
            applyStringModel(this._gpuRow, labels, ids, settings.get_string('gpu-id'));
        }).catch(error => {
            console.warn(`[stats] GPU list failed: ${error.message}`);
        });
    }

    /**
     * @param {Gio.Settings} settings
     */
    _loadTempChoices(settings) {
        const sensor = new TemperatureSensor();
        sensor.discover(null).then(found => {
            sensor.destroy();
            if (this._closed)
                return;
            const labels = ['Automatic'];
            const ids = [''];
            for (const item of found) {
                if (item.celsius == null && item.state !== 'off')
                    continue;
                ids.push(item.id);
                labels.push(`${item.chip} ${item.label}`);
            }
            this._tempIds = ids;
            applyStringModel(this._tempRow, labels, ids, settings.get_string('headline-temp'));
        }).catch(error => {
            sensor.destroy();
            console.warn(`[stats] temperature list failed: ${error.message}`);
        });
    }

    /**
     * @param {Gio.Settings} settings
     */
    _thresholdPage(settings) {
        const page = new Adw.PreferencesPage({
            title: 'Thresholds',
            icon_name: 'preferences-color-symbolic',
        });
        const usage = [
            ['cpu', 'CPU'],
            ['memory', 'Memory'],
            ['storage', 'Storage'],
            ['gpu', 'GPU'],
        ];
        for (const [id, label] of usage) {
            const group = new Adw.PreferencesGroup({title: label});
            page.add(group);
            addSpin(group, settings, `${id}-warning`, 'Warning', 'Percent', 0, 100, 1);
            addSpin(group, settings, `${id}-critical`, 'Critical', 'Percent', 0, 100, 1);
        }
        const temp = new Adw.PreferencesGroup({
            title: 'Temperature',
            description: 'Compared in Celsius even when the bar shows Fahrenheit.',
        });
        page.add(temp);
        addSpin(temp, settings, 'temp-warning', 'Warning', 'Degrees Celsius', 0, 150, 1);
        addSpin(temp, settings, 'temp-critical', 'Critical', 'Degrees Celsius', 0, 150, 1);
        return page;
    }

    /**
     * @param {Gio.Settings} settings
     */
    _popupPage(settings) {
        const page = new Adw.PreferencesPage({
            title: 'Popup',
            icon_name: 'utilities-system-monitor-symbolic',
        });
        const group = new Adw.PreferencesGroup();
        page.add(group);
        addSwitch(group, settings, 'show-sparklines', 'Sparklines', 'Draw history charts in the popup');
        addSwitch(group, settings, 'show-per-core', 'Per-core bars', 'Show one bar for each CPU core');
        const choices = [30, 60, 120];
        const model = new Gtk.StringList();
        for (const seconds of choices)
            model.append(`${seconds} seconds`);
        const row = new Adw.ComboRow({
            title: 'History length',
            model,
        });
        group.add(row);
        const apply = () => {
            let index = choices.indexOf(settings.get_int('history-seconds'));
            if (index < 0)
                index = 1;
            if (row.selected !== index)
                row.set_selected(index);
        };
        apply();
        row.connect('notify::selected', () => {
            const seconds = choices[row.selected];
            if (seconds && settings.get_int('history-seconds') !== seconds)
                settings.set_int('history-seconds', seconds);
        });
        settings.connect('changed::history-seconds', apply);
        return page;
    }

    /**
     * @param {Gio.Settings} settings
     */
    _aboutPage(settings) {
        const page = new Adw.PreferencesPage({
            title: 'About',
            icon_name: 'help-about-symbolic',
        });
        const group = new Adw.PreferencesGroup({title: 'Stats'});
        page.add(group);
        group.add(new Adw.ActionRow({
            title: 'Version',
            subtitle: String(this.metadata.version ?? ''),
        }));
        group.add(new Adw.ActionRow({
            title: 'Project page',
            subtitle: this.metadata.url || 'https://github.com/MatoloJr/stats',
        }));
        const reset = new Adw.ButtonRow({
            title: 'Reset to defaults',
        });
        reset.connect('activated', () => {
            for (const key of settings.list_keys())
                settings.reset(key);
        });
        group.add(reset);
        return page;
    }
}

/**
 * @param {string} iconName
 * @returns {Gtk.Button}
 */
function iconButton(iconName) {
    return new Gtk.Button({
        icon_name: iconName,
        valign: Gtk.Align.CENTER,
    });
}

/**
 * @param {Adw.PreferencesGroup} group
 * @param {Gio.Settings} settings
 * @param {string} key
 * @param {string} title
 * @param {string} subtitle
 */
function addSwitch(group, settings, key, title, subtitle) {
    const row = new Adw.SwitchRow({title, subtitle});
    group.add(row);
    settings.bind(key, row, 'active', Gio.SettingsBindFlags.DEFAULT);
    return row;
}

/**
 * @param {Adw.PreferencesGroup} group
 * @param {Gio.Settings} settings
 * @param {string} key
 * @param {string} title
 * @param {string[]} labels
 * @param {string[]} nicks
 * @param {(nick: string) => void} [onSelect]
 */
function addCombo(group, settings, key, title, labels, nicks, onSelect) {
    const model = new Gtk.StringList();
    for (const label of labels)
        model.append(label);
    const row = new Adw.ComboRow({title, model});
    group.add(row);
    const apply = () => {
        const index = Math.max(0, nicks.indexOf(settings.get_string(key)));
        if (row.selected !== index)
            row.set_selected(index);
    };
    apply();
    row.connect('notify::selected', () => {
        const nick = nicks[row.selected];
        if (!nick || settings.get_string(key) === nick)
            return;
        settings.set_string(key, nick);
        onSelect?.(nick);
    });
    settings.connect(`changed::${key}`, apply);
    return row;
}

/**
 * @param {Adw.PreferencesGroup} group
 * @param {Gio.Settings} settings
 * @param {string} key
 * @param {string} title
 * @param {string} subtitle
 * @param {number} lower
 * @param {number} upper
 * @param {number} step
 */
function addSpin(group, settings, key, title, subtitle, lower, upper, step) {
    const row = new Adw.SpinRow({
        title,
        subtitle,
        adjustment: new Gtk.Adjustment({
            lower,
            upper,
            step_increment: step,
            page_increment: step * 2,
            value: settings.get_int(key),
        }),
    });
    row.set_digits(0);
    group.add(row);
    row.connect('notify::value', () => {
        const next = Math.round(row.value);
        if (settings.get_int(key) !== next)
            settings.set_int(key, next);
    });
    settings.connect(`changed::${key}`, () => {
        const value = settings.get_int(key);
        if (Math.round(row.value) !== value)
            row.set_value(value);
    });
    return row;
}

/**
 * @param {Gio.Settings} settings
 * @param {string} id
 * @param {number} direction
 */
function moveMetric(settings, id, direction) {
    const order = settings.get_strv('bar-metrics')
        .filter(item => METRICS.some(metric => metric.id === item));
    const index = order.indexOf(id);
    const target = index + direction;
    if (index < 0 || target < 0 || target >= order.length)
        return;
    const swap = order[target];
    order[target] = order[index];
    order[index] = swap;
    settings.set_strv('bar-metrics', order);
}

/**
 * @param {Adw.ComboRow} row
 * @param {string[]} labels
 * @param {string[]} ids
 * @param {string} selected
 */
function applyStringModel(row, labels, ids, selected) {
    const model = new Gtk.StringList();
    for (const label of labels)
        model.append(label);
    row._statsLock = true;
    row.set_model(model);
    const index = Math.max(0, ids.indexOf(selected));
    row.set_selected(index);
    row._statsLock = false;
}
