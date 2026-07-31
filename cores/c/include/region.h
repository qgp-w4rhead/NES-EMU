/*
 * region.h — TV system / region enum.
 *
 * Port of src/region.rs. M4.1 introduced the enum + a configurable global that
 * defaults to NTSC. M4.2 adds the region-aware timing accessors used by the PPU
 * (scanlines_per_frame, scanline_prerender, is_pal_palette). These match
 * src/region.rs `Region::scanlines_per_frame` / `scanline_prerender` /
 * `is_pal_palette` field-for-field.
 *
 * See: https://www.nesdev.org/wiki/Clock_rate
 * See: https://www.nesdev.org/wiki/PPU_rendering#Timing
 */
#ifndef NES_CORE_C_REGION_H
#define NES_CORE_C_REGION_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* TV system / region (src/region.rs `Region`). */
typedef enum {
    REGION_NTSC,   /* NTSC — North America / Japan. */
    REGION_PAL,    /* PAL — Europe / Australia. */
    REGION_DENDY   /* Dendy — PAL/SECAM hybrid (Russian clones). */
} Region;

/* NTSC default timing constants (src/region.rs NTSC impl). These are the real
 * values; the PPU (M4.2) consumes the region-aware accessors below. */
#define REGION_NTSC_CPU_HZ     1789773u   /* NTSC CPU clock rate (Hz). */
#define REGION_NTSC_PPU_HZ     31561250u  /* NTSC PPU clock rate (Hz, /4 = pixel). */
#define REGION_NTSC_SCANLINES  262u       /* NTSC scanlines per frame (incl. prerender). */
#define REGION_NTSC_PRERENDER  1u         /* NTSC prerender scanline count. */

/* Get the active region. Defaults to REGION_NTSC. (src/region.rs `Default`.) */
Region region_get(void);

/* Set the active region. Stored in a static; defaults to NTSC. */
void region_set(Region r);

/* ---- M4.2: region-aware timing accessors (src/region.rs `Region` impl) - */

/* Total scanlines per frame. NTSC = 262, PAL/Dendy = 312.
 * (src/region.rs `Region::scanlines_per_frame`.) */
uint16_t region_scanlines_per_frame(Region r);

/* Prerender scanline index (VBlank cleared + status flags cleared at cycle 1).
 * NTSC = 261, PAL/Dendy = 311. (src/region.rs `Region::scanline_prerender`.) */
uint16_t region_scanline_prerender(Region r);

/* Whether this region uses the PAL palette (2C07). Dendy uses the NTSC palette
 * despite PAL timing, so only REGION_PAL returns true.
 * (src/region.rs `Region::is_pal_palette`.) */
bool region_is_pal_palette(Region r);

#ifdef __cplusplus
}
#endif
#endif /* NES_CORE_C_REGION_H */

