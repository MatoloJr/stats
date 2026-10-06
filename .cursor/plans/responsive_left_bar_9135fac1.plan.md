---
name: Responsive left bar
overview: Show each top-bar metric as name, percent or degrees, and speed, separated by pipes, and place that bar in the gap between the workspace dots and the clock. The click menu wraps to the monitor so the same figures stay readable.
todos:
  - id: bar-fields
    content: Show name, percent or degrees, and speed per metric, separated by " | ", and add storage to the default bar
    status: completed
  - id: bar-fit
    content: Drop fixed min-widths and hide speed, icons, then temp/refresh when the bar would overlap the clock
    status: completed
  - id: place-left
    content: Default the indicator to the end of the left box, between the workspace dots and the clock
    status: completed
  - id: popup-fit
    content: Make the click menu wrap and cap its width to the monitor, with the same name, value, and speed on each section
    status: completed
isProject: false
---

# Responsive Stats bar on the left

The indicator is in the right box, and each label is a single short string with a large `min-width` ([stylesheet.css](stylesheet.css)). GPU therefore shows `450 MHz` instead of a percent ([lib/indicator.js](lib/indicator.js) `gpuBarText`). Storage is not in the default `bar-metrics` list. Nothing has been written under `/org/gnome/shell/extensions/stats/`, so schema defaults are what this session uses.

## Top bar contents

In [lib/indicator.js](lib/indicator.js), build each metric as its own row of actors (icon, name, primary, speed) and insert an `St.Label` with the text ` | ` between them. Drop the fixed `min-width` rules in [stylesheet.css](stylesheet.css) so the bar is only as wide as its text.

- CPU: `CPU` `34%` `2.40 GHz` (average frequency)
- Memory: `RAM` `26%` used size (`formatBytes`). There is no memory-clock sensor.
- Storage: `Disk` used percent of the first mount, plus combined read and write (`formatBytesPerSecond`)
- GPU: always a percent. `n/a` when utilisation is missing, including Intel frequency-only samples. Speed is `450 MHz`. `idle/off` stays when the card is suspended.
- Temperature: degrees only. Refresh: `60 Hz` only.

Add `storage` to the default `bar-metrics` in [schemas/org.gnome.shell.extensions.stats.gschema.xml](schemas/org.gnome.shell.extensions.stats.gschema.xml): `cpu`, `memory`, `storage`, `gpu`, `temperature`, `refresh`.

After layout, compare the bar’s preferred width with the gap between this indicator and `Main.panel`’s center box (the clock). If it would overlap, hide in order: speed labels, then icons, then temperature and refresh. The four percent metrics stay. Re-check on allocation and monitor changes so it tracks panel width.

## Place it between the workspace dots and the clock

That gap is the end of the panel’s left box. Change the schema defaults to `position` `left` and `position-index` `20` (the existing maximum; `insert_child_at_index` past the last child appends, so Stats sits after Activities and the workspace dots). Update the Left preference in [prefs.js](prefs.js) so choosing Left sets the index to `20` when it is still `0`, instead of `1`.

## Click popup

Keep the existing panel menu. In [lib/widgets/dashboard.js](lib/widgets/dashboard.js) and [stylesheet.css](stylesheet.css):

- Put name, percent or degrees, and speed on the first line of each section, then the meter and sparkline already there.
- Turn line wrap on for detail labels.
- Cap the menu width at `min(480, monitor width - 24)` using the primary monitor, with `x_expand`, so it stays on screen from the left-side button.

Recompile schemas with `make schemas`. Shell 50 will not re-import `extension.js` until a new session, so the bar moves and the new text appear after logout (or a nested session started after the change).
