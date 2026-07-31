/*
 * emulator.c - Emulator state: ties CPU, PPU, APU, bus, and cartridge
 *              together in a frame-locked loop.
 *
 * Port of src/emulator.rs to C (M4.5). The frame loop (step_frame /
 * step_one_cpu_tick / step_instruction) mirrors the Rust implementation
 * line-for-line: 1 CPU cycle = 3 PPU cycles, audio samples accumulated
 * from the APU + expansion audio, frame boundary detected by the PPU
 * scanline wrapping from the prerender scanline back to 0, leftover PPU
 * cycles carried into the next frame.
 *
 * See: https://www.nesdev.org/wiki/Cycle_reference
 */
#include "emulator.hpp"
#include "bus.hpp"
#include "ppu.hpp"
#include "ppu_render.hpp"
#include "apu.hpp"
#include "cartridge.hpp"
#include "region.hpp"
#include <stdlib.h>
#include <string.h>

/* Audio buffer capacity: NTSC ~735 samples/frame, PAL ~882; reserve
 * headroom. (emulator.rs `new_with_region`.) */
#define EMU_AUDIO_CAP_NTSC 800u
#define EMU_AUDIO_CAP_PAL  950u

/* ---- Construction ----------------------------------------------------- */

void emulator_init_with_region(EmulatorState* emu, Region region) {
    memset(emu, 0, sizeof(*emu));
    cpu_init(&emu->cpu);
    bus_init(&emu->bus);
    ppu_set_region(&emu->bus.ppu, region);
    apu_set_region(&emu->bus.apu, region);
    emu->region = region;
    emu->sample_accumulator = 0.0f;
    emu->ppu_cycle_carry = 0u;
    emu->cartridge = NULL;
    size_t cap = (region_scanlines_per_frame(region) > 262u)
                 ? EMU_AUDIO_CAP_PAL : EMU_AUDIO_CAP_NTSC;
    emu->audio_buffer = (float*)malloc(cap * sizeof(float));
    emu->audio_buffer_capacity = (emu->audio_buffer) ? cap : 0u;
    emu->audio_buffer_count = 0u;
}

void emulator_init(EmulatorState* emu) {
    emulator_init_with_region(emu, REGION_NTSC);
}

void emulator_destroy(EmulatorState* emu) {
    if (!emu) {
        return;
    }
    if (emu->cartridge) {
        cartridge_destroy(emu->cartridge);
        free(emu->cartridge);
        emu->cartridge = NULL;
    }
    if (emu->audio_buffer) {
        free(emu->audio_buffer);
        emu->audio_buffer = NULL;
    }
    emu->audio_buffer_count = 0u;
    emu->audio_buffer_capacity = 0u;
}

void emulator_reset(EmulatorState* emu) {
    cpu_reset(&emu->cpu, &emu->bus);
}

Region emulator_region(const EmulatorState* emu) {
    return emu->region;
}

void emulator_set_region(EmulatorState* emu, Region region) {
    emu->region = region;
    ppu_set_region(&emu->bus.ppu, region);
    apu_set_region(&emu->bus.apu, region);
}

/* ---- Audio buffer push helper (matches Rust Vec::push growth) -------- */

static void emu_push_audio(EmulatorState* emu, float sample) {
    if (emu->audio_buffer_count >= emu->audio_buffer_capacity) {
        /* Grow the buffer (doubling, with a sane minimum). */
        size_t new_cap = emu->audio_buffer_capacity * 2u;
        if (new_cap < 16u) {
            new_cap = 16u;
        }
        float* nb = (float*)realloc(emu->audio_buffer, new_cap * sizeof(float));
        if (!nb) {
            /* Allocation failure: drop the sample (matches Rust panicking
             * on OOM, but we degrade gracefully). */
            return;
        }
        emu->audio_buffer = nb;
        emu->audio_buffer_capacity = new_cap;
    }
    emu->audio_buffer[emu->audio_buffer_count++] = sample;
}

/* ---- Frame loop (emulator.rs step_one_cpu_tick) ---------------------- */

/* Run one CPU instruction plus its PPU/APU/mapper side-effects, returning
 * the CPU cycles consumed this tick and whether the frame just completed.
 * (emulator.rs `step_one_cpu_tick`.) */
static uint32_t emu_step_one_cpu_tick(EmulatorState* emu, uint16_t prev_scanline,
                                      float cycles_per_sample, uint16_t prerender,
                                      bool* out_frame_done) {
    uint32_t cpu_cycles = 0u;

    uint8_t step_cycles = cpu_step(&emu->cpu, &emu->bus);
    cpu_cycles += step_cycles;

    uint32_t dma_cycles = bus_take_dma_stall_cycles(&emu->bus);
    cpu_cycles += dma_cycles;

    /* Advance the bus's total CPU cycle counter for OAM-DMA alignment. */
    bus_advance_cpu_cycles(&emu->bus, step_cycles + dma_cycles);

    uint32_t apu_cycles = step_cycles + dma_cycles;
    bus_step_apu(&emu->bus, apu_cycles);
    bus_clock_cart_cpu(&emu->bus, apu_cycles);

    if (bus_apu_irq_pending(&emu->bus)) {
        cpu_set_irq_pending(&emu->cpu, true);
    }
    if (bus_cart_irq_pending(&emu->bus)) {
        cpu_set_irq_pending(&emu->cpu, true);
    }

    emu->sample_accumulator += (float)apu_cycles;
    while (emu->sample_accumulator >= cycles_per_sample) {
        emu->sample_accumulator -= cycles_per_sample;
        float internal = apu_output(&emu->bus.apu);
        float expansion = bus_expansion_audio_sample(&emu->bus);
        float mixed = internal + expansion;
        if (mixed < -1.0f) mixed = -1.0f;
        if (mixed > 1.0f)  mixed = 1.0f;
        emu_push_audio(emu, mixed);
    }

    uint32_t ppu_cycles = 3u * apu_cycles + emu->ppu_cycle_carry;
    emu->ppu_cycle_carry = 0u;
    uint32_t remaining = ppu_cycles;
    bool frame_done = false;
    while (remaining > 0u) {
        uint32_t chunk = remaining;
        if (chunk > PPU_CYCLES_PER_SCANLINE) {
            chunk = PPU_CYCLES_PER_SCANLINE;
        }
        bus_step_ppu(&emu->bus, chunk);
        remaining -= chunk;

        /* Consume any latched NMI request after each chunk. */
        if (bus_take_nmi_request(&emu->bus)) {
            cpu_set_nmi_pending(&emu->cpu, true);
        }

        /* Detect frame completion: the PPU scanline wrapped from the
         * prerender scanline to scanline 0. */
        uint16_t curr_scanline = ppu_scanline(&emu->bus.ppu);
        if (curr_scanline == 0u && prev_scanline == prerender) {
            emu->ppu_cycle_carry = remaining;
            frame_done = true;
            break;
        }
    }

    *out_frame_done = frame_done;
    return cpu_cycles;
}

uint32_t emulator_step_frame(EmulatorState* emu) {
    uint32_t cpu_cycles = 0u;

    /* Clear the framebuffer to the universal background color first so any
     * pixels not covered by the per-pixel path still have a valid color.
     * (emulator.rs `step_frame`.) */
    uint32_t universal_bg = ppu_universal_bg_argb(&emu->bus.ppu);
    ppu_clear_framebuffer(&emu->bus.ppu, universal_bg);
    ppu_reset_rendered_flag(&emu->bus.ppu);

    float cycles_per_sample = region_cpu_cycles_per_sample(emu->region);
    uint16_t prerender = region_scanline_prerender(emu->region);

    for (;;) {
        uint16_t prev_scanline = ppu_scanline(&emu->bus.ppu);
        bool frame_done = false;
        uint32_t tick_cycles = emu_step_one_cpu_tick(emu, prev_scanline,
                                                    cycles_per_sample, prerender,
                                                    &frame_done);
        cpu_cycles += tick_cycles;
        if (frame_done) {
            break;
        }
    }

    /* Fallback: if no per-pixel output happened, render the whole frame. */
    if (!ppu_rendered_this_frame(&emu->bus.ppu)) {
        bus_render_frame(&emu->bus);
    }

    return cpu_cycles;
}

uint32_t emulator_step_instruction(EmulatorState* emu) {
    uint16_t prev_scanline = ppu_scanline(&emu->bus.ppu);
    float cycles_per_sample = region_cpu_cycles_per_sample(emu->region);
    uint16_t prerender = region_scanline_prerender(emu->region);
    bool frame_done = false;
    return emu_step_one_cpu_tick(emu, prev_scanline, cycles_per_sample,
                                 prerender, &frame_done);
}

/* ---- Output ---------------------------------------------------------- */

const uint32_t* emulator_framebuffer(const EmulatorState* emu) {
    return ppu_framebuffer(&emu->bus.ppu);
}

uint16_t emulator_mapper_number(const EmulatorState* emu) {
    if (emu->cartridge) {
        return emu->cartridge->header.mapper_number;
    }
    return 0u;
}

size_t emulator_take_audio_samples(EmulatorState* emu, float* out, size_t cap) {
    size_t n = emu->audio_buffer_count;
    if (n > cap) {
        n = cap;
    }
    if (n > 0u && out) {
        memcpy(out, emu->audio_buffer, n * sizeof(float));
    }
    /* Clear the buffer (keep capacity) for the next frame. */
    emu->audio_buffer_count = 0u;
    return n;
}

/* ---- Save-state accessors ------------------------------------------- */

float emulator_sample_accumulator(const EmulatorState* emu) {
    return emu->sample_accumulator;
}

void emulator_set_sample_accumulator(EmulatorState* emu, float acc) {
    emu->sample_accumulator = acc;
}

const float* emulator_audio_buffer(const EmulatorState* emu) {
    return emu->audio_buffer;
}

size_t emulator_audio_buffer_count(const EmulatorState* emu) {
    return emu->audio_buffer_count;
}

void emulator_set_audio_buffer(EmulatorState* emu, const float* buf, size_t count) {
    /* Ensure capacity for `count` floats, then copy. */
    if (count > emu->audio_buffer_capacity) {
        float* nb = (float*)realloc(emu->audio_buffer, count * sizeof(float));
        if (!nb) {
            /* Allocation failure: keep the old buffer, drop the restore. */
            emu->audio_buffer_count = 0u;
            return;
        }
        emu->audio_buffer = nb;
        emu->audio_buffer_capacity = count;
    }
    if (count > 0u && buf) {
        memcpy(emu->audio_buffer, buf, count * sizeof(float));
    }
    emu->audio_buffer_count = count;
}

uint32_t emulator_ppu_cycle_carry(const EmulatorState* emu) {
    return emu->ppu_cycle_carry;
}

void emulator_set_ppu_cycle_carry(EmulatorState* emu, uint32_t carry) {
    emu->ppu_cycle_carry = carry;
}
