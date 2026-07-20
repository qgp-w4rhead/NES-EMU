//! PPU viewer — text snapshots of nametables, pattern tables, OAM, and
//! palettes for console-based debugging.
//!
//! This is the M28 PPU visualization deliverable. Like the M27 CPU
//! debugger, it is purely a *consumer* of the emulation core: every read
//! goes through side-effect-free accessors (`Ppu::read_nametable`,
//! `Ppu::read_palette`, `Ppu::oam()`, and `Cartridge::read_chr` — *not*
//! `read_chr_latched`, so MMC2 bank latches do not fire), so opening the
//! PPU viewer cannot perturb the emulated machine.
//!
//! The viewer produces a `String` snapshot rather than writing directly to
//! stderr — the caller (`src/main.rs`) decides where to print it. This
//! makes the module unit-testable without capturing stderr.
//!
//! See: https://www.nesdev.org/wiki/PPU_memory_layout
//! See: https://www.nesdev.org/wiki/PPU_OAM
//! See: https://www.nesdev.org/wiki/PPU_palettes

use crate::bus::Bus;
use crate::ppu::render::nes_color_to_argb;

/// Width of a nametable in tiles (32 tiles × 8 px = 256 px).
const NT_TILE_W: usize = 32;
/// Height of a nametable in tiles (30 tiles × 8 px = 240 px).
const NT_TILE_H: usize = 30;
/// Pixels per pattern-table tile side (8).
const TILE_PX: usize = 8;
/// OAM entries (64 sprites).
const OAM_COUNT: usize = 64;

/// Which PPU view to render. Cycled by the F4 hotkey in `src/main.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PpuView {
    /// Four nametables laid out 2×2, each a 32×30 tile-index grid.
    Nametables,
    /// One 16×16-tile pattern table (8×8 pixels each), using the given
    /// CHR bank (0 = `$0000-$0FFF`, 1 = `$1000-$1FFF`).
    PatternTable { bank: u8 },
    /// OAM contents — 64 sprites with Y / tile / attrs / X.
    Oam,
    /// The 32-byte palette RAM, grouped into 8 palettes (4 bg + 4 sprite)
    /// with color indices and ARGB hex.
    Palettes,
}

impl PpuView {
    /// Display label for the view (used in the snapshot header).
    fn label(self) -> &'static str {
        match self {
            PpuView::Nametables => "nametables",
            PpuView::PatternTable { bank } => match bank {
                0 => "pattern table 0 ($0000-$0FFF)",
                1 => "pattern table 1 ($1000-$1FFF)",
                _ => "pattern table (unknown bank)",
            },
            PpuView::Oam => "OAM (64 sprites)",
            PpuView::Palettes => "palettes (32 bytes)",
        }
    }
}

/// PPU viewer state — which view to show and the current pattern-table
/// bank. Held by the main loop; F4 cycles `mode` and (when in pattern
/// mode) the bank.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PpuViewer {
    /// Currently selected view.
    pub mode: PpuView,
    /// Current pattern-table bank (0 or 1) — used when `mode` is
    /// `PatternTable`. Toggled by F4 while in pattern mode.
    pub pattern_bank: u8,
}

impl Default for PpuViewer {
    fn default() -> Self {
        Self::new()
    }
}

impl PpuViewer {
    /// Construct a viewer starting on the nametables view.
    pub fn new() -> Self {
        Self {
            mode: PpuView::Nametables,
            pattern_bank: 0,
        }
    }

    /// Advance to the next view. The cycle order is:
    /// Nametables → PatternTable(bank) → OAM → Palettes → Nametables.
    /// Pressing F4 while on the pattern view toggles the bank first and
    /// only advances to OAM on the *second* press, so the user can flip
    /// between banks 0 and 1 without cycling through the other views.
    ///
    /// Invariant: `pattern_bank` is only meaningful while `mode ==
    /// PatternTable`. Setting it manually while in another mode will
    /// cause the next `cycle()` to enter `PatternTable` at that bank
    /// (then toggle to the other bank on the following press).
    pub fn cycle(&mut self) {
        match self.mode {
            PpuView::Nametables => {
                self.mode = PpuView::PatternTable {
                    bank: self.pattern_bank & 1,
                };
            }
            PpuView::PatternTable { .. } => {
                if self.pattern_bank & 1 == 0 {
                    self.pattern_bank = 1;
                    self.mode = PpuView::PatternTable { bank: 1 };
                } else {
                    self.pattern_bank = 0;
                    self.mode = PpuView::Oam;
                }
            }
            PpuView::Oam => self.mode = PpuView::Palettes,
            PpuView::Palettes => self.mode = PpuView::Nametables,
        }
    }

    /// Render the current view as a text snapshot. All reads are
    /// side-effect-free.
    pub fn render(&self, bus: &Bus) -> String {
        let mut s = String::with_capacity(4096);
        s.push_str(&format!("--- PPU viewer: {} ---\n", self.mode.label()));
        match self.mode {
            PpuView::Nametables => self.render_nametables(bus, &mut s),
            PpuView::PatternTable { bank } => self.render_pattern_table(bus, bank & 1, &mut s),
            PpuView::Oam => self.render_oam(bus, &mut s),
            PpuView::Palettes => self.render_palettes(bus, &mut s),
        }
        s
    }

    // ---- Nametables: 4 tables, 32×30 tile indices each -------------------

    fn render_nametables(&self, bus: &Bus, out: &mut String) {
        let ppu = bus.ppu();
        // Each nametable is rendered as a 32×30 grid of tile indices,
        // stacked vertically (NT0, NT1, NT2, NT3). A 2×2 side-by-side
        // layout would be 130+ chars wide; stacking keeps the snapshot
        // readable in a standard terminal.
        let bases: [u16; 4] = [0x2000, 0x2400, 0x2800, 0x2C00];
        let names: [&str; 4] = ["NT0 ($2000)", "NT1 ($2400)", "NT2 ($2800)", "NT3 ($2C00)"];
        for (nt, (&base, &name)) in bases.iter().zip(names.iter()).enumerate() {
            out.push_str(&format!("--- {name} ---\n"));
            out.push_str("    ");
            for col in 0..NT_TILE_W {
                out.push_str(&format!("{:X}", col & 0xF));
                if col == NT_TILE_W - 1 {
                    out.push('\n');
                } else {
                    out.push(' ');
                }
            }
            for row in 0..NT_TILE_H {
                out.push_str(&format!("{row:02X}: "));
                for col in 0..NT_TILE_W {
                    let addr = base + (row * NT_TILE_W + col) as u16;
                    let tile = ppu.read_nametable(addr);
                    out.push_str(&format!("{tile:02X}"));
                    if col == NT_TILE_W - 1 {
                        out.push('\n');
                    } else {
                        out.push(' ');
                    }
                }
            }
            // Blank line between nametables (skip after the last one).
            if nt < 3 {
                out.push('\n');
            }
        }
        out.push_str(&format!("mirroring: {:?}\n", bus.ppu().mirroring()));
    }

    // ---- Pattern table: 16×16 tiles, 8×8 pixels each ---------------------

    fn render_pattern_table(&self, bus: &Bus, bank: u8, out: &mut String) {
        let chr_base = (bank as u16) << 12; // $0000 or $1000
        let chr = |tile: usize, row: usize| -> [u8; 8] {
            // Each tile is 16 bytes: plane 0 occupies bytes 0-7 (rows
            // 0-7), plane 1 occupies bytes 8-15 (rows 0-7). Each row is
            // one byte per plane (bit 7 = leftmost pixel).
            // See: https://www.nesdev.org/wiki/PPU_pattern_tables
            let base = chr_base + (tile * 16) as u16;
            let p0 = read_chr_byte(bus, base + row as u16);
            let p1 = read_chr_byte(bus, base + 8 + row as u16);
            let mut px = [0u8; 8];
            for (bit, slot) in px.iter_mut().enumerate() {
                let b = 7 - bit;
                let lo = (p0 >> b) & 1;
                let hi = (p1 >> b) & 1;
                *slot = (hi << 1) | lo;
            }
            px
        };
        // Density ramp for 2-bit pixel values 0..3: ' ' '.' ':' '#'.
        const RAMP: [char; 4] = [' ', '.', ':', '#'];
        out.push_str("16x16 tiles, 8x8 px each (0=space, 1=., 2=:, 3=#)\n");
        for ty in 0..16 {
            for py in 0..TILE_PX {
                for tx in 0..16 {
                    let tile = ty * 16 + tx;
                    let row = chr(tile, py);
                    for px in row.iter() {
                        out.push(RAMP[*px as usize]);
                    }
                    out.push(' ');
                }
                out.push('\n');
            }
            out.push('\n');
        }
    }

    // ---- OAM: 64 sprites -------------------------------------------------

    fn render_oam(&self, bus: &Bus, out: &mut String) {
        let oam = bus.ppu().oam();
        out.push_str("idx  Y    tile attr  X    pal flip pri bg0\n");
        out.push_str("---  ---  ---- ----  ---  --- ---- --- ---\n");
        for i in 0..OAM_COUNT {
            let base = i * 4;
            let y = oam[base];
            let tile = oam[base + 1];
            let attr = oam[base + 2];
            let x = oam[base + 3];
            let pal = attr & 0x03;
            let flip_h = (attr >> 6) & 1;
            let flip_v = (attr >> 7) & 1;
            let priority = (attr >> 5) & 1;
            let behind_bg = priority == 1;
            out.push_str(&format!(
                "{i:3}  {y:02X}   {tile:02X}  {attr:02X}   {x:02X}   {pal}   {flip_h}    {flip_v}    {priority}   {behind_bg}\n",
            ));
        }
    }

    // ---- Palettes: 32 bytes, 8 palettes ----------------------------------

    fn render_palettes(&self, bus: &Bus, out: &mut String) {
        let ppu = bus.ppu();
        out.push_str("bg palettes (0-3) | sprite palettes (4-7)\n");
        out.push_str("pal  c0    c1    c2    c3    (idx / ARGB)\n");
        for pal in 0..8u8 {
            let is_sprite = pal >= 4;
            let base = if is_sprite {
                0x10 + (pal - 4) * 4
            } else {
                pal * 4
            };
            out.push_str(&format!("{pal}    "));
            for c in 0..4 {
                let addr = 0x3F00 + base as u16 + c as u16;
                let idx = ppu.read_palette(addr) & 0x3F;
                let argb = nes_color_to_argb(idx);
                out.push_str(&format!("{idx:02X}/{argb:08X} "));
            }
            out.push_str(if is_sprite { "(sprite)\n" } else { "(bg)\n" });
        }
        // Raw 32-byte palette RAM dump for completeness.
        out.push_str("\nraw palette RAM ($3F00-$3F1F):\n");
        for row in 0..2 {
            out.push_str(&format!("${:04X}:", 0x3F00 + row as u16 * 16));
            for col in 0..16 {
                let addr = 0x3F00 + row * 16 + col;
                out.push_str(&format!(" {:02X}", ppu.read_palette(addr as u16)));
            }
            out.push('\n');
        }
    }
}

/// Read one CHR byte via the cartridge's side-effect-free `read_chr`.
/// Returns 0 when no cartridge is loaded.
fn read_chr_byte(bus: &Bus, addr: u16) -> u8 {
    bus.cartridge().map(|c| c.read_chr(addr)).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycle_advances_through_all_views() {
        let mut v = PpuViewer::new();
        assert_eq!(v.mode, PpuView::Nametables);
        v.cycle();
        assert_eq!(v.mode, PpuView::PatternTable { bank: 0 });
        v.cycle(); // toggle to bank 1
        assert_eq!(v.mode, PpuView::PatternTable { bank: 1 });
        v.cycle(); // bank 1 → OAM
        assert_eq!(v.mode, PpuView::Oam);
        v.cycle();
        assert_eq!(v.mode, PpuView::Palettes);
        v.cycle();
        assert_eq!(v.mode, PpuView::Nametables);
    }

    #[test]
    fn render_nametables_empty_bus_does_not_panic() {
        let bus = Bus::new();
        let v = PpuViewer {
            mode: PpuView::Nametables,
            pattern_bank: 0,
        };
        let s = v.render(&bus);
        assert!(s.contains("nametables"));
        assert!(s.contains("NT0"));
    }

    #[test]
    fn render_pattern_table_empty_bus_does_not_panic() {
        let bus = Bus::new();
        let v = PpuViewer {
            mode: PpuView::PatternTable { bank: 0 },
            pattern_bank: 0,
        };
        let s = v.render(&bus);
        assert!(s.contains("pattern table 0"));
    }

    #[test]
    fn render_oam_empty_bus_shows_64_sprites() {
        let bus = Bus::new();
        let v = PpuViewer {
            mode: PpuView::Oam,
            pattern_bank: 0,
        };
        let s = v.render(&bus);
        // 64 sprite rows + header + separator.
        let lines = s.lines().count();
        assert!(lines >= 64);
    }

    #[test]
    fn render_palettes_shows_8_palettes() {
        let bus = Bus::new();
        let v = PpuViewer {
            mode: PpuView::Palettes,
            pattern_bank: 0,
        };
        let s = v.render(&bus);
        assert!(s.contains("(bg)"));
        assert!(s.contains("(sprite)"));
    }
}
