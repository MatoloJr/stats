// Dashboard logic tests. No GNOME imports: run with `node tests/ui.test.js`
// or `gjs -m tests/ui.test.js`. Throws (non-zero exit) on the first failure.

import {displayName, groupProcesses, isProtected, levels, parseStatus, status, tileModels} from '../lib/overview.js';

let passed = 0;
function check(name, condition, detail = '') {
    if (!condition)
        throw new Error(`FAIL ${name}${detail ? `: ${detail}` : ''}`);
    passed += 1;
}

const thresholds = {
    'cpu-warning': 70, 'cpu-critical': 90,
    'memory-warning': 70, 'memory-critical': 90,
    'storage-warning': 70, 'storage-critical': 90,
    'gpu-warning': 70, 'gpu-critical': 90,
    'temp-warning': 75, 'temp-critical': 90,
};
const get = key => thresholds[key];

const snapshot = {
    cpu: {usage: 34, cores: [{}, {}, {}, {}]},
    memory: {percent: 82, used: 12.9 * 2 ** 30, total: 15.7 * 2 ** 30},
    storage: {mounts: [{path: '/', percent: 55, used: 200 * 2 ** 30, size: 400 * 2 ** 30}], readBps: 1000, writeBps: 3000},
    gpu: {selected: {id: 'card1', vendor: 'AMD', util: 12, vramUsed: 2 ** 30, vramTotal: 4 * 2 ** 30}},
    temperature: {headline: {label: 'CPU package', celsius: 61}},
    network: {downBps: 1200000, upBps: 40000},
};

// Protected programs never get a Close button.
for (const name of ['gnome-shell', 'Xwayland', 'gsd-power', 'pipewire', 'systemd', 'dbus-daemon', 'gvfsd-trash', 'xdg-desktop-por', ''])
    check(`protected ${name || '(empty)'}`, isProtected(name));
for (const name of ['firefox', 'chrome', 'code', 'gnome-calculato', 'node'])
    check(`closable ${name}`, !isProtected(name));

check('display name known', displayName('firefox') === 'Firefox');
check('display name capitalised', displayName('blender') === 'Blender');
check('display name empty', displayName('') === 'Unknown');

// /proc/<pid>/status parsing.
const parsed = parseStatus('Name:\tfirefox\nUmask:\t0002\nState:\tS (sleeping)\nPid:\t4242\nUid:\t1000\t1000\t1000\t1000\nVmRSS:\t  204800 kB\n');
check('status name', parsed.name === 'firefox');
check('status uid', parsed.uid === 1000);
check('status pid', parsed.pid === 4242);
check('status rss', parsed.rss === 204800 * 1024);
check('status kernel thread has no rss', parseStatus('Name:\tkworker/0:1\nPid:\t9\nUid:\t0\t0\t0\t0\n').rss === null);
check('status empty', parseStatus('') === null && parseStatus(null) === null);

// Grouping like "Firefox ×3".
const groups = groupProcesses([
    {pid: 1, comm: 'firefox', rss: 100, cpu: 2},
    {pid: 2, comm: 'firefox', rss: 300, cpu: null},
    {pid: 3, comm: 'code', rss: 250, cpu: 5},
    {pid: 4, comm: 'firefox', rss: 50, cpu: 1},
    {pid: 5, comm: 'gnome-shell', rss: 900, cpu: 3},
    {pid: 6, comm: '', rss: 999, cpu: 0},
]);
check('groups sorted by memory', groups.map(g => g.comm).join(',') === 'gnome-shell,firefox,code', groups.map(g => g.comm).join(','));
const firefox = groups.find(g => g.comm === 'firefox');
check('group count', firefox.count === 3);
check('group rss summed', firefox.rss === 450);
check('group cpu skips unknown', firefox.cpu === 3);
check('group pids', firefox.pids.join(',') === '1,2,4');
check('group protected flag', groups[0].protected && !firefox.protected);
check('group cpu null when unknown', groupProcesses([{pid: 1, comm: 'x', rss: 1, cpu: null}])[0].cpu === null);

// Levels and the status chip.
const level = levels(snapshot, get);
check('memory warning', level.memory === 'warning');
check('cpu normal', level.cpu === 'normal');
check('chip names the worst', status(snapshot, get).text === 'RAM high');
check('chip level', status(snapshot, get).level === 'warning');
const critical = {...snapshot, storage: {mounts: [{path: '/home', percent: 97}]}};
check('critical beats warning', status(critical, get).text === '/home almost full', status(critical, get).text);
check('all good', status({cpu: {usage: 5}, memory: {percent: 20}}, get).text === 'All good');
check('empty snapshot is all good', status({}, get).level === 'normal');
const sleepingGpu = {gpu: {selected: {state: 'off', util: 99}}};
check('sleeping gpu is not busy', levels(sleepingGpu, get).gpu === 'normal');

// Tiles.
const tiles = tileModels(snapshot, get, 'celsius', 'Firefox');
check('six tiles', tiles.length === 6);
check('tile order', tiles.map(t => t.id).join(',') === 'cpu,memory,storage,gpu,temperature,network');
check('cpu busiest', tiles[0].detail === 'Busiest: Firefox');
check('memory level', tiles[1].level === 'warning');
check('disk label has mount', tiles[2].label === 'Disk /');
check('disk detail has free space', tiles[2].detail.includes('free'), tiles[2].detail);
check('gpu vram detail', tiles[3].detail.includes('of'), tiles[3].detail);
check('network arrows', tiles[5].value.startsWith('↓') && tiles[5].detail.startsWith('↑'));
check('cpu cores without busiest', tileModels(snapshot, get, 'celsius', null)[0].detail === '4 cores');
const empty = tileModels({}, get, 'celsius', null);
check('n/a tiles do not throw', empty.length === 6 && empty[1].detail === 'n/a' && empty[4].detail === 'No sensor');
check('sleeping gpu tile', tileModels(sleepingGpu, get, 'celsius')[3].value === 'idle');

const done = `ok: ${passed} dashboard checks passed`;
if (typeof print === 'function')
    print(done); // gjs
else
    console.log(done);
