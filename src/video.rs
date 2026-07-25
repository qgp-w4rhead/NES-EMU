//! SDL2 video output for the NES emulator.
//!
//! Owns the window, renderer, and streaming texture used to display the
//! emulator's 256x240 framebuffer. The framebuffer is kept as a flat
//! `Vec<u32>` of ARGB pixels (0xAARRGGBB, alpha always 0xFF) owned by the
//! emulator core; this module only copies it onto the GPU texture each
//! frame.
//!
//! See: https://www.nesdev.org/wiki/PPU — the NES produces a 256x240 pixel
//! image; we integer-scale it 3x to a 768x720 window for Milestone 1.

use sdl2::pixels::PixelFormatEnum;
use sdl2::rect::Rect;
use sdl2::render::{Texture, TextureAccess, TextureCreator, WindowCanvas};
use sdl2::video::{FullscreenType, Window, WindowContext};

/// Logical NES resolution width in pixels.
pub const NES_WIDTH: u32 = 256;
/// Logical NES resolution height in pixels.
pub const NES_HEIGHT: u32 = 240;

/// Default integer scale factor applied to the logical resolution when
/// creating the window. 3x of 256x240 = 768x720.
pub const DEFAULT_SCALE: u32 = 3;

/// ARGB pixel value for "blank / no signal" black. Alpha is always opaque.
pub const BLACK_ARGB: u32 = 0xFF00_0000;

/// A frame's worth of ARGB pixels, row-major, top-to-bottom.
pub type Framebuffer = Box<[u32]>;

/// Allocate a fresh framebuffer filled with opaque black.
pub fn new_framebuffer() -> Framebuffer {
    vec![BLACK_ARGB; (NES_WIDTH * NES_HEIGHT) as usize].into_boxed_slice()
}

/// Bundles the SDL2 video resources needed to present one framebuffer per
/// frame. Created once at startup and reused for the lifetime of the
/// emulator — no allocation happens in the main loop after construction.
///
/// The `Texture<'static>` is a slight fiction: SDL2 textures do not actually
/// borrow their creator at the C level (the lifetime is only a Rust-side
/// guard), and we keep the `TextureCreator` alive in the same struct for the
/// texture's entire lifetime. The transmute in [`Video::new`] is the
/// canonical sdl2-rs pattern for storing a texture alongside its creator.
pub struct Video {
    pub canvas: WindowCanvas,
    pub texture: Texture<'static>,
    scale: u32,
    // Kept alive so the texture's creator outlives the texture. Never read
    // directly after construction.
    _creator: TextureCreator<WindowContext>,
}

impl Video {
    /// Build the window, renderer, and streaming texture.
    ///
    /// The texture is created with `SDL_TEXTUREACCESS_STREAMING` so the
    /// emulator can update it in place each frame without reallocating.
    pub fn new(video_subsystem: &sdl2::VideoSubsystem, scale: u32) -> Result<Self, String> {
        let window_width = NES_WIDTH * scale;
        let window_height = NES_HEIGHT * scale;
        let window: Window = video_subsystem
            .window("NES Emulator", window_width, window_height)
            .position_centered()
            .build()
            .map_err(|e| format!("window creation failed: {e}"))?;

        // Request a vsynced renderer. We do NOT force `.accelerated()` so
        // SDL2 can fall back to a software / dummy renderer in headless
        // environments (e.g. CI with `SDL_VIDEODRIVER=dummy`); on a real
        // desktop SDL2 picks an accelerated driver by default.
        let canvas = window
            .into_canvas()
            .present_vsync()
            .build()
            .map_err(|e| format!("renderer creation failed: {e}"))?;

        let texture_creator = canvas.texture_creator();
        let texture = texture_creator
            .create_texture(
                PixelFormatEnum::ARGB8888,
                TextureAccess::Streaming,
                NES_WIDTH,
                NES_HEIGHT,
            )
            .map_err(|e| format!("texture creation failed: {e}"))?;

        // SAFETY: SDL2 textures do not actually reference their creator at
        // the C level — the lifetime parameter is a Rust-only guard. We
        // keep `_creator` alive in the same struct for as long as `texture`
        // is reachable, so the borrow is never invalidated. The struct's
        // field declaration order (`texture` before `_creator`) guarantees
        // the texture is dropped before the creator.
        let texture: Texture<'static> =
            unsafe { std::mem::transmute::<Texture<'_>, Texture<'static>>(texture) };

        Ok(Self {
            canvas,
            texture,
            scale,
            _creator: texture_creator,
        })
    }

    /// Upload `fb` to the streaming texture and present it, integer-scaled
    /// to fill the window. In windowed mode the destination rectangle is
    /// the full window (`NES_WIDTH * scale` by `NES_HEIGHT * scale`). In
    /// fullscreen mode the destination rectangle is the largest integer
    /// multiple of the native resolution that fits inside the desktop,
    /// centered — so the aspect ratio and pixel grid are preserved.
    pub fn present(&mut self, fb: &[u32]) -> Result<(), String> {
        self.texture
            .update(None, to_byte_slice(fb), NES_WIDTH as usize * 4)
            .map_err(|e| format!("texture update failed: {e}"))?;

        self.canvas.set_draw_color(sdl2::pixels::Color::BLACK);
        self.canvas.clear();

        let dst = self.destination_rect()?;
        self.canvas.copy(&self.texture, None, Some(dst))?;
        self.canvas.present();
        Ok(())
    }

    /// Compute the destination rectangle for the current mode. In
    /// windowed mode this is the full window. In fullscreen mode this
    /// is the largest centered integer-scaled rectangle that fits
    /// inside the desktop's drawable area, preserving the 256:240
    /// (= 16:15) aspect ratio.
    fn destination_rect(&self) -> Result<Rect, String> {
        if self.is_fullscreen() {
            let (out_w, out_h) = self.canvas.output_size()?;
            Ok(integer_scale_rect(NES_WIDTH, NES_HEIGHT, out_w, out_h))
        } else {
            Ok(Rect::new(
                0,
                0,
                NES_WIDTH * self.scale,
                NES_HEIGHT * self.scale,
            ))
        }
    }

    /// Toggle between windowed and fullscreen-desktop mode. Returns
    /// `Ok(())` on success. The next [`Video::present`] call will use
    /// the new mode's destination rectangle.
    ///
    /// See: https://wiki.libsdl.org/SDL2/SDL_SetWindowFullscreen
    pub fn toggle_fullscreen(&mut self) -> Result<(), String> {
        let next = if self.is_fullscreen() {
            FullscreenType::Off
        } else {
            FullscreenType::Desktop
        };
        self.canvas
            .window_mut()
            .set_fullscreen(next)
            .map_err(|e| format!("set_fullscreen failed: {e}"))
    }

    /// Returns `true` if the window is currently in either true or
    /// desktop fullscreen mode. (`FullscreenType::Unknown` is treated as
    /// not-fullscreen — SDL2 only returns it before the window is
    /// mapped, which never coincides with a `present` call.)
    pub fn is_fullscreen(&self) -> bool {
        matches!(
            self.canvas.window().fullscreen_state(),
            FullscreenType::True | FullscreenType::Desktop
        )
    }

    /// The integer scale factor applied in windowed mode (e.g. 3 →
    /// 768x720 window). In fullscreen the scale is recomputed each
    /// frame from the desktop size via [`integer_scale_rect`].
    pub fn scale(&self) -> u32 {
        self.scale
    }
}

/// Compute the largest centered integer-scaled rectangle that fits
/// `(src_w, src_h)` inside `(dst_w, dst_h)` while preserving the source
/// aspect ratio. The scale factor is `min(dst_w / src_w, dst_h / src_h)`
/// rounded down to an integer (minimum 1).
///
/// Used by [`Video::present`] in fullscreen mode so the NES image keeps
/// its 256:240 aspect ratio and crisp pixel grid on any monitor size.
pub fn integer_scale_rect(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32) -> Rect {
    let scale_w = (dst_w / src_w).max(1);
    let scale_h = (dst_h / src_h).max(1);
    let scale = scale_w.min(scale_h);
    let w = src_w * scale;
    let h = src_h * scale;
    // Center inside the destination. Use saturating subtraction so a
    // very small `dst` (smaller than the scaled source) cannot
    // underflow — the rect is clamped to the origin in that case.
    let x = ((dst_w.saturating_sub(w)) / 2) as i32;
    let y = ((dst_h.saturating_sub(h)) / 2) as i32;
    Rect::new(x, y, w, h)
}

/// Reinterpret a slice of ARGB `u32` pixels as a byte slice for SDL2's
/// texture update API. The framebuffer layout already matches
/// `ARGB8888` on a little-endian host (bytes: B, G, R, A in memory, which
/// SDL2 reads as 0xAARRGGBB).
fn to_byte_slice(fb: &[u32]) -> &[u8] {
    let len = std::mem::size_of_val(fb);
    // SAFETY: `[u32]` and `[u8]` share the same element alignment
    // requirements for the underlying bytes; we only read within `len`.
    unsafe { std::slice::from_raw_parts(fb.as_ptr() as *const u8, len) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framebuffer_is_correct_size_and_black() {
        let fb = new_framebuffer();
        assert_eq!(fb.len(), (NES_WIDTH * NES_HEIGHT) as usize);
        assert!(fb.iter().all(|&p| p == BLACK_ARGB));
    }

    #[test]
    fn default_scale_yields_768x720() {
        assert_eq!(NES_WIDTH * DEFAULT_SCALE, 768);
        assert_eq!(NES_HEIGHT * DEFAULT_SCALE, 720);
    }

    /// Fullscreen on a 1920x1080 desktop: scale = min(1920/256, 1080/240)
    /// = min(7, 4) = 4 → 1024x960, centered at ((1920-1024)/2, (1080-960)/2)
    /// = (448, 60).
    #[test]
    fn integer_scale_rect_1080p() {
        let r = integer_scale_rect(NES_WIDTH, NES_HEIGHT, 1920, 1080);
        assert_eq!(r.width(), 1024);
        assert_eq!(r.height(), 960);
        assert_eq!(r.x(), 448);
        assert_eq!(r.y(), 60);
    }

    /// Fullscreen on a 1280x720 desktop: scale = min(1280/256, 720/240)
    /// = min(5, 3) = 3 → 768x720, centered at ((1280-768)/2, 0) = (256, 0).
    #[test]
    fn integer_scale_rect_720p() {
        let r = integer_scale_rect(NES_WIDTH, NES_HEIGHT, 1280, 720);
        assert_eq!(r.width(), 768);
        assert_eq!(r.height(), 720);
        assert_eq!(r.x(), 256);
        assert_eq!(r.y(), 0);
    }

    /// When the destination is smaller than the source, the scale
    /// saturates at 1 (the source is drawn at native size, possibly
    /// clipped by the renderer — but the rect itself is well-formed).
    #[test]
    fn integer_scale_rect_tiny_destination() {
        let r = integer_scale_rect(NES_WIDTH, NES_HEIGHT, 100, 100);
        assert_eq!(r.width(), NES_WIDTH);
        assert_eq!(r.height(), NES_HEIGHT);
        // Centered: ((100-256)/2, (100-240)/2) → clamped to 0,0.
        assert_eq!(r.x(), 0);
        assert_eq!(r.y(), 0);
    }

    /// A destination exactly the source size yields scale 1, origin 0.
    #[test]
    fn integer_scale_rect_exact_fit() {
        let r = integer_scale_rect(NES_WIDTH, NES_HEIGHT, NES_WIDTH, NES_HEIGHT);
        assert_eq!(r.width(), NES_WIDTH);
        assert_eq!(r.height(), NES_HEIGHT);
        assert_eq!(r.x(), 0);
        assert_eq!(r.y(), 0);
    }

    /// Non-NES source: a 16x16 sprite scaled into a 100x100 box should
    /// pick scale = min(100/16, 100/16) = 6 → 96x96 centered at (2, 2).
    #[test]
    fn integer_scale_rect_non_nes_source() {
        let r = integer_scale_rect(16, 16, 100, 100);
        assert_eq!(r.width(), 96);
        assert_eq!(r.height(), 96);
        assert_eq!(r.x(), 2);
        assert_eq!(r.y(), 2);
    }
}
