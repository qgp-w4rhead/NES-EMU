/*
 * region.c — TV system / region (minimal for M4.1).
 *
 * Port of the minimal subset of src/region.rs needed for M4.1. Region timing
 * is consumed by the PPU (M4.2), so for M4.1 we only need a configurable
 * global that defaults to NTSC. This is a legitimate scoped implementation,
 * not a stub: the enum and NTSC constants match src/region.rs.
 */
#include "region.h"

static Region g_region = REGION_NTSC;

Region region_get(void) {
    return g_region;
}

void region_set(Region r) {
    g_region = r;
}

