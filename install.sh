#!/usr/bin/env bash
# Install, upgrade or remove Stats (GNOME Shell extension) for the current user.
#
#   curl -fsSL https://matolojr.github.io/stats/install.sh | bash
#   curl -fsSL https://matolojr.github.io/stats/install.sh | bash -s -- --uninstall
#   ./install.sh                 from a checkout: links linux/ (for development)
#
# Environment:
#   STATS_VERSION=0.2.0          install that release instead of the latest
#   STATS_RELEASE_BASE=<url>     folder holding the zip and SHA256SUMS (file:// works, for testing)
#   STATS_FORCE=1                skip the GNOME Shell version check
#
# Everything runs inside main(), so a download cut short never runs half a script.
set -euo pipefail

UUID="stats@local"
ZIP_NAME="${UUID}.shell-extension.zip"
REPO="MatoloJr/stats"
MIN_SHELL=46
MAX_SHELL=50
EXT_ROOT="${HOME}/.local/share/gnome-shell/extensions"
EXT_DIR="${EXT_ROOT}/${UUID}"

say() { printf '%s\n' "$*"; }
die() { printf 'Stats: %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

bar() {
    local step="$1" label="$2" width=20
    local filled=$(( step * width / 5 ))
    local hashes dashes
    hashes=$(printf '%*s' "$filled" '' | tr ' ' '#')
    dashes=$(printf '%*s' "$(( width - filled ))" '' | tr ' ' '-')
    printf '  [%s%s] %s\n' "$hashes" "$dashes" "$label"
}

# Package that provides a missing command, for the hint.
package_for() {
    local tool="$1" id=""
    if [ -r /etc/os-release ]; then
        # shellcheck disable=SC1091
        id=$(. /etc/os-release && printf '%s %s' "${ID:-}" "${ID_LIKE:-}")
    fi
    case "$tool" in
        gnome-extensions|gnome-shell) printf 'gnome-shell' ;;
        curl) printf 'curl' ;;
        *)  case "$id" in
                *debian*|*ubuntu*) printf 'libglib2.0-bin' ;;
                *) printf 'glib2' ;;
            esac ;;
    esac
}

install_hint() {
    local pkgs="$*" id=""
    if [ -r /etc/os-release ]; then
        # shellcheck disable=SC1091
        id=$(. /etc/os-release && printf '%s %s' "${ID:-}" "${ID_LIKE:-}")
    fi
    case "$id" in
        *debian*|*ubuntu*) printf 'sudo apt install %s' "$pkgs" ;;
        *fedora*|*rhel*)   printf 'sudo dnf install %s' "$pkgs" ;;
        *arch*)            printf 'sudo pacman -S %s' "$pkgs" ;;
        *suse*)            printf 'sudo zypper install %s' "$pkgs" ;;
        *)                 printf 'install: %s' "$pkgs" ;;
    esac
}

preflight() {
    [ "$(id -u)" -ne 0 ] || die "run this as your normal user, not root or sudo. Extensions are installed per user."
    have gnome-shell || die "GNOME Shell was not found. Stats is a GNOME Shell extension (GNOME ${MIN_SHELL}-${MAX_SHELL})."

    local version major
    version=$(gnome-shell --version 2>/dev/null | grep -oE '[0-9]+(\.[0-9]+)*' | head -n1 || true)
    major="${version%%.*}"
    if [ -z "$major" ]; then
        say "Could not read the GNOME Shell version; continuing."
    elif [ "$major" -lt "$MIN_SHELL" ] || [ "$major" -gt "$MAX_SHELL" ]; then
        [ "${STATS_FORCE:-}" = 1 ] || die "GNOME Shell ${version} is not supported (needs ${MIN_SHELL}-${MAX_SHELL}). Set STATS_FORCE=1 to try anyway."
    fi

    local missing=() tool
    for tool in gnome-extensions glib-compile-schemas gsettings "$@"; do
        have "$tool" || missing+=("$tool")
    done
    if [ "${#missing[@]}" -gt 0 ]; then
        local pkgs=() t p
        for t in "${missing[@]}"; do
            p=$(package_for "$t")
            [[ " ${pkgs[*]-} " == *" $p "* ]] || pkgs+=("$p")
        done
        die "missing ${missing[*]}. Fix with: $(install_hint "${pkgs[@]}")"
    fi
}

# --- gsettings list helpers (no python needed) ------------------------------
gs_get() { gsettings get org.gnome.shell "$1"; }
gs_add() {  # key uuid
    local raw; raw=$(gs_get "$1")
    case "$raw" in *"'$2'"*) return 0 ;; esac
    case "$raw" in
        "@as []"|"[]") raw="['$2']" ;;
        *) raw="${raw%]}, '$2']" ;;
    esac
    gsettings set org.gnome.shell "$1" "$raw"
}
gs_remove() {  # key uuid
    local raw new; raw=$(gs_get "$1")
    case "$raw" in *"'$2'"*) ;; *) return 0 ;; esac
    new=$(printf '%s' "$raw" | sed -e "s/'$2', //; s/, '$2'//; s/'$2'//")
    [ "$new" = "[]" ] && new="@as []"
    gsettings set org.gnome.shell "$1" "$new"
}

enable_extension() {
    if gnome-extensions info "$UUID" >/dev/null 2>&1 && gnome-extensions enable "$UUID" 2>/dev/null; then
        return 0
    fi
    # The running Shell has not scanned the new folder yet: record the setting
    # so it starts at the next login.
    gs_add enabled-extensions "$UUID"
    gs_remove disabled-extensions "$UUID"
}

sha256_of() {
    if have sha256sum; then sha256sum "$1" | cut -d' ' -f1
    elif have shasum; then shasum -a 256 "$1" | cut -d' ' -f1
    else die "need sha256sum or shasum to verify the download"
    fi
}

unpack() {  # zip dest
    if have unzip; then unzip -q "$1" -d "$2"
    elif have python3; then python3 -m zipfile -e "$1" "$2"
    elif have bsdtar; then bsdtar -xf "$1" -C "$2"
    else die "need unzip (or python3) to unpack. Fix with: $(install_hint unzip)"
    fi
}

release_base() {
    if [ -n "${STATS_RELEASE_BASE:-}" ]; then
        printf '%s' "${STATS_RELEASE_BASE%/}"
    elif [ -n "${STATS_VERSION:-}" ]; then
        printf 'https://github.com/%s/releases/download/v%s' "$REPO" "${STATS_VERSION#v}"
    else
        printf 'https://github.com/%s/releases/latest/download' "$REPO"
    fi
}

finish_message() {
    say ""
    if gnome-extensions info "$UUID" 2>/dev/null | grep -qiE 'state: *(active|enabled)'; then
        say "Stats is running. Look at the top bar."
    elif [ "${XDG_SESSION_TYPE:-}" = "x11" ]; then
        say "Stats is installed and enabled. Press Alt+F2, type r and Enter to restart GNOME Shell, or log out and back in."
    else
        say "Stats is installed and enabled. Log out and back in once (GNOME on Wayland only loads new extensions at login)."
    fi
    say "Settings: gnome-extensions prefs ${UUID}    Remove: rerun with --uninstall"
}

install_from_checkout() {
    local src="$1"
    preflight
    bar 1 "Using this checkout ($src)"
    bar 2 "Linking extension"
    mkdir -p "$EXT_ROOT"
    if [ -e "$EXT_DIR" ] && [ ! -L "$EXT_DIR" ]; then
        rm -rf "$EXT_DIR"
    fi
    ln -sfn "$src" "$EXT_DIR"
    bar 3 "Compiling schemas"
    glib-compile-schemas --strict "$EXT_DIR/schemas"
    bar 4 "Enabling"
    enable_extension
    bar 5 "Done"
    finish_message
}

install_release() {
    preflight curl
    local base work staged
    base=$(release_base)
    work=$(mktemp -d)
    # shellcheck disable=SC2064
    trap "rm -rf '$work'" EXIT

    bar 1 "Downloading ${base}/${ZIP_NAME}"
    curl -fsSL --retry 3 "${base}/${ZIP_NAME}" -o "$work/stats.zip" \
        || die "download failed. Check your connection, or that release ${STATS_VERSION:-latest} exists at https://github.com/${REPO}/releases"
    curl -fsSL --retry 3 "${base}/SHA256SUMS" -o "$work/SHA256SUMS" \
        || die "could not download SHA256SUMS to verify the release"

    bar 2 "Verifying checksum"
    local want got
    want=$(tr -d '\r' < "$work/SHA256SUMS" | awk -v f="$ZIP_NAME" '{ n = $2; sub(/^\*/, "", n) } n == f { print $1; exit }')
    [ -n "$want" ] || die "SHA256SUMS has no entry for ${ZIP_NAME}"
    got=$(sha256_of "$work/stats.zip")
    [ "$want" = "$got" ] || die "checksum mismatch (expected $want, got $got). Nothing was changed."

    bar 3 "Unpacking and compiling schemas"
    mkdir -p "$EXT_ROOT"
    # Stage beside the final folder so the swap below is a rename on one disk.
    staged="${EXT_ROOT}/.${UUID}.new.$$"
    rm -rf "$staged"
    mkdir -p "$staged"
    unpack "$work/stats.zip" "$staged"
    [ -f "$staged/metadata.json" ] || { rm -rf "$staged"; die "the zip has no metadata.json; refusing to install"; }
    glib-compile-schemas --strict "$staged/schemas" || { rm -rf "$staged"; die "schema compile failed"; }

    bar 4 "Installing to ${EXT_DIR}"
    if [ -L "$EXT_DIR" ]; then rm -f "$EXT_DIR"; else rm -rf "$EXT_DIR"; fi
    mv "$staged" "$EXT_DIR"

    bar 5 "Enabling"
    enable_extension
    finish_message
}

uninstall() {
    have gnome-extensions && gnome-extensions disable "$UUID" >/dev/null 2>&1 || true
    if have gsettings; then
        gs_remove enabled-extensions "$UUID" || true
    fi
    if [ -L "$EXT_DIR" ]; then rm -f "$EXT_DIR"; else rm -rf "$EXT_DIR"; fi
    say "Stats is removed. Your settings stay in dconf; clear them with:"
    say "  dconf reset -f /org/gnome/shell/extensions/stats/"
}

main() {
    local action=install
    for arg in "$@"; do
        case "$arg" in
            --uninstall|uninstall) action=uninstall ;;
            -h|--help) sed -n '2,13p' "${BASH_SOURCE[0]:-/dev/null}" 2>/dev/null || true; say "See https://matolojr.github.io/stats"; return 0 ;;
            *) die "unknown option: $arg (use --uninstall or --help)" ;;
        esac
    done
    if [ "$action" = uninstall ]; then
        uninstall
        return 0
    fi

    # Run from a checkout (./install.sh) -> link it. Piped from curl -> release.
    local src=""
    if [ -n "${BASH_SOURCE[0]:-}" ] && [ -f "${BASH_SOURCE[0]}" ]; then
        src="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/linux"
    fi
    if [ -n "$src" ] && [ -f "$src/metadata.json" ] && [ -z "${STATS_RELEASE_BASE:-}" ] && [ -z "${STATS_VERSION:-}" ]; then
        install_from_checkout "$src"
    else
        install_release
    fi
}

main "$@"
