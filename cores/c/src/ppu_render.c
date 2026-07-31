/*
 * ppu_render.c - PPU rendering pipeline: background + sprite pixel generation.
 *
 * Port of src/ppu/render.rs to C (M4.2). Implements the three reference
 * palettes (NTSC 2C02, NTSC inaccurate, PAL 2C07), the ARGB color conversion,
 * the whole-frame background/sprite renderers (render_background /
 * render_sprites / render_frame), and the per-pixel cycle-accurate renderer
 * (render_one_pixel) used by ppu_step_rendered.
 *
 * See: https://www.nesdev.org/wiki/PPU_rendering
 * See: https://www.nesdev.org/wiki/PPU_OAM
 * See: https://www.nesdev.org/wiki/PPU_palettes
 */
#include "ppu_render.h"
#include "ppu.h"
#include <string.h>

/* ---- Render constants (render.rs) ----------------------------------- */
/* Nametable tile grid is 32x30 tiles (256x240 pixels). */
#define NT_COLS 32u
#define NT_ROWS 30u
/* Attribute table is 8x8 quadrants of 4x4 tiles each. */
#define ATTR_TABLE_OFFSET 0x03C0u
/* Base address of the nametable region in PPU address space. */
#define NT_BASE 0x2000u
/* Base address of palette RAM. */
#define PAL_BASE 0x3F00u
/* Base address of sprite palettes in palette RAM ($3F10-$3F1F). */
#define SPRITE_PAL_BASE 0x3F10u

/* Sprite attribute byte (OAM byte 2) bits. */
#define ATTR_PALETTE_MASK     0x03u  /* bits 0-1: palette select (4-7). */
#define ATTR_PRIORITY_BEHIND  0x20u  /* bit 5: priority (1 = behind bg). */
#define ATTR_HFLIP            0x40u  /* bit 6: flip horizontally. */
#define ATTR_VFLIP            0x80u  /* bit 7: flip vertically. */

/* Number of sprites in OAM (64). */
#define SPRITE_COUNT 64u
/* Sprite height in 8x8 mode. */
#define SPRITE_HEIGHT_8X8 8u
/* Sprite height in 8x16 mode. */
#define SPRITE_HEIGHT_8X16 16u
/* Sprite width in pixels. */
#define SPRITE_WIDTH 8u
/* OAM Y value at or above which a sprite is hidden ($EF-$FE = 239-254). */
#define OAM_Y_HIDDEN 0xEFu
/* OAM Y value that halts sprite evaluation ($FF). */
#define OAM_Y_HALT 0xFFu
/* Sprite 0 hit is not triggered at x = 255 (hardware quirk). */
#define SPRITE_ZERO_HIT_MAX_X 255u

/* Alpha value for all framebuffer pixels (fully opaque). */
#define ALPHA 0xFFu

/* ---- Palettes (render.rs NES_PALETTE / NES_PALETTE_INACCURATE /
 *        PAL_PALETTE) ------------------------------------------------- */

/* NES 2C02 (NTSC) reference palette - 64 entries x RGB. (render.rs
 * NES_PALETTE.) See: https://www.nesdev.org/wiki/PPU_palettes */
const uint8_t NES_PALETTE[64][3] = {
    /* 0x00-0x0F - dark/unsaturated row */
    {0x7C, 0x7C, 0x7C}, {0x00, 0x00, 0xFC}, {0x00, 0x00, 0xBC}, {0x44, 0x28, 0xBC},
    {0x94, 0x00, 0x84}, {0xA8, 0x00, 0x20}, {0xA8, 0x10, 0x00}, {0x88, 0x14, 0x00},
    {0x50, 0x30, 0x00}, {0x00, 0x78, 0x00}, {0x00, 0x68, 0x00}, {0x00, 0x58, 0x00},
    {0x00, 0x40, 0x58}, {0x00, 0x00, 0x00}, {0x00, 0x00, 0x00}, {0x00, 0x00, 0x00},
    /* 0x10-0x1F - medium row */
    {0xBC, 0xBC, 0xBC}, {0x00, 0x78, 0xF8}, {0x00, 0x58, 0xF8}, {0x68, 0x44, 0xFC},
    {0xD8, 0x00, 0xCC}, {0xE4, 0x00, 0x58}, {0xF8, 0x38, 0x00}, {0xE4, 0x5C, 0x10},
    {0xAC, 0x7C, 0x00}, {0x00, 0xB8, 0x00}, {0x00, 0xA8, 0x00}, {0x00, 0xA8, 0x44},
    {0x00, 0x88, 0x88}, {0x00, 0x00, 0x00}, {0x00, 0x00, 0x00}, {0x00, 0x00, 0x00},
    /* 0x20-0x2F - bright row */
    {0xF8, 0xF8, 0xF8}, {0x3C, 0xBC, 0xFC}, {0x68, 0x88, 0xFC}, {0x98, 0x78, 0xF8},
    {0xF8, 0x78, 0xF8}, {0xF8, 0x58, 0x98}, {0xF8, 0x78, 0x58}, {0xFC, 0xA0, 0x44},
    {0xF8, 0xB8, 0x00}, {0xB8, 0xF8, 0x18}, {0x58, 0xD8, 0x54}, {0x58, 0xF8, 0x98},
    {0x00, 0xE8, 0xD8}, {0x78, 0x78, 0x78}, {0x00, 0x00, 0x00}, {0x00, 0x00, 0x00},
    /* 0x30-0x3F - bright row (near-mirrors of 0x20-0x2F). */
    {0xFC, 0xFC, 0xFC}, {0xA4, 0xE4, 0xFC}, {0xB8, 0xB8, 0xF8}, {0xD8, 0xB8, 0xF8},
    {0xF8, 0xB8, 0xF8}, {0xF8, 0xA4, 0xC0}, {0xF0, 0xD0, 0xB0}, {0xFC, 0xE0, 0xA8},
    {0xF8, 0xD8, 0x78}, {0xD8, 0xF8, 0x78}, {0xB8, 0xF8, 0xB8}, {0xB8, 0xF8, 0xD8},
    {0x00, 0xFC, 0xFC}, {0xF8, 0xD8, 0xF8}, {0x00, 0x00, 0x00}, {0x00, 0x00, 0x00}
};

/* Inaccurate NTSC palette - the previous (incorrect) RGB values. (render.rs
 * NES_PALETTE_INACCURATE.) */
const uint8_t NES_PALETTE_INACCURATE[64][3] = {
    /* 0x00-0x0F - dark/unsaturated row */
    {0x84, 0x84, 0x84}, {0x00, 0x1D, 0x2C}, {0x1C, 0x0C, 0x54}, {0x30, 0x04, 0x64},
    {0x48, 0x00, 0x5C}, {0x58, 0x00, 0x44}, {0x58, 0x00, 0x24}, {0x4C, 0x0C, 0x00},
    {0x38, 0x18, 0x00}, {0x20, 0x28, 0x00}, {0x0C, 0x3C, 0x00}, {0x00, 0x40, 0x00},
    {0x00, 0x3C, 0x1C}, {0x00, 0x38, 0x3C}, {0x00, 0x00, 0x00}, {0x00, 0x00, 0x00},
    /* 0x10-0x1F - medium row */
    {0xB4, 0xB4, 0xB4}, {0x38, 0x6C, 0xBC}, {0x54, 0x58, 0xEC}, {0x70, 0x44, 0xF4},
    {0x90, 0x38, 0xE4}, {0xA8, 0x34, 0xC8}, {0xB8, 0x34, 0x88}, {0xC0, 0x34, 0x44},
    {0xC4, 0x40, 0x14}, {0xC8, 0x50, 0x00}, {0xA8, 0x60, 0x00}, {0x88, 0x70, 0x00},
    {0x6C, 0x80, 0x00}, {0x44, 0x88, 0x00}, {0x00, 0x94, 0x00}, {0x00, 0x00, 0x00},
    /* 0x20-0x2F - bright row */
    {0xFF, 0xFF, 0xFF}, {0x9C, 0xDC, 0xFF}, {0xB8, 0xB8, 0xFF}, {0xD0, 0xB8, 0xFF},
    {0xFF, 0xB0, 0xF4}, {0xFF, 0xA8, 0xE0}, {0xFF, 0xA4, 0xC0}, {0xFF, 0xA0, 0x90},
    {0xF8, 0x94, 0x58}, {0xF0, 0xA0, 0x38}, {0xD8, 0xA8, 0x20}, {0xB8, 0xB0, 0x14},
    {0x98, 0xB8, 0x14}, {0x70, 0xC0, 0x14}, {0x50, 0xC8, 0x24}, {0x00, 0x00, 0x00},
    /* 0x30-0x3F - mirrors of 0x20-0x2F */
    {0xFF, 0xFF, 0xFF}, {0x9C, 0xDC, 0xFF}, {0xB8, 0xB8, 0xFF}, {0xD0, 0xB8, 0xFF},
    {0xFF, 0xB0, 0xF4}, {0xFF, 0xA8, 0xE0}, {0xFF, 0xA4, 0xC0}, {0xFF, 0xA0, 0x90},
    {0xF8, 0x94, 0x58}, {0xF0, 0xA0, 0x38}, {0xD8, 0xA8, 0x20}, {0xB8, 0xB0, 0x14},
    {0x98, 0xB8, 0x14}, {0x70, 0xC0, 0x14}, {0x50, 0xC8, 0x24}, {0x00, 0x00, 0x00}
};

/* NES 2C07 (PAL) reference palette - 64 entries x RGB. (render.rs
 * PAL_PALETTE.) */
const uint8_t PAL_PALETTE[64][3] = {
    /* 0x00-0x0F */
    {0x84, 0x84, 0x84}, {0x00, 0x1D, 0x2C}, {0x0C, 0x0C, 0x44}, {0x24, 0x04, 0x54},
    {0x3C, 0x00, 0x4C}, {0x4C, 0x00, 0x34}, {0x4C, 0x00, 0x18}, {0x40, 0x0C, 0x00},
    {0x2C, 0x18, 0x00}, {0x18, 0x28, 0x00}, {0x08, 0x3C, 0x00}, {0x00, 0x40, 0x00},
    {0x00, 0x3C, 0x1C}, {0x00, 0x38, 0x3C}, {0x04, 0x04, 0x04}, {0x00, 0x00, 0x00},
    /* 0x10-0x1F */
    {0xB4, 0xB4, 0xB4}, {0x30, 0x60, 0xA4}, {0x48, 0x48, 0xC8}, {0x60, 0x38, 0xD8},
    {0x80, 0x30, 0xC8}, {0x98, 0x2C, 0xAC}, {0xA8, 0x2C, 0x70}, {0xB0, 0x2C, 0x38},
    {0xB4, 0x38, 0x10}, {0xB8, 0x48, 0x00}, {0x98, 0x58, 0x00}, {0x78, 0x68, 0x00},
    {0x5C, 0x78, 0x00}, {0x38, 0x80, 0x00}, {0x00, 0x8C, 0x00}, {0x04, 0x04, 0x04},
    /* 0x20-0x2F */
    {0xFF, 0xFF, 0xFF}, {0x8C, 0xCC, 0xFF}, {0xA8, 0xA8, 0xFF}, {0xC0, 0xA8, 0xFF},
    {0xE8, 0xA4, 0xF0}, {0xE8, 0xA0, 0xD8}, {0xE8, 0x9C, 0xB8}, {0xE8, 0x98, 0x88},
    {0xE0, 0x8C, 0x54}, {0xD8, 0x98, 0x38}, {0xC0, 0xA0, 0x24}, {0xA0, 0xA8, 0x18},
    {0x84, 0xB0, 0x18}, {0x60, 0xB8, 0x18}, {0x44, 0xC0, 0x2C}, {0x04, 0x04, 0x04},
    /* 0x30-0x3F - mirrors of 0x20-0x2F */
    {0xFF, 0xFF, 0xFF}, {0x8C, 0xCC, 0xFF}, {0xA8, 0xA8, 0xFF}, {0xC0, 0xA8, 0xFF},
    {0xE8, 0xA4, 0xF0}, {0xE8, 0xA0, 0xD8}, {0xE8, 0x9C, 0xB8}, {0xE8, 0x98, 0x88},
    {0xE0, 0x8C, 0x54}, {0xD8, 0x98, 0x38}, {0xC0, 0xA0, 0x24}, {0xA0, 0xA8, 0x18},
    {0x84, 0xB0, 0x18}, {0x60, 0xB8, 0x18}, {0x44, 0xC0, 0x2C}, {0x04, 0x04, 0x04}
};

/* ---- Forward declarations for private static helpers ----------------- */

static void   ppu_render_background_scanline(Ppu* p, uint16_t py, ChrReader* chr);
static void   ppu_render_sprites_scanline(Ppu* p, uint16_t scanline, ChrReader* chr,
                                          bool* overflow_this_frame, bool* sprite_zero_hit_set);
static void   snapshot_render_position(Ppu* p, size_t px);
static void   effective_render_x(const Ppu* p, size_t px,
                                 uint16_t* out_coarse_x, uint16_t* out_nt_h, uint8_t* out_fine_x);
static BgFetch fetch_bg_pixel(const Ppu* p, size_t px, ChrReader* chr);
static void   evaluate_scanline_sprites(Ppu* p);
static bool   fetch_sprite_pixel(Ppu* p, size_t px, size_t py, ChrReader* chr,
                                 uint8_t* out_pattern, uint8_t* out_pal);
static uint32_t color_to_argb(const Ppu* p, uint8_t index);

/* ---- Color conversion (render.rs nes_color_to_argb*) ---------------- */

uint32_t nes_color_to_argb_for(uint8_t index, Region region) {
    /* render.rs `nes_color_to_argb_for`. */
    uint32_t idx = (uint32_t)(index & 0x3Fu);
    const uint8_t* c = region_is_pal_palette(region) ? PAL_PALETTE[idx] : NES_PALETTE[idx];
    return ((uint32_t)ALPHA << 24) | ((uint32_t)c[0] << 16) |
           ((uint32_t)c[1] << 8) | (uint32_t)c[2];
}

uint32_t nes_color_to_argb(uint8_t index) {
    return nes_color_to_argb_for(index, REGION_NTSC);
}

/* ---- Public render entry points ------------------------------------- */

/* Convert a 6-bit NES color index to ARGB using the PPU's current region and
 * the inaccurate-palette debug toggle. (render.rs `Ppu::color_to_argb`.) */
static uint32_t color_to_argb(const Ppu* p, uint8_t index) {
    uint32_t idx = (uint32_t)(index & 0x3Fu);
    const uint8_t* c;
    if (region_is_pal_palette(p->region)) {
        c = PAL_PALETTE[idx];
    } else if (p->render.use_inaccurate_palette) {
        c = NES_PALETTE_INACCURATE[idx];
    } else {
        c = NES_PALETTE[idx];
    }
    return ((uint32_t)ALPHA << 24) | ((uint32_t)c[0] << 16) |
           ((uint32_t)c[1] << 8) | (uint32_t)c[2];
}

uint32_t ppu_universal_bg_argb(const Ppu* p) {
    /* render.rs `Ppu::universal_bg_argb`. */
    return color_to_argb(p, ppu_read_palette(p, PAL_BASE));
}

void ppu_clear_framebuffer(Ppu* p, uint32_t argb) {
    /* render.rs `Ppu::clear_framebuffer`. */
    for (size_t i = 0; i < PPU_FRAMEBUFFER_SIZE; ++i) {
        p->framebuffer[i] = argb;
    }
}

uint32_t ppu_pixel(const Ppu* p, size_t x, size_t y) {
    /* render.rs `Ppu::pixel`. Out-of-bounds returns universal bg. */
    if (x < PPU_SCREEN_WIDTH && y < PPU_SCREEN_HEIGHT) {
        return p->framebuffer[y * PPU_SCREEN_WIDTH + x];
    }
    return ppu_universal_bg_argb(p);
}

void ppu_render_background(Ppu* p, ChrReader* chr) {
    /* render.rs `Ppu::render_background`. */
    for (uint16_t py = 0; py < PPU_SCREEN_HEIGHT; ++py) {
        ppu_render_background_scanline(p, py, chr);
    }
}

void ppu_render_sprites(Ppu* p, ChrReader* chr) {
    /* render.rs `Ppu::render_sprites`. */
    ppu_set_sprite_overflow(p, false);
    ppu_set_sprite_zero_hit(p, false);
    bool overflow_this_frame = false;
    bool sprite_zero_hit_set = false;
    for (uint16_t sl = 0; sl < PPU_SCREEN_HEIGHT; ++sl) {
        ppu_render_sprites_scanline(p, sl, chr, &overflow_this_frame, &sprite_zero_hit_set);
    }
    if (overflow_this_frame) {
        ppu_set_sprite_overflow(p, true);
    }
}

void ppu_render_frame(Ppu* p, ChrReader* chr) {
    /* render.rs `Ppu::render_frame`. */
    ppu_set_sprite_overflow(p, false);
    ppu_set_sprite_zero_hit(p, false);
    bool overflow_this_frame = false;
    bool sprite_zero_hit_set = false;
    for (uint16_t py = 0; py < PPU_SCREEN_HEIGHT; ++py) {
        ppu_render_background_scanline(p, py, chr);
        ppu_render_sprites_scanline(p, py, chr, &overflow_this_frame, &sprite_zero_hit_set);
    }
    if (overflow_this_frame) {
        ppu_set_sprite_overflow(p, true);
    }
}

/* ---- Per-scanline background renderer (render.rs
 *        `Ppu::render_background_scanline`) --------------------------- */

static void ppu_render_background_scanline(Ppu* p, uint16_t py, ChrReader* chr) {
    bool bg_enabled = (p->ppumask & PPU_MASK_SHOW_BG) != 0u;
    bool bg_left_enabled = (p->ppumask & PPU_MASK_SHOW_BG_LEFT) != 0u;
    uint32_t universal_bg = ppu_universal_bg_argb(p);
    size_t row_base = (size_t)py * PPU_SCREEN_WIDTH;

    if (!bg_enabled) {
        for (size_t px = 0; px < PPU_SCREEN_WIDTH; ++px) {
            p->framebuffer[row_base + px] = universal_bg;
            p->bg_pattern[px] = 0u;
        }
        return;
    }

    uint16_t bg_table = (p->ppuctrl & PPU_CTRL_BG_PATTERN_1000) ? 0x1000u : 0x0000u;
    uint16_t base_nt = (uint16_t)((uint16_t)(p->ppuctrl & PPU_CTRL_BASE_NT_MASK) << 10);
    uint16_t coarse_x = (uint16_t)(p->t & 0x001Fu);
    uint16_t coarse_y = (uint16_t)((p->t >> 5) & 0x001Fu);
    uint16_t fine_y = (uint16_t)((p->t >> 12) & 0x0007u);
    uint16_t scroll_x = (uint16_t)((coarse_x << 3) | p->fine_x);
    uint16_t scroll_y = (uint16_t)((coarse_y << 3) | fine_y);

    uint16_t gy = (uint16_t)(py + scroll_y);
    fine_y = (uint16_t)(gy & 0x0007u);
    uint16_t tile_row = (uint16_t)((gy >> 3) & 0x001Fu);
    uint16_t nt_v = (uint16_t)((gy >> 8) & 1u);

    for (uint16_t px = 0; px < PPU_SCREEN_WIDTH; ++px) {
        if (px < 8u && !bg_left_enabled) {
            p->framebuffer[row_base + px] = universal_bg;
            p->bg_pattern[px] = 0u;
            continue;
        }
        uint16_t gx = (uint16_t)(px + scroll_x);
        uint16_t fine_x = (uint16_t)(gx & 0x0007u);
        uint16_t tile_col = (uint16_t)((gx >> 3) & 0x001Fu);
        uint16_t nt_h = (uint16_t)((gx >> 8) & 1u);
        uint16_t nt = (uint16_t)(((base_nt >> 10) ^ nt_h ^ (uint16_t)(nt_v << 1)) & 0x03u);
        uint16_t nt_addr = (uint16_t)(NT_BASE | (nt << 10) | (tile_row << 5) | tile_col);
        uint16_t tile_index = (uint16_t)ppu_read_nametable(p, nt_addr);
        uint16_t pattern_addr = (uint16_t)(bg_table | (tile_index << 4) | fine_y);
        uint8_t plane0 = chr->read_chr(chr->ctx, pattern_addr);
        uint8_t plane1 = chr->read_chr(chr->ctx, (uint16_t)(pattern_addr | 0x08u));
        uint16_t bit = (uint16_t)(7u - fine_x);
        uint8_t pattern = (uint8_t)(((plane0 >> bit) & 1u) | (uint8_t)(((plane1 >> bit) & 1u) << 1));
        uint16_t attr_col = (uint16_t)(tile_col >> 2);
        uint16_t attr_row = (uint16_t)(tile_row >> 2);
        uint16_t attr_addr = (uint16_t)(NT_BASE | (nt << 10) | ATTR_TABLE_OFFSET | (attr_row << 3) | attr_col);
        uint8_t attr_byte = ppu_read_nametable(p, attr_addr);
        uint16_t shift = (uint16_t)(((tile_row & 0x02u) << 1) | (tile_col & 0x02u));
        uint8_t pal_select = (uint8_t)((attr_byte >> shift) & 0x03u);
        p->bg_pattern[px] = pattern;
        uint16_t color_addr = (pattern == 0u)
            ? PAL_BASE
            : (uint16_t)(PAL_BASE | ((uint16_t)pal_select << 2) | (uint16_t)pattern);
        uint8_t nes_index = ppu_read_palette(p, color_addr);
        p->framebuffer[row_base + px] = color_to_argb(p, nes_index);
    }
}

/* ---- Per-scanline sprite renderer (render.rs
 *        `Ppu::render_sprites_scanline`) ------------------------------ */

static void ppu_render_sprites_scanline(Ppu* p, uint16_t scanline, ChrReader* chr,
                                        bool* overflow_this_frame, bool* sprite_zero_hit_set) {
    bool sprites_enabled = (p->ppumask & PPU_MASK_SHOW_SPRITES) != 0u;
    if (!sprites_enabled) {
        return;
    }
    bool sprites_left_enabled = (p->ppumask & PPU_MASK_SHOW_SPRITES_LEFT) != 0u;
    bool sprite_size_16 = (p->ppuctrl & PPU_CTRL_SPRITE_SIZE_16) != 0u;
    uint16_t sprite_height = sprite_size_16 ? SPRITE_HEIGHT_8X16 : SPRITE_HEIGHT_8X8;
    uint16_t sprite_table_8x8 = (p->ppuctrl & PPU_CTRL_SPRITE_PATTERN_1000) ? 0x1000u : 0x0000u;

    ScanlineSprite selected[PPU_MAX_SPRITES_PER_SCANLINE];
    uint8_t count = 0u;
    for (uint16_t i = 0; i < SPRITE_COUNT; ++i) {
        uint16_t oam_idx = (uint16_t)(i * 4u);
        uint8_t y = p->oam[oam_idx];
        if (y == OAM_Y_HALT) {
            break;
        }
        if (y >= OAM_Y_HIDDEN) {
            continue;
        }
        uint16_t top = (uint16_t)(y + 1u);
        if (scanline < top || scanline >= top + sprite_height) {
            continue;
        }
        if (count < PPU_MAX_SPRITES_PER_SCANLINE) {
            selected[count].oam_i = (uint8_t)i;
            selected[count].y = y;
            selected[count].tile = p->oam[oam_idx + 1];
            selected[count].attr = p->oam[oam_idx + 2];
            selected[count].sx = p->oam[oam_idx + 3];
            count = (uint8_t)(count + 1u);
        } else {
            *overflow_this_frame = true;
        }
    }

    size_t row_base = (size_t)scanline * PPU_SCREEN_WIDTH;
    for (uint16_t px = 0; px < PPU_SCREEN_WIDTH; ++px) {
        if (px < 8u && !sprites_left_enabled) {
            continue;
        }
        for (uint8_t idx = 0; idx < count; ++idx) {
            uint8_t oam_i = selected[idx].oam_i;
            uint8_t y = selected[idx].y;
            uint8_t tile = selected[idx].tile;
            uint8_t attr = selected[idx].attr;
            uint16_t sx = selected[idx].sx;
            if (px < sx || px >= sx + SPRITE_WIDTH) {
                continue;
            }
            uint8_t tile_col = (uint8_t)(px - sx);
            uint8_t tile_row = (uint8_t)(scanline - (y + 1u));
            uint8_t row = (attr & ATTR_VFLIP) ? (uint8_t)((uint8_t)(sprite_height - 1u) - tile_row) : tile_row;
            uint8_t col = (attr & ATTR_HFLIP) ? (uint8_t)(7u - tile_col) : tile_col;

            uint16_t table, tile_base;
            if (sprite_size_16) {
                table = (tile & 1u) ? 0x1000u : 0x0000u;
                tile_base = (uint16_t)(tile & 0xFEu);
            } else {
                table = sprite_table_8x8;
                tile_base = tile;
            }
            uint8_t row_in_tile = sprite_size_16 ? (uint8_t)(row & 0x07u) : row;
            uint16_t tile_for_row = (sprite_size_16 && row >= 8u) ? (uint16_t)(tile_base + 1u) : tile_base;

            uint16_t pattern_addr = (uint16_t)(table | (tile_for_row << 4) | row_in_tile);
            uint8_t plane0 = chr->read_chr(chr->ctx, pattern_addr);
            uint8_t plane1 = chr->read_chr(chr->ctx, (uint16_t)(pattern_addr | 0x08u));
            uint16_t bit = (uint16_t)(7u - col);
            uint8_t pattern = (uint8_t)(((plane0 >> bit) & 1u) | (uint8_t)(((plane1 >> bit) & 1u) << 1));

            if (pattern == 0u) {
                continue;
            }

            size_t col_idx = row_base + px;
            bool bg_opaque = p->bg_pattern[px] != 0u;

            if (!*sprite_zero_hit_set && oam_i == 0u && px < SPRITE_ZERO_HIT_MAX_X && bg_opaque) {
                ppu_set_sprite_zero_hit(p, true);
                *sprite_zero_hit_set = true;
            }

            bool behind_bg = (attr & ATTR_PRIORITY_BEHIND) != 0u;
            if (behind_bg && bg_opaque) {
                break;
            }

            uint16_t pal = (uint16_t)(attr & ATTR_PALETTE_MASK);
            uint16_t color_addr = (uint16_t)(SPRITE_PAL_BASE | (pal << 2) | (uint16_t)pattern);
            uint8_t nes_index = ppu_read_palette(p, color_addr);
            p->framebuffer[col_idx] = color_to_argb(p, nes_index);
            break;
        }
    }
}

/* ---- Per-pixel (cycle-accurate) rendering (render.rs
 *        `Ppu::render_one_pixel` and helpers) ------------------------- */

/* Snapshot the per-scanline render position from v. (render.rs
 * `Ppu::snapshot_render_position`.) */
static void snapshot_render_position(Ppu* p, size_t px) {
    p->render.coarse_y = (uint16_t)((p->v >> 5) & 0x001Fu);
    p->render.fine_y = (uint16_t)((p->v >> 12) & 0x0007u);
    p->render.nt_v = (uint16_t)(p->v & PPU_NT_V_BIT);
    p->render.coarse_x_start = (uint16_t)(p->v & PPU_COARSE_X_MASK);
    p->render.nt_h_start = (uint16_t)(p->v & PPU_NT_H_BIT);
    p->render.fine_x_start = p->fine_x;
    p->render.resync_px = (uint16_t)px;
    p->render.v_dirty = false;
}

/* Compute effective horizontal render position for pixel px. Returns
 * (coarse_x, nt_h, fine_x) via out-params. (render.rs
 * `Ppu::effective_render_x`.) */
static void effective_render_x(const Ppu* p, size_t px,
                               uint16_t* out_coarse_x, uint16_t* out_nt_h, uint8_t* out_fine_x) {
    uint16_t coarse_x = p->render.coarse_x_start;
    uint16_t nt_h = p->render.nt_h_start;
    uint16_t fine_x_start = (uint16_t)p->render.fine_x_start;
    uint16_t advance = (uint16_t)px - p->render.resync_px;
    uint16_t new_fine_x = (uint16_t)((fine_x_start + advance) & 0x0007u);
    uint16_t cx_inc = (uint16_t)((fine_x_start + advance) >> 3);
    uint32_t total = (uint32_t)coarse_x + (uint32_t)cx_inc;
    uint32_t wraps = total / 32u;
    coarse_x = (uint16_t)(total % 32u);
    if ((wraps & 1u) != 0u) {
        nt_h ^= PPU_NT_H_BIT;
    }
    *out_coarse_x = coarse_x;
    *out_nt_h = nt_h;
    *out_fine_x = (uint8_t)new_fine_x;
}

/* Fetch bg pixel (pattern, pal_select) at output pixel px. (render.rs
 * `Ppu::fetch_bg_pixel`.) */
static BgFetch fetch_bg_pixel(const Ppu* p, size_t px, ChrReader* chr) {
    uint16_t coarse_x, nt_h;
    uint8_t fine_x;
    effective_render_x(p, px, &coarse_x, &nt_h, &fine_x);
    uint16_t nt = (uint16_t)(((nt_h >> 10) | (p->render.nt_v >> 10)) & 0x03u);
    uint16_t coarse_y = p->render.coarse_y;

    /* 1) Nametable fetch -> tile index. */
    uint16_t nt_addr = (uint16_t)(NT_BASE | (nt << 10) | (coarse_y << 5) | coarse_x);
    uint16_t tile_index = (uint16_t)ppu_read_nametable(p, nt_addr);

    /* 2) Pattern-table fetch (two bitplanes). */
    uint16_t bg_table = (p->ppuctrl & PPU_CTRL_BG_PATTERN_1000) ? 0x1000u : 0x0000u;
    uint16_t pattern_addr = (uint16_t)(bg_table | (tile_index << 4) | p->render.fine_y);
    uint8_t plane0 = chr->read_chr(chr->ctx, pattern_addr);
    uint8_t plane1 = chr->read_chr(chr->ctx, (uint16_t)(pattern_addr | 0x08u));
    uint16_t bit = (uint16_t)(7u - fine_x);
    uint8_t pattern = (uint8_t)(((plane0 >> bit) & 1u) | (uint8_t)(((plane1 >> bit) & 1u) << 1));

    /* 3) Attribute-table fetch -> 2-bit palette select. */
    uint16_t attr_col = (uint16_t)(coarse_x >> 2);
    uint16_t attr_row = (uint16_t)(coarse_y >> 2);
    uint16_t attr_addr = (uint16_t)(NT_BASE | (nt << 10) | ATTR_TABLE_OFFSET | (attr_row << 3) | attr_col);
    uint8_t attr_byte = ppu_read_nametable(p, attr_addr);
    uint16_t shift = (uint16_t)(((coarse_y & 0x02u) << 1) | (coarse_x & 0x02u));
    uint8_t pal_select = (uint8_t)((attr_byte >> shift) & 0x03u);

    BgFetch result;
    result.pattern = pattern;
    result.pal_select = pal_select;
    return result;
}

/* Evaluate sprites for the current scanline (select first 8 in range).
 * (render.rs `Ppu::evaluate_scanline_sprites`.) */
static void evaluate_scanline_sprites(Ppu* p) {
    p->render.sprite_count = 0u;
    p->render.overflow = false;

    bool sprite_size_16 = (p->ppuctrl & PPU_CTRL_SPRITE_SIZE_16) != 0u;
    uint16_t sprite_height = sprite_size_16 ? SPRITE_HEIGHT_8X16 : SPRITE_HEIGHT_8X8;
    uint16_t scanline = p->scanline;

    for (uint16_t i = 0; i < SPRITE_COUNT; ++i) {
        uint16_t oam_idx = (uint16_t)(i * 4u);
        uint8_t y = p->oam[oam_idx];
        if (y == OAM_Y_HALT) {
            break;
        }
        if (y >= OAM_Y_HIDDEN) {
            continue;
        }
        uint16_t top = (uint16_t)(y + 1u);
        if (scanline < top || scanline >= top + sprite_height) {
            continue;
        }
        if (p->render.sprite_count < PPU_MAX_SPRITES_PER_SCANLINE) {
            uint8_t idx = p->render.sprite_count;
            p->render.sprites[idx].oam_i = (uint8_t)i;
            p->render.sprites[idx].y = y;
            p->render.sprites[idx].tile = p->oam[oam_idx + 1];
            p->render.sprites[idx].attr = p->oam[oam_idx + 2];
            p->render.sprites[idx].sx = p->oam[oam_idx + 3];
            p->render.sprite_count = (uint8_t)(idx + 1u);
        } else {
            p->render.overflow = true;
        }
    }

    if (p->render.overflow) {
        ppu_set_sprite_overflow(p, true);
    }
}

/* Fetch sprite pixel at (px, py); handles sprite 0 hit detection. Returns
 * true and fills out_pattern/out_pal for an opaque, priority-OK sprite pixel.
 * (render.rs `Ppu::fetch_sprite_pixel`.) */
static bool fetch_sprite_pixel(Ppu* p, size_t px, size_t py, ChrReader* chr,
                               uint8_t* out_pattern, uint8_t* out_pal) {
    bool sprite_size_16 = (p->ppuctrl & PPU_CTRL_SPRITE_SIZE_16) != 0u;
    uint16_t sprite_height = sprite_size_16 ? SPRITE_HEIGHT_8X16 : SPRITE_HEIGHT_8X8;
    uint16_t sprite_table_8x8 = (p->ppuctrl & PPU_CTRL_SPRITE_PATTERN_1000) ? 0x1000u : 0x0000u;
    uint16_t scanline = (uint16_t)py;
    bool bg_opaque = p->bg_pattern[px] != 0u;

    for (uint8_t idx = 0; idx < p->render.sprite_count; ++idx) {
        uint8_t oam_i = p->render.sprites[idx].oam_i;
        uint8_t y = p->render.sprites[idx].y;
        uint8_t tile = p->render.sprites[idx].tile;
        uint8_t attr = p->render.sprites[idx].attr;
        uint16_t sx = p->render.sprites[idx].sx;
        if ((uint16_t)px < sx || (uint16_t)px >= sx + SPRITE_WIDTH) {
            continue;
        }
        uint8_t tile_col = (uint8_t)((uint16_t)px - sx);
        uint8_t tile_row = (uint8_t)(scanline - (y + 1u));
        uint8_t row = (attr & ATTR_VFLIP) ? (uint8_t)((uint8_t)(sprite_height - 1u) - tile_row) : tile_row;
        uint8_t col = (attr & ATTR_HFLIP) ? (uint8_t)(7u - tile_col) : tile_col;

        uint16_t table, tile_base;
        if (sprite_size_16) {
            table = (tile & 1u) ? 0x1000u : 0x0000u;
            tile_base = (uint16_t)(tile & 0xFEu);
        } else {
            table = sprite_table_8x8;
            tile_base = tile;
        }
        uint8_t row_in_tile = sprite_size_16 ? (uint8_t)(row & 0x07u) : row;
        uint16_t tile_for_row = (sprite_size_16 && row >= 8u) ? (uint16_t)(tile_base + 1u) : tile_base;

        uint16_t pattern_addr = (uint16_t)(table | (tile_for_row << 4) | row_in_tile);
        uint8_t plane0 = chr->read_chr(chr->ctx, pattern_addr);
        uint8_t plane1 = chr->read_chr(chr->ctx, (uint16_t)(pattern_addr | 0x08u));
        uint16_t bit = (uint16_t)(7u - col);
        uint8_t pattern = (uint8_t)(((plane0 >> bit) & 1u) | (uint8_t)(((plane1 >> bit) & 1u) << 1));

        if (pattern == 0u) {
            continue;
        }

        /* Sprite 0 hit detection (triggers once per frame). */
        if (!p->render.sprite_zero_hit && oam_i == 0u &&
            (uint16_t)px < SPRITE_ZERO_HIT_MAX_X && bg_opaque) {
            ppu_set_sprite_zero_hit(p, true);
            p->render.sprite_zero_hit = true;
        }

        bool behind_bg = (attr & ATTR_PRIORITY_BEHIND) != 0u;
        if (behind_bg && bg_opaque) {
            continue;
        }

        *out_pattern = pattern;
        *out_pal = (uint8_t)(attr & ATTR_PALETTE_MASK);
        return true;
    }
    return false;
}

void ppu_render_one_pixel(Ppu* p, ChrReader* chr) {
    /* render.rs `Ppu::render_one_pixel`. */
    size_t px = (size_t)(p->cycle - 1u);
    size_t py = (size_t)p->scanline;
    size_t col = py * PPU_SCREEN_WIDTH + px;

    /* First pixel of a scanline: snapshot + sprite evaluation. */
    if (!p->render.scanline_initialized) {
        snapshot_render_position(p, px);
        evaluate_scanline_sprites(p);
        p->render.scanline_initialized = true;
    }

    /* Re-sync on mid-scanline register writes. */
    if (p->render.v_dirty) {
        snapshot_render_position(p, px);
    }

    bool bg_enabled = (p->ppumask & PPU_MASK_SHOW_BG) != 0u;
    bool bg_left_enabled = (p->ppumask & PPU_MASK_SHOW_BG_LEFT) != 0u;
    bool sprites_enabled = (p->ppumask & PPU_MASK_SHOW_SPRITES) != 0u;
    bool sprites_left_enabled = (p->ppumask & PPU_MASK_SHOW_SPRITES_LEFT) != 0u;

    uint32_t pixel_argb;

    if (!bg_enabled) {
        pixel_argb = ppu_universal_bg_argb(p);
        p->bg_pattern[px] = 0u;
        p->render.pipeline_primed = false;
    } else {
        /* 2-pixel lookahead fetch pipeline. */
        if (!p->render.pipeline_primed) {
            BgFetch f0 = fetch_bg_pixel(p, px, chr);
            BgFetch f1 = fetch_bg_pixel(p, px + 1u, chr);
            p->render.fetch_buffer[0] = f0;
            p->render.fetch_buffer[1] = f1;
            p->render.fetch_idx = 0u;
            p->render.pipeline_primed = true;
        }

        uint8_t idx = p->render.fetch_idx;
        uint8_t pattern = p->render.fetch_buffer[idx].pattern;
        uint8_t pal_select = p->render.fetch_buffer[idx].pal_select;

        /* Prefetch the pixel 2 positions ahead. */
        BgFetch lookahead = fetch_bg_pixel(p, px + 2u, chr);
        p->render.fetch_buffer[idx] = lookahead;
        p->render.fetch_idx = (uint8_t)(idx ^ 1u);

        if (px < 8u && !bg_left_enabled) {
            pixel_argb = ppu_universal_bg_argb(p);
            p->bg_pattern[px] = 0u;
        } else {
            p->bg_pattern[px] = pattern;
            uint16_t color_addr = (pattern == 0u)
                ? PAL_BASE
                : (uint16_t)(PAL_BASE | ((uint16_t)pal_select << 2) | (uint16_t)pattern);
            uint8_t nes_index = ppu_read_palette(p, color_addr);
            pixel_argb = color_to_argb(p, nes_index);
        }
    }

    /* Composite sprite pixel on top. */
    if (sprites_enabled && (px >= 8u || sprites_left_enabled)) {
        uint8_t sprite_pattern, sprite_pal;
        if (fetch_sprite_pixel(p, px, py, chr, &sprite_pattern, &sprite_pal)) {
            uint16_t color_addr = (uint16_t)(SPRITE_PAL_BASE | ((uint16_t)sprite_pal << 2) | (uint16_t)sprite_pattern);
            uint8_t nes_index = ppu_read_palette(p, color_addr);
            pixel_argb = color_to_argb(p, nes_index);
        }
    }

    p->framebuffer[col] = pixel_argb;
}
