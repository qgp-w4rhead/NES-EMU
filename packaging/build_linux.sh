#!/usr/bin/env bash
# Build a distributable Linux package of the NES emulator (M37).
#
# Produces:
#   - dist/nes-emu-linux-x86_64.tar.gz   (portable tarball)
#   - dist/nes-emu.AppImage              (AppImage, if appimagetool is available)
#
# The portable tarball contains:
#   nes-emu          — the release binary
#   README.md        — build + usage docs
#   config.toml      — default key bindings + settings (if present)
#
# Requirements:
#   - Rust stable toolchain (rustup default stable)
#   - SDL2 development headers (apt: libsdl2-dev)
#
# Usage:
#   ./packaging/build_linux.sh
#
# Exit codes:
#   0 — success
#   1 — build or packaging failure
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

DIST_DIR="$REPO_ROOT/dist"
ARTIFACT_TAR="$DIST_DIR/nes-emu-linux-x86_64.tar.gz"
ARTIFACT_APPIMAGE="$DIST_DIR/nes-emu.AppImage"

echo "==> Building release binary (linux)..."
cargo build --release

BIN_PATH="$REPO_ROOT/target/release/nes-emu"
if [[ ! -x "$BIN_PATH" ]]; then
    echo "error: release binary not found at $BIN_PATH" >&2
    exit 1
fi

echo "==> Assembling portable tarball..."
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR/bin"
cp "$BIN_PATH" "$DIST_DIR/bin/"
[[ -f README.md ]] && cp README.md "$DIST_DIR/"
[[ -f config.toml ]] && cp config.toml "$DIST_DIR/"
tar -czf "$ARTIFACT_TAR" -C "$DIST_DIR" .
echo "    -> $ARTIFACT_TAR"

# ---------------------------------------------------------------------------
# AppImage build (optional — only if appimagetool is present).
# ---------------------------------------------------------------------------
if command -v appimagetool >/dev/null 2>&1; then
    echo "==> Building AppImage..."
    APPDIR="$DIST_DIR/nes-emu.AppDir"
    mkdir -p "$APPDIR/usr/bin"
    cp "$BIN_PATH" "$APPDIR/usr/bin/"
    mkdir -p "$APPDIR/usr/share/applications"
    cat > "$APPDIR/nes-emu.desktop" <<'DESKTOP'
[Desktop Entry]
Name=NES Emulator
Comment=Nintendo Entertainment System emulator
Exec=nes-emu
Icon=nes-emu
Type=Application
Categories=Game;Emulator;
Terminal=false
DESKTOP
    cp "$APPDIR/nes-emu.desktop" "$APPDIR/usr/share/applications/"
    # AppRun shim
    cat > "$APPDIR/AppRun" <<'APPRUN'
#!/usr/bin/env bash
exec "$(dirname "$0")/usr/bin/nes-emu" "$@"
APPRUN
    chmod +x "$APPDIR/AppRun"
    appimagetool "$APPDIR" "$ARTIFACT_APPIMAGE" || {
        echo "warning: AppImage build failed; tarball still produced" >&2
    }
    [[ -f "$ARTIFACT_APPIMAGE" ]] && echo "    -> $ARTIFACT_APPIMAGE"
else
    echo "==> appimagetool not found; skipping AppImage (tarball still produced)"
fi

echo "==> Done. Artifacts in $DIST_DIR/"
