# Stats for Windows

The Windows version of Stats. It is a single native `stats.exe` (Rust and Win32, with no web view and no runtime to install) that uses a few MB of RAM. It shows the same readings as the GNOME extension in a slim floating bar. It also watches for the problems that make a Windows dev laptop crash, and offers a one-click fix for each.

It lives beside the GNOME extension ([`../linux/`](../linux/README.md)). The two share their setting names, which [`shared/defaults.ini`](../shared/defaults.ini) records and CI checks. They cannot share code, because a GNOME Shell extension must be JavaScript. The platform-neutral logic is in the Rust crate [`core/`](../core/), which the macOS version will reuse.

## Install

For users, from PowerShell (no admin):

```powershell
irm https://matolojr.github.io/stats/install.ps1 | iex
```

[`install.ps1`](install.ps1) downloads `stats.exe` from the latest GitHub release and checks it against `SHA256SUMS`. It installs to `%LOCALAPPDATA%\Programs\Stats`, sets Stats to start with Windows, adds a Start Menu shortcut and an entry in Settings > Apps, and starts it. Running it again upgrades in place. `$env:STATS_VERSION = '0.2.0'` before the command pins a release. To remove Stats, use Settings > Apps > Stats > Uninstall, or `irm https://matolojr.github.io/stats/uninstall.ps1 | iex`. Settings in `%APPDATA%\Stats` are kept unless you pass `-Purge` (or set `$env:STATS_PURGE = "1"` before the one-liner).

From a checkout you need Rust with the MSVC toolchain (the same setup you use for Tauri):

```powershell
.\windows\build.ps1 -Install
```

This runs the core tests and builds `stats.exe`. It lays out the release files in `target\dist` and installs from there with `install.ps1`, so you test the same path users get. Afterwards, `cargo clean` at the repo root frees the build folder.

CI builds and clippy-checks the app on every push and test-installs it. Pushing a `v*` tag publishes a release (see the [root README](../README.md#releasing)).

## What you see

- **The bar.** It sits just above the taskbar on the right and always stays on top. It shows `CPU 34% | RAM 68% | Commit 61% | C: 87% 3 MB/s | GPU 12% | ↓ 1.2 MB/s ↑ 40 KB/s`. Each value turns amber at the warning level and red at the critical level, and the thin line along the top takes the colour of the worst one.
  - **Click** the bar for the dashboard. **Drag** it anywhere; its position is saved. **Right-click** for the menu.
  - On a narrow screen the speeds drop first, then the least important readings. Commit, RAM and CPU stay longest.
  - The bar hides while a game or video runs full screen.
- **The tray icon.** Its middle bar fills as committed memory rises and turns amber or red. The tooltip shows the headline numbers. Click it for the dashboard.
- **The dashboard.** Memory comes first, then CPU, GPU, temperature, storage, build folders, network and displays. Click a heading to fold that section; folded sections are remembered.

## How it keeps things from crashing

On Windows, apps crash when **committed memory** reaches the **commit limit** (RAM plus page file), even while RAM still looks partly free. That is what happened on the 16 GB laptop with a 19.7 GB limit. Stats is built around that number.

| Problem | What Stats does |
|---|---|
| Commit, RAM or CPU stays high for 3 polls | Finds the program responsible, adding up all of its processes (Chrome ×23 = 4.1 GB). It names that program in a notification with one next step. The dashboard has a **Close** button for it. |
| Page file too small | Reads the real page file setting from the registry. A fixed size under half your RAM, or no page file, gets a once-a-day warning. **Fix** opens the right Windows dialog with the exact numbers to enter. |
| WSL / Docker VM (`vmmem`) using RAM | Shows how much RAM it uses. **Shut down** runs `wsl --shutdown`. |
| Docker disk growing | Shows the size of `docker_data.vhdx`. **Prune** runs `docker system prune -f`, which keeps your volumes. **Compact** gives the freed space back to Windows; it checks Docker Desktop is closed first, which is the step that failed when we ran it by hand. |
| Build output piling up (the 88 GB `target`) | Scans `scan-roots` once a day at background priority. It notifies you when one folder passes `junk-alert-gb` (20 GB by default). **Delete** or **Delete all** removes the folders. |
| Temp folder and package caches | Shows the Temp folder size. **Clean** empties Temp, skipping files in use. Another **Clean** runs `npm cache clean`, `go clean -cache`, `pip cache purge` and `pnpm store prune` for whichever of those tools you have installed. You can also have Temp files older than N days deleted daily. |
| A drive getting full | Notifies you at the warning level and says how much the build folders and Temp could free. |

Safety rules:

- **Nothing is closed or deleted without a Yes.** Closing a program works like clicking its X, so it can ask you to save. **Force end** is only offered if the program doesn't close.
- Stats never offers to close parts of Windows (svchost, explorer, dwm, Defender and the like).
- A folder only counts as build output when the file that proves it is next to it: `target` needs `Cargo.toml`, `node_modules` needs `package.json`, `build` and `.dart_tool` need `pubspec.yaml`, `.gradle` needs a Gradle build file. A folder that just happens to be called `build` is never touched. Stats checks this again right before deleting, and only deletes inside your scan folders. It never follows links or junctions.

Stats protects itself too:

- A fault in any handler is caught and logged instead of closing the app, and each fix runs on its own thread.
- History has a fixed size, the log is capped at 512 KB, and process lists are only read while there is a warning or the dashboard is open.
- If Explorer restarts, the tray icon comes back. If you change displays, the bar moves back onto a visible screen.
- Starting `stats.exe` a second time opens the running copy's dashboard instead of starting another one.

## Settings

Settings live in `%APPDATA%\Stats\config.ini`. Choose **Settings…** in the menu to open the file in Notepad, edit it and save; Stats reloads it within about 2 seconds. Every setting is explained in the file itself. A bad value falls back to its default and is noted in `%APPDATA%\Stats\stats.log`. The default levels are:

| | warning | critical |
|---|---|---|
| commit | 80 | 92 |
| memory | 85 | 95 |
| storage | 85 | 95 |
| cpu | 90 | 98 |
| gpu | 90 | 98 |
| temperature (°C) | 85 | 95 |

`scan-roots` defaults to `%USERPROFILE%\projects`. To scan several folders, separate them with `;`.

## Where the numbers come from

None of these need administrator rights.

- **CPU:** `GetSystemTimes`.
- **RAM and commit:** `GlobalMemoryStatusEx` and `GetPerformanceInfo`.
- **Disk, network, GPU, temperature and the WSL VM:** Windows performance counters, read by their English names so they work in any Windows language.
- **GPU busy:** worked out the same way Task Manager does it, using the busiest engine.
- **Temperature:** the ACPI thermal zone. Many laptops don't expose one to Windows, and then the bar leaves temperature out.
- **Refresh rate:** the current display mode.
- **Per-program memory:** private (committed) bytes, which is what counts against the limit.

## Develop

```powershell
cargo test -p stats-core          # logic tests (also run on Linux), including the shared-spec check
cargo clippy --workspace --all-targets
cargo build --release -p stats-win
```

Run these from the repo root (it is a Cargo workspace).

- [`../core/src/`](../core/src/) (the `stats_core` library) holds every decision: settings parsing, when to warn, who is to blame, what counts as build output, Temp cleanup, page file rules and bar fitting. It makes no Win32 calls and has unit tests.
- `windows/src/win/` holds the Win32 side: sensors, the bar, the dashboard, the tray and the fixes.
