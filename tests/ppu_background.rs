//! Integration tests for PPU background rendering (M8).
//!
//! These tests exercise the background rendering pipeline end-to-end
//! through the CPU memory bus: nametable / pattern / palette data is
//! loaded via PPUDATA ($2007) writes (routed through the cartridge for
//! CHR and through the PPU for nametables/palettes), then
//! [`Bus::render_background`] is called and the resulting framebuffer is
//! inspected pixel-by-pixel.
//!
//! # Scroll ordering
//!
//! The PPU's `t` register is shared between PPUSCROLL (scroll position)
//! and PPUADDR (PPUDATA access). After PPUDATA writes, `t` holds the
//! last VRAM address, not the scroll. Real games set the scroll via
//! PPUSCROLL as the *last* write during VBlank, before rendering begins.
//! These tests mirror that ordering: all PPUDATA setup first, then
//! `set_scroll` (and PPUCTRL), then `render_background`.
//!
//! Coverage:
//! - Solid-tile fill across the whole screen.
//! - Pattern-table bitplane decode (per-row and per-column bit selection).
//! - Attribute-table palette selection (all four 16×16 sub-quadrants).
//! - PPUCTRL background pattern table select ($0000 vs $1000).
//! - PPUCTRL base nametable select.
//! - PPUMASK show-background / show-left-column masking.
//! - PPUSCROLL horizontal and vertical scroll with nametable wrap.
//! - Nametable mirroring (horizontal / vertical) during rendering.
//! - CHR-ROM vs CHR-RAM pattern sources.
//! - Universal background color (pattern 0 → $3F00).
//! - No-cartridge path (CHR reads return 0 → blank tiles).
//!
//! See: https://www.nesdev.org/wiki/PPU_rendering

use nes_emu::bus::Bus;
use nes_emu::cartridge::Cartridge;
use nes_emu::mappers::Mirroring;
use nes_emu::ppu::render::nes_color_to_argb;
use nes_emu::ppu::{SCREEN_HEIGHT, SCREEN_WIDTH};

// ---- helpers -----------------------------------------------------------

/// Build an NROM cartridge with CHR-RAM (0 CHR banks) and the given
/// mirroring. PRG is 32 KB of zeroes.
fn make_cart(mirroring: Mirroring) -> Cartridge {
    let mut bytes = Vec::with_capacity(16 + 32 * 1024);
    bytes.extend_from_slice(&[b'N', b'E', b'S', 0x1A]);
    bytes.push(2); // 2 × 16 KB PRG
    bytes.push(0); // 0 × 8 KB CHR → CHR-RAM
    let flags6: u8 = match mirroring {
        Mirroring::Vertical => 0b0000_0001,
        Mirroring::Horizontal => 0b0000_0000,
        Mirroring::FourScreen => 0b0000_1000,
        Mirroring::SingleScreen => 0b0000_0000,
    };
    bytes.push(flags6);
    bytes.push(0);
    bytes.extend_from_slice(&[0u8; 8]);
    bytes.resize(16 + 32 * 1024, 0);
    Cartridge::from_bytes(&bytes).expect("build test cartridge")
}

/// Build an NROM cartridge with 8 KB CHR-ROM initialised from `chr_data`.
fn make_cart_chr_rom(chr_data: &[u8; 8192], mirroring: Mirroring) -> Cartridge {
    let mut bytes = Vec::with_capacity(16 + 32 * 1024 + 8192);
    bytes.extend_from_slice(&[b'N', b'E', b'S', 0x1A]);
    bytes.push(2); // 2 × 16 KB PRG
    bytes.push(1); // 1 × 8 KB CHR-ROM
    let flags6: u8 = match mirroring {
        Mirroring::Vertical => 0b0000_0001,
        _ => 0b0000_0000,
    };
    bytes.push(flags6);
    bytes.push(0);
    bytes.extend_from_slice(&[0u8; 8]);
    bytes.resize(16 + 32 * 1024, 0);
    bytes.extend_from_slice(chr_data);
    Cartridge::from_bytes(&bytes).expect("build chr-rom cartridge")
}

/// Set the PPU VRAM address via two PPUADDR ($2006) writes.
fn set_vram_addr(bus: &mut Bus, addr: u16) {
    bus.write(0x2006, ((addr >> 8) & 0x3F) as u8);
    bus.write(0x2006, (addr & 0xFF) as u8);
}

/// Write a sequence of bytes to VRAM starting at `addr` via PPUDATA
/// ($2007), using the default +1 increment.
fn write_vram(bus: &mut Bus, addr: u16, data: &[u8]) {
    set_vram_addr(bus, addr);
    for &b in data {
        bus.write(0x2007, b);
    }
}

/// Fill `count` nametable tiles starting at `nt_addr` with `tile`.
fn fill_nametable_tiles(bus: &mut Bus, nt_addr: u16, tile: u8, count: usize) {
    set_vram_addr(bus, nt_addr);
    for _ in 0..count {
        bus.write(0x2007, tile);
    }
}

/// Set the scroll position via two PPUSCROLL ($2005) writes (X then Y).
/// MUST be called after all PPUDATA writes and before `render_background`,
/// because PPUSCROLL and PPUADDR share the `t` register.
fn set_scroll(bus: &mut Bus, x: u8, y: u8) {
    bus.write(0x2005, x);
    bus.write(0x2005, y);
}

/// Enable background rendering (and left column) via PPUMASK ($2001).
fn enable_bg(bus: &mut Bus) {
    bus.write(0x2001, 0b0000_1010); // show bg + show bg left 8
}

// =====================================================================
//  Basic rendering
// =====================================================================

#[test]
fn render_solid_tile_fills_screen_via_bus() {
    let mut bus = Bus::with_cartridge(make_cart(Mirroring::Horizontal));
    enable_bg(&mut bus);
    // Nametable 0: every tile = 1 (960 tiles = 32×30).
    fill_nametable_tiles(&mut bus, 0x2000, 0x01, 960);
    // Pattern table 0, tile 1 ($0010-$001F): all bits set (solid).
    let solid: [u8; 16] = [0xFF; 16];
    write_vram(&mut bus, 0x0010, &solid);
    // Palette: $3F00 = black (universal bg), $3F03 = white.
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    // Reset scroll to (0,0) AFTER PPUDATA writes (t register is shared).
    set_scroll(&mut bus, 0, 0);
    bus.render_background();
    let white = nes_color_to_argb(0x20);
    let fb = bus.ppu().framebuffer();
    for y in 0..SCREEN_HEIGHT {
        for x in 0..SCREEN_WIDTH {
            assert_eq!(fb[y * SCREEN_WIDTH + x], white, "pixel ({},{})", x, y);
        }
    }
}

#[test]
fn render_pattern_zero_uses_universal_bg() {
    let mut bus = Bus::with_cartridge(make_cart(Mirroring::Horizontal));
    enable_bg(&mut bus);
    // Nametable 0: tile 0 everywhere (already zero-initialised).
    // Palette: $3F00 = 0x21 (blue), $3F01 = 0x20 (white).
    write_vram(&mut bus, 0x3F00, &[0x21, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_background();
    let bg = nes_color_to_argb(0x21);
    let fb = bus.ppu().framebuffer();
    for y in 0..SCREEN_HEIGHT {
        for x in 0..SCREEN_WIDTH {
            assert_eq!(fb[y * SCREEN_WIDTH + x], bg, "pixel ({},{})", x, y);
        }
    }
}

#[test]
fn render_background_disabled_fills_universal_bg() {
    let mut bus = Bus::with_cartridge(make_cart(Mirroring::Horizontal));
    // PPUMASK = 0 → background disabled.
    fill_nametable_tiles(&mut bus, 0x2000, 0x01, 960);
    let solid: [u8; 16] = [0xFF; 16];
    write_vram(&mut bus, 0x0010, &solid);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_background();
    let bg = nes_color_to_argb(0x0E);
    let fb = bus.ppu().framebuffer();
    for y in 0..SCREEN_HEIGHT {
        for x in 0..SCREEN_WIDTH {
            assert_eq!(fb[y * SCREEN_WIDTH + x], bg);
        }
    }
}

#[test]
fn render_left_column_masked_when_ppumask_bit1_clear() {
    let mut bus = Bus::with_cartridge(make_cart(Mirroring::Horizontal));
    // Show background but NOT left 8 pixels.
    bus.write(0x2001, 0b0000_1000);
    fill_nametable_tiles(&mut bus, 0x2000, 0x01, 960);
    let solid: [u8; 16] = [0xFF; 16];
    write_vram(&mut bus, 0x0010, &solid);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_background();
    let bg = nes_color_to_argb(0x0E);
    let white = nes_color_to_argb(0x20);
    let fb = bus.ppu().framebuffer();
    for y in 0..SCREEN_HEIGHT {
        // Left 8 pixels → universal bg.
        for x in 0..8 {
            assert_eq!(fb[y * SCREEN_WIDTH + x], bg, "left pixel ({},{})", x, y);
        }
        // Pixel 8+ → solid white.
        assert_eq!(fb[y * SCREEN_WIDTH + 8], white, "pixel (8,{})", y);
    }
}

// =====================================================================
//  Pattern-table decode
// =====================================================================

#[test]
fn render_fine_y_selects_pattern_row() {
    let mut bus = Bus::with_cartridge(make_cart(Mirroring::Horizontal));
    enable_bg(&mut bus);
    fill_nametable_tiles(&mut bus, 0x2000, 0x01, 960);
    // Tile 1: only the top row (fine_y = 0) has set bits in plane 0.
    let mut tile = [0u8; 16];
    tile[0] = 0xFF; // plane 0, row 0
    write_vram(&mut bus, 0x0010, &tile);
    // Palette: $3F01 = white, $3F00 = black.
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_background();
    let white = nes_color_to_argb(0x20);
    let black = nes_color_to_argb(0x0E);
    let fb = bus.ppu().framebuffer();
    // Row 0 of each 8-pixel tile row → white; rows 1-7 → black.
    assert_eq!(fb[0], white, "fine_y=0 row set");
    assert_eq!(fb[SCREEN_WIDTH], black, "fine_y=1 row clear");
    assert_eq!(fb[7 * SCREEN_WIDTH], black, "fine_y=7 row clear");
    assert_eq!(fb[8 * SCREEN_WIDTH], white, "next tile row fine_y=0");
}

#[test]
fn render_fine_x_selects_pattern_bit() {
    let mut bus = Bus::with_cartridge(make_cart(Mirroring::Horizontal));
    enable_bg(&mut bus);
    fill_nametable_tiles(&mut bus, 0x2000, 0x01, 960);
    // Tile 1: only bit 7 set in each row.
    let mut tile = [0u8; 16];
    for slot in tile.iter_mut().take(8) {
        *slot = 0x80;
    }
    write_vram(&mut bus, 0x0010, &tile);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x20]);
    // Scroll X = 4 (coarse X = 0, fine X = 4) — set AFTER PPUDATA writes.
    set_scroll(&mut bus, 0x04, 0x00);
    bus.render_background();
    let white = nes_color_to_argb(0x20);
    let black = nes_color_to_argb(0x0E);
    let fb = bus.ppu().framebuffer();
    // With fine X = 4, screen X 0 → tile pixel 4 (bit 3, clear).
    assert_eq!(fb[0], black, "screen X 0 → tile pixel 4 (clear)");
    // Screen X 3 → tile pixel 7 (bit 0, clear).
    assert_eq!(fb[3], black, "screen X 3 → tile pixel 7 (clear)");
    // Screen X 4 → tile pixel 0 of next col (bit 7, set).
    assert_eq!(fb[4], white, "screen X 4 → next tile pixel 0 (set)");
}

#[test]
fn render_pattern_table_1000_selected() {
    let mut bus = Bus::with_cartridge(make_cart(Mirroring::Horizontal));
    enable_bg(&mut bus);
    fill_nametable_tiles(&mut bus, 0x2000, 0x01, 960);
    // Pattern table 0, tile 1 = blank; table 1, tile 1 = solid.
    let blank: [u8; 16] = [0x00; 16];
    let solid: [u8; 16] = [0xFF; 16];
    write_vram(&mut bus, 0x0010, &blank);
    write_vram(&mut bus, 0x1010, &solid);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    // PPUCTRL bit 4 = 1 → bg pattern table at $1000. Set AFTER PPUDATA.
    bus.write(0x2000, 0b0001_0000);
    set_scroll(&mut bus, 0, 0);
    bus.render_background();
    let white = nes_color_to_argb(0x20);
    let fb = bus.ppu().framebuffer();
    assert_eq!(fb[0], white, "table 1 tile 1 is solid");
    assert_eq!(fb[SCREEN_WIDTH * SCREEN_HEIGHT - 1], white);
}

// =====================================================================
//  Attribute table
// =====================================================================

#[test]
fn render_attribute_quadrant_palette_selection() {
    let mut bus = Bus::with_cartridge(make_cart(Mirroring::Horizontal));
    enable_bg(&mut bus);
    fill_nametable_tiles(&mut bus, 0x2000, 0x01, 960);
    let solid: [u8; 16] = [0xFF; 16];
    write_vram(&mut bus, 0x0010, &solid);
    // First attribute byte (top-left 32×32) = 0b01_01_01_01 → palette 1
    // for all four 16×16 sub-quadrants.
    set_vram_addr(&mut bus, 0x23C0);
    bus.write(0x2007, 0b01_01_01_01);
    // Palette 0 color 3 = white; palette 1 color 3 = blue.
    write_vram(
        &mut bus,
        0x3F00,
        &[0x0E, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x21],
    );
    set_scroll(&mut bus, 0, 0);
    bus.render_background();
    let blue = nes_color_to_argb(0x21);
    let white = nes_color_to_argb(0x20);
    let fb = bus.ppu().framebuffer();
    // Top-left 32×32 → palette 1 → blue.
    assert_eq!(fb[0], blue, "top-left sub-quadrant → palette 1");
    assert_eq!(
        fb[31 * SCREEN_WIDTH + 31],
        blue,
        "bottom-right of first quadrant → palette 1"
    );
    // Next 32×32 to the right (32..64, 0..32) → palette 0 → white.
    assert_eq!(fb[32], white, "second quadrant → palette 0");
    assert_eq!(fb[31 * SCREEN_WIDTH + 63], white);
}

#[test]
fn render_attribute_all_four_sub_quadrants() {
    let mut bus = Bus::with_cartridge(make_cart(Mirroring::Horizontal));
    enable_bg(&mut bus);
    fill_nametable_tiles(&mut bus, 0x2000, 0x01, 960);
    let solid: [u8; 16] = [0xFF; 16];
    write_vram(&mut bus, 0x0010, &solid);
    // Attribute byte for top-left 32×32: each sub-quadrant gets a
    // different palette (0, 1, 2, 3).
    //   top-left     = palette 0 (bits 0-1)
    //   top-right    = palette 1 (bits 2-3)
    //   bottom-left  = palette 2 (bits 4-5)
    //   bottom-right = palette 3 (bits 6-7)
    let attr: u8 = 0b11_10_01_00;
    set_vram_addr(&mut bus, 0x23C0);
    bus.write(0x2007, attr);
    // Palettes 0-3, color 3 = distinct colors.
    write_vram(
        &mut bus,
        0x3F00,
        &[
            0x0E, 0x00, 0x00, 0x20, // palette 0: white
            0x00, 0x00, 0x00, 0x21, // palette 1: blue
            0x00, 0x00, 0x00, 0x16, // palette 2: red-ish
            0x00, 0x00, 0x00, 0x1A,
        ], // palette 3: green-ish
    );
    set_scroll(&mut bus, 0, 0);
    bus.render_background();
    let fb = bus.ppu().framebuffer();
    // top-left 16×16 → palette 0 → white
    assert_eq!(fb[0], nes_color_to_argb(0x20), "top-left → palette 0");
    // top-right (16,0) → palette 1 → blue
    assert_eq!(fb[16], nes_color_to_argb(0x21), "top-right → palette 1");
    // bottom-left (0,16) → palette 2
    assert_eq!(
        fb[16 * SCREEN_WIDTH],
        nes_color_to_argb(0x16),
        "bottom-left → palette 2"
    );
    // bottom-right (16,16) → palette 3
    assert_eq!(
        fb[16 * SCREEN_WIDTH + 16],
        nes_color_to_argb(0x1A),
        "bottom-right → palette 3"
    );
}

// =====================================================================
//  Nametable select + mirroring
// =====================================================================

#[test]
fn render_base_nametable_select() {
    let mut bus = Bus::with_cartridge(make_cart(Mirroring::Vertical));
    enable_bg(&mut bus);
    // NT 0 blank (zero-initialised). NT 1 solid.
    fill_nametable_tiles(&mut bus, 0x2400, 0x01, 960);
    let solid: [u8; 16] = [0xFF; 16];
    write_vram(&mut bus, 0x0010, &solid);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    // PPUCTRL bits 0-1 = 0b01 → base nametable $2400 (NT 1). Set AFTER PPUDATA.
    bus.write(0x2000, 0b0000_0001);
    set_scroll(&mut bus, 0, 0);
    bus.render_background();
    let white = nes_color_to_argb(0x20);
    let fb = bus.ppu().framebuffer();
    assert_eq!(fb[0], white, "base NT 1 → solid");
}

#[test]
fn render_horizontal_mirroring_nt0_nt1_share() {
    let mut bus = Bus::with_cartridge(make_cart(Mirroring::Horizontal));
    enable_bg(&mut bus);
    // Write solid tiles to NT 0; NT 1 should mirror it under horizontal
    // mirroring, so rendering from base NT 1 should also be solid.
    fill_nametable_tiles(&mut bus, 0x2000, 0x01, 960);
    let solid: [u8; 16] = [0xFF; 16];
    write_vram(&mut bus, 0x0010, &solid);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    // Base nametable = NT 1 ($2400). Set AFTER PPUDATA.
    bus.write(0x2000, 0b0000_0001);
    set_scroll(&mut bus, 0, 0);
    bus.render_background();
    let white = nes_color_to_argb(0x20);
    let fb = bus.ppu().framebuffer();
    assert_eq!(fb[0], white, "NT 1 mirrors NT 0 under horizontal mirroring");
}

#[test]
fn render_vertical_mirroring_nt0_nt2_share() {
    let mut bus = Bus::with_cartridge(make_cart(Mirroring::Vertical));
    enable_bg(&mut bus);
    // Write solid tiles to NT 0; NT 2 should mirror it under vertical
    // mirroring, so rendering from base NT 2 should also be solid.
    fill_nametable_tiles(&mut bus, 0x2000, 0x01, 960);
    let solid: [u8; 16] = [0xFF; 16];
    write_vram(&mut bus, 0x0010, &solid);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    // Base nametable = NT 2 ($2800). Set AFTER PPUDATA.
    bus.write(0x2000, 0b0000_0010);
    set_scroll(&mut bus, 0, 0);
    bus.render_background();
    let white = nes_color_to_argb(0x20);
    let fb = bus.ppu().framebuffer();
    assert_eq!(fb[0], white, "NT 2 mirrors NT 0 under vertical mirroring");
}

// =====================================================================
//  Scrolling
// =====================================================================

#[test]
fn render_horizontal_scroll_wraps_to_next_nametable() {
    let mut bus = Bus::with_cartridge(make_cart(Mirroring::Vertical));
    enable_bg(&mut bus);
    // NT 0 blank. NT 1 solid.
    fill_nametable_tiles(&mut bus, 0x2400, 0x01, 960);
    let solid: [u8; 16] = [0xFF; 16];
    write_vram(&mut bus, 0x0010, &solid);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    // Scroll X = 8 (coarse X = 1, fine X = 0) — set AFTER PPUDATA writes.
    set_scroll(&mut bus, 0x08, 0x00);
    bus.render_background();
    let white = nes_color_to_argb(0x20);
    let black = nes_color_to_argb(0x0E);
    let fb = bus.ppu().framebuffer();
    // Screen X 0..247 → NT 0 (blank). X 248..255 → NT 1 (solid).
    assert_eq!(fb[0], black, "screen X 0 → NT 0 (blank)");
    assert_eq!(fb[248], white, "screen X 248 → NT 1 (solid)");
    assert_eq!(fb[255], white, "screen X 255 → NT 1 (solid)");
}

#[test]
fn render_vertical_scroll_wraps_to_next_nametable() {
    let mut bus = Bus::with_cartridge(make_cart(Mirroring::Horizontal));
    enable_bg(&mut bus);
    // NT 0 blank. NT 2 solid.
    fill_nametable_tiles(&mut bus, 0x2800, 0x01, 960);
    let solid: [u8; 16] = [0xFF; 16];
    write_vram(&mut bus, 0x0010, &solid);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    // Scroll Y = 24 (coarse Y = 3, fine Y = 0) — set AFTER PPUDATA writes.
    set_scroll(&mut bus, 0x00, 0x18);
    bus.render_background();
    let white = nes_color_to_argb(0x20);
    let black = nes_color_to_argb(0x0E);
    let fb = bus.ppu().framebuffer();
    // Screen Y 0..231 → NT 0 (blank). Y 232..239 → NT 2 (solid).
    assert_eq!(fb[0], black, "screen Y 0 → NT 0 (blank)");
    assert_eq!(fb[232 * SCREEN_WIDTH], white, "screen Y 232 → NT 2 (solid)");
    assert_eq!(fb[239 * SCREEN_WIDTH], white, "screen Y 239 → NT 2 (solid)");
}

// =====================================================================
//  CHR-ROM vs CHR-RAM
// =====================================================================

#[test]
fn render_chr_rom_pattern_data() {
    // Build a CHR-ROM cartridge with tile 1 = solid in pattern table 0.
    let mut chr = [0u8; 8192];
    for i in 0..16 {
        chr[0x10 + i] = 0xFF;
    }
    let mut bus = Bus::with_cartridge(make_cart_chr_rom(&chr, Mirroring::Horizontal));
    enable_bg(&mut bus);
    fill_nametable_tiles(&mut bus, 0x2000, 0x01, 960);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_background();
    let white = nes_color_to_argb(0x20);
    let fb = bus.ppu().framebuffer();
    assert_eq!(fb[0], white, "CHR-ROM tile 1 is solid");
    assert_eq!(fb[SCREEN_WIDTH * SCREEN_HEIGHT - 1], white);
}

#[test]
fn render_chr_ram_writes_persist_for_rendering() {
    let mut bus = Bus::with_cartridge(make_cart(Mirroring::Horizontal));
    enable_bg(&mut bus);
    fill_nametable_tiles(&mut bus, 0x2000, 0x01, 960);
    // Write pattern data to CHR-RAM via PPUDATA ($0000-$1FFF region).
    let solid: [u8; 16] = [0xFF; 16];
    write_vram(&mut bus, 0x0010, &solid);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_background();
    let white = nes_color_to_argb(0x20);
    let fb = bus.ppu().framebuffer();
    assert_eq!(fb[0], white, "CHR-RAM pattern write is visible in render");
}

// =====================================================================
//  No-cartridge path
// =====================================================================

#[test]
fn render_no_cartridge_chr_reads_return_blank() {
    let mut bus = Bus::new();
    enable_bg(&mut bus);
    // No cartridge → CHR reads return 0 → all patterns blank → universal bg.
    // Write nametable + palette via the bus (PPU owns nametable/palette).
    fill_nametable_tiles(&mut bus, 0x2000, 0x01, 960);
    write_vram(&mut bus, 0x3F00, &[0x21, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_background();
    let bg = nes_color_to_argb(0x21);
    let fb = bus.ppu().framebuffer();
    for y in 0..SCREEN_HEIGHT {
        for x in 0..SCREEN_WIDTH {
            assert_eq!(fb[y * SCREEN_WIDTH + x], bg, "blank pattern → universal bg");
        }
    }
}

// =====================================================================
//  Distinct-tile rendering (multi-tile scene)
// =====================================================================

#[test]
fn render_two_distinct_tiles_in_nametable() {
    let mut bus = Bus::with_cartridge(make_cart(Mirroring::Horizontal));
    enable_bg(&mut bus);
    // Tile 0 = blank (zero-initialised). Tile 1 = solid.
    let solid: [u8; 16] = [0xFF; 16];
    write_vram(&mut bus, 0x0010, &solid);
    // Nametable: tile 0 in column 0, tile 1 in column 1.
    set_vram_addr(&mut bus, 0x2000);
    for _row in 0..30 {
        bus.write(0x2007, 0x00); // col 0 = tile 0 (blank)
        bus.write(0x2007, 0x01); // col 1 = tile 1 (solid)
                                 // Fill remaining 30 columns with tile 0.
        for _ in 0..30 {
            bus.write(0x2007, 0x00);
        }
    }
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_background();
    let white = nes_color_to_argb(0x20);
    let black = nes_color_to_argb(0x0E);
    let fb = bus.ppu().framebuffer();
    // Column 0 (x 0..7) → tile 0 → blank → black.
    assert_eq!(fb[0], black, "col 0 tile 0 → blank");
    // Column 1 (x 8..15) → tile 1 → solid → white.
    assert_eq!(fb[8], white, "col 1 tile 1 → solid");
    assert_eq!(fb[15], white);
    // Column 2 (x 16) → tile 0 → blank.
    assert_eq!(fb[16], black, "col 2 tile 0 → blank");
}

#[test]
fn render_framebuffer_dimensions() {
    let mut bus = Bus::with_cartridge(make_cart(Mirroring::Horizontal));
    enable_bg(&mut bus);
    set_scroll(&mut bus, 0, 0);
    bus.render_background();
    let fb = bus.ppu().framebuffer();
    assert_eq!(fb.len(), SCREEN_WIDTH * SCREEN_HEIGHT);
    assert_eq!(SCREEN_WIDTH, 256);
    assert_eq!(SCREEN_HEIGHT, 240);
}

#[test]
fn render_pixel_accessor_matches_framebuffer() {
    let mut bus = Bus::with_cartridge(make_cart(Mirroring::Horizontal));
    enable_bg(&mut bus);
    fill_nametable_tiles(&mut bus, 0x2000, 0x01, 960);
    let solid: [u8; 16] = [0xFF; 16];
    write_vram(&mut bus, 0x0010, &solid);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_background();
    let fb = bus.ppu().framebuffer();
    // The pixel() accessor should match direct framebuffer indexing.
    for &(x, y) in &[(0usize, 0usize), (127, 119), (255, 239), (8, 8)] {
        assert_eq!(bus.ppu().pixel(x, y), fb[y * SCREEN_WIDTH + x]);
    }
}

#[test]
fn render_attribute_from_nonzero_base_nametable() {
    // Verify attribute fetch uses the correct nametable's attribute table
    // when the base nametable is not NT 0.
    let mut bus = Bus::with_cartridge(make_cart(Mirroring::Vertical));
    enable_bg(&mut bus);
    // NT 1 ($2400): solid tiles + attribute byte at $27C0 = palette 1.
    fill_nametable_tiles(&mut bus, 0x2400, 0x01, 960);
    let solid: [u8; 16] = [0xFF; 16];
    write_vram(&mut bus, 0x0010, &solid);
    set_vram_addr(&mut bus, 0x27C0); // NT 1 attribute table
    bus.write(0x2007, 0b01_01_01_01); // palette 1 for all sub-quadrants
                                      // Palette 0 color 3 = white, palette 1 color 3 = blue.
    write_vram(
        &mut bus,
        0x3F00,
        &[0x0E, 0x00, 0x00, 0x20, 0x00, 0x00, 0x00, 0x21],
    );
    // Base nametable = NT 1. Set AFTER PPUDATA.
    bus.write(0x2000, 0b0000_0001);
    set_scroll(&mut bus, 0, 0);
    bus.render_background();
    let blue = nes_color_to_argb(0x21);
    let fb = bus.ppu().framebuffer();
    // Top-left of NT 1 → palette 1 → blue (attribute fetched from $27C0).
    assert_eq!(fb[0], blue, "NT 1 attribute at $27C0 → palette 1");
    assert_eq!(fb[31 * SCREEN_WIDTH + 31], blue);
}

#[test]
fn render_combined_xy_scroll_wraps_to_diagonal_nametable() {
    // Scroll both X and Y past 256 → effective nametable = base ^ 3.
    let mut bus = Bus::with_cartridge(make_cart(Mirroring::FourScreen));
    enable_bg(&mut bus);
    // NT 3 ($2C00): solid tiles. NT 0/1/2 blank.
    fill_nametable_tiles(&mut bus, 0x2C00, 0x01, 960);
    let solid: [u8; 16] = [0xFF; 16];
    write_vram(&mut bus, 0x0010, &solid);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    // Scroll X = 8, Y = 24 → screen (248, 232) wraps to NT 3.
    set_scroll(&mut bus, 0x08, 0x18);
    bus.render_background();
    let white = nes_color_to_argb(0x20);
    let black = nes_color_to_argb(0x0E);
    let fb = bus.ppu().framebuffer();
    // Screen (0, 0) → NT 0 (blank).
    assert_eq!(fb[0], black, "screen (0,0) → NT 0 (blank)");
    // Screen (248, 232) → gx=256, gy=256 → NT 3 (solid).
    assert_eq!(
        fb[232 * SCREEN_WIDTH + 248],
        white,
        "screen (248,232) → NT 3 (solid, diagonal wrap)"
    );
    assert_eq!(fb[239 * SCREEN_WIDTH + 255], white);
}
