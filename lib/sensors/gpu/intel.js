import {clampPercent} from '../../numbers.js';

/**
* Pull the last complete JSON objects out of a streaming buffer.
* intel_gpu_top -J prints one object after another.
*
* @param {string} buffer
* @returns {{objects: object[], rest: string}}
*/
export function consumeJsonObjects(buffer) {
    const objects = [];
    let rest = buffer;
    while (rest.includes('{')) {
        const start = rest.indexOf('{');
        const end = findObjectEnd(rest, start);
        if (end < 0) {
            rest = rest.slice(start);
            break;
        }
        const slice = rest.slice(start, end + 1);
        try {
            objects.push(JSON.parse(slice));
        } catch {
            rest = rest.slice(start + 1);
            continue;
        }
        rest = rest.slice(end + 1);
    }
    return {objects, rest};
}

/**
* @param {string} text
* @param {number} start
* @returns {number}
*/
function findObjectEnd(text, start) {
    let depth = 0;
    let inString = false;
    let escape = false;
    for (let index = start; index < text.length; index++) {
        const character = text[index];
        if (inString) {
            if (escape)
                escape = false;
            else if (character === '\\')
                escape = true;
            else if (character === '"')
                inString = false;
            continue;
        }
        if (character === '"')
            inString = true;
        else if (character === '{')
            depth += 1;
        else if (character === '}') {
            depth -= 1;
            if (depth === 0)
                return index;
        }
    }
    return -1;
}

/**
* Candidate sysfs files for the current GT frequency, in preference order.
* i915 exposes these on the DRM card directory (`/sys/class/drm/cardN`), not
* on the PCI device directory. Newer layouts add gt/gt0/rps_act_freq_mhz;
* older ones only have gt_act_freq_mhz. device/ is included for other trees.
*
* @param {string} cardBase
* @returns {string[]}
*/
export function intelFrequencyPaths(cardBase) {
    return [
        `${cardBase}/gt/gt0/rps_act_freq_mhz`,
        `${cardBase}/gt_act_freq_mhz`,
        `${cardBase}/device/gt/gt0/rps_act_freq_mhz`,
        `${cardBase}/device/gt_act_freq_mhz`,
    ];
}

/**
* Engine busy percent from one intel_gpu_top JSON sample.
* Render/3D is the utilisation shown in the bar. Frequency is in MHz.
*
* @param {object} data
* @returns {{util: number|null, frequencyMhz: number|null}}
*/
export function parseIntelGpuTop(data) {
    const engines = data?.engines ?? {};
    const render = engines['Render/3D'] ??
        engines['Render/3D/0'] ??
        Object.values(engines).find(engine => Number.isFinite(engine?.busy));
    const util = Number.isFinite(render?.busy) ? clampPercent(render.busy) : null;
    const actual = data?.frequency?.actual;
    return {
        util,
        frequencyMhz: Number.isFinite(actual) ? actual : null,
    };
}
