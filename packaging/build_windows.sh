#!/usr/bin/env bash
# Build a distributable Windows package of the NES emulator (M37).
#
# Produces:
#   dist/nes-emu-windows-x86_64.zip   (portable ZIP)
#
# The portable ZIP contains:
#   bin/nes-emu.exe   — the release binary
#   README.md         — build + usage docs
#   config.toml       — default key bindings + settings (if present)
#
# This script is intended to run under MSYS2/Git Bash on Windows or as a
# cross-compilation step on Linux (with the mingw target installed). When
# cross-compiling from Linux, set TARGET=x86_64-pc-windows-gnu and install
# the target via `rustup target add $TARGET`.
#
# Requirements:
#   - Rust stable toolchain
#   - For native Windows builds: the `sdl2` crate bundles MSVC binaries.
#   - For cross-builds from Linux: mingw-w64 + SDL2 mingw libraries.
#
# Usage:
#   ./packaging/build_windows.sh                 # native MSVC build
#   TARGET=x86_64-pc-windows-gnu ./packaging/build_windows.sh  # cross from Linux
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$REPO_ROOT"

TARGET="${TARGET:-x86_64-pc-windows-msvc}"
DIST_DIR="$REPO_ROOT/dist"
ARTIFACT="$DIST_DIR/nes-emu-windows-x86_64.zip"

echo "==> Building release binary (windows, target=$TARGET)..."
if [[ "$TARGET" == *-msvc ]]; then
    cargo build --release --target "$TARGET"
else
    cargo build --release --target "$TARGET"
fi

BIN_PATH="$REPO_ROOT/target/$TARGET/release/nes-emu.exe"
if [[ ! -f "$BIN_PATH" ]]; then
    echo "error: release binary not found at $BIN_PATH" >&2
    exit 1
fi

echo "==> Assembling portable ZIP..."
rm -rf "$DIST_DIR"
mkdir -p "$DIST_DIR/bin"
cp "$BIN_PATH" "$DIST_DIR/bin/"
[[ -f README.md ]] && cp README.md "$DIST_DIR/"
[[ -f config.toml ]] && cp config.toml "$DIST_DIR/"

if command -v 7z >/dev/null 2>&1; then
    (cd "$DIST_DIR" && 7z a -tzip "$ARTIFACT" .)
elif command -v zip >/dev/null 2>&1; then
    (cd "$DIST_DIR" && zip -r "$ARTIFACT" .)
else
    echo "warning: neither 7z nor zip found; producing tar.gz fallback" >&2
    ARTIFACT="$DIST_DIR/nes-emu-windows-x86_64.tar.gz"
    tar -czf "$ARTIFACT" -C "$DIST_DIR" .
fi
echo "    -> $ARTIFACT"
echo "==> Done. Artifacts in $DIST_DIR/"
