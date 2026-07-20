//! Region / TV system hotkeys (M32) — F11 cycles NTSC → PAL → Dendy → NTSC.
//!
//! The dispatcher is wired into the main loop's key-handling chain
//! (after audio hotkeys, before joypad routing). On each F11 press it
//! advances the emulator's region via [`Region::cycle`] and propagates
//! the new region to the PPU (scanline count + prerender + palette) and
//! APU (frame-counter thresholds) through [`EmulatorState::set_region`].
//!
//! The cycle order (NTSC → PAL → Dendy) lets the user quickly compare
//! the three modes without navigating a menu. The new region is also
//! surfaced via [`RegionHotkeys::last_region`] so the main loop can log
//! it or display it in the OSD.
//!
//! See: https://www.nesdev.org/wiki/Cycle_reference

#![allow(dead_code)]

use sdl2::keyboard::{Keycode, Mod};

use crate::emulator::EmulatorState;
use crate::region::Region;

/// `F11` cycles the region: NTSC → PAL → Dendy → NTSC.
pub const REGION_CYCLE_KEY: Keycode = Keycode::F11;

/// Region / TV system hotkey dispatcher.
///
/// Held by the main loop; [`RegionHotkeys::handle_key`] is called for
/// each `KeyDown` event before joypad routing. Returns `true` when the
/// key was consumed (so the caller skips joypad routing for it).
#[derive(Debug, Default)]
pub struct RegionHotkeys {
    /// The most recent region set via F11, or `None` if F11 has not
    /// been pressed yet. The main loop can read this to log the change
    /// or update the OSD.
    last_region: Option<Region>,
}

impl RegionHotkeys {
    /// Build a new dispatcher with no prior region change.
    pub fn new() -> Self {
        Self::default()
    }

    /// The most recent region set via F11, or `None` if F11 has not
    /// been pressed yet.
    pub fn last_region(&self) -> Option<Region> {
        self.last_region
    }

    /// Handle a key-down event. Returns `true` if the key was consumed
    /// (region cycled), `false` if it should fall through to the next
    /// dispatcher.
    ///
    /// `F11` with no held modifiers (no Ctrl, no Alt) cycles the region.
    /// Modifier-guarded F11 (e.g. Alt+F11) falls through so it can be
    /// bound by the user to other actions.
    pub fn handle_key(&mut self, emulator: &mut EmulatorState, key: Keycode, keymod: Mod) -> bool {
        if key != REGION_CYCLE_KEY {
            return false;
        }
        // Strict no-modifier guard: bare F11 only. Ctrl+F11 / Alt+F11
        // fall through so they remain available for other bindings.
        let ctrl =
            (keymod & Mod::LCTRLMOD) != Mod::empty() || (keymod & Mod::RCTRLMOD) != Mod::empty();
        let alt =
            (keymod & Mod::LALTMOD) != Mod::empty() || (keymod & Mod::RALTMOD) != Mod::empty();
        if ctrl || alt {
            return false;
        }
        let next = emulator.region().cycle();
        emulator.set_region(next);
        self.last_region = Some(next);
        eprintln!("nes-emu: region switched to {}", next.short_name());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::Cartridge;

    /// Build a minimal NROM-128 cartridge for testing.
    fn make_nop_cart() -> Cartridge {
        let mut bytes = vec![b'N', b'E', b'S', 0x1A, 1, 0, 0, 0];
        bytes.extend_from_slice(&[0u8; 8]);
        bytes.resize(16 + 16 * 1024, 0xEA);
        let reset_off = 16 + 0x3FFC;
        bytes[reset_off] = 0x00;
        bytes[reset_off + 1] = 0xC0;
        Cartridge::from_bytes(&bytes).expect("build NOP cart")
    }

    #[test]
    fn f11_cycles_region() {
        let mut emu = EmulatorState::new(make_nop_cart());
        emu.reset();
        assert_eq!(emu.region(), Region::Ntsc);
        let mut hk = RegionHotkeys::new();
        assert!(hk.handle_key(&mut emu, Keycode::F11, Mod::empty()));
        assert_eq!(emu.region(), Region::Pal);
        assert_eq!(hk.last_region(), Some(Region::Pal));
        assert!(hk.handle_key(&mut emu, Keycode::F11, Mod::empty()));
        assert_eq!(emu.region(), Region::Dendy);
        assert!(hk.handle_key(&mut emu, Keycode::F11, Mod::empty()));
        assert_eq!(emu.region(), Region::Ntsc);
    }

    #[test]
    fn non_f11_falls_through() {
        let mut emu = EmulatorState::new(make_nop_cart());
        let mut hk = RegionHotkeys::new();
        assert!(!hk.handle_key(&mut emu, Keycode::F1, Mod::empty()));
        assert!(!hk.handle_key(&mut emu, Keycode::Return, Mod::empty()));
        assert_eq!(emu.region(), Region::Ntsc);
        assert_eq!(hk.last_region(), None);
    }

    #[test]
    fn modifier_f11_falls_through() {
        let mut emu = EmulatorState::new(make_nop_cart());
        let mut hk = RegionHotkeys::new();
        assert!(!hk.handle_key(&mut emu, Keycode::F11, Mod::LCTRLMOD));
        assert!(!hk.handle_key(&mut emu, Keycode::F11, Mod::LALTMOD));
        assert_eq!(emu.region(), Region::Ntsc);
        assert_eq!(hk.last_region(), None);
    }

    #[test]
    fn region_propagates_to_ppu_and_apu() {
        let mut emu = EmulatorState::new(make_nop_cart());
        let mut hk = RegionHotkeys::new();
        hk.handle_key(&mut emu, Keycode::F11, Mod::empty());
        assert_eq!(emu.bus().ppu().region(), Region::Pal);
        assert_eq!(emu.bus().apu().region(), Region::Pal);
    }

    #[test]
    fn default_last_region_is_none() {
        let hk = RegionHotkeys::new();
        assert_eq!(hk.last_region(), None);
    }
}
