# typescript-nes

A NES emulator core written in TypeScript, compiled to Node.js JavaScript and
exposed to the benchmark harness via a **subprocess binary protocol** over
stdin/stdout. Port of `cores/c/` (the C source of truth).

- **Impl name:** `typescript-nes`
- **Impl version:** `0.1.0`
- **Entry point:** `dist/main.js` (compiled from `src/main.ts`)
- **Target:** Node.js 20+ (tested on Node 24.14.0)

## Architecture

The core is a faithful port of the C reference at `cores/c/src/`. Components:

| File | Responsibility |
|------|----------------|
| `src/cpu.ts` | 6502 CPU registers, flags, interrupts, `step()` |
| `src/cpu_addressing.ts` | Addressing-mode resolvers + read/write helpers (prototype methods) |
| `src/cpu_opcodes.ts` | Official opcode handlers (load/store/arith/shift/branch/...) |
| `src/cpu_execute.ts` | Official opcode dispatch (`execute(opcode)`) |
| `src/cpu_unofficial.ts` | Unofficial / illegal opcode dispatch |
| `src/bus.ts` | CPU memory bus: RAM, PPU regs, APU/IO regs, OAM DMA, cart routing |
| `src/ppu.ts` | PPU registers, VRAM/OAM/palette, scanline/cycle timing, scroll |
| `src/ppu_render.ts` | PPU pixel pipeline: background + sprite rendering, palettes |
| `src/apu.ts` | APU frame counter, mixer, sample output |
| `src/apu_channels.ts` | Pulse / Triangle / Noise / DMC channels |
| `src/cartridge.ts` | iNES header parser + mapper dispatch |
| `src/mapper.ts` | `Mapper` abstract base, `Mirroring` enum, `createMapper` factory |
| `src/mappers/*.ts` | 13 mapper implementations (NROM, MMC1/2/3/5, UxROM, CNROM, AxROM, FME7, VRC6/7, Namco163, FDS) |
| `src/audio/*.ts` | Expansion audio: YM2149, OPLL (VRC7), FDS audio |
| `src/region.ts` | NTSC/PAL/Dendy timing constants |
| `src/joypad.ts` | Two-controller joypad state + strobe/shift |
| `src/emulator.ts` | `Emulator` class: ties CPU/PPU/APU/bus/cart together, frame loop |
| `src/save_state.ts` | Full-state binary serialization (magic + version, round-trip) |
| `src/main.ts` | Subprocess binary protocol entry point (M3 spec) |

### CPU prototype-method pattern

The CPU's addressing modes, opcode handlers, execute dispatch, and unofficial
opcodes are split across four files (`cpu_addressing.ts`, `cpu_opcodes.ts`,
`cpu_execute.ts`, `cpu_unofficial.ts`). Each file assigns methods to
`Cpu.prototype.*` (matching the C file structure). TypeScript does not
natively see these prototype assignments as part of the `Cpu` type, so each
file declares its additions via **declaration merging**:

```typescript
declare module './cpu' {
    interface Cpu {
        resolve(mode: number): Operand;
        // ... every method added in this file
    }
}
Cpu.prototype.resolve = function (mode: number): Operand { ... };
```

This preserves the existing code structure while making the prototype
assignments type-check. `AddrMode` / `Dummy` / `Mirroring` / `Region` are
plain `enum`s (not `const enum`) so they can be referenced across the module
augmentations.

### Frame loop

`Emulator.stepFrame()` mirrors `cores/c/src/emulator.c` line-for-line:

1. Clear the framebuffer to the universal background color.
2. Loop: run one CPU instruction (`cpu.step()`), consume any OAM-DMA stall
   cycles, advance the bus CPU-cycle counter, step the APU + mapper by the
   same cycle count, accumulate audio samples (internal APU + expansion
   audio), then step the PPU by `3 * apuCycles + ppuCycleCarry` cycles
   (chunked to one scanline at a time so the MMC3 IRQ clock fires correctly).
3. Detect frame completion when the PPU scanline wraps from the prerender
   scanline back to 0; carry leftover PPU cycles into the next frame.
4. Fallback: if the per-pixel renderer did not run this frame (e.g. PPU
   disabled), render the whole frame with `Bus.renderFrame()`.

`stepInstruction()` runs a single CPU tick (no frame loop, no framebuffer
clear) and returns its cycles.

## Build

```powershell
cd cores/typescript
npm install        # already done in this repo
npx tsc            # compiles src/ -> dist/
```

`npx tsc` must produce **zero errors**. The `tsconfig.json` targets ES2022
with `module: commonjs` (so `require()` works in Node), `strict: false`, and
`skipLibCheck: true`.

## Test

```powershell
node test/run_tests.js
```

Runs 12 test functions (21 assertions) against the compiled `dist/`:

- `step_frame` — load NOP ROM, step one frame, cycles > 0
- `step_instruction` — step one instruction, cycles > 0
- `mapper_number` — NOP ROM reports mapper 0
- `save_load_roundtrip` — 10 frames, save, 5 more, load, 5 more == fresh 15 frames
- `set_region` — NTSC/PAL/Dendy switching does not crash
- `1000_frame_run` — 1000 frames without crash + deterministic vs. a second run
- `audio_drain` — frames produce > 0 int16 audio samples
- `framebuffer` — framebuffer is 256*240 and not all zeros
- `reset` — after RESET, PC == $C000, SP == $FD, I flag set
- `determinism` — two 100-frame runs produce identical framebuffers
- `impl_name` / `impl_version` — identity strings
- `save_state_size_stable` — two consecutive saves produce identical bytes

All must pass (exit code 0).

## Run as a subprocess

The harness spawns this core with:

```powershell
node --max-old-space-size=64 --no-warnings dist/main.js
```

`main.js` reads binary messages from stdin and writes responses to stdout in
**binary mode** (no encoding set on the streams). Messages are
little-endian:

| Harness -> Core | Format |
|-----------------|--------|
| `INIT`          | `[0x01][rom_len:u32 LE][rom_bytes...]` |
| `RESET`         | `[0x02]` |
| `STEP_FRAME`    | `[0x03][count:u32 LE]` — run `count` frames, ONE result back |
| `STEP_INSTR`    | `[0x04][count:u32 LE]` — run `count` instrs, ONE result back |
| `SAVE_STATE`    | `[0x05]` |
| `LOAD_STATE`    | `[0x06][len:u32 LE][bytes...]` |

| Core -> Harness | Format |
|-----------------|--------|
| OK (after INIT/RESET/LOAD_STATE) | `[0x00]` |
| ERROR | `[0xFF][msg_len:u16 LE][msg utf8]` |
| RESULT (after STEP_FRAME/STEP_INSTR) | `[cycles:u32 LE][fb_len:u32 LE][fb raw bytes, fb_len*4][audio_len:u32 LE][audio int16 bytes, audio_len*2]` |
| SAVE_RESULT (after SAVE_STATE) | `[len:u32 LE][bytes...]` |

For a batch of `count` frames: `cycles` = total cycles across all frames,
`fb` = the **final** framebuffer after the batch, `audio` = **all** audio
samples accumulated across the batch (drained per-frame internally so the
buffer never overflows). `fb_len` is always `256*240` (pixel count);
`audio_len` is the int16 sample count.

## Optimization strategy (M10)

- **No allocation in the hot path.** All framebuffer / VRAM / OAM / palette /
  audio buffers are `TypedArray`s pre-allocated in the `Emulator` constructor
  (via `Bus` / `Ppu` / `Apu`). `stepFrame` / `stepInstruction` do not
  `new` anything.
- **TypedArrays everywhere.** `Uint8Array` for RAM/VRAM/OAM/palette,
  `Uint32Array` for the framebuffer (ARGB), `Float32Array` for the audio
  accumulation buffer, `Int16Array` for the drain buffer in `main.ts`.
- **Chunked PPU stepping.** PPU cycles are advanced one scanline at a time
  (max 341) so the MMC3 scanline IRQ counter fires at the right cycle and
  the frame-boundary check is cheap.
- **Pre-allocated audio drain buffer** in `main.ts` (`AUDIO_DRAIN_CAP =
  16384`) reused across STEP_FRAME batches; grows only if a batch produces
  more than the cap (rare).
- **Declaration merging** for CPU prototype methods keeps the V8 hidden-class
  stable (methods are installed once at module load, not per instance).

### Perf notes

- The per-pixel renderer (`PpuRender.renderOnePixel`) runs inline during PPU
  stepping when the PPU is rendering, so the framebuffer is filled
  incrementally; the end-of-frame `renderFrame()` fallback only runs when
  rendering was disabled the entire frame.
- Audio is mixed with the same pulse/TND formula and LPF/DC-removal filter as
  the C core, so output is byte-identical to the C# reference (which the
  harness compares against via `bench/fb_compare_sub.exe`).
- `cpu_cycle_count` is tracked as a JS `number` (double); it stays well
  below 2^32 for any realistic run, so the save-state u64 split into two u32s
  is safe.

## Save-state format

Custom binary format (NOT compatible with the C core's save states — it only
needs to round-trip within the TS core). Layout (all LE):

```
[4]   magic "NESS" (0x5353454E)
[4]   version (u32 = 1)
[4]   payload_size (u32)
--- payload ---
CPU (9 bytes: A,X,Y,Sp,Pc(2),Status,Flags,pad)
RAM (2048 bytes)
PPU arch state (vram_size(4) + vram + oam(256) + palette(32) + regs)
APU open-bus (24 bytes)
APU state (channels + frame counter + mixer)
Joypad (7 bytes)
Cartridge present flag + iNES header(12) + mapper state
dma_stall(4) + cpu_cycle_count(8) + sample_accumulator(4)
audio_buffer_count(4) + audio_buffer(count*4)
ppu_cycle_carry(4) + region(4)
```

The framebuffer, `bgPattern`, and `RenderPipeline` are NOT serialized (they
are derived data); after load, the framebuffer is cleared to the universal
background color and refilled on the next `stepFrame`.

## Limitations / deviations from C

- Save-state format is **not** byte-compatible with the C core (per M4.5
  spec — only round-trip within the TS core is required).
- `cpu_cycle_count` is a JS `number`, not a true u64; save-state serializes
  it as two u32s (low, high). Safe for any run < ~4 billion CPU cycles.
- FDS mapper requires BIOS + disk data via `createFdsMapper` (not exposed
  through the iNES `loadRom` path, matching the C core).
- The subprocess protocol does not support concurrent emulator instances
  (one `Emulator` per process, per M3 spec).
