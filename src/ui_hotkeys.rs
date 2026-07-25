//! M29 UI hotkey dispatcher — pause/reset/fast-forward/screenshot/fullscreen.
//!
//! Centralizes the M29 control-feature key handling so `src/main.rs` stays
//! under the 400-line file-size limit. The dispatcher owns the fast-forward
//! flag and a screenshot output directory; it does *not* own
//! any SDL2 state — the `Video` and `EmulatorState` are borrowed per call.
//!
//! # Hotkeys
//!
//! | Key            | Action                                                  |
//! |----------------|---------------------------------------------------------|
//! | `Ctrl+R`       | Soft reset (CPU RESET sequence; PPU/APU keep state).   |
//! | `F9`           | Screenshot → `./screenshots/screenshot-<unix_ms>.png`. |
//! | `Alt+Enter`    | Toggle fullscreen (desktop mode, integer-scaled).      |
//! | `Tab`          | Toggle fast-forward (run 4 frames per vsync tick).     |
//!
//! F1 (help overlay) is handled in `src/main.rs` directly. F2/F3 (debugger)
//! and F4/F6/F8 (viewers) are handled by the M27/M28 dispatchers and are
//! *not* routed through this module.
//!
//! See: https://www.nesdev.org/wiki/CPU_interrupts#RESET (soft reset)
//! See: https://wiki.libsdl.org/SDL2/SDL_SetWindowFullscreen (fullscreen)

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use sdl2::keyboard::{Keycode, Mod};

use crate::emulator::EmulatorState;
use crate::screenshot;
use crate::video::Video;

/// Number of emulator frames to run per vsync tick while fast-forward is
/// active. 4× keeps audio roughly in sync (we still push samples each
/// tick) while making the game visibly faster.
pub const FAST_FORWARD_FRAMES: u32 = 4;

/// Available turbo speed multipliers selectable from the F12 menu.
/// Spacebar uses the currently selected value while held.
pub const TURBO_SPEEDS: [f32; 5] = [0.25, 0.5, 2.0, 3.0, 4.0];

/// Default turbo speed index into [`TURBO_SPEEDS`] (2.0× = index 2).
pub const DEFAULT_TURBO_SPEED_INDEX: usize = 2;

/// Default directory for screenshot files.
pub const DEFAULT_SCREENSHOT_DIR: &str = "screenshots";

/// Bundle of M29 UI control state. Held by the main loop; [`UiHotkeys::handle_key`]
/// dispatches key events to the appropriate action.
pub struct UiHotkeys {
    /// Fast-forward flag — when `true`, the main loop runs
    /// [`FAST_FORWARD_FRAMES`] frames per vsync tick instead of one.
    fast_forward: bool,
    /// Turbo (Spacebar) hold flag — when `true`, the main loop runs at
    /// `turbo_ratio`× speed instead of 1×.
    turbo_held: bool,
    /// Current turbo speed multiplier (e.g. 2.0 = double speed).
    turbo_ratio: f32,
    /// Fractional frame accumulator for sub-1× turbo speeds. Each tick
    /// adds `turbo_ratio`; `floor(acc)` frames are stepped, then
    /// subtracted. For ≥1× ratios this always steps at least 1 frame.
    turbo_accumulator: f32,
    /// Directory where screenshot PNGs are written.
    screenshot_dir: PathBuf,
}

impl Default for UiHotkeys {
    fn default() -> Self {
        Self::new(PathBuf::from(DEFAULT_SCREENSHOT_DIR))
    }
}

impl UiHotkeys {
    /// Construct a dispatcher with the given screenshot output directory.
    pub fn new(screenshot_dir: PathBuf) -> Self {
        Self {
            fast_forward: false,
            turbo_held: false,
            turbo_ratio: TURBO_SPEEDS[DEFAULT_TURBO_SPEED_INDEX],
            turbo_accumulator: 0.0,
            screenshot_dir,
        }
    }

    /// Is fast-forward currently active?
    pub fn fast_forward(&self) -> bool {
        self.fast_forward
    }

    /// Is the turbo key (Spacebar) currently held?
    pub fn turbo_held(&self) -> bool {
        self.turbo_held
    }

    /// Stop turbo (called on Spacebar key-up).
    pub fn stop_turbo(&mut self) {
        self.turbo_held = false;
        self.turbo_accumulator = 0.0;
    }

    /// Current turbo speed multiplier.
    pub fn turbo_ratio(&self) -> f32 {
        self.turbo_ratio
    }

    /// Set the turbo speed multiplier.
    pub fn set_turbo_ratio(&mut self, ratio: f32) {
        self.turbo_ratio = ratio;
    }

    /// Return how many emulator frames to step this vsync tick while
    /// turbo is held. Uses a fractional accumulator so sub-1× speeds
    /// (e.g. 0.25×, 0.5×) skip frames correctly.
    pub fn turbo_frame_count(&mut self) -> u32 {
        self.turbo_accumulator += self.turbo_ratio;
        let frames = self.turbo_accumulator.floor() as u32;
        self.turbo_accumulator -= frames as f32;
        frames.max(0)
    }

    /// Handle a key-down event. Returns `true` if the key was consumed by
    /// a UI hotkey (and should *not* be forwarded to the joypad or debug
    /// dispatchers), `false` if it should be routed as usual.
    ///
    /// `keymod` is the SDL2 keyboard modifier bitmask at the time of the
    /// event. Modifier guards are strict: `Ctrl+R` requires Ctrl held
    /// *and* Alt *not* held (so `Ctrl+Alt+R` falls through); `Alt+Enter`
    /// requires Alt *and* not Ctrl. This keeps the bare keys (`R`,
    /// `Return`) routable to the joypad if a user binds them in
    /// `config.toml`. `Tab` is reserved unconditionally for
    /// fast-forward — do not bind NES buttons to it in `config.toml`.
    pub fn handle_key(
        &mut self,
        emulator: &mut EmulatorState,
        video: &mut Video,
        key: Keycode,
        keymod: Mod,
    ) -> bool {
        let ctrl = keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD);
        let alt = keymod.intersects(Mod::LALTMOD | Mod::RALTMOD);
        match key {
            // Ctrl+R (no Alt): soft reset. The 6502 RESET sequence
            // reloads PC from $FFFC/$FFFD, sets SP=$FD, sets the I flag.
            // PPU/APU/mapper keep their current state.
            Keycode::R if ctrl && !alt => {
                emulator.soft_reset();
                eprintln!("nes-emu: soft reset (CPU RESET sequence)");
                true
            }
            // F9: screenshot to PNG in the configured directory.
            Keycode::F9 => {
                self.save_screenshot(emulator);
                true
            }
            // Alt+Enter (no Ctrl): toggle fullscreen (desktop mode). The
            // next `Video::present` recomputes the integer-scaled dst
            // rect against the new desktop size.
            Keycode::Return if alt && !ctrl => {
                match video.toggle_fullscreen() {
                    Ok(()) => {
                        let state = if video.is_fullscreen() { "ON" } else { "OFF" };
                        eprintln!("nes-emu: fullscreen {state}");
                    }
                    Err(e) => eprintln!("nes-emu: could not toggle fullscreen: {e}"),
                }
                true
            }
            // Tab: toggle fast-forward. The main loop reads
            // `fast_forward()` each tick and runs `FAST_FORWARD_FRAMES`
            // frames instead of one when it is on. `Tab` is reserved
            // unconditionally — do not bind NES buttons to it in
            // `config.toml`.
            Keycode::Tab => {
                self.fast_forward = !self.fast_forward;
                let state = if self.fast_forward { "ON" } else { "OFF" };
                eprintln!("nes-emu: fast-forward {state}");
                true
            }
            // Space: hold for turbo speed. While held, the main loop
            // runs at `turbo_ratio`× speed (default 2×, configurable
            // via the F12 menu). Space is reserved unconditionally —
            // do not bind NES buttons to it in `config.toml`.
            Keycode::Space => {
                if !self.turbo_held {
                    self.turbo_held = true;
                    self.turbo_accumulator = 0.0;
                    eprintln!("nes-emu: turbo ON ({:.2}×)", self.turbo_ratio);
                }
                true
            }
            _ => false,
        }
    }

    /// Write the current framebuffer to a PNG file named
    /// `screenshot-<unix_ms>.png` in `screenshot_dir`. Errors
    /// are reported to stderr and do not crash the emulator.
    fn save_screenshot(&mut self, emulator: &EmulatorState) {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);

        let path = self
            .screenshot_dir
            .join(format!("screenshot-{now_ms}.png"));

        let fb = emulator.framebuffer();
        match screenshot::encode_to_path(
            fb,
            crate::video::NES_WIDTH,
            crate::video::NES_HEIGHT,
            &path,
        ) {
            Ok(()) => eprintln!("nes-emu: screenshot saved → {}", path.display()),
            Err(e) => eprintln!("nes-emu: screenshot failed: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_fast_forward_off() {
        let u = UiHotkeys::default();
        assert!(!u.fast_forward());
    }

}
