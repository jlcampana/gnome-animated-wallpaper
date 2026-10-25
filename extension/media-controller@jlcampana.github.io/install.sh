#!/usr/bin/env bash
# Installs Media Controller into your user account:
#
#   curl -fsSL https://raw.githubusercontent.com/jlcampana/mediaplayer-gnome-extension/main/install.sh | bash
#
# Run from a clone, it installs that copy; otherwise it downloads the main branch.
# Disables the original Advanced Media Controller if it is active (two media
# controls would show up in the top bar).
#
# Everything lives in main() so bash reads the whole script before running it
# (needed for «curl | bash»).
set -euo pipefail

REPO="${MEDIA_REPO:-jlcampana/mediaplayer-gnome-extension}"
BRANCH="${MEDIA_BRANCH:-main}"
UUID="media-controller@jlcampana.github.io"
SCHEMA="org.gnome.shell.extensions.media-controller"
UUID_ORIGINAL="advanced-media-controller@sanjai.com"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
DEST="$DATA/gnome-shell/extensions/$UUID"

info() { printf '\033[1;32m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33m==>\033[0m %s\n' "$*" >&2; }
fail() { printf '\033[1;31m==>\033[0m %s\n' "$*" >&2; exit 1; }

main() {
    command -v glib-compile-schemas >/dev/null || fail "glib-compile-schemas not found (package glib2 / libglib2.0-bin)"

    local src tmp=""
    local here
    here="$(cd "$(dirname "${BASH_SOURCE[0]:-.}")" 2>/dev/null && pwd || true)"
    if [[ -n $here && -f $here/metadata.json ]] && grep -q "\"$UUID\"" "$here/metadata.json"; then
        src="$here"
        info "Installing from $src"
    else
        command -v curl >/dev/null || fail "curl not found"
        tmp="$(mktemp -d)"
        trap 'rm -rf "$tmp"' EXIT
        info "Downloading $REPO ($BRANCH)"
        curl -fsSL "https://codeload.github.com/$REPO/tar.gz/refs/heads/$BRANCH" |
            tar -xz -C "$tmp" --strip-components=1
        src="$tmp"
    fi

    rm -rf "$DEST"
    mkdir -p "$DEST"
    # Only the extension itself, not the repo's housekeeping
    tar -C "$src" --exclude=.git --exclude=install.sh --exclude=README.md -cf - . | tar -C "$DEST" -xf -
    glib-compile-schemas "$DEST/schemas"
    mkdir -p "$DATA/glib-2.0/schemas"
    cp "$DEST/schemas/$SCHEMA.gschema.xml" "$DATA/glib-2.0/schemas/"
    glib-compile-schemas "$DATA/glib-2.0/schemas" 2>/dev/null || true
    info "Installed in $DEST"

    if command -v gnome-extensions >/dev/null; then
        if gnome-extensions list --enabled 2>/dev/null | grep -qx "$UUID_ORIGINAL"; then
            gnome-extensions disable "$UUID_ORIGINAL" 2>/dev/null || true
            info "Disabled Advanced Media Controller (replaced by this fork)"
        fi
    fi

    # GNOME Shell only picks up new extensions after logging back in on Wayland,
    # so it goes into the enabled list directly.
    if command -v gsettings >/dev/null; then
        local enabled
        enabled="$(gsettings get org.gnome.shell enabled-extensions 2>/dev/null || echo "@as []")"
        if [[ $enabled != *"'$UUID'"* ]]; then
            if [[ $enabled == "@as []" || $enabled == "[]" ]]; then
                enabled="['$UUID']"
            else
                enabled="${enabled%]}, '$UUID']"
            fi
            gsettings set org.gnome.shell enabled-extensions "$enabled" ||
                warn "could not enable it: gnome-extensions enable $UUID"
        fi
    fi

    if [[ ${XDG_SESSION_TYPE:-} == wayland ]]; then
        info "Log out and back in to load the new version."
    else
        info "Reload GNOME Shell (Alt+F2, r) or log out and back in."
    fi
}

main "$@"
