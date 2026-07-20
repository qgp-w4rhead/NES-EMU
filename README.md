# nes-emu

A Nintendo Entertainment System (NES) emulator written in Rust from scratch.
Every component — the 6502 CPU, PPU (picture processing unit), APU (audio
processing unit), memory bus, cartridge mappers, and SDL2 video/audio/input —
is hand-written. No external emulation libraries (nestopia, fceux, mednafen)
are used.

## Features

- **Cycle-accurate 6502 CPU** — all 151 official opcodes, 13 addressing
  modes, page-crossing cycle penalties, dummy reads, NMI/IRQ/RESET
  interrupts, and unofficial opcodes.
- **Cycle-accurate PPU** — per-pixel renderer with scanline timing, sprite
  zero hits, OAM DMA, 8×16 sprites, mid-scanline register writes, and
  vertical/horizontal/4-screen mirroring.
- **APU** — 2 pulse, 1 triangle, 1 noise, 1 DMC channel with frame counter
  (4-step/5-step), length counters, sweeps, envelopes, and non-linear
  mixing.
- **Expansion audio** — VRC6 (pulse+saw), VRC7 (YM2413 OPLL FM), Sunsoft 5B
  (YM2149 PSG), Namco 163 (wavetable), and FDS (wavetable + modulator).
- **17 cartridge mappers** — NROM (0), MMC1 (1), UxROM (2), CNROM (3),
  MMC3 (4), MMC5 (5), AxROM (7), MMC2 (9), Namco 163 (19), FDS (20), VRC6
  (24/26), FME-7 (69), VRC7 (85).
- **Famicom Disk System** — `.fds` disk image loading with BIOS detection.
- **Save states** — `.nessav` binary format with version migration; 10
  slots + rewind buffer.
- **Battery-backed SRAM** — `.nessram` persistence for battery carts.
- **ROM management** — drag-and-drop loading, recent ROM list, ROM info
  overlay, IPS patching, gzip/ZIP auto-decompression.
- **Regions** — NTSC, PAL, and Dendy with correct timing and palettes.
- **Debug tools** — CPU debugger (pause/step/breakpoints), PPU/nametable/
  pattern/OAM/palette viewers, memory hex editor, instruction trace logger.
- **On-screen display** — FPS, mapper, game name, save-state slot.
- **Screenshots** — PNG export (hand-written encoder).

## Performance

The emulation core runs one NTSC frame in **~1.4 ms** on a modern CPU —
well under the 16.67 ms real-time budget (a 10× margin). See
`benches/frame_bench.rs` for criterion benchmarks covering frame execution,
CPU instruction throughput, PPU rendering, and save-state serialization.

## Building

### Prerequisites

- **Rust** stable toolchain (install via [rustup](https://rustup.rs))
- **SDL2** development libraries:
  - **Linux (Debian/Ubuntu):** `sudo apt-get install libsdl2-dev`
  - **macOS:** `brew install sdl2`
  - **Windows:** the `sdl2` crate bundles prebuilt MSVC binaries — no extra
    install needed when building with the `x86_64-pc-windows-msvc` target.

### Build & run

```bash
cargo build --release                    # build optimized binary
cargo run --release -- --rom game.nes    # launch with a ROM
```

### Command-line options

```
nes-emu --rom <path-to-.nes> [--config <path-to-config.toml>]
```

If `--rom` is omitted the emulator prints usage and exits.

## Usage

### Default keyboard bindings (controller 1)

| NES button | Key |
|------------|-----|
| A          | Z   |
| B          | X   |
| Select     | A   |
| Start      | S   |
| Up         | Up arrow |
| Down       | Down arrow |
| Left       | Left arrow |
| Right      | Right arrow |

Gamepad support is enabled automatically when a controller is connected
(Xbox-style layout: A→A, B→B, Back→Select, Start→Start, D-pad→D-pad).
Bindings are configurable via `config.toml`.

### Hotkeys

| Key            | Action |
|----------------|--------|
| `F1`           | Toggle pause/resume (debugger). |
| `F2`           | Single-step CPU (while paused). |
| `F3`           | Toggle run-to-breakpoint mode. |
| `F4`           | Cycle PPU viewer (nametables → pattern → OAM → palettes). |
| `F5`           | Save state to current slot. |
| `F6`           | Dump memory viewer window. |
| `F7`           | Load state from current slot. |
| `F8`           | Toggle instruction trace logger. |
| `F9`           | Screenshot → PNG in CWD. |
| `F10`          | Toggle on-screen display. |
| `F11`          | Cycle region (NTSC → PAL → Dendy). |
| `F12`          | Toggle ROM info overlay. |
| `Alt+Enter`    | Toggle fullscreen. |
| `Ctrl+R`       | Soft reset. |
| `Tab`          | Toggle fast-forward (4× speed). |
| `Backspace`    | Rewind one frame. |
| `0`–`9`        | Select save-state slot. |
| `PageUp`/`Down`| Memory viewer page navigation. |
| `[` / `]`      | Switch memory viewer region (CPU ↔ PPU). |
| `Alt+<channel>`| Mute/unmute individual APU channels. |

### Drag-and-drop

Drag a `.nes` / `.fds` / `.gz` / `.zip` file onto the emulator window to
load it. The previous cartridge's battery SRAM is saved before the new ROM
is loaded. The 10 most recent ROMs are persisted to `config.toml`.

## Testing

```bash
cargo test                              # all unit + integration tests
cargo test --test cpu_tests             # CPU opcode tests
cargo test --test ppu_cycle_accurate    # PPU timing tests
cargo clippy --all-targets -- -D warnings   # lint (zero warnings)
cargo fmt --check                       # format check
cargo bench                             # criterion benchmarks
cargo bench --no-run                    # compile benchmarks only
```

The test suite includes 1500+ unit and integration tests covering CPU
opcodes, PPU rendering, APU synthesis, every mapper's bank-switching,
save-state round-trips, ROM management, and expansion audio.

## Packaging

Distributable packages are built by the scripts in `packaging/`:

```bash
./packaging/build_linux.sh              # → dist/nes-emu-linux-x86_64.tar.gz (+ AppImage)
./packaging/build_windows.sh            # → dist/nes-emu-windows-x86_64.zip
./packaging/build_macos.sh              # → dist/nes-emu-macos.tar.gz (.app bundle)
```

The GitHub Actions workflow (`.github/workflows/ci.yml`) automates this on
tag pushes (`v*`), building release binaries for Linux, Windows, and macOS
and attaching them to a GitHub Release.

## Project layout

```
src/
├── main.rs             — SDL2 entry point, main loop, event dispatch
├── emulator.rs         — EmulatorState: ties CPU/PPU/APU/bus together
├── cpu/                — 6502 CPU core (opcodes, addressing, interrupts)
├── ppu/                — PPU (registers, per-pixel renderer, scanline timing)
├── apu.rs              — APU (pulse/triangle/noise/DMC, frame counter)
├── bus.rs              — Memory bus (CPU/PPU/APU/cart routing, mirroring)
├── cartridge.rs        — iNES parsing, ROM loading, mapper dispatch
├── mappers/            — 17 mapper implementations + expansion audio chips
├── save_state/         — serde/bincode serialization, slots, rewind
├── debug/              — CPU debugger, PPU/memory viewers, trace logger
├── osd/                — on-screen display (FPS/mapper/slot)
├── video.rs            — SDL2 texture streaming, integer scaling
├── audio.rs            — SDL2 audio callback, ring buffer
├── input.rs            — SDL2 event → joypad mapping, gamepad support
├── config.rs           — TOML config (key bindings, volume, recent ROMs)
├── rom_manager.rs      — drag-and-drop, recent list, ROM info overlay
├── ips.rs              — IPS patch parser/applier
├── compression.rs      — hand-written DEFLATE/gzip/ZIP decompression
├── fds.rs              — FDS disk image parsing + BIOS loading
└── region.rs           — NTSC/PAL/Dendy region handling
benches/
└── frame_bench.rs      — criterion benchmarks (frame/CPU/save-state)
tests/                  — 1500+ integration tests
packaging/              — platform build scripts
.github/workflows/ci.yml — CI + release pipeline
```

## References

- [NESdev wiki](https://www.nesdev.org/wiki/) — the primary hardware reference.
- [Cycle reference](https://www.nesdev.org/wiki/Cycle_reference) — clock
  relationships and per-frame cycle counts.
- [CPU](https://www.nesdev.org/wiki/CPU) / [PPU](https://www.nesdev.org/wiki/PPU)
  / [APU](https://www.nesdev.org/wiki/APU) — per-component documentation.

## License

MIT
