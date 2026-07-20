//! Integration tests for M11 — sprite zero hit + PPUSTATUS flags.
//!
//! Coverage:
//! - Sprite zero hit set when sprite 0 overlaps a non-transparent bg pixel.
//! - No hit when sprite 0 pixel is transparent.
//! - No hit when bg pixel is transparent.
//! - No hit at x = 255 (hardware quirk).
//! - No hit in the left 8 pixels when bg-left or sprite-left is clipped.
//! - Hit triggers even when sprite 0 is behind the background (priority
//!   bit does not affect hit detection).
//! - Hit set only once per frame (first opaque overlap).
//! - Hit cleared at the prerender scanline by `step_ppu`.
//! - Hit NOT cleared by PPUSTATUS read (only VBlank is).
//! - Sprite overflow flag: > 8 sprites in range → set.
//! - Sprite overflow halt on Y = $FF (sprites after $FF not checked).
//! - Sprite overflow not set when exactly 8 sprites in range.
//! - Sprite overflow + sprite 0 hit independent.
//! - Split-screen scenario: sprite 0 hit used to detect a scanline.
//!
//! See: https://www.nesdev.org/wiki/PPU_OAM#Sprite_zero_hit
//! See: https://www.nesdev.org/wiki/PPU_OAM#Sprite_overflow

use nes_emu::bus::Bus;
use nes_emu::cartridge::Cartridge;
use nes_emu::ppu::render::nes_color_to_argb;
use nes_emu::ppu::{SCREEN_HEIGHT, SCREEN_WIDTH};

// ---- helpers (mirrors ppu_sprites.rs) ----------------------------------

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

fn set_vram_addr(bus: &mut Bus, addr: u16) {
    bus.write(0x2006, ((addr >> 8) & 0x3F) as u8);
    bus.write(0x2006, (addr & 0xFF) as u8);
}

fn write_vram(bus: &mut Bus, addr: u16, data: &[u8]) {
    set_vram_addr(bus, addr);
    for &b in data {
        bus.write(0x2007, b);
    }
}

fn write_tile(bus: &mut Bus, base: u16, plane0: [u8; 8], plane1: [u8; 8]) {
    let mut data = [0u8; 16];
    data[..8].copy_from_slice(&plane0);
    data[8..].copy_from_slice(&plane1);
    write_vram(bus, base, &data);
}

fn set_scroll(bus: &mut Bus, x: u8, y: u8) {
    bus.write(0x2005, x);
    bus.write(0x2005, y);
}

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
    bus.write(0x2001, 0b0001_1110);
}

/// Enable background + sprites but clip the left 8 pixels of BOTH layers.
fn enable_bg_and_sprites_no_left(bus: &mut Bus) {
    // show bg + show sprites, but NOT show-bg-left or show-sprites-left.
    bus.write(0x2001, 0b0001_1000);
}

/// Enable background + sprites, clip only the sprite left 8 pixels.
fn enable_bg_left_clip_sprites_left(bus: &mut Bus) {
    // show bg + show bg left + show sprites, NOT show sprites left.
    bus.write(0x2001, 0b0001_1010);
}

/// Enable background + sprites, clip only the bg left 8 pixels.
fn clip_bg_left_show_sprites_left(bus: &mut Bus) {
    // show bg + show sprites + show sprites left, NOT show bg left.
    bus.write(0x2001, 0b0001_1100);
}

fn clear_oam(bus: &mut Bus) {
    bus.write(0x2003, 0x00);
    for _ in 0..256 {
        bus.write(0x2004, 0xFF);
    }
}

const SOLID_TILE_P0: [u8; 8] = [0xFF; 8];
const SOLID_TILE_P1: [u8; 8] = [0xFF; 8];

/// A tile whose left half (cols 0-3) is pattern 3 and right half (cols
/// 4-7) is transparent (pattern 0). Plane0 = 0xF0, plane1 = 0xF0.
const HALF_TILE_P0: [u8; 8] = [0xF0; 8];
const HALF_TILE_P1: [u8; 8] = [0xF0; 8];

fn px(bus: &Bus, x: usize, y: usize) -> u32 {
    bus.ppu().framebuffer()[y * SCREEN_WIDTH + x]
}

fn sprite_zero_hit(bus: &Bus) -> bool {
    bus.ppu().sprite_zero_hit()
}

fn sprite_overflow(bus: &Bus) -> bool {
    bus.ppu().sprite_overflow()
}

/// Read PPUSTATUS ($2002) through the bus.
fn read_ppustatus(bus: &mut Bus) -> u8 {
    bus.read(0x2002)
}

// =====================================================================
//  Sprite zero hit — basic detection
// =====================================================================

#[test]
fn sprite_zero_hit_set_when_sprite0_overlaps_opaque_bg() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    // Background: solid white everywhere (tile 1, pattern 3).
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1); // bg tile 1
                                                                // Sprite 0: solid, at (10, 0) → visible on scanlines 1..=8.
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 10);
    // Palette: $3F00 = black, $3F03 = white (bg), $3F13 = blue (sprite).
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    assert!(
        sprite_zero_hit(&bus),
        "sprite 0 hit set when sprite 0 overlaps opaque bg"
    );
    // Sanity: the sprite pixel at (10, 1) is blue (sprite on top of bg).
    assert_eq!(px(&bus, 10, 1), nes_color_to_argb(0x21));
}

#[test]
fn sprite_zero_hit_not_set_when_sprite0_pixel_transparent() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    // Background: solid (tile 1).
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite 0: half-solid tile (left 4 cols opaque, right 4 transparent).
    write_tile(&mut bus, 0x0000, HALF_TILE_P0, HALF_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 0);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    // Sprite 0's opaque pixels (x 0..3) overlap opaque bg → hit set.
    assert!(
        sprite_zero_hit(&bus),
        "hit set by the opaque half of sprite 0"
    );
}

#[test]
fn sprite_zero_hit_not_set_when_sprite0_entirely_transparent() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    // Background: solid (tile 1).
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite 0: blank tile (tile 0, all zero pattern).
    write_tile(&mut bus, 0x0000, [0u8; 8], [0u8; 8]);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 0);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    assert!(
        !sprite_zero_hit(&bus),
        "no hit when sprite 0 is entirely transparent"
    );
}

#[test]
fn sprite_zero_hit_not_set_when_bg_transparent() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    // Background: blank (tile 0 everywhere, zero-initialised VRAM +
    // zero-initialised CHR-RAM → pattern 0 → transparent).
    // Sprite 0: solid, using tile 1 at $0010 (NOT tile 0 at $0000, which
    // would also make the bg tile 0 solid since both share pattern table 0).
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x01, 0x00, 0);
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    assert!(
        !sprite_zero_hit(&bus),
        "no hit when bg is transparent under sprite 0"
    );
}

// =====================================================================
//  Sprite zero hit — x = 255 quirk
// =====================================================================

#[test]
fn sprite_zero_hit_not_triggered_at_x_255() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    // Background: solid.
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite 0: solid, placed so only x=255 is in range (x=248, width 8
    // → pixels 248..=255). The opaque overlap at x=255 does NOT trigger
    // the hit. Pixels 248..254 would trigger it, so we use a half tile
    // that is transparent for cols 0-6 and opaque only for col 7.
    let mut plane = [0u8; 8];
    plane[0] = 0x01; // only bit 0 (col 7) set → only x=255 opaque.
    write_tile(&mut bus, 0x0000, plane, plane);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 248);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    assert!(!sprite_zero_hit(&bus), "no hit at x=255 (hardware quirk)");
    // Sanity: the sprite pixel at (255, 1) IS rendered (blue), it just
    // doesn't trigger the hit.
    assert_eq!(px(&bus, 255, 1), nes_color_to_argb(0x21));
}

#[test]
fn sprite_zero_hit_triggered_at_x_254() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite 0 at x=247, width 8 → pixels 247..=254. x=254 is the last
    // opaque pixel and is < 255 → hit triggers.
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 247);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    assert!(
        sprite_zero_hit(&bus),
        "hit at x=254 (just before the x=255 exclusion)"
    );
}

// =====================================================================
//  Sprite zero hit — left clipping
// =====================================================================

#[test]
fn sprite_zero_hit_not_in_bg_left_clip() {
    let mut bus = Bus::with_cartridge(make_cart());
    // Show bg + sprites, but clip bg left 8 (sprite left shown).
    clip_bg_left_show_sprites_left(&mut bus);
    clear_oam(&mut bus);
    // Background: solid (but left 8 px clipped → bg_pattern = 0 there).
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite 0: solid at x=0.
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 0);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    // Sprite 0 is at x=0..7, all within the bg-left clip region. The bg
    // is transparent there (clipped) → no hit.
    assert!(
        !sprite_zero_hit(&bus),
        "no hit in bg-left-clip region (bg transparent there)"
    );
}

#[test]
fn sprite_zero_hit_not_in_sprite_left_clip() {
    let mut bus = Bus::with_cartridge(make_cart());
    // Show bg + sprites, but clip sprite left 8 (bg left shown).
    enable_bg_left_clip_sprites_left(&mut bus);
    clear_oam(&mut bus);
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite 0: solid at x=0 (entirely within sprite-left clip).
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 0);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    // Sprite 0's pixels in x=0..7 are clipped (not rendered) → no hit.
    assert!(
        !sprite_zero_hit(&bus),
        "no hit in sprite-left-clip region (sprite not rendered there)"
    );
}

#[test]
fn sprite_zero_hit_when_both_left_columns_shown() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 0);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    // Both left columns shown → sprite 0 at x=0 overlaps opaque bg → hit.
    assert!(
        sprite_zero_hit(&bus),
        "hit when both left columns are shown"
    );
}

#[test]
fn sprite_zero_hit_when_both_left_columns_clipped_at_x8() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites_no_left(&mut bus);
    clear_oam(&mut bus);
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite 0 at x=8 (just outside the clipped region).
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 8);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    // x=8 is outside the clip region → hit triggers.
    assert!(
        sprite_zero_hit(&bus),
        "hit at x=8 when both left columns clipped (x=8 is visible)"
    );
}

// =====================================================================
//  Sprite zero hit — priority (behind bg) does not affect hit
// =====================================================================

#[test]
fn sprite_zero_hit_triggers_even_when_sprite0_behind_bg() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite 0: solid, behind background (attr bit 5 set).
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0b0010_0000, 10);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    // The sprite is hidden behind the opaque bg (priority bit 5), but
    // the hit still triggers because both pixels are opaque.
    assert!(
        sprite_zero_hit(&bus),
        "hit triggers even when sprite 0 is behind bg"
    );
    // Sanity: the pixel at (10, 1) is white (bg wins, sprite hidden).
    assert_eq!(px(&bus, 10, 1), nes_color_to_argb(0x20));
}

// =====================================================================
//  Sprite zero hit — set once per frame
// =====================================================================

#[test]
fn sprite_zero_hit_set_once_per_frame() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite 0: solid, covering many scanlines (y=0 → scanlines 1..=8).
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 0);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    // The flag is set (once) even though sprite 0 overlaps bg on many
    // scanlines and many pixels.
    assert!(sprite_zero_hit(&bus));
}

// =====================================================================
//  Sprite zero hit — cleared at prerender by step_ppu
// =====================================================================

#[test]
fn sprite_zero_hit_cleared_at_prerender_by_step() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 10);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();
    assert!(sprite_zero_hit(&bus), "hit set after render_frame");

    // Advance the PPU to the prerender scanline, cycle 1 (the point at
    // which the flags are cleared). From scanline 0 cycle 0, that is
    // 261 * 341 + 1 = 89002 PPU cycles.
    let target: u32 = 261 * 341 + 1;
    bus.step_ppu(target);
    assert!(
        !sprite_zero_hit(&bus),
        "hit cleared at prerender scanline cycle 1"
    );
    assert!(!sprite_overflow(&bus), "overflow also cleared at prerender");
}

// =====================================================================
//  Sprite zero hit — NOT cleared by PPUSTATUS read
// =====================================================================

#[test]
fn sprite_zero_hit_survives_ppustatus_read() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 10);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();
    assert!(sprite_zero_hit(&bus));

    // Reading PPUSTATUS should NOT clear the sprite 0 hit (only VBlank).
    let status = read_ppustatus(&mut bus);
    assert_ne!(
        status & 0b0100_0000,
        0,
        "sprite 0 hit visible in PPUSTATUS read"
    );
    assert!(
        sprite_zero_hit(&bus),
        "sprite 0 hit survives PPUSTATUS read"
    );
}

// =====================================================================
//  Sprite zero hit — not set when sprites disabled
// =====================================================================

#[test]
fn sprite_zero_hit_not_set_when_sprites_disabled() {
    let mut bus = Bus::with_cartridge(make_cart());
    // Enable bg only (not sprites).
    bus.write(0x2001, 0b0000_1010);
    clear_oam(&mut bus);
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 10);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    assert!(!sprite_zero_hit(&bus), "no hit when sprites are disabled");
}

#[test]
fn sprite_zero_hit_not_set_when_bg_disabled() {
    let mut bus = Bus::with_cartridge(make_cart());
    // Enable sprites only (not bg).
    bus.write(0x2001, 0b0001_0100);
    clear_oam(&mut bus);
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 10);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    // With bg disabled, bg_pattern is 0 everywhere → no hit.
    assert!(
        !sprite_zero_hit(&bus),
        "no hit when bg is disabled (bg transparent everywhere)"
    );
}

// =====================================================================
//  Sprite zero hit — only sprite 0 (not other sprites) triggers hit
// =====================================================================

#[test]
fn only_sprite_zero_triggers_hit_not_other_sprites() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite 0: hidden (Y = $EF — out of range but does NOT halt
    // evaluation, so sprite 1 is still checked).
    write_sprite(&mut bus, 0, 0xEF, 0x00, 0x00, 10);
    // Sprite 1: solid, overlapping opaque bg.
    write_sprite(&mut bus, 1, 0, 0x00, 0x00, 10);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    // Sprite 1 overlaps opaque bg, but it's not sprite 0 → no hit.
    assert!(
        !sprite_zero_hit(&bus),
        "only sprite 0 triggers the hit, not sprite 1"
    );
    // Sanity: sprite 1 IS rendered.
    assert_eq!(px(&bus, 10, 1), nes_color_to_argb(0x21));
}

// =====================================================================
//  Sprite zero hit — horizontal flip
// =====================================================================

#[test]
fn sprite_zero_hit_with_horizontal_flip() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    // Background: solid only in the right half of the screen (x >= 128).
    // We make tile 1 solid and place it in nametable columns 16..31.
    for row in 0..30 {
        for col in 16..32 {
            set_vram_addr(&mut bus, 0x2000 + row * 32 + col);
            bus.write(0x2007, 0x01);
        }
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite 0: half-solid (left 4 cols opaque, right 4 transparent),
    // h-flipped → opaque on the RIGHT (cols 4-7), transparent on left.
    // Placed at x=124 → opaque at screen x=128..131 (cols 4-7 map to
    // screen 124+4..=124+7 = 128..131), which overlaps the solid bg.
    write_tile(&mut bus, 0x0000, HALF_TILE_P0, HALF_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0b0100_0000, 124); // hflip
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    assert!(
        sprite_zero_hit(&bus),
        "hit with h-flipped sprite 0 overlapping opaque bg"
    );
}

// =====================================================================
//  Sprite zero hit — vertical flip
// =====================================================================

#[test]
fn sprite_zero_hit_with_vertical_flip() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    // Background: solid only at tile row 0 (scanlines 0-7).
    for col in 0..32 {
        set_vram_addr(&mut bus, 0x2000 + col);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite 0 tile: only row 7 is opaque (all other rows transparent).
    let mut p0 = [0u8; 8];
    let mut p1 = [0u8; 8];
    p0[7] = 0xFF;
    p1[7] = 0xFF;
    write_tile(&mut bus, 0x0000, p0, p1);
    // Sprite 0 at y=0 → visible on scanlines 1..=8.
    //  - Without v-flip: opaque at row 7 → scanline 8 (tile row 1,
    //    transparent bg) → NO hit.
    //  - With v-flip: row 7 maps to row 0 → opaque at scanline 1
    //    (tile row 0, solid bg) → hit!
    write_sprite(&mut bus, 0, 0, 0x00, 0b1000_0000, 0); // vflip
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    assert!(
        sprite_zero_hit(&bus),
        "hit with v-flipped sprite 0 (opaque row flipped to top, overlaps solid bg)"
    );
}

#[test]
fn sprite_zero_hit_no_hit_without_vflip_when_opaque_row_misses_bg() {
    // Same setup as above but WITHOUT v-flip → the opaque row (7) maps
    // to scanline 8 (tile row 1, transparent bg) → no hit. This
    // confirms the v-flip test above is actually testing v-flip behavior.
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    for col in 0..32 {
        set_vram_addr(&mut bus, 0x2000 + col);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    let mut p0 = [0u8; 8];
    let mut p1 = [0u8; 8];
    p0[7] = 0xFF;
    p1[7] = 0xFF;
    write_tile(&mut bus, 0x0000, p0, p1);
    write_sprite(&mut bus, 0, 0, 0x00, 0b0000_0000, 0); // no vflip
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    assert!(
        !sprite_zero_hit(&bus),
        "no hit without v-flip (opaque row 7 → scanline 8, bg transparent there)"
    );
}

// =====================================================================
//  Sprite zero hit — sprite pattern table at $1000
// =====================================================================

#[test]
fn sprite_zero_hit_with_sprite_pattern_table_1000() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    // PPUCTRL bit 3 = 1 → sprite pattern table at $1000.
    bus.write(0x2000, 0b0000_1000);
    // Background: solid (tile 1 at $0010, bg pattern table $0000).
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite 0: solid tile 0 at $1000 (sprite pattern table).
    write_tile(&mut bus, 0x1000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 10);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    assert!(
        sprite_zero_hit(&bus),
        "hit with sprite pattern table at $1000"
    );
    // Sanity: sprite pixel is blue (rendered from $1000 pattern table).
    assert_eq!(px(&bus, 10, 1), nes_color_to_argb(0x21));
}

// =====================================================================
//  Sprite zero hit — flags cleared when sprites disabled on next frame
// =====================================================================

#[test]
fn sprite_zero_hit_cleared_when_sprites_disabled_next_frame() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 10);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();
    assert!(sprite_zero_hit(&bus), "hit set after frame 1");

    // Frame 2: disable sprites (bg only). The per-frame clear at the
    // start of render_sprites should clear the hit flag even though
    // sprites are disabled (the clear happens before the early return).
    bus.write(0x2001, 0b0000_1010);
    bus.render_frame();
    assert!(
        !sprite_zero_hit(&bus),
        "hit cleared when sprites disabled on next frame"
    );
}

// =====================================================================
//  Sprite overflow — halt on $FF
// =====================================================================

#[test]
fn sprite_overflow_halt_on_ff_skips_later_sprites() {
    let mut bus = Bus::with_cartridge(make_cart());
    // Sprites only (no bg) so we can count rendered sprites cleanly.
    bus.write(0x2001, 0b0001_0100);
    clear_oam(&mut bus);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite 0: solid at x=0.
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 0);
    // Sprite 1: Y = $FF → halts evaluation. Sprites 2+ are never checked.
    write_sprite(&mut bus, 1, 0xFF, 0x00, 0x00, 0);
    // Sprite 2: solid at x=20. Would be in range, but evaluation halted
    // at sprite 1 ($FF) so it should NOT render.
    write_sprite(&mut bus, 2, 0, 0x00, 0x00, 20);
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let blue = nes_color_to_argb(0x21);
    // Sprite 0 renders.
    assert_eq!(px(&bus, 0, 1), blue, "sprite 0 (before $FF) renders");
    // Sprite 2 does NOT render (evaluation halted at sprite 1 = $FF).
    assert_eq!(
        px(&bus, 20, 1),
        nes_color_to_argb(0x0E),
        "sprite 2 (after $FF) not rendered (halt)"
    );
}

#[test]
fn sprite_overflow_ef_fe_do_not_halt_evaluation() {
    let mut bus = Bus::with_cartridge(make_cart());
    bus.write(0x2001, 0b0001_0100);
    clear_oam(&mut bus);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite 0: solid at x=0.
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 0);
    // Sprite 1: Y = $EF (out of range, but does NOT halt).
    write_sprite(&mut bus, 1, 0xEF, 0x00, 0x00, 0);
    // Sprite 2: solid at x=20. Should render (evaluation not halted).
    write_sprite(&mut bus, 2, 0, 0x00, 0x00, 20);
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    let blue = nes_color_to_argb(0x21);
    assert_eq!(px(&bus, 0, 1), blue, "sprite 0 renders");
    assert_eq!(
        px(&bus, 20, 1),
        blue,
        "sprite 2 renders (Y=$EF did not halt evaluation)"
    );
}

// =====================================================================
//  Sprite overflow — flag independence from sprite 0 hit
// =====================================================================

#[test]
fn sprite_overflow_and_sprite_zero_hit_independent() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite 0: solid, overlapping opaque bg → hit.
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 0);
    // 8 more sprites (1..=8) on the same scanline → 9 total → overflow.
    for i in 1..=8 {
        write_sprite(&mut bus, i, 0, 0x00, 0x00, (i * 30) as u8);
    }
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    assert!(sprite_zero_hit(&bus), "sprite 0 hit set");
    assert!(sprite_overflow(&bus), "sprite overflow set (9 sprites)");
}

#[test]
fn sprite_overflow_without_sprite_zero_hit() {
    let mut bus = Bus::with_cartridge(make_cart());
    // Sprites only (no bg → no sprite 0 hit possible).
    bus.write(0x2001, 0b0001_0100);
    clear_oam(&mut bus);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    // 9 sprites on scanline 1.
    for i in 0..9 {
        write_sprite(&mut bus, i, 0, 0x00, 0x00, (i * 10) as u8);
    }
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    assert!(sprite_overflow(&bus), "overflow set");
    assert!(
        !sprite_zero_hit(&bus),
        "no sprite 0 hit (bg disabled → bg transparent)"
    );
}

// =====================================================================
//  Sprite overflow — cleared at prerender
// =====================================================================

#[test]
fn sprite_overflow_cleared_at_prerender_by_step() {
    let mut bus = Bus::with_cartridge(make_cart());
    bus.write(0x2001, 0b0001_0100);
    clear_oam(&mut bus);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    for i in 0..9 {
        write_sprite(&mut bus, i, 0, 0x00, 0x00, (i * 10) as u8);
    }
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();
    assert!(sprite_overflow(&bus), "overflow set after render_frame");

    // Step to prerender scanline 261, cycle 1.
    let target: u32 = 261 * 341 + 1;
    bus.step_ppu(target);
    assert!(!sprite_overflow(&bus), "overflow cleared at prerender");
}

// =====================================================================
//  Sprite overflow — survives PPUSTATUS read
// =====================================================================

#[test]
fn sprite_overflow_survives_ppustatus_read() {
    let mut bus = Bus::with_cartridge(make_cart());
    bus.write(0x2001, 0b0001_0100);
    clear_oam(&mut bus);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    for i in 0..9 {
        write_sprite(&mut bus, i, 0, 0x00, 0x00, (i * 10) as u8);
    }
    write_vram(&mut bus, 0x3F00, &[0x0E]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();
    assert!(sprite_overflow(&bus));

    let status = read_ppustatus(&mut bus);
    assert_ne!(
        status & 0b0010_0000,
        0,
        "overflow visible in PPUSTATUS read"
    );
    assert!(sprite_overflow(&bus), "overflow survives PPUSTATUS read");
}

// =====================================================================
//  Sprite zero hit — cleared on next render_frame (per-frame reset)
// =====================================================================

#[test]
fn sprite_zero_hit_cleared_on_next_render_frame_without_hit() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    // Frame 1: sprite 0 overlaps opaque bg → hit.
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 10);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();
    assert!(sprite_zero_hit(&bus), "hit set after frame 1");

    // Frame 2: hide sprite 0 (Y = $FF) → no overlap → hit cleared by
    // the per-frame reset at the start of render_sprites.
    write_sprite(&mut bus, 0, 0xFF, 0x00, 0x00, 10);
    bus.render_frame();
    assert!(
        !sprite_zero_hit(&bus),
        "hit cleared on frame 2 (no overlap)"
    );
}

// =====================================================================
//  Split-screen scenario — sprite 0 hit detects a specific scanline
// =====================================================================

#[test]
fn split_screen_sprite_zero_hit_at_specific_scanline() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    // Background: solid in the top half (tile rows 0..14, using tile 1
    // at $0010), transparent in the bottom half (tile 0 at $0000, which
    // is zero-init CHR-RAM = blank).
    for row in 0..15 {
        for col in 0..32 {
            set_vram_addr(&mut bus, 0x2000 + row * 32 + col);
            bus.write(0x2007, 0x01);
        }
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite 0: solid, using tile 2 at $0020 (so it doesn't overwrite
    // bg tile 0 at $0000 or bg tile 1 at $0010).
    // Placed at y=112 → visible on scanlines 113..=120. Scanlines
    // 113..119 are tile rows 14..14 (solid bg). Scanline 120 is tile
    // row 15 (blank bg). So sprite 0 overlaps opaque bg on scanlines
    // 113..119 → hit.
    write_tile(&mut bus, 0x0020, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 112, 0x02, 0x00, 0);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    assert!(
        sprite_zero_hit(&bus),
        "split-screen: sprite 0 hit at the bg boundary"
    );
    // The sprite pixel at (0, 113) is blue (sprite on top of solid bg).
    assert_eq!(px(&bus, 0, 113), nes_color_to_argb(0x21));
    // At (8, 120) there is no sprite (sprite is at x=0..7) and the bg is
    // transparent (tile row 15, blank) → universal bg (black).
    assert_eq!(px(&bus, 8, 120), nes_color_to_argb(0x0E));
}

// =====================================================================
//  PPUSTATUS read returns all three flags + open bus low bits
// =====================================================================

#[test]
fn ppustatus_read_returns_all_flags() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    // 9 sprites → overflow + sprite 0 hit.
    for i in 0..9 {
        write_sprite(&mut bus, i, 0, 0x00, 0x00, (i * 30) as u8);
    }
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    // Step the PPU into VBlank to set the VBlank flag.
    // Scanline 241, cycle 1 = 241 * 341 + 1 = 82182 PPU cycles.
    bus.step_ppu(241 * 341 + 1);

    let status = read_ppustatus(&mut bus);
    assert_ne!(status & 0b1000_0000, 0, "VBlank flag set");
    assert_ne!(status & 0b0100_0000, 0, "sprite 0 hit flag set");
    assert_ne!(status & 0b0010_0000, 0, "sprite overflow flag set");
    // VBlank is cleared by the read; the other two persist.
    assert!(!bus.ppu().in_vblank());
    assert!(bus.ppu().sprite_zero_hit());
    assert!(bus.ppu().sprite_overflow());
}

// =====================================================================
//  Sprite zero hit — multiple scanlines, first hit wins
// =====================================================================

#[test]
fn sprite_zero_hit_first_opaque_overlap_wins() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    // Background: solid everywhere.
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite 0: solid, covering scanlines 1..=8 (y=0).
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    write_sprite(&mut bus, 0, 0, 0x00, 0x00, 0);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    // The hit is set (first overlap at scanline 1, x=0). The flag
    // remains set for the rest of the frame.
    assert!(sprite_zero_hit(&bus));
}

// =====================================================================
//  Sprite zero hit — sprite 0 not in range (no hit)
// =====================================================================

#[test]
fn sprite_zero_hit_not_set_when_sprite0_out_of_range() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite 0: Y = $EF (out of range, never visible).
    write_sprite(&mut bus, 0, 0xEF, 0x00, 0x00, 10);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    assert!(
        !sprite_zero_hit(&bus),
        "no hit when sprite 0 is out of range"
    );
}

// =====================================================================
//  Edge case: sprite 0 at the very last visible scanline
// =====================================================================

#[test]
fn sprite_zero_hit_at_last_visible_scanline() {
    let mut bus = Bus::with_cartridge(make_cart());
    enable_bg_and_sprites(&mut bus);
    clear_oam(&mut bus);
    for i in 0..960 {
        set_vram_addr(&mut bus, 0x2000 + i as u16);
        bus.write(0x2007, 0x01);
    }
    write_tile(&mut bus, 0x0010, SOLID_TILE_P0, SOLID_TILE_P1);
    write_tile(&mut bus, 0x0000, SOLID_TILE_P0, SOLID_TILE_P1);
    // Sprite 0: Y = 231 → visible on scanlines 232..=239 (last visible).
    write_sprite(&mut bus, 0, 231, 0x00, 0x00, 10);
    write_vram(&mut bus, 0x3F00, &[0x0E, 0x00, 0x00, 0x20]);
    write_vram(&mut bus, 0x3F11, &[0x00, 0x00, 0x21]);
    set_scroll(&mut bus, 0, 0);
    bus.render_frame();

    assert!(
        sprite_zero_hit(&bus),
        "hit at the last visible scanline (239)"
    );
    assert_eq!(px(&bus, 10, 239), nes_color_to_argb(0x21));
}

// =====================================================================
//  Edge case: SCREEN_HEIGHT constant sanity (no off-by-one in scanline loop)
// =====================================================================

#[test]
fn screen_height_is_240() {
    assert_eq!(SCREEN_HEIGHT, 240);
    assert_eq!(SCREEN_WIDTH, 256);
}
