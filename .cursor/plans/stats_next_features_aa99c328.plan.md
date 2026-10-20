---
name: Stats next features
overview: Add a right-side network speed indicator, switch the narrow-panel floor to name plus value, and keep install progress, pressure alerts, and a one-line installer inside the existing single poller so Stats stays light.
todos:
  - id: network-indicator
    content: Add a /proc/net/dev sensor and a right-box indicator immediately after the clock
    status: in_progress
  - id: name-value-floor
    content: Change the narrow-panel floor from icon-plus-reading to name-plus-value
    status: pending
  - id: progress-cue
    content: Show a real progress fraction while Stats starts, stops, and while install.sh downloads
    status: pending
  - id: install-command
    content: Add install.sh and document the one-line curl install once a release URL exists
    status: pending
  - id: pressure-notify
    content: Notify only after a sustained CPU or memory warning, naming the top process and one next step
    status: pending
isProject: false
---

# What to build next

Stats is one GNOME Shell extension ([extension.js](extension.js)) with one `GLib` timeout in [lib/poller.js](lib/poller.js). Sensors read files and parse them in pure functions covered by `gjs -m tests/parse.test.js`. The left bar in [lib/indicator.js](lib/indicator.js) is a second panel button away from the clock. Disk throughput already uses `counterDelta` in [lib/numbers.js](lib/numbers.js). That is the pattern to copy. Nothing here should add a second timer, a always-on subprocess, or a full `/proc` walk every second.

## 1. Network speeds on the right of the clock

Add `lib/sensors/network.js`. Read `/proc/net/dev`, skip `lo`, sum receive and transmit bytes, and turn the counter delta into bytes/s the same way storage does. The first sample is `n/a`. Push `netDown` and `netUp` into the existing history rings.

Do not put this on the left bar. In [extension.js](extension.js), add a second `PanelMenu.Button` with `Main.panel.addToStatusArea(`${uuid}-network`, indicator, 0, 'right')`. Index `0` is the leftmost child of the right box, which is the slot immediately after the clock. Show `↓ 1.2 MB/s  ↑ 40 KB/s`. The left dashboard gets a Network section with the same two rates and sparklines. The click target on the right can open that same menu.

## 2. Narrow screens show the name and the value

Today detail level `0` keeps the icon and hides the name ([`_applyLevel`](lib/indicator.js)). Icons are the weak part, so invert that floor:

- Widest: icon, name, value, and speed.
- Then drop speed.
- Then drop the icon, leaving `CPU 34%`.
- If that still meets the clock, drop refresh, then temperature, then later percent metrics from the right.

Text mode already has no icons, so it only drops speed and then metrics. The right-side network chip uses the same rule: `Down 1.2 MB/s` before a bare arrow icon.

## 3. Progress while Stats starts, stops, or installs

`/proc/net/dev` has no file size, so a download percent for arbitrary apps would be invented. Use a real fraction only where the total is known.

- Turning on: the left indicator shows the existing meter style at `0`, then fills as CPU, memory, storage, and the first network baseline arrive. It switches to live text on the first complete snapshot.
- Turning off: `disable()` paints that meter back to empty, yields one frame, then destroys the buttons and the poller. The shell will not keep drawing after `disable()` returns, so this is a short cue, not a multi-second animation.
- Installing: the terminal command in section 4 prints a four-step bar (download, unpack, schemas, enable). That is the only place a download has a known length.

## 4. One command to install

[metadata.json](metadata.json) still points at `https://example.invalid/stats`, so there is no public URL yet. Add `install.sh` that downloads a release zip, unpacks it to `~/.local/share/gnome-shell/extensions/stats@local`, compiles schemas, and runs the same enable logic as `make enable`. The README line, once the zip is hosted, is:

`curl -fsSL <release-url>/install.sh | bash`

The script must refuse to run as root, leave the user's other extensions in `enabled-extensions` alone, and print that a logout is required on Wayland. `make install` stays the path for this checkout.

## 5. A warning when one program is making the machine slow

Only look at processes when the machine is already hot. If CPU or memory stays at or above the existing warning threshold for three polls, read `/proc/<pid>/stat` and `statm` once and keep the single largest comm. Compare it with a small ring of that comm's recent share (cap the map, drop the rest). Notify through `Main.notify` when that comm is far above its own recent share, then stay quiet for several minutes.

The body names the program, the figure, and one action: quit it, or open System Monitor when that desktop id exists. No notification for a sleeping GPU, and no scan while the screen is locked. A schema switch defaults this off until the wording is proven, so a noisy false alarm cannot ship as the default.

## 6. Stay light

Network is one file read on the poller that already exists. Process accounting runs only after the threshold streak, not every tick. `intel_gpu_top` and `nvidia-smi` stay optional and still stop when the card is suspended or the session is paused. History stays in the fixed rings already sized by `history-seconds`. The network button is destroyed in `disable()` with the left indicator so a reload cannot leak a status-area role.
