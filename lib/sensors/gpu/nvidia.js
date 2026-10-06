/**
 * Parse one nvidia-smi CSV line from:
 * utilization.gpu, temperature.gpu, memory.used, memory.total, power.draw
 * Memory values are MiB. Missing fields are "N/A" or "[N/A]".
 *
 * @param {string} line
 * @returns {object|null}
 */
export function parseNvidiaCsvLine(line) {
    const parts = String(line).split(',').map(part => part.trim());
    if (parts.length < 5)
        return null;
    if (parts.every(part => part === ''))
        return null;
    const number = value => {
        const cleaned = value.replace(/^\[|\]$/g, '').trim();
        if (!cleaned || cleaned.toUpperCase() === 'N/A')
            return null;
        const parsed = Number(cleaned);
        return Number.isFinite(parsed) ? parsed : null;
    };
    const used = number(parts[2]);
    const total = number(parts[3]);
    return {
        util: number(parts[0]),
        tempC: number(parts[1]),
        vramUsed: used == null ? null : used * 1024 * 1024,
        vramTotal: total == null ? null : total * 1024 * 1024,
        powerW: number(parts[4]),
    };
}
