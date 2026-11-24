---
name: Progressive bar detail
overview: On a narrow panel, show each metric as an icon plus its reading, then add the name and the speed only when they fit. Stop the bar from clipping labels down to “CPU …”.
todos:
  - id: density-ladder
    content: Replace the density ladder so a narrow bar shows icon plus reading, then name, then speed
    status: completed
  - id: no-ellipsis
    content: Disable label ellipsize and size the bar against the panel side budget so text is never clipped to “CPU …”
    status: completed
  - id: readme-ladder
    content: Update the README sentence that describes the old drop order
    status: completed
isProject: false
---

# Show icon and reading first, then add detail

The bar in the screenshot is squeezed against the clock and Clutter ellipsizes the labels (`CPU …`, `RAM …`). [`_applyDensity`](lib/indicator.js) also hides icons before it hides names, which is the reverse of a small-screen floor. The width check uses the indicator’s position after the panel has already clipped it, so the full row can be treated as fitting and then get cut off.

## Ladder

In [`lib/indicator.js`](lib/indicator.js), pick the richest level whose preferred width fits. Turn ellipsize off on the name, value and speed labels (`Pango.EllipsizeMode.NONE`) so a level that does not fit is dropped instead of clipped.

From tightest to fullest:

- Icon and the reading only: percent, degrees, or `60 Hz`. No name, no speed.
- Add the name (`CPU`, `RAM`, `Disk`, `GPU`).
- Add the speed (`2.40 GHz`, used bytes, combined disk throughput, GPU MHz).

Temperature and refresh stay icon plus reading at every level, since they have no speed. Text mode keeps hiding icons. Compact mode still shows one metric until hover or until the menu is open.

If icon plus reading for every metric still exceeds the gap, drop refresh, then temperature and only then later percent metrics from the right, so at least one icon and reading remain.

## Width

Measure the gap the panel will actually give this indicator: half of `(panel width − clock natural width)`, minus siblings already in that box, minus an 8px gap. Compare that with the sum of the visible children’s natural widths after each level change, not with the post-clip allocation. Re-run on allocation and monitor changes and keep the richest level that fits so a resize can add detail back.

Update the one sentence in [`README.md`](README.md) that says the row drops speed, then icons, then temperature and refresh.
