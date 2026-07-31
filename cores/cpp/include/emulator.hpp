/*
 * emulator.h - Emulator state: ties CPU, PPU, APU, bus, and cartridge
 *              together in a frame-locked loop.
 *
 * Port of src/emulator.rs to C (M4.5). The EmulatorState owns the CPU, the
 * Bus (which owns the PPU, APU, RAM, open-bus, and joypad), and a
 * heap-allocated Cartridge. Audio samples produced during a frame are
 * accumulated in a heap-allocated float buffer drained by
 * emulator_take_audio_samples.
 *
 * See: https://www.nesdev.org/wiki/Cycle_reference
 */
#ifndef NES_CORE_C_EMULATOR_H
#define NES_CORE_C_EMULATOR_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>
#include "cpu.hpp"
#include "bus.hpp"
#include "cartridge.hpp"
#include "region.hpp"

#ifdef __cplusplus
extern "C" {
#endif

/* The complete NES emulator state - CPU + bus + audio accumulation.
 * (emulator.rs `struct EmulatorState`.) */
typedef struct EmulatorState {
    Cpu cpu;
    Bus bus;            /* owns PPU, APU, RAM, open-bus, joypad. cartridge ptr is non-owning. */
    Cartridge* cartridge;  /* owned by emulator (heap-allocated). NULL = no cart. */
    float sample_accumulator;
    float* audio_buffer;       /* heap-allocated dynamic array */
    size_t audio_buffer_count;
    size_t audio_buffer_capacity;
    Region region;
    uint32_t ppu_cycle_carry;
} EmulatorState;

/* Construct an emulator with no cartridge (NTSC region). The CPU is left in
 * its power-on state; call emulator_reset to load PC from the cartridge's
 * RESET vector. (emulator.rs `EmulatorState::new` with a placeholder cart.)
 * Use emulator_init_with_region for an explicit region. */
void emulator_init(EmulatorState* emu);

/* Construct an emulator with an explicit region (M32). The PPU and APU are
 * configured for the given region's timing and palette. (emulator.rs
 * `EmulatorState::new_with_region`.) */
void emulator_init_with_region(EmulatorState* emu, Region region);

/* Free heap-allocated resources (cartridge + audio_buffer). Safe to call on
 * a zeroed EmulatorState. (Rust `Drop` for EmulatorState.) */
void emulator_destroy(EmulatorState* emu);

/* Perform the 6502 RESET sequence: load PC from $FFFC/$FFFD, set SP=$FD,
 * set the I flag. (emulator.rs `EmulatorState::reset`.) */
void emulator_reset(EmulatorState* emu);

/* Current TV system / region. (emulator.rs `EmulatorState::region`.) */
Region emulator_region(const EmulatorState* emu);

/* Set the TV system / region (M32). Propagates to the PPU and APU.
 * (emulator.rs `EmulatorState::set_region`.) */
void emulator_set_region(EmulatorState* emu, Region region);

/* Run one full frame: steps the CPU and PPU in lockstep until the PPU
 * scanline wraps from the prerender scanline back to scanline 0, then
 * renders the framebuffer if the per-pixel renderer did not already fill
 * it. Returns the number of CPU cycles executed during this frame.
 * (emulator.rs `EmulatorState::step_frame`.) */
uint32_t emulator_step_frame(EmulatorState* emu);

/* Run one CPU instruction (with PPU/APU/mapper side-effects) and return
 * the CPU cycles consumed. Does NOT loop until a frame completes and does
 * NOT clear the framebuffer. (emulator.rs `EmulatorState::step_instruction`.) */
uint32_t emulator_step_instruction(EmulatorState* emu);

/* Borrow the framebuffer (256x240 ARGB). Owned by the PPU; valid for the
 * lifetime of the emulator. (emulator.rs `EmulatorState::framebuffer`.) */
const uint32_t* emulator_framebuffer(const EmulatorState* emu);

/* The loaded cartridge's iNES mapper number, or 0 if no cartridge is
 * loaded. (emulator.rs `EmulatorState::mapper_number`.) */
uint16_t emulator_mapper_number(const EmulatorState* emu);

/* Drain the audio sample buffer into the caller's float buffer. Copies up
 * to `cap` floats into `out`, clears the internal buffer (capacity
 * preserved), and returns the number of samples copied. (emulator.rs
 * `EmulatorState::take_audio_samples`, adapted to a caller buffer.) */
size_t emulator_take_audio_samples(EmulatorState* emu, float* out, size_t cap);

/* ---- Save-state accessors (emulator.rs sample_accumulator / audio_buffer) */

float emulator_sample_accumulator(const EmulatorState* emu);
void emulator_set_sample_accumulator(EmulatorState* emu, float acc);

const float* emulator_audio_buffer(const EmulatorState* emu);
size_t emulator_audio_buffer_count(const EmulatorState* emu);
void emulator_set_audio_buffer(EmulatorState* emu, const float* buf, size_t count);

uint32_t emulator_ppu_cycle_carry(const EmulatorState* emu);
void emulator_set_ppu_cycle_carry(EmulatorState* emu, uint32_t carry);

#ifdef __cplusplus
}
#endif
#endif /* NES_CORE_C_EMULATOR_H */
