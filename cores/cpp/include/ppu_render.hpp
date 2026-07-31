/*
 * ppu_render.h - PPU rendering pipeline: background + sprite pixel generation.
 *
 * Port of src/ppu/render.rs to C (M4.2). Defines the RenderPipeline transient
 * state, the ScanlineSprite / BgFetch helper structs, the ChrReader callback
 * (the C equivalent of the Rust ChrReader trait - the bus implements it to
 * route pattern-table reads through the cartridge), and the public render
 * entry points used by the PPU / bus / emulator.
 *
 * See: https://www.nesdev.org/wiki/PPU_rendering
 * See: https://www.nesdev.org/wiki/PPU_OAM
 */
#ifndef NES_CORE_C_PPU_RENDER_H
#define NES_CORE_C_PPU_RENDER_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>
#include "region.hpp"

#ifdef __cplusplus
extern "C" {
#endif

/* ---- Constants (render.rs) ------------------------------------------- */

/* Visible screen width in pixels (render.rs SCREEN_WIDTH). */
#define PPU_SCREEN_WIDTH  256u
/* Visible screen height in pixels - 240 scanlines (render.rs SCREEN_HEIGHT). */
#define PPU_SCREEN_HEIGHT 240u
/* Total framebuffer pixel count (render.rs FRAMEBUFFER_SIZE). */
#define PPU_FRAMEBUFFER_SIZE (PPU_SCREEN_WIDTH * PPU_SCREEN_HEIGHT)

/* Maximum number of sprites rendered on a single scanline (mod.rs
 * MAX_SPRITES_PER_SCANLINE). */
#define PPU_MAX_SPRITES_PER_SCANLINE 8u

/* ---- ChrReader (render.rs ChrReader trait) ------------------------- */

/* CHR pattern-table read callback. The bus implements this to route reads
 * through the cartridge (CHR-ROM or CHR-RAM). ctx is opaque user data
 * (typically the Bus*). (render.rs trait ChrReader { fn read_chr(&mut self,
 * addr: u16) -> u8; }.) */
typedef struct ChrReader {
    void* ctx;                                  /* opaque context (Bus* etc.) */
    uint8_t (*read_chr)(void* ctx, uint16_t addr); /* CHR read callback */
} ChrReader;

/* ---- Transient render pipeline structs (render.rs RenderPipeline) --- */

/* A sprite selected for the current scanline by sprite evaluation.
 * (render.rs sprites: [(usize, u8, u8, u8, u8); 8] - fields are
 * (oam_index, y, tile, attr, screen_x).) */
typedef struct ScanlineSprite {
    uint8_t oam_i;  /* OAM sprite index (0..63). */
    uint8_t y;      /* OAM byte 0 (Y position). */
    uint8_t tile;   /* OAM byte 1 (tile index). */
    uint8_t attr;   /* OAM byte 2 (attributes). */
    uint8_t sx;     /* OAM byte 3 (screen X). */
} ScanlineSprite;

/* A prefetched background pixel: (pattern 0..3, palette select 0..3).
 * (render.rs fetch_buffer: [(u8, u8); 2].) */
typedef struct BgFetch {
    uint8_t pattern;    /* 2-bit pattern value (0..3). */
    uint8_t pal_select; /* 2-bit attribute palette select (0..3). */
} BgFetch;

/* Transient per-pixel rendering pipeline state. All fields are recomputed
 * every scanline and are NOT part of the architectural PPU state. Defaults to
 * zero on power-on / save-state restore; the pipeline refills on the next
 * visible scanline. (render.rs struct RenderPipeline.) */
typedef struct RenderPipeline {
    /* Per-scanline vertical position snapshot. (render.rs coarse_y etc.) */
    uint16_t coarse_y;     /* coarse Y (bits 5-9 of v). */
    uint16_t fine_y;       /* fine Y (bits 12-14 of v). */
    uint16_t nt_v;         /* vertical nametable bit (NT_V_BIT). */

    /* Horizontal render position snapshot. */
    uint16_t coarse_x_start; /* coarse X at last snapshot. */
    uint16_t nt_h_start;     /* horizontal nametable bit at last snapshot. */
    uint8_t  fine_x_start;   /* fine X (0-7) at last snapshot. */
    uint16_t resync_px;      /* pixel X (0-255) of last snapshot/re-sync. */

    /* Per-scanline / per-frame latches. */
    bool scanline_initialized; /* snapshot + sprite eval done for this sl. */
    bool v_dirty;              /* mid-scanline v write -> re-sync next pixel. */
    bool rendered_this_frame;  /* renderer produced output this frame. */

    /* Sprite evaluation result for the current scanline. */
    ScanlineSprite sprites[PPU_MAX_SPRITES_PER_SCANLINE];
    uint8_t sprite_count;      /* valid entries in sprites. */
    bool overflow;             /* sprite overflow detected this scanline. */
    bool sprite_zero_hit;      /* sprite 0 hit set during the current frame. */

    /* Debug toggles. */
    bool slant_corruption;     /* fine-X scroll corruption bug. */
    bool use_inaccurate_palette; /* old incorrect NTSC palette. */
    bool nmi_retrigger;        /* retrigger NMI on every PPUCTRL bit7 write. */

    /* 2-pixel lookahead fetch pipeline. */
    BgFetch fetch_buffer[2];   /* ring buffer of prefetched bg pixels. */
    uint8_t fetch_idx;         /* read index into fetch_buffer (toggles 0/1). */
    bool pipeline_primed;      /* fetch buffer primed with 2 entries. */
} RenderPipeline;

/* ---- Palettes (render.rs NES_PALETTE / NES_PALETTE_INACCURATE /
 *        PAL_PALETTE) ------------------------------------------------- */

/* NES 2C02 (NTSC) reference palette - 64 entries x RGB.
 * (render.rs NES_PALETTE.) See: https://www.nesdev.org/wiki/PPU_palettes */
extern const uint8_t NES_PALETTE[64][3];

/* Inaccurate NTSC palette - the previous (incorrect) RGB values that produced
 * wrong colors (e.g. yellow pipes in Mario Bros). Kept for debug toggle.
 * (render.rs NES_PALETTE_INACCURATE.) */
extern const uint8_t NES_PALETTE_INACCURATE[64][3];

/* NES 2C07 (PAL) reference palette - 64 entries x RGB.
 * (render.rs PAL_PALETTE.) */
extern const uint8_t PAL_PALETTE[64][3];

/* ---- Color conversion (render.rs nes_color_to_argb*) --------------- */

/* Convert NES color index to ARGB using the region-appropriate palette.
 * (render.rs nes_color_to_argb_for.) ALPHA = 0xFF. */
uint32_t nes_color_to_argb_for(uint8_t index, Region region);

/* Convert NES color index to ARGB using the NTSC palette (backward compat).
 * (render.rs nes_color_to_argb.) */
uint32_t nes_color_to_argb(uint8_t index);

/* ---- Public render entry points (methods on Ppu in Rust) ------------- *
 * These operate on a Ppu* (forward-declared here to avoid a circular include
 * with ppu.h, which #includes this header). */
struct Ppu;

/* Render the entire visible background (256x240) into the framebuffer.
 * (render.rs Ppu::render_background.) */
void ppu_render_background(struct Ppu* p, ChrReader* chr);

/* The universal background color (palette entry $3F00) as ARGB.
 * (render.rs Ppu::universal_bg_argb.) */
uint32_t ppu_universal_bg_argb(const struct Ppu* p);

/* Render all sprites, compositing on top of the existing framebuffer.
 * (render.rs Ppu::render_sprites.) */
void ppu_render_sprites(struct Ppu* p, ChrReader* chr);

/* Render full frame: background then sprites. (render.rs Ppu::render_frame.) */
void ppu_render_frame(struct Ppu* p, ChrReader* chr);

/* Clear the entire framebuffer to a single ARGB value.
 * (render.rs Ppu::clear_framebuffer.) */
void ppu_clear_framebuffer(struct Ppu* p, uint32_t argb);

/* Read a single framebuffer pixel (ARGB) at (x, y). Returns the universal
 * background color for out-of-bounds coordinates. (render.rs Ppu::pixel.) */
uint32_t ppu_pixel(const struct Ppu* p, size_t x, size_t y);

/* Render one pixel at current (cycle, scanline) - cycle-accurate path used by
 * ppu_step_rendered. (render.rs Ppu::render_one_pixel.) */
void ppu_render_one_pixel(struct Ppu* p, ChrReader* chr);

#ifdef __cplusplus
}
#endif
#endif /* NES_CORE_C_PPU_RENDER_H */
