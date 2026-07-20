//! Save state slots — 10 independent slots for F5/F7 save/load (M30).
//!
//! See: https://www.nesdev.org/wiki/Save_state (slot conventions)

use super::SaveStateError;
use crate::emulator::EmulatorState;

/// Number of save state slots exposed to the user (slots `0..=9`).
///
/// The slot index is selected with the number keys `1..=9, 0` (where `0`
/// selects slot 9); `F5` saves to the current slot, `F7` loads from it.
pub const SAVE_STATE_SLOT_COUNT: usize = 10;

/// A fixed-size bank of save state slots, each holding an optional
/// serialised [`super::SaveState`] blob. Slots are independent — saving to
/// one does not affect the others — and persist only for the lifetime of
/// the emulator process (they are *not* written to disk by this struct;
/// the caller may serialise the whole `SaveStateSlots` if it wants
/// persistence).
///
/// `current_slot` is the slot that `F5` / `F7` act on; it is selected by
/// the number keys `1..=9, 0` (which select slots `0..=9`).
#[derive(Debug, Default)]
pub struct SaveStateSlots {
    /// The `N` slots; `None` = empty (never saved or cleared).
    slots: Box<[Option<Vec<u8>>; SAVE_STATE_SLOT_COUNT]>,
    /// Currently-selected slot index (`0..SAVE_STATE_SLOT_COUNT`).
    current_slot: usize,
}

impl SaveStateSlots {
    /// Build an empty slot bank with `current_slot = 0`.
    pub fn new() -> Self {
        Self::default()
    }

    /// The currently-selected slot index (`0..SAVE_STATE_SLOT_COUNT`).
    pub fn current_slot(&self) -> usize {
        self.current_slot
    }

    /// Select the current slot. Out-of-range values are silently ignored
    /// (the current slot is unchanged) rather than panicking.
    pub fn set_current_slot(&mut self, slot: usize) {
        if slot < SAVE_STATE_SLOT_COUNT {
            self.current_slot = slot;
        }
    }

    /// Is the given slot empty (no save stored)?
    pub fn is_empty(&self, slot: usize) -> bool {
        self.slots
            .get(slot)
            .map(|opt| opt.is_none())
            .unwrap_or(true)
    }

    /// Serialise `emulator` into the given slot, overwriting any previous
    /// content. Returns [`SaveStateError::SlotOutOfRange`] if `slot >=
    /// SAVE_STATE_SLOT_COUNT`; other errors from the underlying serialiser
    /// are propagated as [`SaveStateError::Encode`].
    pub fn save(&mut self, emulator: &EmulatorState, slot: usize) -> Result<(), SaveStateError> {
        if slot >= SAVE_STATE_SLOT_COUNT {
            return Err(SaveStateError::SlotOutOfRange {
                requested: slot,
                max: SAVE_STATE_SLOT_COUNT,
            });
        }
        let blob = emulator.save_state()?;
        self.slots[slot] = Some(blob);
        Ok(())
    }

    /// Serialise `emulator` into the [`current_slot`].
    pub fn save_current(&mut self, emulator: &EmulatorState) -> Result<(), SaveStateError> {
        let slot = self.current_slot;
        self.save(emulator, slot)
    }

    /// Restore `emulator` from the given slot. Returns
    /// [`SaveStateError::SlotEmpty`] if the slot is empty,
    /// [`SaveStateError::SlotOutOfRange`] if `slot >=
    /// SAVE_STATE_SLOT_COUNT`.
    pub fn load(&self, emulator: &mut EmulatorState, slot: usize) -> Result<(), SaveStateError> {
        if slot >= SAVE_STATE_SLOT_COUNT {
            return Err(SaveStateError::SlotOutOfRange {
                requested: slot,
                max: SAVE_STATE_SLOT_COUNT,
            });
        }
        match self.slots[slot].as_ref() {
            Some(blob) => emulator.load_state(blob),
            None => Err(SaveStateError::SlotEmpty { slot }),
        }
    }

    /// Restore `emulator` from the [`current_slot`].
    pub fn load_current(&self, emulator: &mut EmulatorState) -> Result<(), SaveStateError> {
        let slot = self.current_slot;
        self.load(emulator, slot)
    }

    /// Clear the given slot (drop its stored blob). No-op if already empty
    /// or out of range.
    pub fn clear(&mut self, slot: usize) {
        if slot < SAVE_STATE_SLOT_COUNT {
            self.slots[slot] = None;
        }
    }

    /// Borrow the raw blob stored in `slot`, or `None` if empty / OOR.
    pub fn get(&self, slot: usize) -> Option<&[u8]> {
        self.slots.get(slot).and_then(|opt| opt.as_deref())
    }

    /// Number of slots currently holding a save.
    pub fn occupied_count(&self) -> usize {
        self.slots.iter().filter(|opt| opt.is_some()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_start_empty() {
        let s = SaveStateSlots::new();
        for i in 0..SAVE_STATE_SLOT_COUNT {
            assert!(s.is_empty(i), "slot {i} should be empty");
        }
        assert_eq!(s.occupied_count(), 0);
    }

    #[test]
    fn current_slot_default_zero() {
        assert_eq!(SaveStateSlots::new().current_slot(), 0);
    }

    #[test]
    fn set_current_slot_ignores_out_of_range() {
        let mut s = SaveStateSlots::new();
        s.set_current_slot(SAVE_STATE_SLOT_COUNT + 5);
        assert_eq!(s.current_slot(), 0);
        s.set_current_slot(7);
        assert_eq!(s.current_slot(), 7);
    }
}
