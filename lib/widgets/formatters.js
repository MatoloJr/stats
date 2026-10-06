/**
 * @param {number|null|undefined} bytes
 * @returns {string}
 */
export function formatBytes(bytes) {
    if (bytes == null || !Number.isFinite(bytes) || bytes < 0)
        return 'n/a';
    const units = ['B', 'KB', 'MB', 'GB', 'TB'];
    let value = bytes;
    let index = 0;
    while (value >= 1024 && index < units.length - 1) {
        value /= 1024;
        index += 1;
    }
    const digits = index === 0 || value >= 100 ? 0 : value >= 10 ? 1 : 2;
    return `${value.toFixed(digits)} ${units[index]}`;
}

/**
 * @param {number|null|undefined} bytesPerSecond
 * @returns {string}
 */
export function formatBytesPerSecond(bytesPerSecond) {
    if (bytesPerSecond == null || !Number.isFinite(bytesPerSecond))
        return 'n/a';
    return `${formatBytes(bytesPerSecond)}/s`;
}

/**
 * @param {number|null|undefined} percent
 * @returns {string}
 */
export function formatPercent(percent) {
    if (percent == null || !Number.isFinite(percent))
        return 'n/a';
    return `${Math.round(percent)}%`;
}

/**
 * @param {number|null|undefined} celsius
 * @param {'c'|'f'} unit
 * @returns {string}
 */
export function formatTemperature(celsius, unit) {
    if (celsius == null || !Number.isFinite(celsius))
        return 'n/a';
    const value = unit === 'f' ? (celsius * 9) / 5 + 32 : celsius;
    const suffix = unit === 'f' ? '°F' : '°C';
    return `${Math.round(value)}${suffix}`;
}

/**
 * @param {number|null|undefined} mhz
 * @returns {string}
 */
export function formatGigahertz(mhz) {
    if (mhz == null || !Number.isFinite(mhz))
        return 'n/a';
    return `${(mhz / 1000).toFixed(2)} GHz`;
}

/**
 * @param {number|null|undefined} mhz
 * @returns {string}
 */
export function formatMegahertz(mhz) {
    if (mhz == null || !Number.isFinite(mhz))
        return 'n/a';
    return `${Math.round(mhz)} MHz`;
}

/**
 * Nominal mode refresh, rounded to an integer when the mode is within 0.05 Hz
 * (1366x768@59.9985 is shown as 60 Hz). This is not an instantaneous VRR rate.
 *
 * @param {number|null|undefined} hz
 * @returns {string}
 */
export function formatRefreshHz(hz) {
    if (hz == null || !Number.isFinite(hz))
        return 'n/a';
    const rounded = Math.round(hz);
    if (Math.abs(hz - rounded) < 0.05)
        return `${rounded} Hz`;
    return `${hz.toFixed(1)} Hz`;
}

/**
 * @param {number|null|undefined} watts
 * @returns {string}
 */
export function formatWatts(watts) {
    if (watts == null || !Number.isFinite(watts))
        return 'n/a';
    const digits = watts >= 100 ? 0 : 1;
    return `${watts.toFixed(digits)} W`;
}

/**
 * @param {number|null|undefined} value
 * @param {number} warning
 * @param {number} critical
 * @returns {'normal'|'warning'|'critical'}
 */
export function thresholdLevel(value, warning, critical) {
    if (value == null || !Number.isFinite(value))
        return 'normal';
    if (value >= critical)
        return 'critical';
    if (value >= warning)
        return 'warning';
    return 'normal';
}
