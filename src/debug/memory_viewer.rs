//! Memory viewer — hex dump of CPU or PPU address space with edit
//! capability, for console-based debugging.
//!
//! This is the M28 memory inspector deliverable. Like the rest of the
//! debug layer, reads are side-effect-free: CPU-space reads go through
//! [`crate::bus::Bus::peek`] (no PPUSTATUS VBlank clear, no OAMADDR
//! increment, no mapper read side-effects), and PPU-space reads go
//! through the PPU's immutable accessors (`read_nametable`,
//! `read_palette`, `oam()`) and the cartridge's `read_chr` (not
//! `read_chr_latched`, so MMC2 bank latches do not fire).
//!
//! Edits (`poke`) *do* mutate emulator state — that is the whole point
//! of an editor. CPU-space writes go through `Bus::write` (which routes
//! to RAM / PPU registers / APU / cartridge as appropriate), and PPU
//! address-space writes go through the appropriate PPU accessor
//! (`write_nametable`, `write_palette`, or `write_chr` on the
//! cartridge).
//!
//! See: https://www.nesdev.org/wiki/CPU_memory_map
//! See: https://www.nesdev.org/wiki/PPU_memory_layout

use crate::bus::Bus;

/// Number of bytes per row in the hex dump.
const ROW_BYTES: usize = 16;
/// Default number of rows to dump (256 bytes / 16 rows).
const DEFAULT_ROWS: usize = 16;

/// Which address space to inspect.
///
/// `Cpu` is the CPU's 16-bit address space (`$0000..=$FFFF`), routed
/// through the bus. `Ppu` is the PPU's 14-bit address space
/// (`$0000..=$3FFF`): pattern tables (`$0000-$1FFF` via CHR),
/// nametables (`$2000-$2EFF`), and palette RAM (`$3F00-$3FFF`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryRegion {
    /// CPU address space (`$0000..=$FFFF`).
    Cpu,
    /// PPU address space (`$0000..=$3FFF`).
    Ppu,
}

impl MemoryRegion {
    /// Maximum addressable byte for this region.
    fn max(self) -> u16 {
        match self {
            MemoryRegion::Cpu => 0xFFFF,
            MemoryRegion::Ppu => 0x3FFF,
        }
    }

    /// Display label.
    fn label(self) -> &'static str {
        match self {
            MemoryRegion::Cpu => "CPU",
            MemoryRegion::Ppu => "PPU",
        }
    }
}

/// Memory viewer state — which region to inspect, the start address of
/// the current window, and how many rows to display. Held by the main
/// loop; F6 prints the dump, PageUp/PageDown navigate, `[` / `]`
/// switch regions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryViewer {
    /// Address space being inspected.
    pub region: MemoryRegion,
    /// Start address of the current hex-dump window.
    pub addr: u16,
    /// Number of 16-byte rows to display.
    pub rows: u8,
}

impl Default for MemoryViewer {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryViewer {
    /// Construct a viewer on the CPU region at `$0200` (the sprite DMA
    /// page — a useful default for NES debugging).
    pub fn new() -> Self {
        Self {
            region: MemoryRegion::Cpu,
            addr: 0x0200,
            rows: DEFAULT_ROWS as u8,
        }
    }

    /// Switch to the other region (CPU ↔ PPU). The current address is
    /// clamped to the new region's maximum.
    pub fn toggle_region(&mut self) {
        self.region = match self.region {
            MemoryRegion::Cpu => MemoryRegion::Ppu,
            MemoryRegion::Ppu => MemoryRegion::Cpu,
        };
        self.clamp_addr();
    }

    /// Move the window up by one page (16 rows × 16 bytes = 256 bytes).
    pub fn page_up(&mut self) {
        let step = (self.rows as u16) * (ROW_BYTES as u16);
        self.addr = self.addr.saturating_sub(step);
    }

    /// Move the window down by one page. `clamp_addr` pins the window
    /// within the region max.
    pub fn page_down(&mut self) {
        let step = (self.rows as u16) * (ROW_BYTES as u16);
        self.addr = self.addr.saturating_add(step);
        self.clamp_addr();
    }

    /// Jump to a specific start address (clamped to the region max).
    pub fn set_addr(&mut self, addr: u16) {
        self.addr = addr;
        self.clamp_addr();
    }

    /// Ensure `addr` + the window fits within the region.
    fn clamp_addr(&mut self) {
        let max = self.region.max() as u32;
        let window_end = self.addr as u32 + (self.rows as u32) * (ROW_BYTES as u32);
        if window_end > max + 1 {
            // Pin the window so its last byte is at `max`.
            let pinned = max + 1 - (self.rows as u32) * (ROW_BYTES as u32);
            self.addr = pinned as u16;
        }
    }

    /// Read one byte from the current region without side-effects.
    fn read_byte(&self, bus: &Bus, addr: u16) -> u8 {
        match self.region {
            MemoryRegion::Cpu => bus.peek(addr),
            MemoryRegion::Ppu => read_ppu_space(bus, addr),
        }
    }

    /// Render the current window as a hex dump. Each row is:
    /// `$AAAA: BB BB BB ... BB |................|`
    /// (16 bytes, then an ASCII column with `.` for non-printable bytes).
    pub fn dump(&self, bus: &Bus) -> String {
        let mut s = String::with_capacity((self.rows as usize) * 80);
        // Compute the end address in u32 so it cannot overflow, and
        // clamp to the region max (the window may be pinned there by
        // `clamp_addr`).
        let end = (self.addr as u32 + (self.rows as u32) * (ROW_BYTES as u32) - 1)
            .min(self.region.max() as u32);
        s.push_str(&format!(
            "--- memory viewer: {} ${:04X}-${:04X} ---\n",
            self.region.label(),
            self.addr,
            end as u16,
        ));
        for row in 0..self.rows as u16 {
            let row_addr = self.addr.saturating_add(row * ROW_BYTES as u16);
            if row_addr > self.region.max() {
                break;
            }
            s.push_str(&format!("${row_addr:04X}:"));
            let mut ascii = String::with_capacity(ROW_BYTES);
            for col in 0..ROW_BYTES as u16 {
                let addr = row_addr.wrapping_add(col);
                let byte = self.read_byte(bus, addr);
                s.push_str(&format!(" {byte:02X}"));
                ascii.push(if (0x20..=0x7E).contains(&byte) {
                    byte as char
                } else {
                    '.'
                });
            }
            s.push_str(&format!(" |{ascii}|\n"));
        }
        s
    }

    /// Write a single byte at `addr` in the current region. CPU-space
    /// writes go through `Bus::write` (full routing — RAM, PPU
    /// registers, APU, cartridge). PPU-space writes go through the
    /// appropriate PPU accessor:
    /// - `$0000-$1FFF` → `Cartridge::write_chr`
    /// - `$2000-$2EFF` (and `$3000-$3EFF` mirror) → `Ppu::write_nametable`
    /// - `$3F00-$3FFF` → `Ppu::write_palette`
    pub fn poke(&self, bus: &mut Bus, addr: u16, value: u8) {
        match self.region {
            MemoryRegion::Cpu => bus.write(addr, value),
            MemoryRegion::Ppu => write_ppu_space(bus, addr, value),
        }
    }
}

/// Read one byte from PPU address space (`$0000..=$3FFF`) without
/// side-effects. CHR reads use `read_chr` (not `read_chr_latched`) so
/// MMC2 bank latches do not fire.
fn read_ppu_space(bus: &Bus, addr: u16) -> u8 {
    match addr {
        0x0000..=0x1FFF => bus.cartridge().map(|c| c.read_chr(addr)).unwrap_or(0),
        0x2000..=0x3EFF => bus.ppu().read_nametable(addr),
        0x3F00..=0x3FFF => bus.ppu().read_palette(addr),
        // PPU address space is 14-bit; anything above $3FFF is unreachable
        // through the region enum (which caps at $3FFF), but guard anyway.
        _ => 0,
    }
}

/// Write one byte to PPU address space (`$0000..=$3FFF`).
fn write_ppu_space(bus: &mut Bus, addr: u16, value: u8) {
    match addr {
        0x0000..=0x1FFF => {
            if let Some(cart) = bus.cartridge_mut() {
                cart.write_chr(addr, value);
            }
        }
        0x2000..=0x3EFF => bus.ppu_mut().write_nametable(addr, value),
        0x3F00..=0x3FFF => bus.ppu_mut().write_palette(addr, value),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_viewer_is_cpu_at_0200() {
        let v = MemoryViewer::new();
        assert_eq!(v.region, MemoryRegion::Cpu);
        assert_eq!(v.addr, 0x0200);
    }

    #[test]
    fn toggle_region_clamps_addr_to_ppu_max() {
        let mut v = MemoryViewer::new();
        v.set_addr(0xFF00);
        v.toggle_region();
        assert_eq!(v.region, MemoryRegion::Ppu);
        // Address should be clamped so the window fits within $3FFF.
        assert!(v.addr <= 0x3FFF);
    }

    #[test]
    fn page_up_saturates_at_zero() {
        let mut v = MemoryViewer::new();
        v.set_addr(0x0010);
        v.page_up();
        assert_eq!(v.addr, 0);
    }

    #[test]
    fn dump_cpu_region_shows_hex_and_ascii() {
        let mut bus = Bus::new();
        bus.write(0x0200, 0x41);
        bus.write(0x0201, 0x42);
        bus.write(0x0202, 0x43);
        bus.write(0x0203, 0x01); // non-printable → '.'
        let v = MemoryViewer {
            region: MemoryRegion::Cpu,
            addr: 0x0200,
            rows: 1,
        };
        let s = v.dump(&bus);
        assert!(s.contains("$0200:"));
        assert!(s.contains(" 41 42 43 01"));
        // ASCII column is 16 chars; the first 3 are ABC, byte 4 is '.',
        // the rest are '.' (zeroed RAM reads as 0x00 → '.').
        assert!(s.contains("|ABC.............|"));
    }

    #[test]
    fn dump_ppu_region_reads_chr_via_read_chr() {
        let bus = Bus::new();
        let v = MemoryViewer {
            region: MemoryRegion::Ppu,
            addr: 0x0000,
            rows: 1,
        };
        let s = v.dump(&bus);
        assert!(s.contains("$0000:"));
    }

    #[test]
    fn poke_cpu_region_writes_through_bus() {
        let mut bus = Bus::new();
        let v = MemoryViewer {
            region: MemoryRegion::Cpu,
            addr: 0x0200,
            rows: 1,
        };
        v.poke(&mut bus, 0x0200, 0xAB);
        assert_eq!(bus.peek(0x0200), 0xAB);
    }

    #[test]
    fn poke_ppu_region_writes_palette() {
        let mut bus = Bus::new();
        let v = MemoryViewer {
            region: MemoryRegion::Ppu,
            addr: 0x3F00,
            rows: 1,
        };
        v.poke(&mut bus, 0x3F00, 0x21);
        assert_eq!(bus.ppu().read_palette(0x3F00), 0x21);
    }

    #[test]
    fn poke_ppu_region_writes_nametable() {
        let mut bus = Bus::new();
        let v = MemoryViewer {
            region: MemoryRegion::Ppu,
            addr: 0x2000,
            rows: 1,
        };
        v.poke(&mut bus, 0x2000, 0x42);
        assert_eq!(bus.ppu().read_nametable(0x2000), 0x42);
    }

    #[test]
    fn dump_is_side_effect_free_on_ppustatus() {
        let mut bus = Bus::new();
        bus.ppu_mut().set_vblank(true);
        let v = MemoryViewer {
            region: MemoryRegion::Cpu,
            addr: 0x2000,
            rows: 1,
        };
        let _ = v.dump(&bus);
        // PPUSTATUS VBlank should survive a peek-based dump.
        assert!(bus.ppu().in_vblank());
    }
}
