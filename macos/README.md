# Stats for macOS (next iteration)

Not built yet. The plan, so the pieces already in place fit it:

- **A native app in Rust** that links [`../core/`](../core/) (`stats_core`), as the Windows app does. Settings parsing, warning levels, blame, bar fitting and build-folder rules are already platform-neutral and tested.
- **A sensors module for macOS** (`host_statistics64`, `sysctl`, IOKit for disks and temperature) and a menu-bar item instead of the Windows floating bar.
- **The shared spec.** Add a `[macos]` section to [`../shared/defaults.ini`](../shared/defaults.ini), or rely on `[shared]`. `check-spec.mjs` and the core test already reject unknown sections and drift.
- **Install.** Add a `macos/install.sh` and a `stats-macos.zip` release asset. The website already shows a macOS tab.
