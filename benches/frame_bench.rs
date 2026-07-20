//! Criterion benchmarks for the NES emulation core (M37).
//!
//! These benchmarks measure the hot paths that must stay within the
//! real-time budget of an NTSC frame (16.67 ms at 60.0988 Hz):
//!
//! - `step_frame` — one full NTSC frame (CPU + PPU + APU + bus + mapper).
//!   This is the single most important number: it must be < 16.67 ms on a
//!   modern CPU for the emulator to run at full speed.
//! - `cpu_step` — a single 6502 instruction (NOP) through the bus. Measures
//!   raw interpreter throughput.
//! - `ppu_render_frame` — the whole-frame scanline renderer fallback path
//!   (`Bus::render_frame`), used when the per-pixel renderer is disabled.
//! - `save_state` / `load_state` — bincode serialize/deserialize of the
//!   full `EmulatorState`. Must be fast enough that F5/F7 never cause a
//!   visible hitch.
//!
//! The benchmarks build a minimal NROM-128 cartridge in memory (16 KB PRG
//! filled with NOPs, 8 KB CHR-RAM, RESET vector → $C000) so they need no
//! external ROM fixtures and are deterministic.
//!
//! Run with:
//! ```text
//! cargo bench                   # run all benchmarks
//! cargo bench -- step_frame     # run only the frame benchmark
//! cargo bench --no-run          # compile benchmarks without running
//! ```
//!
//! See: https://www.nesdev.org/wiki/Cycle_reference (frame timing budget)
//! See: https://bheisler.github.io/criterion.rs/book/ (criterion API)

use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use nes_emu::cartridge::{Cartridge, PRG_ROM_UNIT};
use nes_emu::emulator::EmulatorState;

/// Build a minimal NROM-128 cartridge (16 KB PRG, 8 KB CHR-RAM) whose PRG
/// is filled with `0xEA` (NOP) and whose RESET vector points to `$C000`.
/// The cartridge is deterministic — the same bytes every run — so
/// benchmark results are stable and comparable across machines.
fn make_nop_cart() -> Cartridge {
    let mut bytes = vec![b'N', b'E', b'S', 0x1A, 1, 0, 0, 0];
    bytes.extend_from_slice(&[0u8; 8]); // remaining header
    bytes.resize(16 + PRG_ROM_UNIT, 0xEA); // PRG filled with NOP
                                           // RESET vector → $C000 (mirrors $8000 in NROM-128).
    let reset_off = 16 + 0x3FFC;
    bytes[reset_off] = 0x00;
    bytes[reset_off + 1] = 0xC0;
    // NMI and IRQ vectors → $0000 (unused).
    Cartridge::from_bytes(&bytes).expect("build NOP cart")
}

/// Build a deterministic emulator with the NOP cartridge, reset, and run
/// one warm-up frame so the PPU/APU/mapper are in a steady state before
/// measurement begins.
fn build_emu() -> EmulatorState {
    let mut emu = EmulatorState::new(make_nop_cart());
    emu.reset();
    // Warm up: run one frame so caches/branch predictors are primed and
    // the PPU is at a clean scanline-0 frame boundary.
    emu.step_frame();
    emu
}

/// Benchmark one full NTSC frame (`step_frame`). This is the headline
/// real-time budget number: it must complete in under 16.67 ms for the
/// emulator to sustain 60 FPS.
fn bench_step_frame(c: &mut Criterion) {
    let mut g = c.benchmark_group("step_frame");
    g.sample_size(50);
    // One NTSC frame ≈ 29,830 CPU cycles = 89,342 PPU cycles.
    g.bench_function("ntsc_frame", |b| {
        b.iter_batched(
            build_emu,
            |mut emu| {
                let cycles = emu.step_frame();
                black_box(cycles);
                black_box(emu.framebuffer());
            },
            criterion::BatchSize::SmallInput,
        );
    });
    g.finish();
}

/// Benchmark raw 6502 instruction throughput — a single `cpu.step` (NOP)
/// including all bus/PPU/APU side-effects for one CPU tick. Measures the
/// interpreter's per-instruction overhead.
fn bench_cpu_step(c: &mut Criterion) {
    let mut g = c.benchmark_group("cpu_step");
    // 1, 10, 100 instructions to measure per-instruction cost amortized
    // over a small batch.
    for &count in &[1u32, 10, 100] {
        g.bench_with_input(BenchmarkId::new("nop", count), &count, |b, &count| {
            b.iter_batched(
                build_emu,
                |mut emu| {
                    let mut total = 0u32;
                    for _ in 0..count {
                        total = total.wrapping_add(emu.step_instruction());
                    }
                    black_box(total);
                },
                criterion::BatchSize::SmallInput,
            );
        });
    }
    g.finish();
}

/// Benchmark the whole-frame scanline renderer (`Bus::render_frame`).
/// This is the fallback path used when the per-pixel renderer has not
/// already filled the framebuffer; it draws all 240 visible scanlines in
/// one pass.
fn bench_render_frame(c: &mut Criterion) {
    let mut g = c.benchmark_group("render_frame");
    g.bench_function("scanline_renderer", |b| {
        b.iter_batched(
            build_emu,
            |mut emu| {
                // Force the whole-frame renderer by clearing the rendered
                // flag, then call render_frame via the bus.
                emu.bus_mut().ppu_mut().reset_rendered_flag();
                emu.bus_mut().render_frame();
                black_box(emu.framebuffer());
            },
            criterion::BatchSize::SmallInput,
        );
    });
    g.finish();
}

/// Benchmark save-state serialization (`save_state`) and deserialization
/// (`load_state`). These must be fast enough that the F5/F7 hotkeys never
/// cause a visible frame hitch (< 16.67 ms).
fn bench_save_state(c: &mut Criterion) {
    let mut g = c.benchmark_group("save_state");
    g.bench_function("serialize", |b| {
        b.iter_batched(
            build_emu,
            |emu| {
                let blob = emu.save_state().expect("serialize");
                black_box(blob);
            },
            criterion::BatchSize::SmallInput,
        );
    });
    g.bench_function("deserialize", |b| {
        b.iter_batched(
            || {
                let emu = build_emu();
                let blob = emu.save_state().expect("serialize for deserialise bench");
                (emu, blob)
            },
            |(mut emu, blob)| {
                emu.load_state(&blob).expect("deserialize");
                black_box(emu.framebuffer());
            },
            criterion::BatchSize::SmallInput,
        );
    });
    g.finish();
}

/// Benchmark a round-trip save-state (serialize + deserialize) to measure
/// the combined cost of the F5→F7 workflow.
fn bench_save_state_round_trip(c: &mut Criterion) {
    let mut g = c.benchmark_group("save_state_round_trip");
    g.bench_function("serialize_then_deserialize", |b| {
        b.iter_batched(
            build_emu,
            |mut emu| {
                let blob = emu.save_state().expect("serialize");
                emu.load_state(&blob).expect("deserialize");
                black_box(emu.framebuffer());
            },
            criterion::BatchSize::SmallInput,
        );
    });
    g.finish();
}

criterion_group!(
    benches,
    bench_step_frame,
    bench_cpu_step,
    bench_render_frame,
    bench_save_state,
    bench_save_state_round_trip,
);
criterion_main!(benches);
