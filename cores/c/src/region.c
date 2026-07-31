/*
 * region.c — TV system / region.
 *
 * Port of src/region.rs. M4.1 introduced the enum + a configurable global that
 * defaults to NTSC. M4.2 adds the region-aware timing accessors used by the PPU
 * (scanlines_per_frame, scanline_prerender, is_pal_palette). These match
 * src/region.rs `Region::scanlines_per_frame` / `scanline_prerender` /
 * `is_pal_palette` field-for-field.
 */
#include "region.h"

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

