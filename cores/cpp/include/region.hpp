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

/* ---- M4.3: APU frame-counter region accessors (src/region.rs 87-208) - */

/* CPU clock rate in Hz. NTSC/Dendy = 1789773.0, PAL = 1662607.0.
 * (src/region.rs `Region::cpu_clock_hz`.) */
float region_cpu_clock_hz(Region r);

/* CPU cycles per audio sample at 44.1 kHz = cpu_clock_hz / 44100.
 * (src/region.rs `Region::cpu_cycles_per_sample`.) */
float region_cpu_cycles_per_sample(Region r);

/* A frame-counter threshold: cycle position + quarter/half flags.
 * (src/region.rs `(u32, bool, bool)` tuple.) */
typedef struct ApuFrameThreshold {
    uint32_t threshold;
    bool quarter;
    bool half;
} ApuFrameThreshold;

/* APU frame-counter 4-step mode thresholds in CPU cycles.
 * NTSC/Dendy: 7457, 14913, 22371, 29828 (IRQ at 29828).
 * PAL:        8314, 16627, 24941, 33255 (IRQ at 33255).
 * (src/region.rs `Region::apu_4step_thresholds`.) */
void region_apu_4step_thresholds(Region r, ApuFrameThreshold out[4]);

/* APU frame-counter 5-step mode thresholds in CPU cycles.
 * NTSC/Dendy: 7457, 14913, 22371, 37281.
 * PAL:        8314, 16627, 24941, 41568.
 * (src/region.rs `Region::apu_5step_thresholds`.) */
void region_apu_5step_thresholds(Region r, ApuFrameThreshold out[4]);

/* APU frame-counter reset point in CPU cycles (end of period).
 * NTSC/Dendy: 5step=37282, 4step=29830. PAL: 5step=41570, 4step=33257.
 * (src/region.rs `Region::apu_reset_at`.) */
uint32_t region_apu_reset_at(Region r, bool mode_5step);

/* APU 4-step mode IRQ threshold (the 4th step where IRQ is raised when not
 * inhibited). NTSC/Dendy = 29828, PAL = 33255.
 * (src/region.rs `Region::apu_4step_irq_threshold`.) */
uint32_t region_apu_4step_irq_threshold(Region r);

#ifdef __cplusplus
}
#endif
#endif /* NES_CORE_C_REGION_H */

