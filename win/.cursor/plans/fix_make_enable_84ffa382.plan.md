---
name: Fix make enable
overview: "`make enable` fails because the already-running GNOME Shell built its extension list at login, before the symlink existed. The fix is to record `stats@local` in enabled-extensions when the live session does not know it yet and to say that a logout is still required before the indicator appears."
todos:
  - id: enable-target
    content: Make `make enable` queue stats@local in gsettings when the running Shell has not scanned it and enable normally when it has
    status: completed
  - id: readme-install
    content: Update Makefile install hints and README so enable-before-logout is expected and does not look like a failed install
    status: completed
isProject: false
---

# Fix `make enable` for a session that has not scanned Stats

## Why it fails

`make install` is correct. The symlink [`~/.local/share/gnome-shell/extensions/stats@local`](~/.local/share/gnome-shell/extensions/stats@local) points at this repo, `metadata.json` uuid is `stats@local` and `enumerate_children` reports that symlink as a directory (`Gio.FileType.DIRECTORY`, `is-symlink` true). Shell 50 does not skip it.

`gnome-extensions enable` calls `EnableExtension` on the running Shell. That method returns false unless the UUID is already in `ExtensionManager._extensions`:

```273:275:js/ui/extensionSystem.js
enableExtension(uuid) {
    if (!this._extensions.has(uuid))
        return false;
```

That map is filled once, in `_loadExtensions()`, at session start. There is no directory monitor and `ReloadExtension` is a no-op (`NOT_SUPPORTED`). `Eval` cannot rescan either: it returns failure unless `global.context.unsafe_mode` is set, which a normal Wayland session does not set.

So the command exits 2 and the UUID is never written to `org.gnome.shell enabled-extensions`. A later logout still leaves Stats disabled.

## Change

Update the `enable` target in [`Makefile`](Makefile):

- If `gnome-extensions info $(UUID)` succeeds, the session already knows the extension. Call `gnome-extensions enable $(UUID)` as today (this also updates gsettings through the Shell).
- Otherwise, with Python (already used for the UUID):
  - Append `stats@local` to `org.gnome.shell enabled-extensions` if it is missing.
  - Remove it from `disabled-extensions` if present (`disabled-extensions` wins over the enabled list).
  - Print that the extension is queued and will load after logout, or inside a nested session started after install.
  - Exit 0. Do not treat “does not exist” as a Make failure once the setting is saved.

Leave the symlink install alone. Copying the tree would not make this session see it.

Adjust the two `echo` lines at the end of `install` and the Install section of [`README.md`](README.md), so the order is: `make install`, `make enable` (safe before the Shell has scanned), then log out and back in. The first appearance of the top-bar indicator still requires a new Shell session.
