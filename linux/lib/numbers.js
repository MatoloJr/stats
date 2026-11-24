/**
 * Clamp a percentage into the closed range 0..100.
 * Non-finite inputs become null so the UI can show "n/a".
 *
 * @param {number} value
 * @returns {number|null}
 */
export function clampPercent(value) {
    if (!Number.isFinite(value))
        return null;
    return Math.min(100, Math.max(0, value));
}

/**
 * Difference of two monotonic counters. A negative delta is a wrap or reset
 * and returns null so the caller can keep the new sample as the baseline.
 *
 * @param {number} previous
 * @param {number} next
 * @returns {number|null}
 */
export function counterDelta(previous, next) {
    if (!Number.isFinite(previous) || !Number.isFinite(next))
        return null;
    const delta = next - previous;
    if (delta < 0)
        return null;
    return delta;
}
