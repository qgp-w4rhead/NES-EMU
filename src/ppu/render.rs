//! PPU rendering pipeline (M8 background + M9 sprites).
//!
//! # Background (M8)
//!
//! Renders the background layer from nametables, attribute tables, and
//! pattern tables into the PPU's framebuffer (`256×240` ARGB pixels).
//!
//! This module implements the *pixel pipeline* — the per-pixel decode of
//! nametable byte → attribute byte → pattern-table bitplanes → palette
//! entry → ARGB output. Scanline timing, VBlank NMI signalling, and the
//! fine/coarse scroll increment machinery (`v` register updates during
//! rendering) are deferred to M10; here we render a full frame on demand
//! from the current scroll position recorded in the `t` register and
//! `fine_x`, which is sufficient to validate the decode against a static
//! test screen.
//!
//! # Sprites (M9)
//!
//! Renders sprites from OAM in 8×8 mode with horizontal/vertical flip,
//! palette selection (sprite palettes 4-7 at `$3F10-$3F1F`), and
//! priority (in front of / behind the background). The hardware limit of
//! 8 sprites per scanline is enforced; the sprite-overflow flag is set
//! when more than 8 sprites are in range on any scanline. Sprite zero
//! hit detection lands in M11. 8×16 sprite mode lands in M23.
//!
//! # Background pixel pipeline (per visible pixel)
//!
//! 1. Add the scroll offset (coarse/fine X and Y from `t` + `fine_x`) to
//!    the pixel's screen coordinates to get global tile-space coordinates.
//! 2. Resolve the nametable select from the base nametable (PPUCTRL bits
//!    0-1) XOR the wrap bits produced when scrolling past a 256-pixel
//!    nametable boundary.
//! 3. Fetch the tile index from the nametable at
//!    `$2000 | (nt << 10) | (row << 5) | col`.
//! 4. Fetch the two pattern-table bitplanes for the tile at
//!    `bg_table | (tile << 4) | fine_y` and `+ 8`.
//! 5. Combine the bitplane bits at `7 - fine_x` into a 2-bit pattern value.
//! 6. Fetch the attribute byte at
//!    `$2000 | (nt << 10) | $03C0 | (attr_row << 3) | attr_col` and extract
//!    the 2-bit palette-select pair for this 32×32 quadrant.
//! 7. Look up the NES color index in palette RAM:
//!    - pattern 0 → `$3F00` (universal background)
//!    - pattern != 0 → `$3F00 | (pal << 2) | pattern`
//! 8. Convert the 6-bit NES color index to ARGB via the [`NES_PALETTE`]
//!    table and store it in the framebuffer.
//!
//! See: https://www.nesdev.org/wiki/PPU_rendering
//! See: https://www.nesdev.org/wiki/PPU_palettes

#![allow(dead_code)]

use crate::ppu::{
    Ppu, CTRL_BASE_NT_MASK, CTRL_BG_PATTERN_1000, CTRL_SPRITE_PATTERN_1000, CTRL_SPRITE_SIZE_16,
    MASK_SHOW_BG, MASK_SHOW_BG_LEFT, MASK_SHOW_SPRITES, MASK_SHOW_SPRITES_LEFT, SCREEN_HEIGHT,
    SCREEN_WIDTH,
};

/// Nametable tile grid is 32×30 tiles (256×240 pixels).
const NT_COLS: u16 = 32;
const NT_ROWS: u16 = 30;
/// Attribute table is 8×8 quadrants of 4×4 tiles each (64×64 px / quadrant).
const ATTR_TABLE_OFFSET: u16 = 0x03C0;

/// Base address of the nametable region in PPU address space.
const NT_BASE: u16 = 0x2000;
/// Base address of palette RAM.
const PAL_BASE: u16 = 0x3F00;
/// Base address of sprite palettes in palette RAM (`$3F10-$3F1F`).
const SPRITE_PAL_BASE: u16 = 0x3F10;

/// Sprite attribute byte (OAM byte 2) bits.
/// Bits 0-1: palette select (4-7).
const ATTR_PALETTE_MASK: u8 = 0b0000_0011;
/// Bit 5: priority (0 = in front of background, 1 = behind background).
const ATTR_PRIORITY_BEHIND: u8 = 0b0010_0000;
/// Bit 6: flip sprite horizontally.
const ATTR_HFLIP: u8 = 0b0100_0000;
/// Bit 7: flip sprite vertically.
const ATTR_VFLIP: u8 = 0b1000_0000;

/// Maximum number of sprites rendered on a single scanline (hardware limit).
const MAX_SPRITES_PER_SCANLINE: usize = 8;
/// Number of sprites in OAM (64).
const SPRITE_COUNT: usize = 64;
/// Sprite height in 8×8 mode (pixels).
const SPRITE_HEIGHT_8X8: u16 = 8;
/// Sprite height in 8×16 mode (pixels).
const SPRITE_HEIGHT_8X16: u16 = 16;
/// Sprite width in pixels.
const SPRITE_WIDTH: u16 = 8;
/// OAM Y value at or above which a sprite is hidden ($EF-$FE = 239-254).
/// Values `$EF-$FE` are out of range but do not halt sprite evaluation;
/// only `$FF` halts evaluation (see [`Ppu::render_sprites`]).
const OAM_Y_HIDDEN: u8 = 0xEF;
/// OAM Y value that halts sprite evaluation ($FF). On real hardware the
/// sprite evaluation unit stops scanning OAM when it encounters a sprite
/// with Y = `$FF`; sprites after it are never checked. This is the
/// hardware-accurate "sprite overflow bug" behaviour — see
/// https://www.nesdev.org/wiki/PPU_OAM#Sprite_overflow.
const OAM_Y_HALT: u8 = 0xFF;
/// Sprite 0 hit is not triggered at x = 255 (hardware quirk).
/// See: https://www.nesdev.org/wiki/PPU_OAM#Sprite_zero_hit
const SPRITE_ZERO_HIT_MAX_X: u16 = 255;

/// NES 2C02 (NTSC) reference palette — 64 entries × RGB.
///
/// Each entry maps a 6-bit NES color index (`$00-$3F`) to an approximate
/// sRGB triple. The real NES produces an analog signal with no canonical
/// RGB mapping; this table is a widely-used approximation (see the
/// "2C02" palette on the nesdev wiki). Color emphasis (PPUMASK bits 4-6)
/// is not yet applied — it lands with the full PPUMASK implementation.
///
/// See: https://www.nesdev.org/wiki/PPU_palettes
pub const NES_PALETTE: [[u8; 3]; 64] = [
    // 0x00-0x0F — dark/unsaturated row
    [0x84, 0x84, 0x84],
    [0x00, 0x1D, 0x2C],
    [0x1C, 0x0C, 0x54],
    [0x30, 0x04, 0x64],
    [0x48, 0x00, 0x5C],
    [0x58, 0x00, 0x44],
    [0x58, 0x00, 0x24],
    [0x4C, 0x0C, 0x00],
    [0x38, 0x18, 0x00],
    [0x20, 0x28, 0x00],
    [0x0C, 0x3C, 0x00],
    [0x00, 0x40, 0x00],
    [0x00, 0x3C, 0x1C],
    [0x00, 0x38, 0x3C],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    // 0x10-0x1F — medium row
    [0xB4, 0xB4, 0xB4],
    [0x38, 0x6C, 0xBC],
    [0x54, 0x58, 0xEC],
    [0x70, 0x44, 0xF4],
    [0x90, 0x38, 0xE4],
    [0xA8, 0x34, 0xC8],
    [0xB8, 0x34, 0x88],
    [0xC0, 0x34, 0x44],
    [0xC4, 0x40, 0x14],
    [0xC8, 0x50, 0x00],
    [0xA8, 0x60, 0x00],
    [0x88, 0x70, 0x00],
    [0x6C, 0x80, 0x00],
    [0x44, 0x88, 0x00],
    [0x00, 0x94, 0x00],
    [0x00, 0x00, 0x00],
    // 0x20-0x2F — bright row
    [0xFF, 0xFF, 0xFF],
    [0x9C, 0xDC, 0xFF],
    [0xB8, 0xB8, 0xFF],
    [0xD0, 0xB8, 0xFF],
    [0xFF, 0xB0, 0xF4],
    [0xFF, 0xA8, 0xE0],
    [0xFF, 0xA4, 0xC0],
    [0xFF, 0xA0, 0x90],
    [0xF8, 0x94, 0x58],
    [0xF0, 0xA0, 0x38],
    [0xD8, 0xA8, 0x20],
    [0xB8, 0xB0, 0x14],
    [0x98, 0xB8, 0x14],
    [0x70, 0xC0, 0x14],
    [0x50, 0xC8, 0x24],
    [0x00, 0x00, 0x00],
    // 0x30-0x3F — bright row. The canonical 2C02 palette has these as
    // near-mirrors of 0x20-0x2F (the NES colour generator produces 56
    // unique colours; $10/$14/$18/$1C mirror $00/$04/$08/$0C, and the
    // $3x row is a bright variant of $2x). We mirror $20-$2F here, which
    // is the approximation used by most emulators.
    [0xFF, 0xFF, 0xFF],
    [0x9C, 0xDC, 0xFF],
    [0xB8, 0xB8, 0xFF],
    [0xD0, 0xB8, 0xFF],
    [0xFF, 0xB0, 0xF4],
    [0xFF, 0xA8, 0xE0],
    [0xFF, 0xA4, 0xC0],
    [0xFF, 0xA0, 0x90],
    [0xF8, 0x94, 0x58],
    [0xF0, 0xA0, 0x38],
    [0xD8, 0xA8, 0x20],
    [0xB8, 0xB0, 0x14],
    [0x98, 0xB8, 0x14],
    [0x70, 0xC0, 0x14],
    [0x50, 0xC8, 0x24],
    [0x00, 0x00, 0x00],
];

/// Alpha value for all framebuffer pixels (fully opaque).
const ALPHA: u32 = 0xFF;

/// Convert a 6-bit NES color index to an ARGB (`0xAARRGGBB`) pixel.
///
/// The index is masked to 6 bits (`& 0x3F`) — the upper two bits of
/// palette RAM reads are open-bus garbage on real hardware and are
/// discarded here.
pub fn nes_color_to_argb(index: u8) -> u32 {
    let idx = (index & 0x3F) as usize;
    let [r, g, b] = NES_PALETTE[idx];
    (ALPHA << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

impl Ppu {
    /// Render the entire visible background (256×240) into the framebuffer.
    ///
    /// `chr_read` supplies pattern-table bytes from CHR (owned by the
    /// cartridge and accessed via the bus). The PPU's own VRAM and palette
    /// RAM are read directly.
    ///
    /// Honours PPUMASK: if the background is disabled (bit 3 clear) the
    /// framebuffer is filled with the universal background color; if the
    /// left 8 pixels are masked (bit 1 clear) they are also filled with
    /// the universal background color.
    ///
    /// Scroll position is taken from the `t` register (coarse X/Y, fine Y,
    /// and nametable select bits) plus `fine_x`. This matches what a game
    /// sets up via PPUSCROLL / PPUADDR before the frame; the per-scanline
    /// `v`-register increment and horizontal/vertical scroll wrap timing
    /// are implemented in M10.
    ///
    /// See: https://www.nesdev.org/wiki/PPU_rendering#Background
    pub fn render_background(&mut self, chr_read: impl Fn(u16) -> u8) {
        let bg_enabled = (self.ppumask & MASK_SHOW_BG) != 0;
        let bg_left_enabled = (self.ppumask & MASK_SHOW_BG_LEFT) != 0;
        let universal_bg = self.universal_bg_argb();

        if !bg_enabled {
            self.framebuffer.fill(universal_bg);
            // Background is fully transparent when disabled → bg_pattern = 0
            // everywhere, so sprites with "behind background" priority show
            // through.
            self.bg_pattern.fill(0);
            return;
        }

        // Background pattern table base: $0000 or $1000 (PPUCTRL bit 4).
        let bg_table: u16 = if (self.ppuctrl & CTRL_BG_PATTERN_1000) != 0 {
            0x1000
        } else {
            0x0000
        };
        // Base nametable select (PPUCTRL bits 0-1): $2000/$2400/$2800/$2C00.
        let base_nt: u16 = ((self.ppuctrl & CTRL_BASE_NT_MASK) as u16) << 10;

        // Scroll position from the t register + fine_x.
        //   coarse X = t bits 0-4,  fine X = fine_x (3 bits)
        //   coarse Y = t bits 5-9,  fine Y = t bits 12-14
        //   nametable select from t bits 10-11 is *not* used here —
        //   PPUCTRL's base nametable (bits 0-1) is the authoritative
        //   source during rendering. On real hardware, writing PPUCTRL
        //   copies bits 0-1 into t's nt bits (10-11); that copy is not
        //   yet implemented (deferred to M10), so we read PPUCTRL
        //   directly to avoid divergence if a game used PPUADDR to set
        //   a different nametable.
        let coarse_x = self.t & 0x1F;
        let coarse_y = (self.t >> 5) & 0x1F;
        let fine_y = (self.t >> 12) & 0x07;
        let scroll_x = (coarse_x << 3) | (self.fine_x as u16);
        let scroll_y = (coarse_y << 3) | fine_y;

        for py in 0..SCREEN_HEIGHT as u16 {
            let gy = py + scroll_y;
            let fine_y = gy & 0x07;
            let tile_row = (gy >> 3) & 0x1F; // wraps within a 256px nametable
                                             // Vertical nametable wrap: bit 1 of nt select toggles when
                                             // coarse Y crosses a 256px boundary (tile 32+).
            let nt_v = (gy >> 8) & 1;
            let row_base = (py as usize) * SCREEN_WIDTH;

            for px in 0..SCREEN_WIDTH as u16 {
                let col = row_base + px as usize;

                // Left-column mask: pixels 0-7 are blanked if bit 1 clear.
                if px < 8 && !bg_left_enabled {
                    self.framebuffer[col] = universal_bg;
                    self.bg_pattern[col] = 0;
                    continue;
                }

                let gx = px + scroll_x;
                let fine_x = gx & 0x07;
                let tile_col = (gx >> 3) & 0x1F; // wraps within a 256px nametable
                                                 // Horizontal nametable wrap: bit 0 of nt select toggles
                                                 // when coarse X crosses a 256px boundary.
                let nt_h = (gx >> 8) & 1;

                // Resolve the effective nametable select (0..3).
                let nt = ((base_nt >> 10) ^ nt_h ^ (nt_v << 1)) & 0x03;
                let nt_addr = NT_BASE | (nt << 10) | (tile_row << 5) | tile_col;

                // 1) Nametable fetch → tile index.
                let tile_index = self.read_nametable(nt_addr) as u16;

                // 2) Pattern-table fetch (two bitplanes).
                let pattern_addr = bg_table | (tile_index << 4) | fine_y;
                let plane0 = chr_read(pattern_addr);
                let plane1 = chr_read(pattern_addr | 0x08);
                let bit = 7 - fine_x;
                let pattern: u8 = ((plane0 >> bit) & 1) | (((plane1 >> bit) & 1) << 1);

                // 3) Attribute-table fetch → 2-bit palette select.
                let attr_col = tile_col >> 2;
                let attr_row = tile_row >> 2;
                let attr_addr =
                    NT_BASE | (nt << 10) | ATTR_TABLE_OFFSET | (attr_row << 3) | attr_col;
                let attr_byte = self.read_nametable(attr_addr);
                // Each attribute byte covers a 4×4-tile (32×32-px) block;
                // the 2-bit pair is selected by bit 1 of tile_col (left/right
                // 16px half → shift 0/2) and bit 1 of tile_row (top/bottom
                // 16px half → shift 0/4).
                let shift = ((tile_row & 0x02) << 1) | (tile_col & 0x02);
                let pal_select = (attr_byte >> shift) & 0x03;

                // Record the background pattern value for sprite priority.
                self.bg_pattern[col] = pattern;

                // 4) Palette lookup.
                let color_addr = if pattern == 0 {
                    PAL_BASE // universal background
                } else {
                    PAL_BASE | ((pal_select as u16) << 2) | (pattern as u16)
                };
                let nes_index = self.read_palette(color_addr);

                // 5) Convert to ARGB and write.
                self.framebuffer[col] = nes_color_to_argb(nes_index);
            }
        }
    }

    /// The universal background color (palette entry `$3F00`) as ARGB.
    /// Used to fill the framebuffer when background rendering is disabled
    /// or when the left 8 pixels are masked.
    pub fn universal_bg_argb(&self) -> u32 {
        nes_color_to_argb(self.read_palette(PAL_BASE))
    }

    /// Render all sprites from OAM in 8×8 mode, compositing on top of the
    /// existing framebuffer (which must have been produced by a prior
    /// [`Ppu::render_background`] call — `render_frame` does this in the
    /// right order).
    ///
    /// `chr_read` supplies pattern-table bytes from CHR (owned by the
    /// cartridge and accessed via the bus).
    ///
    /// # Behaviour
    ///
    /// - Honours PPUMASK bit 4 (show sprites) and bit 2 (show left 8
    ///   pixels of sprites).
    /// - Uses PPUCTRL bit 3 to select the sprite pattern table
    ///   (`$0000` or `$1000`) in 8×8 mode. In 8×16 mode (PPUCTRL bit 5
    ///   set) bit 3 is ignored — the pattern table is selected per
    ///   sprite by bit 0 of its tile index (even → `$0000`, odd →
    ///   `$1000`), the top 8 rows fetch tile `index & 0xFE`, and the
    ///   bottom 8 rows fetch `tile + 1` (`index | 1`). Vertical flip
    ///   mirrors the entire 16-pixel range.
    /// - A sprite is visible on scanline `s` when its OAM Y byte `y`
    ///   satisfies `y + 1 <= s <= y + height` (height = 8 or 16) and
    ///   `y < $EF` (matching the one-scanline delay documented on the
    ///   NESdev wiki — "subtract 1 from the sprite's Y coordinate
    ///   before writing it here").
    /// - At most 8 sprites are rendered per scanline (the first 8 in OAM
    ///   order); if a 9th would be in range, the sprite-overflow flag
    ///   (PPUSTATUS bit 5) is set. Sprite evaluation halts when a sprite
    ///   with Y = `$FF` is encountered (hardware-accurate behaviour —
    ///   sprites after a `$FF` entry are never checked).
    /// - **Sprite zero hit** (M11): when sprite 0 (OAM index 0) is
    ///   rendered and its pixel is opaque, and the background pixel at
    ///   the same position is also opaque, the sprite-0-hit flag
    ///   (PPUSTATUS bit 6) is set. The hit is not triggered at x = 255
    ///   or in the clipped left 8 pixels. The priority bit (attribute
    ///   bit 5) does **not** affect hit detection — the hit triggers
    ///   even if sprite 0 is behind the background. The flag is set once
    ///   per frame (first hit) and cleared at the prerender scanline by
    ///   [`Ppu::step`].
    /// - Sprite-to-sprite priority: lower OAM index = in front. The
    ///   first non-transparent sprite (scanning OAM 0 → 63) claims each
    ///   pixel; no lower-priority sprite can override it.
    /// - Background priority: if the claiming sprite's attribute bit 5
    ///   is set ("behind background"), the sprite only shows where the
    ///   background pattern is 0 (transparent); otherwise the background
    ///   pixel is kept.
    /// - Horizontal/vertical flip (attribute bits 6/7) mirror the
    ///   8×8 tile pixels within the sprite's bounding box.
    /// - Sprite palettes are at `$3F10-$3F1F` (palettes 4-7); the
    ///   sprite's 2-bit palette select picks one of four 4-color
    ///   palettes. Pattern 0 is always transparent.
    ///
    /// See: https://www.nesdev.org/wiki/PPU_OAM
    /// See: https://www.nesdev.org/wiki/PPU_rendering#Sprites
    /// See: https://www.nesdev.org/wiki/PPU_OAM#Sprite_zero_hit
    pub fn render_sprites(&mut self, chr_read: impl Fn(u16) -> u8) {
        // Clear the per-frame status flags at the start of the frame. On
        // real hardware this happens at the prerender scanline (M10);
        // without scanline-locked rendering we clear them here so the
        // flags reflect the current frame rather than sticking from a
        // prior frame. This must happen even when sprites are disabled
        // (a prior frame may have set a flag that needs clearing).
        self.set_sprite_overflow(false);
        self.set_sprite_zero_hit(false);

        let sprites_enabled = (self.ppumask & MASK_SHOW_SPRITES) != 0;
        if !sprites_enabled {
            return;
        }
        let sprites_left_enabled = (self.ppumask & MASK_SHOW_SPRITES_LEFT) != 0;

        // Sprite size: 8×8 (default) or 8×16 (PPUCTRL bit 5).
        // See: https://www.nesdev.org/wiki/PPU_OAM#8x16_sprites
        let sprite_size_16 = (self.ppuctrl & CTRL_SPRITE_SIZE_16) != 0;
        let sprite_height: u16 = if sprite_size_16 {
            SPRITE_HEIGHT_8X16
        } else {
            SPRITE_HEIGHT_8X8
        };

        // Sprite pattern table base: $0000 or $1000 (PPUCTRL bit 3).
        // In 8×16 mode this bit is ignored — the table is selected per
        // sprite by bit 0 of the tile index (handled in the pixel loop).
        let sprite_table_8x8: u16 = if (self.ppuctrl & CTRL_SPRITE_PATTERN_1000) != 0 {
            0x1000
        } else {
            0x0000
        };

        // Per-scanline selected-sprite slots (OAM index, Y, tile, attr, X).
        // Allocated once on the stack; reused for each scanline.
        let mut selected: [(usize, u8, u8, u8, u8); MAX_SPRITES_PER_SCANLINE] =
            [(0, 0, 0, 0, 0); MAX_SPRITES_PER_SCANLINE];
        let mut overflow_this_frame = false;
        // Sprite 0 hit is set once per frame (first opaque overlap).
        let mut sprite_zero_hit_set = false;

        for scanline in 0..SCREEN_HEIGHT as u16 {
            // ---- Sprite evaluation: find first 8 sprites in range. ----
            //
            // Hardware-accurate behaviour: evaluation halts when a sprite
            // with Y = `$FF` is encountered. Sprites with Y = `$EF-$FE`
            // are out of range but do NOT halt evaluation — only `$FF`
            // does. See:
            // https://www.nesdev.org/wiki/PPU_OAM#Sprite_overflow
            let mut count = 0usize;
            for i in 0..SPRITE_COUNT {
                let oam_idx = i * 4;
                let y = self.oam[oam_idx];
                if y == OAM_Y_HALT {
                    // $FF halts sprite evaluation — no further sprites
                    // are checked on this scanline.
                    break;
                }
                if y >= OAM_Y_HIDDEN {
                    // $EF-$FE: out of range, but evaluation continues.
                    continue;
                }
                // Visible on scanlines (y+1)..=(y+height). With y < 0xEF
                // there is no u8 wraparound to worry about.
                let top = y as u16 + 1;
                if scanline < top || scanline >= top + sprite_height {
                    continue;
                }
                if count < MAX_SPRITES_PER_SCANLINE {
                    selected[count] = (
                        i,
                        y,
                        self.oam[oam_idx + 1],
                        self.oam[oam_idx + 2],
                        self.oam[oam_idx + 3],
                    );
                    count += 1;
                } else {
                    // 9th+ in-range sprite → overflow.
                    overflow_this_frame = true;
                }
            }

            // ---- Pixel rendering for this scanline. ----
            let row_base = scanline as usize * SCREEN_WIDTH;
            for px in 0..SCREEN_WIDTH as u16 {
                // Left-column clip for sprites.
                if px < 8 && !sprites_left_enabled {
                    continue;
                }

                // Find the first non-transparent sprite (lowest OAM index
                // = highest priority). It claims the pixel; no later
                // sprite can override it.
                for &(oam_i, y, tile, attr, sx) in selected.iter().take(count) {
                    let sx = sx as u16;
                    if px < sx || px >= sx + SPRITE_WIDTH {
                        continue;
                    }
                    let tile_col = (px - sx) as u8;
                    // Row within the sprite (0..height-1). scanline - (y+1).
                    let tile_row = (scanline - (y as u16 + 1)) as u8;
                    // Vertical flip mirrors the entire sprite height (8 or
                    // 16), not just the 8-pixel tile half.
                    let row = if (attr & ATTR_VFLIP) != 0 {
                        ((sprite_height - 1) as u8) - tile_row
                    } else {
                        tile_row
                    };
                    let col = if (attr & ATTR_HFLIP) != 0 {
                        7 - tile_col
                    } else {
                        tile_col
                    };

                    // Pattern address computation differs between 8×8 and
                    // 8×16 modes.
                    //
                    // 8×8: table from PPUCTRL bit 3; tile index unchanged.
                    //
                    // 8×16: PPUCTRL bit 3 ignored; table from tile bit 0
                    // (even → $0000, odd → $1000); tile base = tile & 0xFE;
                    // rows 0-7 fetch tile_base, rows 8-15 fetch tile_base+1.
                    // See: https://www.nesdev.org/wiki/PPU_OAM#8x16_sprites
                    let (table, tile_base): (u16, u16) = if sprite_size_16 {
                        let t = if (tile & 1) != 0 { 0x1000 } else { 0x0000 };
                        (t, (tile & 0xFE) as u16)
                    } else {
                        (sprite_table_8x8, tile as u16)
                    };
                    // For 8×16, rows 8-15 come from the next tile
                    // (tile_base + 1) at row offset (row - 8).
                    let row_in_tile = if sprite_size_16 { row & 0x07 } else { row };
                    let tile_for_row = if sprite_size_16 && row >= 8 {
                        tile_base + 1
                    } else {
                        tile_base
                    };

                    let pattern_addr = table | (tile_for_row << 4) | (row_in_tile as u16);
                    let plane0 = chr_read(pattern_addr);
                    let plane1 = chr_read(pattern_addr | 0x08);
                    let bit = 7 - col;
                    let pattern: u8 = ((plane0 >> bit) & 1) | (((plane1 >> bit) & 1) << 1);

                    if pattern == 0 {
                        // Transparent — this sprite doesn't claim the pixel;
                        // fall through to the next sprite.
                        continue;
                    }

                    let col_idx = row_base + px as usize;
                    let bg_opaque = self.bg_pattern[col_idx] != 0;

                    // ---- Sprite 0 hit detection (M11) ----
                    //
                    // The hit triggers when sprite 0 (OAM index 0) has an
                    // opaque pixel AND the background pixel at the same
                    // position is opaque. The priority bit does NOT
                    // affect hit detection — the hit triggers even if
                    // sprite 0 is behind the background. The hit is not
                    // triggered at x = 255 or in the clipped left 8
                    // pixels (clipping is handled above for sprites and
                    // in render_background for the background).
                    //
                    // See: https://www.nesdev.org/wiki/PPU_OAM#Sprite_zero_hit
                    if !sprite_zero_hit_set && oam_i == 0 && px < SPRITE_ZERO_HIT_MAX_X && bg_opaque
                    {
                        self.set_sprite_zero_hit(true);
                        sprite_zero_hit_set = true;
                    }

                    // ---- Background priority ----
                    //
                    // If the sprite is "behind background" (attr bit 5)
                    // and the background is opaque here, the sprite is
                    // hidden — the pixel keeps its background value. No
                    // lower-priority sprite may override. The sprite 0
                    // hit (above) still triggered because both pixels
                    // are opaque, regardless of priority.
                    let behind_bg = (attr & ATTR_PRIORITY_BEHIND) != 0;
                    if behind_bg && bg_opaque {
                        break;
                    }

                    let pal = (attr & ATTR_PALETTE_MASK) as u16;
                    let color_addr = SPRITE_PAL_BASE | (pal << 2) | (pattern as u16);
                    let nes_index = self.read_palette(color_addr);
                    self.framebuffer[col_idx] = nes_color_to_argb(nes_index);
                    break;
                }
            }
        }

        if overflow_this_frame {
            self.set_sprite_overflow(true);
        }
    }

    /// Render a full frame: background first, then sprites composited on
    /// top. This is the M9 entry point for producing a complete visible
    /// frame; the video layer (M12) uploads the resulting framebuffer to
    /// an SDL2 texture.
    ///
    /// See: https://www.nesdev.org/wiki/PPU_rendering
    pub fn render_frame(&mut self, chr_read: impl Fn(u16) -> u8) {
        // Pass the closure by reference to both passes; `Fn` is called via
        // `&self`, so `&chr_read` satisfies the `impl Fn(u16) -> u8` bound.
        self.render_background(&chr_read);
        self.render_sprites(&chr_read);
    }

    /// Clear the entire framebuffer to a single ARGB value.
    pub fn clear_framebuffer(&mut self, argb: u32) {
        self.framebuffer.fill(argb);
    }

    /// Read a single framebuffer pixel (ARGB) at `(x, y)`. Returns the
    /// universal background color for out-of-bounds coordinates.
    pub fn pixel(&self, x: usize, y: usize) -> u32 {
        if x < SCREEN_WIDTH && y < SCREEN_HEIGHT {
            self.framebuffer[y * SCREEN_WIDTH + x]
        } else {
            self.universal_bg_argb()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ppu::Ppu;

    #[test]
    fn palette_has_64_entries() {
        assert_eq!(NES_PALETTE.len(), 64);
    }

    #[test]
    fn palette_index_0_is_grey() {
        // 0x00 = grey in the reference palette.
        assert_eq!(NES_PALETTE[0x00], [0x84, 0x84, 0x84]);
    }

    #[test]
    fn palette_index_20_is_white() {
        assert_eq!(NES_PALETTE[0x20], [0xFF, 0xFF, 0xFF]);
    }

    #[test]
    fn palette_index_0e_is_black() {
        assert_eq!(NES_PALETTE[0x0E], [0x00, 0x00, 0x00]);
    }

    #[test]
    fn nes_color_to_argb_packs_channels() {
        // 0x21 = (0x9C, 0xDC, 0xFF) → ARGB 0xFF9CDCFF.
        assert_eq!(nes_color_to_argb(0x21), 0xFF9C_DCFF);
    }

    #[test]
    fn nes_color_masks_to_6_bits() {
        // 0x40 should mask down to 0x00.
        assert_eq!(nes_color_to_argb(0x40), nes_color_to_argb(0x00));
        // 0x80 should mask down to 0x00 as well.
        assert_eq!(nes_color_to_argb(0x80), nes_color_to_argb(0x00));
    }

    #[test]
    fn universal_bg_uses_palette_3f00() {
        let mut ppu = Ppu::new();
        // The PPU register write path doesn't route PPUDATA writes to
        // palette RAM (the bus does that); use the palette API directly.
        ppu.write_palette(0x3F00, 0x21);
        assert_eq!(ppu.universal_bg_argb(), nes_color_to_argb(0x21));
    }

    #[test]
    fn render_background_disabled_fills_universal_bg() {
        let mut ppu = Ppu::new();
        ppu.write_palette(0x3F00, 0x0E); // black
                                         // PPUMASK = 0 → background disabled.
        ppu.render_background(|_| 0);
        let bg = nes_color_to_argb(0x0E);
        for &px in ppu.framebuffer().iter() {
            assert_eq!(px, bg);
        }
    }

    #[test]
    fn render_background_enabled_left_mask_blanks_first_8() {
        let mut ppu = Ppu::new();
        // Enable background but NOT left 8 pixels (PPUMASK = 0b1000).
        ppu.write_register(0x01, 0b0000_1000);
        ppu.write_palette(0x3F00, 0x0E); // universal bg = black
                                         // Fill nametable with a non-zero tile and a non-black palette so
                                         // rendered pixels differ from the universal bg.
        for i in 0..0x3C0u16 {
            ppu.write_nametable(0x2000 | i, 0x01);
        }
        // Pattern table 0, tile 1: all bits set (white pixels).
        // (plane0/1 at $0010..$001F are supplied by the chr closure below.)
        let chr = |addr: u16| -> u8 {
            if (0x10..=0x1F).contains(&addr) {
                0xFF
            } else {
                0
            }
        };
        ppu.render_background(chr);
        let bg = nes_color_to_argb(0x0E);
        // Left 8 pixels of every scanline should be the universal bg.
        for y in 0..SCREEN_HEIGHT {
            for x in 0..8 {
                assert_eq!(
                    ppu.pixel(x, y),
                    bg,
                    "left pixel ({},{}) should be universal bg",
                    x,
                    y
                );
            }
        }
        // Pixel (8, 0) should NOT be the universal bg (tile is non-zero).
        assert_ne!(ppu.pixel(8, 0), bg);
    }

    #[test]
    fn render_background_solid_tile_fills_screen() {
        let mut ppu = Ppu::new();
        // Enable background + left column.
        ppu.write_register(0x01, 0b0000_1010);
        // Nametable 0: every tile = 1.
        for i in 0..(NT_COLS * NT_ROWS) {
            ppu.write_nametable(NT_BASE | i, 0x01);
        }
        // Attribute table: all zeros → palette 0 for all quadrants.
        // Pattern table 0, tile 1: all bits set → pattern value 3.
        let chr = |addr: u16| -> u8 {
            if (0x10..=0x1F).contains(&addr) {
                0xFF
            } else {
                0
            }
        };
        // Palette: $3F00 = black (universal bg), $3F03 = white.
        ppu.write_palette(PAL_BASE, 0x0E);
        ppu.write_palette(PAL_BASE | 0x03, 0x20);
        ppu.render_background(chr);
        // Every pixel should be white (pattern 3, palette 0, color $3F03).
        let white = nes_color_to_argb(0x20);
        for y in 0..SCREEN_HEIGHT {
            for x in 0..SCREEN_WIDTH {
                assert_eq!(ppu.pixel(x, y), white, "pixel ({},{})", x, y);
            }
        }
    }

    #[test]
    fn render_background_pattern_zero_uses_universal_bg() {
        let mut ppu = Ppu::new();
        ppu.write_register(0x01, 0b0000_1010);
        // Nametable 0: tile 0 everywhere (pattern all-zero → pattern value 0).
        // (VRAM is zero-initialised, so this is already the case.)
        // Palette: $3F00 = 0x0E (black), $3F01 = 0x21 (blue).
        ppu.write_palette(PAL_BASE, 0x0E);
        ppu.write_palette(PAL_BASE | 0x01, 0x21);
        ppu.render_background(|_| 0);
        // Every pixel uses the universal background ($3F00).
        let bg = nes_color_to_argb(0x0E);
        for y in 0..SCREEN_HEIGHT {
            for x in 0..SCREEN_WIDTH {
                assert_eq!(ppu.pixel(x, y), bg);
            }
        }
    }

    #[test]
    fn render_background_attribute_selects_palette() {
        let mut ppu = Ppu::new();
        ppu.write_register(0x01, 0b0000_1010);
        // Nametable 0: all tiles = 1 (solid pattern).
        for i in 0..(NT_COLS * NT_ROWS) {
            ppu.write_nametable(NT_BASE | i, 0x01);
        }
        // Pattern table 0, tile 1: solid (pattern 3).
        let chr = |addr: u16| {
            if (0x10..=0x1F).contains(&addr) {
                0xFF
            } else {
                0
            }
        };
        // Set the first attribute byte (top-left 32×32 quadrant) to 0b01_01_01_01
        // → palette 1 for all four 16×16 sub-quadrants.
        ppu.write_nametable(NT_BASE | ATTR_TABLE_OFFSET, 0b01_01_01_01);
        // Palette 0 color 3 = white, palette 1 color 3 = blue.
        ppu.write_palette(PAL_BASE | 0x03, 0x20); // white
        ppu.write_palette(PAL_BASE | 0x07, 0x21); // blue (palette 1, color 3)
        ppu.render_background(chr);
        // Top-left quadrant (0..32, 0..32) → palette 1 → blue.
        let blue = nes_color_to_argb(0x21);
        assert_eq!(ppu.pixel(0, 0), blue);
        assert_eq!(ppu.pixel(31, 31), blue);
        // The next 32×32 quadrant to the right (32..64, 0..32) → palette 0 → white.
        let white = nes_color_to_argb(0x20);
        assert_eq!(ppu.pixel(32, 0), white);
        assert_eq!(ppu.pixel(63, 31), white);
    }

    #[test]
    fn render_background_pattern_table_select() {
        let mut ppu = Ppu::new();
        // PPUCTRL bit 4 = 1 → background pattern table at $1000.
        ppu.write_register(0x00, 0b0001_0000);
        ppu.write_register(0x01, 0b0000_1010);
        for i in 0..(NT_COLS * NT_ROWS) {
            ppu.write_nametable(NT_BASE | i, 0x01);
        }
        // Pattern table 0 (tile 1) = all zero; pattern table 1 (tile 1) = solid.
        let chr = |addr: u16| -> u8 {
            if (0x1010..=0x101F).contains(&addr) {
                0xFF
            } else {
                0
            }
        };
        ppu.write_palette(PAL_BASE | 0x03, 0x20); // white
        ppu.render_background(chr);
        // With table 1 selected, tile 1 is solid → white everywhere.
        let white = nes_color_to_argb(0x20);
        assert_eq!(ppu.pixel(0, 0), white);
        assert_eq!(ppu.pixel(255, 239), white);
    }

    #[test]
    fn render_background_base_nametable_select() {
        let mut ppu = Ppu::new();
        // PPUCTRL bits 0-1 = 0b01 → base nametable $2400 (NT 1).
        ppu.write_register(0x00, 0b0000_0001);
        ppu.write_register(0x01, 0b0000_1010);
        // NT 0: tile 0 (blank). NT 1: tile 1 (solid).
        for i in 0..(NT_COLS * NT_ROWS) {
            ppu.write_nametable(NT_BASE | 0x400 | i, 0x01);
        }
        let chr = |addr: u16| {
            if (0x10..=0x1F).contains(&addr) {
                0xFF
            } else {
                0
            }
        };
        ppu.write_palette(PAL_BASE | 0x03, 0x20); // white
        ppu.write_palette(PAL_BASE, 0x0E); // black universal bg
        ppu.render_background(chr);
        // NT 1 is the base → solid white.
        assert_eq!(ppu.pixel(0, 0), nes_color_to_argb(0x20));
    }

    #[test]
    fn render_background_horizontal_scroll_wraps_nametable() {
        let mut ppu = Ppu::new();
        // Vertical mirroring so NT 0 ($2000) and NT 1 ($2400) are distinct.
        ppu.set_mirroring(crate::mappers::Mirroring::Vertical);
        ppu.write_register(0x01, 0b0000_1010);
        // Set scroll X = 8 via PPUSCROLL (coarse X = 1, fine X = 0).
        ppu.write_register(0x05, 0x08); // first write: coarse X = 1, fine X = 0
        ppu.write_register(0x05, 0x00); // second write: coarse Y = 0, fine Y = 0
                                        // NT 0: tile 0 (blank, zero-initialised). NT 1 ($2400): tile 1 (solid).
        for i in 0..(NT_COLS * NT_ROWS) {
            ppu.write_nametable(NT_BASE | 0x400 | i, 0x01);
        }
        let chr = |addr: u16| {
            if (0x10..=0x1F).contains(&addr) {
                0xFF
            } else {
                0
            }
        };
        ppu.write_palette(PAL_BASE | 0x03, 0x20);
        ppu.write_palette(PAL_BASE, 0x0E);
        ppu.render_background(chr);
        // With scroll X = 8, screen X 0 maps to nametable pixel 8 of NT 0
        // (blank). Screen X 248..255 wrap into NT 1 (solid).
        let white = nes_color_to_argb(0x20);
        let black = nes_color_to_argb(0x0E);
        assert_eq!(ppu.pixel(0, 0), black, "screen X 0 reads NT 0 (blank)");
        assert_eq!(ppu.pixel(248, 0), white, "screen X 248 wraps into NT 1");
        assert_eq!(ppu.pixel(255, 0), white);
    }

    #[test]
    fn render_background_vertical_scroll_wraps_nametable() {
        let mut ppu = Ppu::new();
        // Horizontal mirroring (default) so NT 0 ($2000) and NT 2 ($2800)
        // are distinct.
        ppu.write_register(0x01, 0b0000_1010);
        // Set scroll Y = 24 via PPUSCROLL (coarse Y = 3, fine Y = 0).
        ppu.write_register(0x05, 0x00); // first write: coarse X = 0, fine X = 0
        ppu.write_register(0x05, 0x18); // second write: coarse Y = 3, fine Y = 0
                                        // NT 0: tile 0 (blank, zero-initialised). NT 2 ($2800): tile 1 (solid).
        for i in 0..(NT_COLS * NT_ROWS) {
            ppu.write_nametable(NT_BASE | 0x800 | i, 0x01);
        }
        let chr = |addr: u16| {
            if (0x10..=0x1F).contains(&addr) {
                0xFF
            } else {
                0
            }
        };
        ppu.write_palette(PAL_BASE | 0x03, 0x20);
        ppu.write_palette(PAL_BASE, 0x0E);
        ppu.render_background(chr);
        // Scroll Y = 24 → screen Y 0..231 read NT 0 (gy 24..255, blank),
        // Y 232..239 wrap into NT 2 (gy 256..263, solid).
        let white = nes_color_to_argb(0x20);
        let black = nes_color_to_argb(0x0E);
        assert_eq!(ppu.pixel(0, 0), black);
        assert_eq!(ppu.pixel(0, 232), white);
        assert_eq!(ppu.pixel(0, 239), white);
    }

    #[test]
    fn render_background_fine_y_selects_pattern_row() {
        let mut ppu = Ppu::new();
        ppu.write_register(0x01, 0b0000_1010);
        // Tile 1 in NT 0: only the top row (fine_y = 0) has set bits.
        for i in 0..(NT_COLS * NT_ROWS) {
            ppu.write_nametable(NT_BASE | i, 0x01);
        }
        // Pattern table 0, tile 1: plane0 row 0 = 0xFF, all other rows = 0.
        let chr = |addr: u16| -> u8 {
            if addr == 0x10 {
                0xFF
            } else {
                0
            }
        };
        // Palette 0 color 1 = white-ish; universal bg = black.
        ppu.write_palette(PAL_BASE | 0x01, 0x20);
        ppu.write_palette(PAL_BASE, 0x0E);
        ppu.render_background(chr);
        // Row 0 of each 8-pixel tile row should be white; rows 1-7 black.
        let white = nes_color_to_argb(0x20);
        let black = nes_color_to_argb(0x0E);
        assert_eq!(ppu.pixel(0, 0), white, "fine_y=0 row is set");
        assert_eq!(ppu.pixel(0, 1), black, "fine_y=1 row is clear");
        assert_eq!(ppu.pixel(0, 7), black);
        assert_eq!(ppu.pixel(0, 8), white, "next tile row fine_y=0");
    }

    #[test]
    fn render_background_fine_x_selects_pattern_bit() {
        let mut ppu = Ppu::new();
        ppu.write_register(0x01, 0b0000_1010);
        // Set fine X = 4 via PPUSCROLL first write (coarse X = 0, fine X = 4).
        ppu.write_register(0x05, 0x04);
        ppu.write_register(0x05, 0x00);
        for i in 0..(NT_COLS * NT_ROWS) {
            ppu.write_nametable(NT_BASE | i, 0x01);
        }
        // Tile 1: only the high bit (bit 7) of each row is set.
        let chr = |addr: u16| -> u8 {
            if (0x10..=0x17).contains(&addr) {
                0x80
            } else {
                0
            }
        };
        ppu.write_palette(PAL_BASE | 0x01, 0x20);
        ppu.write_palette(PAL_BASE, 0x0E);
        ppu.render_background(chr);
        // With fine X = 4, screen X 0 reads nametable pixel 4 of the tile,
        // which is bit 3 (clear). Screen X 3 reads nametable pixel 7 = bit 0
        // of the shifted view → bit 7 of the tile (set).
        let white = nes_color_to_argb(0x20);
        let black = nes_color_to_argb(0x0E);
        // Screen X 0 → tile pixel (0 + 4) = 4 → bit 3 → clear.
        assert_eq!(ppu.pixel(0, 0), black);
        // Screen X 3 → tile pixel (3 + 4) = 7 → bit 0 → clear.
        assert_eq!(ppu.pixel(3, 0), black);
        // Screen X 4 → tile pixel (4 + 4) = 8 → wraps to next tile pixel 0
        // → bit 7 → set.
        assert_eq!(ppu.pixel(4, 0), white);
    }

    #[test]
    fn render_background_uses_mirroring_for_nametable_fetch() {
        // Horizontal mirroring: NT 0 and NT 1 share VRAM; NT 2 and NT 3 share.
        let mut ppu = Ppu::new();
        ppu.set_mirroring(crate::mappers::Mirroring::Horizontal);
        // Base nametable = NT 1 ($2400). Under horizontal mirroring NT 1
        // maps to the same physical VRAM as NT 0, so writing NT 0 should
        // be visible when rendering from NT 1.
        ppu.write_register(0x00, 0b0000_0001); // base NT = 1
        ppu.write_register(0x01, 0b0000_1010);
        for i in 0..(NT_COLS * NT_ROWS) {
            ppu.write_nametable(NT_BASE | i, 0x01); // write NT 0
        }
        let chr = |addr: u16| {
            if (0x10..=0x1F).contains(&addr) {
                0xFF
            } else {
                0
            }
        };
        ppu.write_palette(PAL_BASE | 0x03, 0x20);
        ppu.render_background(chr);
        // NT 1 mirrors NT 0 under horizontal mirroring → solid white.
        assert_eq!(ppu.pixel(0, 0), nes_color_to_argb(0x20));
    }

    #[test]
    fn clear_framebuffer_sets_all_pixels() {
        let mut ppu = Ppu::new();
        ppu.clear_framebuffer(0xABCDEF12);
        for &px in ppu.framebuffer().iter() {
            assert_eq!(px, 0xABCDEF12);
        }
    }

    #[test]
    fn pixel_out_of_bounds_returns_universal_bg() {
        let mut ppu = Ppu::new();
        ppu.write_palette(PAL_BASE, 0x21);
        let bg = nes_color_to_argb(0x21);
        assert_eq!(ppu.pixel(SCREEN_WIDTH, 0), bg);
        assert_eq!(ppu.pixel(0, SCREEN_HEIGHT), bg);
    }
}
