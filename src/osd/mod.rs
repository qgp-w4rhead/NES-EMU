//! On-screen display (OSD) — FPS / mapper / game-name / slot / rewind
//! overlay rendered directly into the framebuffer (M30).
//!
//! The OSD draws text into the emulator's 256x240 ARGB framebuffer using a
//! hand-rolled 8x8 bitmap font (see [`font`]). No external crates are used
//! (the tech-stack document forbids adding new dependencies).
//!
//! The OSD is toggled with `F10`. When enabled, [`Osd::render`] is called
//! after the emulator has produced the frame and *before* the video layer
//! uploads the framebuffer to the SDL2 texture, so the overlay appears on
//! top of the game image.
//!
//! See: https://www.nesdev.org/wiki/PPU — native NES resolution is 256x240.

mod font;

pub use font::glyph_bits;

use std::path::Path;
use std::time::{Duration, Instant};

/// OSD text foreground colour (bright white, opaque).
pub const OSD_FG_ARGB: u32 = 0xFFFF_FFFF;
/// OSD text background colour (semi-transparent black — alpha is ignored
/// by the ARGB8888 blit since we just overwrite pixels, but kept opaque
/// so the OSD is readable over any game image).
pub const OSD_BG_ARGB: u32 = 0xFF00_0000;

/// 8x8 bitmap font glyph cell width in pixels.
pub const GLYPH_W: u32 = 8;
/// 8x8 bitmap font glyph cell height in pixels.
pub const GLYPH_H: u32 = 8;

/// Number of frame-time samples averaged for the FPS readout. A 30-frame
/// window smooths out jitter while staying responsive (~0.5 s at 60 fps).
pub const FPS_WINDOW: usize = 30;

/// On-screen display state. Held by the main loop; [`Osd::record_frame`]
/// is called once per presented frame, and [`Osd::render`] draws the
/// overlay into the framebuffer when [`Osd::enabled`] is true.
#[derive(Debug)]
pub struct Osd {
    /// Whether the OSD is currently visible on screen.
    enabled: bool,
    /// Ring buffer of recent inter-frame intervals (monotonic clock).
    frame_intervals: [Duration; FPS_WINDOW],
    /// Index into `frame_intervals` for the next write (ring wrap).
    frame_idx: usize,
    /// Number of intervals recorded so far (capped at `FPS_WINDOW`).
    frame_count: usize,
    /// Timestamp of the previous `record_frame` call.
    last_frame: Option<Instant>,
}

impl Default for Osd {
    fn default() -> Self {
        Self {
            enabled: false,
            frame_intervals: [Duration::ZERO; FPS_WINDOW],
            frame_idx: 0,
            frame_count: 0,
            last_frame: None,
        }
    }
}

impl Osd {
    /// Build a new OSD, disabled by default.
    pub fn new() -> Self {
        Self::default()
    }

    /// Is the OSD currently visible?
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Toggle the OSD on/off. Returns the new state.
    pub fn toggle(&mut self) -> bool {
        self.enabled = !self.enabled;
        self.enabled
    }

    /// Explicitly set the OSD visibility.
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
    }

    /// Record the completion of one presented frame. The interval since
    /// the previous call is added to the FPS ring buffer. The first call
    /// after construction (or after a long pause) just seeds the
    /// `last_frame` timestamp without recording an interval, so the FPS
    /// readout is not polluted by the gap.
    pub fn record_frame(&mut self, now: Instant) {
        if let Some(prev) = self.last_frame {
            let delta = now.saturating_duration_since(prev);
            // Ignore pathological gaps (> 1 s) — likely a pause, debugger
            // stop, or save-state load — so they don't tank the average.
            if delta.as_secs_f32() < 1.0 {
                self.frame_intervals[self.frame_idx] = delta;
                self.frame_idx = (self.frame_idx + 1) % FPS_WINDOW;
                if self.frame_count < FPS_WINDOW {
                    self.frame_count += 1;
                }
            }
        }
        self.last_frame = Some(now);
    }

    /// Current smoothed FPS (frames per second) computed from the ring
    /// buffer. Returns `0.0` if no intervals have been recorded yet.
    pub fn fps(&self) -> f32 {
        if self.frame_count == 0 {
            return 0.0;
        }
        let mut sum = Duration::ZERO;
        for i in 0..self.frame_count {
            sum += self.frame_intervals[i];
        }
        let secs = sum.as_secs_f64() / self.frame_count as f64;
        if secs > 0.0 {
            (1.0 / secs) as f32
        } else {
            0.0
        }
    }

    /// Render the OSD into the framebuffer. The overlay is drawn in the
    /// top-left corner, one line per element of `lines`. Each line is
    /// drawn with a 1-pixel opaque-black background per glyph cell so the
    /// text is readable over any game image.
    ///
    /// `fb` is the emulator framebuffer (`width * height` ARGB pixels,
    /// row-major). Lines that would extend past the right or bottom edge
    /// of the framebuffer are clipped (per-glyph: a glyph that would
    /// start off-screen is skipped entirely).
    pub fn render(&self, fb: &mut [u32], width: u32, height: u32, lines: &[&str]) {
        if !self.enabled || fb.len() != (width * height) as usize {
            return;
        }
        let mut y = 0u32;
        for line in lines {
            self.draw_line(fb, width, height, line, y);
            y = y.saturating_add(GLYPH_H);
            if y + GLYPH_H > height {
                break;
            }
        }
    }

    /// Draw a single line of text at row `y`, with a per-glyph opaque
    /// background. Glyphs that would extend past the right edge are
    /// skipped (clipped).
    fn draw_line(&self, fb: &mut [u32], width: u32, height: u32, text: &str, y: u32) {
        if y >= height {
            return;
        }
        let mut x = 0u32;
        for ch in text.chars() {
            if x + GLYPH_W > width {
                break;
            }
            self.draw_glyph(fb, width, height, ch, x, y);
            x = x.saturating_add(GLYPH_W);
        }
    }

    /// Draw one glyph at pixel `(x, y)` with an opaque-black background.
    /// Unknown characters (not in the font table) are drawn as a solid
    /// background cell (i.e. a blank space) so the layout stays aligned.
    fn draw_glyph(&self, fb: &mut [u32], width: u32, height: u32, ch: char, x: u32, y: u32) {
        let glyph = glyph_bits(ch);
        for gy in 0..GLYPH_H {
            let row = glyph[gy as usize];
            let py = y + gy;
            if py >= height {
                break;
            }
            for gx in 0..GLYPH_W {
                let px = x + gx;
                if px >= width {
                    break;
                }
                let bit = (row >> (7 - gx)) & 1;
                let color = if bit != 0 { OSD_FG_ARGB } else { OSD_BG_ARGB };
                let idx = (py * width + px) as usize;
                fb[idx] = color;
            }
        }
    }
}

/// Build the OSD line set for a standard frame. Centralised here so the
/// main loop and tests construct identical strings.
///
/// `game_name` is typically the ROM file stem (no directory, no
/// extension). `slot` is the current save-state slot index. `slots_empty`
/// is the count of empty slots (so the OSD can show whether the current
/// slot has a save). `rewind_len` is the current rewind buffer depth
/// (number of frames stored).
//
// NOTE: This returns a `Vec<String>` (4 small allocations) per call. The
// OSD is off by default and the allocation is tiny (~4 × 30 bytes), so
// this is an accepted exception to tech-stack.md's "no allocation in main
// loop" rule. A future optimisation could write into a `&mut [String]`
// buffer to eliminate even these.
pub fn build_lines(
    fps: f32,
    mapper_number: u16,
    game_name: &str,
    slot: usize,
    slot_occupied: bool,
    rewind_len: usize,
) -> Vec<String> {
    let slot_state = if slot_occupied { "OCCUPIED" } else { "EMPTY" };
    vec![
        format!("FPS:{:.1} MAP:{}", fps, mapper_number),
        format!("GAME:{}", game_name),
        format!(
            "SLOT:{}/{} {}",
            slot + 1,
            crate::save_state::SAVE_STATE_SLOT_COUNT,
            slot_state
        ),
        format!("REWIND:{}", rewind_len),
    ]
}

/// Truncate `path` to its file stem (no directory, no extension) for OSD
/// display. Returns `"?"` on parse failure.
pub fn game_name_from_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("?")
        .to_string()
}

#[cfg(test)]
mod tests;
