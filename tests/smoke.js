import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

import {CpuSensor} from '../lib/sensors/cpu.js';
import {DisplaySensor} from '../lib/sensors/display.js';
import {GpuSensor} from '../lib/sensors/gpu/index.js';
import {MemorySensor} from '../lib/sensors/memory.js';
import {StorageSensor} from '../lib/sensors/storage.js';
import {TemperatureSensor} from '../lib/sensors/temperature.js';

const cancellable = new Gio.Cancellable();
const context = {
    cancellable,
    settings: null,
    requestUpdate() {},
};
const sensors = [
    new CpuSensor(),
    new MemorySensor(),
    new StorageSensor(),
    new TemperatureSensor(),
    new GpuSensor(),
    new DisplaySensor(),
];

for (const sensor of sensors)
    await sensor.init(context);

await new Promise(resolve => {
    GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1200, () => {
        resolve();
        return GLib.SOURCE_REMOVE;
    });
});

for (const sensor of sensors)
    await sensor.sample(context);

await new Promise(resolve => {
    GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1200, () => {
        resolve();
        return GLib.SOURCE_REMOVE;
    });
});

for (const sensor of sensors) {
    const sample = await sensor.sample(context);
    const brief = JSON.stringify(sample);
    console.log(sensor.id, brief.length > 500 ? `${brief.slice(0, 500)}…` : brief);
}

for (const sensor of sensors)
    sensor.destroy();
