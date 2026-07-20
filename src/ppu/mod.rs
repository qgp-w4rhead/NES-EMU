//! Picture Processing Unit (PPU) — register interface and memory.
//!
//! This module implements the PPU register file (`$2000-`$2007` + `$4014
//! OAMDMA), internal VRAM (nametables), OAM (sprite RAM), and palette
//! memory. The rendering pipeline (background/sprite pixel generation,
//! scanline timing) is added in M8-M11; M7 covers only the register
//! interface and memory access semantics.
//!
//! # PPU address space
//!
//! | Range           | Device                                     |
//! |-----------------|--------------------------------------------|
//! | `$0000-$0FFF`   | Pattern table 0 (CHR, via cartridge)       |
//! | `$1000-$1FFF`   | Pattern table 1 (CHR, via cartridge)       |
//! | `$2000-$2FFF`   | Nametables (4 × 1 KB, mirrored)            |
//! | `$3000-$3EFF`   | Mirror of `$2000-$2EFF`                    |
//! | `$3F00-$3FFF`   | Palette RAM (25 unique bytes, rest mirrored)|
//!
//! See: https://www.nesdev.org/wiki/PPU_registers
//! See: https://www.nesdev.org/wiki/PPU_memory_map
//!
//! # Register summary
//!
//! | Addr    | Name      | R/W | Notes                                         |
//! |---------|-----------|-----|-----------------------------------------------|
//! | `$2000` | PPUCTRL   | W   | NMI enable, increment, pattern tables, etc.  |
//! | `$2001` | PPUMASK   | W   | Rendering enable, color emphasis.            |
//! | `$2002` | PPUSTATUS | R   | VBlank / sprite 0 hit / overflow; clears VBlank on read. |
//! | `$2003` | OAMADDR   | W   | OAM address pointer.                         |
//! | `$2004` | OAMDATA   | RW  | Read/write OAM at OAMADDR; increments.       |
//! | `$2005` | PPUSCROLL | W   | Two writes: X then Y (shared `w` latch).     |
//! | `$2006` | PPUADDR   | W   | Two writes: hi then lo (shared `w` latch).   |
//! | `$2007` | PPUDATA   | RW  | VRAM/palette access; buffered read; auto-increment. |
//! | `$4014` | OAMDMA    | W   | 256-byte DMA from CPU page to OAM.           |

#![allow(dead_code)]

pub mod render;

use crate::mappers::Mirroring;

/// Visible screen width in pixels.
pub const SCREEN_WIDTH: usize = 256;
/// Visible screen height in pixels (240 scanlines).
pub const SCREEN_HEIGHT: usize = 240;
/// Total framebuffer pixel count.
pub const FRAMEBUFFER_SIZE: usize = SCREEN_WIDTH * SCREEN_HEIGHT;

/// Size of nametable VRAM (4 KB — enough for 4-screen mirroring; H/V use 2 KB).
const VRAM_SIZE: usize = 0x1000;

/// Size of OAM (64 sprites × 4 bytes = 256 bytes).
const OAM_SIZE: usize = 256;

/// Size of palette RAM (32 bytes; `$3F20-$3FFF` mirrors down to 32 bytes).
const PALETTE_SIZE: usize = 32;

/// PPUCTRL bit: generate NMI on VBlank.
const CTRL_NMI: u8 = 0b1000_0000;
/// PPUCTRL bit: VRAM address increment by 32 instead of 1.
const CTRL_INCREMENT_32: u8 = 0b0000_0100;
/// PPUCTRL bit: background pattern table select (0 = `$0000`, 1 = `$1000`).
const CTRL_BG_PATTERN_1000: u8 = 0b0001_0000;
/// PPUCTRL bit: sprite pattern table select (0 = `$0000`, 1 = `$1000`).
const CTRL_SPRITE_PATTERN_1000: u8 = 0b0000_1000;
/// PPUCTRL bits 0-1: base nametable address (`$2000`/`$2400`/`$2800`/`$2C00`).
const CTRL_BASE_NT_MASK: u8 = 0b0000_0011;
/// PPUMASK bit: show background (enable background rendering).
const MASK_SHOW_BG: u8 = 0b0000_1000;
/// PPUMASK bit: show leftmost 8 pixels of background.
const MASK_SHOW_BG_LEFT: u8 = 0b0000_0010;
/// PPUMASK bit: show sprites (enable sprite rendering).
const MASK_SHOW_SPRITES: u8 = 0b0001_0000;
/// PPUMASK bit: show leftmost 8 pixels of sprites.
const MASK_SHOW_SPRITES_LEFT: u8 = 0b0000_0100;

/// PPUSTATUS bit: VBlank (bit 7).
const STATUS_VBLANK: u8 = 0b1000_0000;
/// PPUSTATUS bit: sprite 0 hit (bit 6).
const STATUS_SPRITE_ZERO: u8 = 0b0100_0000;
/// PPUSTATUS bit: sprite overflow (bit 5).
const STATUS_OVERFLOW: u8 = 0b0010_0000;
/// Mask for the status flag bits (bits 7-5).
const STATUS_FLAG_MASK: u8 = 0b1110_0000;
/// Mask for the open-bus bits in PPUSTATUS reads (bits 4-0).
const STATUS_OPEN_BUS_MASK: u8 = 0b0001_1111;

// ---- Scanline / cycle timing (M10) -------------------------------------
//
// The NTSC PPU runs 262 scanlines per frame, 341 PPU cycles per scanline.
// Scanlines 0-239 are visible, 240 is post-render (idle), 241-260 are
// VBlank, and 261 is the prerender scanline (sometimes labelled -1).
//
// See: https://www.nesdev.org/wiki/PPU_rendering#Timing
/// Number of scanlines in an NTSC frame (262).
pub const SCANLINES_PER_FRAME: u16 = 262;
/// Number of PPU cycles per scanline (341).
pub const CYCLES_PER_SCANLINE: u16 = 341;
/// Scanline on which VBlank begins (NMI asserted at cycle 1).
pub const SCANLINE_VBLANK_START: u16 = 241;
/// Prerender scanline (VBlank cleared + status flags cleared at cycle 1).
pub const SCANLINE_PRERENDER: u16 = 261;
/// PPU cycle within the scanline at which VBlank is asserted / cleared.
const VBLANK_NMI_CYCLE: u16 = 1;
/// PPU cycle at which the vertical scroll component is incremented (fine Y).
const VERT_SCROLL_INC_CYCLE: u16 = 256;
/// PPU cycle at which the horizontal `t→v` copy happens (end of visible line).
const H_COPY_CYCLE: u16 = 257;
/// First prerender cycle at which the vertical `t→v` copy happens.
const V_COPY_CYCLE_START: u16 = 280;
/// Last prerender cycle at which the vertical `t→v` copy happens.
const V_COPY_CYCLE_END: u16 = 304;
/// PPU cycle step between horizontal scroll increments during a scanline.
const H_SCROLL_INC_STEP: u16 = 8;
/// Last PPU cycle at which a horizontal scroll increment happens.
const H_SCROLL_INC_LAST: u16 = 248;

/// Mask for the nametable-select bits (10-11) of the `t` / `v` registers.
const NT_SELECT_MASK: u16 = 0b0000_1100_0000_0000;
/// Mask for the coarse-X bits (0-4) of the `t` / `v` registers.
const COARSE_X_MASK: u16 = 0b0000_0000_0001_1111;
/// Mask for the coarse-Y bits (5-9) of the `t` / `v` registers.
const COARSE_Y_MASK: u16 = 0b0000_0011_1110_0000;
/// Mask for the fine-Y bits (12-14) of the `t` / `v` registers.
const FINE_Y_MASK: u16 = 0b0111_1000_0000_0000;
/// Bit 0 of the nametable select (horizontal wrap).
const NT_H_BIT: u16 = 0b0000_0100_0000_0000;
/// Bit 1 of the nametable select (vertical wrap).
const NT_V_BIT: u16 = 0b0000_1000_0000_0000;

/// The Picture Processing Unit.
///
/// Owns nametable VRAM, OAM, and palette RAM. CHR pattern data (`$0000-$1FFF`)
/// is owned by the cartridge and accessed through the bus — the PPU never
/// touches CHR directly.
pub struct Ppu {
    // ---- writable registers (write-only; reads return open bus) ----
    ppuctrl: u8,
    ppumask: u8,
    oamaddr: u8,

    // ---- readable status register ----
    ppustatus: u8,

    // ---- VRAM address registers (used by PPUSCROLL / PPUADDR / PPUDATA) ----
    /// Current VRAM address (15-bit, effectively 14-bit since bit 14 is
    /// masked off by the address bus). Used by PPUDATA reads/writes.
    v: u16,
    /// Temporary VRAM address (15-bit). Loaded by PPUSCROLL and PPUADDR
    /// writes; copied to `v` on the second PPUADDR write and during
    /// rendering (M8+).
    t: u16,
    /// Fine X scroll (3 bits), set by the first PPUSCROLL write.
    fine_x: u8,
    /// Shared write latch for PPUSCROLL and PPUADDR (false = first write,
    /// true = second write). Cleared by PPUSTATUS read and at VBlank start.
    w: bool,

    // ---- PPUDATA buffered read ----
    /// Buffered data for PPUDATA reads. The first read from a non-palette
    /// address returns this stale value; the real data is loaded into the
    /// buffer for the *next* read. Palette reads bypass the buffer.
    ppudata_buffer: u8,

    /// Open-bus latch: the last byte written to any PPU register. Reads of
    /// write-only registers return this; PPUSTATUS returns its low 5 bits
    /// alongside the status flags.
    open_bus: u8,

    // ---- memory ----
    /// Nametable VRAM (4 KB). For horizontal/vertical mirroring only 2 KB
    /// is meaningful; the mapping is computed by [`Ppu::map_nametable`].
    vram: [u8; VRAM_SIZE],
    /// Object Attribute Memory — 64 sprites × 4 bytes.
    oam: [u8; OAM_SIZE],
    /// Palette RAM (32 bytes). `$3F00-$3F1F` with internal mirroring.
    palette: [u8; PALETTE_SIZE],

    // ---- framebuffer (M8: background rendering) ----
    /// Output framebuffer: 256×240 ARGB pixels (0xAARRGGBB). Heap-allocated
    /// (240 KB) so the `Ppu` struct stays small enough to construct on the
    /// stack in tests. Written by the background (and, later, sprite)
    /// rendering pipeline; the video layer (M12) uploads it to an SDL2
    /// texture each frame. Allocated once at construction — no allocation
    /// in the render path.
    framebuffer: Vec<u32>,

    // ---- background pattern buffer (M9: sprite priority) ----
    /// Per-pixel background pattern value (0-3) from the most recent
    /// background render. Used by [`Ppu::render_sprites`] to resolve
    /// sprite priority: a "behind background" sprite only shows where
    /// the background is transparent (pattern 0). Heap-allocated
    /// (61 KB), filled by [`Ppu::render_background`].
    bg_pattern: Vec<u8>,

    // ---- scanline / cycle timing (M10) ----
    /// Current scanline within the frame (0..=261; 261 = prerender).
    scanline: u16,
    /// Current PPU cycle within the scanline (0..=340).
    cycle: u16,
    /// Set when VBlank begins and PPUCTRL bit 7 (NMI enable) is set.
    /// The bus / emulator polls this via [`Ppu::take_nmi_request`] to
    /// raise `Cpu::nmi_pending`. Latched (not auto-cleared) so a slow
    /// consumer can't miss it — `take_nmi_request` clears it.
    nmi_request: bool,

    // ---- configuration ----
    /// Nametable mirroring mode, set from the cartridge by the bus.
    mirroring: Mirroring,
}

impl Ppu {
    /// Construct a PPU in power-on state: all registers zeroed, VRAM/OAM/
    /// palette uninitialised (zeroed), horizontal mirroring by default.
    ///
    /// On real hardware VRAM and palette contents are random at power-on;
    /// we zero them for determinism (the tech-stack mandates deterministic
    /// emulation).
    pub fn new() -> Self {
        Self {
            ppuctrl: 0,
            ppumask: 0,
            oamaddr: 0,
            ppustatus: 0,
            v: 0,
            t: 0,
            fine_x: 0,
            w: false,
            ppudata_buffer: 0,
            open_bus: 0,
            vram: [0u8; VRAM_SIZE],
            oam: [0u8; OAM_SIZE],
            palette: [0u8; PALETTE_SIZE],
            framebuffer: vec![0u32; FRAMEBUFFER_SIZE],
            bg_pattern: vec![0u8; FRAMEBUFFER_SIZE],
            scanline: 0,
            cycle: 0,
            nmi_request: false,
            mirroring: Mirroring::Horizontal,
        }
    }

    /// Set the nametable mirroring mode (from the cartridge, via the bus).
    pub fn set_mirroring(&mut self, mirroring: Mirroring) {
        self.mirroring = mirroring;
    }

    // =================================================================
    //  Register reads / writes (called by Bus::ppu_read / ppu_write)
    // =================================================================

    /// Read a PPU register by de-mirrored index (`0..=7` = `$2000..=$2007`).
    ///
    /// Write-only registers (PPUCTRL, PPUMASK, OAMADDR, PPUSCROLL, PPUADDR)
    /// return the open-bus latch. PPUSTATUS returns the status flags with
    /// open-bus low bits and has read side-effects (clears VBlank, resets
    /// `w`). OAMDATA returns OAM at the current OAMADDR and increments it.
    /// PPUDATA (index 7) is handled by the bus (it needs CHR routing) —
    /// calling this method for index 7 returns the buffered value without
    /// advancing the address; the bus should use [`Ppu::read_ppudata_step`]
    /// instead.
    pub fn read_register(&mut self, reg: u16) -> u8 {
        match reg & 0x07 {
            0 | 1 | 3 | 5 | 6 => self.open_bus,
            2 => self.read_status(),
            4 => self.read_oamdata(),
            7 => self.ppudata_buffer, // bus handles full semantics
            _ => self.open_bus,
        }
    }

    /// Write a PPU register by de-mirrored index (`0..=7`).
    ///
    /// All writes update the open-bus latch. PPUDATA (index 7) is handled
    /// by the bus (it needs CHR routing) — calling this method for index 7
    /// is a no-op apart from updating open-bus; the bus should use
    /// [`Ppu::write_ppudata_step`] instead.
    pub fn write_register(&mut self, reg: u16, value: u8) {
        self.open_bus = value;
        match reg & 0x07 {
            0 => {
                // PPUCTRL: in addition to latching the register, writing it
                // copies the base-nametable bits (0-1) into the `t`
                // register's nametable-select bits (10-11). This is how the
                // base nametable becomes part of the scroll position used
                // during rendering.
                //
                // NMI-during-VBlank quirk: if the NMI-enable bit (bit 7)
                // is set *while VBlank is active*, an NMI is generated
                // immediately — not just at the VBlank-start edge. Games
                // rely on this when they enable NMI inside the VBlank
                // handler for the next frame.
                // See: https://www.nesdev.org/wiki/PPU_registers#PPUCTRL
                self.ppuctrl = value;
                let nt = (value as u16) & 0b11;
                self.t = (self.t & !NT_SELECT_MASK) | (nt << 10);
                if (value & CTRL_NMI) != 0 && self.in_vblank() {
                    self.nmi_request = true;
                }
            }
            1 => self.ppumask = value,
            2 => {} // PPUSTATUS is read-only; writes ignored (open-bus still latches)
            3 => self.oamaddr = value,
            4 => self.write_oamdata(value),
            5 => self.write_ppuscroll(value),
            6 => self.write_ppuaddr(value),
            7 => {} // handled by bus
            _ => {}
        }
    }

    // ---- PPUSTATUS ($2002) -------------------------------------------

    /// Read PPUSTATUS: returns the status byte (bits 7-5 = flags, bits 4-0
    /// = open bus), then clears the VBlank flag and resets the `w` latch.
    ///
    /// See: https://www.nesdev.org/wiki/PPU_registers#PPUSTATUS
    fn read_status(&mut self) -> u8 {
        let result = (self.ppustatus & STATUS_FLAG_MASK) | (self.open_bus & STATUS_OPEN_BUS_MASK);
        // Reading PPUSTATUS clears VBlank and resets the write latch.
        self.ppustatus &= !STATUS_VBLANK;
        self.w = false;
        result
    }

    // ---- OAMDATA ($2004) ---------------------------------------------

    /// Read OAMDATA: returns OAM at the current OAMADDR, then increments
    /// OAMADDR. During rendering this would return garbage, but M7 has no
    /// rendering yet.
    ///
    /// See: https://www.nesdev.org/wiki/PPU_registers#OAMDATA
    fn read_oamdata(&mut self) -> u8 {
        let value = self.oam[self.oamaddr as usize];
        self.oamaddr = self.oamaddr.wrapping_add(1);
        value
    }

    /// Write OAMDATA: writes to OAM at the current OAMADDR, then increments
    /// OAMADDR.
    fn write_oamdata(&mut self, value: u8) {
        self.oam[self.oamaddr as usize] = value;
        self.oamaddr = self.oamaddr.wrapping_add(1);
    }

    // ---- PPUSCROLL ($2005) -------------------------------------------

    /// Write PPUSCROLL using the shared `w` latch:
    /// - First write:  fine X (bits 0-2) + coarse X (bits 3-7) → `t` bits
    ///   0-4 and `fine_x`; `w` becomes true.
    /// - Second write: fine Y (bits 0-2) → `t` bits 12-14, coarse Y
    ///   (bits 3-7) → `t` bits 5-9; `w` becomes false.
    ///
    /// See: https://www.nesdev.org/wiki/PPU_registers#PPUSCROLL
    fn write_ppuscroll(&mut self, value: u8) {
        if !self.w {
            // First write: coarse X → t[0:4], fine X → fine_x.
            // Preserve bits 15-5 (fine Y, nametable select, coarse Y).
            self.t = (self.t & 0b1111_1111_1110_0000) | ((value as u16) >> 3);
            self.fine_x = value & 0b0000_0111;
            self.w = true;
        } else {
            // Second write: coarse Y → t[5:9], fine Y → t[12:14].
            // Preserve bits 15, 14-12 (fine Y is overwritten), 11-10
            // (nametable select), and 4-0 (coarse X). The mask keeps
            // bit 15 (unused), bits 11-10 (nametable select), and bits
            // 4-0 (coarse X); it clears bits 14-12 (fine Y) and bits
            // 9-5 (coarse Y) so the OR can set them from `value`.
            self.t = (self.t & 0b1000_1100_0001_1111)
                | (((value as u16) & 0b1111_1000) << 2)
                | (((value as u16) & 0b0000_0111) << 12);
            self.w = false;
        }
    }

    // ---- PPUADDR ($2006) ---------------------------------------------

    /// Write PPUADDR using the shared `w` latch:
    /// - First write:  high byte → `t` bits 8-13 (bit 14 cleared; PPU
    ///   address space is 14-bit); `w` becomes true.
    /// - Second write: low byte → `t` bits 0-7, then `t` is copied to `v`;
    ///   `w` becomes false.
    ///
    /// See: https://www.nesdev.org/wiki/PPU_registers#PPUADDR
    fn write_ppuaddr(&mut self, value: u8) {
        if !self.w {
            // First write: high byte → t[8:13], clear bit 14.
            self.t = (self.t & 0b0000_0000_1111_1111) | (((value as u16) & 0b0011_1111) << 8);
            self.w = true;
        } else {
            // Second write: low byte → t[0:7], then copy t → v.
            self.t = (self.t & 0b1111_1111_0000_0000) | (value as u16);
            self.v = self.t;
            self.w = false;
        }
    }

    // =================================================================
    //  PPUDATA ($2007) — bus-mediated access
    // =================================================================

    /// Current VRAM address (the `v` register). The bus reads this to
    /// decide where to route a PPUDATA access (CHR / nametable / palette).
    pub fn vram_addr(&self) -> u16 {
        self.v
    }

    /// VRAM address increment per PPUDATA access: 1 or 32, selected by
    /// PPUCTRL bit 2.
    pub fn vram_increment(&self) -> u16 {
        if (self.ppuctrl & CTRL_INCREMENT_32) != 0 {
            32
        } else {
            1
        }
    }

    /// Advance `v` by the current increment, wrapping within the 14-bit
    /// PPU address space (`$0000-$3FFF`).
    pub fn advance_vram_addr(&mut self) {
        let inc = self.vram_increment();
        self.v = (self.v.wrapping_add(inc)) & 0x3FFF;
    }

    /// Current PPUDATA read buffer (used by the bus for buffered reads).
    pub fn ppudata_buffer(&self) -> u8 {
        self.ppudata_buffer
    }

    /// Set the PPUDATA read buffer (used by the bus after fetching the real
    /// value from CHR or nametable memory).
    pub fn set_ppudata_buffer(&mut self, value: u8) {
        self.ppudata_buffer = value;
    }

    /// Read a nametable byte at `addr` (in `$2000-$3EFF`). Handles the
    /// `$3000-$3EFF` mirror and nametable mirroring.
    pub fn read_nametable(&self, addr: u16) -> u8 {
        let idx = self.map_nametable(addr);
        self.vram[idx]
    }

    /// Write a nametable byte at `addr` (in `$2000-$3EFF`).
    pub fn write_nametable(&mut self, addr: u16, value: u8) {
        let idx = self.map_nametable(addr);
        self.vram[idx] = value;
    }

    /// Read a palette byte at `addr` (in `$3F00-$3FFF`). Handles palette
    /// internal mirroring (`$3F10/$3F14/$3F18/$3F1C` mirror `$3F00/$3F04/
    /// `$3F08/$3F0C`; `$3F20-$3FFF` mirrors `$3F00-$3F1F`).
    pub fn read_palette(&self, addr: u16) -> u8 {
        let idx = self.map_palette(addr);
        self.palette[idx]
    }

    /// Write a palette byte at `addr` (in `$3F00-$3FFF`).
    pub fn write_palette(&mut self, addr: u16, value: u8) {
        let idx = self.map_palette(addr);
        self.palette[idx] = value;
    }

    // =================================================================
    //  OAM DMA ($4014)
    // =================================================================

    /// Perform a 256-byte OAM DMA: copy `data` into OAM starting at
    /// OAMADDR=0. On real hardware this takes 512 CPU cycles (the CPU is
    /// stalled); cycle accounting is deferred to M12.
    ///
    /// See: https://www.nesdev.org/wiki/PPU_registers#OAMDMA
    pub fn oam_dma(&mut self, data: &[u8; OAM_SIZE]) {
        self.oam.copy_from_slice(data);
        // After DMA, OAMADDR is left at 0 (the transfer always fills the
        // full 256 bytes starting from 0 on real hardware).
        self.oamaddr = 0;
    }

    // =================================================================
    //  VBlank / NMI control (used by PPU step in M10)
    // =================================================================

    /// Set or clear the VBlank flag (bit 7 of PPUSTATUS). Called by the
    /// PPU scanline stepper at VBlank start (set) and end (clear).
    pub fn set_vblank(&mut self, on: bool) {
        if on {
            self.ppustatus |= STATUS_VBLANK;
        } else {
            self.ppustatus &= !STATUS_VBLANK;
        }
    }

    /// Set the sprite 0 hit flag (bit 6 of PPUSTATUS). Used by the
    /// rendering pipeline in M11.
    pub fn set_sprite_zero_hit(&mut self, on: bool) {
        if on {
            self.ppustatus |= STATUS_SPRITE_ZERO;
        } else {
            self.ppustatus &= !STATUS_SPRITE_ZERO;
        }
    }

    /// Set the sprite overflow flag (bit 5 of PPUSTATUS). Used by the
    /// rendering pipeline in M11.
    pub fn set_sprite_overflow(&mut self, on: bool) {
        if on {
            self.ppustatus |= STATUS_OVERFLOW;
        } else {
            self.ppustatus &= !STATUS_OVERFLOW;
        }
    }

    /// True if VBlank NMI is enabled (PPUCTRL bit 7 set).
    pub fn nmi_enabled(&self) -> bool {
        (self.ppuctrl & CTRL_NMI) != 0
    }

    /// True if the VBlank flag is currently set.
    pub fn in_vblank(&self) -> bool {
        (self.ppustatus & STATUS_VBLANK) != 0
    }

    /// True if the sprite 0 hit flag (PPUSTATUS bit 6) is set.
    /// Set by [`Ppu::render_sprites`] when sprite 0 overlaps an opaque
    /// background pixel; cleared at the prerender scanline by
    /// [`Ppu::step`] (and at the start of [`Ppu::render_sprites`]).
    /// Not cleared by PPUSTATUS reads (only VBlank is).
    /// See: https://www.nesdev.org/wiki/PPU_OAM#Sprite_zero_hit
    pub fn sprite_zero_hit(&self) -> bool {
        (self.ppustatus & STATUS_SPRITE_ZERO) != 0
    }

    /// True if the sprite overflow flag (PPUSTATUS bit 5) is set.
    /// Set by [`Ppu::render_sprites`] when more than 8 sprites are in
    /// range on any scanline; cleared at the prerender scanline by
    /// [`Ppu::step`] (and at the start of [`Ppu::render_sprites`]).
    /// Not cleared by PPUSTATUS reads.
    /// See: https://www.nesdev.org/wiki/PPU_OAM#Sprite_overflow
    pub fn sprite_overflow(&self) -> bool {
        (self.ppustatus & STATUS_OVERFLOW) != 0
    }

    // =================================================================
    //  Scanline / cycle stepper (M10)
    // =================================================================

    /// Current scanline within the frame (`0..=261`; 261 is the prerender
    /// scanline).
    pub fn scanline(&self) -> u16 {
        self.scanline
    }

    /// Current PPU cycle within the scanline (`0..=340`).
    pub fn cycle(&self) -> u16 {
        self.cycle
    }

    /// Consume and return the pending NMI request. The emulator main loop
    /// (M12) calls this after each `step` batch and raises
    /// `Cpu::nmi_pending` when it returns `true`.
    pub fn take_nmi_request(&mut self) -> bool {
        let r = self.nmi_request;
        self.nmi_request = false;
        r
    }

    /// Advance the PPU by one cycle, returning `true` if an NMI should be
    /// raised this cycle (VBlank just started and PPUCTRL bit 7 is set).
    ///
    /// The PPU runs 3 cycles per CPU cycle; the main loop (M12) calls this
    /// 3× per `Cpu::step`. This method handles:
    ///
    /// - **VBlank assertion** at scanline 241, cycle 1: sets the VBlank
    ///   flag (PPUSTATUS bit 7) and, if PPUCTRL bit 7 is set, latches
    ///   `nmi_request`.
    /// - **VBlank clearing** at the prerender scanline (261), cycle 1:
    ///   clears VBlank, sprite-overflow, and sprite-0-hit flags.
    /// - **Scroll increments** during visible scanlines (0-239) when
    ///   rendering is enabled (PPUMASK show-bg or show-sprites):
    ///   - cycles 8, 16, ..., 248: horizontal scroll increment (fine X →
    ///     coarse X → nametable horizontal wrap).
    ///   - cycle 256: vertical scroll increment (fine Y → coarse Y →
    ///     nametable vertical wrap).
    ///   - cycle 257: copy horizontal bits of `t` into `v` (coarse X +
    ///     nametable bits 0-1).
    /// - **Vertical `t→v` copy** at the prerender scanline (261), cycles
    ///   280-304: copies coarse Y, fine Y, and nametable bits from `t`
    ///   into `v`.
    ///
    /// See: https://www.nesdev.org/wiki/PPU_rendering#Timing
    /// See: https://www.nesdev.org/wiki/PPU_scrolling
    pub fn step(&mut self) -> bool {
        let mut nmi = false;

        // ---- Advance the cycle counter first ----
        //
        // Events are described in nesdev terms as happening at "dot N"
        // (1-indexed within the scanline). We 0-index the counter but fire
        // events *after* advancing, so "dot 1" = counter reads 1 after one
        // step into the scanline, "dot 256" = counter reads 256, etc.
        self.cycle += 1;
        if self.cycle >= CYCLES_PER_SCANLINE {
            self.cycle = 0;
            self.scanline += 1;
            if self.scanline >= SCANLINES_PER_FRAME {
                self.scanline = 0;
            }
        }

        // ---- Per-cycle events at the new (cycle, scanline) ----
        match self.scanline {
            SCANLINE_VBLANK_START if self.cycle == VBLANK_NMI_CYCLE => {
                self.set_vblank(true);
                if self.nmi_enabled() {
                    self.nmi_request = true;
                    nmi = true;
                }
            }
            SCANLINE_PRERENDER if self.cycle == VBLANK_NMI_CYCLE => {
                // Prerender: clear VBlank and the per-frame status flags.
                self.set_vblank(false);
                self.set_sprite_overflow(false);
                self.set_sprite_zero_hit(false);
            }
            _ => {}
        }

        // ---- Rendering-only scroll updates ----
        //
        // Per NESdev, the scroll increments and t→v copies fire when
        // rendering is enabled (PPUMASK show-bg or show-sprites). The
        // prerender scanline (261) performs the same h/v scroll increments
        // as a visible scanline (no pixels are output, but the scroll
        // state is updated). The horizontal t→v copy at dot 257 fires on
        // every scanline; the vertical t→v copy at dots 280-304 fires only
        // on the prerender scanline.
        // See: https://www.nesdev.org/wiki/PPU_scrolling#During_rendering
        let rendering = (self.ppumask & (MASK_SHOW_BG | MASK_SHOW_SPRITES)) != 0;
        if rendering {
            let does_scroll_inc =
                self.scanline < SCREEN_HEIGHT as u16 || self.scanline == SCANLINE_PRERENDER;
            // Horizontal scroll increment at dots 8,16,...,248.
            if does_scroll_inc
                && self.cycle >= H_SCROLL_INC_STEP
                && self.cycle <= H_SCROLL_INC_LAST
                && (self.cycle % H_SCROLL_INC_STEP) == 0
            {
                self.increment_h_scroll();
            }
            // Vertical scroll increment at dot 256.
            if does_scroll_inc && self.cycle == VERT_SCROLL_INC_CYCLE {
                self.increment_v_scroll();
            }
            // Horizontal t→v copy at dot 257 — every scanline.
            if self.cycle == H_COPY_CYCLE {
                self.copy_h_t_to_v();
            }
            // Vertical t→v copy at prerender dots 280-304.
            if self.scanline == SCANLINE_PRERENDER
                && self.cycle >= V_COPY_CYCLE_START
                && self.cycle <= V_COPY_CYCLE_END
            {
                self.copy_v_t_to_v();
            }
        }

        nmi
    }

    /// Horizontal scroll increment: advance `fine_x`; on wrap, advance
    /// coarse X (in `v`); on coarse-X wrap (past 31), toggle the
    /// horizontal nametable bit.
    ///
    /// See: https://www.nesdev.org/wiki/PPU_scrolling#Coarse_X_increment
    fn increment_h_scroll(&mut self) {
        if self.fine_x < 7 {
            self.fine_x += 1;
        } else {
            self.fine_x = 0;
            let coarse_x = self.v & COARSE_X_MASK;
            if coarse_x == 31 {
                // Wrap coarse X to 0 and toggle the horizontal nt bit.
                self.v &= !COARSE_X_MASK;
                self.v ^= NT_H_BIT;
            } else {
                self.v = (self.v & !COARSE_X_MASK) | (coarse_x + 1);
            }
        }
    }

    /// Vertical scroll increment: advance fine Y (in `v`); on wrap,
    /// advance coarse Y; on coarse-Y wrap past 29, reset coarse Y to 0
    /// and toggle the vertical nametable bit; on wrap past 31 (within the
    /// same nametable's attribute region), just reset coarse Y.
    ///
    /// See: https://www.nesdev.org/wiki/PPU_scrolling#Y_increment
    fn increment_v_scroll(&mut self) {
        let fine_y = (self.v & FINE_Y_MASK) >> 12;
        if fine_y < 7 {
            self.v = (self.v & !FINE_Y_MASK) | ((fine_y + 1) << 12);
        } else {
            self.v &= !FINE_Y_MASK; // fine Y → 0
            let coarse_y = (self.v & COARSE_Y_MASK) >> 5;
            if coarse_y == 29 {
                // Wrap coarse Y to 0 and toggle the vertical nt bit.
                self.v &= !COARSE_Y_MASK;
                self.v ^= NT_V_BIT;
            } else if coarse_y == 31 {
                // Coarse Y wraps within the nametable (no nt toggle).
                self.v &= !COARSE_Y_MASK;
            } else {
                self.v = (self.v & !COARSE_Y_MASK) | ((coarse_y + 1) << 5);
            }
        }
    }

    /// Copy the horizontal components of `t` (coarse X + nametable bits
    /// 0-1) into `v`. Happens at cycle 257 of every visible scanline.
    ///
    /// See: https://www.nesdev.org/wiki/PPU_scrolling#At_cycle_257
    fn copy_h_t_to_v(&mut self) {
        // Copy t bits 0-4 (coarse X) and 10-11 (nt) into v.
        let h_bits = self.t & (COARSE_X_MASK | NT_SELECT_MASK);
        self.v = (self.v & !(COARSE_X_MASK | NT_SELECT_MASK)) | h_bits;
    }

    /// Copy the vertical components of `t` (coarse Y, fine Y, nametable
    /// bits 0-1) into `v`. Happens at prerender scanline cycles 280-304.
    ///
    /// See: https://www.nesdev.org/wiki/PPU_scrolling#At_cycle_280_to_304
    fn copy_v_t_to_v(&mut self) {
        // Copy t bits 5-9 (coarse Y), 10-11 (nt), 12-14 (fine Y) into v.
        let v_bits = self.t & (COARSE_Y_MASK | NT_SELECT_MASK | FINE_Y_MASK);
        self.v = (self.v & !(COARSE_Y_MASK | NT_SELECT_MASK | FINE_Y_MASK)) | v_bits;
    }

    // =================================================================
    //  Internal address mapping
    // =================================================================

    /// Map a nametable address (`$2000-$3EFF`) to a VRAM byte index.
    ///
    /// `$3000-$3EFF` mirrors `$2000-$2EFF`. The four nametable slots
    /// (`$2000`, `$2400`, `$2800`, `$2C00`) are collapsed according to the
    /// cartridge's mirroring mode.
    ///
    /// See: https://www.nesdev.org/wiki/Mirroring
    fn map_nametable(&self, addr: u16) -> usize {
        // Collapse $3000-$3EFF onto $2000-$2EFF.
        let addr = addr & 0x2FFF;
        let local = (addr - 0x2000) as usize; // 0..0xFFF
        let nt = local >> 10; // nametable index 0..3
        let offset = local & 0x3FF; // byte within nametable
        let phys = match self.mirroring {
            Mirroring::Horizontal => nt >> 1, // NT 0,1 → 0; NT 2,3 → 1
            Mirroring::Vertical => nt & 1,    // NT 0,2 → 0; NT 1,3 → 1
            Mirroring::FourScreen => nt,      // all four unique
            Mirroring::SingleScreen => 0,     // all map to NT 0
        };
        phys * 0x400 + offset
    }

    /// Map a palette address (`$3F00-$3FFF`) to a palette RAM index (0..31).
    ///
    /// `$3F20-$3FFF` mirrors `$3F00-$3F1F` (mask with `0x1F`). Within the
    /// 32-byte palette, `$3F10/$3F14/$3F18/$3F1C` are mirrors of
    /// `$3F00/$3F04/$3F08/$3F0C` (sprite color 0 mirrors background color 0).
    ///
    /// See: https://www.nesdev.org/wiki/PPU_palettes
    fn map_palette(&self, addr: u16) -> usize {
        let a = (addr & 0x1F) as u8;
        // $10, $14, $18, $1C mirror $00, $04, $08, $0C.
        if (a & 0x13) == 0x10 {
            (a & 0x0F) as usize
        } else {
            a as usize
        }
    }

    // =================================================================
    //  Test / debug accessors
    // =================================================================

    /// Current PPUCTRL value.
    pub fn ppuctrl(&self) -> u8 {
        self.ppuctrl
    }
    /// Current PPUMASK value.
    pub fn ppumask(&self) -> u8 {
        self.ppumask
    }
    /// Current PPUSTATUS byte (raw, without open-bus fill).
    pub fn ppustatus(&self) -> u8 {
        self.ppustatus
    }
    /// Current OAMADDR value.
    pub fn oamaddr(&self) -> u8 {
        self.oamaddr
    }
    /// Temporary VRAM address (`t` register).
    pub fn temp_vram_addr(&self) -> u16 {
        self.t
    }
    /// Fine X scroll (3 bits).
    pub fn fine_x(&self) -> u8 {
        self.fine_x
    }
    /// Shared write latch (`w`).
    pub fn write_latch(&self) -> bool {
        self.w
    }
    /// Borrow the OAM array.
    pub fn oam(&self) -> &[u8; OAM_SIZE] {
        &self.oam
    }
    /// Mutably borrow the OAM array.
    pub fn oam_mut(&mut self) -> &mut [u8; OAM_SIZE] {
        &mut self.oam
    }
    /// Borrow the palette array.
    pub fn palette(&self) -> &[u8; PALETTE_SIZE] {
        &self.palette
    }
    /// Borrow the nametable VRAM array.
    pub fn vram(&self) -> &[u8; VRAM_SIZE] {
        &self.vram
    }
    /// Current mirroring mode.
    pub fn mirroring(&self) -> Mirroring {
        self.mirroring
    }
    /// Borrow the output framebuffer (256×240 ARGB).
    pub fn framebuffer(&self) -> &[u32] {
        &self.framebuffer
    }
    /// Mutably borrow the output framebuffer.
    pub fn framebuffer_mut(&mut self) -> &mut [u32] {
        &mut self.framebuffer
    }
    /// Borrow the per-pixel background pattern buffer (0-3 per pixel).
    pub fn bg_pattern(&self) -> &[u8] {
        &self.bg_pattern
    }
    /// Screen dimensions (width, height) in pixels.
    pub fn screen_size() -> (usize, usize) {
        (SCREEN_WIDTH, SCREEN_HEIGHT)
    }
}

impl Default for Ppu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- PPUSTATUS read side-effects ---------------------------------

    #[test]
    fn status_read_clears_vblank() {
        let mut ppu = Ppu::new();
        ppu.set_vblank(true);
        assert!(ppu.in_vblank());
        let r = ppu.read_status();
        assert_eq!(r & 0x80, 0x80, "VBlank bit visible in read");
        assert!(!ppu.in_vblank(), "VBlank cleared after read");
    }

    #[test]
    fn status_read_resets_write_latch() {
        let mut ppu = Ppu::new();
        ppu.write_register(6, 0x20); // first PPUADDR write → w = true
        assert!(ppu.write_latch());
        ppu.read_status();
        assert!(!ppu.write_latch(), "w reset by PPUSTATUS read");
    }

    // ---- PPUADDR two-write sequence ----------------------------------

    #[test]
    fn ppuaddr_two_writes_set_v() {
        let mut ppu = Ppu::new();
        ppu.write_register(6, 0x21); // hi
        assert!(ppu.write_latch());
        assert_eq!(ppu.temp_vram_addr() & 0xFF00, 0x2100);
        ppu.write_register(6, 0x08); // lo → v = $2108
        assert!(!ppu.write_latch());
        assert_eq!(ppu.vram_addr(), 0x2108);
    }

    #[test]
    fn ppuaddr_high_byte_masks_to_14_bits() {
        let mut ppu = Ppu::new();
        ppu.write_register(6, 0xFF); // bit 6+ should be dropped
        ppu.write_register(6, 0x00);
        assert_eq!(ppu.vram_addr(), 0x3F00, "0xFF masked to 0x3F");
    }

    // ---- PPUSCROLL two-write sequence --------------------------------

    #[test]
    fn ppuscroll_two_writes_set_t_and_fine_x() {
        let mut ppu = Ppu::new();
        // First write: value 0x7D → coarse X = 0x0F (bits 3-7), fine X = 5
        ppu.write_register(5, 0x7D);
        assert_eq!(ppu.fine_x(), 5);
        assert_eq!(ppu.temp_vram_addr() & 0x1F, 0x0F);
        assert!(ppu.write_latch());
        // Second write: value 0x1B → coarse Y = 3 (bits 3-7), fine Y = 3
        ppu.write_register(5, 0x1B);
        assert!(!ppu.write_latch());
        let t = ppu.temp_vram_addr();
        assert_eq!(t & 0x1F, 0x0F, "coarse X preserved");
        assert_eq!((t >> 5) & 0x1F, 3, "coarse Y = 3");
        assert_eq!((t >> 12) & 0x07, 3, "fine Y = 3");
    }

    // ---- PPUDATA increment -------------------------------------------

    #[test]
    fn vram_increment_default_is_1() {
        let ppu = Ppu::new();
        assert_eq!(ppu.vram_increment(), 1);
    }

    #[test]
    fn vram_increment_32_when_ctrl_bit_set() {
        let mut ppu = Ppu::new();
        ppu.write_register(0, 0x04); // PPUCTRL bit 2
        assert_eq!(ppu.vram_increment(), 32);
    }

    #[test]
    fn advance_vram_wraps_at_3fff() {
        let mut ppu = Ppu::new();
        // Set v = 0x3FFF via PPUADDR writes.
        ppu.write_register(6, 0x3F);
        ppu.write_register(6, 0xFF);
        assert_eq!(ppu.vram_addr(), 0x3FFF);
        ppu.advance_vram_addr();
        assert_eq!(ppu.vram_addr(), 0x0000, "wraps to 0 with increment 1");
    }

    // ---- Nametable mirroring -----------------------------------------

    #[test]
    fn horizontal_mirroring_nt0_nt1_share_vram() {
        let mut ppu = Ppu::new();
        ppu.set_mirroring(Mirroring::Horizontal);
        ppu.write_nametable(0x2000, 0x11); // NT0
        assert_eq!(ppu.read_nametable(0x2400), 0x11, "NT1 mirrors NT0");
        assert_eq!(ppu.read_nametable(0x2000), 0x11);
        ppu.write_nametable(0x2800, 0x22); // NT2
        assert_eq!(ppu.read_nametable(0x2C00), 0x22, "NT3 mirrors NT2");
    }

    #[test]
    fn vertical_mirroring_nt0_nt2_share_vram() {
        let mut ppu = Ppu::new();
        ppu.set_mirroring(Mirroring::Vertical);
        ppu.write_nametable(0x2000, 0x33); // NT0
        assert_eq!(ppu.read_nametable(0x2800), 0x33, "NT2 mirrors NT0");
        ppu.write_nametable(0x2400, 0x44); // NT1
        assert_eq!(ppu.read_nametable(0x2C00), 0x44, "NT3 mirrors NT1");
    }

    #[test]
    fn four_screen_mirroring_all_unique() {
        let mut ppu = Ppu::new();
        ppu.set_mirroring(Mirroring::FourScreen);
        ppu.write_nametable(0x2000, 0xAA);
        ppu.write_nametable(0x2400, 0xBB);
        ppu.write_nametable(0x2800, 0xCC);
        ppu.write_nametable(0x2C00, 0xDD);
        assert_eq!(ppu.read_nametable(0x2000), 0xAA);
        assert_eq!(ppu.read_nametable(0x2400), 0xBB);
        assert_eq!(ppu.read_nametable(0x2800), 0xCC);
        assert_eq!(ppu.read_nametable(0x2C00), 0xDD);
    }

    #[test]
    fn nametable_3000_mirror_of_2000() {
        let mut ppu = Ppu::new();
        ppu.set_mirroring(Mirroring::Horizontal);
        ppu.write_nametable(0x2000, 0x55);
        assert_eq!(ppu.read_nametable(0x3000), 0x55, "$3000 mirrors $2000");
        assert_eq!(ppu.read_nametable(0x3EFF), ppu.read_nametable(0x2EFF));
    }

    // ---- Palette mirroring -------------------------------------------

    #[test]
    fn palette_3f10_mirrors_3f00() {
        let mut ppu = Ppu::new();
        ppu.write_palette(0x3F00, 0x0F);
        assert_eq!(ppu.read_palette(0x3F10), 0x0F);
        assert_eq!(ppu.read_palette(0x3F14), ppu.read_palette(0x3F04));
        assert_eq!(ppu.read_palette(0x3F18), ppu.read_palette(0x3F08));
        assert_eq!(ppu.read_palette(0x3F1C), ppu.read_palette(0x3F0C));
    }

    #[test]
    fn palette_3f20_mirrors_3f00() {
        let mut ppu = Ppu::new();
        ppu.write_palette(0x3F00, 0x21);
        assert_eq!(ppu.read_palette(0x3F20), 0x21);
        assert_eq!(ppu.read_palette(0x3FFF), ppu.read_palette(0x3F1F));
    }

    #[test]
    fn palette_3f11_is_not_mirror_of_3f01() {
        let mut ppu = Ppu::new();
        ppu.write_palette(0x3F01, 0x11);
        ppu.write_palette(0x3F11, 0x22);
        assert_eq!(ppu.read_palette(0x3F01), 0x11);
        assert_eq!(ppu.read_palette(0x3F11), 0x22);
    }

    // ---- OAMDATA ------------------------------------------------------

    #[test]
    fn oamdata_write_increments_oamaddr() {
        let mut ppu = Ppu::new();
        ppu.write_register(3, 0x00); // OAMADDR = 0
        ppu.write_register(4, 0xAA);
        ppu.write_register(4, 0xBB);
        assert_eq!(ppu.oam()[0], 0xAA);
        assert_eq!(ppu.oam()[1], 0xBB);
        assert_eq!(ppu.oamaddr(), 2);
    }

    #[test]
    fn oamdata_read_increments_oamaddr() {
        let mut ppu = Ppu::new();
        ppu.write_register(3, 0x10);
        ppu.write_register(4, 0xCC); // OAM[0x10] = 0xCC, OAMADDR → 0x11
        ppu.write_register(3, 0x10); // reset to 0x10
        let v = ppu.read_register(4); // read OAM[0x10]
        assert_eq!(v, 0xCC);
        assert_eq!(ppu.oamaddr(), 0x11);
    }

    // ---- OAM DMA ------------------------------------------------------

    #[test]
    fn oam_dma_copies_256_bytes() {
        let mut ppu = Ppu::new();
        let mut data = [0u8; 256];
        for (i, slot) in data.iter_mut().enumerate() {
            *slot = (i as u8).wrapping_add(0x40);
        }
        ppu.oam_dma(&data);
        for (i, &byte) in ppu.oam().iter().enumerate() {
            assert_eq!(byte, data[i]);
        }
        assert_eq!(ppu.oamaddr(), 0, "OAMADDR reset to 0 after DMA");
    }

    // ---- NMI enable ---------------------------------------------------

    #[test]
    fn nmi_enabled_reflects_ppuctrl_bit7() {
        let mut ppu = Ppu::new();
        assert!(!ppu.nmi_enabled());
        ppu.write_register(0, 0x80);
        assert!(ppu.nmi_enabled());
        ppu.write_register(0, 0x00);
        assert!(!ppu.nmi_enabled());
    }

    // ---- Open bus -----------------------------------------------------

    #[test]
    fn write_only_register_read_returns_open_bus() {
        let mut ppu = Ppu::new();
        ppu.write_register(0, 0xA5); // PPUCTRL write
        let r = ppu.read_register(0); // PPUCTRL is write-only → open bus
        assert_eq!(r, 0xA5);
    }

    #[test]
    fn status_read_fills_low_bits_from_open_bus() {
        let mut ppu = Ppu::new();
        ppu.write_register(0, 0x1F); // open bus = 0x1F
        ppu.set_vblank(true);
        let r = ppu.read_status();
        assert_eq!(r, 0x80 | 0x1F); // VBlank + open bus low 5 bits
    }

    // ---- M10: PPUCTRL → t nametable-select copy ----------------------

    #[test]
    fn ppuctrl_write_copies_base_nt_into_t() {
        let mut ppu = Ppu::new();
        // t starts at 0; writing PPUCTRL with base NT = 0b10 should set
        // t bits 10-11 to 0b10 (= 0x800).
        ppu.write_register(0, 0b0000_0010);
        let t = ppu.temp_vram_addr();
        assert_eq!(t & NT_SELECT_MASK, 0b10 << 10, "t nt bits = 0b10");
        // Other t bits should be untouched (still 0 here).
        assert_eq!(t & !NT_SELECT_MASK, 0);
    }

    #[test]
    fn ppuctrl_write_preserves_other_t_bits() {
        let mut ppu = Ppu::new();
        // Set coarse X = 5 and fine Y = 3 via PPUSCROLL/PPUADDR first.
        ppu.write_register(5, 0x28); // coarse X = 5, fine X = 0
        ppu.write_register(5, 0x18); // coarse Y = 3, fine Y = 0
        let t_before = ppu.temp_vram_addr();
        // Now write PPUCTRL with base NT = 0b01.
        ppu.write_register(0, 0b0000_0001);
        let t_after = ppu.temp_vram_addr();
        // nt bits changed to 0b01; coarse X / coarse Y preserved.
        assert_eq!(t_after & NT_SELECT_MASK, 0b01 << 10);
        assert_eq!(t_after & COARSE_X_MASK, t_before & COARSE_X_MASK);
        assert_eq!(t_after & COARSE_Y_MASK, t_before & COARSE_Y_MASK);
    }

    // ---- M10: step() cycle / scanline advance -------------------------

    #[test]
    fn step_advances_cycle() {
        let mut ppu = Ppu::new();
        assert_eq!(ppu.scanline(), 0);
        assert_eq!(ppu.cycle(), 0);
        ppu.step();
        assert_eq!(ppu.cycle(), 1);
        assert_eq!(ppu.scanline(), 0);
    }

    #[test]
    fn step_wraps_cycle_to_next_scanline() {
        let mut ppu = Ppu::new();
        // Advance to the last cycle of scanline 0.
        for _ in 0..(CYCLES_PER_SCANLINE - 1) {
            ppu.step();
        }
        assert_eq!(ppu.cycle(), CYCLES_PER_SCANLINE - 1);
        assert_eq!(ppu.scanline(), 0);
        ppu.step();
        assert_eq!(ppu.cycle(), 0);
        assert_eq!(ppu.scanline(), 1);
    }

    #[test]
    fn step_wraps_scanline_to_zero_after_prerender() {
        let mut ppu = Ppu::new();
        // Advance to the last cycle of the prerender scanline (261).
        let total: u32 = (SCANLINES_PER_FRAME as u32) * (CYCLES_PER_SCANLINE as u32);
        for _ in 0..(total - 1) {
            ppu.step();
        }
        assert_eq!(ppu.scanline(), SCANLINE_PRERENDER);
        assert_eq!(ppu.cycle(), CYCLES_PER_SCANLINE - 1);
        ppu.step();
        assert_eq!(ppu.scanline(), 0);
        assert_eq!(ppu.cycle(), 0);
    }

    // ---- M10: VBlank NMI timing ---------------------------------------

    #[test]
    fn vblank_set_at_scanline_241_cycle_1() {
        let mut ppu = Ppu::new();
        ppu.write_register(0, 0x00); // NMI disabled
                                     // Advance to scanline 241, cycle 0.
        let pre: u32 = (SCANLINE_VBLANK_START as u32) * (CYCLES_PER_SCANLINE as u32);
        for _ in 0..pre {
            ppu.step();
        }
        assert_eq!(ppu.scanline(), SCANLINE_VBLANK_START);
        assert_eq!(ppu.cycle(), 0);
        assert!(!ppu.in_vblank());
        // One more step → cycle 1 → VBlank asserted.
        let nmi = ppu.step();
        assert_eq!(ppu.cycle(), 1);
        assert!(ppu.in_vblank(), "VBlank set at scanline 241 cycle 1");
        assert!(!nmi, "no NMI when PPUCTRL bit 7 clear");
    }

    #[test]
    fn nmi_requested_when_vblank_starts_and_nmi_enabled() {
        let mut ppu = Ppu::new();
        ppu.write_register(0, 0x80); // NMI enabled
        let pre: u32 = (SCANLINE_VBLANK_START as u32) * (CYCLES_PER_SCANLINE as u32);
        for _ in 0..pre {
            ppu.step();
        }
        let nmi = ppu.step();
        assert!(nmi, "step returns true when NMI requested");
        assert!(ppu.take_nmi_request(), "nmi_request latched");
        assert!(!ppu.take_nmi_request(), "nmi_request cleared after take");
    }

    #[test]
    fn nmi_not_requested_when_nmi_disabled() {
        let mut ppu = Ppu::new();
        ppu.write_register(0, 0x00); // NMI disabled
        let pre: u32 = (SCANLINE_VBLANK_START as u32) * (CYCLES_PER_SCANLINE as u32);
        for _ in 0..pre {
            ppu.step();
        }
        let nmi = ppu.step();
        assert!(!nmi);
        assert!(!ppu.take_nmi_request());
        // VBlank flag is still set even though NMI is disabled.
        assert!(ppu.in_vblank());
    }

    #[test]
    fn vblank_cleared_at_prerender_cycle_1() {
        let mut ppu = Ppu::new();
        ppu.write_register(0, 0x00);
        // Advance into VBlank.
        let to_vblank: u32 = (SCANLINE_VBLANK_START as u32) * (CYCLES_PER_SCANLINE as u32) + 5;
        for _ in 0..to_vblank {
            ppu.step();
        }
        assert!(ppu.in_vblank());
        // Advance to prerender scanline, cycle 0.
        let to_prerender: u32 =
            (SCANLINE_PRERENDER as u32) * (CYCLES_PER_SCANLINE as u32) - to_vblank;
        for _ in 0..to_prerender {
            ppu.step();
        }
        assert_eq!(ppu.scanline(), SCANLINE_PRERENDER);
        assert_eq!(ppu.cycle(), 0);
        assert!(
            ppu.in_vblank(),
            "still in VBlank just before prerender cycle 1"
        );
        ppu.step();
        assert!(!ppu.in_vblank(), "VBlank cleared at prerender cycle 1");
    }

    #[test]
    fn sprite_flags_cleared_at_prerender_cycle_1() {
        let mut ppu = Ppu::new();
        ppu.set_sprite_overflow(true);
        ppu.set_sprite_zero_hit(true);
        // Advance to prerender scanline, cycle 0.
        let pre: u32 = (SCANLINE_PRERENDER as u32) * (CYCLES_PER_SCANLINE as u32);
        for _ in 0..pre {
            ppu.step();
        }
        assert_eq!(ppu.scanline(), SCANLINE_PRERENDER);
        assert_eq!(ppu.cycle(), 0);
        // Flags still set just before cycle 1.
        assert_eq!(ppu.ppustatus() & 0b0110_0000, 0b0110_0000);
        ppu.step();
        assert_eq!(ppu.ppustatus() & 0b0110_0000, 0, "sprite flags cleared");
    }

    #[test]
    fn one_nmi_per_frame_even_if_vblank_flag_lingers() {
        let mut ppu = Ppu::new();
        ppu.write_register(0, 0x80);
        // Run a full frame + 1 cycle into the next frame's VBlank.
        let one_frame: u32 = (SCANLINES_PER_FRAME as u32) * (CYCLES_PER_SCANLINE as u32);
        let mut nmi_count = 0u32;
        for _ in 0..(one_frame + (SCANLINE_VBLANK_START as u32) * (CYCLES_PER_SCANLINE as u32) + 2)
        {
            if ppu.step() {
                nmi_count += 1;
            }
        }
        // Exactly 2 NMIs: one at the first frame's scanline 241, one at the
        // second frame's scanline 241.
        assert_eq!(nmi_count, 2);
    }

    // ---- M10: scroll increments during rendering ----------------------

    #[test]
    fn h_scroll_increment_advances_fine_x() {
        let mut ppu = Ppu::new();
        // Enable rendering so step() performs scroll increments.
        ppu.write_register(1, MASK_SHOW_BG);
        // Advance to scanline 0, cycle 8 (first h-scroll increment).
        for _ in 0..8 {
            ppu.step();
        }
        assert_eq!(ppu.cycle(), 8);
        // fine_x should have advanced from 0 to 1.
        assert_eq!(ppu.fine_x(), 1);
    }

    #[test]
    fn h_scroll_increment_wraps_coarse_x_and_nt_bit() {
        let mut ppu = Ppu::new();
        ppu.write_register(1, MASK_SHOW_BG);
        // Set fine_x = 7 so the next increment wraps to coarse X.
        // Use PPUSCROLL first write: value 0x07 → coarse X = 0, fine X = 7.
        ppu.write_register(5, 0x07);
        ppu.write_register(5, 0x00);
        // Set v's coarse X = 31 via PPUADDR (so the wrap toggles nt bit 0).
        // v = 0x001F (coarse X = 31, nt = 0).
        ppu.write_register(6, 0x00);
        ppu.write_register(6, 0x1F);
        assert_eq!(ppu.vram_addr(), 0x001F);
        // Advance to cycle 8 of scanline 0 (first h-scroll increment).
        for _ in 0..8 {
            ppu.step();
        }
        // fine_x wrapped 7 → 0, coarse X wrapped 31 → 0, nt bit 0 toggled.
        assert_eq!(ppu.fine_x(), 0);
        assert_eq!(ppu.vram_addr() & COARSE_X_MASK, 0);
        assert_eq!(ppu.vram_addr() & NT_H_BIT, NT_H_BIT, "nt H bit toggled");
    }

    #[test]
    fn v_scroll_increment_at_cycle_256_advances_fine_y() {
        let mut ppu = Ppu::new();
        ppu.write_register(1, MASK_SHOW_BG);
        // Advance to scanline 0, cycle 256.
        for _ in 0..256 {
            ppu.step();
        }
        assert_eq!(ppu.cycle(), 256);
        // fine Y (v bits 12-14) should have advanced from 0 to 1.
        let fine_y = (ppu.vram_addr() & FINE_Y_MASK) >> 12;
        assert_eq!(fine_y, 1);
    }

    #[test]
    fn v_scroll_increment_wraps_coarse_y_and_nt_bit() {
        let mut ppu = Ppu::new();
        ppu.write_register(1, MASK_SHOW_BG);
        // Set v directly: fine Y = 7, coarse Y = 29, nt = 0. (PPUADDR
        // can't set fine Y bit 14 due to its 0x3F hi-byte mask, so we
        // poke v directly — tests live in the same module.)
        ppu.v = (7u16 << 12) | (29u16 << 5);
        assert_eq!((ppu.vram_addr() & COARSE_Y_MASK) >> 5, 29);
        assert_eq!((ppu.vram_addr() & FINE_Y_MASK) >> 12, 7);
        // Advance to cycle 256 of scanline 0.
        for _ in 0..256 {
            ppu.step();
        }
        // fine Y wrapped 7 → 0, coarse Y wrapped 29 → 0, nt V bit toggled.
        assert_eq!((ppu.vram_addr() & FINE_Y_MASK) >> 12, 0);
        assert_eq!((ppu.vram_addr() & COARSE_Y_MASK) >> 5, 0);
        assert_eq!(ppu.vram_addr() & NT_V_BIT, NT_V_BIT, "nt V bit toggled");
    }

    #[test]
    fn h_t_to_v_copy_at_cycle_257() {
        let mut ppu = Ppu::new();
        ppu.write_register(1, MASK_SHOW_BG);
        // Set t's coarse X = 10, nt = 0b11 via PPUCTRL + PPUSCROLL.
        ppu.write_register(0, 0b11); // nt bits → 0b11
        ppu.write_register(5, 0x50); // coarse X = 10, fine X = 0
        ppu.write_register(5, 0x00); // coarse Y = 0, fine Y = 0
                                     // v starts at 0. Advance to cycle 257 of scanline 0.
        for _ in 0..257 {
            ppu.step();
        }
        assert_eq!(ppu.cycle(), 257);
        // v should now have t's coarse X (10) and nt bits (0b11).
        // (Note: h-scroll increments during cycles 8..248 will have
        // advanced v's coarse X, but the copy at 257 overwrites it.)
        assert_eq!(ppu.vram_addr() & COARSE_X_MASK, 10);
        assert_eq!(ppu.vram_addr() & NT_SELECT_MASK, 0b11 << 10);
    }

    #[test]
    fn v_t_to_v_copy_at_prerender_cycles_280_304() {
        let mut ppu = Ppu::new();
        ppu.write_register(1, MASK_SHOW_BG);
        // Set t's coarse Y = 15, fine Y = 5, nt = 0b10.
        ppu.write_register(0, 0b10); // nt → 0b10
                                     // PPUSCROLL second write: coarse Y = 15, fine Y = 5 → (15<<3)|5 = 0x7D.
        ppu.write_register(5, 0x00);
        ppu.write_register(5, 0x7D);
        // Advance to prerender scanline, cycle 304 (last V-copy cycle).
        let pre: u32 = (SCANLINE_PRERENDER as u32) * (CYCLES_PER_SCANLINE as u32) + 304;
        for _ in 0..pre {
            ppu.step();
        }
        // v should now have t's coarse Y (15), fine Y (5), nt (0b10).
        assert_eq!((ppu.vram_addr() & COARSE_Y_MASK) >> 5, 15);
        assert_eq!((ppu.vram_addr() & FINE_Y_MASK) >> 12, 5);
        assert_eq!(ppu.vram_addr() & NT_SELECT_MASK, 0b10 << 10);
    }

    #[test]
    fn no_scroll_increments_when_rendering_disabled() {
        let mut ppu = Ppu::new();
        // PPUMASK = 0 → rendering disabled.
        // Advance through a full visible scanline.
        for _ in 0..CYCLES_PER_SCANLINE {
            ppu.step();
        }
        // v and fine_x should be unchanged.
        assert_eq!(ppu.vram_addr(), 0);
        assert_eq!(ppu.fine_x(), 0);
    }

    // ---- M10: NMI-during-VBlank quirk --------------------------------

    #[test]
    fn ppuctrl_nmi_enable_during_vblank_latches_nmi() {
        let mut ppu = Ppu::new();
        ppu.write_register(0, 0x00); // NMI disabled
                                     // Force into VBlank.
        ppu.set_vblank(true);
        assert!(ppu.in_vblank());
        assert!(!ppu.take_nmi_request());
        // Enable NMI while VBlank is active → immediate NMI request.
        ppu.write_register(0, 0x80);
        assert!(
            ppu.take_nmi_request(),
            "NMI latched when enabled during VBlank"
        );
    }

    #[test]
    fn ppuctrl_nmi_enable_outside_vblank_no_immediate_nmi() {
        let mut ppu = Ppu::new();
        assert!(!ppu.in_vblank());
        ppu.write_register(0, 0x80);
        assert!(
            !ppu.take_nmi_request(),
            "no NMI when enabled outside VBlank"
        );
    }

    #[test]
    fn ppuctrl_nmi_enable_when_already_enabled_during_vblank_no_duplicate() {
        let mut ppu = Ppu::new();
        ppu.write_register(0, 0x80); // NMI enabled, not in VBlank → no request
        assert!(!ppu.take_nmi_request());
        ppu.set_vblank(true);
        // Writing PPUCTRL again with bit 7 set while in VBlank → request.
        ppu.write_register(0, 0x80);
        assert!(ppu.take_nmi_request());
        // A second write with bit 7 set while still in VBlank → another
        // request (each qualifying write latches).
        ppu.write_register(0, 0x80);
        assert!(ppu.take_nmi_request());
    }

    // ---- M10: prerender scanline scroll increments -------------------

    #[test]
    fn prerender_scanline_performs_h_scroll_increments() {
        let mut ppu = Ppu::new();
        ppu.write_register(1, MASK_SHOW_BG);
        // Advance to prerender scanline, cycle 8 (first h-scroll inc).
        let pre: u32 = (SCANLINE_PRERENDER as u32) * (CYCLES_PER_SCANLINE as u32) + 8;
        for _ in 0..pre {
            ppu.step();
        }
        assert_eq!(ppu.scanline(), SCANLINE_PRERENDER);
        assert_eq!(ppu.cycle(), 8);
        // fine_x should have advanced (from 0 to 1, modulo prior frame
        // drift — just check it's not stuck at 0).
        assert_eq!(ppu.fine_x(), 1, "h-scroll increment fires on prerender");
    }

    #[test]
    fn h_t_to_v_copy_fires_on_every_scanline_when_rendering() {
        let mut ppu = Ppu::new();
        ppu.write_register(1, MASK_SHOW_BG);
        // Set t's coarse X = 20, nt = 0b01.
        ppu.write_register(0, 0b0000_0001);
        ppu.write_register(5, 0xA0); // coarse X = 20, fine X = 0
        ppu.write_register(5, 0x00);
        // Advance to post-render scanline 240, cycle 257 (a non-visible,
        // non-prerender scanline). The H t→v copy should still fire.
        let target: u32 = 240 * (CYCLES_PER_SCANLINE as u32) + 257;
        for _ in 0..target {
            ppu.step();
        }
        assert_eq!(ppu.scanline(), 240);
        assert_eq!(ppu.cycle(), 257);
        // v should have t's coarse X (20) and nt (0b01).
        assert_eq!(ppu.vram_addr() & COARSE_X_MASK, 20);
        assert_eq!(ppu.vram_addr() & NT_SELECT_MASK, 0b01 << 10);
    }

    // ---- M11: PPUSTATUS flag behaviours -------------------------------

    #[test]
    fn ppustatus_read_does_not_clear_sprite_zero_hit() {
        let mut ppu = Ppu::new();
        ppu.set_sprite_zero_hit(true);
        assert!(ppu.sprite_zero_hit());
        let _ = ppu.read_status();
        assert!(
            ppu.sprite_zero_hit(),
            "sprite 0 hit survives PPUSTATUS read"
        );
    }

    #[test]
    fn ppustatus_read_does_not_clear_sprite_overflow() {
        let mut ppu = Ppu::new();
        ppu.set_sprite_overflow(true);
        assert!(ppu.sprite_overflow());
        let _ = ppu.read_status();
        assert!(
            ppu.sprite_overflow(),
            "sprite overflow survives PPUSTATUS read"
        );
    }

    #[test]
    fn ppustatus_read_clears_only_vblank() {
        let mut ppu = Ppu::new();
        ppu.set_vblank(true);
        ppu.set_sprite_zero_hit(true);
        ppu.set_sprite_overflow(true);
        let r = ppu.read_status();
        // All three flags visible in the returned byte.
        assert_eq!(r & 0b1110_0000, 0b1110_0000);
        // Only VBlank cleared; the other two persist.
        assert!(!ppu.in_vblank());
        assert!(ppu.sprite_zero_hit());
        assert!(ppu.sprite_overflow());
    }

    #[test]
    fn prerender_clears_sprite_zero_hit_and_overflow() {
        let mut ppu = Ppu::new();
        ppu.set_sprite_zero_hit(true);
        ppu.set_sprite_overflow(true);
        ppu.set_vblank(true);
        // Advance to prerender scanline 261, cycle 1.
        let target: u32 = (SCANLINE_PRERENDER as u32) * (CYCLES_PER_SCANLINE as u32) + 1;
        for _ in 0..target {
            ppu.step();
        }
        assert_eq!(ppu.scanline(), SCANLINE_PRERENDER);
        assert_eq!(ppu.cycle(), 1);
        assert!(!ppu.in_vblank(), "VBlank cleared at prerender");
        assert!(!ppu.sprite_zero_hit(), "sprite 0 hit cleared at prerender");
        assert!(
            !ppu.sprite_overflow(),
            "sprite overflow cleared at prerender"
        );
    }

    #[test]
    fn sprite_zero_hit_accessor_reflects_flag() {
        let mut ppu = Ppu::new();
        assert!(!ppu.sprite_zero_hit());
        ppu.set_sprite_zero_hit(true);
        assert!(ppu.sprite_zero_hit());
        ppu.set_sprite_zero_hit(false);
        assert!(!ppu.sprite_zero_hit());
    }

    #[test]
    fn sprite_overflow_accessor_reflects_flag() {
        let mut ppu = Ppu::new();
        assert!(!ppu.sprite_overflow());
        ppu.set_sprite_overflow(true);
        assert!(ppu.sprite_overflow());
        ppu.set_sprite_overflow(false);
        assert!(!ppu.sprite_overflow());
    }
}
