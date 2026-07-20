//! NES standard controller input — `$4016`/`$4017` strobe + shift-register
//! polling for two controllers.
//!
//! The NES controller is read through a serial shift register. The CPU
//! performs the following sequence to sample a controller:
//!
//! 1. **Strobe on**: write `1` to bit 0 of `$4016`. While the strobe is
//!    high, the controller's shift register is continuously reloaded with
//!    the live button state.
//! 2. **Strobe off**: write `0` to bit 0 of `$4016`. The shift register
//!    freezes, holding a snapshot of the buttons at the moment the strobe
//!    dropped.
//! 3. **Read 8 bits**: read `$4016` (controller 1) or `$4017` (controller 2)
//!    eight times. Each read returns the next button state in bit 0, in
//!    this order: **A, B, Select, Start, Up, Down, Left, Right**.
//!
//! After 8 reads the shift register is exhausted; subsequent reads return
//! `1` (open bus / "always 1" on a standard controller). Reads while the
//! strobe is held high continuously return the A button state.
//!
//! Button bit layout in the snapshot byte (bit 0 is read first):
//!
//! | Bit | Button  |
//! |-----|---------|
//! | 0   | A       |
//! | 1   | B       |
//! | 2   | Select  |
//! | 3   | Start   |
//! | 4   | Up      |
//! | 5   | Down    |
//! | 6   | Left    |
//! | 7   | Right   |
//!
//! See: https://www.nesdev.org/wiki/Controller_port
//! See: https://www.nesdev.org/wiki/Standard_controller

#![allow(dead_code)]

/// Number of controllers supported (controller 1 at `$4016`, controller 2
/// at `$4017`).
pub const CONTROLLER_COUNT: usize = 2;

/// Number of buttons on a standard NES controller.
pub const BUTTON_COUNT: u8 = 8;

/// Button bit indices in the snapshot byte. These double as the `button`
/// argument to [`Joypad::set_button`].
pub mod button {
    /// A button (bit 0 — read first).
    pub const A: u8 = 0;
    /// B button (bit 1).
    pub const B: u8 = 1;
    /// Select button (bit 2).
    pub const SELECT: u8 = 2;
    /// Start button (bit 3).
    pub const START: u8 = 3;
    /// Up on the D-pad (bit 4).
    pub const UP: u8 = 4;
    /// Down on the D-pad (bit 5).
    pub const DOWN: u8 = 5;
    /// Left on the D-pad (bit 6).
    pub const LEFT: u8 = 6;
    /// Right on the D-pad (bit 7).
    pub const RIGHT: u8 = 7;
}

/// Mask of all 8 button bits.
const ALL_BUTTONS: u8 = 0xFF;

/// The NES joypad state for two standard controllers.
///
/// Owns the live button state (updated by the host input layer) and the
/// shift-register snapshot used by the CPU polling sequence. The strobe
/// logic follows the NESdev wiki: a high strobe continuously reloads the
/// shift register; a `1 → 0` strobe transition freezes a snapshot and
/// resets the read counters.
///
/// This struct has no SDL2 dependency — it is pure emulation logic. The
/// `src/input.rs` module bridges SDL2 keyboard events into
/// [`Joypad::set_button`] calls.
///
/// See: https://www.nesdev.org/wiki/Controller_port
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Joypad {
    /// Live button state for each controller, one bit per button using the
    /// [`button`] module bit layout. Updated by the host input layer via
    /// [`Joypad::set_button`].
    #[serde(skip)]
    current: [u8; CONTROLLER_COUNT],

    /// Current strobe line state (bit 0 of the last write to `$4016`).
    strobe: bool,

    /// Frozen snapshot of the button state at the moment the strobe went
    /// from high to low. Read out one bit at a time by [`Joypad::read`].
    shift: [u8; CONTROLLER_COUNT],

    /// Number of reads performed since the last `1 → 0` strobe transition,
    /// per controller. Bits 0..8 of `shift` are returned in order; reads
    /// beyond 8 return `1`.
    counter: [u8; CONTROLLER_COUNT],
}

impl Joypad {
    /// Construct a joypad with no buttons pressed and the strobe low.
    pub fn new() -> Self {
        Self {
            current: [0; CONTROLLER_COUNT],
            strobe: false,
            shift: [0; CONTROLLER_COUNT],
            counter: [0; CONTROLLER_COUNT],
        }
    }

    /// Set or clear a button on the given controller (`0` or `1`).
    ///
    /// `button` is one of the [`button`] constants (`button::A`, etc.).
    /// `pressed` is `true` when the button is held down, `false` when
    /// released. While the strobe is high, the shift register tracks the
    /// live state so this update is immediately visible to reads.
    pub fn set_button(&mut self, controller: usize, button: u8, pressed: bool) {
        if controller >= CONTROLLER_COUNT || button >= BUTTON_COUNT {
            return;
        }
        if pressed {
            self.current[controller] |= 1 << button;
        } else {
            self.current[controller] &= !(1 << button);
        }
        // While the strobe is high the shift register is continuously
        // reloaded, so live changes are reflected immediately.
        if self.strobe {
            self.shift[controller] = self.current[controller];
        }
    }

    /// Convenience: press a button (equivalent to `set_button(.., true)`).
    pub fn press(&mut self, controller: usize, button: u8) {
        self.set_button(controller, button, true);
    }

    /// Convenience: release a button (equivalent to `set_button(.., false)`).
    pub fn release(&mut self, controller: usize, button: u8) {
        self.set_button(controller, button, false);
    }

    /// Clear all buttons on both controllers (e.g. on host focus loss).
    pub fn clear(&mut self) {
        for c in 0..CONTROLLER_COUNT {
            self.current[c] = 0;
            if self.strobe {
                self.shift[c] = 0;
            }
        }
    }

    /// Write to `$4016` bit 0 — the controller strobe line.
    ///
    /// A `1 → 0` transition snapshots the live button state into the
    /// shift registers and resets the read counters. While the strobe is
    /// high the shift registers continuously track the live state.
    ///
    /// See: https://www.nesdev.org/wiki/Controller_port#Writing
    pub fn write_strobe(&mut self, value: u8) {
        let new_strobe = (value & 1) != 0;
        if self.strobe && !new_strobe {
            // Falling edge: freeze a snapshot and reset the read counters.
            for c in 0..CONTROLLER_COUNT {
                self.shift[c] = self.current[c];
                self.counter[c] = 0;
            }
        }
        self.strobe = new_strobe;
        if self.strobe {
            // While strobed, the shift register tracks the live state.
            for c in 0..CONTROLLER_COUNT {
                self.shift[c] = self.current[c];
            }
        }
    }

    /// Read one bit from the given controller (`0` = `$4016`, `1` = `$4017`).
    ///
    /// Returns `1` or `0` in bit 0. While the strobe is high, reads return
    /// the live A button state. Otherwise each read returns the next button
    /// in order (A, B, Select, Start, Up, Down, Left, Right) and advances
    /// the counter; reads beyond 8 return `1`.
    ///
    /// See: https://www.nesdev.org/wiki/Controller_port#Reading
    pub fn read(&mut self, controller: usize) -> u8 {
        if controller >= CONTROLLER_COUNT {
            return 1;
        }
        if self.strobe {
            // While strobed, reads continuously return the A button.
            return self.current[controller] & 1;
        }
        let bit = if self.counter[controller] < BUTTON_COUNT {
            (self.shift[controller] >> self.counter[controller]) & 1
        } else {
            // Standard controllers return 1 after the 8 button bits.
            1
        };
        self.counter[controller] = self.counter[controller].saturating_add(1);
        bit
    }

    /// Current strobe line state.
    pub fn strobe(&self) -> bool {
        self.strobe
    }

    /// Live button state for a controller (for diagnostics / tests).
    pub fn current(&self, controller: usize) -> u8 {
        self.current.get(controller).copied().unwrap_or(0)
    }

    /// Frozen snapshot for a controller (for diagnostics / tests).
    pub fn shift(&self, controller: usize) -> u8 {
        self.shift.get(controller).copied().unwrap_or(0)
    }
}

impl Default for Joypad {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read all 8 buttons from a controller after a strobe cycle.
    fn read_all(joypad: &mut Joypad, controller: usize) -> [u8; 8] {
        let mut bits = [0u8; 8];
        for slot in bits.iter_mut() {
            *slot = joypad.read(controller);
        }
        bits
    }

    #[test]
    fn new_joypad_has_no_buttons_pressed() {
        let j = Joypad::new();
        assert_eq!(j.current(0), 0);
        assert_eq!(j.current(1), 0);
        assert!(!j.strobe());
    }

    #[test]
    fn strobe_cycle_returns_pressed_buttons_in_order() {
        let mut j = Joypad::new();
        // Press A and Start on controller 1.
        j.press(0, button::A);
        j.press(0, button::START);

        // Standard polling sequence: strobe on, strobe off, read 8 bits.
        j.write_strobe(1);
        j.write_strobe(0);

        let bits = read_all(&mut j, 0);
        // Order: A, B, Select, Start, Up, Down, Left, Right.
        assert_eq!(bits, [1, 0, 0, 1, 0, 0, 0, 0]);
    }

    #[test]
    fn all_eight_buttons_read_correctly() {
        let mut j = Joypad::new();
        // Press every button on controller 1.
        for b in 0..8 {
            j.press(0, b);
        }
        j.write_strobe(1);
        j.write_strobe(0);

        let bits = read_all(&mut j, 0);
        assert_eq!(bits, [1, 1, 1, 1, 1, 1, 1, 1]);
    }

    #[test]
    fn no_buttons_pressed_returns_all_zeros() {
        let mut j = Joypad::new();
        j.write_strobe(1);
        j.write_strobe(0);

        let bits = read_all(&mut j, 0);
        assert_eq!(bits, [0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn reads_beyond_eight_return_one() {
        let mut j = Joypad::new();
        j.write_strobe(1);
        j.write_strobe(0);

        // Read 8 button bits (all 0 since nothing pressed).
        for _ in 0..8 {
            assert_eq!(j.read(0), 0);
        }
        // Subsequent reads return 1 (standard controller behavior).
        for _ in 0..4 {
            assert_eq!(j.read(0), 1);
        }
    }

    #[test]
    fn strobe_high_reads_return_live_a_button() {
        let mut j = Joypad::new();
        j.press(0, button::A);
        j.write_strobe(1);
        // While strobe is high, reads return the live A state.
        assert_eq!(j.read(0), 1);
        assert_eq!(j.read(0), 1);
        // Release A mid-strobe: reads now return 0.
        j.release(0, button::A);
        assert_eq!(j.read(0), 0);
    }

    #[test]
    fn strobe_snapshot_freezes_state_at_falling_edge() {
        let mut j = Joypad::new();
        j.press(0, button::A);
        j.write_strobe(1);
        j.write_strobe(0);
        // Release A after the snapshot — the frozen bit should still be 1.
        j.release(0, button::A);
        assert_eq!(j.read(0), 1);
    }

    #[test]
    fn pressing_after_strobe_off_does_not_change_snapshot() {
        let mut j = Joypad::new();
        j.write_strobe(1);
        j.write_strobe(0);
        // Press A after the snapshot was taken.
        j.press(0, button::A);
        // First read should still be 0 (A was not pressed at snapshot time).
        assert_eq!(j.read(0), 0);
    }

    #[test]
    fn two_controllers_are_independent() {
        let mut j = Joypad::new();
        j.press(0, button::A);
        j.press(1, button::B);

        j.write_strobe(1);
        j.write_strobe(0);

        // Controller 1: A pressed → bit 0 = 1, B = 0.
        assert_eq!(j.read(0), 1);
        assert_eq!(j.read(0), 0);
        // Controller 2: A = 0, B pressed → bit 1 = 1.
        assert_eq!(j.read(1), 0);
        assert_eq!(j.read(1), 1);
    }

    #[test]
    fn read_counters_are_independent_per_controller() {
        let mut j = Joypad::new();
        j.press(0, button::A);
        j.press(1, button::START);
        j.write_strobe(1);
        j.write_strobe(0);

        // Interleaved reads: controller 1 reads A then B; controller 2
        // reads A then B then Select then Start.
        assert_eq!(j.read(0), 1); // ctrl1 bit0 = A = 1
        assert_eq!(j.read(1), 0); // ctrl2 bit0 = A = 0
        assert_eq!(j.read(0), 0); // ctrl1 bit1 = B = 0
        assert_eq!(j.read(1), 0); // ctrl2 bit1 = B = 0
        assert_eq!(j.read(1), 0); // ctrl2 bit2 = Select = 0
        assert_eq!(j.read(1), 1); // ctrl2 bit3 = Start = 1
    }

    #[test]
    fn re_strobing_resets_read_counter() {
        let mut j = Joypad::new();
        j.press(0, button::A);
        j.write_strobe(1);
        j.write_strobe(0);
        // Read 3 bits.
        assert_eq!(j.read(0), 1);
        assert_eq!(j.read(0), 0);
        assert_eq!(j.read(0), 0);
        // Re-strobe: counter resets to 0, A is read first again.
        j.write_strobe(1);
        j.write_strobe(0);
        assert_eq!(j.read(0), 1);
    }

    #[test]
    fn clear_releases_all_buttons() {
        let mut j = Joypad::new();
        for b in 0..8 {
            j.press(0, b);
            j.press(1, b);
        }
        j.clear();
        assert_eq!(j.current(0), 0);
        assert_eq!(j.current(1), 0);
        j.write_strobe(1);
        j.write_strobe(0);
        assert_eq!(read_all(&mut j, 0), [0; 8]);
        assert_eq!(read_all(&mut j, 1), [0; 8]);
    }

    #[test]
    fn release_clears_a_pressed_button() {
        let mut j = Joypad::new();
        j.press(0, button::A);
        j.release(0, button::A);
        j.write_strobe(1);
        j.write_strobe(0);
        assert_eq!(j.read(0), 0);
    }

    #[test]
    fn set_button_out_of_range_controller_is_ignored() {
        let mut j = Joypad::new();
        j.set_button(5, button::A, true);
        // No panic; nothing changed.
        assert_eq!(j.current(0), 0);
    }

    #[test]
    fn set_button_out_of_range_button_index_is_ignored() {
        let mut j = Joypad::new();
        // Button index >= 8 would overflow `1 << button` in debug builds;
        // the guard makes this a no-op instead of a panic.
        j.set_button(0, 8, true);
        j.set_button(0, 255, true);
        assert_eq!(j.current(0), 0);
    }

    #[test]
    fn read_out_of_range_controller_returns_one() {
        let mut j = Joypad::new();
        assert_eq!(j.read(5), 1);
    }

    #[test]
    fn full_polling_sequence_matches_nesdev_order() {
        // Press one button at a time and confirm the read order:
        // A, B, Select, Start, Up, Down, Left, Right.
        let order = [
            button::A,
            button::B,
            button::SELECT,
            button::START,
            button::UP,
            button::DOWN,
            button::LEFT,
            button::RIGHT,
        ];
        for (idx, &btn) in order.iter().enumerate() {
            let mut j = Joypad::new();
            j.press(0, btn);
            j.write_strobe(1);
            j.write_strobe(0);
            let bits = read_all(&mut j, 0);
            // Only the idx-th read should be 1.
            for (i, &b) in bits.iter().enumerate() {
                assert_eq!(
                    b,
                    if i == idx { 1 } else { 0 },
                    "button {btn} at index {idx}: bit {i} = {b}"
                );
            }
        }
    }

    #[test]
    fn opposite_dpad_directions_can_both_be_pressed() {
        // The NES hardware does not prevent opposing directions; it's up to
        // the game. We should faithfully report both.
        let mut j = Joypad::new();
        j.press(0, button::UP);
        j.press(0, button::DOWN);
        j.write_strobe(1);
        j.write_strobe(0);
        let bits = read_all(&mut j, 0);
        // Up=bit4, Down=bit5.
        assert_eq!(bits[4], 1);
        assert_eq!(bits[5], 1);
    }

    #[test]
    fn strobe_writes_only_care_about_bit0() {
        let mut j = Joypad::new();
        j.press(0, button::A);
        // 0xFE has bit 0 clear → strobe low (no snapshot taken yet).
        j.write_strobe(0xFE);
        assert!(!j.strobe());
        // 0x01 has bit 0 set → strobe high.
        j.write_strobe(0x01);
        assert!(j.strobe());
        // 0x02 has bit 0 clear → strobe falling edge → snapshot taken.
        j.write_strobe(0x02);
        assert!(!j.strobe());
        assert_eq!(j.read(0), 1); // A was pressed at snapshot time.
    }

    #[test]
    fn all_buttons_mask_covers_eight_bits() {
        assert_eq!(ALL_BUTTONS, 0xFF);
        assert_eq!(BUTTON_COUNT, 8);
    }
}
