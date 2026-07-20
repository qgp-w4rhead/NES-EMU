//! Integration tests for PPU sprite rendering (M9).
//!
//! These tests exercise the sprite rendering pipeline end-to-end through
//! the CPU memory bus: OAM is loaded via OAMADDR ($2003) + OAMDATA
//! ($2004), CHR pattern data via PPUDATA ($2007) writes (routed through
//! the cartridge for CHR-RAM), palettes via PPUDATA to `$3F00-$3F1F`,
//! then [`Bus::render_frame`] (or [`Bus::render_sprites`] on top of a
//! prior [`Bus::render_background`]) is called and the framebuffer is
//! inspected pixel-by-pixel.
//!
//! # OAM Y convention
//!
//! Per the NESdev wiki, OAM byte 0 is the desired top screen Y minus 1
//! (sprite data is delayed by one scanline). A sprite with OAM Y = `y`
//! is visible on scanlines `y+1` through `y+8`. Values `$EF-$FF` hide
//! the sprite. So to render a sprite at screen Y = `S`, the test writes
//! `S - 1` into OAM byte 0.
//!
//! Coverage:
//! - Basic 8×8 sprite placement (X, Y, tile, color).
//! - OAM Y one-scanline delay (sprite at OAM Y=0 appears at screen Y=1).
//! - Hidden sprites (OAM Y = `$EF`, `$FF`).
//! - Pattern-table select (PPUCTRL bit 3: `$0000` vs `$1000`).
//! - Tile index selection.
//! - Horizontal flip, vertical flip, both flips.
//! - Palette select (attribute bits 0-1 → palettes 4-7 at `$3F10`).
//! - Pattern 0 transparency (donut / ring sprite).
//! - Priority in front of background (default).
//! - Priority behind background (attribute bit 5) — sprite hidden where
//!   bg is opaque, visible where bg is transparent.
//! - Sprite-to-sprite priority (lower OAM index in front).
//! - 8-sprite-per-scanline limit + overflow flag.
//! - Left 8-pixel clip (PPUMASK bit 2).
//! - Sprites disabled (PPUMASK bit 4 clear).
//! - `render_frame` composites background + sprites.
//! - Sprite partially off the right edge.
//! - `bg_pattern` buffer populated by `render_background`.
//!
//! See: https://www.nesdev.org/wiki/PPU_OAM
//! See: https://www.nesdev.org/wiki/PPU_rendering#Sprites

use nes_emu::bus::Bus;
use nes_emu::cartridge::Cartridge;
use nes_emu::ppu::render::nes_color_to_argb;
use nes_emu::ppu::{SCREEN_HEIGHT, SCREEN_WIDTH};

// ---- helpers -----------------------------------------------------------

/// Build an NROM cartridge with CHR-RAM (0 CHR banks) and horizontal
/// mirroring. PRG is 32 KB of zeroes.
fn make_cart() -> Cartridge {
    let mut bytes = Vec::with_capacity(16 + 32 * 1024);
    bytes.extend_from_slice(&[b'N', b'E', b'S', 0x1A]);
    bytes.push(2); // 2 × 16 KB PRG
    bytes.push(0); // 0 × 8 KB CHR → CHR-RAM
    bytes.push(0); // flags6: horizontal mirroring
    bytes.push(0);
    bytes.extend_from_slice(&[0u8; 8]);
    bytes.resize(16 + 32 * 1024, 0);
    Cartridge::from_bytes(&bytes).expect("build test cartridge")
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

/// Write the 16 bytes of a single 8×8 pattern tile to CHR at `base`
/// (`$0000`-`$1FFF`).
fn write_tile(bus: &mut Bus, base: u16, plane0: [u8; 8], plane1: [u8; 8]) {
    let mut data = [0u8; 16];
    data[..8].copy_from_slice(&plane0);
    data[8..].copy_from_slice(&plane1);
    write_vram(bus, base, &data);
}

/// Set the scroll position via two PPUSCROLL ($2005) writes (X then Y).
/// MUST be called after all PPUDATA writes (shared `t` register).
fn set_scroll(bus: &mut Bus, x: u8, y: u8) {
    bus.write(0x2005, x);
    bus.write(0x2005, y);
}

/// Write a single sprite's 4 bytes to OAM at sprite index `i` (0..64)
/// via OAMADDR ($2003) + OAMDATA ($2004).
fn write_sprite(bus: &mut Bus, i: usize, y: u8, tile: u8, attr: u8, x: u8) {
    let oam_addr = (i * 4) as u8;
    bus.write(0x2003, oam_addr);
    bus.write(0x2004, y);
    bus.write(0x2004, tile);
    bus.write(0x2004, attr);
    bus.write(0x2004, x);
}

/// Enable background + sprites (and both left columns) via PPUMASK.
fn enable_bg_and_sprites(bus: &mut Bus) {
    // show bg + show bg left + show sprites + show sprites left
    bus.write(0x2001, 0b0001_1110);
}

/// Enable sprites only (and left column).
fn enable_sprites_only(bus: &mut Bus) {
    bus.write(0x2001, 0b0001_0100);
}

/// Clear OAM to $FF (all sprites hidden: Y=$FF, tile=$FF, attr=$FF, X=$FF).
/// OAM is zero-initialised in the Ppu, which would make every sprite
/// visible at (0, 1-8) with tile 0. Tests must call this before writing
/// their test sprites to avoid spurious zero-initialised sprites.
fn clear_oam(bus: &mut Bus) {
    bus.write(0x2003, 0x00);
    for _ in 0..256 {
        bus.write(0x2004, 0xFF);
    }
}

/// A solid 8×8 tile (all pixels pattern 3).
const SOLID_TILE_P0: [u8; 8] = [0xFF; 8];
const SOLID_TILE_P1: [u8; 8] = [0xFF; 8];

/// A tile whose left half (cols 0-3) is pattern 3 and right half (cols
/// 4-7) is transparent (pattern 0). Plane0 = 0xF0, plane1 = 0xF0.
const HALF_TILE_P0: [u8; 8] = [0xF0; 8];
const HALF_TILE_P1: [u8; 8] = [0xF0; 8];

/// Read a framebuffer pixel from the bus's PPU.
fn px(bus: &Bus, x: usize, y: usize) -> u32 {
    bus.ppu().framebuffer()[y * SCREEN_WIDTH + x]
}

// =====================================================================
//  Basic placement
// =====================================================================

#[test]
fn sprite_renders_solid_8x8_block_at_position() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_sprites_only(&mut bus);
    clear_oam(&mut bus);
    // Sprite 0: OAM Y = 0 → visible on scanlines 1..=8, X = 0, tile = 0.
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 0);
    // Tile 0 at $0000: solid (pattern 3 everywhere).
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    // Palette: $3F00 = black (universal bg), $3F10 = sprite pal 4 color 0,
    // $3F13 = sprite pal 4 color 3 = white.
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    let black = nes_color_to_argb(0x0E);
    // Sprite occupies screen Y 1..=8, X 0..=7.
    for y in 1..=8 {
        for x in 0..=7 {
            assert_eq!(px(&bus, x, y), white, "sprite pixel ({},{})", x, y);
        }
    }
    // Outside the sprite: universal bg (black).
    assert_eq!(px(&bus, 0, 0), black, "scanline 0 has no sprite (Y delay)");
    assert_eq!(px(&bus, 8, 1), black, "x=8 outside sprite width");
    assert_eq!(px(&bus, 0, 9), black, "y=9 below sprite");
}

#[test]
fn sprite_y_has_one_scanline_delay() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_sprites_only(&mut bus);
    clear_oam(&mut bus);
    // OAM Y = 5 → visible on scanlines 6..=13.
    write_sprite(&mut bus, 0, 5, 0x00, 0x00, 0);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    let black = nes_color_to_argb(0x0E);
    assert_eq!(px(&bus, 0, 5), black, "scanline 5 = OAM Y, not yet visible");
    assert_eq!(px(&bus, 0, 6), white, "first visible scanline = Y+1");
    assert_eq!(px(&bus, 0, 13), white, "last visible scanline = Y+8");
    assert_eq!(px(&bus, 0, 14), black, "scanline Y+9 not visible");
}

#[test]
fn sprite_x_position_selects_column() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_sprites_only(&mut bus);
    clear_oam(&mut bus);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 100);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    let black = nes_color_to_argb(0x0E);
    assert_eq!(px(&bus, 99, 1), black, "x=99 is left of sprite");
    for x in 100..=107 {
        assert_eq!(px(&bus, x, 1), white, "sprite column {}", x);
    }
    assert_eq!(px(&bus, 108, 1), black, "x=108 is right of sprite");
}

// =====================================================================
//  Hidden sprites
// =====================================================================

#[test]
fn sprite_hidden_when_oam_y_is_0xef() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_sprites_only(&mut bus);
    clear_oam(&mut bus);
    write_sprite(&mut bus, 0, 0xEF, 0x00, 0x00, 0);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let black = nes_color_to_argb(0x0E);
    // No sprite pixels anywhere on screen.
    for y in 0..SCREEN_HEIGHT {
        for x in 0..SCREEN_WIDTH {
            assert_eq!(px(&bus, x, y), black, "no sprite at ({},{})", x, y);
        }
    }
}

#[test]
fn sprite_hidden_when_oam_y_is_0xff() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_sprites_only(&mut bus);
    clear_oam(&mut bus);
    write_sprite(&mut bus, 0, 0xFF, 0x00, 0x00, 0);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let black = nes_color_to_argb(0x0E);
    // Y=$FF would wrap to scanlines 256..263 (off-screen) anyway; ensure
    // nothing rendered.
    for y in 0..SCREEN_HEIGHT {
        assert_eq!(px(&bus, 0, y), black);
    }
}

// =====================================================================
//  Pattern table + tile index
// =====================================================================

#[test]
fn sprite_pattern_table_select_1000() {
    let mut bus = Bus::with_cartridge(make_cart());
    // PPUCTRL bit 3 = 1 → sprite pattern table at $1000.
    bus.write(0x2000, 0b0000_1000);
    enable_sprites_only(&mut bus);
    clear_oam(&mut bus);
    write_sprite(&mut bus, 0, 0, 0x01, 0x00, 0);
    // Tile 1 at $0000: blank. Tile 1 at $1000: solid.
    write_tile(&mut bus, 0x0010, [0x00; 8], [0x00; 8]);
    write_tile(&mut bus, 0x1010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    assert_eq!(px(&bus, 0, 1), white, "sprite uses $1000 table tile 1");
}

#[test]
fn sprite_tile_index_selects_pattern() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_sprites_only(&mut bus);
    clear_oam(&mut bus);
    // Sprite uses tile 2 (at $0020).
    write_sprite(&mut bus, 0, 0, 0x02, 0x00, 0);
    write_tile(&mut bus, 0x0020, SOLID_TILE_P0, SOLID_TILE_P1);
    // Tile 0 stays blank.
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    assert_eq!(px(&bus, 0, 1), white, "tile 2 is solid");
}

// =====================================================================
//  Flipping
// =====================================================================

#[test]
fn sprite_horizontal_flip_mirrors_columns() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_sprites_only(&mut bus);
    clear_oam(&mut bus);
    // Tile 0: left half solid, right half transparent.
    write_tile(&mut bus, 0x0000, HALF_TILE_P0, HALF_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0b0100_0000, 0); // hflip
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    let black = nes_color_to_argb(0x0E);
    // Without flip, cols 0-3 would be solid. With hflip, cols 4-7 are solid.
    for x in 0..4 {
        assert_eq!(px(&bus, x, 1), black, "hflipped col {} transparent", x);
    }
    for x in 4..8 {
        assert_eq!(px(&bus, x, 1), white, "hflipped col {} solid", x);
    }
}

#[test]
fn sprite_vertical_flip_mirrors_rows() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_sprites_only(&mut bus);
    clear_oam(&mut bus);
    // Tile 0: only the top row (row 0) is solid; rows 1-7 transparent.
    let mut p0 = [0u8; 8];
    let mut p1 = [0u8; 8];
    p0[0] = 0xFF;
    p1[0] = 0xFF;
    write_tile(&mut bus, 0x0000, p0, p1);
    write_sprite(&mut bus, 0, 0, 0x00, 0b1000_0000, 0); // vflip
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    let black = nes_color_to_argb(0x0E);
    // Without flip, scanline 1 (row 0) would be solid. With vflip, the
    // bottom row (scanline 8) is solid.
    assert_eq!(px(&bus, 0, 1), black, "vflipped top row transparent");
    assert_eq!(px(&bus, 0, 8), white, "vflipped bottom row solid");
}

#[test]
fn sprite_both_flips_mirrors_rows_and_columns() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_sprites_only(&mut bus);
    clear_oam(&mut bus);
    // Tile 0: left half solid.
    write_tile(&mut bus, 0x0000, HALF_TILE_P0, HALF_TILE_P1);
    // Only top row solid.
    let mut p0 = [0u8; 8];
    let mut p1 = [0u8; 8];
    p0[0] = 0xF0;
    p1[0] = 0xF0;
    write_tile(&mut bus, 0x0000, p0, p1);
    write_sprite(&mut bus, 0, 0, 0x00, 0b1100_0000, 0); // hflip + vflip
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    let black = nes_color_to_argb(0x0E);
    // Original: row 0, cols 0-3 solid. After both flips: row 7 (scanline 8),
    // cols 4-7 solid.
    assert_eq!(px(&bus, 4, 8), white, "h+v flipped corner solid");
    assert_eq!(px(&bus, 0, 8), black);
    assert_eq!(px(&bus, 4, 1), black);
}

// =====================================================================
//  Palette select
// =====================================================================

#[test]
fn sprite_palette_select_uses_3f10_region() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_sprites_only(&mut bus);
    clear_oam(&mut bus);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    // Four sprite palettes, color 3 each: 0x21, 0x22, 0x23, 0x24.
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    write_vram(&mut bus, 0x3F14, &[0x00, 0x00, 0x00, 0x22]);
    write_vram(&mut bus, 0x3F18, &[0x00, 0x00, 0x00, 0x23]);
    write_vram(&mut bus, 0x3F1C, &[0x00, 0x00, 0x00, 0x24]);
    // Four sprites, each using a different palette (bits 0-1).
    write_sprite(&mut bus, 0, 0, 0x00, 0b00, 0);
    write_sprite(&mut bus, 1, 0, 0x00, 0b01, 10);
    write_sprite(&mut bus, 2, 0, 0x00, 0b10, 20);
    write_sprite(&mut bus, 3, 0, 0x00, 0b11, 30);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    assert_eq!(px(&bus, 0, 1), nes_color_to_argb(0x21), "palette 4");
    assert_eq!(px(&bus, 10, 1), nes_color_to_argb(0x22), "palette 5");
    assert_eq!(px(&bus, 20, 1), nes_color_to_argb(0x23), "palette 6");
    assert_eq!(px(&bus, 30, 1), nes_color_to_argb(0x24), "palette 7");
}

// =====================================================================
//  Transparency
// =====================================================================

#[test]
fn sprite_pattern_zero_is_transparent() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_sprites_only(&mut bus);
    clear_oam(&mut bus);
    // Tile 0: left half solid, right half transparent.
    write_tile(&mut bus, 0x0000, HALF_TILE_P0, HALF_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 0);
    // Universal bg = black, sprite color 3 = white.
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    let black = nes_color_to_argb(0x0E);
    // Cols 0-3 solid (pattern 3), cols 4-7 transparent → universal bg.
    for x in 0..4 {
        assert_eq!(px(&bus, x, 1), white, "col {} opaque", x);
    }
    for x in 4..8 {
        assert_eq!(px(&bus, x, 1), black, "col {} transparent", x);
    }
}

// =====================================================================
//  Background priority
// =====================================================================

#[test]
fn sprite_in_front_of_background_by_default() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    // Background: solid white everywhere (nametable tile 1, tile 1 solid).
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite: solid blue, at (0, 0).
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 0);
    // Palette: bg $3F03 = white, sprite $3F13 = blue (0x21).
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let blue = nes_color_to_argb(0x21);
    // Sprite covers bg.
    assert_eq!(px(&bus, 0, 1), blue, "sprite in front of bg");
}

#[test]
fn sprite_behind_background_hidden_where_bg_opaque() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    // Background: left half of screen solid (tile 1), right half blank
    // (tile 0). Use a single tile row at y=0..7 for simplicity: nametable
    // tiles 0..31. Set tiles 0-15 = 1 (solid), 16-31 = 0 (blank).
    for i in 0..16 {
        set_vram_addr(&mut bus, 0x2000 + i);
        bus.write(0x2007, 0x01);
    }
    for i in 16..32 {
        set_vram_addr(&mut bus, 0x2000 + i);
        bus.write(0x2007, 0x00);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite: solid, priority behind bg (attr bit 5 set), at x=0..7.
    // Use tile 2 (at $0020) so bg tile 0 ($0000) stays blank.
    write_tile(&mut bus, 0x0020, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x02, 0b0010_0000, 0);
    // Palette: bg $3F03 = white, sprite $3F13 = blue.
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    let blue = nes_color_to_argb(0x21);
    let black = nes_color_to_argb(0x0E);
    // x=0..7: bg solid (white) → sprite hidden.
    for x in 0..8 {
        assert_eq!(px(&bus, x, 1), white, "behind-bg sprite hidden at x={}", x);
    }
    // x=8..15: bg solid (white, tiles 8-15 = 1) → sprite hidden.
    for x in 8..16 {
        assert_eq!(px(&bus, x, 1), white, "behind-bg sprite hidden at x={}", x);
    }
    // The sprite is only 8px wide (x=0..7), so x=8..15 has no sprite anyway.
    // Verify the bg-blank region (x=128..135) shows the sprite if we put one
    // there. Instead, just confirm bg blank region is black.
    assert_eq!(px(&bus, 128, 1), black, "blank bg is universal bg");
    let _ = blue; // blue unused here; tested in next case.
}

#[test]
fn sprite_behind_background_visible_where_bg_transparent() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    // Background: all blank (tile 0, pattern 0 → transparent).
    // (VRAM is zero-initialised, so nametable is already all tile 0.)
    // Sprite: solid, priority behind bg, at x=0. Use tile 1 ($0010) so
    // bg tile 0 ($0000) stays blank (transparent).
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x01, 0b0010_0000, 0);
    // Palette: universal bg = black, sprite $3F13 = blue.
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let blue = nes_color_to_argb(0x21);
    // bg transparent → behind-bg sprite visible.
    assert_eq!(
        px(&bus, 0, 1),
        blue,
        "behind-bg sprite visible over transparent bg"
    );
}

// =====================================================================
//  Sprite-to-sprite priority
// =====================================================================

#[test]
fn lower_oam_index_sprite_is_in_front() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_sprites_only(&mut bus);
    clear_oam(&mut bus);
    // Two overlapping solid sprites at (0, 0). Sprite 0 = blue, sprite 1 = red.
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0b00, 0); // palette 4 → $3F13
    write_sprite(&mut bus, 1, 0, 0x00, 0b01, 0); // palette 5 → $3F17
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]); // blue
    write_vram(&mut bus, 0x3F14, &[0x00, 0x00, 0x00, 0x16]); // red
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let blue = nes_color_to_argb(0x21);
    // Sprite 0 (lower index) wins.
    assert_eq!(px(&bus, 0, 1), blue, "sprite 0 in front of sprite 1");
}

#[test]
fn transparent_pixel_lets_lower_priority_sprite_show() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_sprites_only(&mut bus);
    clear_oam(&mut bus);
    // Sprite 0: left half solid, right half transparent.
    write_tile(&mut bus, 0x0000, HALF_TILE_P0, HALF_TILE_P1);
    // Sprite 1: fully solid, same position.
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0b00, 0); // palette 4 (blue), half-solid
    write_sprite(&mut bus, 1, 0, 0x01, 0b01, 0); // palette 5 (red), solid
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]); // blue
    write_vram(&mut bus, 0x3F14, &[0x00, 0x00, 0x00, 0x16]); // red
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let blue = nes_color_to_argb(0x21);
    let red = nes_color_to_argb(0x16);
    // Cols 0-3: sprite 0 opaque → blue.
    for x in 0..4 {
        assert_eq!(px(&bus, x, 1), blue, "col {} sprite 0 wins", x);
    }
    // Cols 4-7: sprite 0 transparent → sprite 1 shows → red.
    for x in 4..8 {
        assert_eq!(px(&bus, x, 1), red, "col {} sprite 1 shows", x);
    }
}

// =====================================================================
//  8-sprite-per-scanline limit + overflow flag
// =====================================================================

#[test]
fn max_8_sprites_per_scanline_overflow_flag_set() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_sprites_only(&mut bus);
    clear_oam(&mut bus);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    // 9 sprites all on scanline 1 (OAM Y = 0), at x = 0, 10, 20, ..., 80.
    for i in 0..9 {
        write_sprite(&mut bus, i, 0, 0x00, 0x00, (i * 10) as u8);
    }
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    // First 8 sprites render (x = 0, 10, ..., 70).
    for i in 0..8 {
        assert_eq!(px(&bus, i * 10, 1), white, "sprite {} rendered", i);
    }
    // 9th sprite (x = 80) does NOT render.
    assert_eq!(
        px(&bus, 80, 1),
        nes_color_to_argb(0x0E),
        "9th sprite dropped"
    );
    // Overflow flag set.
    assert_ne!(
        bus.ppu().ppustatus() & 0b0010_0000,
        0,
        "sprite overflow flag set"
    );
}

#[test]
fn overflow_flag_cleared_on_next_frame_without_overflow() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_sprites_only(&mut bus);
    clear_oam(&mut bus);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    // Frame 1: 9 sprites → overflow.
    for i in 0..9 {
        write_sprite(&mut bus, i, 0, 0x00, 0x00, (i * 10) as u8);
    }
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();
    assert_ne!(
        bus.ppu().ppustatus() & 0b0010_0000,
        0,
        "overflow set after frame 1"
    );
    // Frame 2: remove the 9th sprite (hide sprite 8).
    write_sprite(&mut bus, 8, 0xFF, 0x00, 0x00, 0);
    bus.render_frame();
    assert_eq!(
        bus.ppu().ppustatus() & 0b0010_0000,
        0,
        "overflow cleared after frame 2 (no overflow)"
    );
}

#[test]
fn exactly_8_sprites_no_overflow_flag() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_sprites_only(&mut bus);
    clear_oam(&mut bus);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    for i in 0..8 {
        write_sprite(&mut bus, i, 0, 0x00, 0x00, (i * 10) as u8);
    }
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    assert_eq!(
        bus.ppu().ppustatus() & 0b0010_0000,
        0,
        "no overflow with exactly 8 sprites"
    );
}

// =====================================================================
//  PPUMASK gating
// =====================================================================

#[test]
fn sprites_disabled_no_sprite_pixels() {
    let mut bus = Bus::with_cartridge(make_cart());
    // Enable bg only (not sprites). BG nametable is zero-init (tile 0,
    // blank) so the framebuffer is the universal bg color (black).
    bus.write(0x2001, 0b0000_1010);
    clear_oam(&mut bus);
    // Write a sprite + its tile pattern; sprites are disabled so neither
    // should affect the framebuffer. Use tile 1 ($0010) for the sprite
    // pattern so bg tile 0 ($0000) stays blank.
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x01, 0x00, 0);
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let black = nes_color_to_argb(0x0E);
    // No sprite anywhere — every pixel is the universal bg (black).
    for y in 0..SCREEN_HEIGHT {
        for x in 0..SCREEN_WIDTH {
            assert_eq!(px(&bus, x, y), black, "no sprite at ({},{})", x, y);
        }
    }
}

#[test]
fn sprite_left_8_pixels_clipped() {
    let mut bus = Bus::with_cartridge(make_cart());
    // Show sprites but NOT left 8 pixels of sprites.
    bus.write(0x2001, 0b0001_1000);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 0);
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    let black = nes_color_to_argb(0x0E);
    // Left 8 pixels of sprite clipped.
    for x in 0..8 {
        assert_eq!(px(&bus, x, 1), black, "left col {} clipped", x);
    }
    // The sprite is at x=0..7, all within the clip window → no sprite visible.
    // Place a second sprite at x=10 to confirm sprites still render outside
    // the clip window.
    write_sprite(&mut bus, 1, 0, 0x00, 0x00, 10);
    bus.render_frame();
    assert_eq!(px(&bus, 10, 1), white, "sprite outside clip window renders");
}

// =====================================================================
//  render_frame composites bg + sprites
// =====================================================================

#[test]
fn render_frame_composites_background_and_sprites() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    // Background: solid white (tile 1 everywhere).
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite: solid blue at (50, 0).
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 50);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    let blue = nes_color_to_argb(0x21);
    // Most of the screen is white (bg).
    assert_eq!(px(&bus, 0, 100), white, "bg fills screen");
    // Sprite region is blue.
    assert_eq!(px(&bus, 50, 1), blue, "sprite on top of bg");
    assert_eq!(px(&bus, 57, 8), blue);
    // Outside sprite: bg shows.
    assert_eq!(px(&bus, 58, 1), white);
}

#[test]
fn render_sprites_on_top_of_existing_background() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    // Background: solid white.
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    // Render background first, then add sprite and render sprites.
    bus.render_background();
    let white = nes_color_to_argb(0x20);
    assert_eq!(px(&bus, 0, 1), white, "bg rendered");
    // Now add sprite and render sprites only.
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 0);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    // NOTE: writing $3F10 via PPUDATA changed the t register; reset scroll.
    set_scroll(&mut bus, 0, 0);
    bus.render_sprites();
    let blue = nes_color_to_argb(0x21);
    assert_eq!(px(&bus, 0, 1), blue, "sprite composited on existing bg");
    assert_eq!(px(&bus, 8, 1), white, "bg preserved outside sprite");
}

// =====================================================================
//  Sprite partially off the right edge
// =====================================================================

#[test]
fn sprite_partially_off_right_edge() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_sprites_only(&mut bus);
    clear_oam(&mut bus);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite at x = 254 → only cols 0,1 visible (x = 254, 255).
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 254);
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    let black = nes_color_to_argb(0x0E);
    assert_eq!(px(&bus, 254, 1), white, "right-edge col 254 visible");
    assert_eq!(px(&bus, 255, 1), white, "right-edge col 255 visible");
    assert_eq!(px(&bus, 253, 1), black, "col 253 left of sprite");
}

// =====================================================================
//  bg_pattern buffer
// =====================================================================

#[test]
fn bg_pattern_buffer_populated_by_render_background() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    // Background: tile 1 (solid) everywhere.
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_background();

    let bgp = bus.ppu().bg_pattern();
    // Solid tile → pattern 3 everywhere.
    for y in 0..SCREEN_HEIGHT {
        for x in 0..SCREEN_WIDTH {
            assert_eq!(bgp[y * SCREEN_WIDTH + x], 3, "bg pattern at ({},{})", x, y);
        }
    }
}

#[test]
fn bg_pattern_zero_where_background_transparent() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    // Background: tile 0 everywhere (pattern 0 → transparent).
    // (VRAM zero-initialised.)
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    set_scroll(&mut bus, 0, 0);
    bus.render_background();

    let bgp = bus.ppu().bg_pattern();
    for y in 0..SCREEN_HEIGHT {
        for x in 0..SCREEN_WIDTH {
            assert_eq!(
                bgp[y * SCREEN_WIDTH + x],
                0,
                "transparent bg at ({},{})",
                x,
                y
            );
        }
    }
}

#[test]
fn bg_pattern_zero_when_background_disabled() {
    let mut bus = Bus::with_cartridge(make_cart());
    // Background disabled.
    enable_sprites_only(&mut bus);
    clear_oam(&mut bus);
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_background();

    let bgp = bus.ppu().bg_pattern();
    for y in 0..SCREEN_HEIGHT {
        for x in 0..SCREEN_WIDTH {
            assert_eq!(bgp[y * SCREEN_WIDTH + x], 0, "bg disabled → pattern 0");
        }
    }
}

// =====================================================================
//  Sprite-only frame (no background) — sprites still render
// =====================================================================

#[test]
fn sprites_render_with_background_disabled() {
    let mut bus = Bus::with_cartridge(make_cart());
    // Enable sprites only.
    enable_sprites_only(&mut bus);
    clear_oam(&mut bus);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 0);
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    let black = nes_color_to_argb(0x0E);
    assert_eq!(px(&bus, 0, 1), white, "sprite renders without bg");
    assert_eq!(px(&bus, 8, 1), black, "no sprite outside its area");
}

// =====================================================================
//  Multiple sprites on different scanlines
// =====================================================================

#[test]
fn multiple_sprites_on_different_scanlines() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_sprites_only(&mut bus);
    clear_oam(&mut bus);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite 0 at OAM Y=0 (scanlines 1-8), sprite 1 at OAM Y=100 (scanlines 101-108).
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 0);
    write_sprite(&mut bus, 1, 100, 0x00, 0x00, 0);
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    assert_eq!(px(&bus, 0, 1), white, "sprite 0 at top");
    assert_eq!(px(&bus, 0, 101), white, "sprite 1 lower");
    assert_eq!(
        px(&bus, 0, 50),
        nes_color_to_argb(0x0E),
        "gap between sprites"
    );
}

// =====================================================================
//  8×16 sprite mode (M23)
//
//  PPUCTRL bit 5 selects 8×16 sprite mode. In this mode:
//   - PPUCTRL bit 3 (sprite pattern table) is IGNORED.
//   - The pattern table is selected per sprite by bit 0 of the tile
//     index: even → $0000, odd → $1000.
//   - The tile base is `tile & 0xFE`; the top 8 rows fetch tile_base,
//     the bottom 8 rows fetch tile_base + 1.
//   - Vertical flip mirrors the entire 16-pixel range.
//   - A sprite is visible on scanlines (y+1)..=(y+16).
//
//  See: https://www.nesdev.org/wiki/PPU_OAM#8x16_sprites
// =====================================================================

/// Enable 8×16 sprite mode (PPUCTRL bit 5) + sprites + left column.
fn enable_8x16_sprites(bus: &mut Bus) {
    // PPUCTRL = bit5 (8x16) ; PPUMASK = show sprites + left.
    bus.write(0x2000, 0b0010_0000);
    bus.write(0x2001, 0b0001_0100);
}

/// A tile whose top half (rows 0-3) is solid and bottom half (rows 4-7)
/// is transparent. Used to distinguish top-tile from bottom-tile fetch.
const TOP_HALF_SOLID_P0: [u8; 8] = [0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00];
const TOP_HALF_SOLID_P1: [u8; 8] = [0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00];

#[test]
fn sprite_8x16_height_is_16_scanlines() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_8x16_sprites(&mut bus);
    clear_oam(&mut bus);
    // Tile 0 (even → $0000): solid everywhere (top + bottom halves).
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 0);
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    let black = nes_color_to_argb(0x0E);
    // OAM Y=0 → visible on scanlines 1..=16.
    for y in 1..=16 {
        assert_eq!(px(&bus, 0, y), white, "8x16 sprite visible at y={}", y);
    }
    assert_eq!(px(&bus, 0, 17), black, "scanline 17 outside 8x16 sprite");
    assert_eq!(px(&bus, 0, 0), black, "scanline 0 (Y delay)");
}

#[test]
fn sprite_8x16_even_tile_uses_0000_table() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_8x16_sprites(&mut bus);
    clear_oam(&mut bus);
    // Tile 0 at $0000: solid. Tile 0 at $1000: blank.
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    // (Tile 1 at $0010 is the bottom half of an 8x16 sprite with tile 0.)
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 0); // tile 0 = even → $0000
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    assert_eq!(px(&bus, 0, 1), white, "even tile fetches $0000");
}

#[test]
fn sprite_8x16_odd_tile_uses_1000_table() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_8x16_sprites(&mut bus);
    clear_oam(&mut bus);
    // Tile 1 at $0000: blank. Tile 1 at $1000: solid.
    // For an 8x16 sprite with tile=1 (odd): table=$1000, tile_base=0.
    // Top 8 rows → $1000 | (0<<4) = $1000 (tile 0 at $1000).
    // Bottom 8 rows → $1000 | (1<<4) = $1010 (tile 1 at $1000).
    write_tile(&mut bus, 0x1000, SOLID_TILE_P0, SOLID_TILE_P1); // top half
    write_tile(&mut bus, 0x1010, SOLID_TILE_P0, SOLID_TILE_P1); // bottom half
                                                                // Leave $0000/$0010 blank so we can confirm the sprite did NOT read there.
    write_sprite(&mut bus, 0, 0, 0x01, 0x00, 0); // tile 1 = odd → $1000
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    let black = nes_color_to_argb(0x0E);
    // Top half (scanlines 1-8) and bottom half (9-16) both solid → $1000.
    assert_eq!(px(&bus, 0, 1), white, "odd tile top half from $1000");
    assert_eq!(px(&bus, 0, 9), white, "odd tile bottom half from $1000+1");
    assert_eq!(px(&bus, 0, 0), black, "Y delay");
}

#[test]
fn sprite_8x16_ppuctrl_bit3_ignored() {
    let mut bus = Bus::with_cartridge(make_cart());
    // PPUCTRL = bit5 (8x16) | bit3 (sprite table $1000). Bit 3 must be
    // ignored in 8x16 mode; an even tile still reads $0000.
    bus.write(0x2000, 0b0010_1000);
    bus.write(0x2001, 0b0001_0100);
    clear_oam(&mut bus);
    // Tile 0 at $0000: solid. Tile 0 at $1000: blank.
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 0); // even → $0000 despite bit3
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    assert_eq!(px(&bus, 0, 1), white, "bit3 ignored; even tile uses $0000");
}

#[test]
fn sprite_8x16_top_and_bottom_halves_fetch_different_tiles() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_8x16_sprites(&mut bus);
    clear_oam(&mut bus);
    // Tile 0 at $0000 (top half): top 4 rows solid, bottom 4 transparent.
    write_tile(&mut bus, 0x0000, TOP_HALF_SOLID_P0, TOP_HALF_SOLID_P1);
    // Tile 1 at $0010 (bottom half): solid everywhere.
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 0); // tile_base=0
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    let black = nes_color_to_argb(0x0E);
    // Top tile (rows 0-7 → scanlines 1-8): rows 0-3 solid, 4-7 transparent.
    assert_eq!(px(&bus, 0, 1), white, "top tile row 0 solid");
    assert_eq!(px(&bus, 0, 4), white, "top tile row 3 solid");
    assert_eq!(px(&bus, 0, 5), black, "top tile row 4 transparent");
    assert_eq!(px(&bus, 0, 8), black, "top tile row 7 transparent");
    // Bottom tile (rows 8-15 → scanlines 9-16): solid everywhere.
    assert_eq!(px(&bus, 0, 9), white, "bottom tile row 0 solid");
    assert_eq!(px(&bus, 0, 16), white, "bottom tile row 7 solid");
}

#[test]
fn sprite_8x16_vflip_mirrors_full_16px_range() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_8x16_sprites(&mut bus);
    clear_oam(&mut bus);
    // Tile 0 at $0000 (top half of sprite): top 4 rows solid, bottom 4 transparent.
    write_tile(&mut bus, 0x0000, TOP_HALF_SOLID_P0, TOP_HALF_SOLID_P1);
    // Tile 1 at $0010 (bottom half of sprite): solid everywhere.
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    // vflip → the bottom of the bottom-tile appears at the top of the sprite.
    write_sprite(&mut bus, 0, 0, 0x00, 0b1000_0000, 0);
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    let black = nes_color_to_argb(0x0E);
    // Without flip: scanlines 1-4 solid (top tile rows 0-3), 5-8 transparent,
    // 9-16 solid (bottom tile). With vflip the order reverses over 16px:
    //   scanline 1 ← original row 15 (bottom tile row 7) → solid
    //   scanline 8 ← original row 8  (bottom tile row 0) → solid
    //   scanline 9 ← original row 7  (top tile row 7)    → transparent
    //   scanline 12 ← original row 4 (top tile row 4)    → transparent
    //   scanline 13 ← original row 3 (top tile row 3)    → solid
    //   scanline 16 ← original row 0 (top tile row 0)    → solid
    assert_eq!(px(&bus, 0, 1), white, "vflip: bottom-tile row 7 at top");
    assert_eq!(px(&bus, 0, 8), white, "vflip: bottom-tile row 0");
    assert_eq!(px(&bus, 0, 9), black, "vflip: top-tile row 7 transparent");
    assert_eq!(px(&bus, 0, 12), black, "vflip: top-tile row 4 transparent");
    assert_eq!(px(&bus, 0, 13), white, "vflip: top-tile row 3 solid");
    assert_eq!(px(&bus, 0, 16), white, "vflip: top-tile row 0 at bottom");
}

#[test]
fn sprite_8x16_hflip_mirrors_columns() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_8x16_sprites(&mut bus);
    clear_oam(&mut bus);
    // Tile 0 at $0000: left half solid, right half transparent.
    write_tile(&mut bus, 0x0000, HALF_TILE_P0, HALF_TILE_P1);
    write_tile(&mut bus, 0x0010, HALF_TILE_P0, HALF_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0b0100_0000, 0); // hflip
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    let black = nes_color_to_argb(0x0E);
    // hflip: cols 0-3 transparent, 4-7 solid (mirrored within the 8px width).
    for x in 0..4 {
        assert_eq!(px(&bus, x, 1), black, "hflipped col {} transparent", x);
    }
    for x in 4..8 {
        assert_eq!(px(&bus, x, 1), white, "hflipped col {} solid", x);
    }
}

#[test]
fn sprite_8x16_sprite_zero_hit_triggers() {
    let mut bus = Bus::with_cartridge(make_cart());
    // Enable bg + sprites + left columns, 8x16 mode.
    bus.write(0x2000, 0b0010_0000);
    bus.write(0x2001, 0b0001_1110);
    clear_oam(&mut bus);
    // Background: solid white (nametable tile 1, tile 1 solid).
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite 0: solid, at (0,0), 8x16 (tile 0 even → $0000).
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 0);
    // Palette: bg $3F03 = white, sprite $3F13 = blue.
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    assert!(bus.ppu().sprite_zero_hit(), "8x16 sprite 0 hit triggers");
}

#[test]
fn sprite_8x16_overflow_still_8_per_scanline() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_8x16_sprites(&mut bus);
    clear_oam(&mut bus);
    // 9 sprites all in range on scanline 1 (OAM Y=0, 8x16 → scanlines 1-16).
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    for i in 0..9 {
        write_sprite(&mut bus, i, 0, 0x00, 0x00, (i * 8) as u8);
    }
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    assert!(bus.ppu().sprite_overflow(), "9th 8x16 sprite sets overflow");
}

#[test]
fn sprite_8x16_partial_bottom_off_screen() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_8x16_sprites(&mut bus);
    clear_oam(&mut bus);
    // OAM Y = 230 → visible on scanlines 231..=246. Only scanlines 231-239
    // are on-screen (SCREEN_HEIGHT=240); the rest are off-screen.
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 230, 0x00, 0x00, 0);
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x20]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let white = nes_color_to_argb(0x20);
    // Scanlines 231-239 should be solid (top 9 rows of the 8x16 sprite).
    for y in 231..=239 {
        assert_eq!(px(&bus, 0, y), white, "8x16 visible at y={}", y);
    }
}
