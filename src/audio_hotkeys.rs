//! Per-channel APU volume / mute hotkeys (M31).
//!
//! Runtime controls for the non-linear mixer's per-channel volume
//! scalars and mute flags. All hotkeys use the `Alt` modifier (without
//! `Ctrl`) so they do not collide with the `Ctrl+R` reset (M29) or the
//! bare number keys used for save-state slot selection (M30).
//!
//! | Key              | Action                                          |
//! |------------------|-------------------------------------------------|
//! | `Alt+1`..`Alt+5` | Select channel 0..4 (pulse1..dmc)               |
//! | `Alt+M`          | Toggle mute on the selected channel             |
//! | `Alt+Up`         | +0.05 volume on the selected channel            |
//! | `Alt+Down`       | -0.05 volume on the selected channel            |
//! | `Alt+0`          | Reset all channels to unmuted, volume 1.0       |
//!
//! The selected channel and current volumes live on the [`Apu`] itself
//! (so they survive save-state round-trips); this dispatcher only routes
//! keys. A short confirmation is printed to stderr on each action so the
//! user gets feedback in the absence of an on-screen overlay for audio.
//!
//! See: https://www.nesdev.org/wiki/APU_Mixer

use sdl2::keyboard::{Keycode, Mod};

use crate::apu::Apu;

/// Volume step applied by `Alt+Up` / `Alt+Down`.
pub const VOLUME_STEP: f32 = 0.05;

/// Channel names in APU index order, for stderr feedback.
const CHANNEL_NAMES: [&str; 5] = ["pulse1", "pulse2", "triangle", "noise", "dmc"];

/// Per-channel APU volume / mute hotkey dispatcher (M31).
///
/// Stateless aside from a "last action" string used for tests; all
/// persistent state (selected channel, volumes, mutes) lives on the
/// [`Apu`]. Construct with [`AudioHotkeys::default`].
#[derive(Default)]
pub struct AudioHotkeys {
    /// Human-readable description of the last consumed action (for tests
    /// and debugging). Empty if no key has been consumed yet.
    last_action: String,
}

impl AudioHotkeys {
    /// Create a new dispatcher.
    pub fn new() -> Self {
        Self::default()
    }

    /// Human-readable description of the last consumed action (e.g.
    /// `"selected pulse1"`, `"muted triangle"`, `"noise vol 0.45"`).
    /// Empty until a hotkey is consumed.
    pub fn last_action(&self) -> &str {
        &self.last_action
    }

    /// Route a keydown event. Returns `true` if the key was consumed
    /// (an Alt-modifier audio hotkey), `false` otherwise so the caller
    /// can fall through to the next dispatcher. Only `Alt` (without
    /// `Ctrl` / `Shift`) is accepted for the per-channel hotkeys so
    /// that `Ctrl+Alt+R` and other combos still fall through to the
    /// joypad / other dispatchers.
    pub fn handle_key(&mut self, apu: &mut Apu, key: Keycode, keymod: Mod) -> bool {
        // All audio hotkeys require Alt held and Ctrl *not* held. We
        // allow Shift (so Alt+Shift+digit still works on layouts where
        // the digit requires Shift) but ignore NUMMOD/CAPSMOD.
        let alt = keymod.intersects(Mod::LALTMOD | Mod::RALTMOD);
        if !alt {
            return false;
        }
        let ctrl = keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD);
        if ctrl {
            return false;
        }

        match key {
            // Alt+1..Alt+5: select channel 0..4.
            Keycode::Num1 => {
                apu.set_selected_channel(0);
                self.set_action(&format!("selected {}", CHANNEL_NAMES[0]));
                true
            }
            Keycode::Num2 => {
                apu.set_selected_channel(1);
                self.set_action(&format!("selected {}", CHANNEL_NAMES[1]));
                true
            }
            Keycode::Num3 => {
                apu.set_selected_channel(2);
                self.set_action(&format!("selected {}", CHANNEL_NAMES[2]));
                true
            }
            Keycode::Num4 => {
                apu.set_selected_channel(3);
                self.set_action(&format!("selected {}", CHANNEL_NAMES[3]));
                true
            }
            Keycode::Num5 => {
                apu.set_selected_channel(4);
                self.set_action(&format!("selected {}", CHANNEL_NAMES[4]));
                true
            }
            // Alt+0: reset all channels to unmuted, volume 1.0.
            Keycode::Num0 => {
                apu.reset_channel_mix();
                self.set_action("reset all channels");
                true
            }
            // Alt+M: toggle mute on the selected channel.
            Keycode::M => {
                let idx = apu.selected_channel() as usize;
                match apu.toggle_channel_mute(idx) {
                    Some(true) => {
                        self.set_action(&format!("muted {}", CHANNEL_NAMES[idx.min(4)]));
                        true
                    }
                    Some(false) => {
                        self.set_action(&format!("unmuted {}", CHANNEL_NAMES[idx.min(4)]));
                        true
                    }
                    None => false,
                }
            }
            // Alt+Up: +VOLUME_STEP on the selected channel.
            Keycode::Up => {
                let idx = apu.selected_channel() as usize;
                let v = (apu.channel_volume(idx) + VOLUME_STEP).clamp(0.0, 1.0);
                apu.set_channel_volume(idx, v);
                self.set_action(&format!("{} vol {:.2}", CHANNEL_NAMES[idx.min(4)], v));
                true
            }
            // Alt+Down: -VOLUME_STEP on the selected channel.
            Keycode::Down => {
                let idx = apu.selected_channel() as usize;
                let v = (apu.channel_volume(idx) - VOLUME_STEP).clamp(0.0, 1.0);
                apu.set_channel_volume(idx, v);
                self.set_action(&format!("{} vol {:.2}", CHANNEL_NAMES[idx.min(4)], v));
                true
            }
            _ => false,
        }
    }

    fn set_action(&mut self, s: &str) {
        self.last_action.clear();
        self.last_action.push_str(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_last_action_is_empty() {
        let h = AudioHotkeys::default();
        assert_eq!(h.last_action(), "");
    }

    #[test]
    fn volume_step_is_reasonable() {
        // 0.05 → 20 steps from 0.0 to 1.0.
        assert!((VOLUME_STEP - 0.05).abs() < 1e-6);
    }
}
