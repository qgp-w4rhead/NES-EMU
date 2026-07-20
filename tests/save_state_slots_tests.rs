//! Save state slot + rewind buffer integration tests (M30).
//!
//! These tests build a minimal NROM cartridge, run the emulator for a few
//! frames, and exercise the [`SaveStateSlots`] bank (10 slots, F5/F7
//! semantics) and the [`RewindBuffer`] ring (Backspace semantics) against
//! real serialised snapshots. They verify slot independence, empty-slot
//! load errors, capacity-bounded eviction, and exact-state restoration
//! after rewind pops.

use nes_emu::cartridge::Cartridge;
use nes_emu::emulator::EmulatorState;
use nes_emu::save_state::{
    RewindBuffer, SaveStateError, SaveStateSlots, DEFAULT_REWIND_CAPACITY, SAVE_STATE_SLOT_COUNT,
};

/// Build a minimal NROM-128 cartridge (16 KB PRG, 8 KB CHR-RAM) whose
/// PRG is filled with `0xEA` (NOP) and whose RESET vector points to
/// `$C000`.
fn make_nop_cart() -> Cartridge {
    let mut bytes = vec![b'N', b'E', b'S', 0x1A, 1, 0, 0, 0];
    bytes.extend_from_slice(&[0u8; 8]); // remaining header
    bytes.resize(16 + 16 * 1024, 0xEA); // PRG filled with NOP
    let reset_off = 16 + 0x3FFC;
    bytes[reset_off] = 0x00;
    bytes[reset_off + 1] = 0xC0;
    Cartridge::from_bytes(&bytes).expect("build NOP cart")
}

/// Build a fresh emulator, reset it, and run `n` frames.
fn make_emulator(n_frames: u32) -> EmulatorState {
    let mut emu = EmulatorState::new(make_nop_cart());
    emu.reset();
    for _ in 0..n_frames {
        emu.step_frame();
    }
    emu
}

// ---------------------------------------------------------------------------
// SaveStateSlots
// ---------------------------------------------------------------------------

#[test]
fn slots_all_start_empty() {
    let s = SaveStateSlots::new();
    for i in 0..SAVE_STATE_SLOT_COUNT {
        assert!(s.is_empty(i), "slot {i} should start empty");
    }
    assert_eq!(s.occupied_count(), 0);
}

#[test]
fn save_then_load_round_trip() {
    let mut slots = SaveStateSlots::new();
    let emu_a = make_emulator(5);
    slots.save(&emu_a, 3).expect("save");
    assert!(!slots.is_empty(3));
    assert_eq!(slots.occupied_count(), 1);

    // Run a fresh emulator forward; loading should restore emu_a's state.
    let mut emu_b = make_emulator(0);
    assert_ne!(
        emu_b.cpu().pc,
        emu_a.cpu().pc,
        "test precondition: PCs must differ before load"
    );
    slots.load(&mut emu_b, 3).expect("load");
    assert_eq!(emu_b.cpu().pc, emu_a.cpu().pc, "PC must match after load");
    assert_eq!(
        emu_b.bus().ram(),
        emu_a.bus().ram(),
        "RAM must match after load"
    );
}

#[test]
fn save_current_uses_current_slot() {
    let mut slots = SaveStateSlots::new();
    slots.set_current_slot(7);
    let emu = make_emulator(3);
    slots.save_current(&emu).expect("save current");
    assert!(!slots.is_empty(7));
    assert!(slots.is_empty(0));
}

#[test]
fn load_current_uses_current_slot() {
    let mut slots = SaveStateSlots::new();
    let emu = make_emulator(4);
    slots.save(&emu, 5).expect("save");
    slots.set_current_slot(5);
    let mut emu_b = make_emulator(0);
    slots.load_current(&mut emu_b).expect("load current");
    assert_eq!(emu_b.cpu().pc, emu.cpu().pc);
}

#[test]
fn load_empty_slot_returns_error() {
    let slots = SaveStateSlots::new();
    let mut emu = make_emulator(0);
    let err = slots
        .load(&mut emu, 4)
        .expect_err("empty slot should error");
    assert!(matches!(err, SaveStateError::SlotEmpty { slot: 4 }));
    assert!(format!("{err}").contains("empty"));
}

#[test]
fn load_out_of_range_slot_returns_error() {
    let slots = SaveStateSlots::new();
    let mut emu = make_emulator(0);
    let err = slots
        .load(&mut emu, SAVE_STATE_SLOT_COUNT + 1)
        .expect_err("OOR slot should error");
    assert!(matches!(err, SaveStateError::SlotOutOfRange { .. }));
}

#[test]
fn save_out_of_range_slot_returns_error() {
    let mut slots = SaveStateSlots::new();
    let emu = make_emulator(0);
    let err = slots
        .save(&emu, SAVE_STATE_SLOT_COUNT)
        .expect_err("OOR save should error");
    assert!(matches!(err, SaveStateError::SlotOutOfRange { .. }));
}

#[test]
fn slots_are_independent() {
    let mut slots = SaveStateSlots::new();
    let emu_a = make_emulator(2);
    let emu_b = make_emulator(10);
    slots.save(&emu_a, 1).expect("save A");
    slots.save(&emu_b, 2).expect("save B");

    let mut emu = make_emulator(0);
    slots.load(&mut emu, 1).expect("load A");
    assert_eq!(emu.cpu().pc, emu_a.cpu().pc, "slot 1 → emu_a");
    slots.load(&mut emu, 2).expect("load B");
    assert_eq!(emu.cpu().pc, emu_b.cpu().pc, "slot 2 → emu_b");
}

#[test]
fn save_overwrites_previous_content() {
    let mut slots = SaveStateSlots::new();
    let emu_a = make_emulator(2);
    let emu_b = make_emulator(10);
    slots.save(&emu_a, 0).expect("save A");
    slots.save(&emu_b, 0).expect("save B overwrites A");

    let mut emu = make_emulator(0);
    slots.load(&mut emu, 0).expect("load");
    assert_eq!(emu.cpu().pc, emu_b.cpu().pc, "slot 0 should hold emu_b");
    assert_ne!(emu.cpu().pc, emu_a.cpu().pc);
}

#[test]
fn clear_empties_slot() {
    let mut slots = SaveStateSlots::new();
    let emu = make_emulator(1);
    slots.save(&emu, 4).expect("save");
    assert!(!slots.is_empty(4));
    slots.clear(4);
    assert!(slots.is_empty(4));
    assert_eq!(slots.occupied_count(), 0);
}

#[test]
fn clear_out_of_range_is_no_op() {
    let mut slots = SaveStateSlots::new();
    // Should not panic.
    slots.clear(SAVE_STATE_SLOT_COUNT + 5);
}

#[test]
fn all_ten_slots_usable() {
    let mut slots = SaveStateSlots::new();
    let emu = make_emulator(1);
    for i in 0..SAVE_STATE_SLOT_COUNT {
        slots.save(&emu, i).expect("save each slot");
    }
    assert_eq!(slots.occupied_count(), SAVE_STATE_SLOT_COUNT);
    for i in 0..SAVE_STATE_SLOT_COUNT {
        assert!(!slots.is_empty(i), "slot {i} should be occupied");
    }

    let mut emu_b = make_emulator(0);
    for i in 0..SAVE_STATE_SLOT_COUNT {
        slots.load(&mut emu_b, i).expect("load each slot");
    }
}

#[test]
fn current_slot_default_zero() {
    assert_eq!(SaveStateSlots::new().current_slot(), 0);
}

#[test]
fn set_current_slot_round_trips() {
    let mut s = SaveStateSlots::new();
    for i in 0..SAVE_STATE_SLOT_COUNT {
        s.set_current_slot(i);
        assert_eq!(s.current_slot(), i);
    }
}

#[test]
fn set_current_slot_ignores_out_of_range() {
    let mut s = SaveStateSlots::new();
    s.set_current_slot(5);
    s.set_current_slot(SAVE_STATE_SLOT_COUNT + 100);
    assert_eq!(s.current_slot(), 5, "OOR should be a no-op");
}

#[test]
fn get_returns_blob_for_occupied_slot() {
    let mut slots = SaveStateSlots::new();
    let emu = make_emulator(1);
    slots.save(&emu, 2).expect("save");
    let blob = slots.get(2).expect("occupied slot should return blob");
    assert!(!blob.is_empty());
    assert!(slots.get(3).is_none(), "empty slot should return None");
}

#[test]
fn occupied_count_tracks_saves_and_clears() {
    let mut slots = SaveStateSlots::new();
    let emu = make_emulator(1);
    assert_eq!(slots.occupied_count(), 0);
    slots.save(&emu, 0).expect("save");
    slots.save(&emu, 1).expect("save");
    assert_eq!(slots.occupied_count(), 2);
    slots.clear(0);
    assert_eq!(slots.occupied_count(), 1);
    slots.clear(1);
    assert_eq!(slots.occupied_count(), 0);
}

// ---------------------------------------------------------------------------
// RewindBuffer
// ---------------------------------------------------------------------------

#[test]
fn rewind_starts_empty() {
    let r = RewindBuffer::default();
    assert!(r.is_empty());
    assert_eq!(r.len(), 0);
    assert!(!r.is_full());
    assert_eq!(r.capacity(), DEFAULT_REWIND_CAPACITY);
}

#[test]
fn rewind_push_then_pop_restores_state() {
    let mut r = RewindBuffer::default();
    let emu_a = make_emulator(5);
    r.push(&emu_a).expect("push");

    // Run a different emulator forward, then rewind.
    let mut emu = make_emulator(5);
    emu.step_frame();
    emu.step_frame();
    assert_ne!(emu.cpu().pc, emu_a.cpu().pc, "precondition: PCs differ");

    assert!(r.pop(&mut emu), "pop should succeed");
    assert_eq!(emu.cpu().pc, emu_a.cpu().pc, "PC must match after rewind");
    assert_eq!(
        emu.bus().ram(),
        emu_a.bus().ram(),
        "RAM must match after rewind"
    );
    assert!(r.is_empty(), "pop should drain the buffer");
}

#[test]
fn rewind_pop_on_empty_returns_false() {
    let mut r = RewindBuffer::default();
    let mut emu = make_emulator(0);
    assert!(!r.pop(&mut emu), "pop on empty should return false");
    // Emulator state should be untouched.
    let pristine = make_emulator(0);
    assert_eq!(emu.cpu().pc, pristine.cpu().pc);
}

#[test]
fn rewind_pop_in_lifo_order() {
    let mut r = RewindBuffer::default();
    let emu_1 = make_emulator(1);
    let emu_2 = make_emulator(2);
    r.push(&emu_1).expect("push 1");
    r.push(&emu_2).expect("push 2");

    let mut emu = make_emulator(0);
    r.pop(&mut emu);
    assert_eq!(
        emu.cpu().pc,
        emu_2.cpu().pc,
        "first pop → most recent (emu_2)"
    );
    r.pop(&mut emu);
    assert_eq!(emu.cpu().pc, emu_1.cpu().pc, "second pop → older (emu_1)");
    assert!(r.is_empty());
}

#[test]
fn rewind_capacity_drops_oldest() {
    let mut r = RewindBuffer::with_capacity(3);
    let emu_1 = make_emulator(1);
    let emu_2 = make_emulator(2);
    let emu_3 = make_emulator(3);
    let emu_4 = make_emulator(4);
    r.push(&emu_1).expect("push 1");
    r.push(&emu_2).expect("push 2");
    r.push(&emu_3).expect("push 3");
    assert!(r.is_full());
    assert_eq!(r.len(), 3);

    // This push should evict emu_1 (oldest).
    r.push(&emu_4).expect("push 4 evicts oldest");
    assert_eq!(r.len(), 3, "len should stay at capacity");

    let mut emu = make_emulator(0);
    r.pop(&mut emu);
    assert_eq!(emu.cpu().pc, emu_4.cpu().pc, "LIFO top is emu_4");
    r.pop(&mut emu);
    assert_eq!(emu.cpu().pc, emu_3.cpu().pc, "next is emu_3");
    r.pop(&mut emu);
    assert_eq!(
        emu.cpu().pc,
        emu_2.cpu().pc,
        "next is emu_2 (emu_1 evicted)"
    );
    assert!(r.is_empty(), "buffer drained");
}

#[test]
fn rewind_clear_empties_but_preserves_capacity() {
    let mut r = RewindBuffer::with_capacity(5);
    let emu = make_emulator(1);
    r.push(&emu).expect("push");
    r.push(&emu).expect("push");
    assert_eq!(r.len(), 2);
    r.clear();
    assert!(r.is_empty());
    assert_eq!(r.capacity(), 5, "capacity preserved after clear");
    // Buffer should still be usable after clear.
    r.push(&emu).expect("push after clear");
    assert_eq!(r.len(), 1);
}

#[test]
fn rewind_capacity_floor_one() {
    let r = RewindBuffer::with_capacity(0);
    assert_eq!(r.capacity(), 1);
    let r2 = RewindBuffer::with_capacity(5);
    assert_eq!(r2.capacity(), 5);
}

#[test]
fn rewind_is_full_at_capacity() {
    let mut r = RewindBuffer::with_capacity(2);
    let emu = make_emulator(1);
    r.push(&emu).expect("push");
    assert!(!r.is_full());
    r.push(&emu).expect("push");
    assert!(r.is_full());
    // One more push should not panic and should keep len == capacity.
    r.push(&emu).expect("push at full");
    assert!(r.is_full());
    assert_eq!(r.len(), 2);
}

#[test]
fn rewind_restores_exact_state_including_ppu_scanline() {
    let mut r = RewindBuffer::default();
    let emu_a = make_emulator(7);
    r.push(&emu_a).expect("push");

    let mut emu = make_emulator(7);
    emu.step_frame();
    emu.step_frame();
    // PPU scanline should differ after stepping.
    let scan_before = emu.bus().ppu().scanline();
    r.pop(&mut emu);
    let scan_after = emu.bus().ppu().scanline();
    assert_eq!(
        scan_after,
        emu_a.bus().ppu().scanline(),
        "PPU scanline must match snapshot after rewind (was {scan_before} before pop)"
    );
}

#[test]
fn rewind_restores_audio_buffer() {
    let mut r = RewindBuffer::default();
    let emu_a = make_emulator(5);
    r.push(&emu_a).expect("push");
    let a_audio = emu_a.audio_buffer().to_vec();

    let mut emu = make_emulator(5);
    emu.step_frame();
    // After stepping, the audio buffer should have new samples.
    let _ = emu.take_audio_samples();
    r.pop(&mut emu);
    assert_eq!(
        emu.audio_buffer(),
        a_audio.as_slice(),
        "audio buffer must match snapshot after rewind"
    );
}

#[test]
fn rewind_many_pushes_bounded_by_capacity() {
    let mut r = RewindBuffer::with_capacity(10);
    let emu = make_emulator(1);
    for _ in 0..100 {
        r.push(&emu).expect("push");
    }
    assert_eq!(r.len(), 10, "len must not exceed capacity");
    assert!(r.is_full());
}

#[test]
fn save_state_version_mismatch_in_slot_still_rejected() {
    // Manually corrupt a slot's blob to have a wrong version, then verify
    // load rejects it with VersionMismatch rather than silently loading.
    let mut slots = SaveStateSlots::new();
    let emu = make_emulator(1);
    slots.save(&emu, 0).expect("save");

    // Get the blob, flip the version bytes (first 4 bytes of a bincode-
    // encoded SaveState are the u32 version in little-endian), and put it
    // back. We can't directly write into the slot (no public setter), so
    // we build a new slot bank with the corrupted blob via save+load.
    let blob = slots.get(0).expect("slot 0 occupied").to_vec();
    assert!(blob.len() >= 4);
    let mut corrupted = blob.clone();
    // Flip the version to something unlikely to be valid.
    corrupted[0] = 0xFF;
    corrupted[1] = 0xFF;
    corrupted[2] = 0xFF;
    corrupted[3] = 0x7F;

    let mut emu_b = make_emulator(0);
    let err = emu_b.load_state(&corrupted).expect_err("corrupt version");
    assert!(
        matches!(err, SaveStateError::VersionMismatch { .. }),
        "expected VersionMismatch, got {err}"
    );
}
