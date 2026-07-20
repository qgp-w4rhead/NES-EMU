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

use crate::mappers::Mirroring;

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
            0 => self.ppuctrl = value,
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
}
