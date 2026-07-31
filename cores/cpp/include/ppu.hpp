/*
 * ppu.h - Picture Processing Unit: registers, VRAM, OAM, palette, scroll,
 *         scanline/cycle stepper.
 *
 * Port of src/ppu/mod.rs to C (M4.2). The Ppu struct mirrors the Rust struct
 * field-for-field. The PPU owns nametable VRAM (2 KB or 4 KB depending on
 * mirroring), OAM (256 bytes), palette RAM (32 bytes), and the 256x240 ARGB
 * framebuffer. CHR pattern data ($0000-$1FFF) is owned by the cartridge and
 * accessed through the bus via the ChrReader callback - the PPU never touches
 * CHR directly.
 *
 * See: https://www.nesdev.org/wiki/PPU_registers
 * See: https://www.nesdev.org/wiki/PPU_rendering
 * See: https://www.nesdev.org/wiki/PPU_scrolling
 */
#ifndef NES_CORE_C_PPU_H
#define NES_CORE_C_PPU_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>
#include "cartridge.hpp"   /* Mirroring */
#include "region.hpp"      /* Region */
#include "ppu_render.hpp"  /* RenderPipeline, ChrReader */

#ifdef __cplusplus
extern "C" {
#endif

/* ---- Constants (mod.rs) ---------------------------------------------- */

/* VRAM size for 4-screen mirroring (4 KB). (mod.rs VRAM_SIZE_4K.) */
#define PPU_VRAM_SIZE_4K 0x1000u
/* VRAM size for horizontal/vertical/single-screen mirroring (2 KB).
 * (mod.rs VRAM_SIZE_2K.) */
#define PPU_VRAM_SIZE_2K 0x0800u
/* Size of OAM (64 sprites x 4 bytes = 256 bytes). (mod.rs OAM_SIZE.) */
#define PPU_OAM_SIZE 256u
/* Size of palette RAM (32 bytes; $3F20-$3FFF mirrors down to 32 bytes).
 * (mod.rs PALETTE_SIZE.) */
#define PPU_PALETTE_SIZE 32u

/* Number of scanlines in an NTSC frame (262). (mod.rs SCANLINES_PER_FRAME.) */
#define PPU_SCANLINES_PER_FRAME 262u
/* Number of PPU cycles per scanline (341). (mod.rs CYCLES_PER_SCANLINE.) */
#define PPU_CYCLES_PER_SCANLINE 341u
/* Scanline on which VBlank begins (NMI asserted at cycle 1).
 * (mod.rs SCANLINE_VBLANK_START.) */
#define PPU_SCANLINE_VBLANK_START 241u
/* Prerender scanline (NTSC). (mod.rs SCANLINE_PRERENDER.) */
#define PPU_SCANLINE_PRERENDER 261u

/* ---- PPUCTRL bit masks (mod.rs) -------------------------------------- */
#define PPU_CTRL_NMI                 0x80u  /* bit 7: generate NMI on VBlank. */
#define PPU_CTRL_INCREMENT_32        0x04u  /* bit 2: VRAM addr increment 32. */
#define PPU_CTRL_SPRITE_PATTERN_1000 0x08u  /* bit 3: sprite table $1000. */
#define PPU_CTRL_BG_PATTERN_1000     0x10u  /* bit 4: bg table $1000. */
#define PPU_CTRL_SPRITE_SIZE_16      0x20u  /* bit 5: 8x16 sprite size. */
#define PPU_CTRL_BASE_NT_MASK        0x03u  /* bits 0-1: base nametable. */

/* ---- PPUMASK bit masks (mod.rs) -------------------------------------- */
#define PPU_MASK_SHOW_BG_LEFT        0x02u  /* bit 1: show leftmost 8 bg px. */
#define PPU_MASK_SHOW_SPRITES_LEFT   0x04u  /* bit 2: show leftmost 8 spr px. */
#define PPU_MASK_SHOW_BG             0x08u  /* bit 3: show background. */
#define PPU_MASK_SHOW_SPRITES        0x10u  /* bit 4: show sprites. */

/* ---- PPUSTATUS bit masks (mod.rs) ------------------------------------ */
#define PPU_STATUS_VBLANK            0x80u  /* bit 7: VBlank. */
#define PPU_STATUS_SPRITE_ZERO       0x40u  /* bit 6: sprite 0 hit. */
#define PPU_STATUS_OVERFLOW          0x20u  /* bit 5: sprite overflow. */
#define PPU_STATUS_FLAG_MASK         0xE0u  /* bits 7-5: status flags. */
#define PPU_STATUS_OPEN_BUS_MASK     0x1Fu  /* bits 4-0: open bus on read. */

/* ---- v / t register bit masks (mod.rs) ------------------------------- */
#define PPU_NT_SELECT_MASK  0x0C00u  /* bits 10-11: nametable select. */
#define PPU_COARSE_X_MASK   0x001Fu  /* bits 0-4: coarse X. */
#define PPU_COARSE_Y_MASK   0x03E0u  /* bits 5-9: coarse Y. */
#define PPU_FINE_Y_MASK     0x7000u  /* bits 12-14: fine Y. */
#define PPU_NT_H_BIT        0x0400u  /* bit 10: nametable horizontal wrap. */
#define PPU_NT_V_BIT        0x0800u  /* bit 11: nametable vertical wrap. */

/* ---- The PPU struct (mod.rs `struct Ppu`) ---------------------------- */

/* The Picture Processing Unit. Owns nametable VRAM, OAM, palette RAM, and the
 * 256x240 ARGB framebuffer. CHR pattern data is owned by the cartridge and
 * accessed through the bus. (mod.rs `struct Ppu`.) */
typedef struct Ppu {
    /* ---- writable registers (write-only; reads return open bus) ---- */
    uint8_t ppuctrl;
    uint8_t ppumask;
    uint8_t oamaddr;

    /* ---- readable status register ---- */
    uint8_t ppustatus;

    /* ---- VRAM address registers (PPUSCROLL / PPUADDR / PPUDATA) ---- */
    uint16_t v;            /* current VRAM address (14-bit effective). */
    uint16_t t;            /* temporary VRAM address, copied to v. */
    uint8_t  fine_x;       /* fine X scroll (3 bits). */
    bool     w;            /* shared write latch for PPUSCROLL/PPUADDR. */

    /* ---- PPUDATA buffered read ---- */
    uint8_t ppudata_buffer; /* buffered data for PPUDATA reads. */

    /* Open-bus latch: last byte written to any PPU register. */
    uint8_t open_bus;

    /* ---- memory ---- */
    /* Nametable VRAM (2 KB or 4 KB depending on mirroring). The array is
     * always 4 KB to cover 4-screen mirroring; for H/V/single-screen only the
     * first vram_size bytes are used. */
    uint8_t  vram[PPU_VRAM_SIZE_4K];
    uint32_t vram_size;    /* 0x800 or 0x1000. */
    uint8_t  oam[PPU_OAM_SIZE];
    uint8_t  palette[PPU_PALETTE_SIZE];

    /* ---- framebuffer (256x240 ARGB) ---- */
    uint32_t framebuffer[PPU_FRAMEBUFFER_SIZE];

    /* Per-pixel bg pattern (0-3) for current scanline (sprite priority). */
    uint8_t bg_pattern[PPU_SCREEN_WIDTH];

    /* ---- timing ---- */
    uint16_t scanline;     /* 0..=261 (261 = prerender). */
    uint16_t cycle;        /* 0..=340. */
    bool nmi_request;      /* latched NMI request. */

    /* ---- configuration ---- */
    Mirroring mirroring;
    Region region;

    /* ---- M25: per-pixel (cycle-accurate) rendering pipeline ---- */
    RenderPipeline render;
} Ppu;

/* ---- Construction / configuration (mod.rs `new` / `set_*`) ----------- */

/* Construct a PPU in power-on state (zeroed, horizontal mirroring, NTSC).
 * (mod.rs `Ppu::new`.) */
void ppu_init(Ppu* p);

/* Set mirroring mode; resizes VRAM if needed (4 KB for four-screen).
 * (mod.rs `Ppu::set_mirroring`.) */
void ppu_set_mirroring(Ppu* p, Mirroring m);

/* Current TV system / region. (mod.rs `Ppu::region`.) */
Region ppu_region(const Ppu* p);

/* Set TV system / region (updates scanline count and prerender; clamps the
 * current scanline into the new range). (mod.rs `Ppu::set_region`.) */
void ppu_set_region(Ppu* p, Region r);

/* Enable/disable the fine-X scroll corruption bug (debug only).
 * (mod.rs `Ppu::set_slant_corruption`.) */
void ppu_set_slant_corruption(Ppu* p, bool enabled);

/* Enable/disable the inaccurate NES palette (debug only).
 * (mod.rs `Ppu::set_inaccurate_palette`.) */
void ppu_set_inaccurate_palette(Ppu* p, bool enabled);

/* Enable/disable the NMI retrigger bug (debug only).
 * (mod.rs `Ppu::set_nmi_retrigger`.) */
void ppu_set_nmi_retrigger(Ppu* p, bool enabled);

/* ---- Register reads / writes (mod.rs `read_register` / `write_register`) */

/* Read PPU register by de-mirrored index (0..=7). Index 7 returns the buffered
 * PPUDATA value only; the BUS handles the full CHR-routed semantics for reg 7.
 * (mod.rs `Ppu::read_register`.) */
uint8_t ppu_read_register(Ppu* p, uint16_t reg);

/* Write a PPU register by de-mirrored index (0..=7). All writes update the
 * open-bus latch. PPUDATA (index 7) is a no-op here apart from open-bus; the
 * bus handles CHR routing via ppu_write_ppudata. (mod.rs `Ppu::write_register`.) */
void ppu_write_register(Ppu* p, uint16_t reg, uint8_t value);

/* ---- PPUDATA accessors (used by the bus for CHR-routed PPUDATA) ------ */

/* Current VRAM address (the v register). (mod.rs `Ppu::vram_addr`.) */
uint16_t ppu_vram_addr(const Ppu* p);

/* VRAM address increment per PPUDATA access: 1 or 32, selected by PPUCTRL
 * bit 2. (mod.rs `Ppu::vram_increment`.) */
uint16_t ppu_vram_increment(const Ppu* p);

/* Advance v by the current increment, wrapping within the 14-bit PPU address
 * space ($0000-$3FFF). (mod.rs `Ppu::advance_vram_addr`.) */
void ppu_advance_vram_addr(Ppu* p);

/* Current PPUDATA read buffer. (mod.rs `Ppu::ppudata_buffer`.) */
uint8_t ppu_ppudata_buffer(const Ppu* p);

/* Set the PPUDATA read buffer (used by the bus after fetching the real value
 * from CHR or nametable memory). (mod.rs `Ppu::set_ppudata_buffer`.) */
void ppu_set_ppudata_buffer(Ppu* p, uint8_t value);

/* The PPU open-bus latch (last byte written to any PPU register).
 * (mod.rs `Ppu::open_bus`.) */
uint8_t ppu_open_bus(const Ppu* p);

/* Set the open-bus latch. Used by the bus after PPUDATA reads and OAMDMA
 * writes. (mod.rs `Ppu::set_open_bus`.) */
void ppu_set_open_bus(Ppu* p, uint8_t value);

/* ---- Nametable / palette memory access (mod.rs) --------------------- */

/* Read a nametable byte at addr (in $2000-$3EFF). Handles the $3000-$3EFF
 * mirror and nametable mirroring. (mod.rs `Ppu::read_nametable`.) */
uint8_t ppu_read_nametable(const Ppu* p, uint16_t addr);

/* Write a nametable byte at addr (in $2000-$3EFF). (mod.rs `Ppu::write_nametable`.) */
void ppu_write_nametable(Ppu* p, uint16_t addr, uint8_t value);

/* Read a palette byte at addr (in $3F00-$3FFF). Handles palette internal
 * mirroring. (mod.rs `Ppu::read_palette`.) */
uint8_t ppu_read_palette(const Ppu* p, uint16_t addr);

/* Write a palette byte at addr (in $3F00-$3FFF). (mod.rs `Ppu::write_palette`.) */
void ppu_write_palette(Ppu* p, uint16_t addr, uint8_t value);

/* ---- OAM DMA (mod.rs `oam_dma`) ------------------------------------- */

/* Perform a 256-byte OAM DMA: copy data into OAM starting at OAMADDR=0.
 * (mod.rs `Ppu::oam_dma`.) See: https://www.nesdev.org/wiki/PPU_registers#OAMDMA */
void ppu_oam_dma(Ppu* p, const uint8_t data[256]);

/* ---- VBlank / NMI / sprite flag control (mod.rs) -------------------- */

/* Set or clear the VBlank flag (bit 7 of PPUSTATUS). (mod.rs `Ppu::set_vblank`.) */
void ppu_set_vblank(Ppu* p, bool on);

/* Set the sprite 0 hit flag (bit 6 of PPUSTATUS). (mod.rs `Ppu::set_sprite_zero_hit`.) */
void ppu_set_sprite_zero_hit(Ppu* p, bool on);

/* Set the sprite overflow flag (bit 5 of PPUSTATUS). (mod.rs `Ppu::set_sprite_overflow`.) */
void ppu_set_sprite_overflow(Ppu* p, bool on);

/* True if VBlank NMI is enabled (PPUCTRL bit 7 set). (mod.rs `Ppu::nmi_enabled`.) */
bool ppu_nmi_enabled(const Ppu* p);

/* True if the VBlank flag is currently set. (mod.rs `Ppu::in_vblank`.) */
bool ppu_in_vblank(const Ppu* p);

/* True if the sprite 0 hit flag is set. (mod.rs `Ppu::sprite_zero_hit`.) */
bool ppu_sprite_zero_hit(const Ppu* p);

/* True if the sprite overflow flag is set. (mod.rs `Ppu::sprite_overflow`.) */
bool ppu_sprite_overflow(const Ppu* p);

/* ---- Scanline / cycle stepper (mod.rs `step` / `step_rendered`) ----- */

/* Current scanline within the frame (0..=261; 261 = prerender).
 * (mod.rs `Ppu::scanline`.) */
uint16_t ppu_scanline(const Ppu* p);

/* Current PPU cycle within the scanline (0..=340). (mod.rs `Ppu::cycle`.) */
uint16_t ppu_cycle(const Ppu* p);

/* Consume and return the pending NMI request. (mod.rs `Ppu::take_nmi_request`.) */
bool ppu_take_nmi_request(Ppu* p);

/* Advance the PPU by one cycle, returning true if an NMI should be raised
 * this cycle. (mod.rs `Ppu::step`.) See: https://www.nesdev.org/wiki/PPU_rendering#Timing */
bool ppu_step(Ppu* p);

/* Advance the PPU by one cycle AND perform per-pixel (cycle-accurate)
 * rendering, returning true if an NMI should be raised this cycle. The
 * ChrReader supplies pattern-table bytes from CHR (owned by the cartridge).
 * (mod.rs `Ppu::step_rendered`.) */
bool ppu_step_rendered(Ppu* p, ChrReader* chr_read);

/* ---- Test / debug accessors (mod.rs) -------------------------------- */

/* Current PPUCTRL value. (mod.rs `Ppu::ppuctrl`.) */
uint8_t ppu_ppuctrl(const Ppu* p);

/* Current PPUMASK value. (mod.rs `Ppu::ppumask`.) */
uint8_t ppu_ppumask(const Ppu* p);

/* Whether the PPU is currently rendering (background or sprites enabled).
 * (mod.rs `Ppu::is_rendering`.) */
bool ppu_is_rendering(const Ppu* p);

/* Current PPUSTATUS byte (raw, without open-bus fill). (mod.rs `Ppu::ppustatus`.) */
uint8_t ppu_ppustatus(const Ppu* p);

/* Current OAMADDR value. (mod.rs `Ppu::oamaddr`.) */
uint8_t ppu_oamaddr(const Ppu* p);

/* Temporary VRAM address (t register). (mod.rs `Ppu::temp_vram_addr`.) */
uint16_t ppu_temp_vram_addr(const Ppu* p);

/* Fine X scroll (3 bits). (mod.rs `Ppu::fine_x`.) */
uint8_t ppu_fine_x(const Ppu* p);

/* Shared write latch (w). (mod.rs `Ppu::write_latch`.) */
bool ppu_write_latch(const Ppu* p);

/* Borrow the OAM array. (mod.rs `Ppu::oam`.) */
const uint8_t* ppu_oam(const Ppu* p);

/* Mutably borrow the OAM array. (mod.rs `Ppu::oam_mut`.) */
uint8_t* ppu_oam_mut(Ppu* p);

/* Borrow the palette array. (mod.rs `Ppu::palette`.) */
const uint8_t* ppu_palette(const Ppu* p);

/* Borrow the nametable VRAM slice. (mod.rs `Ppu::vram`.) */
const uint8_t* ppu_vram(const Ppu* p);

/* Current mirroring mode. (mod.rs `Ppu::mirroring`.) */
Mirroring ppu_mirroring(const Ppu* p);

/* Borrow the output framebuffer (256x240 ARGB). (mod.rs `Ppu::framebuffer`.) */
const uint32_t* ppu_framebuffer(const Ppu* p);

/* Mutably borrow the output framebuffer. (mod.rs `Ppu::framebuffer_mut`.) */
uint32_t* ppu_framebuffer_mut(Ppu* p);

/* Borrow the per-scanline background pattern buffer. (mod.rs `Ppu::bg_pattern`.) */
const uint8_t* ppu_bg_pattern(const Ppu* p);

/* Whether the per-pixel renderer produced output during the current frame.
 * (mod.rs `Ppu::rendered_this_frame`.) */
bool ppu_rendered_this_frame(const Ppu* p);

/* Reset the rendered_this_frame flag at the start of a new frame.
 * (mod.rs `Ppu::reset_rendered_flag`.) */
void ppu_reset_rendered_flag(Ppu* p);

/* Screen dimensions (width, height) in pixels. (mod.rs `Ppu::screen_size`.) */
void ppu_screen_size(size_t* width, size_t* height);

#ifdef __cplusplus
}
#endif
#endif /* NES_CORE_C_PPU_H */
