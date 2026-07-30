# nes-emu

A Nintendo Entertainment System (NES) emulator written in Rust from scratch.
Every component - the 6502 CPU, PPU (picture processing unit), APU (audio
processing unit), memory bus, cartridge mappers, and SDL2 video/audio/input -
is hand-written. No external emulation libraries (nestopia, fceux, mednafen)
are used.

## Features

- **Cycle-accurate 6502 CPU** - all 151 official opcodes, 13 addressing
  modes, page-crossing cycle penalties, dummy reads, NMI/IRQ/RESET
  interrupts, and unofficial opcodes.
- **Cycle-accurate PPU** - per-pixel renderer with scanline timing, sprite
  zero hits, OAM DMA, 8x16 sprites, mid-scanline register writes, and
  vertical/horizontal/4-screen mirroring.
- **APU** - 2 pulse, 1 triangle, 1 noise, 1 DMC channel with frame counter
  (4-step/5-step), length counters, sweeps, envelopes, and non-linear
  mixing. Per-channel mute/volume control via hotkeys.
- **Expansion audio** - VRC6 (pulse+saw), VRC7 (YM2413 OPLL FM), Sunsoft 5B
  (YM2149 PSG), Namco 163 (wavetable), and FDS (wavetable + modulator).
- **13 cartridge mappers** - NROM (0), MMC1 (1), UxROM (2), CNROM (3),
  MMC3 (4), MMC5 (5), AxROM (7), MMC2 (9), Namco 163 (19), FDS (20), VRC6
  (24/26), FME-7 (69), VRC7 (85).
- **Famicom Disk System** - `.fds` disk image loading with BIOS detection.
- **Save states** - `.nessav` binary format with version migration; 10
  slots + rewind buffer (default 300 frames, configurable up to 60000).
- **Rewind & timeline branching** - hold Backspace to rewind,
  Shift+Backspace to forward; variable speed (1x/2x/4x/8x cycled with
  Space); up to 5 alternate timeline branches with accept/deny at
  divergence points; visual timeline overlay bar.
- **Turbo & fast-forward** - hold Space for turbo speed (configurable
  0.25x-4x via F12 menu); Tab toggles fast-forward (4 frames per vsync).
- **Battery-backed SRAM** - `.nessram` persistence for battery carts.
- **ROM management** - drag-and-drop loading, recent ROM list, ROM info
  overlay, IPS patching, gzip/ZIP auto-decompression.
- **Regions** - NTSC, PAL, and Dendy with correct timing and palettes.
- **Debug tools** - CPU debugger (pause/step/breakpoints), PPU/nametable/
  pattern/OAM/palette viewers, memory hex editor, instruction trace
  logger, PPU write logger, ring buffer trace (last 10,000 instructions
  flushed on pause).
- **Debug bug injection** - slant corruption, inaccurate palette, NMI
  retrigger bug (toggleable via F12 menu for visual regression testing). Just for fun.
- **In-emulator menu** (F12) - key binding configuration, debug flag
  toggles, turbo speed selection, rewind capacity adjustment.
- **On-screen display** - FPS, mapper, region, game name, save-state slot
  (with occupied/empty status), rewind buffer depth.
- **Gamepad support** - automatic when a controller is connected
  (Xbox-style layout: A->A, B->B, Back->Select, Start->Start, D-pad->D-pad).
- **Configurable key bindings** - via `config.toml` and in-emulator menu.
- **Screenshots** - PNG export (hand-written encoder).

## Performance

The emulation core runs one NTSC frame in **~1.4 ms** on a modern CPU -
well under the 16.67 ms real-time budget (a 10× margin). See
`benches/frame_bench.rs` for criterion benchmarks covering frame execution,
CPU instruction throughput, PPU rendering, and save-state serialization.

## Building

### Prerequisites

- **Rust** stable toolchain (install via [rustup](https://rustup.rs))
- **SDL2** development libraries:
  - **Linux (Debian/Ubuntu):** `sudo apt-get install libsdl2-dev`
  - **macOS:** `brew install sdl2`
  - **Windows:** the `sdl2` crate bundles prebuilt MSVC binaries - no extra
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
| A          | L |
| B          | K |
| Select     | H |
| Start      | G |
| Up         | W |
| Down       | S |
| Left       | A |
| Right      | D |

Gamepad support is enabled automatically when a controller is connected
(Xbox-style layout: A→A, B→B, Back→Select, Start→Start, D-pad→D-pad).
Bindings are configurable via `config.toml`.

### Hotkeys

| Key                  | Action |
|----------------------|--------|
| `F1`                 | Toggle help overlay. |
| `F2`                 | Toggle pause/resume (debugger). |
| `F3`                 | Toggle run-to-breakpoint mode. |
| `F4`                 | Cycle PPU viewer (nametables → pattern → OAM → palettes). |
| `F5`                 | Save state to current slot. |
| `F6`                 | Dump memory viewer window. |
| `F7`                 | Load state from current slot. |
| `F8`                 | Toggle instruction trace logger. |
| `F9`                 | Screenshot → PNG in `screenshots/` directory. |
| `F10`                | Toggle on-screen display. |
| `F11`                | Cycle region (NTSC → PAL → Dendy). |
| `F12`                | Toggle in-emulator menu (ROM info, key bindings, debug flags). |
| `Alt+Enter`          | Toggle fullscreen. |
| `Ctrl+R`             | Soft reset. |
| `Tab`                | Toggle fast-forward (4 frames per vsync tick). |
| `Space`              | Hold for turbo speed (default 2×, configurable via F12 menu). |
| `Backspace`          | Hold to rewind. |
| `Shift+Backspace`    | Hold to forward through rewind buffer. |
| `N`                  | Single-step CPU (when paused via F2). |
| `T`                  | Toggle PPU write logger. |
| `O`                  | Accept branch point (when paused during rewind, branching on). |
| `P`                  | Deny branch point (continue past branch, branching on). |
| `0`–`9`              | Select save-state slot. |
| `PageUp`/`PageDown`  | Memory viewer page navigation. |
| `[` / `]`            | Switch memory viewer region (CPU ↔ PPU). |
| `Alt+1`–`Alt+5`      | Select APU channel (pulse1, pulse2, triangle, noise, DMC). |
| `Alt+M`              | Mute/unmute selected APU channel. |
| `Alt+Up`/`Alt+Down`  | Volume up/down on selected APU channel. |
| `Alt+0`              | Reset all APU channels (unmute, volume 1.0). |
| `Escape` / `Q`       | Quit emulator. |

### Rewind / Forward

Hold `Backspace` to rewind through recent frames. Hold `Shift+Backspace` to
fast-forward back through rewound frames. Press `Space` while rewinding to
cycle the speed (1× → 2× → 4× → 8×).

**Timeline branching** can be toggled on or off via the F12 menu under
**Rewind Settings → Branching**, or via `rewind_branching` in `config.toml`:

- **Branching ON** (default): Releasing `Backspace` after rewinding creates
  an alternate timeline branch. Rewinding past a branch divergence point
  pauses emulation — press `O` to accept (switch to that branch) or `P` to
  deny (continue past it).
- **Branching OFF**: Rewind/forward operates as a simple linear buffer with
  no branching. No branch points are created or paused at — just pure
  rewind/forward like a standard emulator.

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
├── main.rs              - SDL2 entry point, main loop, event dispatch
├── emulator.rs          - EmulatorState: ties CPU/PPU/APU/bus together
├── cpu/                 - 6502 CPU core (opcodes, addressing, interrupts)
...etc
benches/
└── frame_bench.rs       - criterion benchmarks (frame/CPU/save-state)
tests/                   - 1500+ integration tests
packaging/               - platform build scripts
.github/workflows/ci.yml - CI + release pipeline
```

## References

- [NESdev wiki](https://www.nesdev.org/wiki/) - the primary hardware reference.
- [Cycle reference](https://www.nesdev.org/wiki/Cycle_reference) - clock
  relationships and per-frame cycle counts.
- [CPU](https://www.nesdev.org/wiki/CPU) / [PPU](https://www.nesdev.org/wiki/PPU)
  / [APU](https://www.nesdev.org/wiki/APU) - per-component documentation.
- [NESDEV Emulator tests](https://www.nesdev.org/wiki/Emulator_tests) - test ROMs and validation procedures.

## License

MIT
