//! SDL2 keyboard + gamepad → NES joypad button mapping with configurable bindings.
//!
//! See: https://www.nesdev.org/wiki/Controller_port

#![allow(dead_code)]

use std::collections::HashMap;

use sdl2::controller::Button;
use sdl2::keyboard::Keycode;

use crate::config::Config;
use crate::joypad::{button, Joypad};

/// Default mapping from SDL2 `Keycode` to NES controller-1 buttons.
///
/// Returns `Some(button_index)` when the key is bound, `None` otherwise.
/// Lookups are exhaustive over the [`Keycode`] enum so the match stays
/// total (no `_ =>` arm that could silently swallow a new binding).
///
/// This is the legacy free function used before configurable bindings
/// landed (M13). It is kept for backward compatibility with existing tests
/// and as a convenience for callers that just want the default layout.
/// New code should prefer [`InputMapper`] built from a [`Config`].
pub fn keycode_to_button1(key: Keycode) -> Option<u8> {
    let btn = match key {
        Keycode::L => button::A,
        Keycode::K => button::B,
        Keycode::H => button::SELECT,
        Keycode::G => button::START,
        Keycode::W => button::UP,
        Keycode::S => button::DOWN,
        Keycode::A => button::LEFT,
        Keycode::D => button::RIGHT,
        _ => return None,
    };
    Some(btn)
}

/// Apply a keyboard key event to the joypad using the **default** key
/// bindings (controller 1 only).
///
/// `pressed` is `true` for a key-down event, `false` for key-up. If the
/// key is not bound to any NES button, this is a no-op. Only controller 1
/// is wired by default.
///
/// This is the legacy free function from M13. New code should use
/// [`InputMapper::handle_key`] for configurable bindings and controller 2
/// support.
pub fn handle_key(joypad: &mut Joypad, key: Keycode, pressed: bool) {
    if let Some(btn) = keycode_to_button1(key) {
        joypad.set_button(0, btn, pressed);
    }
}

/// Maps SDL2 host input events (keyboard + gamepad) to NES joypad button
/// presses using bindings from a [`Config`].
///
/// The mapper holds per-source button state (keyboard vs. gamepad) so that
/// both input sources can drive the same NES button simultaneously. When
/// either source presses a button, the NES button is pressed; it is only
/// released when **both** sources release it. This matches the NES
/// hardware behavior where multiple controllers' inputs are OR'd together.
///
/// The `gamepad_index` argument to [`InputMapper::handle_gamepad_button`]
/// is a **sequential NES controller index** (0, 1, ...), not the SDL2
/// joystick instance ID. The caller (typically `main.rs`) is responsible
/// for translating SDL2 instance IDs to sequential indices via a
/// `HashMap<u32, usize>` built when the gamepads are opened. Indices
/// beyond 1 are clamped to NES controller 0 (the NES only has two ports).
pub struct InputMapper {
    /// `Keycode → (controller, button)` for both keyboard controllers.
    /// A single keycode can only map to one (controller, button) pair —
    /// the last binding wins when building the map.
    keyboard: HashMap<Keycode, (usize, u8)>,
    /// `GameControllerButton → NES button index`, shared by all gamepads.
    /// Gamepad `N` applies this map to NES controller `N`.
    gamepad: HashMap<Button, u8>,
    /// Per-controller keyboard button bitmask (live state of all keyboard
    /// sources). Bit `b` set = keyboard is currently holding NES button
    /// `b` on this controller.
    kb_state: [u8; 2],
    /// Per-controller gamepad button bitmask (live state of all gamepad
    /// sources). Bit `b` set = some gamepad is currently holding NES
    /// button `b` on this controller.
    gp_state: [u8; 2],
}

impl InputMapper {
    /// Build a mapper from a [`Config`]. Invalid binding names (typos in
    /// `config.toml`) are silently skipped — [`Config::keycode_for`] /
    /// [`Config::gamepad_button_for`] already filter them out.
    pub fn from_config(config: &Config) -> Self {
        let mut keyboard = HashMap::new();
        for controller in 0..=1usize {
            for btn in 0..8u8 {
                if let Some(kc) = config.keycode_for(controller, btn) {
                    keyboard.insert(kc, (controller, btn));
                }
            }
        }
        let mut gamepad = HashMap::new();
        for btn in 0..8u8 {
            if let Some(gb) = config.gamepad_button_for(btn) {
                gamepad.insert(gb, btn);
            }
        }
        Self {
            keyboard,
            gamepad,
            kb_state: [0; 2],
            gp_state: [0; 2],
        }
    }

    /// Build a mapper from the default config (no `config.toml` needed).
    /// This is also available via `InputMapper::default()` (the `Default`
    /// trait).
    pub fn new_default() -> Self {
        Self::from_config(&Config::default())
    }

    /// Apply a keyboard key event to the joypad.
    ///
    /// `pressed` is `true` for key-down, `false` for key-up. If the key is
    /// not bound to any NES button on either controller, this is a no-op.
    /// The keyboard source state is tracked independently from gamepad
    /// state — releasing a keyboard key only clears the NES button if no
    /// gamepad is also holding it.
    pub fn handle_key(&mut self, joypad: &mut Joypad, key: Keycode, pressed: bool) {
        let Some(&(controller, btn)) = self.keyboard.get(&key) else {
            return;
        };
        if pressed {
            self.kb_state[controller] |= 1 << btn;
        } else {
            self.kb_state[controller] &= !(1 << btn);
        }
        // OR keyboard + gamepad state for this button — the NES button is
        // pressed if either source is holding it.
        let combined = self.kb_state[controller] | self.gp_state[controller];
        let is_pressed = (combined >> btn) & 1 != 0;
        joypad.set_button(controller, btn, is_pressed);
    }

    /// Apply a gamepad button event to the joypad.
    ///
    /// `gamepad_index` is the SDL2 controller index (0, 1, ...). It is
    /// clamped to NES controller `0` or `1` — gamepads beyond the second
    /// are routed to controller 1 (index 0) as a fallback so they remain
    /// usable rather than being dropped.
    ///
    /// The gamepad source state is tracked independently from keyboard
    /// state — releasing a gamepad button only clears the NES button if no
    /// keyboard key is also holding it.
    pub fn handle_gamepad_button(
        &mut self,
        joypad: &mut Joypad,
        gamepad_index: usize,
        gp_button: Button,
        pressed: bool,
    ) {
        let Some(&btn) = self.gamepad.get(&gp_button) else {
            return;
        };
        // Clamp to NES controller 0 or 1 — the NES only has two ports.
        let controller = if gamepad_index >= 2 { 0 } else { gamepad_index };
        if pressed {
            self.gp_state[controller] |= 1 << btn;
        } else {
            self.gp_state[controller] &= !(1 << btn);
        }
        // OR keyboard + gamepad state for this button.
        let combined = self.kb_state[controller] | self.gp_state[controller];
        let is_pressed = (combined >> btn) & 1 != 0;
        joypad.set_button(controller, btn, is_pressed);
    }

    /// Number of keyboard bindings (across both controllers).
    pub fn keyboard_binding_count(&self) -> usize {
        self.keyboard.len()
    }

    /// Number of gamepad button bindings.
    pub fn gamepad_binding_count(&self) -> usize {
        self.gamepad.len()
    }

    /// Look up the (controller, button) pair bound to a keycode, if any.
    /// Mainly useful for tests.
    pub fn keyboard_binding(&self, key: Keycode) -> Option<(usize, u8)> {
        self.keyboard.get(&key).copied()
    }

    /// Look up the NES button bound to a gamepad button, if any.
    pub fn gamepad_binding(&self, gp_button: Button) -> Option<u8> {
        self.gamepad.get(&gp_button).copied()
    }
}

impl Default for InputMapper {
    fn default() -> Self {
        Self::from_config(&Config::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    // --- Legacy free-function tests (preserved from M13) ---

    #[test]
    fn bound_keys_map_to_correct_buttons() {
        assert_eq!(keycode_to_button1(Keycode::L), Some(button::A));
        assert_eq!(keycode_to_button1(Keycode::K), Some(button::B));
        assert_eq!(keycode_to_button1(Keycode::H), Some(button::SELECT));
        assert_eq!(keycode_to_button1(Keycode::G), Some(button::START));
        assert_eq!(keycode_to_button1(Keycode::W), Some(button::UP));
        assert_eq!(keycode_to_button1(Keycode::S), Some(button::DOWN));
        assert_eq!(keycode_to_button1(Keycode::A), Some(button::LEFT));
        assert_eq!(keycode_to_button1(Keycode::D), Some(button::RIGHT));
    }

    #[test]
    fn unbound_keys_return_none() {
        assert_eq!(keycode_to_button1(Keycode::Return), None);
        assert_eq!(keycode_to_button1(Keycode::Space), None);
        assert_eq!(keycode_to_button1(Keycode::Q), None);
        assert_eq!(keycode_to_button1(Keycode::Z), None);
        assert_eq!(keycode_to_button1(Keycode::X), None);
    }

    #[test]
    fn handle_key_press_sets_button() {
        let mut j = Joypad::new();
        handle_key(&mut j, Keycode::L, true);
        assert_eq!(j.current(0), 1 << button::A);
    }

    #[test]
    fn handle_key_release_clears_button() {
        let mut j = Joypad::new();
        handle_key(&mut j, Keycode::L, true);
        handle_key(&mut j, Keycode::L, false);
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
        handle_key(&mut j, Keycode::L, true);
        assert_eq!(j.current(0), 1 << button::A);
        assert_eq!(j.current(1), 0);
    }

    #[test]
    fn multiple_keys_press_multiple_buttons() {
        let mut j = Joypad::new();
        handle_key(&mut j, Keycode::L, true); // A
        handle_key(&mut j, Keycode::W, true); // Up
        handle_key(&mut j, Keycode::G, true); // Start
        assert_eq!(
            j.current(0),
            (1 << button::A) | (1 << button::UP) | (1 << button::START)
        );
    }

    // --- InputMapper tests (M22) ---

    #[test]
    fn default_mapper_matches_legacy_bindings() {
        let m = InputMapper::default();
        assert_eq!(m.keyboard_binding(Keycode::L), Some((0, button::A)));
        assert_eq!(m.keyboard_binding(Keycode::K), Some((0, button::B)));
        assert_eq!(m.keyboard_binding(Keycode::H), Some((0, button::SELECT)));
        assert_eq!(m.keyboard_binding(Keycode::G), Some((0, button::START)));
        assert_eq!(m.keyboard_binding(Keycode::W), Some((0, button::UP)));
        assert_eq!(m.keyboard_binding(Keycode::S), Some((0, button::DOWN)));
        assert_eq!(m.keyboard_binding(Keycode::A), Some((0, button::LEFT)));
        assert_eq!(m.keyboard_binding(Keycode::D), Some((0, button::RIGHT)));
        // 8 keyboard bindings on controller 1, none on controller 2.
        assert_eq!(m.keyboard_binding_count(), 8);
    }

    #[test]
    fn default_mapper_has_gamepad_bindings() {
        let m = InputMapper::default();
        assert_eq!(m.gamepad_binding(Button::A), Some(button::A));
        assert_eq!(m.gamepad_binding(Button::B), Some(button::B));
        assert_eq!(m.gamepad_binding(Button::Back), Some(button::SELECT));
        assert_eq!(m.gamepad_binding(Button::Start), Some(button::START));
        assert_eq!(m.gamepad_binding(Button::DPadUp), Some(button::UP));
        assert_eq!(m.gamepad_binding(Button::DPadDown), Some(button::DOWN));
        assert_eq!(m.gamepad_binding(Button::DPadLeft), Some(button::LEFT));
        assert_eq!(m.gamepad_binding(Button::DPadRight), Some(button::RIGHT));
        assert_eq!(m.gamepad_binding_count(), 8);
    }

    #[test]
    fn mapper_handle_key_sets_button() {
        let mut m = InputMapper::default();
        let mut j = Joypad::new();
        m.handle_key(&mut j, Keycode::L, true);
        assert_eq!(j.current(0), 1 << button::A);
        m.handle_key(&mut j, Keycode::L, false);
        assert_eq!(j.current(0), 0);
    }

    #[test]
    fn mapper_handle_key_unbound_is_noop() {
        let mut m = InputMapper::default();
        let mut j = Joypad::new();
        m.handle_key(&mut j, Keycode::Return, true);
        assert_eq!(j.current(0), 0);
        assert_eq!(j.current(1), 0);
    }

    #[test]
    fn custom_kb1_binding_respected() {
        let mut c = Config::default();
        // Remap A from L to Return.
        c.keys
            .controller1
            .insert("A".to_string(), "Return".to_string());
        let mut m = InputMapper::from_config(&c);

        let mut j = Joypad::new();
        // L is no longer bound.
        m.handle_key(&mut j, Keycode::L, true);
        assert_eq!(j.current(0), 0);
        // Return is now A.
        m.handle_key(&mut j, Keycode::Return, true);
        assert_eq!(j.current(0), 1 << button::A);
    }

    #[test]
    fn custom_kb2_binding_respected() {
        let mut c = Config::default();
        c.keys.controller2.insert("A".to_string(), "Q".to_string());
        let mut m = InputMapper::from_config(&c);

        let mut j = Joypad::new();
        m.handle_key(&mut j, Keycode::Q, true);
        assert_eq!(j.current(0), 0);
        assert_eq!(j.current(1), 1 << button::A);
        m.handle_key(&mut j, Keycode::Q, false);
        assert_eq!(j.current(1), 0);
    }

    #[test]
    fn gamepad_button_sets_controller1() {
        let mut m = InputMapper::default();
        let mut j = Joypad::new();
        m.handle_gamepad_button(&mut j, 0, Button::A, true);
        assert_eq!(j.current(0), 1 << button::A);
        assert_eq!(j.current(1), 0);
        m.handle_gamepad_button(&mut j, 0, Button::A, false);
        assert_eq!(j.current(0), 0);
    }

    #[test]
    fn gamepad_button_sets_controller2() {
        let mut m = InputMapper::default();
        let mut j = Joypad::new();
        m.handle_gamepad_button(&mut j, 1, Button::DPadUp, true);
        assert_eq!(j.current(0), 0);
        assert_eq!(j.current(1), 1 << button::UP);
    }

    #[test]
    fn gamepad_index_beyond_two_falls_back_to_controller1() {
        let mut m = InputMapper::default();
        let mut j = Joypad::new();
        // Gamepad 5 → controller 0 (clamped).
        m.handle_gamepad_button(&mut j, 5, Button::A, true);
        assert_eq!(j.current(0), 1 << button::A);
        assert_eq!(j.current(1), 0);
    }

    #[test]
    fn unbound_gamepad_button_is_noop() {
        let mut m = InputMapper::default();
        let mut j = Joypad::new();
        // RightShoulder is not in the default gamepad bindings.
        m.handle_gamepad_button(&mut j, 0, Button::RightShoulder, true);
        assert_eq!(j.current(0), 0);
    }

    #[test]
    fn keyboard_and_gamepad_work_simultaneously() {
        // Both keyboard L and gamepad A map to NES A on controller 1.
        // Pressing either sets the button; the joypad ORs them.
        let mut m = InputMapper::default();
        let mut j = Joypad::new();

        // Press keyboard A.
        m.handle_key(&mut j, Keycode::L, true);
        assert_eq!(j.current(0), 1 << button::A);

        // Also press gamepad A — button stays set (OR semantics).
        m.handle_gamepad_button(&mut j, 0, Button::A, true);
        assert_eq!(j.current(0), 1 << button::A);

        // Release keyboard — gamepad still holding.
        m.handle_key(&mut j, Keycode::L, false);
        assert_eq!(
            j.current(0),
            1 << button::A,
            "releasing keyboard while gamepad held should keep button pressed"
        );

        // Release gamepad — now clear.
        m.handle_gamepad_button(&mut j, 0, Button::A, false);
        assert_eq!(j.current(0), 0);
    }

    #[test]
    fn custom_gamepad_binding_respected() {
        let mut c = Config::default();
        // Remap NES A from gamepad A to RightShoulder.
        c.gamepad
            .buttons
            .insert("A".to_string(), "RightShoulder".to_string());
        let mut m = InputMapper::from_config(&c);

        let mut j = Joypad::new();
        // Button::A is no longer bound.
        m.handle_gamepad_button(&mut j, 0, Button::A, true);
        assert_eq!(j.current(0), 0);
        // RightShoulder is now NES A.
        m.handle_gamepad_button(&mut j, 0, Button::RightShoulder, true);
        assert_eq!(j.current(0), 1 << button::A);
    }

    #[test]
    fn invalid_binding_in_config_is_skipped() {
        let mut c = Config::default();
        // Typo — not a real Keycode name. Should be silently skipped.
        c.keys
            .controller1
            .insert("A".to_string(), "NotAKey".to_string());
        let m = InputMapper::from_config(&c);
        // A is now unbound (the invalid name was skipped).
        assert_eq!(m.keyboard_binding(Keycode::L), None);
        assert_eq!(m.keyboard_binding_count(), 7); // 8 - 1 invalid
    }
}
