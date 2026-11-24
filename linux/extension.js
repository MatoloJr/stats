import GLib from 'gi://GLib';
import Shell from 'gi://Shell';

import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

import {StatsIndicator} from './lib/indicator.js';
import {logInfo} from './lib/log.js';
import {NetworkIndicator} from './lib/network_indicator.js';
import {Poller} from './lib/poller.js';
import {pressureCopy} from './lib/pressure.js';

const SYSTEM_MONITOR_IDS = [
    'org.gnome.SystemMonitor.desktop',
    'gnome-system-monitor.desktop',
];

export default class StatsExtension extends Extension {
    enable() {
        this._settings = this.getSettings();
        this._poller = new Poller(this._settings);
        this._indicator = null;
        this._network = null;
        this._placeIds = [];
        this._shieldIds = [];
        this._rightBox = null;
        this._rightBoxId = 0;
        this._pinId = 0;
        this._placeIndicator();
        this._placeNetwork();
        this._poller.setPressureHandler(notice => this._notifyPressure(notice));
        this._placeIds = [
            this._settings.connect('changed::position', () => this._placeIndicator()),
            this._settings.connect('changed::position-index', () => this._placeIndicator()),
        ];
        this._bindShield();
        this._poller.start();
        this._syncPause();
        logInfo('enabled');
    }

    /**
     * Everything goes synchronously. Deferring the destroy to an idle leaves
     * the status-area roles taken and addToStatusArea throws on a conflict,
     * so a disable followed by an enable in the same main-loop iteration
     * would fail to come back.
     */
    disable() {
        this._unbindShield();
        for (const id of this._placeIds)
            this._settings?.disconnect(id);
        this._placeIds = [];
        this._unpinNetwork();
        this._poller?.pause();
        this._indicator?.destroy();
        this._network?.destroy();
        this._poller?.destroy();
        this._indicator = null;
        this._network = null;
        this._poller = null;
        this._settings = null;
        logInfo('disabled');
    }

    /**
     * Destroying the button removes it from the panel status area. The new
     * button is added only after that, so other indicators keep their order.
     */
    _placeIndicator() {
        const position = this._settings.get_string('position');
        const index = this._settings.get_int('position-index');
        this._indicator?.destroy();
        this._indicator = null;
        this._indicator = new StatsIndicator(
            this._poller,
            this._settings,
            () => this.openPreferences());
        // GNOME 50 defaults an omitted index to 0 (`position ??= 0` in
        // js/ui/panel.js). Always pass the index so Shell 46 and 50 agree.
        Main.panel.addToStatusArea(this.uuid, this._indicator, index, position);
    }

    /**
     * Index 0 in the right box is the leftmost status icon, directly after the clock.
     */
    _placeNetwork() {
        this._network?.destroy();
        this._network = null;
        this._network = new NetworkIndicator(
            this._poller,
            this._settings,
            () => this._indicator?.menu.toggle());
        Main.panel.addToStatusArea(`${this.uuid}-network`, this._network, 0, 'right');
        this._pinNetwork();
    }

    /**
     * Every other extension also asks for index 0 of the right box and the
     * one that loads last wins, so the chip drifts away from the clock. Watch
     * the box and move it back to the front.
     */
    _pinNetwork() {
        const box = Main.panel._rightBox;
        if (!box?.connect)
            return;
        if (!this._rightBoxId) {
            this._rightBox = box;
            this._rightBoxId = box.connect('child-added', () => this._queuePin());
        }
        this._queuePin();
    }

    _queuePin() {
        if (this._pinId)
            return;
        // An idle, because moving the child re-emits child-added and because
        // an extension enabled after this one has not been added yet.
        this._pinId = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
            this._moveNetworkToFront();
            this._pinId = 0;
            return GLib.SOURCE_REMOVE;
        });
        GLib.Source.set_name_by_id(this._pinId, '[stats] pin network chip');
    }

    _moveNetworkToFront() {
        const box = this._rightBox;
        const container = this._network?.container;
        if (!box || !container || container.get_parent() !== box)
            return;
        if (box.get_children()[0] === container)
            return;
        box.set_child_at_index(container, 0);
    }

    _unpinNetwork() {
        if (this._pinId) {
            GLib.source_remove(this._pinId);
            this._pinId = 0;
        }
        if (this._rightBoxId && this._rightBox) {
            try {
                this._rightBox.disconnect(this._rightBoxId);
            } catch {
                // The panel can already be gone while the session closes.
            }
        }
        this._rightBoxId = 0;
        this._rightBox = null;
    }

    _notifyPressure(notice) {
        const appSystem = Shell.AppSystem.get_default();
        const offerMonitor = SYSTEM_MONITOR_IDS.some(id => appSystem.lookup_app(id));
        const text = pressureCopy(notice, notice.reason, notice.baseline, offerMonitor);
        Main.notify(text.title, text.body);
    }

    _bindShield() {
        const shield = Main.screenShield;
        if (!shield?.connect)
            return;
        // ScreenShield emits locked-changed and active-changed on GNOME 46
        // through 50. `active` is the idle blank; `locked` is the lock screen.
        for (const signal of ['locked-changed', 'active-changed']) {
            try {
                const id = shield.connect(signal, () => this._syncPause());
                this._shieldIds.push([shield, id]);
            } catch {
                logInfo(`screen shield signal ${signal} is unavailable`);
            }
        }
        const settingsId = this._settings.connect(
            'changed::pause-when-locked',
            () => this._syncPause());
        this._shieldIds.push([this._settings, settingsId]);
    }

    _unbindShield() {
        for (const [target, id] of this._shieldIds) {
            try {
                target.disconnect(id);
            } catch {
                // The emitter can already be gone while the session is closing.
            }
        }
        this._shieldIds = [];
    }

    _syncPause() {
        if (!this._poller || !this._settings)
            return;
        if (!this._settings.get_boolean('pause-when-locked')) {
            this._poller.resume();
            return;
        }
        const shield = Main.screenShield;
        if (shield?.locked || shield?.active)
            this._poller.pause();
        else
            this._poller.resume();
    }
}
