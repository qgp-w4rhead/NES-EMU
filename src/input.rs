//! SDL2 keyboard event → NES joypad button mapping.
//!
//! This module bridges the host input layer (SDL2 `Keycode` events) into
//! the emulator-core [`Joypad`](crate::joypad::Joypad) struct, which has no
//! SDL2 dependency. Keeping the SDL2 types out of `joypad.rs` means the
//! emulation core stays portable and unit-testable without an SDL2
//! context.
//!
//! # Default key bindings
//!
//! | NES button | Keyboard key |
//! |------------|--------------|
//! | A          | Z            |
//! | B          | X            |
//! | Select     | A            |
//! | Start      | S            |
//! | Up         | Up arrow     |
//! | Down       | Down arrow   |
//! | Left       | Left arrow   |
//! | Right      | Right arrow  |
//!
//! These mirror the layout used by common NES emulators (FCEUX / Nestopia):
//! `Z`/`X` for the right-hand action buttons, arrow keys for the D-pad, and
//! `A`/`S` for Select/Start on the left side of the keyboard. Only
//! controller 1 is mapped by default; controller 2 is reserved for future
//! gamepad support.
//!
//! See: https://www.nesdev.org/wiki/Controller_port

#![allow(dead_code)]

use sdl2::keyboard::Keycode;

use crate::joypad::{button, Joypad};

/// Default mapping from SDL2 `Keycode` to NES controller-1 buttons.
///
/// Returns `Some(button_index)` when the key is bound, `None` otherwise.
/// Lookups are exhaustive over the [`Keycode`] enum so the match stays
/// total (no `_ =>` arm that could silently swallow a new binding).
pub fn keycode_to_button1(key: Keycode) -> Option<u8> {
    let btn = match key {
        Keycode::Z => button::A,
        Keycode::X => button::B,
        Keycode::A => button::SELECT,
        Keycode::S => button::START,
        Keycode::Up => button::UP,
        Keycode::Down => button::DOWN,
        Keycode::Left => button::LEFT,
        Keycode::Right => button::RIGHT,
        _ => return None,
    };
    Some(btn)
}

/// Apply a keyboard key event to the joypad.
///
/// `pressed` is `true` for a key-down event, `false` for key-up. If the
/// key is not bound to any NES button, this is a no-op. Only controller 1
/// is wired by default.
pub fn handle_key(joypad: &mut Joypad, key: Keycode, pressed: bool) {
    if let Some(btn) = keycode_to_button1(key) {
        joypad.set_button(0, btn, pressed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bound_keys_map_to_correct_buttons() {
        assert_eq!(keycode_to_button1(Keycode::Z), Some(button::A));
        assert_eq!(keycode_to_button1(Keycode::X), Some(button::B));
        assert_eq!(keycode_to_button1(Keycode::A), Some(button::SELECT));
        assert_eq!(keycode_to_button1(Keycode::S), Some(button::START));
        assert_eq!(keycode_to_button1(Keycode::Up), Some(button::UP));
        assert_eq!(keycode_to_button1(Keycode::Down), Some(button::DOWN));
        assert_eq!(keycode_to_button1(Keycode::Left), Some(button::LEFT));
        assert_eq!(keycode_to_button1(Keycode::Right), Some(button::RIGHT));
    }

    #[test]
    fn unbound_keys_return_none() {
        assert_eq!(keycode_to_button1(Keycode::Return), None);
        assert_eq!(keycode_to_button1(Keycode::Space), None);
        assert_eq!(keycode_to_button1(Keycode::Q), None);
    }

    #[test]
    fn handle_key_press_sets_button() {
        let mut j = Joypad::new();
        handle_key(&mut j, Keycode::Z, true);
        assert_eq!(j.current(0), 1 << button::A);
    }

    #[test]
    fn handle_key_release_clears_button() {
        let mut j = Joypad::new();
        handle_key(&mut j, Keycode::Z, true);
        handle_key(&mut j, Keycode::Z, false);
        assert_eq!(j.current(0), 0);
    }

    #[test]
    fn handle_key_unbound_is_noop() {
        let mut j = Joypad::new();
        handle_key(&mut j, Keycode::Return, true);
        assert_eq!(j.current(0), 0);
    }

    #[test]
    fn handle_key_only_affects_controller1() {
        let mut j = Joypad::new();
        handle_key(&mut j, Keycode::Z, true);
        assert_eq!(j.current(0), 1 << button::A);
        assert_eq!(j.current(1), 0);
    }

    #[test]
    fn multiple_keys_press_multiple_buttons() {
        let mut j = Joypad::new();
        handle_key(&mut j, Keycode::Z, true); // A
        handle_key(&mut j, Keycode::Up, true); // Up
        handle_key(&mut j, Keycode::S, true); // Start
                                              // A=bit0, Up=bit4, Start=bit3 → 0b0001_1001 = 0x19.
        assert_eq!(
            j.current(0),
            (1 << button::A) | (1 << button::UP) | (1 << button::START)
        );
    }
}
