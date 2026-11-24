import * as Main from 'resource:///org/gnome/shell/ui/main.js';

/** Gap kept between a side indicator and the clock. */
const CLOCK_GAP = 8;

/**
 * @param {Clutter.Actor} actor
 * @returns {number}
 */
export function naturalWidth(actor) {
    const preferred = actor?.get_preferred_width?.(-1);
    if (Array.isArray(preferred))
        return preferred[1] ?? preferred[0] ?? 0;
    return preferred || 0;
}

/**
 * Panel.addToStatusArea inserts `indicator.container`, not the indicator, so
 * the other status icons are siblings of that container. Reading
 * `indicator.get_parent()` finds only the St.Bin wrapper and always sums to
 * zero, which hands the indicator the whole panel side.
 *
 * @param {St.Widget} indicator
 * @returns {number}
 */
export function siblingWidth(indicator) {
    const container = indicator?.container;
    const parent = container?.get_parent?.();
    if (!parent?.get_children)
        return 0;
    let total = 0;
    for (const child of parent.get_children()) {
        if (child !== container && child.visible)
            total += naturalWidth(child);
    }
    return total;
}

/**
 * Shell centres the clock on the work area rather than the monitor, so a dock
 * that reserves space shifts both sides. This mirrors the offset in
 * Panel.vfunc_allocate (js/ui/panel.js).
 *
 * @returns {number}
 */
function centerOffset() {
    const layout = Main.layoutManager;
    const monitor = layout?.findMonitorForActor?.(Main.panel);
    if (!monitor)
        return 0;
    const workArea = layout.getWorkAreaForMonitor(monitor.index);
    if (!workArea)
        return 0;
    return 2 * (workArea.x - monitor.x) + workArea.width - monitor.width;
}

/**
 * The width the panel will actually give this indicator: Shell's own side
 * budget, minus the status icons already in the same box and the clock gap.
 * Zero means the panel is not laid out yet and the caller should show its
 * richest level.
 *
 * @param {St.Widget} indicator
 * @returns {number}
 */
export function sideBudget(indicator) {
    const panel = Main.panel;
    const center = panel?._centerBox;
    if (!panel || !center)
        return 0;
    const panelWidth = panel.width || Main.layoutManager?.primaryMonitor?.width || 0;
    if (!(panelWidth > 0))
        return 0;
    const side = Math.max(
        0,
        Math.floor((panelWidth - naturalWidth(center) + centerOffset()) / 2));
    return Math.max(0, side - siblingWidth(indicator) - CLOCK_GAP);
}
