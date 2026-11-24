# Stats

A light system monitor that shows CPU, memory, storage, GPU, temperature and network in one slim bar. It warns before your machine runs out of something and names the program responsible.

**Website and downloads:** https://matolojr.github.io/stats/

| Platform | Install (one command) | Code |
|---|---|---|
| Linux (GNOME 46 to 50) | `curl -fsSL https://matolojr.github.io/stats/install.sh \| bash` | [`linux/`](linux/README.md): GNOME Shell extension (GJS) |
| Windows 10/11 | `irm https://matolojr.github.io/stats/install.ps1 \| iex` | [`windows/`](windows/README.md): native Rust/Win32 app |
| macOS | coming next | [`macos/`](macos/README.md) |

Both installers run without admin rights. They check the system first and verify the download against `SHA256SUMS`, and running one again upgrades in place. Each README covers uninstalling.

## How the repo is organised

```
linux/     GNOME Shell extension. It must be JavaScript, so it is its own app.
core/      Rust library `stats_core`: settings, warning levels, blame, bar fitting,
           build-folder rules. No OS calls; used by windows/ now and macOS next.
windows/   Rust/Win32 app (stats.exe) built on core/, plus install.ps1 and uninstall.ps1.
macos/     Placeholder for the next iteration.
shared/    defaults.ini: one list of setting names and defaults that every app is
           checked against (check-spec.mjs for Linux, a core test for Windows).
html/      The download website (static, no build step), deployed to GitHub Pages.
```

The apps share **behaviour, not code**. They use the same setting names, so a key means the same thing everywhere, and CI fails if a default changes on one side without `shared/defaults.ini`. Platform-neutral logic lives in `core/` so each new native app (macOS next) only adds sensors and UI.

## Develop

```bash
cargo test --workspace             # core logic + the Windows side of the spec check
node shared/check-spec.mjs         # Linux side of the spec check
make -C linux test lint            # on Linux: gjs parser tests and ESLint
.\windows\build.ps1 -Install       # on Windows: build and install from the local build
```

Open `html/index.html` in a browser to work on the site.

## Releasing

1. Bump the version in `VERSION`, `Cargo.toml` (`[workspace.package]`) and `linux/metadata.json` (`version-name`). `scripts/check-version.sh` confirms they match.
2. Commit, then tag and push: `git tag v0.2.0 && git push origin v0.2.0`.
3. [`release.yml`](.github/workflows/release.yml) builds `stats@local.shell-extension.zip` and `stats.exe` and writes `SHA256SUMS`. It publishes a GitHub Release with `uninstall.ps1` included. Tags with a `-` (`v0.2.0-rc1`) become pre-releases, which the installers skip.
4. [`pages.yml`](.github/workflows/pages.yml) redeploys the site with the current installers. The one-liners then install the new release.

One-time setup: in the repo's Settings > Pages, set **Source** to **GitHub Actions**. The repository (or at least its releases) must be public for the one-liners to download without a token.
