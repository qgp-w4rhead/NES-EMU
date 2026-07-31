/*
 * region.c — TV system / region.
 *
 * Port of src/region.rs. M4.1 introduced the enum + a configurable global that
 * defaults to NTSC. M4.2 adds the region-aware timing accessors used by the PPU
 * (scanlines_per_frame, scanline_prerender, is_pal_palette). These match
 * src/region.rs `Region::scanlines_per_frame` / `scanline_prerender` /
 * `is_pal_palette` field-for-field.
 */
#include "region.hpp"

static Region g_region = REGION_NTSC;

Region region_get(void) {
    return g_region;
}

void region_set(Region r) {
    g_region = r;
}

/* ---- M4.2: region-aware timing accessors (src/region.rs `Region` impl) - */

uint16_t region_scanlines_per_frame(Region r) {
    /* src/region.rs: Ntsc => 262, Pal | Dendy => 312. */
    switch (r) {
        case REGION_NTSC:  return 262u;
        case REGION_PAL:   return 312u;
        case REGION_DENDY: return 312u;
        default:           return 262u;
    }
}

uint16_t region_scanline_prerender(Region r) {
    /* src/region.rs: Ntsc => 261, Pal | Dendy => 311. */
    switch (r) {
        case REGION_NTSC:  return 261u;
        case REGION_PAL:   return 311u;
        case REGION_DENDY: return 311u;
        default:           return 261u;
    }
}

bool region_is_pal_palette(Region r) {
    /* src/region.rs: matches!(self, Region::Pal). Dendy uses NTSC palette. */
    return r == REGION_PAL;
}

/* ---- M4.3: APU frame-counter region accessors (src/region.rs 87-208) - */

float region_cpu_clock_hz(Region r) {
    /* src/region.rs: Ntsc | Dendy => 1_789_773.0, Pal => 1_662_607.0. */
    switch (r) {
        case REGION_NTSC:  return 1789773.0f;
        case REGION_PAL:   return 1662607.0f;
        case REGION_DENDY: return 1789773.0f;
        default:           return 1789773.0f;
    }
}

float region_cpu_cycles_per_sample(Region r) {
    /* src/region.rs: cpu_clock_hz / SAMPLE_RATE (44100). The 44100 literal
     * matches src/audio.rs `SAMPLE_RATE` and apu.h `APU_SAMPLE_RATE`; we use
     * the literal here to avoid a circular include with apu.h. */
    return region_cpu_clock_hz(r) / 44100.0f;
}

void region_apu_4step_thresholds(Region r, ApuFrameThreshold out[4]) {
    /* src/region.rs `apu_4step_thresholds`. NTSC/Dendy and PAL differ;
     * the quarter/half flag pattern is the same for both. */
    const uint32_t ntsc[4] = { 7457u, 14913u, 22371u, 29828u };
    const uint32_t pal[4]  = { 8314u, 16627u, 24941u, 33255u };
    const bool quarter[4]  = { true,  true,  true,  true  };
    const bool half[4]     = { false, true,  false, true  };
    const uint32_t* t = (r == REGION_PAL) ? pal : ntsc;
    for (int i = 0; i < 4; ++i) {
        out[i].threshold = t[i];
        out[i].quarter   = quarter[i];
        out[i].half      = half[i];
    }
}

void region_apu_5step_thresholds(Region r, ApuFrameThreshold out[4]) {
    /* src/region.rs `apu_5step_thresholds`. No IRQ in 5-step mode. */
    const uint32_t ntsc[4] = { 7457u, 14913u, 22371u, 37281u };
    const uint32_t pal[4]  = { 8314u, 16627u, 24941u, 41568u };
    const bool quarter[4]  = { true,  true,  true,  true  };
    const bool half[4]     = { false, true,  false, true  };
    const uint32_t* t = (r == REGION_PAL) ? pal : ntsc;
    for (int i = 0; i < 4; ++i) {
        out[i].threshold = t[i];
        out[i].quarter   = quarter[i];
        out[i].half      = half[i];
    }
}

uint32_t region_apu_reset_at(Region r, bool mode_5step) {
    /* src/region.rs `apu_reset_at`. */
    switch (r) {
        case REGION_NTSC:
        case REGION_DENDY:
            return mode_5step ? 37282u : 29830u;
        case REGION_PAL:
            return mode_5step ? 41570u : 33257u;
        default:
            return mode_5step ? 37282u : 29830u;
    }
}

uint32_t region_apu_4step_irq_threshold(Region r) {
    /* src/region.rs `apu_4step_irq_threshold`. */
    switch (r) {
        case REGION_NTSC:
        case REGION_DENDY:
            return 29828u;
        case REGION_PAL:
            return 33255u;
        default:
            return 29828u;
    }
}

