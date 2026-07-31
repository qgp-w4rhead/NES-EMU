/*
 * apu.h - Audio Processing Unit: pulse, triangle, noise, and DMC channels.
 *
 * Port of src/apu.rs to C (milestone M4.3). Field-for-field and
 * function-for-function port of the Rust APU. The APU owns the two pulse
 * channels, the triangle channel, the noise channel, the DMC channel, and
 * the frame counter. Audio output is produced via non-linear mixing,
 * box-filter decimation to 44.1 kHz, a one-pole low-pass filter, and a DC
 * blocker (matching src/apu.rs `Apu::output` / `mix_raw`).
 *
 * See: https://www.nesdev.org/wiki/APU
 */
#ifndef NES_CORE_C_APU_H
#define NES_CORE_C_APU_H

#include <stdint.h>
#include <stdbool.h>
#include <stddef.h>
#include "region.h"

#ifdef __cplusplus
extern "C" {
#endif

/* ---- Constants (src/apu.rs lines 8-37, 711-714) ---------------------- */

/* 4 duty-cycle patterns (8-step sequences). Indexed by `duty` (0..=3). */
extern const uint8_t APU_DUTY_PATTERNS[4][8];
/* Length-counter lookup table (5-bit index -> half-frame clocks). */
extern const uint8_t APU_LENGTH_TABLE[32];
/* 32-step triangle waveform (15 -> 0 -> 15). */
extern const uint8_t APU_TRIANGLE_SEQUENCE[32];
/* Noise channel timer periods (4-bit period select -> APU cycles). */
extern const uint16_t APU_NOISE_PERIOD_TABLE[16];
/* DMC rate table (NTSC, APU cycles). */
extern const uint16_t APU_DMC_RATE_TABLE[16];

/* Maximum 11-bit timer period value (the period is 11 bits, $000-$7FF). */
#define APU_MAX_PERIOD 0x7FFu
/* A pulse channel's period below this value silences the channel (sweep mute). */
#define APU_MIN_AUDIBLE_PERIOD 8u
/* Audio output sample rate (matches src/audio.rs `SAMPLE_RATE`). */
#define APU_SAMPLE_RATE 44100

/* ---- PulseChannel (src/apu.rs lines 41-312) -------------------------- */

/* One of the two NES pulse wave channels. `pulse2` enables the sweep negate
 * quirk (extra -1 on negate). */
typedef struct PulseChannel {
    bool pulse2;
    uint8_t duty;
    bool halt;
    bool constant_volume;
    uint8_t volume;
    bool sweep_enabled;
    uint8_t sweep_period;
    bool sweep_negate;
    uint8_t sweep_shift;
    uint8_t sweep_divider;
    bool sweep_reload;
    uint16_t timer_period;
    uint16_t timer;
    uint8_t sequence;
    uint8_t length_counter;
    bool enabled;
    uint8_t envelope_divider;
    uint8_t envelope_decay;
    bool envelope_start;
} PulseChannel;

/* Construct a reset pulse channel. `pulse2` selects the pulse-2 sweep quirk. */
void pulse_init(PulseChannel* c, bool pulse2);
/* Write to pulse channel register `reg` (0..=3). */
void pulse_write_register(PulseChannel* c, uint8_t reg, uint8_t value);
/* Set channel enable from $4015; clearing forces length counter to 0. */
void pulse_set_enabled(PulseChannel* c, bool enabled);
/* Advance by one APU cycle: timer counts down, sequencer advances on reload. */
void pulse_tick(PulseChannel* c);
/* Clock the envelope (quarter-frame). */
void pulse_clock_envelope(PulseChannel* c);
/* Quarter-frame clock: clocks the envelope. */
void pulse_clock_quarter_frame(PulseChannel* c);
/* Half-frame clock: clocks the length counter and sweep unit. */
void pulse_clock_half_frame(PulseChannel* c);
/* Current output sample (0..=15), or 0 if silenced. */
uint8_t pulse_sample(const PulseChannel* c);

/* Accessors mirroring the Rust impl. */
bool pulse_enabled(const PulseChannel* c);
uint8_t pulse_length_counter(const PulseChannel* c);
uint16_t pulse_timer_period(const PulseChannel* c);
uint16_t pulse_timer(const PulseChannel* c);
uint8_t pulse_envelope_decay(const PulseChannel* c);
uint8_t pulse_sequence(const PulseChannel* c);
/* Compute sweep target period (wrapping). Exposed for testing. */
uint16_t pulse_sweep_target(const PulseChannel* c);
/* Whether the sweep unit is muting (period < 8 or target > 11-bit range). */
bool pulse_is_muted(const PulseChannel* c);

/* ---- TriangleChannel (src/apu.rs lines 317-486) ---------------------- */

typedef struct TriangleChannel {
    bool halt;
    uint8_t linear_reload;
    uint8_t linear_counter;
    bool linear_start;
    uint16_t timer_period;
    uint16_t timer;
    uint8_t sequence;
    uint8_t length_counter;
    bool enabled;
} TriangleChannel;

void triangle_init(TriangleChannel* c);
void triangle_write_register(TriangleChannel* c, uint8_t reg, uint8_t value);
void triangle_set_enabled(TriangleChannel* c, bool enabled);
void triangle_tick(TriangleChannel* c);
void triangle_clock_quarter_frame(TriangleChannel* c);
void triangle_clock_half_frame(TriangleChannel* c);
uint8_t triangle_sample(const TriangleChannel* c);

bool triangle_enabled(const TriangleChannel* c);
uint8_t triangle_length_counter(const TriangleChannel* c);
uint8_t triangle_linear_counter(const TriangleChannel* c);
uint16_t triangle_timer_period(const TriangleChannel* c);
uint16_t triangle_timer(const TriangleChannel* c);
uint8_t triangle_sequence(const TriangleChannel* c);

/* ---- NoiseChannel (src/apu.rs lines 491-709) ------------------------- */

typedef struct NoiseChannel {
    bool halt;
    bool constant_volume;
    uint8_t volume;
    bool mode;
    uint8_t period_index;
    uint16_t timer_period;
    uint16_t timer;
    uint16_t lfsr;
    uint8_t length_counter;
    bool enabled;
    uint8_t envelope_divider;
    uint8_t envelope_decay;
    bool envelope_start;
} NoiseChannel;

void noise_init(NoiseChannel* c);
void noise_write_register(NoiseChannel* c, uint8_t reg, uint8_t value);
void noise_set_enabled(NoiseChannel* c, bool enabled);
void noise_tick(NoiseChannel* c);
void noise_clock_envelope(NoiseChannel* c);
void noise_clock_quarter_frame(NoiseChannel* c);
void noise_clock_half_frame(NoiseChannel* c);
uint8_t noise_sample(const NoiseChannel* c);

bool noise_enabled(const NoiseChannel* c);
uint8_t noise_length_counter(const NoiseChannel* c);
uint8_t noise_envelope_decay(const NoiseChannel* c);
uint16_t noise_timer_period(const NoiseChannel* c);
uint16_t noise_timer(const NoiseChannel* c);
uint16_t noise_lfsr(const NoiseChannel* c);
bool noise_mode(const NoiseChannel* c);
uint8_t noise_period_index(const NoiseChannel* c);

/* ---- DmcChannel (src/apu.rs lines 719-939) --------------------------- */

/* DMC DMA read callback: fetches a byte from CPU memory for the DMC.
 * `ctx` is the user-provided context (typically the Bus). */
typedef uint8_t (*DmcReadFn)(uint16_t addr, void* ctx);

typedef struct DmcChannel {
    bool irq_enable;
    bool loop_flag;
    uint8_t rate_index;
    uint16_t timer_period;
    uint16_t timer;
    uint8_t output_counter;
    uint8_t sample_buffer;
    uint8_t buffer_bits;
    uint16_t sample_addr_base;
    uint16_t sample_address;
    uint16_t sample_length;
    uint16_t bytes_remaining;
    bool enabled;
    bool irq_flag;
} DmcChannel;

void dmc_init(DmcChannel* c);
void dmc_write_register(DmcChannel* c, uint8_t reg, uint8_t value);
void dmc_set_enabled(DmcChannel* c, bool enabled);
void dmc_clear_irq(DmcChannel* c);
/* Advance by one APU cycle; `read` fetches bytes from CPU memory for DMA. */
void dmc_tick(DmcChannel* c, DmcReadFn read, void* read_ctx);
uint8_t dmc_sample(const DmcChannel* c);

bool dmc_enabled(const DmcChannel* c);
uint16_t dmc_bytes_remaining(const DmcChannel* c);
uint8_t dmc_output_counter(const DmcChannel* c);
uint16_t dmc_timer_period(const DmcChannel* c);
uint8_t dmc_rate_index(const DmcChannel* c);
uint16_t dmc_sample_address(const DmcChannel* c);
uint16_t dmc_sample_addr_base(const DmcChannel* c);
uint16_t dmc_sample_length(const DmcChannel* c);
bool dmc_loop_flag(const DmcChannel* c);
bool dmc_irq_enable(const DmcChannel* c);
bool dmc_irq_flag(const DmcChannel* c);

/* ---- Apu (src/apu.rs lines 944-1435) --------------------------------- */

/* Number of audio channels (pulse1, pulse2, triangle, noise, dmc). */
#define APU_CHANNEL_COUNT 5u

typedef struct Apu {
    PulseChannel pulse1;
    PulseChannel pulse2;
    TriangleChannel triangle;
    NoiseChannel noise;
    DmcChannel dmc;

    /* Half-cycle accumulator (APU runs at CPU/2). */
    uint32_t cycle_accumulator;

    /* Per-channel volume / mute (M31). */
    float channel_volumes[APU_CHANNEL_COUNT];
    bool channel_muted[APU_CHANNEL_COUNT];
    uint8_t selected_channel;

    /* Low-pass filter (M31). */
    float lpf_prev;
    /* DC blocker. */
    float dc_prev_x;
    float dc_prev_y;
    /* Anti-alias decimation. */
    float mix_accumulator;
    uint32_t mix_count;
    float sample_accumulator;
    float last_decimated;

    /* Frame counter ($4017). */
    bool frame_mode_5step;
    bool frame_irq_inhibit;
    uint32_t frame_cycle;
    bool frame_irq;
    uint32_t frame_reset_delay;

    /* Region / TV system (M32). */
    Region region;
} Apu;

/* Construct a reset APU with all channels silenced. */
void apu_init(Apu* a);

/* Channel accessors. */
PulseChannel* apu_pulse1_mut(Apu* a);
PulseChannel* apu_pulse2_mut(Apu* a);
TriangleChannel* apu_triangle_mut(Apu* a);
NoiseChannel* apu_noise_mut(Apu* a);
DmcChannel* apu_dmc_mut(Apu* a);
const PulseChannel* apu_pulse1(const Apu* a);
const PulseChannel* apu_pulse2(const Apu* a);
const TriangleChannel* apu_triangle(const Apu* a);
const NoiseChannel* apu_noise(const Apu* a);
const DmcChannel* apu_dmc(const Apu* a);

/* Write $4015 status register (channel enable/disable bits). */
void apu_write_status(Apu* a, uint8_t value);
/* Read $4015 status (channel active bits + IRQ flags; clears IRQs). */
uint8_t apu_read_status(Apu* a);
/* Write $4017 frame counter control (mode + IRQ inhibit). */
void apu_write_frame_counter(Apu* a, uint8_t value);
/* Whether the APU has a pending IRQ (frame counter or DMC). */
bool apu_irq_pending(const Apu* a);
/* Current TV system / region. */
Region apu_region(const Apu* a);
/* Set the TV system / region. */
void apu_set_region(Apu* a, Region region);

/* Advance APU by `cpu_cycles` CPU cycles; `read` fetches bytes for DMC DMA. */
void apu_step(Apu* a, uint32_t cpu_cycles, DmcReadFn read, void* read_ctx);

/* Quarter/half-frame clock signals (used internally + by $4017 5-step). */
void apu_clock_quarter_frame(Apu* a);
void apu_clock_half_frame(Apu* a);

/* Linear mix of 4 channels (0..=15, clamped). Debug-only. */
uint8_t apu_mix(const Apu* a);
/* Full audio output [-1.0, 1.0] with non-linear mixing + LPF + DC blocker. */
float apu_output(Apu* a);

/* ---- Per-channel volume / mute accessors (M31) ----------------------- */

void apu_channel_volumes(const Apu* a, float out[APU_CHANNEL_COUNT]);
void apu_set_channel_volume(Apu* a, size_t idx, float vol);
float apu_channel_volume(const Apu* a, size_t idx);
void apu_channel_muted(const Apu* a, bool out[APU_CHANNEL_COUNT]);
bool apu_channel_muted_at(const Apu* a, size_t idx);
/* Toggle mute for channel idx; returns true if idx valid and now muted,
 * false if idx valid and now unmuted. For OOB idx the return value is
 * false and no action is taken. Use `apu_channel_muted_at` to disambiguate. */
bool apu_toggle_channel_mute(Apu* a, size_t idx);
void apu_set_channel_muted(Apu* a, size_t idx, bool muted);
uint8_t apu_selected_channel(const Apu* a);
void apu_set_selected_channel(Apu* a, uint8_t idx);
void apu_reset_channel_mix(Apu* a);
void apu_apply_channel_volumes(Apu* a, const float* vols, size_t n);

#ifdef __cplusplus
}
#endif
#endif /* NES_CORE_C_APU_H */
