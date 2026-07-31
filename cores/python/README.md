# Python NES Core (Cython) — M11

Full NES emulator core ported from the C reference (`cores/c/`) to **Cython 3.x**,
compiled to a native `.pyd` extension module, running as a subprocess speaking
the M3 binary protocol over stdin/stdout.

## Architecture

The entire emulator lives in a single Cython source file
(`nes_core/_core.pyx`, ~4800 lines) so all `cdef` types can reference each other
at C level with no Python object traffic in the hot path. The compiled extension
(`nes_core/_core.cp313-win_amd64.pyd`) is driven by a pure-Python protocol loop
(`nes_core/protocol.py`) that implements the M3 subprocess binary protocol.

### Components

- **CPU** (6502): 151 official + 105 unofficial opcodes, 13 addressing modes,
  interrupts (NMI/IRQ/RESET), cycle-accurate timing with page-crossing penalties.
- **PPU**: registers, VRAM, OAM, palette, per-pixel render pipeline, NTSC/PAL/
  Dendy palettes, scroll latches, sprite evaluation.
- **APU**: 5 channels (2 pulse, 1 triangle, 1 noise, 1 DMC), frame counter
  (4-step/5-step), non-linear mixer, LPF, DC blocker, box-filter decimation.
- **Bus**: CPU memory routing ($0000-$FFFF), PPU register dispatch ($2000-$3FFF),
  APU/IO registers ($4000-$4017), OAM-DMA ($4014), cartridge space ($4020-$FFFF).
- **Cartridge**: iNES loader, 13 mappers (NROM, UxROM, CNROM, MMC1, MMC3, MMC5,
  MMC2, AxROM, FME-7, VRC6, VRC7, Namco163, FDS) + 4 expansion audio chips.
- **Save State**: manual binary serialization (magic "NESS", version 1),
  round-trips within this core only.

### Cython optimization

- `cdef` types everywhere in hot paths (`uint8_t`, `uint16_t`, `uint32_t`)
- `@cython.boundscheck(False)`, `@cython.wraparound(False)` globally
- `cdef inline` functions for hot paths
- `nogil` blocks around `step_frame` / `step_instruction` (releases GIL)
- No Python object creation in `step_frame` (verified via `sys.getrefcount`)
- Compiler flags: `/O3` via `extra_compile_args` in setup.py

## Build

```powershell
cd cores/python
python setup.py build_ext --inplace
```

Produces `nes_core/_core.cp313-win_amd64.pyd`.

### Requirements

- Python 3.13+
- Cython 3.0+ (`pip install cython setuptools wheel`)
- MSVC Build Tools 2022 (cl.exe/link.exe)

## Test

```powershell
cd cores/python
python test/run_tests.py
```

18 assertions across 11 test functions: step_frame, step_instruction,
mapper_number, save/load roundtrip, set_region, 1000-frame run, audio drain,
framebuffer, reset, determinism, no-Python-object-creation.

## Validate (byte-identical to C core)

```powershell
# From repo root:
$env:PYTHONPATH = "cores\python"
.\bench\fb_compare_sub.exe .\cores\c\nes_core_c.dll "python -m nes_core" 20
```

Result: 0 framebuffer/audio/cycle mismatches across 20 frames.

## Benchmark

```powershell
$env:PYTHONPATH = "cores\python"
.\bench\harness.exe --subprocess-bench "python -m nes_core" --subprocess-name "python-cython"
```

### Performance (vs C core)

| Benchmark    | Cython (this)  | C core   | Ratio  |
|--------------|----------------|----------|--------|
| step_frame   | 78315 us/frame | 2080 us  | 37.7x  |
| cpu_step     | 7417 ns/instr  | 163 ns   | 45.5x  |
| save_state   | 231 us         | 2.3 us   | 100x   |
| render_frame | 77846 us/frame | 2080 us  | 37.4x  |

This is the **slowest** core — that is the point (establishes the baseline for
the cross-language comparison). Even with Cython's C-level types and `nogil`,
the overhead of the Cython-generated C code (type conversions, bounds checking
on C-level array access via struct copies, Python object wrappers for `cdef
class` methods) is significant compared to hand-written C.

## File structure

```
cores/python/
  setup.py              — cythonize + build_ext
  Makefile              — convenience targets
  README.md             — this file
  requirements.txt      — cython>=3.0
  nes_core/
    __init__.py         — package init, exports Emulator
    __main__.py         — `python -m nes_core` entry point
    protocol.py         — M3 subprocess binary protocol loop
    _core.pyx           — full Cython NES core (CPU+PPU+APU+Bus+Mappers+SaveState)
    _core.c             — generated C (by cythonize)
    _core.cp313-win_amd64.pyd — compiled extension
  test/
    run_tests.py        — smoke tests
```
