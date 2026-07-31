/*
 * ppu.c - Picture Processing Unit: registers, VRAM, OAM, palette, scroll,
 *         scanline/cycle stepper.
 *
 * Port of src/ppu/mod.rs to C (M4.2). Mirrors the Rust source field-for-field
 * and method-for-method. The PPU owns nametable VRAM, OAM, palette RAM, and
 * the framebuffer; CHR pattern data is owned by the cartridge and accessed
 * through the bus via the ChrReader callback (used by ppu_step_rendered and
 * the render pipeline in ppu_render.c).
 *
 * See: https://www.nesdev.org/wiki/PPU_registers
 * See: https://www.nesdev.org/wiki/PPU_scrolling
 * See: https://www.nesdev.org/wiki/PPU_rendering#Timing
 */
#include "ppu.h"
#include "ppu_render.h"
#include <string.h>

/* ---- Internal timing constants (mod.rs) ------------------------------ */
/* PPU cycle within the scanline at which VBlank is asserted / cleared. */
#define VBLANK_NMI_CYCLE       1u
/* PPU cycle at which the vertical scroll component is incremented (fine Y). */
#define VERT_SCROLL_INC_CYCLE  256u
/* PPU cycle at which the horizontal t->v copy happens (end of visible line). */
#define H_COPY_CYCLE           257u
/* First prerender cycle at which the vertical t->v copy happens. */
#define V_COPY_CYCLE_START     280u
/* Last prerender cycle at which the vertical t->v copy happens. */
#define V_COPY_CYCLE_END       304u
/* PPU cycle step between horizontal scroll increments during a scanline. */
#define H_SCROLL_INC_STEP      8u
/* Last PPU cycle at which a horizontal scroll increment happens. */
#define H_SCROLL_INC_LAST      248u

/* ---- Forward declarations for private static helpers ----------------- */

static uint8_t ppu_read_status(Ppu* p);
static uint8_t ppu_read_oamdata(Ppu* p);
static void    ppu_write_oamdata(Ppu* p, uint8_t value);
static void    ppu_write_ppuctrl(Ppu* p, uint8_t value);
static void    ppu_write_ppuscroll(Ppu* p, uint8_t value);
static void    ppu_write_ppuaddr(Ppu* p, uint8_t value);
static void    increment_h_scroll(Ppu* p);
static void    increment_v_scroll(Ppu* p);
static void    copy_h_t_to_v(Ppu* p);
static void    copy_v_t_to_v(Ppu* p);
static uint16_t map_nametable(const Ppu* p, uint16_t addr);
static uint16_t map_palette(uint16_t addr);

/* ---- Construction / configuration ------------------------------------ */

void ppu_init(Ppu* p) {
    /* mod.rs `Ppu::new`. Zero everything, then set defaults. */
    memset(p, 0, sizeof(*p));
    p->vram_size = PPU_VRAM_SIZE_2K;
    p->mirroring = MIRROR_HORIZONTAL;
    p->region = REGION_NTSC;
    /* render, framebuffer, vram, oam, palette already zeroed by memset. */
}

void ppu_set_mirroring(Ppu* p, Mirroring m) {
    /* mod.rs `Ppu::set_mirroring`. The vram array is always 4 KB; for H/V
     * mirroring only the first vram_size (0x800) bytes are used. When
     * switching to FourScreen we just expand vram_size to 0x1000 (the extra
     * bytes are already zeroed from ppu_init). When switching back we shrink
     * vram_size; bytes beyond are ignored. We do not need to copy because the
     * array is fixed-size and the first 0x800 bytes are preserved. */
    uint32_t needed = (m == MIRROR_FOUR_SCREEN) ? PPU_VRAM_SIZE_4K : PPU_VRAM_SIZE_2K;
    if (p->vram_size != needed) {
        if (needed > p->vram_size) {
            /* Zero the newly-exposed bytes (preserve existing 0x800). */
            memset(p->vram + p->vram_size, 0, needed - p->vram_size);
        }
        p->vram_size = needed;
    }
    p->mirroring = m;
}

Region ppu_region(const Ppu* p) {
    return p->region;
}

void ppu_set_region(Ppu* p, Region r) {
    /* mod.rs `Ppu::set_region`. Clamp scanline into the new region's range. */
    p->region = r;
    uint16_t max_sl = region_scanlines_per_frame(r);
    if (p->scanline >= max_sl) {
        p->scanline = (uint16_t)(max_sl - 1u);
    }
}

void ppu_set_slant_corruption(Ppu* p, bool enabled) {
    p->render.slant_corruption = enabled;
}

void ppu_set_inaccurate_palette(Ppu* p, bool enabled) {
    p->render.use_inaccurate_palette = enabled;
}

void ppu_set_nmi_retrigger(Ppu* p, bool enabled) {
    p->render.nmi_retrigger = enabled;
}

/* ---- Register reads / writes ----------------------------------------- */

uint8_t ppu_read_register(Ppu* p, uint16_t reg) {
    /* mod.rs `Ppu::read_register`. Index 7 returns the buffered value only;
     * the bus handles the full CHR-routed semantics. */
    switch (reg & 0x07u) {
        case 0: case 1: case 3: case 5: case 6:
            return p->open_bus;
        case 2: return ppu_read_status(p);
        case 4: return ppu_read_oamdata(p);
        case 7: return p->ppudata_buffer;
        default: return p->open_bus;
    }
}

void ppu_write_register(Ppu* p, uint16_t reg, uint8_t value) {
    /* mod.rs `Ppu::write_register`. All writes latch open_bus. */
    p->open_bus = value;
    switch (reg & 0x07u) {
        case 0: ppu_write_ppuctrl(p, value); break;
        case 1: p->ppumask = value; break;
        case 2: /* PPUSTATUS read-only; writes ignored (open-bus still latched). */ break;
        case 3: p->oamaddr = value; break;
        case 4: ppu_write_oamdata(p, value); break;
        case 5: ppu_write_ppuscroll(p, value); break;
        case 6: ppu_write_ppuaddr(p, value); break;
        case 7: /* handled by bus (ppu_write_ppudata); no-op here. */ break;
        default: break;
    }
}

/* ---- PPUSTATUS ($2002) ----------------------------------------------- */

static uint8_t ppu_read_status(Ppu* p) {
    /* mod.rs `Ppu::read_status`. Returns flags | open_bus low bits, clears
     * VBlank, resets w, latches the combined byte onto open bus. */
    uint8_t result = (uint8_t)((p->ppustatus & PPU_STATUS_FLAG_MASK) |
                               (p->open_bus & PPU_STATUS_OPEN_BUS_MASK));
    p->ppustatus &= (uint8_t)(~PPU_STATUS_VBLANK);
    p->w = false;
    p->open_bus = result;
    return result;
}

/* ---- OAMDATA ($2004) ------------------------------------------------- */

static uint8_t ppu_read_oamdata(Ppu* p) {
    /* mod.rs `Ppu::read_oamdata`. Returns OAM[oamaddr], increments oamaddr,
     * latches the byte onto open bus. */
    uint8_t value = p->oam[p->oamaddr];
    p->oamaddr = (uint8_t)(p->oamaddr + 1u);
    p->open_bus = value;
    return value;
}

static void ppu_write_oamdata(Ppu* p, uint8_t value) {
    /* mod.rs `Ppu::write_oamdata`. */
    p->oam[p->oamaddr] = value;
    p->oamaddr = (uint8_t)(p->oamaddr + 1u);
}

/* ---- PPUCTRL ($2000) ------------------------------------------------- */

static void ppu_write_ppuctrl(Ppu* p, uint8_t value) {
    /* mod.rs `Ppu::write_register` case 0. Copies base-NT bits 0-1 into t
     * bits 10-11. NMI-during-VBlank quirk: rising edge of bit 7 while in
     * VBlank latches nmi_request. Debug nmi_retrigger reproduces the old
     * buggy behavior (every write with bit 7 set during VBlank). */
    bool nmi_was_enabled = (p->ppuctrl & PPU_CTRL_NMI) != 0u;
    p->ppuctrl = value;
    uint16_t nt = (uint16_t)(value & PPU_CTRL_BASE_NT_MASK);
    p->t = (uint16_t)((p->t & (uint16_t)(~PPU_NT_SELECT_MASK)) | (uint16_t)(nt << 10));
    if ((value & PPU_CTRL_NMI) != 0u && ppu_in_vblank(p)) {
        if (p->render.nmi_retrigger || !nmi_was_enabled) {
            p->nmi_request = true;
        }
    }
}

/* ---- PPUSCROLL ($2005) ----------------------------------------------- */

static void ppu_write_ppuscroll(Ppu* p, uint8_t value) {
    /* mod.rs `Ppu::write_ppuscroll`. Shared w latch. */
    if (!p->w) {
        /* First write: coarse X -> t[0:4], fine X -> fine_x. */
        p->t = (uint16_t)((p->t & 0xFFE0u) | (uint16_t)(value >> 3));
        p->fine_x = (uint8_t)(value & 0x07u);
        p->w = true;
        /* Mid-scanline PPUSCROLL writes change the render position's fine X;
         * mark v_dirty so the per-pixel renderer re-syncs. */
        p->render.v_dirty = true;
    } else {
        /* Second write: coarse Y -> t[5:9], fine Y -> t[12:14]. */
        p->t = (uint16_t)((p->t & 0x8C1Fu) |
                          (uint16_t)(((uint16_t)(value & 0xF8u)) << 2) |
                          (uint16_t)(((uint16_t)(value & 0x07u)) << 12));
        p->w = false;
        /* Second write does NOT immediately affect the current scanline. */
    }
}

/* ---- PPUADDR ($2006) ------------------------------------------------- */

static void ppu_write_ppuaddr(Ppu* p, uint8_t value) {
    /* mod.rs `Ppu::write_ppuaddr`. Shared w latch. */
    if (!p->w) {
        /* First write: high byte -> t[8:13], clear bit 14. */
        p->t = (uint16_t)((p->t & 0x00FFu) | (uint16_t)(((uint16_t)(value & 0x3Fu)) << 8));
        p->w = true;
    } else {
        /* Second write: low byte -> t[0:7], then copy t -> v. */
        p->t = (uint16_t)((p->t & 0xFF00u) | (uint16_t)value);
        p->v = p->t;
        p->w = false;
        /* Do NOT set v_dirty here (matches mod.rs comment about fetch delay). */
    }
}

/* ---- PPUDATA accessors (used by the bus) ----------------------------- */

uint16_t ppu_vram_addr(const Ppu* p) {
    return p->v;
}

uint16_t ppu_vram_increment(const Ppu* p) {
    return ((p->ppuctrl & PPU_CTRL_INCREMENT_32) != 0u) ? 32u : 1u;
}

void ppu_advance_vram_addr(Ppu* p) {
    /* mod.rs `Ppu::advance_vram_addr`. Wraps within 14-bit PPU address space. */
    uint16_t inc = ppu_vram_increment(p);
    p->v = (uint16_t)((p->v + inc) & 0x3FFFu);
}

uint8_t ppu_ppudata_buffer(const Ppu* p) {
    return p->ppudata_buffer;
}

void ppu_set_ppudata_buffer(Ppu* p, uint8_t value) {
    p->ppudata_buffer = value;
}

uint8_t ppu_open_bus(const Ppu* p) {
    return p->open_bus;
}

void ppu_set_open_bus(Ppu* p, uint8_t value) {
    p->open_bus = value;
}

/* ---- Nametable / palette memory access ------------------------------- */

/* Map a nametable address ($2000-$3EFF) to a VRAM byte index.
 * (mod.rs `Ppu::map_nametable`.) $3000-$3EFF mirrors $2000-$2EFF; the four
 * nametable slots collapse according to mirroring.
 *
 * NOTE: the C Mirroring enum (cartridge.h) lacks SingleScreen. The Rust
 * Mirroring::SingleScreen(nt) maps all four NTs to a single physical NT. For
 * M4.2 we only handle H/V/4-screen (the only modes NROM uses in M4.4); any
 * unknown value is treated as horizontal. This is documented and acceptable
 * for the M4.2 scope. */
static uint16_t map_nametable(const Ppu* p, uint16_t addr) {
    uint16_t a = (uint16_t)(addr & 0x2FFFu);
    uint16_t local = (uint16_t)(a - 0x2000u);       /* 0..0xFFF */
    uint16_t nt = (uint16_t)(local >> 10);          /* nametable index 0..3 */
    uint16_t offset = (uint16_t)(local & 0x03FFu);  /* byte within nametable */
    uint16_t phys;
    switch (p->mirroring) {
        case MIRROR_HORIZONTAL:  phys = (uint16_t)(nt >> 1); break; /* NT 0,1->0; 2,3->1 */
        case MIRROR_VERTICAL:    phys = (uint16_t)(nt & 1u);  break; /* NT 0,2->0; 1,3->1 */
        case MIRROR_FOUR_SCREEN: phys = nt;                   break; /* all four unique */
        default:                 phys = (uint16_t)(nt >> 1);  break; /* unknown -> horizontal */
    }
    return (uint16_t)(phys * 0x400u + offset);
}

uint8_t ppu_read_nametable(const Ppu* p, uint16_t addr) {
    return p->vram[map_nametable(p, addr)];
}

void ppu_write_nametable(Ppu* p, uint16_t addr, uint8_t value) {
    p->vram[map_nametable(p, addr)] = value;
}

/* Map a palette address ($3F00-$3FFF) to a palette RAM index (0..31).
 * (mod.rs `Ppu::map_palette`.) $3F20-$3FFF mirrors $3F00-$3F1F; within the
 * 32-byte palette, $3F10/$3F14/$3F18/$3F1C mirror $3F00/$3F04/$3F08/$3F0C. */
static uint16_t map_palette(uint16_t addr) {
    uint8_t a = (uint8_t)(addr & 0x1Fu);
    if ((a & 0x13u) == 0x10u) {
        return (uint16_t)(a & 0x0Fu);
    }
    return (uint16_t)a;
}

uint8_t ppu_read_palette(const Ppu* p, uint16_t addr) {
    return p->palette[map_palette(addr)];
}

void ppu_write_palette(Ppu* p, uint16_t addr, uint8_t value) {
    p->palette[map_palette(addr)] = value;
}

/* ---- OAM DMA ($4014) ------------------------------------------------- */

void ppu_oam_dma(Ppu* p, const uint8_t data[256]) {
    /* mod.rs `Ppu::oam_dma`. Copy 256 bytes into OAM, reset oamaddr to 0. */
    memcpy(p->oam, data, 256u);
    p->oamaddr = 0u;
}

/* ---- VBlank / NMI / sprite flag control ------------------------------ */

void ppu_set_vblank(Ppu* p, bool on) {
    if (on) p->ppustatus |= PPU_STATUS_VBLANK;
    else    p->ppustatus &= (uint8_t)(~PPU_STATUS_VBLANK);
}

void ppu_set_sprite_zero_hit(Ppu* p, bool on) {
    if (on) p->ppustatus |= PPU_STATUS_SPRITE_ZERO;
    else    p->ppustatus &= (uint8_t)(~PPU_STATUS_SPRITE_ZERO);
}

void ppu_set_sprite_overflow(Ppu* p, bool on) {
    if (on) p->ppustatus |= PPU_STATUS_OVERFLOW;
    else    p->ppustatus &= (uint8_t)(~PPU_STATUS_OVERFLOW);
}

bool ppu_nmi_enabled(const Ppu* p) {
    return (p->ppuctrl & PPU_CTRL_NMI) != 0u;
}

bool ppu_in_vblank(const Ppu* p) {
    return (p->ppustatus & PPU_STATUS_VBLANK) != 0u;
}

bool ppu_sprite_zero_hit(const Ppu* p) {
    return (p->ppustatus & PPU_STATUS_SPRITE_ZERO) != 0u;
}

bool ppu_sprite_overflow(const Ppu* p) {
    return (p->ppustatus & PPU_STATUS_OVERFLOW) != 0u;
}

/* ---- Scanline / cycle stepper ---------------------------------------- */

uint16_t ppu_scanline(const Ppu* p) {
    return p->scanline;
}

uint16_t ppu_cycle(const Ppu* p) {
    return p->cycle;
}

bool ppu_take_nmi_request(Ppu* p) {
    bool r = p->nmi_request;
    p->nmi_request = false;
    return r;
}

/* Horizontal scroll increment: advance coarse X (in v); on coarse-X wrap
 * (past 31), toggle the horizontal nametable bit. (mod.rs
 * `Ppu::increment_h_scroll`.) Includes the slant_corruption debug branch. */
static void increment_h_scroll(Ppu* p) {
    if (p->render.slant_corruption) {
        if (p->fine_x < 7u) {
            p->fine_x = (uint8_t)(p->fine_x + 1u);
            p->render.fine_x_start = (uint8_t)((p->render.fine_x_start + 1u) & 0x07u);
        } else {
            p->fine_x = 0u;
            p->render.fine_x_start = 0u;
            uint16_t coarse_x = (uint16_t)(p->v & PPU_COARSE_X_MASK);
            if (coarse_x == 31u) {
                p->v &= (uint16_t)(~PPU_COARSE_X_MASK);
                p->v ^= PPU_NT_H_BIT;
                p->render.coarse_x_start = 0u;
                p->render.nt_h_start ^= PPU_NT_H_BIT;
            } else {
                p->v = (uint16_t)((p->v & (uint16_t)(~PPU_COARSE_X_MASK)) | (uint16_t)(coarse_x + 1u));
                p->render.coarse_x_start = (uint16_t)((coarse_x + 1u) & PPU_COARSE_X_MASK);
            }
        }
    } else {
        uint16_t coarse_x = (uint16_t)(p->v & PPU_COARSE_X_MASK);
        if (coarse_x == 31u) {
            p->v &= (uint16_t)(~PPU_COARSE_X_MASK);
            p->v ^= PPU_NT_H_BIT;
        } else {
            p->v = (uint16_t)((p->v & (uint16_t)(~PPU_COARSE_X_MASK)) | (uint16_t)(coarse_x + 1u));
        }
    }
}

/* Vertical scroll increment: advance fine Y (in v); on wrap, advance coarse
 * Y; on coarse-Y wrap past 29, reset coarse Y and toggle the vertical NT bit;
 * on wrap past 31, just reset coarse Y. (mod.rs `Ppu::increment_v_scroll`.) */
static void increment_v_scroll(Ppu* p) {
    uint16_t fine_y = (uint16_t)((p->v & PPU_FINE_Y_MASK) >> 12);
    if (fine_y < 7u) {
        p->v = (uint16_t)((p->v & (uint16_t)(~PPU_FINE_Y_MASK)) | (uint16_t)((fine_y + 1u) << 12));
    } else {
        p->v &= (uint16_t)(~PPU_FINE_Y_MASK); /* fine Y -> 0 */
        uint16_t coarse_y = (uint16_t)((p->v & PPU_COARSE_Y_MASK) >> 5);
        if (coarse_y == 29u) {
            p->v &= (uint16_t)(~PPU_COARSE_Y_MASK);
            p->v ^= PPU_NT_V_BIT;
        } else if (coarse_y == 31u) {
            p->v &= (uint16_t)(~PPU_COARSE_Y_MASK);
        } else {
            p->v = (uint16_t)((p->v & (uint16_t)(~PPU_COARSE_Y_MASK)) | (uint16_t)((coarse_y + 1u) << 5));
        }
    }
}

/* Copy the horizontal components of t (coarse X + nametable bit 0) into v.
 * (mod.rs `Ppu::copy_h_t_to_v`.) Happens at cycle 257 of every visible sl. */
static void copy_h_t_to_v(Ppu* p) {
    uint16_t h_bits = (uint16_t)(p->t & (PPU_COARSE_X_MASK | PPU_NT_H_BIT));
    p->v = (uint16_t)((p->v & (uint16_t)(~(PPU_COARSE_X_MASK | PPU_NT_H_BIT))) | h_bits);
}

/* Copy the vertical components of t (coarse Y, fine Y, nametable bit 1) into
 * v. (mod.rs `Ppu::copy_v_t_to_v`.) Happens at prerender cycles 280-304. */
static void copy_v_t_to_v(Ppu* p) {
    uint16_t v_bits = (uint16_t)(p->t & (PPU_COARSE_Y_MASK | PPU_NT_V_BIT | PPU_FINE_Y_MASK));
    p->v = (uint16_t)((p->v & (uint16_t)(~(PPU_COARSE_Y_MASK | PPU_NT_V_BIT | PPU_FINE_Y_MASK))) | v_bits);
}

bool ppu_step(Ppu* p) {
    /* mod.rs `Ppu::step`. Returns true if an NMI should be raised this cycle. */
    bool nmi = false;

    uint16_t scanlines_per_frame = region_scanlines_per_frame(p->region);
    uint16_t prerender = region_scanline_prerender(p->region);

    /* Advance the cycle counter first. */
    p->cycle = (uint16_t)(p->cycle + 1u);
    if (p->cycle >= PPU_CYCLES_PER_SCANLINE) {
        p->cycle = 0u;
        p->scanline = (uint16_t)(p->scanline + 1u);
        if (p->scanline >= scanlines_per_frame) {
            p->scanline = 0u;
        }
    }

    /* Per-cycle events at the new (cycle, scanline). */
    if (p->scanline == PPU_SCANLINE_VBLANK_START && p->cycle == VBLANK_NMI_CYCLE) {
        ppu_set_vblank(p, true);
        if (ppu_nmi_enabled(p)) {
            p->nmi_request = true;
            nmi = true;
        }
    } else if (p->scanline == prerender && p->cycle == VBLANK_NMI_CYCLE) {
        ppu_set_vblank(p, false);
        ppu_set_sprite_overflow(p, false);
        ppu_set_sprite_zero_hit(p, false);
        /* Reset the per-pixel renderer's per-frame latch so sprite 0 hit can
         * trigger again on the next frame. */
        p->render.sprite_zero_hit = false;
    }

    /* Rendering-only scroll updates. */
    bool rendering = (p->ppumask & (PPU_MASK_SHOW_BG | PPU_MASK_SHOW_SPRITES)) != 0u;
    if (rendering) {
        bool does_scroll_inc =
            (p->scanline < PPU_SCREEN_HEIGHT) || (p->scanline == prerender);
        /* Horizontal scroll increment at dots 8,16,...,248. */
        if (does_scroll_inc &&
            p->cycle >= H_SCROLL_INC_STEP &&
            p->cycle <= H_SCROLL_INC_LAST &&
            (p->cycle % H_SCROLL_INC_STEP) == 0u) {
            increment_h_scroll(p);
        }
        /* Vertical scroll increment at dot 256. */
        if (does_scroll_inc && p->cycle == VERT_SCROLL_INC_CYCLE) {
            increment_v_scroll(p);
        }
        /* Horizontal t->v copy at dot 257 (visible + prerender only). */
        if (does_scroll_inc && p->cycle == H_COPY_CYCLE) {
            copy_h_t_to_v(p);
        }
        /* Vertical t->v copy at prerender dots 280-304. */
        if (p->scanline == prerender &&
            p->cycle >= V_COPY_CYCLE_START &&
            p->cycle <= V_COPY_CYCLE_END) {
            copy_v_t_to_v(p);
        }
    }

    return nmi;
}

bool ppu_step_rendered(Ppu* p, ChrReader* chr_read) {
    /* mod.rs `Ppu::step_rendered`. Run the architectural step first, then
     * render one pixel per PPU cycle on visible scanlines (cycles 1-256). */
    bool nmi = ppu_step(p);

    /* Reset the per-scanline render init flag at cycle 0. */
    if (p->cycle == 0u) {
        p->render.scanline_initialized = false;
        p->render.pipeline_primed = false;
    }

    /* Per-pixel rendering: one pixel per cycle on visible scanlines. */
    if (p->scanline < PPU_SCREEN_HEIGHT &&
        p->cycle >= 1u &&
        p->cycle <= PPU_SCREEN_WIDTH) {
        ppu_render_one_pixel(p, chr_read);
        p->render.rendered_this_frame = true;
    }

    return nmi;
}

/* ---- Test / debug accessors ------------------------------------------ */

uint8_t ppu_ppuctrl(const Ppu* p) { return p->ppuctrl; }
uint8_t ppu_ppumask(const Ppu* p) { return p->ppumask; }

bool ppu_is_rendering(const Ppu* p) {
    return (p->ppumask & (PPU_MASK_SHOW_BG | PPU_MASK_SHOW_SPRITES)) != 0u;
}

uint8_t ppu_ppustatus(const Ppu* p) { return p->ppustatus; }
uint8_t ppu_oamaddr(const Ppu* p) { return p->oamaddr; }
uint16_t ppu_temp_vram_addr(const Ppu* p) { return p->t; }
uint8_t ppu_fine_x(const Ppu* p) { return p->fine_x; }
bool ppu_write_latch(const Ppu* p) { return p->w; }

const uint8_t* ppu_oam(const Ppu* p) { return p->oam; }
uint8_t* ppu_oam_mut(Ppu* p) { return p->oam; }
const uint8_t* ppu_palette(const Ppu* p) { return p->palette; }
const uint8_t* ppu_vram(const Ppu* p) { return p->vram; }
Mirroring ppu_mirroring(const Ppu* p) { return p->mirroring; }
const uint32_t* ppu_framebuffer(const Ppu* p) { return p->framebuffer; }
uint32_t* ppu_framebuffer_mut(Ppu* p) { return p->framebuffer; }
const uint8_t* ppu_bg_pattern(const Ppu* p) { return p->bg_pattern; }

bool ppu_rendered_this_frame(const Ppu* p) { return p->render.rendered_this_frame; }

void ppu_reset_rendered_flag(Ppu* p) {
    p->render.rendered_this_frame = false;
}

void ppu_screen_size(size_t* width, size_t* height) {
    *width = PPU_SCREEN_WIDTH;
    *height = PPU_SCREEN_HEIGHT;
}
