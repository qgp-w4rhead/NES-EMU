/*
 * test_ppu.c - M4.2 acceptance test for the NES C PPU.
 *
 * Verifies the port of src/ppu/mod.rs + src/ppu/render.rs is faithful to the
 * Rust source across:
 *  (a) ppu_init defaults + configuration (mirroring, region, debug flags).
 *  (b) Register reads/writes: PPUCTRL, PPUSTATUS, OAMDATA, PPUSCROLL, PPUADDR,
 *      PPUDATA buffered read + palette read, OAM DMA.
 *  (c) VBlank / NMI / sprite flag control.
 *  (d) step() cycle/scanline advance, VBlank timing, scroll increments.
 *  (e) Region timing (PAL/Dendy scanline counts).
 *  (f) Palette color conversion (NTSC).
 *  (g) render_frame / render_background / render_sprites / render_one_pixel
 *      with a stub ChrReader.
 *
 * Print PASS/FAIL per check and a final summary. Exit 0 on all pass, non-zero
 * on any failure.
 */
#include "ppu.hpp"
#include "ppu_render.hpp"
#include "region.hpp"
#include "cartridge.hpp"
#include "bus.hpp"
#include <stdint.h>
#include <stdio.h>
#include <string.h>

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

/* ---- Stub ChrReader -------------------------------------------------- *
 * Returns pattern bytes from a static 8 KB array. Tests can pre-fill the
 * array to set up pattern-table data. */

static uint8_t g_chr[8192];

static uint8_t stub_read_chr(void* ctx, uint16_t addr) {
    (void)ctx;
    return g_chr[addr & 0x1FFFu];
}

static ChrReader make_stub_reader(void) {
    ChrReader r;
    r.ctx = NULL;
    r.read_chr = stub_read_chr;
    return r;
}

/* ---- (a) ppu_init defaults + configuration --------------------------- */

static void test_init_defaults(void) {
    Ppu p;
    ppu_init(&p);
    printf("\n== (a) ppu_init defaults + configuration ==\n");

    CHECK_EQ_U8(ppu_ppuctrl(&p), 0x00, "ppu_init: PPUCTRL == 0");
    CHECK_EQ_U8(ppu_ppumask(&p), 0x00, "ppu_init: PPUMASK == 0");
    CHECK_EQ_U8(ppu_oamaddr(&p), 0x00, "ppu_init: OAMADDR == 0");
    CHECK_EQ_U8(ppu_ppustatus(&p), 0x00, "ppu_init: PPUSTATUS == 0");
    CHECK_EQ_U16(ppu_vram_addr(&p), 0x0000, "ppu_init: v == 0");
    CHECK_EQ_U16(ppu_temp_vram_addr(&p), 0x0000, "ppu_init: t == 0");
    CHECK_EQ_U8(ppu_fine_x(&p), 0x00, "ppu_init: fine_x == 0");
    CHECK(!ppu_write_latch(&p), "ppu_init: w == false");
    CHECK_EQ_U8(ppu_ppudata_buffer(&p), 0x00, "ppu_init: ppudata_buffer == 0");
    CHECK_EQ_U8(ppu_open_bus(&p), 0x00, "ppu_init: open_bus == 0");
    CHECK_EQ_U16(ppu_scanline(&p), 0, "ppu_init: scanline == 0");
    CHECK_EQ_U16(ppu_cycle(&p), 0, "ppu_init: cycle == 0");
    CHECK(!ppu_nmi_enabled(&p), "ppu_init: NMI disabled");
    CHECK(!ppu_in_vblank(&p), "ppu_init: not in VBlank");
    CHECK(ppu_mirroring(&p) == MIRROR_HORIZONTAL, "ppu_init: mirroring == HORIZONTAL");
    CHECK(ppu_region(&p) == REGION_NTSC, "ppu_init: region == NTSC");
    CHECK_EQ_U32(p.vram_size, 0x0800u, "ppu_init: vram_size == 0x800 (2KB)");

    const uint32_t* fb = ppu_framebuffer(&p);
    bool fb_zero = true;
    for (size_t i = 0; i < PPU_FRAMEBUFFER_SIZE; ++i) {
        if (fb[i] != 0u) { fb_zero = false; break; }
    }
    CHECK(fb_zero, "ppu_init: framebuffer all zero");

    ppu_set_mirroring(&p, MIRROR_FOUR_SCREEN);
    CHECK_EQ_U32(p.vram_size, 0x1000u, "set_mirroring(FOUR_SCREEN): vram_size == 0x1000");
    ppu_set_mirroring(&p, MIRROR_HORIZONTAL);
    CHECK_EQ_U32(p.vram_size, 0x0800u, "set_mirroring(HORIZONTAL): vram_size == 0x800");

    ppu_set_region(&p, REGION_PAL);
    CHECK(ppu_region(&p) == REGION_PAL, "set_region(PAL): region == PAL");
    CHECK_EQ_U16(region_scanlines_per_frame(ppu_region(&p)), 312u, "PAL scanlines_per_frame == 312");
    CHECK_EQ_U16(region_scanline_prerender(ppu_region(&p)), 311u, "PAL scanline_prerender == 311");
    ppu_set_region(&p, REGION_NTSC);
    CHECK_EQ_U16(region_scanlines_per_frame(ppu_region(&p)), 262u, "NTSC scanlines_per_frame == 262");
    CHECK_EQ_U16(region_scanline_prerender(ppu_region(&p)), 261u, "NTSC scanline_prerender == 261");

    CHECK(region_is_pal_palette(REGION_PAL), "region_is_pal_palette(PAL) == true");
    CHECK(!region_is_pal_palette(REGION_NTSC), "region_is_pal_palette(NTSC) == false");
    CHECK(!region_is_pal_palette(REGION_DENDY), "region_is_pal_palette(DENDY) == false");
}

/* ---- (b) Register reads/writes --------------------------------------- */

static void test_registers(void) {
    Ppu p;
    ppu_init(&p);
    printf("\n== (b) Register reads/writes ==\n");

    ppu_write_register(&p, 0, 0x02);
    CHECK_EQ_U8(ppu_ppuctrl(&p), 0x02, "PPUCTRL write: ppuctrl == 0x02");
    CHECK_EQ_U16(ppu_temp_vram_addr(&p) & PPU_NT_SELECT_MASK, (uint16_t)(0x02u << 10), "PPUCTRL: t nt bits == 0b10");
    CHECK(!ppu_nmi_enabled(&p), "PPUCTRL bit7=0: NMI disabled");
    ppu_write_register(&p, 0, 0x80);
    CHECK(ppu_nmi_enabled(&p), "PPUCTRL bit7=1: NMI enabled");

    ppu_set_vblank(&p, true);
    ppu_write_register(&p, 0, 0x1F);
    uint8_t r = ppu_read_register(&p, 2);
    CHECK_EQ_U8(r, (uint8_t)(0x80u | 0x1Fu), "PPUSTATUS read: VBlank | open_bus low 5 bits");
    CHECK(!ppu_in_vblank(&p), "PPUSTATUS read clears VBlank");
    CHECK(!ppu_write_latch(&p), "PPUSTATUS read resets w");
    CHECK_EQ_U8(ppu_open_bus(&p), (uint8_t)(0x80u | 0x1Fu), "PPUSTATUS read latches combined byte onto open bus");

    ppu_write_register(&p, 3, 0x10);
    ppu_write_register(&p, 4, 0xCC);
    CHECK_EQ_U8(ppu_oamaddr(&p), 0x11, "OAMDATA write increments oamaddr");
    ppu_write_register(&p, 3, 0x10);
    uint8_t v = ppu_read_register(&p, 4);
    CHECK_EQ_U8(v, 0xCC, "OAMDATA read returns OAM[oamaddr]");
    CHECK_EQ_U8(ppu_oamaddr(&p), 0x11, "OAMDATA read increments oamaddr");

    ppu_write_register(&p, 3, 0xFF);
    ppu_write_register(&p, 4, 0xAA);
    CHECK_EQ_U8(ppu_oamaddr(&p), 0x00, "OAMDATA write wraps oamaddr 0xFF -> 0x00");

    ppu_init(&p);
    ppu_write_register(&p, 5, 0x7D);
    CHECK_EQ_U8(ppu_fine_x(&p), 5, "PPUSCROLL 1st: fine_x == 5");
    CHECK_EQ_U16(ppu_temp_vram_addr(&p) & 0x1Fu, 0x0Fu, "PPUSCROLL 1st: t coarse X == 0x0F");
    CHECK(ppu_write_latch(&p), "PPUSCROLL 1st: w == true");
    ppu_write_register(&p, 5, 0x1B);
    CHECK(!ppu_write_latch(&p), "PPUSCROLL 2nd: w == false");
    uint16_t t = ppu_temp_vram_addr(&p);
    CHECK_EQ_U16(t & 0x1Fu, 0x0Fu, "PPUSCROLL 2nd: coarse X preserved");
    CHECK_EQ_U16((t >> 5) & 0x1Fu, 3u, "PPUSCROLL 2nd: coarse Y == 3");
    CHECK_EQ_U16((t >> 12) & 0x07u, 3u, "PPUSCROLL 2nd: fine Y == 3");

    ppu_init(&p);
    ppu_write_register(&p, 6, 0x21);
    CHECK(ppu_write_latch(&p), "PPUADDR 1st: w == true");
    CHECK_EQ_U16(ppu_temp_vram_addr(&p) & 0xFF00u, 0x2100u, "PPUADDR 1st: t hi == 0x21");
    ppu_write_register(&p, 6, 0x08);
    CHECK(!ppu_write_latch(&p), "PPUADDR 2nd: w == false");
    CHECK_EQ_U16(ppu_vram_addr(&p), 0x2108u, "PPUADDR 2nd: v == 0x2108");

    ppu_init(&p);
    ppu_write_register(&p, 6, 0xFF);
    ppu_write_register(&p, 6, 0x00);
    CHECK_EQ_U16(ppu_vram_addr(&p), 0x3F00u, "PPUADDR 0xFF masked to 0x3F");

    /* PPUDATA buffered read via the bus (CHR-routed path). */
    {
        Bus bus;
        bus_init(&bus);
        bus_write(&bus, 0x2006, 0x20);
        bus_write(&bus, 0x2006, 0x00);
        ppu_write_nametable(&bus.ppu, 0x2000, 0xAB);
        uint8_t r1 = bus_read(&bus, 0x2007);
        CHECK_EQ_U8(r1, 0x00, "PPUDATA 1st read returns stale buffer (0)");
        CHECK_EQ_U16(ppu_vram_addr(&bus.ppu), 0x2001u, "PPUDATA read advances v by 1");
        bus_write(&bus, 0x2006, 0x20);
        bus_write(&bus, 0x2006, 0x00);
        uint8_t r2 = bus_read(&bus, 0x2007);
        CHECK_EQ_U8(r2, 0xAB, "PPUDATA 2nd read returns fetched value (0xAB)");

        bus_write(&bus, 0x2000, 0x04);
        bus_write(&bus, 0x2006, 0x20);
        bus_write(&bus, 0x2006, 0x00);
        (void)bus_read(&bus, 0x2007);
        CHECK_EQ_U16(ppu_vram_addr(&bus.ppu), 0x2020u, "PPUDATA read with CTRL_INCREMENT_32 advances v by 32");
    }

    /* PPUDATA palette read returns palette value directly with open_bus bits 6-7. */
    {
        Bus bus;
        bus_init(&bus);
        ppu_write_palette(&bus.ppu, 0x3F00, 0x21);
        bus_write(&bus, 0x2006, 0x3F);
        bus_write(&bus, 0x2006, 0x00);
        bus_write(&bus, 0x2000, 0xC0);
        uint8_t pr = bus_read(&bus, 0x2007);
        CHECK_EQ_U8(pr, (uint8_t)((0x21u & 0x3Fu) | (0xC0u & 0xC0u)), "PPUDATA palette read: value | open_bus bits 6-7");
    }

    /* oam_dma copies 256 bytes and resets oamaddr to 0. */
    {
        Ppu p2;
        ppu_init(&p2);
        uint8_t data[256];
        for (int i = 0; i < 256; ++i) {
            data[i] = (uint8_t)((i + 0x40) & 0xFF);
        }
        ppu_write_register(&p2, 3, 0x42);
        ppu_oam_dma(&p2, data);
        const uint8_t* oam = ppu_oam(&p2);
        bool ok = true;
        for (int i = 0; i < 256; ++i) {
            if (oam[i] != data[i]) { ok = false; break; }
        }
        CHECK(ok, "oam_dma copies 256 bytes");
        CHECK_EQ_U8(ppu_oamaddr(&p2), 0x00, "oam_dma resets oamaddr to 0");
    }
}

/* ---- (c) VBlank / NMI / sprite flag control ------------------------- */

static void test_flags(void) {
    Ppu p;
    ppu_init(&p);
    printf("\n== (c) VBlank / NMI / sprite flag control ==\n");

    ppu_set_vblank(&p, true);
    CHECK(ppu_in_vblank(&p), "set_vblank(true): in_vblank");
    ppu_set_vblank(&p, false);
    CHECK(!ppu_in_vblank(&p), "set_vblank(false): not in_vblank");

    ppu_set_sprite_zero_hit(&p, true);
    CHECK(ppu_sprite_zero_hit(&p), "set_sprite_zero_hit(true)");
    ppu_set_sprite_zero_hit(&p, false);
    CHECK(!ppu_sprite_zero_hit(&p), "set_sprite_zero_hit(false)");

    ppu_set_sprite_overflow(&p, true);
    CHECK(ppu_sprite_overflow(&p), "set_sprite_overflow(true)");
    ppu_set_sprite_overflow(&p, false);
    CHECK(!ppu_sprite_overflow(&p), "set_sprite_overflow(false)");

    ppu_set_vblank(&p, true);
    ppu_set_sprite_zero_hit(&p, true);
    ppu_set_sprite_overflow(&p, true);
    CHECK_EQ_U8(ppu_ppustatus(&p) & 0xE0u, 0xE0u, "PPUSTATUS flags: VBlank|sprite0|overflow");

    (void)ppu_read_register(&p, 2);
    CHECK(!ppu_in_vblank(&p), "PPUSTATUS read clears VBlank");
    CHECK(ppu_sprite_zero_hit(&p), "sprite 0 hit survives PPUSTATUS read");
    CHECK(ppu_sprite_overflow(&p), "sprite overflow survives PPUSTATUS read");

    ppu_init(&p);
    p.nmi_request = true;
    CHECK(ppu_take_nmi_request(&p), "take_nmi_request returns true once");
    CHECK(!ppu_take_nmi_request(&p), "take_nmi_request returns false after take");

    ppu_init(&p);
    CHECK(!ppu_is_rendering(&p), "is_rendering false when mask == 0");
    ppu_write_register(&p, 1, PPU_MASK_SHOW_BG);
    CHECK(ppu_is_rendering(&p), "is_rendering true when SHOW_BG set");
    ppu_write_register(&p, 1, PPU_MASK_SHOW_SPRITES);
    CHECK(ppu_is_rendering(&p), "is_rendering true when SHOW_SPRITES set");
    ppu_write_register(&p, 1, 0);
    CHECK(!ppu_is_rendering(&p), "is_rendering false when mask cleared");
}

/* ---- (d) step() cycle/scanline advance, VBlank, scroll -------------- */

static void test_step(void) {
    Ppu p;
    ppu_init(&p);
    printf("\n== (d) step() cycle/scanline advance, VBlank, scroll ==\n");

    CHECK_EQ_U16(ppu_scanline(&p), 0, "step: initial scanline 0");
    CHECK_EQ_U16(ppu_cycle(&p), 0, "step: initial cycle 0");
    ppu_step(&p);
    CHECK_EQ_U16(ppu_cycle(&p), 1, "step: cycle 1 after one step");
    CHECK_EQ_U16(ppu_scanline(&p), 0, "step: scanline 0 after one step");

    ppu_init(&p);
    for (int i = 0; i < (PPU_CYCLES_PER_SCANLINE - 1); ++i) {
        ppu_step(&p);
    }
    CHECK_EQ_U16(ppu_cycle(&p), (uint16_t)(PPU_CYCLES_PER_SCANLINE - 1), "step: last cycle of scanline 0");
    CHECK_EQ_U16(ppu_scanline(&p), 0, "step: still scanline 0 at last cycle");
    ppu_step(&p);
    CHECK_EQ_U16(ppu_cycle(&p), 0, "step: cycle wraps to 0");
    CHECK_EQ_U16(ppu_scanline(&p), 1, "step: scanline advances to 1");

    ppu_init(&p);
    uint32_t total = (uint32_t)PPU_SCANLINES_PER_FRAME * (uint32_t)PPU_CYCLES_PER_SCANLINE;
    for (uint32_t i = 0; i < total - 1u; ++i) {
        ppu_step(&p);
    }
    CHECK_EQ_U16(ppu_scanline(&p), PPU_SCANLINE_PRERENDER, "step: at prerender scanline 261");
    CHECK_EQ_U16(ppu_cycle(&p), (uint16_t)(PPU_CYCLES_PER_SCANLINE - 1), "step: last cycle of prerender");
    ppu_step(&p);
    CHECK_EQ_U16(ppu_scanline(&p), 0, "step: scanline wraps to 0 after prerender");
    CHECK_EQ_U16(ppu_cycle(&p), 0, "step: cycle 0 after wrap");

    ppu_init(&p);
    ppu_write_register(&p, 0, 0x80);
    uint32_t pre = (uint32_t)PPU_SCANLINE_VBLANK_START * (uint32_t)PPU_CYCLES_PER_SCANLINE;
    for (uint32_t i = 0; i < pre; ++i) {
        ppu_step(&p);
    }
    CHECK_EQ_U16(ppu_scanline(&p), PPU_SCANLINE_VBLANK_START, "step: at scanline 241");
    CHECK_EQ_U16(ppu_cycle(&p), 0, "step: cycle 0 at scanline 241");
    CHECK(!ppu_in_vblank(&p), "step: not yet in VBlank at cycle 0");
    bool nmi = ppu_step(&p);
    CHECK_EQ_U16(ppu_cycle(&p), 1, "step: cycle 1 at scanline 241");
    CHECK(ppu_in_vblank(&p), "step: VBlank set at scanline 241 cycle 1");
    CHECK(nmi, "step: NMI requested when nmi_enabled");
    CHECK(ppu_take_nmi_request(&p), "step: nmi_request latched");
    CHECK(!ppu_take_nmi_request(&p), "step: nmi_request cleared after take");

    ppu_init(&p);
    ppu_write_register(&p, 0, 0x00);
    ppu_set_sprite_overflow(&p, true);
    ppu_set_sprite_zero_hit(&p, true);
    uint32_t to_prerender = (uint32_t)PPU_SCANLINE_PRERENDER * (uint32_t)PPU_CYCLES_PER_SCANLINE;
    for (uint32_t i = 0; i < to_prerender; ++i) {
        ppu_step(&p);
    }
    CHECK_EQ_U16(ppu_scanline(&p), PPU_SCANLINE_PRERENDER, "step: at prerender 261");
    CHECK_EQ_U16(ppu_cycle(&p), 0, "step: prerender cycle 0");
    CHECK(ppu_in_vblank(&p), "step: still in VBlank just before prerender cycle 1");
    ppu_step(&p);
    CHECK(!ppu_in_vblank(&p), "step: VBlank cleared at prerender cycle 1");
    CHECK(!ppu_sprite_zero_hit(&p), "step: sprite 0 hit cleared at prerender");
    CHECK(!ppu_sprite_overflow(&p), "step: sprite overflow cleared at prerender");

    ppu_init(&p);
    ppu_write_register(&p, 1, PPU_MASK_SHOW_BG);
    for (int i = 0; i < 8; ++i) {
        ppu_step(&p);
    }
    CHECK_EQ_U16(ppu_cycle(&p), 8, "scroll: cycle 8");
    CHECK_EQ_U16(ppu_vram_addr(&p) & PPU_COARSE_X_MASK, 1u, "scroll: coarse X incremented at cycle 8");
    CHECK_EQ_U8(ppu_fine_x(&p), 0, "scroll: fine_x unchanged (PPUSCROLL)");

    ppu_init(&p);
    ppu_write_register(&p, 1, PPU_MASK_SHOW_BG);
    for (int i = 0; i < 256; ++i) {
        ppu_step(&p);
    }
    CHECK_EQ_U16(ppu_cycle(&p), 256, "scroll: cycle 256");
    CHECK_EQ_U16((ppu_vram_addr(&p) & PPU_FINE_Y_MASK) >> 12, 1u, "scroll: fine Y incremented at cycle 256");

    ppu_init(&p);
    ppu_write_register(&p, 1, PPU_MASK_SHOW_BG);
    ppu_write_register(&p, 0, 0x03);
    ppu_write_register(&p, 5, 0x50);
    ppu_write_register(&p, 5, 0x00);
    for (int i = 0; i < 257; ++i) {
        ppu_step(&p);
    }
    CHECK_EQ_U16(ppu_cycle(&p), 257, "scroll: cycle 257");
    CHECK_EQ_U16(ppu_vram_addr(&p) & PPU_COARSE_X_MASK, 10u, "scroll: h-copy sets v coarse X = t coarse X (10)");
    CHECK_EQ_U16(ppu_vram_addr(&p) & PPU_NT_H_BIT, PPU_NT_H_BIT, "scroll: h-copy sets v nt H bit");

    ppu_init(&p);
    for (int i = 0; i < PPU_CYCLES_PER_SCANLINE; ++i) {
        ppu_step(&p);
    }
    CHECK_EQ_U16(ppu_vram_addr(&p), 0, "scroll: v unchanged when rendering disabled");
    CHECK_EQ_U8(ppu_fine_x(&p), 0, "scroll: fine_x unchanged when rendering disabled");
}

/* ---- (e) Region timing (PAL/Dendy) ---------------------------------- */

static void test_region_timing(void) {
    printf("\n== (e) Region timing (PAL/Dendy) ==\n");
    Ppu p;
    ppu_init(&p);
    ppu_set_region(&p, REGION_PAL);
    CHECK_EQ_U16(region_scanlines_per_frame(ppu_region(&p)), 312u, "PAL: scanlines_per_frame == 312");
    CHECK_EQ_U16(region_scanline_prerender(ppu_region(&p)), 311u, "PAL: scanline_prerender == 311");

    ppu_set_region(&p, REGION_DENDY);
    CHECK_EQ_U16(region_scanlines_per_frame(ppu_region(&p)), 312u, "Dendy: scanlines_per_frame == 312");
    CHECK_EQ_U16(region_scanline_prerender(ppu_region(&p)), 311u, "Dendy: scanline_prerender == 311");

    ppu_init(&p);
    p.scanline = 300;
    ppu_set_region(&p, REGION_NTSC);
    CHECK(ppu_scanline(&p) < PPU_SCANLINES_PER_FRAME, "set_region(NTSC) clamps scanline < 262");
}

/* ---- (f) Palette color conversion (NTSC) ---------------------------- */

static void test_palette(void) {
    printf("\n== (f) Palette color conversion (NTSC) ==\n");
    CHECK_EQ_U32(nes_color_to_argb(0x00), 0xFF7C7C7Cu, "nes_color_to_argb(0x00) == grey 0xFF7C7C7C");
    CHECK_EQ_U32(nes_color_to_argb(0x20), 0xFFF8F8F8u, "nes_color_to_argb(0x20) == white 0xFFF8F8F8");
    CHECK_EQ_U32(nes_color_to_argb(0x0F), 0xFF000000u, "nes_color_to_argb(0x0F) == black 0xFF000000");
    CHECK_EQ_U32(nes_color_to_argb(0x21), 0xFF3CBCFCu, "nes_color_to_argb(0x21) == 0xFF3CBCFC");
    CHECK_EQ_U32(nes_color_to_argb(0x40), nes_color_to_argb(0x00), "nes_color_to_argb masks to 6 bits (0x40 -> 0x00)");
}

/* ---- (g) render_frame / render_background / render_sprites ---------- */

static void test_render(void) {
    printf("\n== (g) render_frame / render_background / render_sprites ==\n");

    /* render_frame with bg disabled fills framebuffer with universal bg color. */
    {
        Ppu p;
        ppu_init(&p);
        ppu_write_palette(&p, 0x3F00, 0x0E);
        ChrReader chr = make_stub_reader();
        ppu_render_frame(&p, &chr);
        uint32_t bg = nes_color_to_argb(0x0E);
        bool all_bg = true;
        const uint32_t* fb = ppu_framebuffer(&p);
        for (size_t i = 0; i < PPU_FRAMEBUFFER_SIZE; ++i) {
            if (fb[i] != bg) { all_bg = false; break; }
        }
        CHECK(all_bg, "render_frame with bg disabled fills universal bg color");
    }

    /* render_frame with a simple nametable+pattern setup produces non-zero pixels. */
    {
        Ppu p;
        ppu_init(&p);
        memset(g_chr, 0, sizeof(g_chr));
        ppu_write_register(&p, 1, PPU_MASK_SHOW_BG | PPU_MASK_SHOW_BG_LEFT);
        for (uint16_t i = 0; i < (32u * 30u); ++i) {
            ppu_write_nametable(&p, (uint16_t)(0x2000u + i), 0x01);
        }
        for (uint16_t a = 0x10u; a <= 0x1Fu; ++a) {
            g_chr[a] = 0xFF;
        }
        ppu_write_palette(&p, 0x3F00, 0x0E);
        ppu_write_palette(&p, 0x3F03, 0x20);
        ChrReader chr = make_stub_reader();
        ppu_render_frame(&p, &chr);
        uint32_t white = nes_color_to_argb(0x20);
        CHECK(ppu_pixel(&p, 0, 0) == white, "render_frame: solid tile produces white pixel (0,0)");
        CHECK(ppu_pixel(&p, 255, 239) == white, "render_frame: solid tile produces white pixel (255,239)");
        uint32_t bg = nes_color_to_argb(0x0E);
        bool any_non_bg = false;
        const uint32_t* fb = ppu_framebuffer(&p);
        for (size_t i = 0; i < PPU_FRAMEBUFFER_SIZE; ++i) {
            if (fb[i] != bg) { any_non_bg = true; break; }
        }
        CHECK(any_non_bg, "render_frame: framebuffer has non-universal-bg pixels");
    }

    /* render_background with left mask blanks first 8 pixels. */
    {
        Ppu p;
        ppu_init(&p);
        memset(g_chr, 0, sizeof(g_chr));
        ppu_write_register(&p, 1, PPU_MASK_SHOW_BG);
        ppu_write_palette(&p, 0x3F00, 0x0E);
        for (uint16_t i = 0; i < (32u * 30u); ++i) {
            ppu_write_nametable(&p, (uint16_t)(0x2000u + i), 0x01);
        }
        for (uint16_t a = 0x10u; a <= 0x1Fu; ++a) {
            g_chr[a] = 0xFF;
        }
        ChrReader chr = make_stub_reader();
        ppu_render_background(&p, &chr);
        uint32_t bg = nes_color_to_argb(0x0E);
        CHECK(ppu_pixel(&p, 0, 0) == bg, "render_background: left pixel (0,0) is universal bg (left mask)");
        CHECK(ppu_pixel(&p, 8, 0) != bg, "render_background: pixel (8,0) is not universal bg");
    }

    /* clear_framebuffer sets all pixels. */
    {
        Ppu p;
        ppu_init(&p);
        ppu_clear_framebuffer(&p, 0xABCDEF12u);
        const uint32_t* fb = ppu_framebuffer(&p);
        bool all_set = true;
        for (size_t i = 0; i < PPU_FRAMEBUFFER_SIZE; ++i) {
            if (fb[i] != 0xABCDEF12u) { all_set = false; break; }
        }
        CHECK(all_set, "clear_framebuffer sets all pixels");
    }

    /* pixel() out of bounds returns universal bg. */
    {
        Ppu p;
        ppu_init(&p);
        ppu_write_palette(&p, 0x3F00, 0x21);
        uint32_t bg = nes_color_to_argb(0x21);
        CHECK_EQ_U32(ppu_pixel(&p, PPU_SCREEN_WIDTH, 0), bg, "pixel() OOB x returns universal bg");
        CHECK_EQ_U32(ppu_pixel(&p, 0, PPU_SCREEN_HEIGHT), bg, "pixel() OOB y returns universal bg");
    }

    /* step_rendered writes framebuffer on visible scanlines. */
    {
        Ppu p;
        ppu_init(&p);
        memset(g_chr, 0, sizeof(g_chr));
        ppu_write_register(&p, 1, PPU_MASK_SHOW_BG | PPU_MASK_SHOW_BG_LEFT);
        for (uint16_t i = 0; i < (32u * 30u); ++i) {
            ppu_write_nametable(&p, (uint16_t)(0x2000u + i), 0x01);
        }
        for (uint16_t a = 0x10u; a <= 0x1Fu; ++a) {
            g_chr[a] = 0xFF;
        }
        ppu_write_palette(&p, 0x3F00, 0x0E);
        ppu_write_palette(&p, 0x3F03, 0x20);
        ChrReader chr = make_stub_reader();
        for (int i = 0; i < 300; ++i) {
            ppu_step_rendered(&p, &chr);
        }
        CHECK(ppu_rendered_this_frame(&p), "step_rendered: rendered_this_frame set");
        uint32_t white = nes_color_to_argb(0x20);
        CHECK_EQ_U32(ppu_pixel(&p, 0, 0), white, "step_rendered: pixel (0,0) written (white)");
    }

    /* Sprites: set OAM sprite 0 with a tile, enable sprites, render, assert sprite pixel appears. */
    {
        Ppu p;
        ppu_init(&p);
        memset(g_chr, 0, sizeof(g_chr));
        ppu_write_register(&p, 1, PPU_MASK_SHOW_BG | PPU_MASK_SHOW_BG_LEFT |
                                  PPU_MASK_SHOW_SPRITES | PPU_MASK_SHOW_SPRITES_LEFT);
        ppu_write_palette(&p, 0x3F00, 0x0E);
        uint8_t* oam = ppu_oam_mut(&p);
        oam[0] = 0x00; oam[1] = 0x01; oam[2] = 0x00; oam[3] = 0x00;
        for (uint16_t a = 0x10u; a <= 0x1Fu; ++a) {
            g_chr[a] = 0xFF;
        }
        ppu_write_palette(&p, 0x3F13, 0x20);
        ChrReader chr = make_stub_reader();
        ppu_render_frame(&p, &chr);
        uint32_t white = nes_color_to_argb(0x20);
        uint32_t bg = nes_color_to_argb(0x0E);
        CHECK(ppu_pixel(&p, 0, 1) == white, "render_frame: sprite 0 pixel appears at (0,1)");
        CHECK(ppu_pixel(&p, 0, 0) == bg, "render_frame: pixel (0,0) is bg (sprite starts at row 1)");
        CHECK(!ppu_sprite_zero_hit(&p), "render_frame: no sprite 0 hit when bg is blank");
    }

    /* Sprite 0 hit triggers when sprite 0 overlaps an opaque bg pixel. */
    {
        Ppu p;
        ppu_init(&p);
        memset(g_chr, 0, sizeof(g_chr));
        ppu_write_register(&p, 1, PPU_MASK_SHOW_BG | PPU_MASK_SHOW_BG_LEFT |
                                  PPU_MASK_SHOW_SPRITES | PPU_MASK_SHOW_SPRITES_LEFT);
        for (uint16_t i = 0; i < (32u * 30u); ++i) {
            ppu_write_nametable(&p, (uint16_t)(0x2000u + i), 0x01);
        }
        for (uint16_t a = 0x10u; a <= 0x1Fu; ++a) {
            g_chr[a] = 0xFF;
        }
        ppu_write_palette(&p, 0x3F00, 0x0E);
        ppu_write_palette(&p, 0x3F03, 0x20);
        uint8_t* oam = ppu_oam_mut(&p);
        oam[0] = 0x00; oam[1] = 0x01; oam[2] = 0x00; oam[3] = 0x00;
        ppu_write_palette(&p, 0x3F13, 0x21);
        ChrReader chr = make_stub_reader();
        ppu_render_frame(&p, &chr);
        CHECK(ppu_sprite_zero_hit(&p), "render_frame: sprite 0 hit set when sprite overlaps opaque bg");
    }

    /* Nametable mirroring: horizontal / vertical / 4-screen. */
    {
        Ppu p;
        ppu_init(&p);
        ppu_set_mirroring(&p, MIRROR_HORIZONTAL);
        ppu_write_nametable(&p, 0x2000, 0x11);
        CHECK_EQ_U8(ppu_read_nametable(&p, 0x2400), 0x11, "horizontal mirroring: NT1 mirrors NT0");
        ppu_write_nametable(&p, 0x2800, 0x22);
        CHECK_EQ_U8(ppu_read_nametable(&p, 0x2C00), 0x22, "horizontal mirroring: NT3 mirrors NT2");

        ppu_set_mirroring(&p, MIRROR_VERTICAL);
        ppu_write_nametable(&p, 0x2000, 0x33);
        CHECK_EQ_U8(ppu_read_nametable(&p, 0x2800), 0x33, "vertical mirroring: NT2 mirrors NT0");

        ppu_set_mirroring(&p, MIRROR_FOUR_SCREEN);
        ppu_write_nametable(&p, 0x2000, 0xAA);
        ppu_write_nametable(&p, 0x2400, 0xBB);
        ppu_write_nametable(&p, 0x2800, 0xCC);
        ppu_write_nametable(&p, 0x2C00, 0xDD);
        CHECK_EQ_U8(ppu_read_nametable(&p, 0x2000), 0xAA, "4-screen: NT0 unique");
        CHECK_EQ_U8(ppu_read_nametable(&p, 0x2400), 0xBB, "4-screen: NT1 unique");
        CHECK_EQ_U8(ppu_read_nametable(&p, 0x2800), 0xCC, "4-screen: NT2 unique");
        CHECK_EQ_U8(ppu_read_nametable(&p, 0x2C00), 0xDD, "4-screen: NT3 unique");
    }

    /* Palette mirroring: $3F10 mirrors $3F00, $3F20 mirrors $3F00. */
    {
        Ppu p;
        ppu_init(&p);
        ppu_write_palette(&p, 0x3F00, 0x0F);
        CHECK_EQ_U8(ppu_read_palette(&p, 0x3F10), 0x0F, "palette: $3F10 mirrors $3F00");
        CHECK_EQ_U8(ppu_read_palette(&p, 0x3F20), 0x0F, "palette: $3F20 mirrors $3F00");
        ppu_write_palette(&p, 0x3F01, 0x11);
        ppu_write_palette(&p, 0x3F11, 0x22);
        CHECK_EQ_U8(ppu_read_palette(&p, 0x3F01), 0x11, "palette: $3F01 distinct");
        CHECK_EQ_U8(ppu_read_palette(&p, 0x3F11), 0x22, "palette: $3F11 distinct from $3F01");
    }

    /* NMI-during-VBlank quirk. */
    {
        Ppu p;
        ppu_init(&p);
        ppu_write_register(&p, 0, 0x00);
        ppu_set_vblank(&p, true);
        CHECK(!ppu_take_nmi_request(&p), "NMI quirk: no request before enable");
        ppu_write_register(&p, 0, 0x80);
        CHECK(ppu_take_nmi_request(&p), "NMI quirk: request latched on rising edge during VBlank");
        ppu_write_register(&p, 0, 0x80);
        CHECK(!ppu_take_nmi_request(&p), "NMI quirk: re-writing bit7=1 does not re-trigger");
        ppu_write_register(&p, 0, 0x00);
        CHECK(!ppu_take_nmi_request(&p), "NMI quirk: no request after disable");
        ppu_write_register(&p, 0, 0x80);
        CHECK(ppu_take_nmi_request(&p), "NMI quirk: rising edge after disable re-triggers");
    }

    /* nmi_retrigger debug flag. */
    {
        Ppu p;
        ppu_init(&p);
        ppu_set_nmi_retrigger(&p, true);
        ppu_write_register(&p, 0, 0x80);
        CHECK(!ppu_take_nmi_request(&p), "nmi_retrigger: no request outside VBlank");
        ppu_set_vblank(&p, true);
        ppu_write_register(&p, 0, 0x80);
        CHECK(ppu_take_nmi_request(&p), "nmi_retrigger: re-writing bit7=1 during VBlank triggers");
        ppu_write_register(&p, 0, 0x80);
        CHECK(ppu_take_nmi_request(&p), "nmi_retrigger: every write with bit7 during VBlank triggers");
    }

    /* slant_corruption debug flag. */
    {
        Ppu p;
        ppu_init(&p);
        ppu_write_register(&p, 1, PPU_MASK_SHOW_BG);
        ppu_set_slant_corruption(&p, true);
        for (int i = 0; i < 8; ++i) {
            ppu_step(&p);
        }
        CHECK_EQ_U8(ppu_fine_x(&p), 1, "slant_corruption: fine_x incremented at cycle 8");
        CHECK_EQ_U16(ppu_vram_addr(&p) & PPU_COARSE_X_MASK, 0u, "slant_corruption: coarse X unchanged");
    }

    /* Open bus: write-only register read returns open bus. */
    {
        Ppu p;
        ppu_init(&p);
        ppu_write_register(&p, 0, 0xA5);
        CHECK_EQ_U8(ppu_read_register(&p, 0), 0xA5, "open bus: write-only reg read returns open bus");
        ppu_set_open_bus(&p, 0xAB);
        CHECK_EQ_U8(ppu_open_bus(&p), 0xAB, "set_open_bus / open_bus round-trip");
        CHECK_EQ_U8(ppu_read_register(&p, 0), 0xAB, "open bus: reflects set_open_bus");
    }

    /* advance_vram_addr wraps at 0x3FFF. */
    {
        Ppu p;
        ppu_init(&p);
        ppu_write_register(&p, 6, 0x3F);
        ppu_write_register(&p, 6, 0xFF);
        CHECK_EQ_U16(ppu_vram_addr(&p), 0x3FFFu, "advance: v == 0x3FFF");
        ppu_advance_vram_addr(&p);
        CHECK_EQ_U16(ppu_vram_addr(&p), 0x0000u, "advance: wraps to 0 with increment 1");
    }

    /* screen_size. */
    {
        size_t w, h;
        ppu_screen_size(&w, &h);
        CHECK(w == PPU_SCREEN_WIDTH && h == PPU_SCREEN_HEIGHT, "ppu_screen_size == 256x240");
    }
}

/* ---- (h) edge cases flagged by review -------------------------------- */

static void test_edge_cases(void) {
    printf("\n== (h) edge cases (PPUDATA writes, CHR-RAM, sprite overflow, "
           "8x16, v-copy, PAL palette, inaccurate palette, v-scroll wrap) ==\n");

    /* PPUDATA write to nametable via the bus ($2007 with v in $2000-$3EFF). */
    {
        Bus bus;
        bus_init(&bus);
        /* PPUADDR = $2000 (nametable 0, byte 0). */
        bus_write(&bus, 0x2006, 0x20);
        bus_write(&bus, 0x2006, 0x00);
        bus_write(&bus, 0x2007, 0x77);
        Ppu* p = bus_ppu(&bus);
        CHECK_EQ_U8(ppu_read_nametable(p, 0x2000), 0x77, "PPUDATA write to nametable via bus");
        /* v should have advanced by 1 (default increment). */
        CHECK_EQ_U16(ppu_vram_addr(p), 0x2001u, "PPUDATA nametable write advances v by 1");
    }

    /* PPUDATA write to CHR-RAM via the bus ($2007 with v < $2000, chr_is_ram cart). */
    {
        /* Build a minimal NROM-128 ROM with 0 CHR-ROM banks -> CHR-RAM. */
        uint8_t rom[16 + 16384];
        memset(rom, 0, sizeof(rom));
        rom[0] = 'N'; rom[1] = 'E'; rom[2] = 'S'; rom[3] = 0x1A;
        rom[4] = 1;   /* 1 PRG bank (16 KB) */
        rom[5] = 0;   /* 0 CHR banks -> CHR-RAM */
        rom[6] = 0;   /* mapper 0, horizontal mirroring */
        rom[7] = 0;
        Cartridge cart;
        CHECK(cartridge_from_bytes(rom, sizeof(rom), &cart) == 0, "CHR-RAM cart parse succeeds");
        CHECK(cartridge_chr_is_ram(&cart), "CHR-RAM cart: chr_is_ram == true");

        Bus bus;
        bus_init_with_cartridge(&bus, &cart);
        /* PPUADDR = $0000 (CHR pattern table 0, byte 0). */
        bus_write(&bus, 0x2006, 0x00);
        bus_write(&bus, 0x2006, 0x00);
        bus_write(&bus, 0x2007, 0xAB);
        CHECK_EQ_U8(cartridge_read_chr(&cart, 0x0000), 0xAB, "PPUDATA write to CHR-RAM via bus");
        /* CHR-ROM carts would ignore the write; CHR-RAM must persist it. */
        bus_write(&bus, 0x2006, 0x10);
        bus_write(&bus, 0x2006, 0x00);
        bus_write(&bus, 0x2007, 0xCD);
        CHECK_EQ_U8(cartridge_read_chr(&cart, 0x1000), 0xCD, "PPUDATA write to CHR-RAM pattern table 1");
    }

    /* Sprite overflow: >8 in-range sprites sets the overflow flag. */
    {
        Ppu p;
        ppu_init(&p);
        memset(g_chr, 0, sizeof(g_chr));
        ppu_write_register(&p, 1, PPU_MASK_SHOW_SPRITES | PPU_MASK_SHOW_SPRITES_LEFT);
        ppu_write_palette(&p, 0x3F00, 0x0E);
        ppu_write_palette(&p, 0x3F13, 0x20);
        uint8_t* oam = ppu_oam_mut(&p);
        /* Place 9 sprites all on scanline 0 (y=0xFF would hide; use y=0x00 -> top=1, but
         * scanline 0 is not in range for top=1. Use y such that scanline 10 is in range:
         * top = y+1; want scanline 10 in [top, top+8). y=9 -> top=10, rows 10..17. */
        for (int i = 0; i < 9; ++i) {
            uint16_t o = (uint16_t)(i * 4);
            oam[o + 0] = 0x09;     /* y */
            oam[o + 1] = 0x01;     /* tile */
            oam[o + 2] = 0x00;     /* attr */
            oam[o + 3] = (uint8_t)(i * 16); /* x (spread so they don't all overlap) */
        }
        for (uint16_t a = 0x10u; a <= 0x1Fu; ++a) {
            g_chr[a] = 0xFF;
        }
        ChrReader chr = make_stub_reader();
        ppu_render_frame(&p, &chr);
        CHECK(ppu_sprite_overflow(&p), "sprite overflow: 9 in-range sprites sets overflow flag");
    }

    /* 8x16 sprite mode: sprite uses tile bit 0 for pattern table, tile&0xFE base. */
    {
        Ppu p;
        ppu_init(&p);
        memset(g_chr, 0, sizeof(g_chr));
        /* PPUCTRL: sprite size 16 (bit 5). */
        ppu_write_register(&p, 0, PPU_CTRL_SPRITE_SIZE_16);
        ppu_write_register(&p, 1, PPU_MASK_SHOW_SPRITES | PPU_MASK_SHOW_SPRITES_LEFT);
        ppu_write_palette(&p, 0x3F00, 0x0E);
        ppu_write_palette(&p, 0x3F11, 0x21);
        uint8_t* oam = ppu_oam_mut(&p);
        /* Sprite 0: y=0 (top=1), tile=0x01 (odd -> pattern table $1000, base tile 0),
         * x=0. Row 0 of the sprite -> pattern $1000 | (0<<4) | 0 = $1000. */
        oam[0] = 0x00; oam[1] = 0x01; oam[2] = 0x00; oam[3] = 0x00;
        /* Fill only plane0 of pattern $1000 row 0 (addr 0x1000..0x1007) with 0xFF so
         * pattern == 1 (plane1 stays 0). color_addr -> $3F11 -> palette 0x21. */
        for (uint16_t a = 0x1000u; a <= 0x1007u; ++a) {
            g_chr[a & 0x1FFFu] = 0xFF;
        }
        ChrReader chr = make_stub_reader();
        ppu_render_frame(&p, &chr);
        uint32_t color = nes_color_to_argb(0x21);
        /* Sprite top = y+1 = 1, so pixel (0,1) should be the sprite color. */
        CHECK_EQ_U32(ppu_pixel(&p, 0, 1), color, "8x16 sprite: odd tile uses pattern table $1000");
    }

    /* Vertical t->v copy at prerender cycles 280-304.
     *
     * Set t's vertical bits via PPUADDR (nt_v) + PPUSCROLL (coarse Y, fine Y),
     * enable rendering, step to prerender cycle 305 (just past the 280-304
     * v-copy window), then assert v's vertical bits == t's. PPUADDR must run
     * BEFORE PPUSCROLL so the fine Y bits aren't clobbered. PPUSCROLL 2nd write
     * value = (coarseY<<3)|fineY = (5<<3)|2 = 0x2A. PPUADDR 1st write 0x28 sets
     * t bit 11 (nt_v). */
    {
        Ppu p;
        ppu_init(&p);
        ppu_write_register(&p, 6, 0x28);   /* PPUADDR 1st: t hi = 0x2800 (nt_v set), w=true */
        ppu_write_register(&p, 6, 0x00);   /* PPUADDR 2nd: t lo = 0, v = 0x2800, w=false */
        ppu_write_register(&p, 5, 0x00);   /* PPUSCROLL 1st: coarse X=0, fine X=0, w=true */
        ppu_write_register(&p, 5, 0x2A);   /* PPUSCROLL 2nd: coarse Y=5, fine Y=2, w=false */
        ppu_write_register(&p, 1, PPU_MASK_SHOW_BG);
        /* Step to prerender scanline 261, cycle 305 = 261*341 + 305 = 89306. */
        for (uint32_t i = 0; i < 89306u; ++i) {
            ppu_step(&p);
        }
        CHECK_EQ_U16(ppu_scanline(&p), 261u, "v-copy: at prerender scanline 261");
        CHECK_EQ_U16(ppu_cycle(&p), 305u, "v-copy: at cycle 305 (past 280-304 window)");
        uint16_t v = ppu_vram_addr(&p);
        CHECK_EQ_U16((v >> 5) & 0x1Fu, 5u, "v-copy: v coarse Y == 5 (from t)");
        CHECK_EQ_U16((v >> 12) & 0x07u, 2u, "v-copy: v fine Y == 2 (from t)");
        CHECK((v & PPU_NT_V_BIT) != 0, "v-copy: v nt_v bit set (from t)");
    }

    /* PAL palette color conversion uses PAL_PALETTE. */
    {
        /* PAL palette index 0x00 is [0x84,0x84,0x84] -> ARGB 0xFF848484. */
        uint32_t c = nes_color_to_argb_for(0x00, REGION_PAL);
        CHECK_EQ_U32(c, 0xFF848484u, "PAL palette: index 0x00 == 0xFF848484");
        /* PAL index 0x20 is [0xFF,0xFF,0xFF] -> 0xFFFFFFFF. */
        CHECK_EQ_U32(nes_color_to_argb_for(0x20, REGION_PAL), 0xFFFFFFFFu, "PAL palette: index 0x20 == white");
        /* Dendy uses NTSC palette (is_pal_palette false). */
        CHECK_EQ_U32(nes_color_to_argb_for(0x00, REGION_DENDY), 0xFF7C7C7Cu, "Dendy palette: index 0x00 == NTSC grey");
    }

    /* inaccurate_palette debug toggle uses NES_PALETTE_INACCURATE for NTSC. */
    {
        Ppu p;
        ppu_init(&p);
        /* Inaccurate NTSC palette index 0x00 is [0x84,0x84,0x84] -> 0xFF848484
         * (vs accurate 0xFF7C7C7C). Use render_frame with bg disabled so the
         * universal bg color is computed via color_to_argb with the toggle. */
        ppu_set_inaccurate_palette(&p, true);
        ppu_write_palette(&p, 0x3F00, 0x00);
        ChrReader chr = make_stub_reader();
        ppu_render_frame(&p, &chr);
        /* universal_bg_argb -> color_to_argb(palette[0]) with inaccurate palette. */
        CHECK_EQ_U32(ppu_pixel(&p, 0, 0), 0xFF848484u, "inaccurate_palette: index 0x00 uses inaccurate grey");
    }

    /* Vertical scroll wrap at coarse_y==29 -> nt_v toggle.
     *
     * Set t's vertical bits (coarse Y=29, fine Y=7) via PPUADDR (nt_v=0) +
     * PPUSCROLL, use the prerender v-copy (cycles 280-304) to load them into v,
     * then step to cycle 256 of scanline 0 to fire increment_v_scroll and
     * observe the wrap (fine Y 7->0, coarse Y 29->0, nt_v toggles).
     *
     * PPUSCROLL 2nd write value = (coarseY<<3)|fineY = (29<<3)|7 = 0xEF. PPUADDR
     * must run BEFORE PPUSCROLL so fine Y bits 13-14 aren't clobbered. */
    {
        Ppu p;
        ppu_init(&p);
        ppu_write_register(&p, 6, 0x00);   /* PPUADDR 1st: t hi = 0 (nt_v clear), w=true */
        ppu_write_register(&p, 6, 0x00);   /* PPUADDR 2nd: t lo = 0, v = 0, w=false */
        ppu_write_register(&p, 5, 0x00);   /* PPUSCROLL 1st: coarse X=0, fine X=0, w=true */
        ppu_write_register(&p, 5, 0xEF);   /* PPUSCROLL 2nd: coarse Y=29, fine Y=7, w=false */
        ppu_write_register(&p, 1, PPU_MASK_SHOW_BG);

        /* Step to prerender scanline 261, cycle 305 (v-copy 280-304 has fired,
         * loading t's vertical bits into v). 261*341 + 305 = 89306. */
        for (uint32_t i = 0; i < 89306u; ++i) {
            ppu_step(&p);
        }
        uint16_t v_before = ppu_vram_addr(&p);
        CHECK_EQ_U16((v_before >> 5) & 0x1Fu, 29u, "v-scroll wrap: v coarse Y == 29 before increment");
        CHECK_EQ_U16((v_before >> 12) & 0x07u, 7u, "v-scroll wrap: v fine Y == 7 before increment");
        CHECK((v_before & PPU_NT_V_BIT) == 0, "v-scroll wrap: nt_v clear before wrap");

        /* Step to scanline 0, cycle 256 (fires increment_v_scroll). */
        while (!(ppu_scanline(&p) == 0 && ppu_cycle(&p) == 256)) {
            ppu_step(&p);
        }
        uint16_t v_after = ppu_vram_addr(&p);
        /* coarse Y 29 + fine Y 7 wrap: fine Y->0, coarse Y 29->0, nt_v toggles. */
        CHECK_EQ_U16((v_after >> 5) & 0x1Fu, 0u, "v-scroll wrap: coarse Y wraps 29 -> 0");
        CHECK_EQ_U16((v_after >> 12) & 0x07u, 0u, "v-scroll wrap: fine Y wraps 7 -> 0");
        CHECK((v_after & PPU_NT_V_BIT) != 0, "v-scroll wrap: nt_v toggles on coarse Y 29 wrap");
    }
}

/* ---- main ------------------------------------------------------------ */

int main(void) {
    printf("=== NES C core M4.2 PPU acceptance test ===\n");
    test_init_defaults();
    test_registers();
    test_flags();
    test_step();
    test_region_timing();
    test_palette();
    test_render();
    test_edge_cases();

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
