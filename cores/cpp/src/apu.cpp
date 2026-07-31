/*
 * apu.c - Audio Processing Unit: pulse, triangle, noise, and DMC channels.
 *
 * Port of src/apu.rs to C (milestone M4.3). Field-for-field and
 * function-for-function port. See apu.h for the public API and
 * src/apu.rs for the original Rust implementation.
 *
 * See: https://www.nesdev.org/wiki/APU
 */
#include "apu.hpp"
#include "region.hpp"

#include <string.h>

/* Forward declarations of private helpers used before definition. */
static float apu_mix_raw(const Apu* a);

/* ---- Constants (src/apu.rs lines 8-37, 711-714) ---------------------- */

const uint8_t APU_DUTY_PATTERNS[4][8] = {
    { 0, 1, 0, 0, 0, 0, 0, 0 }, /* 0: 12.5%  */
    { 0, 1, 1, 0, 0, 0, 0, 0 }, /* 1: 25%    */
    { 0, 1, 1, 1, 1, 0, 0, 0 }, /* 2: 50%    */
    { 1, 0, 0, 0, 1, 1, 1, 1 }  /* 3: 25% negated */
};

const uint8_t APU_LENGTH_TABLE[32] = {
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14,
    12, 16, 24, 18, 48, 20, 96, 22, 192, 24, 72, 26, 16, 28, 32, 30
};

const uint8_t APU_TRIANGLE_SEQUENCE[32] = {
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0,
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15
};

const uint16_t APU_NOISE_PERIOD_TABLE[16] = {
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068
};

const uint16_t APU_DMC_RATE_TABLE[16] = {
    214, 190, 170, 160, 149, 138, 127, 113, 107, 95, 85, 80, 71, 63, 54, 42
};

/* ---- PulseChannel (src/apu.rs lines 41-312) -------------------------- */

void pulse_init(PulseChannel* c, bool pulse2) {
    memset(c, 0, sizeof(*c));
    c->pulse2 = pulse2;
}

void pulse_write_register(PulseChannel* c, uint8_t reg, uint8_t value) {
    switch (reg) {
        case 0u:
            /* $4000/$4004: DDLC VVVV */
            c->duty = (uint8_t)((value >> 6) & 0x03u);
            c->halt = (value & 0x20u) != 0u;
            c->constant_volume = (value & 0x10u) != 0u;
            c->volume = (uint8_t)(value & 0x0Fu);
            break;
        case 1u:
            /* $4001/$4005: EPPP NSSS - sweep params + reload flag. */
            c->sweep_enabled = (value & 0x80u) != 0u;
            c->sweep_period = (uint8_t)((value >> 4) & 0x07u);
            c->sweep_negate = (value & 0x08u) != 0u;
            c->sweep_shift = (uint8_t)(value & 0x07u);
            c->sweep_reload = true;
            break;
        case 2u:
            /* $4002/$4006: timer low 8 bits. */
            c->timer_period = (uint16_t)((c->timer_period & 0xFF00u) | (uint16_t)value);
            break;
        case 3u: {
            /* $4003/$4007: LLLL LTTT - length load + timer high 3 bits. */
            uint16_t high = (uint16_t)(value & 0x07u);
            c->timer_period = (uint16_t)((c->timer_period & 0x00FFu) | (uint16_t)(high << 8));
            /* Running timer's high 3 bits set immediately; low 8 bits
             * unaffected until next tick. */
            c->timer = (uint16_t)((c->timer & 0x00FFu) | (uint16_t)(high << 8));
            /* Length counter load (only if enabled via $4015). */
            if (c->enabled) {
                uint8_t idx = (uint8_t)(value >> 3);
                c->length_counter = APU_LENGTH_TABLE[idx];
            }
            /* Side effects: envelope restart + sequencer reset. */
            c->envelope_start = true;
            c->sequence = 0u;
            break;
        }
        default:
            break;
    }
}

void pulse_set_enabled(PulseChannel* c, bool enabled) {
    c->enabled = enabled;
    if (!enabled) {
        c->length_counter = 0u;
    }
}

/* sweep_target: compute sweep target period (wrapping). */
uint16_t pulse_sweep_target(const PulseChannel* c) {
    if (!c->sweep_enabled || c->sweep_shift == 0u) {
        return c->timer_period;
    }
    uint16_t shifted = (uint16_t)(c->timer_period >> c->sweep_shift);
    if (c->sweep_negate) {
        if (c->pulse2) {
            return (uint16_t)((uint16_t)(c->timer_period - shifted) - 1u);
        }
        return (uint16_t)(c->timer_period - shifted);
    }
    return (uint16_t)(c->timer_period + shifted);
}

/* is_muted: period < 8 or target > 11-bit range. */
bool pulse_is_muted(const PulseChannel* c) {
    return c->timer_period < APU_MIN_AUDIBLE_PERIOD || pulse_sweep_target(c) > APU_MAX_PERIOD;
}

void pulse_tick(PulseChannel* c) {
    if (c->timer == 0u) {
        c->timer = c->timer_period;
        c->sequence = (uint8_t)((c->sequence + 1u) & 0x07u);
    } else {
        c->timer = (uint16_t)(c->timer - 1u);
    }
}

void pulse_clock_envelope(PulseChannel* c) {
    if (c->envelope_start) {
        c->envelope_start = false;
        c->envelope_decay = 15u;
        c->envelope_divider = c->volume;
    } else if (c->envelope_divider == 0u) {
        c->envelope_divider = c->volume;
        if (c->envelope_decay > 0u) {
            c->envelope_decay = (uint8_t)(c->envelope_decay - 1u);
        } else if (c->halt) {
            c->envelope_decay = 15u; /* loop */
        }
    } else {
        c->envelope_divider = (uint8_t)(c->envelope_divider - 1u);
    }
}

static void pulse_clock_length(PulseChannel* c) {
    if (!c->halt && c->length_counter > 0u) {
        c->length_counter = (uint8_t)(c->length_counter - 1u);
    }
}

static void pulse_clock_sweep(PulseChannel* c) {
    bool divider_zero = c->sweep_divider == 0u;
    /* Step 1: apply target only when divider reached zero. */
    if (divider_zero && c->sweep_enabled && c->sweep_shift != 0u) {
        uint16_t target = pulse_sweep_target(c);
        if (target <= APU_MAX_PERIOD && c->timer_period >= APU_MIN_AUDIBLE_PERIOD) {
            c->timer_period = target;
        }
    }
    /* Step 2: reload or decrement the divider. */
    if (divider_zero || c->sweep_reload) {
        c->sweep_divider = c->sweep_period;
        c->sweep_reload = false;
    } else {
        c->sweep_divider = (uint8_t)(c->sweep_divider - 1u);
    }
}

void pulse_clock_quarter_frame(PulseChannel* c) {
    pulse_clock_envelope(c);
}

void pulse_clock_half_frame(PulseChannel* c) {
    pulse_clock_length(c);
    pulse_clock_sweep(c);
}

uint8_t pulse_sample(const PulseChannel* c) {
    if (!c->enabled || c->length_counter == 0u || pulse_is_muted(c)) {
        return 0u;
    }
    uint8_t duty_bit = APU_DUTY_PATTERNS[c->duty][c->sequence];
    if (duty_bit == 0u) {
        return 0u;
    }
    return c->constant_volume ? c->volume : c->envelope_decay;
}

bool pulse_enabled(const PulseChannel* c) { return c->enabled; }
uint8_t pulse_length_counter(const PulseChannel* c) { return c->length_counter; }
uint16_t pulse_timer_period(const PulseChannel* c) { return c->timer_period; }
uint16_t pulse_timer(const PulseChannel* c) { return c->timer; }
uint8_t pulse_envelope_decay(const PulseChannel* c) { return c->envelope_decay; }
uint8_t pulse_sequence(const PulseChannel* c) { return c->sequence; }

/* ---- TriangleChannel (src/apu.rs lines 317-486) ---------------------- */

void triangle_init(TriangleChannel* c) {
    memset(c, 0, sizeof(*c));
}

void triangle_write_register(TriangleChannel* c, uint8_t reg, uint8_t value) {
    switch (reg) {
        case 0u:
            /* $4008: Clll llll - halt + linear counter reload value. */
            c->halt = (value & 0x80u) != 0u;
            c->linear_reload = (uint8_t)(value & 0x7Fu);
            break;
        case 1u:
            /* $4009: unused. */
            break;
        case 2u:
            /* $400A: timer low 8 bits. */
            c->timer_period = (uint16_t)((c->timer_period & 0xFF00u) | (uint16_t)value);
            break;
        case 3u: {
            /* $400B: LLLL LTTT - length load + timer high 3 bits. */
            uint16_t high = (uint16_t)(value & 0x07u);
            c->timer_period = (uint16_t)((c->timer_period & 0x00FFu) | (uint16_t)(high << 8));
            c->timer = (uint16_t)((c->timer & 0x00FFu) | (uint16_t)(high << 8));
            if (c->enabled) {
                uint8_t idx = (uint8_t)(value >> 3);
                c->length_counter = APU_LENGTH_TABLE[idx];
            }
            /* Side effects: linear counter restart + sequencer reset. */
            c->linear_start = true;
            c->sequence = 0u;
            break;
        }
        default:
            break;
    }
}

void triangle_set_enabled(TriangleChannel* c, bool enabled) {
    c->enabled = enabled;
    if (!enabled) {
        c->length_counter = 0u;
    }
}

void triangle_tick(TriangleChannel* c) {
    if (c->timer == 0u) {
        c->timer = c->timer_period;
        c->sequence = (uint8_t)((c->sequence + 1u) & 0x1Fu);
    } else {
        c->timer = (uint16_t)(c->timer - 1u);
    }
}

static void triangle_clock_linear(TriangleChannel* c) {
    if (c->linear_start) {
        c->linear_start = false;
        c->linear_counter = c->linear_reload;
    } else if (c->linear_counter > 0u) {
        c->linear_counter = (uint8_t)(c->linear_counter - 1u);
    }
    if (c->halt) {
        c->linear_start = true;
    }
}

static void triangle_clock_length(TriangleChannel* c) {
    if (!c->halt && c->length_counter > 0u) {
        c->length_counter = (uint8_t)(c->length_counter - 1u);
    }
}

void triangle_clock_quarter_frame(TriangleChannel* c) {
    triangle_clock_linear(c);
}

void triangle_clock_half_frame(TriangleChannel* c) {
    triangle_clock_length(c);
}

uint8_t triangle_sample(const TriangleChannel* c) {
    if (!c->enabled || c->length_counter == 0u || c->linear_counter == 0u) {
        return 0u;
    }
    return APU_TRIANGLE_SEQUENCE[c->sequence];
}

bool triangle_enabled(const TriangleChannel* c) { return c->enabled; }
uint8_t triangle_length_counter(const TriangleChannel* c) { return c->length_counter; }
uint8_t triangle_linear_counter(const TriangleChannel* c) { return c->linear_counter; }
uint16_t triangle_timer_period(const TriangleChannel* c) { return c->timer_period; }
uint16_t triangle_timer(const TriangleChannel* c) { return c->timer; }
uint8_t triangle_sequence(const TriangleChannel* c) { return c->sequence; }

/* ---- NoiseChannel (src/apu.rs lines 491-709) ------------------------- */

void noise_init(NoiseChannel* c) {
    memset(c, 0, sizeof(*c));
    c->timer_period = APU_NOISE_PERIOD_TABLE[0];
    c->lfsr = 1u;
}

void noise_write_register(NoiseChannel* c, uint8_t reg, uint8_t value) {
    switch (reg) {
        case 0u:
            /* $400C: --LC VVVV */
            c->halt = (value & 0x20u) != 0u;
            c->constant_volume = (value & 0x10u) != 0u;
            c->volume = (uint8_t)(value & 0x0Fu);
            break;
        case 1u:
            /* $400D: unused. */
            break;
        case 2u:
            /* $400E: L--- PPPP - mode + period select. */
            c->mode = (value & 0x80u) != 0u;
            c->period_index = (uint8_t)(value & 0x0Fu);
            c->timer_period = APU_NOISE_PERIOD_TABLE[c->period_index];
            /* Running timer is not reset on a period change. */
            break;
        case 3u:
            /* $400F: LLLL L--- - length load + envelope restart. */
            if (c->enabled) {
                uint8_t idx = (uint8_t)(value >> 3);
                c->length_counter = APU_LENGTH_TABLE[idx];
            }
            c->envelope_start = true;
            break;
        default:
            break;
    }
}

void noise_set_enabled(NoiseChannel* c, bool enabled) {
    c->enabled = enabled;
    if (!enabled) {
        c->length_counter = 0u;
    }
}

void noise_tick(NoiseChannel* c) {
    if (c->timer == 0u) {
        c->timer = c->timer_period;
        /* Clock the LFSR. */
        uint16_t bit0 = (uint16_t)(c->lfsr & 0x0001u);
        uint16_t tap = c->mode ? 6u : 1u;
        uint16_t tap_bit = (uint16_t)((c->lfsr >> tap) & 0x0001u);
        uint16_t feedback = bit0 ^ tap_bit;
        c->lfsr = (uint16_t)(c->lfsr >> 1);
        if (feedback != 0u) {
            c->lfsr = (uint16_t)(c->lfsr | 0x4000u); /* bit 14 */
        }
    } else {
        c->timer = (uint16_t)(c->timer - 1u);
    }
}

void noise_clock_envelope(NoiseChannel* c) {
    if (c->envelope_start) {
        c->envelope_start = false;
        c->envelope_decay = 15u;
        c->envelope_divider = c->volume;
    } else if (c->envelope_divider == 0u) {
        c->envelope_divider = c->volume;
        if (c->envelope_decay > 0u) {
            c->envelope_decay = (uint8_t)(c->envelope_decay - 1u);
        } else if (c->halt) {
            c->envelope_decay = 15u;
        }
    } else {
        c->envelope_divider = (uint8_t)(c->envelope_divider - 1u);
    }
}

static void noise_clock_length(NoiseChannel* c) {
    if (!c->halt && c->length_counter > 0u) {
        c->length_counter = (uint8_t)(c->length_counter - 1u);
    }
}

void noise_clock_quarter_frame(NoiseChannel* c) {
    noise_clock_envelope(c);
}

void noise_clock_half_frame(NoiseChannel* c) {
    noise_clock_length(c);
}

uint8_t noise_sample(const NoiseChannel* c) {
    if (!c->enabled || c->length_counter == 0u) {
        return 0u;
    }
    if ((c->lfsr & 1u) != 0u) {
        return 0u;
    }
    return c->constant_volume ? c->volume : c->envelope_decay;
}

bool noise_enabled(const NoiseChannel* c) { return c->enabled; }
uint8_t noise_length_counter(const NoiseChannel* c) { return c->length_counter; }
uint8_t noise_envelope_decay(const NoiseChannel* c) { return c->envelope_decay; }
uint16_t noise_timer_period(const NoiseChannel* c) { return c->timer_period; }
uint16_t noise_timer(const NoiseChannel* c) { return c->timer; }
uint16_t noise_lfsr(const NoiseChannel* c) { return c->lfsr; }
bool noise_mode(const NoiseChannel* c) { return c->mode; }
uint8_t noise_period_index(const NoiseChannel* c) { return c->period_index; }

/* ---- DmcChannel (src/apu.rs lines 719-939) --------------------------- */

void dmc_init(DmcChannel* c) {
    memset(c, 0, sizeof(*c));
    c->timer_period = APU_DMC_RATE_TABLE[0];
    c->sample_addr_base = 0xC000u;
    c->sample_address = 0xC000u;
    c->sample_length = 1u;
}

void dmc_write_register(DmcChannel* c, uint8_t reg, uint8_t value) {
    switch (reg) {
        case 0u:
            /* $4010: IL-- RRRR - IRQ enable, loop, rate index. */
            c->irq_enable = (value & 0x80u) != 0u;
            c->loop_flag = (value & 0x40u) != 0u;
            c->rate_index = (uint8_t)(value & 0x0Fu);
            c->timer_period = APU_DMC_RATE_TABLE[c->rate_index];
            break;
        case 1u:
            /* $4011: -DDD DDDD - direct DAC load (bits 0-6). */
            c->output_counter = (uint8_t)(value & 0x7Fu);
            break;
        case 2u:
            /* $4012: sample address base = (value << 6) + $C000. */
            c->sample_addr_base = (uint16_t)(((uint16_t)value << 6) | 0xC000u);
            break;
        case 3u:
            /* $4013: sample length = (value << 4) + 1. */
            c->sample_length = (uint16_t)(((uint16_t)value << 4) | 1u);
            break;
        default:
            break;
    }
}

void dmc_set_enabled(DmcChannel* c, bool enabled) {
    c->enabled = enabled;
    if (enabled) {
        /* Only restart if the sample has finished (or was never started). */
        if (c->bytes_remaining == 0u) {
            c->sample_address = c->sample_addr_base;
            c->bytes_remaining = c->sample_length;
            c->buffer_bits = 0u; /* empty the sample buffer */
        }
    } else {
        /* Stop playback; the output counter retains its value. */
        c->bytes_remaining = 0u;
    }
}

void dmc_clear_irq(DmcChannel* c) {
    c->irq_flag = false;
}

/* clock_output_unit: fetch byte if buffer empty, shift one bit, update DAC. */
static void dmc_clock_output_unit(DmcChannel* c, DmcReadFn read, void* read_ctx) {
    /* Step 1: refill the sample buffer if empty and bytes remain. */
    if (c->buffer_bits == 0u && c->bytes_remaining > 0u) {
        c->sample_buffer = read(c->sample_address, read_ctx);
        c->buffer_bits = 8u;
        /* Advance the sample address, wrapping $FFFF -> $8000. */
        c->sample_address = (uint16_t)(c->sample_address + 1u);
        if (c->sample_address == 0u) {
            c->sample_address = 0x8000u;
        }
        c->bytes_remaining = (uint16_t)(c->bytes_remaining - 1u);
        /* If the sample is now exhausted, handle loop / IRQ. */
        if (c->bytes_remaining == 0u) {
            if (c->loop_flag) {
                c->sample_address = c->sample_addr_base;
                c->bytes_remaining = c->sample_length;
            } else if (c->irq_enable) {
                c->irq_flag = true;
            }
        }
    }

    /* Step 2: output one bit from the sample buffer. */
    if (c->buffer_bits > 0u) {
        uint8_t bit = (uint8_t)(c->sample_buffer & 1u);
        if (bit == 0u) {
            /* saturating_sub(2) */
            c->output_counter = (c->output_counter >= 2u)
                ? (uint8_t)(c->output_counter - 2u)
                : 0u;
        } else {
            /* saturating_add(2).min(127) */
            uint16_t v = (uint16_t)(c->output_counter + 2u);
            if (v > 127u) v = 127u;
            c->output_counter = (uint8_t)v;
        }
        c->sample_buffer = (uint8_t)(c->sample_buffer >> 1);
        c->buffer_bits = (uint8_t)(c->buffer_bits - 1u);
    }
}

void dmc_tick(DmcChannel* c, DmcReadFn read, void* read_ctx) {
    if (c->timer == 0u) {
        c->timer = c->timer_period;
        dmc_clock_output_unit(c, read, read_ctx);
    } else {
        c->timer = (uint16_t)(c->timer - 1u);
    }
}

uint8_t dmc_sample(const DmcChannel* c) { return c->output_counter; }

bool dmc_enabled(const DmcChannel* c) { return c->enabled; }
uint16_t dmc_bytes_remaining(const DmcChannel* c) { return c->bytes_remaining; }
uint8_t dmc_output_counter(const DmcChannel* c) { return c->output_counter; }
uint16_t dmc_timer_period(const DmcChannel* c) { return c->timer_period; }
uint8_t dmc_rate_index(const DmcChannel* c) { return c->rate_index; }
uint16_t dmc_sample_address(const DmcChannel* c) { return c->sample_address; }
uint16_t dmc_sample_addr_base(const DmcChannel* c) { return c->sample_addr_base; }
uint16_t dmc_sample_length(const DmcChannel* c) { return c->sample_length; }
bool dmc_loop_flag(const DmcChannel* c) { return c->loop_flag; }
bool dmc_irq_enable(const DmcChannel* c) { return c->irq_enable; }
bool dmc_irq_flag(const DmcChannel* c) { return c->irq_flag; }

/* ---- Apu (src/apu.rs lines 944-1435) --------------------------------- */

void apu_init(Apu* a) {
    memset(a, 0, sizeof(*a));
    pulse_init(&a->pulse1, false);
    pulse_init(&a->pulse2, true);
    triangle_init(&a->triangle);
    noise_init(&a->noise);
    dmc_init(&a->dmc);

    a->cycle_accumulator = 0u;
    for (size_t i = 0; i < APU_CHANNEL_COUNT; ++i) {
        a->channel_volumes[i] = 1.0f;
        a->channel_muted[i] = false;
    }
    a->selected_channel = 0u;
    /* Silence level (-1.0) so cold-boot produces steady -1.0 with no
     * startup transient. */
    a->lpf_prev = -1.0f;
    a->dc_prev_x = -1.0f;
    a->dc_prev_y = 0.0f;
    a->mix_accumulator = 0.0f;
    a->mix_count = 0u;
    a->sample_accumulator = 0.0f;
    a->last_decimated = -1.0f;
    a->frame_mode_5step = false;
    a->frame_irq_inhibit = false;
    a->frame_cycle = 0u;
    a->frame_irq = false;
    a->frame_reset_delay = 0u;
    a->region = REGION_NTSC;
}

PulseChannel* apu_pulse1_mut(Apu* a) { return &a->pulse1; }
PulseChannel* apu_pulse2_mut(Apu* a) { return &a->pulse2; }
TriangleChannel* apu_triangle_mut(Apu* a) { return &a->triangle; }
NoiseChannel* apu_noise_mut(Apu* a) { return &a->noise; }
DmcChannel* apu_dmc_mut(Apu* a) { return &a->dmc; }
const PulseChannel* apu_pulse1(const Apu* a) { return &a->pulse1; }
const PulseChannel* apu_pulse2(const Apu* a) { return &a->pulse2; }
const TriangleChannel* apu_triangle(const Apu* a) { return &a->triangle; }
const NoiseChannel* apu_noise(const Apu* a) { return &a->noise; }
const DmcChannel* apu_dmc(const Apu* a) { return &a->dmc; }

void apu_write_status(Apu* a, uint8_t value) {
    pulse_set_enabled(&a->pulse1, (value & 0x01u) != 0u);
    pulse_set_enabled(&a->pulse2, (value & 0x02u) != 0u);
    triangle_set_enabled(&a->triangle, (value & 0x04u) != 0u);
    noise_set_enabled(&a->noise, (value & 0x08u) != 0u);
    dmc_set_enabled(&a->dmc, (value & 0x10u) != 0u);
}

uint8_t apu_read_status(Apu* a) {
    uint8_t v = 0u;
    if (a->pulse1.length_counter > 0u)    v |= 0x01u;
    if (a->pulse2.length_counter > 0u)    v |= 0x02u;
    if (a->triangle.length_counter > 0u)  v |= 0x04u;
    if (a->noise.length_counter > 0u)     v |= 0x08u;
    if (a->dmc.bytes_remaining > 0u)      v |= 0x10u;
    if (a->frame_irq)                     v |= 0x40u;
    if (a->dmc.irq_flag)                  v |= 0x80u;
    /* Reading $4015 clears both IRQ flags. */
    a->frame_irq = false;
    a->dmc.irq_flag = false;
    return v;
}

void apu_write_frame_counter(Apu* a, uint8_t value) {
    bool new_mode_5step = (value & 0x80u) != 0u;
    a->frame_irq_inhibit = (value & 0x40u) != 0u;
    /* If IRQ inhibit set, clear any pending frame IRQ. */
    if (a->frame_irq_inhibit) {
        a->frame_irq = false;
    }
    /* 5-step mode: immediately clock quarter + half frame. */
    if (new_mode_5step) {
        apu_clock_quarter_frame(a);
        apu_clock_half_frame(a);
    }
    /* Schedule a frame counter reset after ~4 CPU cycles. */
    a->frame_reset_delay = 4u;
    /* Update the mode immediately (the reset will zero the cycle counter). */
    a->frame_mode_5step = new_mode_5step;
}

bool apu_irq_pending(const Apu* a) {
    return a->frame_irq || a->dmc.irq_flag;
}

Region apu_region(const Apu* a) { return a->region; }

void apu_set_region(Apu* a, Region region) { a->region = region; }

/* step_frame_counter: advance frame counter by cpu_cycles, firing
 * quarter/half-frame clocks and IRQ. (src/apu.rs `step_frame_counter`.) */
static void apu_step_frame_counter(Apu* a, uint32_t cpu_cycles) {
    if (a->frame_reset_delay > 0u) {
        uint32_t advance = (cpu_cycles < a->frame_reset_delay) ? cpu_cycles : a->frame_reset_delay;
        a->frame_reset_delay -= advance;
        if (a->frame_reset_delay == 0u) {
            /* The reset takes effect: zero the cycle counter. */
            a->frame_cycle = 0u;
        }
        /* During the reset delay, the frame counter does not advance.
         * The remaining cycles (if any) are applied after the reset. */
        uint32_t remaining = cpu_cycles - advance;
        if (remaining == 0u) {
            return;
        }
        /* saturating_add */
        if (a->frame_cycle > 0xFFFFFFFFu - remaining) {
            a->frame_cycle = 0xFFFFFFFFu;
        } else {
            a->frame_cycle += remaining;
        }
    } else {
        /* saturating_add */
        if (a->frame_cycle > 0xFFFFFFFFu - cpu_cycles) {
            a->frame_cycle = 0xFFFFFFFFu;
        } else {
            a->frame_cycle += cpu_cycles;
        }
    }

    /* Threshold crossings. Check each threshold against pre/post cycle count
     * so a large batch doesn't skip a threshold. */
    uint32_t prev = (a->frame_cycle >= cpu_cycles) ? (a->frame_cycle - cpu_cycles) : 0u;
    ApuFrameThreshold thresholds[4];
    if (a->frame_mode_5step) {
        region_apu_5step_thresholds(a->region, thresholds);
    } else {
        region_apu_4step_thresholds(a->region, thresholds);
    }
    uint32_t irq_threshold = region_apu_4step_irq_threshold(a->region);

    for (int i = 0; i < 4; ++i) {
        uint32_t threshold = thresholds[i].threshold;
        bool quarter = thresholds[i].quarter;
        bool half = thresholds[i].half;
        if (prev < threshold && a->frame_cycle >= threshold) {
            if (quarter) {
                apu_clock_quarter_frame(a);
            }
            if (half) {
                apu_clock_half_frame(a);
            }
            /* IRQ only in 4-step mode, at the 4th step, if not inhibited. */
            if (!a->frame_mode_5step && threshold == irq_threshold && !a->frame_irq_inhibit) {
                a->frame_irq = true;
            }
        }
    }

    /* Reset the counter at the end of the period. */
    uint32_t reset_at = region_apu_reset_at(a->region, a->frame_mode_5step);
    if (a->frame_cycle >= reset_at) {
        a->frame_cycle -= reset_at;
    }
}

void apu_step(Apu* a, uint32_t cpu_cycles, DmcReadFn read, void* read_ctx) {
    /* ---- Frame counter (CPU clock rate) ---- */
    apu_step_frame_counter(a, cpu_cycles);

    /* ---- Channel timers (APU clock rate = CPU / 2) ---- */
    float apu_cycles_per_sample = region_cpu_cycles_per_sample(a->region) / 2.0f;
    /* saturating_add */
    if (a->cycle_accumulator > 0xFFFFFFFFu - cpu_cycles) {
        a->cycle_accumulator = 0xFFFFFFFFu;
    } else {
        a->cycle_accumulator += cpu_cycles;
    }
    while (a->cycle_accumulator >= 2u) {
        a->cycle_accumulator -= 2u;
        pulse_tick(&a->pulse1);
        pulse_tick(&a->pulse2);
        triangle_tick(&a->triangle);
        noise_tick(&a->noise);
        dmc_tick(&a->dmc, read, read_ctx);

        a->mix_accumulator += apu_mix_raw(a);
        a->mix_count += 1u;
        a->sample_accumulator += 1.0f;

        if (a->sample_accumulator >= apu_cycles_per_sample) {
            a->sample_accumulator -= apu_cycles_per_sample;
            if (a->mix_count > 0u) {
                a->last_decimated = a->mix_accumulator / (float)a->mix_count;
                a->mix_accumulator = 0.0f;
                a->mix_count = 0u;
            }
        }
    }
}

void apu_clock_quarter_frame(Apu* a) {
    pulse_clock_quarter_frame(&a->pulse1);
    pulse_clock_quarter_frame(&a->pulse2);
    triangle_clock_quarter_frame(&a->triangle);
    noise_clock_quarter_frame(&a->noise);
}

void apu_clock_half_frame(Apu* a) {
    pulse_clock_half_frame(&a->pulse1);
    pulse_clock_half_frame(&a->pulse2);
    triangle_clock_half_frame(&a->triangle);
    noise_clock_half_frame(&a->noise);
}

uint8_t apu_mix(const Apu* a) {
    uint16_t s = (uint16_t)((uint16_t)pulse_sample(&a->pulse1)
                            + (uint16_t)pulse_sample(&a->pulse2)
                            + (uint16_t)triangle_sample(&a->triangle)
                            + (uint16_t)noise_sample(&a->noise));
    if (s > 15u) {
        return 15u;
    }
    return (uint8_t)s;
}

float apu_output(Apu* a) {
    /* Use the most recently decimated sample from step(). */
    float clamped = a->last_decimated;
    /* One-pole low-pass filter (cutoff ~12 kHz at 44.1 kHz). */
    const float LPF_ALPHA = 0.8192f;
    float filtered = LPF_ALPHA * clamped + (1.0f - LPF_ALPHA) * a->lpf_prev;
    a->lpf_prev = filtered;
    /* DC blocker (one-pole high-pass ~20 Hz). */
    const float DC_R = 0.99715f;
    float dc_out = filtered - a->dc_prev_x + DC_R * a->dc_prev_y;
    a->dc_prev_x = filtered;
    a->dc_prev_y = dc_out;
    return dc_out;
}

/* scaled_sample: scale a channel's sample by its volume scalar (0 if muted). */
static float apu_scaled_sample(const Apu* a, size_t idx, uint8_t raw) {
    if (idx >= APU_CHANNEL_COUNT || a->channel_muted[idx]) {
        return 0.0f;
    }
    return (float)raw * a->channel_volumes[idx];
}

static float apu_mix_raw(const Apu* a) {
    float p1 = apu_scaled_sample(a, 0u, pulse_sample(&a->pulse1));
    float p2 = apu_scaled_sample(a, 1u, pulse_sample(&a->pulse2));
    float tri = apu_scaled_sample(a, 2u, triangle_sample(&a->triangle));
    float noise = apu_scaled_sample(a, 3u, noise_sample(&a->noise));
    float dmc = apu_scaled_sample(a, 4u, dmc_sample(&a->dmc));

    float pulse_sum = p1 + p2;
    float pulse_out = (pulse_sum > 0.0f)
        ? (95.52f / (8128.0f / pulse_sum + 100.0f))
        : 0.0f;

    float tnd_inner = tri / 8227.0f + noise / 12241.0f + dmc / 22638.0f;
    float tnd_out = (tnd_inner > 0.0f)
        ? (163.67f / (1.0f / tnd_inner + 100.0f))
        : 0.0f;

    float mixed = (pulse_out + tnd_out) * 2.0f - 1.0f;
    if (mixed < -1.0f) mixed = -1.0f;
    if (mixed > 1.0f)  mixed = 1.0f;
    return mixed;
}

/* ---- Per-channel volume / mute accessors (M31) ----------------------- */

void apu_channel_volumes(const Apu* a, float out[APU_CHANNEL_COUNT]) {
    for (size_t i = 0; i < APU_CHANNEL_COUNT; ++i) {
        out[i] = a->channel_volumes[i];
    }
}

void apu_set_channel_volume(Apu* a, size_t idx, float vol) {
    if (idx < APU_CHANNEL_COUNT) {
        if (vol < 0.0f) vol = 0.0f;
        if (vol > 1.0f) vol = 1.0f;
        a->channel_volumes[idx] = vol;
    }
}

float apu_channel_volume(const Apu* a, size_t idx) {
    if (idx >= APU_CHANNEL_COUNT) {
        return 0.0f;
    }
    return a->channel_volumes[idx];
}

void apu_channel_muted(const Apu* a, bool out[APU_CHANNEL_COUNT]) {
    for (size_t i = 0; i < APU_CHANNEL_COUNT; ++i) {
        out[i] = a->channel_muted[i];
    }
}

bool apu_channel_muted_at(const Apu* a, size_t idx) {
    if (idx >= APU_CHANNEL_COUNT) {
        return true;
    }
    return a->channel_muted[idx];
}

bool apu_toggle_channel_mute(Apu* a, size_t idx) {
    if (idx >= APU_CHANNEL_COUNT) {
        return false;
    }
    a->channel_muted[idx] = !a->channel_muted[idx];
    return a->channel_muted[idx];
}

void apu_set_channel_muted(Apu* a, size_t idx, bool muted) {
    if (idx < APU_CHANNEL_COUNT) {
        a->channel_muted[idx] = muted;
    }
}

uint8_t apu_selected_channel(const Apu* a) {
    return a->selected_channel;
}

void apu_set_selected_channel(Apu* a, uint8_t idx) {
    a->selected_channel = (idx > 4u) ? 4u : idx;
}

void apu_reset_channel_mix(Apu* a) {
    for (size_t i = 0; i < APU_CHANNEL_COUNT; ++i) {
        a->channel_volumes[i] = 1.0f;
        a->channel_muted[i] = false;
    }
}

void apu_apply_channel_volumes(Apu* a, const float* vols, size_t n) {
    for (size_t i = 0; i < n; ++i) {
        if (i < APU_CHANNEL_COUNT) {
            float v = vols[i];
            if (v < 0.0f) v = 0.0f;
            if (v > 1.0f) v = 1.0f;
            a->channel_volumes[i] = v;
        }
    }
}
