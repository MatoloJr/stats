import Clutter from 'gi://Clutter';
import GObject from 'gi://GObject';
import Pango from 'gi://Pango';
import St from 'gi://St';

import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';

import {naturalWidth, sideBudget} from './panel_fit.js';
import {Meter} from './widgets/meter.js';
import {
    NETWORK_LEVELS,
    formatBytesPerSecond,
    networkText,
} from './widgets/formatters.js';

/**
 * Upload and download speeds in the right box, immediately after the clock.
 * As the gap narrows the chip drops the icon, then the words, then the upload
 * figure, so each step is strictly narrower than the one before it.
 */
export const NetworkIndicator = GObject.registerClass(
    class NetworkIndicator extends PanelMenu.Button {
        /**
         * @param {import('./poller.js').Poller} poller
         * @param {Gio.Settings} settings
         * @param {() => void} openMain
         */
        _init(poller, settings, openMain) {
            // dontCreateMenu. With a real menu, PanelMenu.Button enables a
            // click gesture that opens an empty popup of its own and registers
            // it with the panel menu manager.
            super._init(0.5, 'Network', true);
            this._settings = settings;
            this._openMain = openMain;
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

            this._startup = new St.BoxLayout({style_class: 'stats-network-startup'});
            this._startupMeter = new Meter();
            this._startup.add_child(this._startupMeter);

            // PanelMenu.ButtonBox measures and allocates its first child and
            // nothing else, so the row and the meter share one root box.
            this._root = new St.BoxLayout({style_class: 'stats-root'});
            this._root.add_child(this._row);
            this._root.add_child(this._startup);
            this.add_child(this._root);

            this._releaseId = this.connect('button-release-event', (_actor, event) => {
                if (event.get_button() !== 1)
                    return Clutter.EVENT_PROPAGATE;
                this._openMain?.();
                return Clutter.EVENT_STOP;
            });
            this._allocId = this.connect('notify::allocation', () => this._fit());
            this._settingId = settings.connect('changed::display-mode', () => this._fit());
            // Before subscribe: a poller that already has a snapshot delivers
            // it synchronously, and that live row must not be hidden again.
            this.setProgress(0);
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
        }

        /**
         * @param {number} fraction
         */
        setProgress(fraction) {
            const percent = Math.max(0, Math.min(1, fraction)) * 100;
            this._startup.visible = true;
            this._row.visible = false;
            this._startupMeter.setPercent(percent);
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

        /**
         * Use the richest layout that fits the gap beside the clock.
         */
        _fit() {
            if (this._fitting || !this._row.visible)
                return;
            const available = sideBudget(this);
            this._fitting = true;
            try {
                const levels = this._settings?.get_string('display-mode') === 'text'
                    ? NETWORK_LEVELS.filter(level => level !== 'icon-named')
                    : NETWORK_LEVELS;
                let chosen = levels[levels.length - 1];
                for (const level of levels) {
                    this._paint(level);
                    if (!(available > 0) || naturalWidth(this.container) <= available) {
                        chosen = level;
                        break;
                    }
                }
                this._paint(chosen);
            } finally {
                this._fitting = false;
            }
        }

        /**
         * @param {string} level
         */
        _paint(level) {
            this._icon.visible = level === 'icon-named';
            const text = networkText(
                level,
                formatBytesPerSecond(this._down),
                formatBytesPerSecond(this._up));
            if (this._label.text !== text)
                this._label.text = text;
        }
    });
