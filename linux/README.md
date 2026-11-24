# Stats

Stats is a GNOME Shell extension that shows real-time system usage in the top bar: CPU, memory, storage, GPU, temperature and the nominal refresh rate of each monitor. Click the indicator for a dashboard; it scrolls and clicking a section heading folds that section away. The display name, preferences title and popup header are **Stats**.

The UUID is `stats@local`, set only in [`metadata.json`](metadata.json). The install directory must use that same name. The GSettings schema id is `org.gnome.shell.extensions.stats` (`lib/constants.js`).

Shell versions 46 through 50 are listed in `metadata.json`. The extension is an ES module (GNOME 45 and newer). It does not use X11 tools. Optional programs (`nvidia-smi`, `intel_gpu_top`, `gnome-system-monitor`) are used only when they are installed; missing hardware or tools show `n/a`.

## Install

For users (GNOME Shell 46 to 50, no root):

```bash
curl -fsSL https://matolojr.github.io/stats/install.sh | bash
```

The script checks GNOME Shell and the tools it needs first, and prints the package to install when one is missing. It downloads the latest release zip and verifies it against `SHA256SUMS`. It unpacks into a staging folder that replaces the old version in one step, then enables the extension. `STATS_VERSION=0.2.0` pins a release. `... | bash -s -- --uninstall` removes Stats. On Wayland you still log out and back in once before the indicator appears.

From this checkout (inside `linux/`):

```bash
../install.sh     # links this folder, compiles the schema, enables; same progress bar
# or
make install
make enable
```

`make install` symlinks this directory to `~/.local/share/gnome-shell/extensions/stats@local/` and compiles the schema. Settings persist in dconf across logout.

`make enable` turns Stats on when this Shell session already knows the extension. On a first install the running session has not scanned `~/.local/share/gnome-shell/extensions` yet, so `gnome-extensions enable` reports that `stats@local` does not exist and does not save the setting. `make enable` records the UUID in `org.gnome.shell enabled-extensions` (and removes it from `disabled-extensions`) and exits successfully. The top-bar indicator appears after you log out and back in.

GNOME Shell on Wayland does not reload extensions with Alt+F2 `r`. Later code changes also need a new session (or the nested session below) before the shell picks them up.

```bash
make disable
make logs
```

## Nested session

GNOME 49 and newer, including Ubuntu 26.04 / GNOME Shell 50:

```bash
sudo apt install mutter-dev-bin
dbus-run-session -- gnome-shell --devkit --wayland
```

`mutter-dev-bin` provides `/usr/libexec/mutter-devkit`. Without it, `--devkit` exits immediately.

GNOME 48 and earlier (including Ubuntu 24.04 / GNOME 46):

```bash
dbus-run-session -- gnome-shell --nested --wayland
```

Enable the extension inside that session with `gnome-extensions enable stats@local`. The nested shell is not fully isolated from the host.

## What the numbers mean

- CPU percent is the busy share of `/proc/stat` between ticks (`idle + iowait` against total). The first tick is `n/a`. Frequency comes from `scaling_cur_freq` and is hidden when cpufreq is missing.
- Memory used is `MemTotal - MemAvailable`.
- Storage space is read per mount point (default `/`). Read/write speed sums physical disks from `/proc/diskstats` (512-byte sectors), skipping loop, ram, device-mapper and partitions whose parent disk is already counted.
- GPU vendor comes from `/sys/class/drm/card*/device/vendor`. A card whose runtime PM status is `suspended` is shown as `idle/off` and is not queried, so a sleeping discrete GPU is not woken up. NVIDIA samples come from one long-lived `nvidia-smi -l 1` process. AMD uses sysfs. Intel shows the current GT frequency from sysfs (`gt/gt0/rps_act_freq_mhz`, or `gt_act_freq_mhz` on older layouts) and engine utilisation only when `intel_gpu_top` runs.
- Temperature is read from hwmon (Intel `coretemp` package, AMD `k10temp` Tctl/Tdie, NVMe composite, amdgpu) and from the `x86_pkg_temp` thermal zone. Values are converted from millidegrees. Thresholds are stored in Celsius.
- Refresh rate comes from Mutter `GetCurrentState` on `org.gnome.Mutter.DisplayConfig`, updated by the `MonitorsChanged` signal rather than the poll timer. The figure is the **nominal mode rate**. Variable-refresh panels expose `refresh-rate-mode = variable`, but not the instantaneous rate, so Stats does not invent one.
- Network speed sums `/proc/net/dev`, skipping loopback. The first sample is `n/a`. Down is received bytes per second and up is transmitted bytes per second.

The top bar sits on the left by default, after the workspace indicator and before the clock. Download and upload speeds sit on the right, immediately after the clock. Change **Position** in preferences to move the main indicator; the network chip stays on the right. Index `20` on the left keeps the main indicator at the end of that side. Each metric shows its name, a percent or a temperature and a speed, separated by `|`. On a narrow panel the speed drops first, then the icon, leaving the name and the value (`CPU 34%`). If that still meets the clock, refresh and temperature drop, then later percent metrics from the right. The network chip reads `↓ 1.2 MB/s | ↑ 40 KB/s`, each direction with its own arrow; when the gap narrows it keeps the download speed alone. In text mode the arrows become the words `Down` and `Up`. It is pinned to the front of the right box, so an extension enabled later cannot push it away from the clock. Compact mode shows the first enabled metric until the pointer hovers the indicator or the menu is open. Resource warnings are off until **Resource warnings** is enabled in preferences.

## The dashboard

Click the readings in the top bar (or press `Ctrl+Alt+S`) for the dashboard. It looks and works like the Windows app's:

- **Header:** a status chip that names the most serious thing going on ("All good", "RAM high", "/ almost full", "CPU busy", "Running hot") and a gear that opens Preferences.
- **Six live tiles:** CPU, RAM, Disk, GPU, Temperature and Network, each with a big value in its warning colour, a detail line and a small graph. Click a tile to jump to its card.
- **Cards:** CPU (with per-core bars), Memory, Programs, Storage, Network, GPU, Temperatures and Displays, as label/value rows. Click a heading to fold a card. Folded cards are remembered in `collapsed-sections` and are not refreshed while folded, so folding the ones you do not watch also cuts the work done on every poll.
- **Programs:** the programs using the most memory, grouped by name ("Firefox ×12 · 3.1 GB · 4%"). They are read from `/proc` only while the dashboard is open, and only your own processes are listed. **Close** asks first, then sends SIGTERM (the same request to quit as at log-out). If the program is still running 5 seconds later, the button becomes **Force end** (SIGKILL, also confirmed). Parts of the desktop (gnome-shell, gsd-*, Xwayland, PipeWire, systemd and so on) never get a button. **System Monitor** opens GNOME System Monitor.
- **Quick controls:** top-bar side, top-bar style, graphs, per-core bars, pressure alerts, pause when locked and frosted glass, plus All settings… and System Monitor.

The dashboard is **frosted**: the shell's own background blur shows what is behind it through a graphite tint. Turn it off in Preferences › Dashboard (`frosted-popup`). The popup is capped at 480px wide and at 72% of the work-area height, and it scrolls.

### Keyboard shortcuts

They work from anywhere in GNOME. Change them in Preferences › Shortcuts (Set…, then press the combination; Backspace turns one off).

| Shortcut | Does | Setting |
|---|---|---|
| `Ctrl+Alt+S` | Open or close the dashboard | `shortcut-dashboard` |
| `Ctrl+Alt+,` | Open Preferences | `shortcut-settings` |
| `Ctrl+Alt+H` | Cycle the top-bar style: icons, text, compact | `shortcut-display-mode` |

## Develop

```bash
make test     # shared-spec check (node) + gjs parser and dashboard tests
node tests/ui.test.js   # the dashboard logic tests also run under Node
make lint     # ESLint 9 (npm install on first run; system ESLint 6 cannot load eslint.config.js)
make schemas
make pack     # stats@local.shell-extension.zip (the release asset; needs only python3)
```

Parser tests cover `/proc/stat`, `/proc/meminfo`, `/proc/diskstats`, hwmon selection, nvidia-smi CSV, intel_gpu_top JSON and Mutter monitor state. They do not need a running shell.

### Enable and disable leak check

With the extension installed in a nested session or after login:

```bash
for i in $(seq 1 20); do
  gnome-extensions disable stats@local
  gnome-extensions enable stats@local
done
```

Then confirm `journalctl -o cat /usr/bin/gnome-shell` has no Stats errors and that no `nvidia-smi` or `intel_gpu_top` process remains after disable. Polling also pauses while the screen is locked or the session is idle when that preference is on.
