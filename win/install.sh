#!/usr/bin/env bash
# Install Stats for the current user. From a checkout this links the tree.
# A hosted zip is used only when STATS_RELEASE_URL points at a real file.
set -euo pipefail

if [ "$(id -u)" -eq 0 ]; then
    echo "Run install.sh as your user, not root." >&2
    exit 1
fi

RELEASE_URL="${STATS_RELEASE_URL:-https://example.invalid/stats/stats@local.shell-extension.zip}"
UUID="stats@local"
EXT_DIR="${HOME}/.local/share/gnome-shell/extensions/${UUID}"

bar() {
    local step="$1"
    local label="$2"
    local width=20
    local filled=$(( step * width / 4 ))
    local empty=$(( width - filled ))
    local hashes dashes
    hashes=$(printf '%*s' "$filled" '' | tr ' ' '#')
    dashes=$(printf '%*s' "$empty" '' | tr ' ' '-')
    printf '  [%s%s] %s\n' "$hashes" "$dashes" "$label"
}

enable_extension() {
    python3 - "$UUID" <<'PY'
import ast, subprocess, sys
uuid = sys.argv[1]
schema = "org.gnome.shell"

def get_list(key):
    raw = subprocess.check_output(["gsettings", "get", schema, key], text=True).strip()
    if raw in ("@as []", "[]"):
        return []
    return list(ast.literal_eval(raw))

def set_list(key, values):
    subprocess.check_call(["gsettings", "set", schema, key, str(values)])

if subprocess.call(["gnome-extensions", "info", uuid], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL) == 0:
    subprocess.check_call(["gnome-extensions", "enable", uuid])
else:
    enabled = get_list("enabled-extensions")
    if uuid not in enabled:
        enabled.append(uuid)
        set_list("enabled-extensions", enabled)
    disabled = get_list("disabled-extensions")
    if uuid in disabled:
        set_list("disabled-extensions", [item for item in disabled if item != uuid])
    print(f"{uuid} is not loaded in this Shell session yet.")
    print("It will start after you log out and back in.")
PY
}

SCRIPT_DIR=""
if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "${BASH_SOURCE[0]}" ]; then
    SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
fi

if [ -n "$SCRIPT_DIR" ] && [ -f "$SCRIPT_DIR/metadata.json" ]; then
    bar 1 "Using this checkout"
    bar 2 "Linking extension"
    mkdir -p "$(dirname "$EXT_DIR")"
    ln -sfn "$SCRIPT_DIR" "$EXT_DIR"
else
    case "$RELEASE_URL" in
        *example.invalid*)
            echo "No release is published at ${RELEASE_URL} yet." >&2
            echo "From a checkout, run ./install.sh" >&2
            exit 1
            ;;
    esac
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    bar 1 "Downloading"
    curl -fsSL "$RELEASE_URL" -o "$work/stats.zip"
    bar 2 "Unpacking"
    rm -rf "$EXT_DIR"
    mkdir -p "$EXT_DIR"
    unzip -q "$work/stats.zip" -d "$EXT_DIR"
fi

bar 3 "Compiling schemas"
glib-compile-schemas --strict "$EXT_DIR/schemas"
bar 4 "Enabling"
enable_extension
echo "On Wayland, log out and back in before the indicators appear."
