import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';

import {StatsIndicator} from './lib/indicator.js';
import {logInfo} from './lib/log.js';
import {Poller} from './lib/poller.js';

export default class StatsExtension extends Extension {
    enable() {
        this._settings = this.getSettings();
        this._poller = new Poller(this._settings);
        this._indicator = null;
        this._placeIds = [];
        this._shieldIds = [];
        this._placeIndicator();
        this._placeIds = [
            this._settings.connect('changed::position', () => this._placeIndicator()),
            this._settings.connect('changed::position-index', () => this._placeIndicator()),
        ];
        this._bindShield();
        this._poller.start();
        this._syncPause();
        logInfo('enabled');
    }

    disable() {
        this._unbindShield();
        for (const id of this._placeIds)
            this._settings?.disconnect(id);
        this._placeIds = [];
        this._indicator?.destroy();
        this._indicator = null;
        this._poller?.destroy();
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
