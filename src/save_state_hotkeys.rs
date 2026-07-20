//! M30 save-state / rewind / OSD hotkey dispatcher.
//!
//! Centralizes the M30 control-feature key handling so `src/main.rs` stays
//! under the 400-line file-size limit. The dispatcher owns the
//! [`SaveStateSlots`] bank, the [`RewindBuffer`], and the [`Osd`] state;
//! it does *not* own any SDL2 state — the `EmulatorState` is borrowed per
//! call.
//!
//! # Hotkeys
//!
//! | Key            | Action                                                  |
//! |----------------|---------------------------------------------------------|
//! | `F5`           | Save state to current slot.                             |
//! | `F7`           | Load state from current slot.                           |
//! | `F10`          | Toggle on-screen display (FPS / mapper / game / slot).  |
//! | `Backspace`    | Rewind one frame (pop the rewind buffer).               |
//! | `1..=9`        | Select save state slot 0..=8.                           |
//! | `0`            | Select save state slot 9.                               |
//!
//! Modifier guards are strict: the number keys are only intercepted when
//! no `Ctrl` / `Alt` / `Shift` modifier is held, so `Ctrl+1`, `Alt+1`,
//! and `Shift+1` still route to the joypad (some users bind NES buttons
//! to number keys). `F5` / `F7` / `F10` / `Backspace` are reserved
//! unconditionally — do not bind NES buttons to them in `config.toml`.
//!
//! See: https://www.nesdev.org/wiki/Save_state (slot conventions)
//! See: [`crate::save_state`] for the slot + rewind data structures.
//! See: [`crate::osd`] for the on-screen display renderer.

use sdl2::keyboard::{Keycode, Mod};

use crate::emulator::EmulatorState;
use crate::osd::{build_lines, Osd};
use crate::save_state::{RewindBuffer, SaveStateError, SaveStateSlots};

/// Number of emulator frames between rewind buffer snapshots. 2 = every
/// other frame (halves the per-frame serialisation cost while keeping
/// ~2 s of rewind depth at the default capacity of 60). Set to 1 for
/// maximum granularity.
pub const REWIND_INTERVAL: u32 = 2;

/// Bundle of M30 save-state / rewind / OSD state. Held by the main loop;
/// [`SaveStateHotkeys::handle_key`] dispatches key events to the
/// appropriate action.
pub struct SaveStateHotkeys {
    /// 10 save state slots (F5 save / F7 load).
    pub slots: SaveStateSlots,
    /// Ring buffer of recent snapshots for the rewind feature
    /// (Backspace pops one frame).
    pub rewind: RewindBuffer,
    /// On-screen display state (F10 toggles).
    pub osd: Osd,
    /// Game name shown in the OSD (ROM file stem, set at startup).
    pub game_name: String,
    /// Frame counter used to throttle rewind snapshots to every
    /// `REWIND_INTERVAL`-th frame.
    frame_counter: u32,
}

impl Default for SaveStateHotkeys {
    fn default() -> Self {
        Self {
            slots: SaveStateSlots::new(),
            rewind: RewindBuffer::default(),
            osd: Osd::new(),
            game_name: String::new(),
            frame_counter: 0,
        }
    }
}

impl SaveStateHotkeys {
    /// Construct a dispatcher with the given game name (ROM file stem)
    /// for OSD display.
    pub fn new(game_name: String) -> Self {
        Self {
            game_name,
            ..Self::default()
        }
    }

    /// Is the OSD currently visible on screen?
    pub fn osd_enabled(&self) -> bool {
        self.osd.enabled()
    }

    /// Current save-state slot index (`0..SAVE_STATE_SLOT_COUNT`).
    pub fn current_slot(&self) -> usize {
        self.slots.current_slot()
    }

    /// Capture a rewind snapshot if the frame counter hit the
    /// `REWIND_INTERVAL` boundary. Called once per presented frame by the
    /// main loop. Errors from the serialiser are reported to stderr and
    /// do not crash the emulator.
    pub fn maybe_push_rewind(&mut self, emulator: &EmulatorState) {
        self.frame_counter = self.frame_counter.wrapping_add(1);
        if self.frame_counter % REWIND_INTERVAL != 0 {
            return;
        }
        if let Err(e) = self.rewind.push(emulator) {
            eprintln!("nes-emu: rewind snapshot failed: {e}");
        }
    }

    /// Render the OSD into the framebuffer. No-op when the OSD is
    /// disabled. The lines shown are built from the current FPS, the
    /// supplied `mapper_number`, game name, slot, and rewind depth.
    ///
    /// `mapper_number` is passed in by the caller (rather than borrowed
    /// from the emulator) so the caller can take the mutable framebuffer
    /// borrow without conflicting with an immutable emulator borrow.
    pub fn render_osd(
        &self,
        mapper_number: u16,
        region: crate::region::Region,
        fb: &mut [u32],
        width: u32,
        height: u32,
    ) {
        if !self.osd.enabled() {
            return;
        }
        let slot = self.slots.current_slot();
        let slot_occupied = !self.slots.is_empty(slot);
        let lines_str = build_lines(
            self.osd.fps(),
            mapper_number,
            &self.game_name,
            slot,
            slot_occupied,
            self.rewind.len(),
            region,
        );
        let lines: Vec<&str> = lines_str.iter().map(|s| s.as_str()).collect();
        self.osd.render(fb, width, height, &lines);
    }

    /// Post-frame hook: capture a rewind snapshot, then (if the OSD is
    /// enabled) record the frame timestamp, blit the OSD into the
    /// framebuffer, and re-present via the supplied `present` closure.
    /// Extracted from `src/main.rs` to keep the binary under the 400-line
    /// file-size limit.
    ///
    /// The rewind snapshot is taken *before* the OSD blit so the snapshot
    /// stays clean of OSD pixels — the OSD is a host-side overlay, not
    /// part of the emulated framebuffer.
    ///
    /// `present` is called with the post-OSD framebuffer so the overlay is
    /// actually visible this frame. It is only invoked when the OSD is
    /// enabled — the caller is responsible for presenting when the OSD is
    /// off (see `src/main.rs`). This avoids a double-present that would
    /// halve the frame rate (the renderer is vsync-locked, so each
    /// `present` blocks until the next vsync).
    pub fn post_frame(
        &mut self,
        emulator: &mut EmulatorState,
        present: impl FnOnce(&[u32]) -> Result<(), String>,
    ) -> Result<(), String> {
        self.maybe_push_rewind(emulator);
        if self.osd_enabled() {
            self.osd.record_frame(std::time::Instant::now());
            let mapper_number = emulator.mapper_number();
            let region = emulator.region();
            let fb = emulator.framebuffer_mut();
            self.render_osd(
                mapper_number,
                region,
                fb,
                crate::video::NES_WIDTH,
                crate::video::NES_HEIGHT,
            );
            // Re-borrow the framebuffer immutably for the present call.
            // The mutable borrow above has ended, so this is fine.
            present(emulator.framebuffer())?;
        }
        Ok(())
    }

    /// Handle a key-down event. Returns `true` if the key was consumed by
    /// a save-state / rewind / OSD hotkey (and should *not* be forwarded
    /// to the joypad or debug dispatchers), `false` if it should be
    /// routed as usual.
    ///
    /// `keymod` is the SDL2 keyboard modifier bitmask at the time of the
    /// event. Number-key slot selection requires no *held* modifiers
    /// (Shift / Ctrl / Alt) so `Ctrl+1` / `Alt+1` / `Shift+1` still route
    /// to the joypad. Toggle states (Num Lock, Caps Lock) are intentionally
    /// *not* checked — they appear in `keymod` whenever active and would
    /// otherwise break slot selection on systems where Num Lock is on by
    /// default.
    pub fn handle_key(&mut self, emulator: &mut EmulatorState, key: Keycode, keymod: Mod) -> bool {
        let no_mod = !keymod.intersects(
            Mod::LSHIFTMOD
                | Mod::RSHIFTMOD
                | Mod::LCTRLMOD
                | Mod::RCTRLMOD
                | Mod::LALTMOD
                | Mod::RALTMOD,
        );
        match key {
            // F5: save state to current slot.
            Keycode::F5 => {
                let slot = self.slots.current_slot();
                match self.slots.save_current(emulator) {
                    Ok(()) => eprintln!(
                        "nes-emu: state saved to slot {}/{}",
                        slot + 1,
                        crate::save_state::SAVE_STATE_SLOT_COUNT
                    ),
                    Err(e) => eprintln!("nes-emu: save state failed: {e}"),
                }
                true
            }
            // F7: load state from current slot.
            Keycode::F7 => {
                let slot = self.slots.current_slot();
                match self.slots.load_current(emulator) {
                    Ok(()) => {
                        eprintln!(
                            "nes-emu: state loaded from slot {}/{}",
                            slot + 1,
                            crate::save_state::SAVE_STATE_SLOT_COUNT
                        );
                        // Loading a save state invalidates the rewind
                        // buffer — the snapshots in it belong to a
                        // timeline that no longer matches the restored
                        // state. Clear it so a subsequent Backspace does
                        // not jump to an unrelated frame.
                        self.rewind.clear();
                    }
                    Err(SaveStateError::SlotEmpty { .. }) => {
                        eprintln!(
                            "nes-emu: slot {}/{} is empty — nothing to load",
                            slot + 1,
                            crate::save_state::SAVE_STATE_SLOT_COUNT
                        );
                    }
                    Err(e) => eprintln!("nes-emu: load state failed: {e}"),
                }
                true
            }
            // F10: toggle OSD.
            Keycode::F10 => {
                let on = self.osd.toggle();
                eprintln!("nes-emu: OSD {}", if on { "ON" } else { "OFF" });
                true
            }
            // Backspace: rewind one frame.
            Keycode::Backspace => {
                if self.rewind.pop(emulator) {
                    eprintln!(
                        "nes-emu: rewound one frame ({} snapshots left)",
                        self.rewind.len()
                    );
                } else {
                    eprintln!("nes-emu: rewind buffer empty — nothing to rewind");
                }
                true
            }
            // Number keys 1..=9 select slots 0..=8; 0 selects slot 9.
            // Only when no modifiers are held so Ctrl+1 / Alt+1 / Shift+1
            // still route to the joypad.
            Keycode::Num1 if no_mod => {
                self.select_slot(0);
                true
            }
            Keycode::Num2 if no_mod => {
                self.select_slot(1);
                true
            }
            Keycode::Num3 if no_mod => {
                self.select_slot(2);
                true
            }
            Keycode::Num4 if no_mod => {
                self.select_slot(3);
                true
            }
            Keycode::Num5 if no_mod => {
                self.select_slot(4);
                true
            }
            Keycode::Num6 if no_mod => {
                self.select_slot(5);
                true
            }
            Keycode::Num7 if no_mod => {
                self.select_slot(6);
                true
            }
            Keycode::Num8 if no_mod => {
                self.select_slot(7);
                true
            }
            Keycode::Num9 if no_mod => {
                self.select_slot(8);
                true
            }
            Keycode::Num0 if no_mod => {
                self.select_slot(9);
                true
            }
            _ => false,
        }
    }

    /// Select a save state slot and report it to stderr.
    fn select_slot(&mut self, slot: usize) {
        self.slots.set_current_slot(slot);
        let occupied = !self.slots.is_empty(slot);
        eprintln!(
            "nes-emu: slot {}/{} selected ({})",
            slot + 1,
            crate::save_state::SAVE_STATE_SLOT_COUNT,
            if occupied { "occupied" } else { "empty" }
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_no_osd() {
        let h = SaveStateHotkeys::default();
        assert!(!h.osd_enabled());
    }

    #[test]
    fn default_slot_zero() {
        let h = SaveStateHotkeys::default();
        assert_eq!(h.current_slot(), 0);
    }

    #[test]
    fn default_rewind_empty() {
        let h = SaveStateHotkeys::default();
        assert!(h.rewind.is_empty());
    }

    #[test]
    fn new_sets_game_name() {
        let h = SaveStateHotkeys::new("TestGame".to_string());
        assert_eq!(h.game_name, "TestGame");
    }
}
