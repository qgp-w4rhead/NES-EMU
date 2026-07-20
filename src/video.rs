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
use sdl2::video::{Window, WindowContext};

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
pub type Framebuffer = Vec<u32>;

/// Allocate a fresh framebuffer filled with opaque black.
pub fn new_framebuffer() -> Framebuffer {
    vec![BLACK_ARGB; (NES_WIDTH * NES_HEIGHT) as usize]
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
    /// to fill the window.
    pub fn present(&mut self, fb: &[u32]) -> Result<(), String> {
        self.texture
            .update(None, to_byte_slice(fb), NES_WIDTH as usize * 4)
            .map_err(|e| format!("texture update failed: {e}"))?;

        self.canvas.set_draw_color(sdl2::pixels::Color::BLACK);
        self.canvas.clear();

        let dst = Rect::new(0, 0, NES_WIDTH * self.scale, NES_HEIGHT * self.scale);
        self.canvas.copy(&self.texture, None, Some(dst))?;
        self.canvas.present();
        Ok(())
    }
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
}
