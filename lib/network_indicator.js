import Clutter from 'gi://Clutter';
import GObject from 'gi://GObject';
import Pango from 'gi://Pango';
import St from 'gi://St';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';

import {Meter} from './widgets/meter.js';
import {formatBytesPerSecond} from './widgets/formatters.js';

const CLOCK_GAP = 8;

/**
 * Upload and download speeds in the right box, immediately after the clock.
 * A narrow gap drops the icon and keeps the words Down and Up.
 */
export const NetworkIndicator = GObject.registerClass(
    class NetworkIndicator extends PanelMenu.Button {
        /**
         * @param {import('./poller.js').Poller} poller
         * @param {Gio.Settings} settings
         * @param {() => void} openMain
         */
        _init(poller, settings, openMain) {
            super._init(1, 'Network', false);
            this._settings = settings;
            this._openMain = openMain;
            this._wide = true;
            this._down = null;
            this._up = null;
            this._fitting = false;
            this.add_style_class_name('stats-network');

            this._row = new St.BoxLayout({style_class: 'stats-network-row'});
            this._icon = new St.Icon({
                icon_name: 'network-transmit-receive-symbolic',
                style_class: 'system-status-icon stats-icon',
            });
            this._label = new St.Label({
                text: '',
                style_class: 'stats-value',
                y_align: Clutter.ActorAlign.CENTER,
            });
            this._label.clutter_text.ellipsize = Pango.EllipsizeMode.NONE;
            this._row.add_child(this._icon);
            this._row.add_child(this._label);
            this.add_child(this._row);

            this._startup = new Meter();
            this._startup.visible = false;
            this.add_child(this._startup);

            this._releaseId = this.connect('button-release-event', (_actor, event) => {
                if (event.get_button() !== 1)
                    return Clutter.EVENT_PROPAGATE;
                this._openMain?.();
                return Clutter.EVENT_STOP;
            });
            this._allocId = this.connect('notify::allocation', () => this._fit());
            this._settingId = settings.connect('changed::display-mode', () => this._fit());
            this._unsubscribe = poller.subscribe(snapshot => {
                this._down = snapshot.network?.downBps ?? null;
                this._up = snapshot.network?.upBps ?? null;
                const progress = snapshot.progress ?? 0;
                if (progress < 1) {
                    this.setProgress(progress);
                    return;
                }
                this._showLive();
                this._fit();
            });
            this.connect('destroy', () => this._onDestroy());
            this.setProgress(0);
        }

        /**
         * @param {number} fraction
         */
        setProgress(fraction) {
            const percent = Math.max(0, Math.min(1, fraction)) * 100;
            this._startup.visible = true;
            this._row.visible = false;
            this._startup.setPercent(percent);
        }

        _showLive() {
            this._startup.visible = false;
            this._row.visible = true;
        }

        _onDestroy() {
            if (this._releaseId) {
                this.disconnect(this._releaseId);
                this._releaseId = 0;
            }
            if (this._allocId) {
                this.disconnect(this._allocId);
                this._allocId = 0;
            }
            if (this._settingId && this._settings) {
                this._settings.disconnect(this._settingId);
                this._settingId = 0;
            }
            this._unsubscribe?.();
            this._unsubscribe = null;
            this._settings = null;
            this._openMain = null;
        }

        _fit() {
            if (this._fitting || !this._row.visible)
                return;
            const available = sideBudget(this);
            this._fitting = true;
            try {
                this._wide = true;
                this._paint();
                if (available > 0 && naturalWidth(this) > available) {
                    this._wide = false;
                    this._paint();
                }
            } finally {
                this._fitting = false;
            }
        }

        _paint() {
            const down = formatBytesPerSecond(this._down);
            const up = formatBytesPerSecond(this._up);
            const named = !this._wide || this._settings?.get_string('display-mode') === 'text';
            this._icon.visible = !named;
            const text = named
                ? `Down ${down}  Up ${up}`
                : `↓ ${down}  ↑ ${up}`;
            if (this._label.text !== text)
                this._label.text = text;
        }
    });

/**
 * @param {Clutter.Actor} actor
 * @returns {number}
 */
function naturalWidth(actor) {
    const preferred = actor.get_preferred_width(-1);
    if (Array.isArray(preferred))
        return preferred[1] ?? preferred[0] ?? 0;
    return preferred || 0;
}

/**
 * Half of the panel beside the clock, minus the other actors in this box.
 *
 * @param {St.Widget} indicator
 * @returns {number}
 */
function sideBudget(indicator) {
    const panel = Main.panel;
    const center = panel?._centerBox;
    if (!panel || !center)
        return 0;
    const panelWidth = panel.width || Main.layoutManager?.primaryMonitor?.width || 0;
    if (!(panelWidth > 0))
        return 0;
    const side = Math.floor((panelWidth - naturalWidth(center)) / 2);
    const parent = indicator.get_parent();
    let siblings = 0;
    if (parent?.get_children) {
        for (const child of parent.get_children()) {
            if (child !== indicator && child.visible)
                siblings += naturalWidth(child);
        }
    }
    return Math.max(0, side - siblings - CLOCK_GAP);
}
