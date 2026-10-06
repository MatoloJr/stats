import Gio from 'gi://Gio';

import {Sensor} from './sensor.js';

const DISPLAY_CONFIG_XML = `
<node>
  <interface name="org.gnome.Mutter.DisplayConfig">
    <method name="GetCurrentState">
      <arg type="u" direction="out" name="serial"/>
      <arg type="a((ssss)a(siiddada{sv})a{sv})" direction="out" name="monitors"/>
      <arg type="a(iiduba(ssss)a{sv})" direction="out" name="logical_monitors"/>
      <arg type="a{sv}" direction="out" name="properties"/>
    </method>
    <signal name="MonitorsChanged"/>
  </interface>
</node>`;

const DisplayConfigProxy = Gio.DBusProxy.makeProxyWrapper(DISPLAY_CONFIG_XML);

/**
 * Unpack a D-Bus variant. GJS leaves mode properties as GLib.Variant.
 *
 * @param {any} value
 */
export function unpack(value) {
    if (value != null && typeof value.unpack === 'function')
        return value.unpack();
    return value;
}

/**
 * Parse GetCurrentState monitors and logical monitors.
 * `refresh` is the nominal mode rate. When `refresh-rate-mode` is "variable"
 * the panel is VRR and this number is not the instantaneous rate.
 *
 * @param {any[]} monitors
 * @param {any[]} logicalMonitors
 */
export function parseDisplayState(monitors, logicalMonitors) {
    const parsed = [];
    for (const monitor of monitors ?? []) {
        const identity = monitor[0] ?? [];
        const modes = monitor[1] ?? [];
        const [connector, vendor, product, serial] = identity;
        let current = null;
        for (const mode of modes) {
            const props = mode[6] ?? {};
            if (!unpack(props['is-current']))
                continue;
            const rateMode = unpack(props['refresh-rate-mode']);
            current = {
                width: mode[1],
                height: mode[2],
                refresh: mode[3],
                vrr: rateMode === 'variable',
            };
            break;
        }
        parsed.push({
            connector: connector ?? 'unknown',
            vendor: vendor ?? '',
            product: product ?? '',
            serial: serial ?? '',
            width: current?.width ?? null,
            height: current?.height ?? null,
            refresh: current?.refresh ?? null,
            vrr: current?.vrr ?? false,
        });
    }

    let primaryConnector = null;
    for (const logical of logicalMonitors ?? []) {
        const primary = logical[4];
        const members = logical[5] ?? [];
        if (primary && members.length > 0) {
            primaryConnector = members[0][0];
            break;
        }
    }
    const primary = parsed.find(monitor => monitor.connector === primaryConnector) ??
        parsed.find(monitor => monitor.refresh != null) ??
        null;
    return {
        monitors: parsed,
        primaryConnector: primary?.connector ?? null,
        primaryHz: primary?.refresh ?? null,
    };
}

export class DisplaySensor extends Sensor {
    constructor() {
        super('display', 'Display');
        this._proxy = null;
        this._signalId = 0;
        this._cache = {monitors: [], primaryConnector: null, primaryHz: null};
        this._onUpdate = null;
    }

    /**
     * Subscribes to MonitorsChanged. The poller does not need to poll this.
     *
     * @param {object} context
     */
    async init(context) {
        this._onUpdate = context.requestUpdate ?? null;
        try {
            this._proxy = new DisplayConfigProxy(
                Gio.DBus.session,
                'org.gnome.Mutter.DisplayConfig',
                '/org/gnome/Mutter/DisplayConfig');
            this._signalId = this._proxy.connectSignal('MonitorsChanged', () => {
                this._refresh().catch(error => {
                    if (!this.isCancelled(error))
                        this.warnOnce('signal', error.message);
                });
            });
            await this._refresh();
        } catch (error) {
            this.warnOnce('init', `display config unavailable (${error.message})`);
        }
    }

    /**
     * @returns {Promise<object>}
     */
    /**
     * @returns {object}
     */
    current() {
        return this._cache;
    }

    async sample() {
        return this.current();
    }

    destroy() {
        if (this._proxy && this._signalId)
            this._proxy.disconnectSignal(this._signalId);
        this._signalId = 0;
        this._proxy = null;
        this._onUpdate = null;
        super.destroy();
    }

    async _refresh() {
        if (!this._proxy)
            return;
        try {
            const [, monitors, logicalMonitors] = await this._proxy.GetCurrentStateAsync();
            if (!this._proxy)
                return;
            this._cache = parseDisplayState(monitors, logicalMonitors);
            this.clearWarning('refresh');
            this._onUpdate?.();
        } catch (error) {
            if (this.isCancelled(error))
                throw error;
            this.warnOnce('refresh', `GetCurrentState failed (${error.message})`);
        }
    }
}
