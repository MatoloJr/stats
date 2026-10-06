import Gio from 'gi://Gio';
import System from 'system';

import {parseProcStat, parseFrequencyKhz, usagePercent} from '../lib/sensors/cpu.js';
import {parseDisplayState, unpack} from '../lib/sensors/display.js';
import {isSuspended, parseVendor, selectCard} from '../lib/sensors/gpu/detect.js';
import {consumeJsonObjects, parseIntelGpuTop} from '../lib/sensors/gpu/intel.js';
import {parseNvidiaCsvLine} from '../lib/sensors/gpu/nvidia.js';
import {parseMeminfo} from '../lib/sensors/memory.js';
import {
    isVirtualDevice,
    parseDiskstats,
    partitionParent,
    physicalDevices,
    sumSectors,
    throughputFromDeltas,
} from '../lib/sensors/storage.js';
import {
    buildTemperatureCatalog,
    chooseHeadline,
    celsiusFromMilli,
} from '../lib/sensors/temperature.js';
import {
    formatBytes,
    formatPercent,
    formatRefreshHz,
    formatTemperature,
} from '../lib/widgets/formatters.js';

let failed = 0;

/**
 * @param {boolean} condition
 * @param {string} message
 */
function assert(condition, message) {
    if (!condition) {
        failed += 1;
        console.error(`FAIL ${message}`);
    }
}

/**
 * @param {number} actual
 * @param {number} expected
 * @param {string} message
 */
function assertClose(actual, expected, message) {
    assert(Math.abs(actual - expected) < 0.001, `${message}: ${actual} vs ${expected}`);
}

/**
 * @param {string} name
 * @returns {string}
 */
function readFixture(name) {
    const file = Gio.File.new_for_uri(import.meta.url)
        .get_parent()
        .get_child('fixtures')
        .get_child(name);
    const [, bytes] = file.load_contents(null);
    return new TextDecoder().decode(bytes);
}

const statA = parseProcStat(readFixture('proc_stat_a.txt'));
const statB = parseProcStat(readFixture('proc_stat_b.txt'));
const statWrap = parseProcStat(readFixture('proc_stat_wrap.txt'));
assert(statA.cores.length === 2, 'two cores in the first stat snapshot');
assert(usagePercent(null, statA.aggregate) === null, 'first CPU sample has no percent');
assertClose(usagePercent(statA.aggregate, statB.aggregate), 50, 'aggregate CPU');
assertClose(usagePercent(statA.cores[0], statB.cores[0]), 80, 'cpu0');
assertClose(usagePercent(statA.cores[1], statB.cores[1]), 20, 'cpu1');
assert(usagePercent(statB.aggregate, statWrap.aggregate) === null, 'CPU counter wrap');
assertClose(parseFrequencyKhz('2400000\n'), 2400, 'frequency kHz to MHz');
assert(parseFrequencyKhz('nope') === null, 'bad frequency');

const memory = parseMeminfo(readFixture('meminfo.txt'));
assert(memory.used === (8000000 - 3000000) * 1024, 'used is total minus available');
assert(memory.used !== (8000000 - 1000000) * 1024, 'used does not use MemFree');
assertClose(memory.percent, 62.5, 'memory percent');
assert(memory.cached === 1500000 * 1024, 'cached');
assert(memory.swapUsed === 500000 * 1024, 'swap used');
assert(parseMeminfo('MemTotal: 1 kB\n') === null, 'incomplete meminfo');

const diskA = physicalDevices(parseDiskstats(readFixture('diskstats_a.txt')));
const diskB = physicalDevices(parseDiskstats(readFixture('diskstats_b.txt')));
assert(diskA.map(device => device.name).join(',') === 'sda,nvme0n1', 'physical disks only');
assert(isVirtualDevice('loop0') && isVirtualDevice('dm-0') && isVirtualDevice('ram0'), 'virtual devices');
assert(partitionParent('nvme0n1') === null, 'nvme namespace is a disk');
assert(partitionParent('nvme0n1p1') === 'nvme0n1', 'nvme partition parent');
assert(partitionParent('sda1') === 'sda', 'scsi partition parent');
assert(partitionParent('mmcblk0p1') === 'mmcblk0', 'mmc partition parent');
const throughput = throughputFromDeltas(sumSectors(diskA), sumSectors(diskB), 1);
assert(throughput.readBps === 1100 * 512, 'read throughput');
assert(throughput.writeBps === 600 * 512, 'write throughput');
assert(throughputFromDeltas(sumSectors(diskB), sumSectors(diskA), 1) === null, 'disk counter wrap');

const chips = [
    {
        id: 'hwmon3',
        name: 'coretemp',
        temps: [
            {label: 'Core 0', millidegree: 53000, index: 2},
            {label: 'Package id 0', millidegree: 54000, index: 1},
        ],
    },
    {
        id: 'hwmon4',
        name: 'nvme',
        temps: [{label: 'Composite', millidegree: 42000, index: 1}],
    },
    {
        id: 'hwmon5',
        name: 'amdgpu',
        suspended: true,
        temps: [],
    },
];
const zones = [{id: 'thermal_zone0', type: 'x86_pkg_temp', millidegree: 61000}];
const catalog = buildTemperatureCatalog(chips, zones);
const headline = chooseHeadline(catalog, '');
assert(headline.id === 'hwmon3:Package id 0', 'package temperature is the headline');
assertClose(headline.celsius, 54, 'package celsius');
assert(catalog.find(sensor => sensor.chip === 'amdgpu').state === 'off', 'suspended GPU is idle');
assert(catalog.some(sensor => sensor.chip === 'x86_pkg_temp'), 'thermal zone is included');
const k10 = buildTemperatureCatalog([{
    id: 'hwmon1',
    name: 'k10temp',
    temps: [
        {label: 'Tdie', millidegree: 48000, index: 2},
        {label: 'Tctl', millidegree: 49000, index: 1},
    ],
}], []);
assert(chooseHeadline(k10, '').label === 'Tctl', 'Tctl beats Tdie');
assert(chooseHeadline(catalog, 'hwmon4:Composite').label === 'Composite', 'explicit headline');
assertClose(celsiusFromMilli(45123), 45.123, 'millidegree conversion');

const nvidiaLines = readFixture('nvidia.txt').trim().split('\n');
const nvidia = parseNvidiaCsvLine(nvidiaLines[0]);
assertClose(nvidia.util, 12, 'nvidia util');
assertClose(nvidia.tempC, 54, 'nvidia temp');
assert(nvidia.vramUsed === 512 * 1024 * 1024, 'nvidia used MiB');
assert(nvidia.vramTotal === 8192 * 1024 * 1024, 'nvidia total MiB');
assertClose(nvidia.powerW, 23.5, 'nvidia power');
const nvidiaMissing = parseNvidiaCsvLine(nvidiaLines[1]);
assert(nvidiaMissing.util === null && nvidiaMissing.powerW === null, 'nvidia N/A fields');
assert(nvidiaMissing.vramTotal === 2048 * 1024 * 1024, 'nvidia total beside N/A');

const intel = parseIntelGpuTop(JSON.parse(readFixture('intel_gpu_top.json')));
assertClose(intel.util, 12.5, 'intel render busy');
assert(intel.frequencyMhz === 450, 'intel frequency');
const streamed = `noise\n${readFixture('intel_gpu_top.json')}\n{"engines":{"Render/3D":{"busy":4}}}`;
const first = consumeJsonObjects(streamed.slice(0, 40));
assert(first.objects.length === 0, 'incomplete JSON stays buffered');
const done = consumeJsonObjects(first.rest + streamed.slice(40));
assert(done.objects.length === 2, 'two intel JSON objects');
assertClose(parseIntelGpuTop(done.objects[1]).util, 4, 'second intel sample');

assert(parseVendor('0x10de\n') === 'nvidia', 'nvidia vendor');
assert(parseVendor('0x1002') === 'amd', 'amd vendor');
assert(parseVendor('0x8086') === 'intel', 'intel vendor');
assert(parseVendor('0x1234') === null, 'unknown vendor');
assert(isSuspended('suspended\n') && !isSuspended('active'), 'runtime status');
const cards = [
    {id: 'card0', vendor: 'intel', state: 'active'},
    {id: 'card1', vendor: 'nvidia', state: 'off'},
];
assert(selectCard(cards, '').id === 'card0', 'skip suspended discrete GPU');
assert(selectCard(cards, 'card1').id === 'card1', 'explicit GPU id');
cards[1].state = 'active';
assert(selectCard(cards, '').id === 'card1', 'prefer active discrete GPU');

const monitors = [[
    ['eDP-1', 'BOE', 'panel', '0'],
    [
        ['1366x768@59.999', 1366, 768, 59.9985, 1, [1], {'is-current': {unpack: () => true}}],
        ['1366x768@40', 1366, 768, 40, 1, [1], {'refresh-rate-mode': {unpack: () => 'variable'}}],
    ],
    {},
]];
const logical = [[0, 0, 1, 0, true, [['eDP-1', 'BOE', 'panel', '0']], {}]];
const display = parseDisplayState(monitors, logical);
assert(display.primaryConnector === 'eDP-1', 'primary connector');
assertClose(display.primaryHz, 59.9985, 'nominal refresh');
assert(display.monitors[0].vrr === false, 'current mode is fixed');
assert(formatRefreshHz(display.primaryHz) === '60 Hz', '59.9985 displays as 60 Hz');
assert(formatRefreshHz(59.94) === '59.9 Hz', '59.94 keeps one decimal');
assert(unpack({unpack: () => 7}) === 7, 'variant unpack');

const vrrMonitors = [[
    ['DP-1', 'A', 'B', 'C'],
    [['2560x1440', 2560, 1440, 144, 1, [1], {'is-current': true, 'refresh-rate-mode': 'variable'}]],
    {},
]];
assert(parseDisplayState(vrrMonitors, []).monitors[0].vrr === true, 'VRR mode flag');

assert(formatBytes(1536) === '1.5 KB', 'format bytes');
assert(formatBytes(0) === '0 B', 'zero bytes');
assert(formatPercent(null) === 'n/a', 'missing percent');
assert(formatTemperature(0, 'f') === '32°F', 'freezing in fahrenheit');
assert(formatTemperature(54.2, 'c') === '54°C', 'celsius rounding');

if (failed > 0) {
    console.error(`${failed} test(s) failed`);
    System.exit(1);
}

console.log('all parser tests passed');
