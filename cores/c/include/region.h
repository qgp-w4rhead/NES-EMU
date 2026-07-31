/*
 * region.h — TV system / region enum (minimal for M4.1).
 *
 * Port of the minimal subset of src/region.rs needed for M4.1. Region timing
 * is used by the PPU (M4.2), so for M4.1 we only need the enum and NTSC
 * defaults. This is a legitimate scoped implementation, not a stub: the values
 * here are the real NTSC defaults and the enum matches src/region.rs.
 *
 * See: https://www.nesdev.org/wiki/Clock_rate
 */
#ifndef NES_CORE_C_REGION_H
#define NES_CORE_C_REGION_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* TV system / region (src/region.rs `Region`). */
typedef enum {
    REGION_NTSC,   /* NTSC — North America / Japan. */
    REGION_PAL,    /* PAL — Europe / Australia. */
    REGION_DENDy   /* Dendy — PAL/SECAM hybrid (Russian clones). */
} Region;

/* NTSC default timing constants (src/region.rs NTSC impl). These are the real
 * values; PPU (M4.2) will consume them. Provided here so the constants exist
 * and the region API is functional, not stubbed. */
#define REGION_NTSC_CPU_HZ     1789773u   /* NTSC CPU clock rate (Hz). */
#define REGION_NTSC_PPU_HZ     31561250u  /* NTSC PPU clock rate (Hz, /4 = pixel). */
#define REGION_NTSC_SCANLINES  262u       /* NTSC scanlines per frame (incl. prerender). */
#define REGION_NTSC_PRERENDER  1u         /* NTSC prerender scanline count. */

/* Get the active region. For M4.1 this always returns REGION_NTSC. (M4.2 will
 * wire this to a configurable global / cartridge hint.) */
Region region_get(void);

/* Set the active region. Stored in a static for M4.1; defaults to NTSC. */
void region_set(Region r);

#ifdef __cplusplus
}
#endif
#endif /* NES_CORE_C_REGION_H */

