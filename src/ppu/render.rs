//! PPU rendering pipeline — background and sprite pixel generation.
//!
//! See: https://www.nesdev.org/wiki/PPU_rendering

#![allow(dead_code)]

use crate::ppu::{
    ChrReader, Ppu, COARSE_X_MASK, CTRL_BASE_NT_MASK, CTRL_BG_PATTERN_1000,
    CTRL_SPRITE_PATTERN_1000, CTRL_SPRITE_SIZE_16, MASK_SHOW_BG, MASK_SHOW_BG_LEFT,
    MASK_SHOW_SPRITES, MASK_SHOW_SPRITES_LEFT, MAX_SPRITES_PER_SCANLINE, NT_H_BIT, NT_V_BIT,
    SCREEN_HEIGHT, SCREEN_WIDTH,
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
/// OAM Y value that halts sprite evaluation ($FF). See: https://www.nesdev.org/wiki/PPU_OAM#Sprite_overflow
const OAM_Y_HALT: u8 = 0xFF;
/// Sprite 0 hit is not triggered at x = 255 (hardware quirk).
/// See: https://www.nesdev.org/wiki/PPU_OAM#Sprite_zero_hit
const SPRITE_ZERO_HIT_MAX_X: u16 = 255;

/// Transient per-pixel rendering pipeline state.
///
/// All fields are recomputed every scanline and are not part of the
/// architectural PPU state. Skipped during save-state serialization;
/// defaults to zero on restore — the pipeline refills on the next
/// visible scanline.
#[derive(Clone, Default)]
pub(super) struct RenderPipeline {
    /// Per-scanline vertical position snapshot: coarse Y (bits 5-9).
    pub(super) coarse_y: u16,
    /// Per-scanline vertical position snapshot: fine Y (bits 12-14).
    pub(super) fine_y: u16,
    /// Per-scanline vertical position snapshot: vertical nametable bit.
    pub(super) nt_v: u16,

    /// Horizontal render position snapshot — coarse X at last snapshot point.
    pub(super) coarse_x_start: u16,
    /// Horizontal render position snapshot — horizontal nametable bit.
    pub(super) nt_h_start: u16,
    /// Horizontal render position snapshot — fine X (0-7) at last snapshot.
    pub(super) fine_x_start: u8,
    /// Pixel X (0-255) at which the last snapshot/re-sync happened.
    pub(super) resync_px: u16,

    /// Whether the per-scanline render snapshot has been taken.
    pub(super) scanline_initialized: bool,
    /// Set when `v` is modified mid-scanline (triggers render re-sync).
    pub(super) v_dirty: bool,
    /// Whether the renderer produced output this frame.
    pub(super) rendered_this_frame: bool,

    /// Sprite evaluation result for the current scanline.
    pub(super) sprites: [(usize, u8, u8, u8, u8); MAX_SPRITES_PER_SCANLINE],
    /// Number of valid entries in `sprites`.
    pub(super) sprite_count: usize,
    /// Whether sprite overflow was detected for the current scanline.
    pub(super) overflow: bool,
    /// Whether sprite 0 hit has been set during the current frame.
    pub(super) sprite_zero_hit: bool,

    /// Debug: enable fine-X scroll corruption bug.
    pub(super) slant_corruption: bool,
    /// Debug: use the inaccurate NES palette (yellow pipes etc.).
    pub(super) use_inaccurate_palette: bool,
    /// Debug: retrigger NMI on every PPUCTRL write with bit 7 set during
    /// VBlank (the old buggy behavior, fixed). Causes spurious extra NMIs
    /// that waste VBlank time on redundant OAM DMAs, forcing VRAM writes
    /// to spill into visible scanlines where scroll increments corrupt
    /// the target addresses.
    pub(super) nmi_retrigger: bool,

    // ---- M-PPU-09: 2-pixel lookahead fetch pipeline ----
    //
    // Real hardware fetches tile data (NT, AT, BG low, BG high) 2 cycles
    // before the pixel is rendered. We model this with a 2-entry ring
    // buffer: each pixel cycle, we fetch CHR for pixel `px + 2` and store
    // the result; the pixel output uses the entry fetched 2 cycles ago.
    //
    // This means a mid-scanline CHR bank switch affects pixels 2 cycles
    // later than the renderer would otherwise predict.
    /// Ring buffer holding (pattern, pal_select) for the 2 most recent
    /// prefetches. `fetch_idx` points to the slot to read/output next;
    /// the other slot holds the newer prefetch.
    pub(super) fetch_buffer: [(u8, u8); 2],
    /// Read index into `fetch_buffer` (toggles 0↔1). The slot at
    /// `fetch_idx` holds the oldest prefetch (for the current pixel);
    /// `fetch_idx ^ 1` holds the newer one.
    pub(super) fetch_idx: u8,
    /// Whether the fetch buffer has been primed with 2 entries. Reset at
    /// each scanline start and when bg rendering is disabled.
    pub(super) pipeline_primed: bool,
}

/// Inaccurate NTSC palette — the previous (incorrect) RGB values that
/// produced wrong colors (e.g. yellow pipes in Mario Bros). Kept for
/// debug toggle purposes. See `use_inaccurate_palette` on `RenderState`.
pub const NES_PALETTE_INACCURATE: [[u8; 3]; 64] = [
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
    // 0x30-0x3F — mirrors of 0x20-0x2F
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

/// NES 2C02 (NTSC) reference palette — 64 entries × RGB. See: https://www.nesdev.org/wiki/PPU_palettes
pub const NES_PALETTE: [[u8; 3]; 64] = [
    // 0x00-0x0F — dark/unsaturated row
    [0x7C, 0x7C, 0x7C],
    [0x00, 0x00, 0xFC],
    [0x00, 0x00, 0xBC],
    [0x44, 0x28, 0xBC],
    [0x94, 0x00, 0x84],
    [0xA8, 0x00, 0x20],
    [0xA8, 0x10, 0x00],
    [0x88, 0x14, 0x00],
    [0x50, 0x30, 0x00],
    [0x00, 0x78, 0x00],
    [0x00, 0x68, 0x00],
    [0x00, 0x58, 0x00],
    [0x00, 0x40, 0x58],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    // 0x10-0x1F — medium row
    [0xBC, 0xBC, 0xBC],
    [0x00, 0x78, 0xF8],
    [0x00, 0x58, 0xF8],
    [0x68, 0x44, 0xFC],
    [0xD8, 0x00, 0xCC],
    [0xE4, 0x00, 0x58],
    [0xF8, 0x38, 0x00],
    [0xE4, 0x5C, 0x10],
    [0xAC, 0x7C, 0x00],
    [0x00, 0xB8, 0x00],
    [0x00, 0xA8, 0x00],
    [0x00, 0xA8, 0x44],
    [0x00, 0x88, 0x88],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    // 0x20-0x2F — bright row
    [0xF8, 0xF8, 0xF8],
    [0x3C, 0xBC, 0xFC],
    [0x68, 0x88, 0xFC],
    [0x98, 0x78, 0xF8],
    [0xF8, 0x78, 0xF8],
    [0xF8, 0x58, 0x98],
    [0xF8, 0x78, 0x58],
    [0xFC, 0xA0, 0x44],
    [0xF8, 0xB8, 0x00],
    [0xB8, 0xF8, 0x18],
    [0x58, 0xD8, 0x54],
    [0x58, 0xF8, 0x98],
    [0x00, 0xE8, 0xD8],
    [0x78, 0x78, 0x78],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
    // 0x30-0x3F — bright row. The canonical 2C02 palette has these as
    // near-mirrors of 0x20-0x2F (the NES colour generator produces 56
    // unique colours; $10/$14/$18/$1C mirror $00/$04/$08/$0C, and the
    // $3x row is a bright variant of $2x). We mirror $20-$2F here, which
    // is the approximation used by most emulators.
    [0xFC, 0xFC, 0xFC],
    [0xA4, 0xE4, 0xFC],
    [0xB8, 0xB8, 0xF8],
    [0xD8, 0xB8, 0xF8],
    [0xF8, 0xB8, 0xF8],
    [0xF8, 0xA4, 0xC0],
    [0xF0, 0xD0, 0xB0],
    [0xFC, 0xE0, 0xA8],
    [0xF8, 0xD8, 0x78],
    [0xD8, 0xF8, 0x78],
    [0xB8, 0xF8, 0xB8],
    [0xB8, 0xF8, 0xD8],
    [0x00, 0xFC, 0xFC],
    [0xF8, 0xD8, 0xF8],
    [0x00, 0x00, 0x00],
    [0x00, 0x00, 0x00],
];

/// NES 2C07 (PAL) reference palette — 64 entries × RGB. See: https://www.nesdev.org/wiki/PPU_palettes#Palettes
pub const PAL_PALETTE: [[u8; 3]; 64] = [
    // 0x00-0x0F
    [0x84, 0x84, 0x84],
    [0x00, 0x1D, 0x2C],
    [0x0C, 0x0C, 0x44],
    [0x24, 0x04, 0x54],
    [0x3C, 0x00, 0x4C],
    [0x4C, 0x00, 0x34],
    [0x4C, 0x00, 0x18],
    [0x40, 0x0C, 0x00],
    [0x2C, 0x18, 0x00],
    [0x18, 0x28, 0x00],
    [0x08, 0x3C, 0x00],
    [0x00, 0x40, 0x00],
    [0x00, 0x3C, 0x1C],
    [0x00, 0x38, 0x3C],
    [0x04, 0x04, 0x04],
    [0x00, 0x00, 0x00],
    // 0x10-0x1F
    [0xB4, 0xB4, 0xB4],
    [0x30, 0x60, 0xA4],
    [0x48, 0x48, 0xC8],
    [0x60, 0x38, 0xD8],
    [0x80, 0x30, 0xC8],
    [0x98, 0x2C, 0xAC],
    [0xA8, 0x2C, 0x70],
    [0xB0, 0x2C, 0x38],
    [0xB4, 0x38, 0x10],
    [0xB8, 0x48, 0x00],
    [0x98, 0x58, 0x00],
    [0x78, 0x68, 0x00],
    [0x5C, 0x78, 0x00],
    [0x38, 0x80, 0x00],
    [0x00, 0x8C, 0x00],
    [0x04, 0x04, 0x04],
    // 0x20-0x2F
    [0xFF, 0xFF, 0xFF],
    [0x8C, 0xCC, 0xFF],
    [0xA8, 0xA8, 0xFF],
    [0xC0, 0xA8, 0xFF],
    [0xE8, 0xA4, 0xF0],
    [0xE8, 0xA0, 0xD8],
    [0xE8, 0x9C, 0xB8],
    [0xE8, 0x98, 0x88],
    [0xE0, 0x8C, 0x54],
    [0xD8, 0x98, 0x38],
    [0xC0, 0xA0, 0x24],
    [0xA0, 0xA8, 0x18],
    [0x84, 0xB0, 0x18],
    [0x60, 0xB8, 0x18],
    [0x44, 0xC0, 0x2C],
    [0x04, 0x04, 0x04],
    // 0x30-0x3F — mirrors of 0x20-0x2F (PAL 2C07 also produces 56 unique
    // colours; the $3x row is a bright variant of $2x).
    [0xFF, 0xFF, 0xFF],
    [0x8C, 0xCC, 0xFF],
    [0xA8, 0xA8, 0xFF],
    [0xC0, 0xA8, 0xFF],
    [0xE8, 0xA4, 0xF0],
    [0xE8, 0xA0, 0xD8],
    [0xE8, 0x9C, 0xB8],
    [0xE8, 0x98, 0x88],
    [0xE0, 0x8C, 0x54],
    [0xD8, 0x98, 0x38],
    [0xC0, 0xA0, 0x24],
    [0xA0, 0xA8, 0x18],
    [0x84, 0xB0, 0x18],
    [0x60, 0xB8, 0x18],
    [0x44, 0xC0, 0x2C],
    [0x04, 0x04, 0x04],
];

/// Alpha value for all framebuffer pixels (fully opaque).
const ALPHA: u32 = 0xFF;

/// Convert NES color index to ARGB using region-appropriate palette.
pub fn nes_color_to_argb_for(index: u8, region: crate::region::Region) -> u32 {
    let idx = (index & 0x3F) as usize;
    let [r, g, b] = if region.is_pal_palette() {
        PAL_PALETTE[idx]
    } else {
        NES_PALETTE[idx]
    };
    (ALPHA << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
}

/// Convert NES color index to ARGB using NTSC palette (backward compat).
pub fn nes_color_to_argb(index: u8) -> u32 {
    nes_color_to_argb_for(index, crate::region::Region::Ntsc)
}

impl Ppu {
    /// Render the entire visible background (256×240) into the framebuffer.
    /// Delegates to [`Ppu::render_background_scanline`] per scanline.
    ///
    /// See: https://www.nesdev.org/wiki/PPU_rendering#Background
    pub fn render_background(&mut self, mut chr_read: impl FnMut(u16) -> u8) {
        for py in 0..SCREEN_HEIGHT as u16 {
            self.render_background_scanline(py, &mut chr_read);
        }
    }

    /// The universal background color (palette entry `$3F00`) as ARGB.
    /// Used to fill the framebuffer when background rendering is disabled
    /// or when the left 8 pixels are masked.
    #[inline]
    pub fn universal_bg_argb(&self) -> u32 {
        self.color_to_argb(self.read_palette(PAL_BASE))
    }

    /// Convert a 6-bit NES color index to an ARGB pixel using the PPU's
    /// current region (M32). NTSC/Dendy use [`NES_PALETTE`]; PAL uses
    /// [`PAL_PALETTE`]. When `use_inaccurate_palette` is set, the old
    /// inaccurate NTSC palette ([`NES_PALETTE_INACCURATE`]) is used instead
    /// for NTSC/Dendy, reproducing the yellow-pipes color bug.
    #[inline]
    fn color_to_argb(&self, index: u8) -> u32 {
        let idx = (index & 0x3F) as usize;
        let [r, g, b] = if self.region.is_pal_palette() {
            PAL_PALETTE[idx]
        } else if self.render.use_inaccurate_palette {
            NES_PALETTE_INACCURATE[idx]
        } else {
            NES_PALETTE[idx]
        };
        (ALPHA << 24) | ((r as u32) << 16) | ((g as u32) << 8) | (b as u32)
    }

    /// Render all sprites, compositing on top of the existing framebuffer.
    /// Delegates to [`Ppu::render_sprites_scanline`] per scanline.
    ///
    /// See: https://www.nesdev.org/wiki/PPU_OAM
    pub fn render_sprites(&mut self, mut chr_read: impl FnMut(u16) -> u8) {
        self.set_sprite_overflow(false);
        self.set_sprite_zero_hit(false);
        let mut overflow_this_frame = false;
        let mut sprite_zero_hit_set = false;
        for scanline in 0..SCREEN_HEIGHT as u16 {
            self.render_sprites_scanline(
                scanline,
                &mut chr_read,
                &mut overflow_this_frame,
                &mut sprite_zero_hit_set,
            );
        }
        if overflow_this_frame {
            self.set_sprite_overflow(true);
        }
    }

    /// Render one scanline of background into the framebuffer and bg_pattern.
    fn render_background_scanline(&mut self, py: u16, chr_read: &mut impl FnMut(u16) -> u8) {
        let bg_enabled = (self.ppumask & MASK_SHOW_BG) != 0;
        let bg_left_enabled = (self.ppumask & MASK_SHOW_BG_LEFT) != 0;
        let universal_bg = self.universal_bg_argb();
        let row_base = (py as usize) * SCREEN_WIDTH;

        if !bg_enabled {
            for px in 0..SCREEN_WIDTH {
                self.framebuffer[row_base + px] = universal_bg;
                self.bg_pattern[px] = 0;
            }
            return;
        }

        let bg_table: u16 = if (self.ppuctrl & CTRL_BG_PATTERN_1000) != 0 {
            0x1000
        } else {
            0x0000
        };
        let base_nt: u16 = ((self.ppuctrl & CTRL_BASE_NT_MASK) as u16) << 10;
        let coarse_x = self.t & 0x1F;
        let coarse_y = (self.t >> 5) & 0x1F;
        let fine_y = (self.t >> 12) & 0x07;
        let scroll_x = (coarse_x << 3) | (self.fine_x as u16);
        let scroll_y = (coarse_y << 3) | fine_y;

        let gy = py + scroll_y;
        let fine_y = gy & 0x07;
        let tile_row = (gy >> 3) & 0x1F;
        let nt_v = (gy >> 8) & 1;

        for px in 0..SCREEN_WIDTH as u16 {
            if px < 8 && !bg_left_enabled {
                self.framebuffer[row_base + px as usize] = universal_bg;
                self.bg_pattern[px as usize] = 0;
                continue;
            }

            let gx = px + scroll_x;
            let fine_x = gx & 0x07;
            let tile_col = (gx >> 3) & 0x1F;
            let nt_h = (gx >> 8) & 1;
            let nt = ((base_nt >> 10) ^ nt_h ^ (nt_v << 1)) & 0x03;
            let nt_addr = NT_BASE | (nt << 10) | (tile_row << 5) | tile_col;
            let tile_index = self.read_nametable(nt_addr) as u16;
            let pattern_addr = bg_table | (tile_index << 4) | fine_y;
            let plane0 = chr_read(pattern_addr);
            let plane1 = chr_read(pattern_addr | 0x08);
            let bit = 7 - fine_x;
            let pattern: u8 = ((plane0 >> bit) & 1) | (((plane1 >> bit) & 1) << 1);
            let attr_col = tile_col >> 2;
            let attr_row = tile_row >> 2;
            let attr_addr = NT_BASE | (nt << 10) | ATTR_TABLE_OFFSET | (attr_row << 3) | attr_col;
            let attr_byte = self.read_nametable(attr_addr);
            let shift = ((tile_row & 0x02) << 1) | (tile_col & 0x02);
            let pal_select = (attr_byte >> shift) & 0x03;
            self.bg_pattern[px as usize] = pattern;
            let color_addr = if pattern == 0 {
                PAL_BASE
            } else {
                PAL_BASE | ((pal_select as u16) << 2) | (pattern as u16)
            };
            let nes_index = self.read_palette(color_addr);
            self.framebuffer[row_base + px as usize] = self.color_to_argb(nes_index);
        }
    }

    /// Render sprites for one scanline, compositing on top of the existing
    /// framebuffer. `overflow_this_frame` and `sprite_zero_hit_set` are
    /// shared across scanlines for the whole frame.
    fn render_sprites_scanline(
        &mut self,
        scanline: u16,
        chr_read: &mut impl FnMut(u16) -> u8,
        overflow_this_frame: &mut bool,
        sprite_zero_hit_set: &mut bool,
    ) {
        let sprites_enabled = (self.ppumask & MASK_SHOW_SPRITES) != 0;
        if !sprites_enabled {
            return;
        }
        let sprites_left_enabled = (self.ppumask & MASK_SHOW_SPRITES_LEFT) != 0;
        let sprite_size_16 = (self.ppuctrl & CTRL_SPRITE_SIZE_16) != 0;
        let sprite_height: u16 = if sprite_size_16 {
            SPRITE_HEIGHT_8X16
        } else {
            SPRITE_HEIGHT_8X8
        };
        let sprite_table_8x8: u16 = if (self.ppuctrl & CTRL_SPRITE_PATTERN_1000) != 0 {
            0x1000
        } else {
            0x0000
        };

        let mut selected: [(usize, u8, u8, u8, u8); MAX_SPRITES_PER_SCANLINE] =
            [(0, 0, 0, 0, 0); MAX_SPRITES_PER_SCANLINE];
        let mut count = 0usize;
        for i in 0..SPRITE_COUNT {
            let oam_idx = i * 4;
            let y = self.oam[oam_idx];
            if y == OAM_Y_HALT {
                break;
            }
            if y >= OAM_Y_HIDDEN {
                continue;
            }
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
                *overflow_this_frame = true;
            }
        }

        let row_base = scanline as usize * SCREEN_WIDTH;
        for px in 0..SCREEN_WIDTH as u16 {
            if px < 8 && !sprites_left_enabled {
                continue;
            }
            for &(oam_i, y, tile, attr, sx) in selected.iter().take(count) {
                let sx = sx as u16;
                if px < sx || px >= sx + SPRITE_WIDTH {
                    continue;
                }
                let tile_col = (px - sx) as u8;
                let tile_row = (scanline - (y as u16 + 1)) as u8;
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

                let (table, tile_base): (u16, u16) = if sprite_size_16 {
                    let t = if (tile & 1) != 0 { 0x1000 } else { 0x0000 };
                    (t, (tile & 0xFE) as u16)
                } else {
                    (sprite_table_8x8, tile as u16)
                };
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
                    continue;
                }

                let col_idx = row_base + px as usize;
                let bg_opaque = self.bg_pattern[px as usize] != 0;

                if !*sprite_zero_hit_set && oam_i == 0 && px < SPRITE_ZERO_HIT_MAX_X && bg_opaque {
                    self.set_sprite_zero_hit(true);
                    *sprite_zero_hit_set = true;
                }

                let behind_bg = (attr & ATTR_PRIORITY_BEHIND) != 0;
                if behind_bg && bg_opaque {
                    break;
                }

                let pal = (attr & ATTR_PALETTE_MASK) as u16;
                let color_addr = SPRITE_PAL_BASE | (pal << 2) | (pattern as u16);
                let nes_index = self.read_palette(color_addr);
                self.framebuffer[col_idx] = self.color_to_argb(nes_index);
                break;
            }
        }
    }

    /// Render full frame: background then sprites. See: https://www.nesdev.org/wiki/PPU_rendering
    pub fn render_frame(&mut self, mut chr_read: impl FnMut(u16) -> u8) {
        self.set_sprite_overflow(false);
        self.set_sprite_zero_hit(false);
        let mut overflow_this_frame = false;
        let mut sprite_zero_hit_set = false;
        for py in 0..SCREEN_HEIGHT as u16 {
            self.render_background_scanline(py, &mut chr_read);
            self.render_sprites_scanline(
                py,
                &mut chr_read,
                &mut overflow_this_frame,
                &mut sprite_zero_hit_set,
            );
        }
        if overflow_this_frame {
            self.set_sprite_overflow(true);
        }
    }

    // =================================================================
    //  M25: Per-pixel (cycle-accurate) rendering
    // =================================================================

    /// Render one pixel at current `(cycle, scanline)` (cycle-accurate path).
    #[inline]
    pub(super) fn render_one_pixel(&mut self, chr_read: &mut impl ChrReader) {
        // The output pixel X is derived from the PPU cycle (cycle 1 → px 0,
        // cycle 256 → px 255). This is inherently save-state-safe: the
        // position is recomputed from the architectural `cycle` field every
        // pixel rather than being a separate counter that can drift.
        let px = (self.cycle - 1) as usize;
        let py = self.scanline as usize;
        let col = py * SCREEN_WIDTH + px;

        // ---- First pixel of a scanline: snapshot + sprite evaluation ----
        if !self.render.scanline_initialized {
            self.snapshot_render_position(px);
            self.evaluate_scanline_sprites();
            self.render.scanline_initialized = true;
        }

        // ---- Re-sync on mid-scanline register writes ----
        if self.render.v_dirty {
            self.snapshot_render_position(px);
        }

        let bg_enabled = (self.ppumask & MASK_SHOW_BG) != 0;
        let bg_left_enabled = (self.ppumask & MASK_SHOW_BG_LEFT) != 0;
        let sprites_enabled = (self.ppumask & MASK_SHOW_SPRITES) != 0;
        let sprites_left_enabled = (self.ppumask & MASK_SHOW_SPRITES_LEFT) != 0;

        // ---- Compute the background pixel ----
        let mut pixel_argb: u32;

        if !bg_enabled {
            // Background disabled → universal bg color, transparent.
            pixel_argb = self.universal_bg_argb();
            self.bg_pattern[px] = 0;
            self.render.pipeline_primed = false;
        } else {
            // ---- M-PPU-09: 2-pixel lookahead fetch pipeline ----
            //
            // Real hardware fetches tile data 2 cycles before the pixel is
            // rendered. We model this with a 2-entry ring buffer: each
            // pixel, we output the buffered result (fetched 2 cycles ago)
            // and prefetch the pixel 2 positions ahead.
            //
            // The pipeline always runs when bg is enabled, even if the left
            // 8 pixels are masked — on real hardware the PPU fetches
            // continuously regardless of the left mask.

            if !self.render.pipeline_primed {
                // Prime: fetch the current and next pixel immediately.
                // The first 2 pixels of a bg-enabled region have no delay
                // (their data was not prefetched). This is acceptable
                // because CHR bank switches at scanline start are rare.
                let f0 = self.fetch_bg_pixel(px, chr_read);
                let f1 = self.fetch_bg_pixel(px + 1, chr_read);
                self.render.fetch_buffer[0] = f0;
                self.render.fetch_buffer[1] = f1;
                self.render.fetch_idx = 0;
                self.render.pipeline_primed = true;
            }

            // Output the buffered result for the current pixel.
            let idx = self.render.fetch_idx as usize;
            let (pattern, pal_select) = self.render.fetch_buffer[idx];

            // Prefetch the pixel 2 positions ahead.
            let lookahead = self.fetch_bg_pixel(px + 2, chr_read);
            self.render.fetch_buffer[idx] = lookahead;
            self.render.fetch_idx ^= 1;

            // Apply the left-column mask to the output only (the pipeline
            // still ran, keeping it in sync for when the mask ends).
            if px < 8 && !bg_left_enabled {
                pixel_argb = self.universal_bg_argb();
                self.bg_pattern[px] = 0;
            } else {
                self.bg_pattern[px] = pattern;
                let color_addr = if pattern == 0 {
                    PAL_BASE // universal background
                } else {
                    PAL_BASE | ((pal_select as u16) << 2) | (pattern as u16)
                };
                let nes_index = self.read_palette(color_addr);
                pixel_argb = self.color_to_argb(nes_index);
            }
        }

        // ---- Composite sprite pixel on top ----
        if sprites_enabled && (px >= 8 || sprites_left_enabled) {
            if let Some((sprite_pattern, sprite_pal)) = self.fetch_sprite_pixel(px, py, chr_read) {
                // fetch_sprite_pixel returns Some only for an opaque,
                // priority-OK sprite pixel, so we can safely overwrite the
                // background pixel with the sprite color.
                let color_addr =
                    SPRITE_PAL_BASE | ((sprite_pal as u16) << 2) | (sprite_pattern as u16);
                let nes_index = self.read_palette(color_addr);
                pixel_argb = self.color_to_argb(nes_index);
            }
        }

        self.framebuffer[col] = pixel_argb;
    }

    /// Snapshot the per-scanline render position from `v`. Called at the
    /// first pixel of each visible scanline and after mid-scanline `v` writes.
    fn snapshot_render_position(&mut self, px: usize) {
        self.render.coarse_y = (self.v >> 5) & 0x1F;
        self.render.fine_y = (self.v >> 12) & 0x07;
        self.render.nt_v = self.v & NT_V_BIT;
        self.render.coarse_x_start = self.v & COARSE_X_MASK;
        self.render.nt_h_start = self.v & NT_H_BIT;
        self.render.fine_x_start = self.fine_x;
        self.render.resync_px = px as u16;
        self.render.v_dirty = false;
    }

    /// Compute effective horizontal render position for pixel `px`.
    #[inline]
    fn effective_render_x(&self, px: usize) -> (u16, u16, u8) {
        let mut coarse_x = self.render.coarse_x_start;
        let mut nt_h = self.render.nt_h_start;
        let fine_x_start = self.render.fine_x_start as u16;
        let advance = px as u16 - self.render.resync_px;
        // Advance fine X first; on wrap, advance coarse X with nametable
        // horizontal-bit wrap.
        let new_fine_x = (fine_x_start + advance) & 0x07;
        let cx_inc = (fine_x_start + advance) >> 3;
        // Now advance coarse X by `cx_inc`, wrapping at 32 with nt_h flip.
        let total = (coarse_x + cx_inc) as u32;
        let wraps = total / 32;
        coarse_x = (total % 32) as u16;
        if wraps & 1 != 0 {
            nt_h ^= NT_H_BIT;
        }
        (coarse_x, nt_h, new_fine_x as u8)
    }

    /// Fetch bg pixel `(pattern, pal_select)` at output pixel `px`.
    fn fetch_bg_pixel(&self, px: usize, chr_read: &mut impl ChrReader) -> (u8, u8) {
        let (coarse_x, nt_h, fine_x) = self.effective_render_x(px);
        // Effective nametable select (0..3) from the horizontal + vertical
        // nametable bits.
        let nt = ((nt_h >> 10) | (self.render.nt_v >> 10)) & 0x03;
        let coarse_y = self.render.coarse_y;

        // 1) Nametable fetch → tile index.
        let nt_addr = NT_BASE | (nt << 10) | (coarse_y << 5) | coarse_x;
        let tile_index = self.read_nametable(nt_addr) as u16;

        // 2) Pattern-table fetch (two bitplanes).
        let bg_table: u16 = if (self.ppuctrl & CTRL_BG_PATTERN_1000) != 0 {
            0x1000
        } else {
            0x0000
        };
        let pattern_addr = bg_table | (tile_index << 4) | self.render.fine_y;
        let plane0 = chr_read.read_chr(pattern_addr);
        let plane1 = chr_read.read_chr(pattern_addr | 0x08);
        let bit = 7 - (fine_x as u16);
        let pattern: u8 = ((plane0 >> bit) & 1) | (((plane1 >> bit) & 1) << 1);

        // 3) Attribute-table fetch → 2-bit palette select.
        let attr_col = coarse_x >> 2;
        let attr_row = coarse_y >> 2;
        let attr_addr = NT_BASE | (nt << 10) | ATTR_TABLE_OFFSET | (attr_row << 3) | attr_col;
        let attr_byte = self.read_nametable(attr_addr);
        let shift = ((coarse_y & 0x02) << 1) | (coarse_x & 0x02);
        let pal_select = (attr_byte >> shift) & 0x03;

        (pattern, pal_select)
    }

    /// Evaluate sprites for current scanline (select first 8 in range). See: https://www.nesdev.org/wiki/PPU_OAM#Sprite_overflow
    fn evaluate_scanline_sprites(&mut self) {
        self.render.sprite_count = 0;
        self.render.overflow = false;

        let sprite_size_16 = (self.ppuctrl & CTRL_SPRITE_SIZE_16) != 0;
        let sprite_height: u16 = if sprite_size_16 {
            SPRITE_HEIGHT_8X16
        } else {
            SPRITE_HEIGHT_8X8
        };
        let scanline = self.scanline;

        for i in 0..SPRITE_COUNT {
            let oam_idx = i * 4;
            let y = self.oam[oam_idx];
            if y == OAM_Y_HALT {
                break;
            }
            if y >= OAM_Y_HIDDEN {
                continue;
            }
            let top = y as u16 + 1;
            if scanline < top || scanline >= top + sprite_height {
                continue;
            }
            if self.render.sprite_count < MAX_SPRITES_PER_SCANLINE {
                self.render.sprites[self.render.sprite_count] = (
                    i,
                    y,
                    self.oam[oam_idx + 1],
                    self.oam[oam_idx + 2],
                    self.oam[oam_idx + 3],
                );
                self.render.sprite_count += 1;
            } else {
                self.render.overflow = true;
            }
        }

        // Latch the overflow flag into PPUSTATUS (set, never cleared mid-frame).
        if self.render.overflow {
            self.set_sprite_overflow(true);
        }
    }

    /// Fetch sprite pixel at `(px, py)`; handles sprite 0 hit detection. See: https://www.nesdev.org/wiki/PPU_OAM#Sprite_zero_hit
    fn fetch_sprite_pixel(
        &mut self,
        px: usize,
        py: usize,
        chr_read: &mut impl ChrReader,
    ) -> Option<(u8, u8)> {
        let sprite_size_16 = (self.ppuctrl & CTRL_SPRITE_SIZE_16) != 0;
        let sprite_height: u16 = if sprite_size_16 {
            SPRITE_HEIGHT_8X16
        } else {
            SPRITE_HEIGHT_8X8
        };
        let sprite_table_8x8: u16 = if (self.ppuctrl & CTRL_SPRITE_PATTERN_1000) != 0 {
            0x1000
        } else {
            0x0000
        };
        let scanline = py as u16;
        let bg_opaque = self.bg_pattern[px] != 0;

        for idx in 0..self.render.sprite_count {
            let (oam_i, y, tile, attr, sx) = self.render.sprites[idx];
            let sx = sx as u16;
            if (px as u16) < sx || (px as u16) >= sx + SPRITE_WIDTH {
                continue;
            }
            let tile_col = (px as u16 - sx) as u8;
            let tile_row = (scanline - (y as u16 + 1)) as u8;
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

            let (table, tile_base): (u16, u16) = if sprite_size_16 {
                let t = if (tile & 1) != 0 { 0x1000 } else { 0x0000 };
                (t, (tile & 0xFE) as u16)
            } else {
                (sprite_table_8x8, tile as u16)
            };
            let row_in_tile = if sprite_size_16 { row & 0x07 } else { row };
            let tile_for_row = if sprite_size_16 && row >= 8 {
                tile_base + 1
            } else {
                tile_base
            };

            let pattern_addr = table | (tile_for_row << 4) | (row_in_tile as u16);
            let plane0 = chr_read.read_chr(pattern_addr);
            let plane1 = chr_read.read_chr(pattern_addr | 0x08);
            let bit = 7 - col;
            let pattern: u8 = ((plane0 >> bit) & 1) | (((plane1 >> bit) & 1) << 1);

            if pattern == 0 {
                continue; // transparent — try next sprite
            }

            // Sprite 0 hit detection (triggers once per frame).
            if !self.render.sprite_zero_hit
                && oam_i == 0
                && (px as u16) < SPRITE_ZERO_HIT_MAX_X
                && bg_opaque
            {
                self.set_sprite_zero_hit(true);
                self.render.sprite_zero_hit = true;
            }

            let behind_bg = (attr & ATTR_PRIORITY_BEHIND) != 0;
            if behind_bg && bg_opaque {
                // Sprite is behind an opaque background → not visible.
                // Continue to the next sprite (lower priority, can't claim).
                continue;
            }

            let pal = attr & ATTR_PALETTE_MASK;
            return Some((pattern, pal));
        }

        None
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
        assert_eq!(NES_PALETTE[0x00], [0x7C, 0x7C, 0x7C]);
    }

    #[test]
    fn palette_index_20_is_white() {
        assert_eq!(NES_PALETTE[0x20], [0xF8, 0xF8, 0xF8]);
    }

    #[test]
    fn palette_index_0e_is_black() {
        assert_eq!(NES_PALETTE[0x0E], [0x00, 0x00, 0x00]);
    }

    #[test]
    fn nes_color_to_argb_packs_channels() {
        // 0x21 = (0x3C, 0xBC, 0xFC) → ARGB 0xFF3CBCFC.
        assert_eq!(nes_color_to_argb(0x21), 0xFF3C_BCFC);
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
