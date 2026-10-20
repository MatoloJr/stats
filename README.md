# Stats

Stats is a GNOME Shell extension that shows real-time system usage in the top bar: CPU, memory, storage, GPU, temperature and the nominal refresh rate of each monitor. Click the indicator for a dashboard. The display name, preferences title and popup header are **Stats**.

The UUID is `stats@local`, set only in [`metadata.json`](metadata.json). The install directory must use that same name. The GSettings schema id is `org.gnome.shell.extensions.stats` (`lib/constants.js`).

Shell versions 46 through 50 are listed in `metadata.json`. The extension is an ES module (GNOME 45 and newer). It does not use X11 tools. Optional programs (`nvidia-smi`, `intel_gpu_top`, `gnome-system-monitor`) are used only when they are installed; missing hardware or tools show `n/a`.

## Install

```bash
make install
make enable
```

From this checkout, `./install.sh` does the same install and prints a four-step progress bar. A hosted release can be installed with one command:

```bash
curl -fsSL https://example.invalid/stats/install.sh | bash
```

That host does not serve the script yet. The script refuses to run as root, leaves your other extensions enabled, and still needs a logout on Wayland before the indicators appear.

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

The top bar sits on the left by default, after the workspace indicator and before the clock. Download and upload speeds sit on the right, immediately after the clock. Change **Position** in preferences to move the main indicator; the network chip stays on the right. Index `20` on the left keeps the main indicator at the end of that side. Each metric shows its name, a percent or a temperature, and a speed, separated by `|`. On a narrow panel the icon drops first, leaving the name and the value (`CPU 34%`), then the speed. If that still meets the clock, refresh and temperature drop first. Compact mode shows the first enabled metric until the pointer hovers the indicator or the menu is open. Resource warnings are off until **Resource warnings** is enabled in preferences.

## Develop

```bash
make test     # gjs parser tests
make lint     # ESLint 9 (npm install on first run; system ESLint 6 cannot load eslint.config.js)
make schemas
make pack     # stats@local.shell-extension.zip
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
