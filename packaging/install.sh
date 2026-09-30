#!/bin/sh
# Builds both frontends and installs them for the current user: the binaries,
# plus, on Linux, the windowed one's desktop entry and icon, so application
# launchers can start it. macOS has no use for either, and gets the binaries
# alone. Windows is served by packaging/install.bat.
set -eu

prefix="${PREFIX:-$HOME/.local}"
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo=$(dirname -- "$here")

case "$(uname -s)" in
    Darwin) launcher=no ;;
    *) launcher=yes ;;
esac

echo "building uwot-m8 and uwot-tui"
cargo build --release --manifest-path "$repo/Cargo.toml" \
    --bin uwot-m8 --bin uwot-tui

# install -D is GNU-only, and macOS ships the BSD one.
mkdir -p "$prefix/bin"
install -m755 "$repo/target/release/uwot-m8" "$prefix/bin/uwot-m8"
install -m755 "$repo/target/release/uwot-tui" "$prefix/bin/uwot-tui"
if [ "$launcher" = yes ]; then
    icons="$prefix/share/icons/hicolor/scalable/apps"
    mkdir -p "$icons"
    install -m644 "$here/uwot-m8.svg" "$icons/uwot-m8.svg"

    # The launcher needs an absolute Exec, since it may not share your PATH.
    mkdir -p "$prefix/share/applications"
    sed "s|^Exec=uwot-m8$|Exec=$prefix/bin/uwot-m8|" "$here/uwot-m8.desktop" \
        > "$prefix/share/applications/uwot-m8.desktop"
    chmod 644 "$prefix/share/applications/uwot-m8.desktop"

    if command -v update-desktop-database >/dev/null 2>&1; then
        update-desktop-database "$prefix/share/applications" || true
    fi
    if command -v gtk-update-icon-cache >/dev/null 2>&1; then
        gtk-update-icon-cache -qtf "$prefix/share/icons/hicolor" 2>/dev/null || true
    fi
fi

echo "installed uwot-m8 and uwot-tui to $prefix/bin"
if [ "$launcher" = yes ]; then
    echo "desktop entry: $prefix/share/applications/uwot-m8.desktop"
fi

if [ "$launcher" = no ]; then
    echo
    echo "note: the first time you turn the audio on, macOS will ask for"
    echo "microphone access. The M8 arrives as an input device, so that is what"
    echo "playing its audio needs."
fi

if [ "$launcher" = yes ] && [ ! -e /etc/udev/rules.d/70-m8.rules ]; then
    echo
    echo "note: the M8's serial port is root:dialout by default. To use it as"
    echo "your own user, install the udev rule:"
    echo "  sudo install -m644 $here/70-m8.rules /etc/udev/rules.d/70-m8.rules"
    echo "  sudo udevadm control --reload-rules && sudo udevadm trigger --subsystem-match=tty"
fi
