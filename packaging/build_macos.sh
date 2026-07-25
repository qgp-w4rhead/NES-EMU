#!/usr/bin/env bash
# Build a distributable macOS .app bundle of the NES emulator (M37).
#
# Produces:
#   dist/nes-emu.app/                    — the .app bundle
#   dist/nes-emu-macos.tar.gz            — tarball of the .app bundle
#
# The .app bundle layout:
#   nes-emu.app/
#     Contents/
#       Info.plist          — minimal macOS app metadata
#       MacOS/nes-emu       — the release binary
#       Resources/          — README.md + config.toml (if present)
#
# Requirements:
#   - Rust stable toolchain
#   - SDL2 framework (brew install sdl2)
#
# Usage:
#   ./packaging/build_macos.sh
#   TARGET=aarch64-apple-darwin ./packaging/build_macos.sh  # Apple Silicon
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

TARGET="${TARGET:-$(rustc -vV | sed -n 's/host: //p')}"
DIST_DIR="$REPO_ROOT/dist"
APP_DIR="$DIST_DIR/nes-emu.app"
ARTIFACT="$DIST_DIR/nes-emu-macos.tar.gz"

echo "==> Building release binary (macos, target=$TARGET)..."
cargo build --release --target "$TARGET"

BIN_PATH="$REPO_ROOT/target/$TARGET/release/nes-emu"
if [[ ! -x "$BIN_PATH" ]]; then
    echo "error: release binary not found at $BIN_PATH" >&2
    exit 1
fi

echo "==> Assembling .app bundle..."
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"
cp "$BIN_PATH" "$APP_DIR/Contents/MacOS/nes-emu"
[[ -f README.md ]] && cp README.md "$APP_DIR/Contents/Resources/"
[[ -f config.toml ]] && cp config.toml "$APP_DIR/Contents/Resources/"

cat > "$APP_DIR/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>NES Emulator</string>
    <key>CFBundleDisplayName</key>
    <string>NES Emulator</string>
    <key>CFBundleIdentifier</key>
    <string>com.nes-emu.app</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleExecutable</key>
    <string>nes-emu</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>LSMinimumSystemVersion</key>
    <string>10.12</string>
</dict>
</plist>
PLIST

echo "==> Creating tarball..."
tar -czf "$ARTIFACT" -C "$DIST_DIR" nes-emu.app
echo "    -> $ARTIFACT"
echo "    -> $APP_DIR"
echo "==> Done. Artifacts in $DIST_DIR/"
