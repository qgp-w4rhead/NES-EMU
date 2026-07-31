/*
 * test_apu.c - M4.3 acceptance test for the NES C APU.
 *
 * Verifies the port of src/apu.rs is faithful to the Rust source across:
 *  (1) Constants (DUTY_PATTERNS, LENGTH_TABLE, TRIANGLE_SEQUENCE,
 *      NOISE_PERIOD_TABLE, DMC_RATE_TABLE).
 *  (2) PulseChannel: register decoding, tick/sequence, sweep_target wrapping
 *      (pulse2 -1 quirk), is_muted, envelope, length counter, sweep, sample.
 *  (3) TriangleChannel: register decoding, tick, linear counter, sample.
 *  (4) NoiseChannel: register decoding, LFSR tick (mode 0/1), sample.
 *  (5) DmcChannel: register decoding, set_enabled restart, tick with stub
 *      read callback, output saturation, address wrap, loop, IRQ.
 *  (6) Apu: write_status/read_status round-trip, write_frame_counter (5-step
 *      immediate clock, IRQ inhibit), irq_pending, step frame counter
 *      4-step/5-step thresholds + IRQ, step channel timers, output() float
 *      range, mix_raw non-linear formula, per-channel volume/mute.
 *  (7) Bus integration: $4000-$4017 routing, bus_step_apu DMC DMA from RAM,
 *      bus_apu_irq_pending.
 *
 * Print PASS/FAIL per check and a final summary. Exit 0 on all pass, non-zero
 * on any failure.
 */
#include "apu.h"
#include "region.h"
#include "bus.h"
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <math.h>

/* ---- Test framework helpers ------------------------------------------ */

static int g_pass = 0;
static int g_fail = 0;

#define CHECK(cond, msg) do { \
    if (cond) { printf("PASS: %s\n", msg); g_pass++; } \
    else      { printf("FAIL: %s\n", msg); g_fail++; } \
} while (0)

#define CHECK_EQ_U8(actual, expected, msg) do { \
    uint8_t a_ = (actual), e_ = (expected); \
    if (a_ == e_) { printf("PASS: %s (got 0x%02X)\n", msg, a_); g_pass++; } \
    else          { printf("FAIL: %s (got 0x%02X, want 0x%02X)\n", msg, a_, e_); g_fail++; } \
} while (0)

#define CHECK_EQ_U16(actual, expected, msg) do { \
    uint16_t a_ = (actual), e_ = (expected); \
    if (a_ == e_) { printf("PASS: %s (got 0x%04X)\n", msg, a_); g_pass++; } \
    else          { printf("FAIL: %s (got 0x%04X, want 0x%04X)\n", msg, a_, e_); g_fail++; } \
} while (0)

#define CHECK_EQ_U32(actual, expected, msg) do { \
    uint32_t a_ = (actual), e_ = (expected); \
    if (a_ == e_) { printf("PASS: %s (got 0x%08X)\n", msg, a_); g_pass++; } \
    else          { printf("FAIL: %s (got 0x%08X, want 0x%08X)\n", msg, a_, e_); g_fail++; } \
} while (0)

#define CHECK_EQ_BOOL(actual, expected, msg) do { \
    bool a_ = (actual), e_ = (expected); \
    if (a_ == e_) { printf("PASS: %s (got %s)\n", msg, a_ ? "true" : "false"); g_pass++; } \
    else          { printf("FAIL: %s (got %s, want %s)\n", msg, a_ ? "true" : "false", e_ ? "true" : "false"); g_fail++; } \
} while (0)

#define CHECK_APPROX(actual, expected, tol, msg) do { \
    float a_ = (actual), e_ = (expected), t_ = (tol); \
    if (fabsf(a_ - e_) < t_) { printf("PASS: %s (got %f)\n", msg, a_); g_pass++; } \
    else                     { printf("FAIL: %s (got %f, want %f +/- %f)\n", msg, a_, e_, t_); g_fail++; } \
} while (0)

/* ---- Stub DMC read callback ------------------------------------------ *
 * Returns a fixed byte so tests can feed known sample data. */
static uint8_t g_dmc_feed = 0xAAu;

static uint8_t stub_dmc_read(uint16_t addr, void* ctx) {
    (void)addr;
    (void)ctx;
    return g_dmc_feed;
}

/* Tracking read callback: records the last address fetched so the DMC
 * $FFFF -> $8000 address-wrap can be verified. Returns g_dmc_feed. */
static uint16_t g_dmc_last_addr = 0;
static bool     g_dmc_saw_8000  = false;
static uint8_t tracking_dmc_read(uint16_t addr, void* ctx) {
    (void)ctx;
    g_dmc_last_addr = addr;
    if (addr == 0x8000u) g_dmc_saw_8000 = true;
    return g_dmc_feed;
}

/* ---- (1) Constants --------------------------------------------------- */

static void test_constants(void) {
    printf("\n== (1) Constants ==\n");

    /* DUTY_PATTERNS */
    CHECK_EQ_U8(APU_DUTY_PATTERNS[0][0], 0u, "DUTY[0][0]");
    CHECK_EQ_U8(APU_DUTY_PATTERNS[0][1], 1u, "DUTY[0][1]");
    CHECK_EQ_U8(APU_DUTY_PATTERNS[1][2], 1u, "DUTY[1][2]");
    CHECK_EQ_U8(APU_DUTY_PATTERNS[2][4], 1u, "DUTY[2][4]");
    CHECK_EQ_U8(APU_DUTY_PATTERNS[3][0], 1u, "DUTY[3][0]");
    CHECK_EQ_U8(APU_DUTY_PATTERNS[3][4], 1u, "DUTY[3][4]");

    /* LENGTH_TABLE spot checks */
    CHECK_EQ_U8(APU_LENGTH_TABLE[0], 10u,  "LENGTH[0]=10");
    CHECK_EQ_U8(APU_LENGTH_TABLE[1], 254u, "LENGTH[1]=254");
    CHECK_EQ_U8(APU_LENGTH_TABLE[3], 2u,   "LENGTH[3]=2");
    CHECK_EQ_U8(APU_LENGTH_TABLE[31], 30u, "LENGTH[31]=30");

    /* TRIANGLE_SEQUENCE */
    CHECK_EQ_U8(APU_TRIANGLE_SEQUENCE[0], 15u, "TRI[0]=15");
    CHECK_EQ_U8(APU_TRIANGLE_SEQUENCE[15], 0u, "TRI[15]=0");
    CHECK_EQ_U8(APU_TRIANGLE_SEQUENCE[16], 0u, "TRI[16]=0");
    CHECK_EQ_U8(APU_TRIANGLE_SEQUENCE[31], 15u, "TRI[31]=15");

    /* NOISE_PERIOD_TABLE */
    CHECK_EQ_U16(APU_NOISE_PERIOD_TABLE[0], 4u,    "NOISE[0]=4");
    CHECK_EQ_U16(APU_NOISE_PERIOD_TABLE[15], 4068u, "NOISE[15]=4068");

    /* DMC_RATE_TABLE */
    CHECK_EQ_U16(APU_DMC_RATE_TABLE[0], 214u, "DMC[0]=214");
    CHECK_EQ_U16(APU_DMC_RATE_TABLE[15], 42u, "DMC[15]=42");
}

/* ---- (2) PulseChannel ------------------------------------------------ */

static void test_pulse(void) {
    printf("\n== (2) PulseChannel ==\n");
    PulseChannel c;

    /* init defaults */
    pulse_init(&c, false);
    CHECK_EQ_BOOL(pulse_enabled(&c), false, "pulse_init enabled=false");
    CHECK_EQ_U8(pulse_length_counter(&c), 0u, "pulse_init length=0");
    CHECK_EQ_U16(pulse_timer_period(&c), 0u, "pulse_init timer_period=0");

    /* $4000: DDLC VVVV - duty 2, halt, const vol, volume 5 */
    pulse_write_register(&c, 0u, 0xB5u);
    CHECK_EQ_U8(c.duty, 2u, "$4000 duty=2");
    CHECK_EQ_BOOL(c.halt, true, "$4000 halt=true");
    CHECK_EQ_BOOL(c.constant_volume, true, "$4000 const_vol=true");
    CHECK_EQ_U8(c.volume, 5u, "$4000 volume=5");

    /* $4001: EPPP NSSS - enabled, period 3, negate, shift 2 */
    pulse_write_register(&c, 1u, 0xBAu);
    CHECK_EQ_BOOL(c.sweep_enabled, true, "$4001 sweep_enabled");
    CHECK_EQ_U8(c.sweep_period, 3u, "$4001 sweep_period=3");
    CHECK_EQ_BOOL(c.sweep_negate, true, "$4001 sweep_negate");
    CHECK_EQ_U8(c.sweep_shift, 2u, "$4001 sweep_shift=2");
    CHECK_EQ_BOOL(c.sweep_reload, true, "$4001 sweep_reload");

    /* $4002: timer low */
    pulse_write_register(&c, 2u, 0xABu);
    CHECK_EQ_U16(pulse_timer_period(&c), 0xABu, "$4002 timer low=0xAB");

    /* $4003: timer high + length + envelope restart + sequence reset.
     * Channel must be enabled for length load. */
    pulse_init(&c, false);
    pulse_set_enabled(&c, true);
    pulse_write_register(&c, 2u, 0x00u);
    pulse_write_register(&c, 3u, 0x04u); /* high=4, length idx=0 -> 10 */
    CHECK_EQ_U16(pulse_timer_period(&c), 0x400u, "$4003 timer_period=0x400");
    CHECK_EQ_U16(pulse_timer(&c), 0x400u, "$4003 timer high set immediately");
    CHECK_EQ_U8(pulse_length_counter(&c), 10u, "$4003 length=10 (idx 0)");
    CHECK_EQ_BOOL(c.envelope_start, true, "$4003 envelope_start=true");
    CHECK_EQ_U8(pulse_sequence(&c), 0u, "$4003 sequence reset to 0");

    /* length load suppressed when disabled */
    pulse_init(&c, false);
    pulse_write_register(&c, 3u, 0x00u);
    CHECK_EQ_U8(pulse_length_counter(&c), 0u, "length load suppressed when disabled");

    /* tick: period 0 -> timer stays 0 -> every tick advances sequence */
    pulse_init(&c, false);
    pulse_set_enabled(&c, true);
    pulse_write_register(&c, 0u, (uint8_t)(0x90u | 15u)); /* duty 2, const vol 15 */
    pulse_write_register(&c, 2u, 0x00u);
    pulse_write_register(&c, 3u, 0x00u); /* period 0, length 10 */
    uint8_t s0 = pulse_sequence(&c);
    pulse_tick(&c);
    CHECK_EQ_U8(pulse_sequence(&c), (uint8_t)((s0 + 1u) & 0x07u), "tick advances sequence (period 0)");
    pulse_tick(&c);
    CHECK_EQ_U8(pulse_sequence(&c), (uint8_t)((s0 + 2u) & 0x07u), "tick advances sequence again");

    /* tick: period 3 -> counts down then reloads */
    pulse_init(&c, false);
    pulse_set_enabled(&c, true);
    pulse_write_register(&c, 0u, (uint8_t)(0x90u | 15u));
    pulse_write_register(&c, 2u, 0x03u);
    pulse_write_register(&c, 3u, 0x00u); /* period 3, timer starts 0 (high 0) */
    CHECK_EQ_U16(pulse_timer(&c), 0u, "timer starts 0 after $4003 high=0");
    pulse_tick(&c); /* 0 -> reload 3 + advance */
    CHECK_EQ_U16(pulse_timer(&c), 3u, "tick 0->reload(3)");
    pulse_tick(&c);
    CHECK_EQ_U16(pulse_timer(&c), 2u, "tick 3->2");
    pulse_tick(&c);
    CHECK_EQ_U16(pulse_timer(&c), 1u, "tick 2->1");
    pulse_tick(&c);
    CHECK_EQ_U16(pulse_timer(&c), 0u, "tick 1->0");
    pulse_tick(&c);
    CHECK_EQ_U16(pulse_timer(&c), 3u, "tick 0->reload(3)");

    /* $4003 preserves low 8 bits of running timer */
    pulse_init(&c, false);
    pulse_set_enabled(&c, true);
    pulse_write_register(&c, 0u, (uint8_t)(0x90u | 15u));
    pulse_write_register(&c, 2u, 0xFFu);
    pulse_write_register(&c, 3u, 0x01u); /* high=1 -> period 0x1FF, timer=0x100 */
    CHECK_EQ_U16(pulse_timer(&c), 0x100u, "timer=0x100 after $4003 high=1");
    pulse_tick(&c); /* 0x100 -> 0x0FF */
    CHECK_EQ_U16(pulse_timer(&c), 0x0FFu, "tick 0x100->0x0FF");
    pulse_write_register(&c, 3u, 0x03u); /* high=3, low preserved */
    CHECK_EQ_U16(pulse_timer(&c), 0x3FFu, "$4003 preserves low 8 bits (0xFF) + high 3");
    CHECK_EQ_U16(pulse_timer_period(&c), 0x3FFu, "timer_period=0x3FF");

    /* sweep_target: add when not negate */
    pulse_init(&c, false);
    pulse_write_register(&c, 2u, 0x00u);
    pulse_write_register(&c, 3u, 0x04u); /* period 0x400 */
    pulse_write_register(&c, 1u, 0x81u); /* enabled, shift 1, add */
    CHECK_EQ_U16(pulse_sweep_target(&c), (uint16_t)(0x400u + (0x400u >> 1)), "sweep_target add");

    /* sweep_target: subtract pulse1 */
    pulse_init(&c, false);
    pulse_write_register(&c, 2u, 0x00u);
    pulse_write_register(&c, 3u, 0x04u);
    pulse_write_register(&c, 1u, 0x89u); /* enabled, negate, shift 1 */
    CHECK_EQ_U16(pulse_sweep_target(&c), (uint16_t)(0x400u - (0x400u >> 1)), "sweep_target sub pulse1");

    /* sweep_target: pulse2 -1 quirk */
    pulse_init(&c, true);
    pulse_write_register(&c, 2u, 0x00u);
    pulse_write_register(&c, 3u, 0x04u);
    pulse_write_register(&c, 1u, 0x89u);
    CHECK_EQ_U16(pulse_sweep_target(&c), (uint16_t)(0x400u - (0x400u >> 1) - 1u), "sweep_target sub pulse2 -1 quirk");

    /* sweep disabled returns current period */
    pulse_init(&c, false);
    pulse_write_register(&c, 2u, 0x00u);
    pulse_write_register(&c, 3u, 0x04u);
    pulse_write_register(&c, 1u, 0x01u); /* disabled, shift 1 */
    CHECK_EQ_U16(pulse_sweep_target(&c), 0x400u, "sweep disabled returns current period");

    /* sweep shift 0 returns current period */
    pulse_init(&c, false);
    pulse_write_register(&c, 2u, 0x00u);
    pulse_write_register(&c, 3u, 0x04u);
    pulse_write_register(&c, 1u, 0x80u); /* enabled, shift 0 */
    CHECK_EQ_U16(pulse_sweep_target(&c), 0x400u, "sweep shift 0 returns current period");

    /* is_muted: period < 8 */
    pulse_init(&c, false);
    pulse_set_enabled(&c, true);
    pulse_write_register(&c, 2u, 0x04u);
    pulse_write_register(&c, 3u, 0x00u); /* period 4 < 8 */
    CHECK_EQ_BOOL(pulse_is_muted(&c), true, "is_muted period<8");

    /* is_muted: target > 0x7FF */
    pulse_init(&c, false);
    pulse_set_enabled(&c, true);
    pulse_write_register(&c, 2u, 0xFFu);
    pulse_write_register(&c, 3u, 0x07u); /* period 0x7FF */
    pulse_write_register(&c, 1u, 0x81u); /* enabled, shift 1, add -> 0x7FF+0x3FF > 0x7FF */
    CHECK_EQ_BOOL(pulse_is_muted(&c), true, "is_muted target>0x7FF");

    /* is_muted: false for normal period */
    pulse_init(&c, false);
    pulse_set_enabled(&c, true);
    pulse_write_register(&c, 2u, 0x00u);
    pulse_write_register(&c, 3u, 0x04u); /* period 0x400 */
    CHECK_EQ_BOOL(pulse_is_muted(&c), false, "is_muted false for period 0x400");

    /* envelope: start -> decay=15, divider=volume */
    pulse_init(&c, false);
    pulse_write_register(&c, 0u, 0x00u); /* volume 0, envelope mode */
    pulse_write_register(&c, 3u, 0x00u); /* triggers envelope_start */
    pulse_clock_envelope(&c); /* start -> decay=15, divider=0 */
    CHECK_EQ_U8(pulse_envelope_decay(&c), 15u, "envelope start decay=15");

    /* envelope: volume 0 -> divider period 1 -> one decay per clock */
    pulse_clock_envelope(&c); /* divider 0 -> reload 0, decay 15->14 */
    CHECK_EQ_U8(pulse_envelope_decay(&c), 14u, "envelope decay 15->14");
    pulse_clock_envelope(&c);
    CHECK_EQ_U8(pulse_envelope_decay(&c), 13u, "envelope decay 14->13");

    /* envelope: loops when halt set */
    pulse_init(&c, false);
    pulse_write_register(&c, 0u, 0x20u); /* halt, volume 0 */
    pulse_write_register(&c, 3u, 0x00u); /* start */
    pulse_clock_envelope(&c); /* start -> decay=15, divider=0 */
    for (int i = 0; i < 15; ++i) pulse_clock_envelope(&c); /* 15->0 */
    CHECK_EQ_U8(pulse_envelope_decay(&c), 0u, "envelope halt decay reaches 0");
    pulse_clock_envelope(&c); /* 0 + halt -> loop to 15 */
    CHECK_EQ_U8(pulse_envelope_decay(&c), 15u, "envelope halt loops to 15");

    /* length counter: decrements on half frame */
    pulse_init(&c, false);
    pulse_set_enabled(&c, true);
    pulse_write_register(&c, 3u, 0x00u); /* length 10 */
    uint8_t lc0 = pulse_length_counter(&c);
    pulse_clock_half_frame(&c);
    CHECK_EQ_U8(pulse_length_counter(&c), (uint8_t)(lc0 - 1u), "length decrements on half frame");

    /* length counter: halt freezes */
    pulse_init(&c, false);
    pulse_set_enabled(&c, true);
    pulse_write_register(&c, 0u, 0x20u); /* halt */
    pulse_write_register(&c, 3u, 0x00u); /* length 10 */
    lc0 = pulse_length_counter(&c);
    pulse_clock_half_frame(&c);
    CHECK_EQ_U8(pulse_length_counter(&c), lc0, "length halt freezes count");

    /* length counter: stops at 0 */
    pulse_init(&c, false);
    pulse_set_enabled(&c, true);
    pulse_write_register(&c, 3u, (uint8_t)(3u << 3)); /* idx 3 -> length 2 */
    CHECK_EQ_U8(pulse_length_counter(&c), 2u, "length=2 (idx 3)");
    pulse_clock_half_frame(&c);
    CHECK_EQ_U8(pulse_length_counter(&c), 1u, "length 2->1");
    pulse_clock_half_frame(&c);
    CHECK_EQ_U8(pulse_length_counter(&c), 0u, "length 1->0");
    pulse_clock_half_frame(&c);
    CHECK_EQ_U8(pulse_length_counter(&c), 0u, "length stays 0");

    /* disabling via set_enabled clears length */
    pulse_init(&c, false);
    pulse_set_enabled(&c, true);
    pulse_write_register(&c, 3u, 0x00u);
    CHECK(pulse_length_counter(&c) > 0u, "length > 0 after load");
    pulse_set_enabled(&c, false);
    CHECK_EQ_U8(pulse_length_counter(&c), 0u, "set_enabled(false) clears length");

    /* sweep: applies after divider period */
    pulse_init(&c, false);
    pulse_set_enabled(&c, true);
    pulse_write_register(&c, 2u, 0x00u);
    pulse_write_register(&c, 3u, 0x01u); /* period 0x100 */
    pulse_write_register(&c, 1u, 0xA1u); /* enabled, period 2, shift 1, add */
    pulse_clock_half_frame(&c); /* divider 0 -> apply + reload to 2 */
    CHECK_EQ_U16(pulse_timer_period(&c), (uint16_t)(0x100u + (0x100u >> 1)), "sweep apply 0x100->0x180");
    uint16_t after_first = pulse_timer_period(&c);
    pulse_clock_half_frame(&c); /* divider 2->1 */
    CHECK_EQ_U16(pulse_timer_period(&c), after_first, "sweep no apply (divider 2->1)");
    pulse_clock_half_frame(&c); /* divider 1->0 */
    CHECK_EQ_U16(pulse_timer_period(&c), after_first, "sweep no apply (divider 1->0)");
    pulse_clock_half_frame(&c); /* divider 0 -> apply + reload */
    CHECK_EQ_U16(pulse_timer_period(&c), (uint16_t)(after_first + (after_first >> 1)), "sweep apply 0x180->0x240");

    /* sample: muted -> 0 */
    pulse_init(&c, false);
    pulse_set_enabled(&c, true);
    pulse_write_register(&c, 0u, 0x9Fu); /* duty 2, const vol 15 */
    pulse_write_register(&c, 2u, 0x04u);
    pulse_write_register(&c, 3u, 0x00u); /* period 4 < 8 -> muted */
    CHECK_EQ_U8(pulse_sample(&c), 0u, "sample muted=0");

    /* sample: duty_bit 0 -> 0 */
    pulse_init(&c, false);
    pulse_set_enabled(&c, true);
    pulse_write_register(&c, 0u, 0x9Fu); /* duty 2, const vol 15; duty2 seq0=0 */
    pulse_write_register(&c, 2u, 0x10u);
    pulse_write_register(&c, 3u, 0x00u); /* period 0x10, length 10, seq 0 */
    CHECK_EQ_U8(pulse_sample(&c), 0u, "sample duty_bit=0 -> 0");

    /* sample: const volume */
    pulse_init(&c, false);
    pulse_set_enabled(&c, true);
    pulse_write_register(&c, 0u, 0xD7u); /* duty 3, const vol 7; duty3 seq0=1 */
    pulse_write_register(&c, 2u, 0x10u);
    pulse_write_register(&c, 3u, 0x00u);
    CHECK_EQ_U8(pulse_sample(&c), 7u, "sample const vol=7");

    /* sample: envelope decay used as volume */
    pulse_init(&c, false);
    pulse_set_enabled(&c, true);
    pulse_write_register(&c, 0u, 0x8Fu); /* duty 2, envelope mode, vol 15 */
    pulse_write_register(&c, 2u, 0x08u);
    pulse_write_register(&c, 3u, 0x00u); /* period 8, length 10, seq 0 */
    pulse_clock_envelope(&c); /* start -> decay=15 */
    pulse_tick(&c); /* seq 0->1 (duty 2 seq1=1) */
    CHECK_EQ_U8(pulse_sample(&c), 15u, "sample envelope decay=15");
    for (int i = 0; i < 16; ++i) pulse_clock_envelope(&c); /* vol 15 -> 16 clocks per decay step */
    CHECK_EQ_U8(pulse_envelope_decay(&c), 14u, "envelope decay 15->14 after 16 clocks");
    CHECK_EQ_U8(pulse_sample(&c), 14u, "sample envelope decay=14");
}

/* ---- (3) TriangleChannel -------------------------------------------- */

static void test_triangle(void) {
    printf("\n== (3) TriangleChannel ==\n");
    TriangleChannel c;

    triangle_init(&c);
    CHECK_EQ_BOOL(triangle_enabled(&c), false, "tri init enabled=false");
    CHECK_EQ_U8(triangle_length_counter(&c), 0u, "tri init length=0");

    /* $4008: halt + linear reload */
    triangle_write_register(&c, 0u, 0xFFu);
    CHECK_EQ_BOOL(c.halt, true, "$4008 halt=true");
    CHECK_EQ_U8(c.linear_reload, 0x7Fu, "$4008 linear_reload=0x7F");

    /* $400A: timer low */
    triangle_write_register(&c, 2u, 0xABu);
    CHECK_EQ_U16(triangle_timer_period(&c), 0xABu, "$400A timer low=0xAB");

    /* $400B: timer high + length + linear start + sequence reset */
    triangle_init(&c);
    triangle_set_enabled(&c, true);
    triangle_write_register(&c, 2u, 0x00u);
    triangle_write_register(&c, 3u, 0x04u); /* high=4, length idx 0 -> 10 */
    CHECK_EQ_U16(triangle_timer_period(&c), 0x400u, "$400B timer_period=0x400");
    CHECK_EQ_U8(triangle_length_counter(&c), 10u, "$400B length=10");
    CHECK_EQ_BOOL(c.linear_start, true, "$400B linear_start=true");
    CHECK_EQ_U8(triangle_sequence(&c), 0u, "$400B sequence reset");

    /* tick: sequence & 0x1F */
    triangle_init(&c);
    triangle_set_enabled(&c, true);
    triangle_write_register(&c, 2u, 0x00u);
    triangle_write_register(&c, 3u, 0x00u); /* period 0 */
    uint8_t s0 = triangle_sequence(&c);
    triangle_tick(&c);
    CHECK_EQ_U8(triangle_sequence(&c), (uint8_t)((s0 + 1u) & 0x1Fu), "tri tick advances sequence &0x1F");
    /* wrap at 32 */
    for (int i = 0; i < 31; ++i) triangle_tick(&c);
    CHECK_EQ_U8(triangle_sequence(&c), (uint8_t)((s0 + 0u) & 0x1Fu), "tri sequence wraps at 32");

    /* clock_linear: start reloads, halt sets start */
    triangle_init(&c);
    triangle_write_register(&c, 0u, 0x7Fu); /* linear_reload=127, halt clear */
    c.linear_start = true;
    triangle_clock_quarter_frame(&c);
    CHECK_EQ_U8(triangle_linear_counter(&c), 127u, "linear start reloads 127");
    CHECK_EQ_BOOL(c.linear_start, false, "linear_start cleared after reload");
    /* halt sets linear_start */
    triangle_write_register(&c, 0u, 0x80u); /* halt=true, linear_reload=0 */
    triangle_clock_quarter_frame(&c);
    CHECK_EQ_BOOL(c.linear_start, true, "halt sets linear_start=true");

    /* sample: silenced when length/linear = 0 */
    triangle_init(&c);
    triangle_set_enabled(&c, true);
    CHECK_EQ_U8(triangle_sample(&c), 0u, "tri sample=0 when length=0");
    triangle_write_register(&c, 3u, 0x00u); /* length 10, but linear_counter=0 */
    CHECK_EQ_U8(triangle_sample(&c), 0u, "tri sample=0 when linear=0");
    /* load linear counter */
    triangle_write_register(&c, 0u, 0x7Fu); /* linear_reload=127 */
    c.linear_start = true;
    triangle_clock_quarter_frame(&c); /* linear_counter=127 */
    CHECK_EQ_U8(triangle_sample(&c), APU_TRIANGLE_SEQUENCE[0], "tri sample=15 when audible seq0");
}

/* ---- (4) NoiseChannel ----------------------------------------------- */

static void test_noise(void) {
    printf("\n== (4) NoiseChannel ==\n");
    NoiseChannel c;

    noise_init(&c);
    CHECK_EQ_U16(noise_lfsr(&c), 1u, "noise init lfsr=1");
    CHECK_EQ_U16(noise_timer_period(&c), APU_NOISE_PERIOD_TABLE[0], "noise init timer_period=table[0]");

    /* $400C: --LC VVVV */
    noise_write_register(&c, 0u, 0x35u);
    CHECK_EQ_BOOL(c.halt, true, "$400C halt=true");
    CHECK_EQ_BOOL(c.constant_volume, true, "$400C const_vol=true");
    CHECK_EQ_U8(c.volume, 5u, "$400C volume=5");

    /* $400E: mode + period */
    noise_write_register(&c, 2u, 0x82u); /* mode=1, period idx 2 */
    CHECK_EQ_BOOL(noise_mode(&c), true, "$400E mode=true");
    CHECK_EQ_U8(noise_period_index(&c), 2u, "$400E period_index=2");
    CHECK_EQ_U16(noise_timer_period(&c), APU_NOISE_PERIOD_TABLE[2], "$400E timer_period=table[2]");

    /* $400F: length + envelope restart */
    noise_init(&c);
    noise_set_enabled(&c, true);
    noise_write_register(&c, 3u, 0x00u); /* length idx 0 -> 10 */
    CHECK_EQ_U8(noise_length_counter(&c), 10u, "$400F length=10");
    CHECK_EQ_BOOL(c.envelope_start, true, "$400F envelope_start=true");

    /* tick LFSR: mode 0 XOR bit0/bit1. lfsr=1 -> bit0=1, bit1=0 -> feedback=1
     * -> shift right (0), set bit14 -> lfsr = 0x4000. */
    noise_init(&c);
    c.timer = 0u; /* force reload on next tick */
    noise_tick(&c);
    CHECK_EQ_U16(noise_lfsr(&c), 0x4000u, "LFSR mode0: lfsr 1 -> 0x4000");

    /* tick LFSR: mode 1 XOR bit0/bit6. Set lfsr=0x41 (bit0=1, bit6=1) -> fb=0
     * -> shift right, no bit14 -> lfsr = 0x20. */
    noise_init(&c);
    c.mode = true;
    c.lfsr = 0x41u;
    c.timer = 0u;
    noise_tick(&c);
    CHECK_EQ_U16(noise_lfsr(&c), 0x20u, "LFSR mode1: lfsr 0x41 -> 0x20 (fb=0)");

    /* sample: lfsr&1 silenced */
    noise_init(&c);
    noise_set_enabled(&c, true);
    noise_write_register(&c, 0u, 0x15u); /* const vol 5 */
    noise_write_register(&c, 3u, 0x00u); /* length 10 */
    CHECK_EQ_U8(noise_sample(&c), 0u, "noise sample=0 when lfsr&1 (lfsr=1)");
    /* shift LFSR until bit0=0 */
    while ((noise_lfsr(&c) & 1u) != 0u) {
        c.timer = 0u;
        noise_tick(&c);
    }
    CHECK_EQ_U8(noise_sample(&c), 5u, "noise sample=5 when lfsr bit0=0, const vol 5");
}

/* ---- (5) DmcChannel ------------------------------------------------- */

static void test_dmc(void) {
    printf("\n== (5) DmcChannel ==\n");
    DmcChannel d;

    dmc_init(&d);
    CHECK_EQ_U16(d.sample_addr_base, 0xC000u, "dmc init addr_base=0xC000");
    CHECK_EQ_U16(d.sample_address, 0xC000u, "dmc init address=0xC000");
    CHECK_EQ_U16(d.sample_length, 1u, "dmc init length=1");
    CHECK_EQ_U16(d.timer_period, APU_DMC_RATE_TABLE[0], "dmc init timer_period=table[0]");

    /* $4010: IL-- RRRR */
    dmc_write_register(&d, 0u, 0xFFu);
    CHECK_EQ_BOOL(dmc_irq_enable(&d), true, "$4010 irq_enable=true");
    CHECK_EQ_BOOL(dmc_loop_flag(&d), true, "$4010 loop=true");
    CHECK_EQ_U8(dmc_rate_index(&d), 0x0Fu, "$4010 rate_index=15");
    CHECK_EQ_U16(dmc_timer_period(&d), APU_DMC_RATE_TABLE[15], "$4010 timer_period=table[15]");

    /* $4011: 7-bit DAC */
    dmc_write_register(&d, 1u, 0x7Fu);
    CHECK_EQ_U8(dmc_output_counter(&d), 127u, "$4011 output=127");
    dmc_write_register(&d, 1u, 0x80u); /* bit 7 ignored */
    CHECK_EQ_U8(dmc_output_counter(&d), 0u, "$4011 bit7 ignored -> 0");

    /* $4012: addr base = (v<<6)|0xC000 */
    dmc_write_register(&d, 2u, 0x00u);
    CHECK_EQ_U16(dmc_sample_addr_base(&d), 0xC000u, "$4012 base=0xC000");
    dmc_write_register(&d, 2u, 0x40u);
    CHECK_EQ_U16(dmc_sample_addr_base(&d), 0xD000u, "$4012 base=0xD000");
    dmc_write_register(&d, 2u, 0xFFu);
    CHECK_EQ_U16(dmc_sample_addr_base(&d), 0xFFC0u, "$4012 base=0xFFC0");

    /* $4013: length = (v<<4)|1 */
    dmc_write_register(&d, 3u, 0x00u);
    CHECK_EQ_U16(dmc_sample_length(&d), 1u, "$4013 length=1");
    dmc_write_register(&d, 3u, 0x01u);
    CHECK_EQ_U16(dmc_sample_length(&d), 17u, "$4013 length=17");
    dmc_write_register(&d, 3u, 0xFFu);
    CHECK_EQ_U16(dmc_sample_length(&d), 0xFF1u, "$4013 length=0xFF1");

    /* set_enabled restarts sample from base */
    dmc_init(&d);
    dmc_write_register(&d, 2u, 0x10u); /* base = $C400 */
    dmc_write_register(&d, 3u, 0x01u); /* length = 17 */
    dmc_set_enabled(&d, true);
    CHECK_EQ_BOOL(dmc_enabled(&d), true, "dmc enabled=true");
    CHECK_EQ_U16(dmc_sample_address(&d), 0xC400u, "dmc restart address=0xC400");
    CHECK_EQ_U16(dmc_bytes_remaining(&d), 17u, "dmc restart bytes=17");

    /* disable stops fetch but keeps output */
    dmc_init(&d);
    dmc_write_register(&d, 1u, 0x40u); /* output=64 */
    dmc_set_enabled(&d, true);
    dmc_set_enabled(&d, false);
    CHECK_EQ_U16(dmc_bytes_remaining(&d), 0u, "dmc disable bytes=0");
    CHECK_EQ_U8(dmc_output_counter(&d), 64u, "dmc disable keeps output=64");

    /* tick: feed 0xFF -> 8 increments of 2 = +16 (saturating at 127) */
    dmc_init(&d);
    dmc_write_register(&d, 1u, 0x00u); /* output=0 */
    dmc_write_register(&d, 0u, 0x0Fu); /* rate index 15, period 42 */
    dmc_set_enabled(&d, true);
    g_dmc_feed = 0xFFu;
    for (int i = 0; i < 42 * 8 + 1; ++i) dmc_tick(&d, stub_dmc_read, NULL);
    CHECK_EQ_U8(dmc_output_counter(&d), 16u, "dmc 0xFF -> +16");

    /* tick: feed 0x00 -> 8 decrements of 2 = -16 */
    dmc_init(&d);
    dmc_write_register(&d, 1u, 0x7Fu); /* output=127 */
    dmc_write_register(&d, 0u, 0x0Fu);
    dmc_set_enabled(&d, true);
    g_dmc_feed = 0x00u;
    for (int i = 0; i < 42 * 8 + 1; ++i) dmc_tick(&d, stub_dmc_read, NULL);
    CHECK_EQ_U8(dmc_output_counter(&d), (uint8_t)(127u - 16u), "dmc 0x00 -> -16");

    /* output saturates at 127 */
    dmc_init(&d);
    dmc_write_register(&d, 1u, 0x7Eu); /* output=126 */
    dmc_write_register(&d, 0u, 0x0Fu);
    dmc_set_enabled(&d, true);
    g_dmc_feed = 0xFFu;
    for (int i = 0; i < 42 * 8 + 1; ++i) dmc_tick(&d, stub_dmc_read, NULL);
    CHECK_EQ_U8(dmc_output_counter(&d), 127u, "dmc saturates at 127");

    /* output saturates at 0 */
    dmc_init(&d);
    dmc_write_register(&d, 1u, 0x02u); /* output=2 */
    dmc_write_register(&d, 0u, 0x0Fu);
    dmc_set_enabled(&d, true);
    g_dmc_feed = 0x00u;
    for (int i = 0; i < 42 * 8 + 1; ++i) dmc_tick(&d, stub_dmc_read, NULL);
    CHECK_EQ_U8(dmc_output_counter(&d), 0u, "dmc saturates at 0");

    /* DMA fetch advances address and decrements bytes */
    dmc_init(&d);
    dmc_write_register(&d, 2u, 0x00u); /* base=$C000 */
    dmc_write_register(&d, 3u, 0x01u); /* length=17 */
    dmc_write_register(&d, 0u, 0x0Fu);
    dmc_set_enabled(&d, true);
    g_dmc_feed = 0xAAu;
    for (int i = 0; i < 42 * 8 + 1; ++i) dmc_tick(&d, stub_dmc_read, NULL);
    CHECK_EQ_U16(dmc_sample_address(&d), 0xC001u, "dmc fetch advances address");
    CHECK_EQ_U16(dmc_bytes_remaining(&d), 16u, "dmc fetch decrements bytes");

    /* IRQ raised on completion without loop */
    dmc_init(&d);
    dmc_write_register(&d, 0u, 0x8Fu); /* IRQ enable, no loop, rate 15 */
    dmc_write_register(&d, 3u, 0x00u); /* length=1 */
    dmc_set_enabled(&d, true);
    g_dmc_feed = 0x00u;
    for (int i = 0; i < 42 * 8 + 1; ++i) dmc_tick(&d, stub_dmc_read, NULL);
    CHECK_EQ_BOOL(dmc_irq_flag(&d), true, "dmc IRQ on completion without loop");

    /* no IRQ when loop set */
    dmc_init(&d);
    dmc_write_register(&d, 0u, 0xCFu); /* IRQ enable + loop + rate 15 */
    dmc_write_register(&d, 3u, 0x00u); /* length=1 */
    dmc_set_enabled(&d, true);
    g_dmc_feed = 0x00u;
    for (int i = 0; i < 42 * 8 + 1; ++i) dmc_tick(&d, stub_dmc_read, NULL);
    CHECK_EQ_BOOL(dmc_irq_flag(&d), false, "dmc no IRQ when loop set");
    /* loop restart: bytes_remaining back to 1, address back to base */
    CHECK_EQ_U16(dmc_bytes_remaining(&d), 1u, "dmc loop restart bytes=1");
    CHECK_EQ_U16(dmc_sample_address(&d), 0xC000u, "dmc loop restart address=0xC000");

    /* clear_irq */
    dmc_init(&d);
    d.irq_flag = true;
    dmc_clear_irq(&d);
    CHECK_EQ_BOOL(dmc_irq_flag(&d), false, "dmc clear_irq");

    /* sample returns output_counter */
    dmc_init(&d);
    dmc_write_register(&d, 1u, 0x5Au);
    CHECK_EQ_U8(dmc_sample(&d), (uint8_t)(0x5Au & 0x7Fu), "dmc sample=output_counter");

    /* $FFFF -> $8000 address wrap: base $FFC0 ($4012=0xFF), length 81
     * ($4013=0x05). Fetches 1..64 read $FFC0..$FFFF; fetch 64 advances
     * $FFFF -> $0000 -> $8000 (apu.rs:906-909); fetch 65 then reads $8000.
     * Length must exceed 65 so a fetch actually occurs at the wrapped addr. */
    dmc_init(&d);
    dmc_write_register(&d, 2u, 0xFFu); /* base = (0xFF<<6)|0xC000 = $FFC0 */
    dmc_write_register(&d, 3u, 0x05u); /* length = (0x05<<4)|1 = 81 */
    dmc_write_register(&d, 0u, 0x0Fu); /* rate 15, period 42 */
    dmc_set_enabled(&d, true);
    g_dmc_feed = 0x00u;
    g_dmc_saw_8000 = false;
    g_dmc_last_addr = 0;
    /* 81 bytes * 8 bits = 648 output-unit clocks; each needs a timer tick
     * (period 42), so ~27216 ticks comfortably fetches all 81 bytes. */
    for (int i = 0; i < 81 * 42 * 8 + 42; ++i) dmc_tick(&d, tracking_dmc_read, NULL);
    CHECK_EQ_BOOL(g_dmc_saw_8000, true, "dmc $FFFF -> $8000 address wrap observed");
    /* The DMC continues fetching past $8000 (length 81), so the last fetch
     * address is $8000 + (81-65) = $800F; only the wrap observation matters. */
    CHECK(g_dmc_last_addr >= 0x8000u, "dmc post-wrap fetch address >= $8000");
}

/* ---- (6) Apu -------------------------------------------------------- */

static void test_apu(void) {
    printf("\n== (6) Apu ==\n");
    Apu a;

    apu_init(&a);
    CHECK_EQ_U8(apu_selected_channel(&a), 0u, "apu init selected_channel=0");
    CHECK_EQ_BOOL(apu_irq_pending(&a), false, "apu init irq_pending=false");
    CHECK(apu_region(&a) == REGION_NTSC, "apu init region=NTSC");

    /* write_status enables channels */
    apu_write_status(&a, 0x1Fu); /* all 5 channels */
    CHECK_EQ_BOOL(pulse_enabled(apu_pulse1(&a)), true, "write_status pulse1 enabled");
    CHECK_EQ_BOOL(pulse_enabled(apu_pulse2(&a)), true, "write_status pulse2 enabled");
    CHECK_EQ_BOOL(triangle_enabled(apu_triangle(&a)), true, "write_status triangle enabled");
    CHECK_EQ_BOOL(noise_enabled(apu_noise(&a)), true, "write_status noise enabled");
    CHECK_EQ_BOOL(dmc_enabled(apu_dmc(&a)), true, "write_status dmc enabled");

    /* read_status: length>0 sets bits; IRQ flags cleared on read */
    apu_init(&a);
    apu_write_status(&a, 0x01u); /* enable pulse1 */
    pulse_write_register(apu_pulse1_mut(&a), 3u, 0x00u); /* length 10 */
    uint8_t s = apu_read_status(&a);
    CHECK_EQ_U8((uint8_t)(s & 0x01u), 0x01u, "read_status pulse1 length>0 -> bit0");

    /* read_status clears frame + dmc IRQ */
    apu_init(&a);
    a.frame_irq = true;
    a.dmc.irq_flag = true;
    s = apu_read_status(&a);
    CHECK_EQ_U8((uint8_t)(s & 0x40u), 0x40u, "read_status frame_irq bit6");
    CHECK_EQ_U8((uint8_t)(s & 0x80u), 0x80u, "read_status dmc_irq bit7");
    uint8_t s2 = apu_read_status(&a);
    CHECK_EQ_U8((uint8_t)(s2 & 0xC0u), 0x00u, "read_status second read clears IRQ bits");

    /* read_status reflects dmc bytes_remaining */
    apu_init(&a);
    dmc_write_register(apu_dmc_mut(&a), 3u, 0x01u); /* length 17 */
    dmc_set_enabled(apu_dmc_mut(&a), true);
    s = apu_read_status(&a);
    CHECK_EQ_U8((uint8_t)(s & 0x10u), 0x10u, "read_status dmc bytes>0 -> bit4");

    /* write_frame_counter: 5-step immediate quarter+half clock */
    apu_init(&a);
    apu_write_status(&a, 0x01u);
    pulse_write_register(apu_pulse1_mut(&a), 3u, 0x00u); /* length 10 */
    uint8_t len0 = pulse_length_counter(apu_pulse1(&a));
    apu_write_frame_counter(&a, 0x80u); /* 5-step -> immediate Q+H clock */
    CHECK_EQ_U8(pulse_length_counter(apu_pulse1(&a)), (uint8_t)(len0 - 1u), "5-step immediate half-frame clocks length");

    /* write_frame_counter: IRQ inhibit clears frame_irq */
    apu_init(&a);
    a.frame_irq = true;
    apu_write_frame_counter(&a, 0x40u); /* IRQ inhibit */
    CHECK_EQ_BOOL(a.frame_irq, false, "IRQ inhibit clears frame_irq");
    CHECK_EQ_BOOL(a.frame_irq_inhibit, true, "frame_irq_inhibit set");
    CHECK_EQ_U32(a.frame_reset_delay, 4u, "frame_reset_delay=4 after $4017 write");

    /* irq_pending reflects both sources */
    apu_init(&a);
    CHECK_EQ_BOOL(apu_irq_pending(&a), false, "irq_pending false initially");
    a.frame_irq = true;
    CHECK_EQ_BOOL(apu_irq_pending(&a), true, "irq_pending true with frame_irq");
    a.frame_irq = false;
    a.dmc.irq_flag = true;
    CHECK_EQ_BOOL(apu_irq_pending(&a), true, "irq_pending true with dmc.irq_flag");

    /* 4-step frame counter raises IRQ at 29828 (+4 reset delay) */
    apu_init(&a);
    apu_write_frame_counter(&a, 0x00u); /* 4-step, IRQ enabled */
    apu_step(&a, 29833u, stub_dmc_read, NULL);
    CHECK_EQ_BOOL(apu_irq_pending(&a), true, "4-step IRQ at 29828+4");

    /* 4-step IRQ inhibited */
    apu_init(&a);
    apu_write_frame_counter(&a, 0x40u); /* 4-step, IRQ inhibited */
    apu_step(&a, 29833u, stub_dmc_read, NULL);
    CHECK_EQ_BOOL(apu_irq_pending(&a), false, "4-step no IRQ when inhibited");

    /* 5-step no IRQ */
    apu_init(&a);
    apu_write_frame_counter(&a, 0x80u); /* 5-step */
    apu_step(&a, 37288u, stub_dmc_read, NULL);
    CHECK_EQ_BOOL(apu_irq_pending(&a), false, "5-step no IRQ");

    /* PAL 4-step IRQ at 33255 (+4 reset delay). NTSC would not fire here. */
    apu_init(&a);
    apu_set_region(&a, REGION_PAL);
    apu_write_frame_counter(&a, 0x00u); /* 4-step, IRQ enabled */
    apu_step(&a, 33260u, stub_dmc_read, NULL);
    CHECK_EQ_BOOL(apu_irq_pending(&a), true, "PAL 4-step IRQ at 33255+4");

    /* PAL 4-step does NOT fire IRQ at the NTSC threshold 29828. */
    apu_init(&a);
    apu_set_region(&a, REGION_PAL);
    apu_write_frame_counter(&a, 0x00u);
    apu_step(&a, 29833u, stub_dmc_read, NULL);
    CHECK_EQ_BOOL(apu_irq_pending(&a), false, "PAL 4-step no IRQ at NTSC threshold 29828");

    /* PAL quarter-frame threshold 8314 clocks the envelope. The first
     * quarter clock restarts the envelope (start->decay=15); decay reaches
     * 14 only after the second quarter clock (at 16627). Verify the first
     * threshold fired by checking decay==15 (init decay is 0, so 15 proves
     * the quarter clock ran). */
    apu_init(&a);
    apu_set_region(&a, REGION_PAL);
    apu_write_status(&a, 0x01u);
    pulse_write_register(apu_pulse1_mut(&a), 0u, 0x00u); /* envelope mode */
    pulse_write_register(apu_pulse1_mut(&a), 3u, 0x00u); /* envelope_start */
    apu_write_frame_counter(&a, 0x00u);
    apu_step(&a, 8318u, stub_dmc_read, NULL); /* past PAL 1st quarter at 8314 */
    CHECK_EQ_U8(pulse_envelope_decay(apu_pulse1(&a)), 15u, "PAL quarter-frame at 8314 restarts envelope -> decay 15");

    /* PAL half-frame threshold 16627 clocks the length counter. */
    apu_init(&a);
    apu_set_region(&a, REGION_PAL);
    apu_write_status(&a, 0x01u);
    pulse_write_register(apu_pulse1_mut(&a), 3u, 0x00u); /* length 10 */
    apu_write_frame_counter(&a, 0x00u);
    apu_step(&a, 16632u, stub_dmc_read, NULL); /* past PAL 2nd half-frame at 16627 */
    CHECK_EQ_U8(pulse_length_counter(apu_pulse1(&a)), 9u, "PAL half-frame at 16627 clocks length -> 9");

    /* 4-step quarter-frame clocks envelope */
    apu_init(&a);
    apu_write_status(&a, 0x01u);
    pulse_write_register(apu_pulse1_mut(&a), 0u, 0x00u); /* envelope mode, volume 0 */
    pulse_write_register(apu_pulse1_mut(&a), 3u, 0x00u); /* length 10, envelope_start */
    apu_write_frame_counter(&a, 0x00u);
    apu_step(&a, 14918u, stub_dmc_read, NULL); /* past 2nd quarter-frame at 14913 */
    CHECK_EQ_U8(pulse_envelope_decay(apu_pulse1(&a)), 14u, "4-step quarter-frame clocks envelope -> decay 14");

    /* 4-step half-frame clocks length */
    apu_init(&a);
    apu_write_status(&a, 0x01u);
    pulse_write_register(apu_pulse1_mut(&a), 3u, 0x00u); /* length 10 */
    apu_write_frame_counter(&a, 0x00u);
    apu_step(&a, 14918u, stub_dmc_read, NULL); /* past first half-frame at 14913 */
    CHECK_EQ_U8(pulse_length_counter(apu_pulse1(&a)), 9u, "4-step half-frame clocks length -> 9");

    /* step ticks triangle timer (cycle_accumulator /2) */
    apu_init(&a);
    apu_write_status(&a, 0x04u); /* enable triangle */
    triangle_write_register(apu_triangle_mut(&a), 2u, 0x00u);
    triangle_write_register(apu_triangle_mut(&a), 3u, 0x00u); /* period 0 */
    uint8_t ts0 = triangle_sequence(apu_triangle(&a));
    apu_step(&a, 2u, stub_dmc_read, NULL); /* one APU cycle */
    CHECK_EQ_U8(triangle_sequence(apu_triangle(&a)), (uint8_t)((ts0 + 1u) & 0x1Fu), "apu_step ticks triangle timer");

    /* output() produces float in [-1, 1] (silence -> 0.0) */
    apu_init(&a);
    float out = apu_output(&a);
    CHECK_APPROX(out, 0.0f, 1e-6f, "apu_output silence = 0.0 (DC blocker)");
    for (int i = 0; i < 10; ++i) {
        float o = apu_output(&a);
        CHECK(fabsf(o) < 1e-6f, "apu_output silence stays 0.0");
    }

    /* output() with DMC produces a float in [-1, 1] */
    apu_init(&a);
    dmc_write_register(apu_dmc_mut(&a), 1u, 0x7Fu); /* DMC=127 */
    apu_step(&a, 42u, stub_dmc_read, NULL);
    out = apu_output(&a);
    CHECK(out >= -1.0f && out <= 1.0f, "apu_output DMC in [-1,1]");
    /* Expected ~0.9641 (non-linear DMC + LPF + DC blocker transient). */
    CHECK_APPROX(out, 0.9641f, 2e-2f, "apu_output DMC transient ~0.9641");

    /* output() with pulse+DMC -> ~1.2081 transient.
     * Note: Rust output() does NOT clamp the final DC-blocker output (only
     * mix_raw clamps to [-1,1] before filtering), so transients can exceed
     * 1.0 briefly as the DC blocker removes the -1.0 silence offset. */
    apu_init(&a);
    apu_write_status(&a, 0x01u);
    pulse_write_register(apu_pulse1_mut(&a), 0u, 0xBFu); /* duty 2, const vol 15 */
    pulse_write_register(apu_pulse1_mut(&a), 2u, 0xFFu);
    pulse_write_register(apu_pulse1_mut(&a), 3u, 0x00u);
    dmc_write_register(apu_dmc_mut(&a), 1u, 0x7Fu);
    apu_step(&a, 42u, stub_dmc_read, NULL);
    out = apu_output(&a);
    CHECK_APPROX(out, 1.2081f, 2e-2f, "apu_output pulse+DMC transient ~1.2081");

    /* apu_mix linear debug mix: triangle + noise */
    apu_init(&a);
    apu_write_status(&a, 0x0Cu); /* triangle + noise */
    triangle_write_register(apu_triangle_mut(&a), 0u, 127u); /* linear reload 127 */
    triangle_write_register(apu_triangle_mut(&a), 2u, 0x00u);
    triangle_write_register(apu_triangle_mut(&a), 3u, 0x00u); /* length 10, start */
    apu_clock_quarter_frame(&a); /* linear counter -> 127 */
    noise_write_register(apu_noise_mut(&a), 0u, 0x15u); /* const vol 5 */
    noise_write_register(apu_noise_mut(&a), 3u, 0x00u); /* length 10 */
    CHECK_EQ_U8(triangle_sample(apu_triangle(&a)), 15u, "tri sample=15 (seq0)");
    CHECK_EQ_U8(noise_sample(apu_noise(&a)), 0u, "noise sample=0 (lfsr bit0=1)");
    CHECK_EQ_U8(apu_mix(&a), 15u, "apu_mix = 15 (tri only)");
    /* shift noise LFSR so bit0=0 */
    while ((noise_lfsr(apu_noise(&a)) & 1u) != 0u) {
        noise_tick(apu_noise_mut(&a));
    }
    CHECK_EQ_U8(noise_sample(apu_noise(&a)), 5u, "noise sample=5 after LFSR shift");
    CHECK_EQ_U8(apu_mix(&a), 15u, "apu_mix clamps 15+5 -> 15");

    /* per-channel volume/mute */
    apu_init(&a);
    float vols[APU_CHANNEL_COUNT];
    apu_channel_volumes(&a, vols);
    for (size_t i = 0; i < APU_CHANNEL_COUNT; ++i) {
        CHECK_APPROX(vols[i], 1.0f, 1e-6f, "channel_volumes init=1.0");
    }
    /* set_channel_volume clamp */
    apu_set_channel_volume(&a, 0u, 0.5f);
    CHECK_APPROX(apu_channel_volume(&a, 0u), 0.5f, 1e-6f, "set_channel_volume 0.5");
    apu_set_channel_volume(&a, 0u, 2.0f); /* clamp to 1.0 */
    CHECK_APPROX(apu_channel_volume(&a, 0u), 1.0f, 1e-6f, "set_channel_volume clamps to 1.0");
    apu_set_channel_volume(&a, 0u, -1.0f); /* clamp to 0.0 */
    CHECK_APPROX(apu_channel_volume(&a, 0u), 0.0f, 1e-6f, "set_channel_volume clamps to 0.0");
    /* OOB idx */
    CHECK_APPROX(apu_channel_volume(&a, 99u), 0.0f, 1e-6f, "channel_volume OOB=0.0");
    CHECK_EQ_BOOL(apu_channel_muted_at(&a, 99u), true, "channel_muted_at OOB=true");

    /* toggle_channel_mute */
    apu_init(&a);
    bool muted = apu_toggle_channel_mute(&a, 0u);
    CHECK_EQ_BOOL(muted, true, "toggle_channel_mute returns new state (true)");
    CHECK_EQ_BOOL(apu_channel_muted_at(&a, 0u), true, "channel 0 muted after toggle");
    muted = apu_toggle_channel_mute(&a, 0u);
    CHECK_EQ_BOOL(muted, false, "toggle_channel_mute returns new state (false)");
    CHECK_EQ_BOOL(apu_channel_muted_at(&a, 0u), false, "channel 0 unmuted after 2nd toggle");

    /* set_channel_muted */
    apu_set_channel_muted(&a, 1u, true);
    CHECK_EQ_BOOL(apu_channel_muted_at(&a, 1u), true, "set_channel_muted true");

    /* muted channel -> 0 in mix_raw (output silence) */
    apu_init(&a);
    apu_write_status(&a, 0x10u); /* enable DMC only */
    dmc_write_register(apu_dmc_mut(&a), 1u, 0x7Fu); /* DMC=127 */
    apu_set_channel_muted(&a, 4u, true); /* mute DMC */
    apu_step(&a, 42u, stub_dmc_read, NULL);
    out = apu_output(&a);
    /* With DMC muted, mix_raw = -1.0 (silence). LPF from -1.0 stays -1.0.
     * DC blocker: -1.0 - (-1.0) + R*0 = 0.0. */
    CHECK_APPROX(out, 0.0f, 1e-6f, "muted DMC -> silence 0.0");

    /* selected_channel clamp */
    apu_init(&a);
    apu_set_selected_channel(&a, 4u);
    CHECK_EQ_U8(apu_selected_channel(&a), 4u, "selected_channel=4");
    apu_set_selected_channel(&a, 9u); /* clamp to 4 */
    CHECK_EQ_U8(apu_selected_channel(&a), 4u, "selected_channel clamps to 4");

    /* reset_channel_mix */
    apu_init(&a);
    apu_set_channel_volume(&a, 0u, 0.3f);
    apu_set_channel_muted(&a, 1u, true);
    apu_reset_channel_mix(&a);
    apu_channel_volumes(&a, vols);
    CHECK_APPROX(vols[0u], 1.0f, 1e-6f, "reset_channel_mix volume=1.0");
    CHECK_EQ_BOOL(apu_channel_muted_at(&a, 1u), false, "reset_channel_mix unmuted");

    /* apply_channel_volumes */
    apu_init(&a);
    float new_vols[3] = { 0.1f, 0.2f, 0.3f };
    apu_apply_channel_volumes(&a, new_vols, 3u);
    CHECK_APPROX(apu_channel_volume(&a, 0u), 0.1f, 1e-6f, "apply_channel_volumes[0]=0.1");
    CHECK_APPROX(apu_channel_volume(&a, 1u), 0.2f, 1e-6f, "apply_channel_volumes[1]=0.2");
    CHECK_APPROX(apu_channel_volume(&a, 2u), 0.3f, 1e-6f, "apply_channel_volumes[2]=0.3");
    CHECK_APPROX(apu_channel_volume(&a, 3u), 1.0f, 1e-6f, "apply_channel_volumes[3] unchanged=1.0");
    /* apply clamps */
    float big_vols[1] = { 5.0f };
    apu_apply_channel_volumes(&a, big_vols, 1u);
    CHECK_APPROX(apu_channel_volume(&a, 0u), 1.0f, 1e-6f, "apply_channel_volumes clamps to 1.0");

    /* region accessors */
    apu_init(&a);
    apu_set_region(&a, REGION_PAL);
    CHECK(apu_region(&a) == REGION_PAL, "apu_set_region PAL");
}

/* ---- (7) Bus integration -------------------------------------------- */

static void test_bus_integration(void) {
    printf("\n== (7) Bus integration ==\n");
    Bus bus;
    bus_init(&bus);

    /* $4000 write routes to pulse1 */
    bus_write(&bus, 0x4000u, 0xD7u); /* duty 3, const vol 7 */
    CHECK_EQ_U8(apu_pulse1(&bus.apu)->duty, 3u, "bus $4000 -> pulse1 duty=3");
    CHECK_EQ_U8(apu_pulse1(&bus.apu)->volume, 7u, "bus $4000 -> pulse1 volume=7");
    /* open bus latched */
    CHECK_EQ_U8(bus.apu_open_bus[0x00u], 0xD7u, "bus $4000 open bus latched");

    /* $4004 write routes to pulse2 */
    bus_write(&bus, 0x4004u, 0x23u); /* duty 0, halt, const vol clear, vol 3 */
    CHECK_EQ_U8(apu_pulse2(&bus.apu)->duty, 0u, "bus $4004 -> pulse2 duty=0");
    CHECK_EQ_U8(apu_pulse2(&bus.apu)->volume, 3u, "bus $4004 -> pulse2 volume=3");

    /* $4008 write routes to triangle */
    bus_write(&bus, 0x4008u, 0xFFu);
    CHECK_EQ_U8(apu_triangle(&bus.apu)->linear_reload, 0x7Fu, "bus $4008 -> triangle linear_reload=0x7F");
    CHECK_EQ_BOOL(apu_triangle(&bus.apu)->halt, true, "bus $4008 -> triangle halt=true");

    /* $400C write routes to noise */
    bus_write(&bus, 0x400Cu, 0x35u);
    CHECK_EQ_U8(apu_noise(&bus.apu)->volume, 5u, "bus $400C -> noise volume=5");
    CHECK_EQ_BOOL(apu_noise(&bus.apu)->halt, true, "bus $400C -> noise halt=true");

    /* $4010 write routes to dmc */
    bus_write(&bus, 0x4010u, 0xFFu);
    CHECK_EQ_BOOL(apu_dmc(&bus.apu)->irq_enable, true, "bus $4010 -> dmc irq_enable=true");
    CHECK_EQ_U8(apu_dmc(&bus.apu)->rate_index, 0x0Fu, "bus $4010 -> dmc rate_index=15");

    /* $4015 write enables channels */
    bus_init(&bus);
    bus_write(&bus, 0x4015u, 0x1Fu);
    CHECK_EQ_BOOL(pulse_enabled(apu_pulse1(&bus.apu)), true, "bus $4015 -> pulse1 enabled");
    CHECK_EQ_BOOL(dmc_enabled(apu_dmc(&bus.apu)), true, "bus $4015 -> dmc enabled");
    CHECK_EQ_U8(bus.apu_open_bus[0x15u], 0x1Fu, "bus $4015 open bus latched");

    /* $4015 read: status bits + open-bus bit 5 */
    bus_init(&bus);
    bus_write(&bus, 0x4015u, 0x20u); /* open bus = 0x20, no channels enabled */
    /* Manually enable pulse1 + load length via $4003 (bypass $4015). */
    pulse_set_enabled(apu_pulse1_mut(&bus.apu), true);
    bus_write(&bus, 0x4003u, 0x00u); /* length idx 0 -> 10 */
    uint8_t st = bus_read(&bus, 0x4015u);
    /* bit 0 = pulse1 length>0, bit 5 = open bus 0x20 */
    CHECK_EQ_U8((uint8_t)(st & 0x01u), 0x01u, "bus $4015 read bit0 (pulse1 length)");
    CHECK_EQ_U8((uint8_t)(st & 0x20u), 0x20u, "bus $4015 read bit5 (open bus preserved)");

    /* $4017 write sets frame mode */
    bus_init(&bus);
    bus_write(&bus, 0x4017u, 0x80u); /* 5-step */
    CHECK_EQ_BOOL(bus.apu.frame_mode_5step, true, "bus $4017 -> frame_mode_5step=true");
    CHECK_EQ_U8(bus.apu_open_bus[0x17u], 0x80u, "bus $4017 open bus latched");

    /* bus_step_apu advances APU (DMC DMA reads from cartridge PRG at $C000).
     * Build a minimal NROM-128 cartridge with 0xFF at PRG offset $4000 ($C000
     * CPU address). base=$C000, length=1, rate index 15 (period 42). */
    {
        /* Minimal iNES header + 16KB PRG + 8KB CHR. */
        static uint8_t rom[16u + 16384u + 8192u];
        static Cartridge cart;
        memset(rom, 0, sizeof(rom));
        rom[0] = 'N'; rom[1] = 'E'; rom[2] = 'S'; rom[3] = 0x1A;
        rom[4] = 1;  /* 1 x 16KB PRG */
        rom[5] = 1;  /* 1 x 8KB CHR */
        rom[6] = 0; rom[7] = 0;
        /* Put 0xFF at PRG offset 0 (CPU $C000 for NROM-128 mirror).
         * ROM file offset = 16 + 0 = 16. */
        rom[16u + 0u] = 0xFFu;
        /* RESET vector -> $C000. */
        rom[16u + 0x3FFCu] = 0x00u;
        rom[16u + 0x3FFDu] = 0xC0u;
        int rc = cartridge_from_bytes(rom, (size_t)sizeof(rom), &cart);
        CHECK(rc == 0, "bus_step_apu: cartridge_from_bytes succeeds");
        bus_insert_cartridge(&bus, &cart);

        bus_write(&bus, 0x4010u, 0x0Fu); /* rate index 15, no IRQ, no loop */
        bus_write(&bus, 0x4011u, 0x00u); /* output=0 */
        bus_write(&bus, 0x4012u, 0x00u); /* base=$C000 */
        bus_write(&bus, 0x4013u, 0x00u); /* length=1 */
        bus_write(&bus, 0x4015u, 0x10u); /* enable DMC */
        /* Step enough CPU cycles to fetch 1 byte (8 bits): ~42*8+1 APU cycles
         * = ~2*(42*8+1) CPU cycles. */
        bus_step_apu(&bus, 2u * (uint32_t)(42 * 8 + 1));
        /* DMC fetched 0xFF from cartridge PRG -> +16 (8 bits of 1). */
        CHECK_EQ_U8(dmc_output_counter(apu_dmc(&bus.apu)), 16u, "bus_step_apu DMC DMA reads cart PRG -> +16");

        bus_remove_cartridge(&bus);
    }

    /* bus_apu_irq_pending reflects frame_irq */
    bus_init(&bus);
    bus_write(&bus, 0x4017u, 0x00u); /* 4-step, IRQ enabled */
    bus_step_apu(&bus, 29833u);
    CHECK_EQ_BOOL(bus_apu_irq_pending(&bus), true, "bus_apu_irq_pending true after 4-step IRQ");

    /* bus_apu / bus_apu_const return &bus.apu */
    bus_init(&bus);
    CHECK(bus_apu(&bus) == &bus.apu, "bus_apu returns &bus.apu");
    CHECK(bus_apu_const(&bus) == &bus.apu, "bus_apu_const returns &bus.apu");

    /* $4000-$4013 reads return open bus (write-only regs) */
    bus_init(&bus);
    bus_write(&bus, 0x4000u, 0xABu);
    uint8_t r = bus_read(&bus, 0x4000u);
    CHECK_EQ_U8(r, 0xABu, "bus $4000 read returns open bus");

    /* $4014 read returns open bus */
    bus_init(&bus);
    bus.apu_open_bus[0x14u] = 0xCDu;
    r = bus_read(&bus, 0x4014u);
    CHECK_EQ_U8(r, 0xCDu, "bus $4014 read returns open bus");
}

/* ---- main ----------------------------------------------------------- */

int main(void) {
    printf("=== NES C core M4.3 APU acceptance test ===\n");
    test_constants();
    test_pulse();
    test_triangle();
    test_noise();
    test_dmc();
    test_apu();
    test_bus_integration();

    printf("\n=== Summary ===\n");
    printf("PASS: %d\n", g_pass);
    printf("FAIL: %d\n", g_fail);
    if (g_fail == 0) {
        printf("ALL TESTS PASSED\n");
        return 0;
    }
    printf("TESTS FAILED\n");
    return 1;
}
