//! Integration tests for M31: per-channel APU volume / mute hotkeys.
//!
//! These exercise the `AudioHotkeys` dispatcher's key routing logic
//! against a real `Apu`. The SDL2 `Mod` bitmask is constructed directly
//! (no SDL2 event pump required).

use nes_emu::apu::Apu;
use nes_emu::audio_hotkeys::{AudioHotkeys, VOLUME_STEP};
use sdl2::keyboard::{Keycode, Mod};

/// `Mod` bitmask with only Alt held (left alt).
fn alt_mod() -> Mod {
    Mod::LALTMOD
}

/// `Mod` bitmask with Alt + Ctrl held.
fn alt_ctrl_mod() -> Mod {
    Mod::LALTMOD | Mod::LCTRLMOD
}

/// `Mod` bitmask with no modifiers.
fn no_mod() -> Mod {
    Mod::empty()
}

// ---- Non-audio keys fall through --------------------------------------

#[test]
fn non_alt_key_is_not_consumed() {
    let mut h = AudioHotkeys::new();
    let mut apu = Apu::new();
    assert!(!h.handle_key(&mut apu, Keycode::Num1, no_mod()));
    assert!(!h.handle_key(&mut apu, Keycode::M, no_mod()));
    assert!(!h.handle_key(&mut apu, Keycode::Up, no_mod()));
    assert_eq!(h.last_action(), "");
}

#[test]
fn alt_ctrl_combo_is_not_consumed() {
    // Alt+Ctrl should fall through so Ctrl+Alt+R etc. reach other
    // dispatchers (e.g. joypad).
    let mut h = AudioHotkeys::new();
    let mut apu = Apu::new();
    assert!(!h.handle_key(&mut apu, Keycode::Num1, alt_ctrl_mod()));
    assert!(!h.handle_key(&mut apu, Keycode::M, alt_ctrl_mod()));
}

#[test]
fn unhandled_alt_key_is_not_consumed() {
    let mut h = AudioHotkeys::new();
    let mut apu = Apu::new();
    // Alt+Z is not an audio hotkey.
    assert!(!h.handle_key(&mut apu, Keycode::Z, alt_mod()));
    assert_eq!(h.last_action(), "");
}

// ---- Channel selection (Alt+1..5) -------------------------------------

#[test]
fn alt_num1_selects_pulse1() {
    let mut h = AudioHotkeys::new();
    let mut apu = Apu::new();
    assert!(h.handle_key(&mut apu, Keycode::Num1, alt_mod()));
    assert_eq!(apu.selected_channel(), 0);
    assert_eq!(h.last_action(), "selected pulse1");
}

#[test]
fn alt_num2_selects_pulse2() {
    let mut h = AudioHotkeys::new();
    let mut apu = Apu::new();
    assert!(h.handle_key(&mut apu, Keycode::Num2, alt_mod()));
    assert_eq!(apu.selected_channel(), 1);
    assert_eq!(h.last_action(), "selected pulse2");
}

#[test]
fn alt_num3_selects_triangle() {
    let mut h = AudioHotkeys::new();
    let mut apu = Apu::new();
    assert!(h.handle_key(&mut apu, Keycode::Num3, alt_mod()));
    assert_eq!(apu.selected_channel(), 2);
    assert_eq!(h.last_action(), "selected triangle");
}

#[test]
fn alt_num4_selects_noise() {
    let mut h = AudioHotkeys::new();
    let mut apu = Apu::new();
    assert!(h.handle_key(&mut apu, Keycode::Num4, alt_mod()));
    assert_eq!(apu.selected_channel(), 3);
    assert_eq!(h.last_action(), "selected noise");
}

#[test]
fn alt_num5_selects_dmc() {
    let mut h = AudioHotkeys::new();
    let mut apu = Apu::new();
    assert!(h.handle_key(&mut apu, Keycode::Num5, alt_mod()));
    assert_eq!(apu.selected_channel(), 4);
    assert_eq!(h.last_action(), "selected dmc");
}

// ---- Mute toggle (Alt+M) ----------------------------------------------

#[test]
fn alt_m_toggles_mute_on_selected_channel() {
    let mut h = AudioHotkeys::new();
    let mut apu = Apu::new();
    // Select pulse1 (channel 0).
    assert!(h.handle_key(&mut apu, Keycode::Num1, alt_mod()));
    // Toggle mute → now muted.
    assert!(h.handle_key(&mut apu, Keycode::M, alt_mod()));
    assert!(apu.channel_muted_at(0));
    assert_eq!(h.last_action(), "muted pulse1");
    // Toggle again → unmuted.
    assert!(h.handle_key(&mut apu, Keycode::M, alt_mod()));
    assert!(!apu.channel_muted_at(0));
    assert_eq!(h.last_action(), "unmuted pulse1");
}

#[test]
fn alt_m_mutes_triangle_after_alt_num3() {
    let mut h = AudioHotkeys::new();
    let mut apu = Apu::new();
    assert!(h.handle_key(&mut apu, Keycode::Num3, alt_mod()));
    assert!(h.handle_key(&mut apu, Keycode::M, alt_mod()));
    assert!(apu.channel_muted_at(2));
    assert_eq!(h.last_action(), "muted triangle");
}

// ---- Volume adjust (Alt+Up / Alt+Down) --------------------------------

#[test]
fn alt_up_increments_selected_channel_volume() {
    let mut h = AudioHotkeys::new();
    let mut apu = Apu::new();
    // Default volume is 1.0; Alt+Up should clamp at 1.0.
    assert!(h.handle_key(&mut apu, Keycode::Up, alt_mod()));
    assert_eq!(apu.channel_volume(0), 1.0);
    // Lower volume first, then Alt+Up should raise it.
    apu.set_channel_volume(0, 0.5);
    assert!(h.handle_key(&mut apu, Keycode::Up, alt_mod()));
    assert!(
        (apu.channel_volume(0) - (0.5 + VOLUME_STEP)).abs() < 1e-6,
        "expected {}, got {}",
        0.5 + VOLUME_STEP,
        apu.channel_volume(0)
    );
}

#[test]
fn alt_down_decrements_selected_channel_volume() {
    let mut h = AudioHotkeys::new();
    let mut apu = Apu::new();
    // Default volume 1.0; Alt+Down → 1.0 - 0.05 = 0.95.
    assert!(h.handle_key(&mut apu, Keycode::Down, alt_mod()));
    assert!(
        (apu.channel_volume(0) - (1.0 - VOLUME_STEP)).abs() < 1e-6,
        "expected {}, got {}",
        1.0 - VOLUME_STEP,
        apu.channel_volume(0)
    );
}

#[test]
fn alt_down_clamps_at_zero() {
    let mut h = AudioHotkeys::new();
    let mut apu = Apu::new();
    apu.set_channel_volume(0, 0.01);
    assert!(h.handle_key(&mut apu, Keycode::Down, alt_mod()));
    assert_eq!(apu.channel_volume(0), 0.0);
}

#[test]
fn alt_up_affects_selected_channel_only() {
    let mut h = AudioHotkeys::new();
    let mut apu = Apu::new();
    // Select channel 2 (triangle).
    assert!(h.handle_key(&mut apu, Keycode::Num3, alt_mod()));
    assert!(h.handle_key(&mut apu, Keycode::Down, alt_mod()));
    // Only triangle volume changed.
    assert!((apu.channel_volume(2) - (1.0 - VOLUME_STEP)).abs() < 1e-6);
    assert_eq!(apu.channel_volume(0), 1.0);
    assert_eq!(apu.channel_volume(1), 1.0);
    assert_eq!(apu.channel_volume(3), 1.0);
    assert_eq!(apu.channel_volume(4), 1.0);
}

// ---- Reset all (Alt+0) ------------------------------------------------

#[test]
fn alt_num0_resets_all_channels() {
    let mut h = AudioHotkeys::new();
    let mut apu = Apu::new();
    // Mutate state: mute some channels, change volumes.
    apu.set_channel_muted(0, true);
    apu.set_channel_muted(3, true);
    apu.set_channel_volume(1, 0.3);
    apu.set_channel_volume(4, 0.0);
    // Alt+0 resets.
    assert!(h.handle_key(&mut apu, Keycode::Num0, alt_mod()));
    for i in 0..5 {
        assert!(!apu.channel_muted_at(i), "channel {i} should be unmuted");
        assert_eq!(apu.channel_volume(i), 1.0, "channel {i} should be 1.0");
    }
    assert_eq!(h.last_action(), "reset all channels");
}

// ---- Sequence of actions ----------------------------------------------

#[test]
fn select_then_mute_then_reset_sequence() {
    let mut h = AudioHotkeys::new();
    let mut apu = Apu::new();
    // Select noise (channel 3).
    assert!(h.handle_key(&mut apu, Keycode::Num4, alt_mod()));
    // Mute it.
    assert!(h.handle_key(&mut apu, Keycode::M, alt_mod()));
    assert!(apu.channel_muted_at(3));
    // Lower volume of DMC (channel 4) — need to select first.
    assert!(h.handle_key(&mut apu, Keycode::Num5, alt_mod()));
    assert!(h.handle_key(&mut apu, Keycode::Down, alt_mod()));
    assert!(h.handle_key(&mut apu, Keycode::Down, alt_mod()));
    assert!((apu.channel_volume(4) - (1.0 - 2.0 * VOLUME_STEP)).abs() < 1e-6);
    // Reset all.
    assert!(h.handle_key(&mut apu, Keycode::Num0, alt_mod()));
    assert!(!apu.channel_muted_at(3));
    assert_eq!(apu.channel_volume(4), 1.0);
}

// ---- Right Alt works too ----------------------------------------------

#[test]
fn right_alt_works_for_channel_select() {
    let mut h = AudioHotkeys::new();
    let mut apu = Apu::new();
    let ralt = Mod::RALTMOD;
    assert!(h.handle_key(&mut apu, Keycode::Num2, ralt));
    assert_eq!(apu.selected_channel(), 1);
}

// ---- last_action tracking ---------------------------------------------

#[test]
fn last_action_updates_on_each_consumed_key() {
    let mut h = AudioHotkeys::new();
    let mut apu = Apu::new();
    assert!(h.handle_key(&mut apu, Keycode::Num1, alt_mod()));
    assert_eq!(h.last_action(), "selected pulse1");
    assert!(h.handle_key(&mut apu, Keycode::M, alt_mod()));
    assert_eq!(h.last_action(), "muted pulse1");
    assert!(h.handle_key(&mut apu, Keycode::Num0, alt_mod()));
    assert_eq!(h.last_action(), "reset all channels");
}
