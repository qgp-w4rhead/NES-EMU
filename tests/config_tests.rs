//! Integration tests for `config.toml` parsing, persistence, and
//! `InputMapper` wiring (M22 — gamepad support + key rebinding).
//!
//! These tests exercise the public `Config` and `InputMapper` APIs against
//! real files on disk and real SDL2 `Keycode` / `GameControllerButton`
//! name resolution. They do not open an SDL2 device — that is verified
//! through the `InputMapper` mapping logic, which is device-independent.

use std::collections::BTreeMap;
use std::path::PathBuf;

use nes_emu::config::{ChannelVolumes, Config, GamepadBindings, KeyBindings};
use nes_emu::input::InputMapper;
use nes_emu::joypad::{button, Joypad};
use sdl2::controller::Button;
use sdl2::keyboard::Keycode;

/// Unique per-test temp dir under the system temp dir. Avoids pulling in a
/// `tempfile` dev-dependency — the tech stack keeps deps minimal.
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("nes-emu-m22-{}-{}", std::process::id(), name));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cleanup(dir: &PathBuf) {
    let _ = std::fs::remove_dir_all(dir);
}

// ---------------------------------------------------------------------------
// Config file persistence
// ---------------------------------------------------------------------------

#[test]
fn default_config_file_is_written_on_first_run() {
    let dir = temp_dir("default_written");
    let path = dir.join("config.toml");
    let _ = std::fs::remove_file(&path);

    let created = Config::ensure_default_file(&path).expect("ensure");
    assert!(created, "first call should create the file");
    assert!(path.exists(), "config.toml should exist on disk");

    // The written file should parse back to the default config.
    let loaded = Config::load_from_path(&path).expect("load");
    assert_eq!(loaded.keycode_for(0, button::A), Some(Keycode::Z));
    assert_eq!(loaded.gamepad_button_for(button::A), Some(Button::A));

    cleanup(&dir);
}

#[test]
fn ensure_default_file_is_idempotent() {
    let dir = temp_dir("idempotent");
    let path = dir.join("config.toml");
    let _ = std::fs::remove_file(&path);

    let c1 = Config::ensure_default_file(&path).expect("ensure 1");
    let c2 = Config::ensure_default_file(&path).expect("ensure 2");
    assert!(c1);
    assert!(!c2, "second call must not overwrite an existing file");

    cleanup(&dir);
}

#[test]
fn custom_bindings_persist_across_save_load() {
    let dir = temp_dir("persist");
    let path = dir.join("config.toml");

    let mut c = Config::default();
    c.keys
        .controller1
        .insert("A".to_string(), "Return".to_string());
    c.keys.controller2.insert("B".to_string(), "Q".to_string());
    c.gamepad
        .buttons
        .insert("Select".to_string(), "LeftShoulder".to_string());
    c.audio_volume = 0.5;
    c.window_scale = 4;
    c.save_to_path(&path).expect("save");

    // Simulate a restart: load the file fresh.
    let loaded = Config::load_from_path(&path).expect("load");
    assert_eq!(loaded.keycode_for(0, button::A), Some(Keycode::Return));
    assert_eq!(loaded.keycode_for(1, button::B), Some(Keycode::Q));
    assert_eq!(
        loaded.gamepad_button_for(button::SELECT),
        Some(Button::LeftShoulder)
    );
    assert_eq!(loaded.audio_volume, 0.5);
    assert_eq!(loaded.window_scale, 4);

    // Untouched defaults are preserved.
    assert_eq!(loaded.keycode_for(0, button::B), Some(Keycode::X));
    assert_eq!(loaded.gamepad_button_for(button::A), Some(Button::A));

    cleanup(&dir);
}

#[test]
fn missing_config_file_yields_defaults() {
    let dir = temp_dir("missing");
    let path = dir.join("does-not-exist.toml");
    let _ = std::fs::remove_file(&path);

    let c = Config::load_from_path(&path).expect("missing file is Ok");
    assert_eq!(c.keycode_for(0, button::A), Some(Keycode::Z));
    assert_eq!(c.gamepad_button_for(button::A), Some(Button::A));
    assert_eq!(c.audio_volume, 1.0);
    assert_eq!(c.window_scale, 3);

    cleanup(&dir);
}

#[test]
fn malformed_config_file_is_an_error() {
    let dir = temp_dir("malformed");
    let path = dir.join("bad.toml");
    std::fs::write(&path, "this is = not = valid = toml =").unwrap();

    let err = Config::load_from_path(&path);
    assert!(
        err.is_err(),
        "malformed TOML should be an error, not silent defaults"
    );

    cleanup(&dir);
}

#[test]
fn partial_config_overrides_only_listed_bindings() {
    // Only override A on controller 1; everything else should be unbound
    // (the file is the source of truth — missing keys = unbound, not
    // "fall back to default").
    let text = r#"
[keys.controller1]
A = "Return"

[gamepad]
A = "X"
"#;
    let c = Config::from_toml(text).expect("parse");
    assert_eq!(c.keycode_for(0, button::A), Some(Keycode::Return));
    // B was not listed → unbound.
    assert_eq!(c.keycode_for(0, button::B), None);
    // Gamepad A was overridden to X.
    assert_eq!(c.gamepad_button_for(button::A), Some(Button::X));
    // Gamepad B was not listed → unbound.
    assert_eq!(c.gamepad_button_for(button::B), None);
    // Top-level fields default.
    assert_eq!(c.audio_volume, 1.0);
    assert_eq!(c.window_scale, 3);
}

#[test]
fn empty_config_toml_yields_all_unbound() {
    let c = Config::from_toml("").expect("empty parses");
    // No bindings → everything unbound.
    assert_eq!(c.keycode_for(0, button::A), None);
    assert_eq!(c.gamepad_button_for(button::A), None);
    // Top-level defaults still apply.
    assert_eq!(c.audio_volume, 1.0);
    assert_eq!(c.window_scale, 3);
}

#[test]
fn invalid_keycode_name_in_file_is_silently_unbound() {
    // A typo in the key name should not crash the emulator — the binding
    // is treated as unbound.
    let text = r#"
[keys.controller1]
A = "NotARealKey"
"#;
    let c = Config::from_toml(text).expect("parse");
    assert_eq!(c.keycode_for(0, button::A), None);
}

#[test]
fn invalid_gamepad_button_name_in_file_is_silently_unbound() {
    let text = r#"
[gamepad]
A = "NotARealButton"
"#;
    let c = Config::from_toml(text).expect("parse");
    assert_eq!(c.gamepad_button_for(button::A), None);
}

#[test]
fn save_creates_parent_directories() {
    let dir = temp_dir("nested");
    let nested = dir.join("a/b/c");
    let path = nested.join("config.toml");

    let c = Config::default();
    c.save_to_path(&path).expect("save with nested dirs");
    assert!(path.exists());

    cleanup(&dir);
}

#[test]
fn config_with_kb2_bindings_round_trips() {
    let dir = temp_dir("kb2");
    let path = dir.join("config.toml");

    let mut c = Config::default();
    let mut kb2 = BTreeMap::new();
    kb2.insert("A".to_string(), "U".to_string());
    kb2.insert("B".to_string(), "I".to_string());
    kb2.insert("Select".to_string(), "J".to_string());
    kb2.insert("Start".to_string(), "K".to_string());
    kb2.insert("Up".to_string(), "W".to_string());
    kb2.insert("Down".to_string(), "E".to_string());
    kb2.insert("Left".to_string(), "R".to_string());
    kb2.insert("Right".to_string(), "T".to_string());
    c.keys.controller2 = kb2;

    c.save_to_path(&path).expect("save");
    let loaded = Config::load_from_path(&path).expect("load");
    assert_eq!(loaded.keycode_for(1, button::A), Some(Keycode::U));
    assert_eq!(loaded.keycode_for(1, button::RIGHT), Some(Keycode::T));

    cleanup(&dir);
}

// ---------------------------------------------------------------------------
// InputMapper wiring
// ---------------------------------------------------------------------------

#[test]
fn mapper_from_default_config_binds_all_eight_kb1_buttons() {
    let m = InputMapper::default();
    assert_eq!(m.keyboard_binding_count(), 8);
    assert_eq!(m.gamepad_binding_count(), 8);
}

#[test]
fn mapper_respects_custom_kb1_binding() {
    let mut c = Config::default();
    c.keys
        .controller1
        .insert("A".to_string(), "Return".to_string());
    let mut m = InputMapper::from_config(&c);

    let mut j = Joypad::new();
    m.handle_key(&mut j, Keycode::Z, true); // Z is no longer bound
    assert_eq!(j.current(0), 0);
    m.handle_key(&mut j, Keycode::Return, true); // Return is now A
    assert_eq!(j.current(0), 1 << button::A);
}

#[test]
fn mapper_respects_kb2_binding() {
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
fn mapper_gamepad_routes_to_correct_controller() {
    let mut m = InputMapper::default();
    let mut j = Joypad::new();

    m.handle_gamepad_button(&mut j, 0, Button::A, true);
    assert_eq!(j.current(0), 1 << button::A);
    assert_eq!(j.current(1), 0);

    m.handle_gamepad_button(&mut j, 1, Button::A, true);
    assert_eq!(j.current(0), 1 << button::A);
    assert_eq!(j.current(1), 1 << button::A);
}

#[test]
fn mapper_gamepad_beyond_two_falls_back_to_controller1() {
    let mut m = InputMapper::default();
    let mut j = Joypad::new();
    m.handle_gamepad_button(&mut j, 5, Button::A, true);
    assert_eq!(j.current(0), 1 << button::A);
    assert_eq!(j.current(1), 0);
}

#[test]
fn mapper_keyboard_and_gamepad_simultaneous() {
    let mut m = InputMapper::default();
    let mut j = Joypad::new();

    // Press keyboard Z (NES A on controller 1).
    m.handle_key(&mut j, Keycode::Z, true);
    assert_eq!(j.current(0), 1 << button::A);

    // Press gamepad A (also NES A on controller 1) — button stays set.
    m.handle_gamepad_button(&mut j, 0, Button::A, true);
    assert_eq!(j.current(0), 1 << button::A);

    // Release keyboard — gamepad still holding.
    m.handle_key(&mut j, Keycode::Z, false);
    assert_eq!(j.current(0), 1 << button::A);

    // Release gamepad — now clear.
    m.handle_gamepad_button(&mut j, 0, Button::A, false);
    assert_eq!(j.current(0), 0);
}

#[test]
fn mapper_unbound_keyboard_key_is_noop() {
    let mut m = InputMapper::default();
    let mut j = Joypad::new();
    m.handle_key(&mut j, Keycode::F1, true);
    assert_eq!(j.current(0), 0);
    assert_eq!(j.current(1), 0);
}

#[test]
fn mapper_unbound_gamepad_button_is_noop() {
    let mut m = InputMapper::default();
    let mut j = Joypad::new();
    // RightShoulder is not in the default gamepad bindings.
    m.handle_gamepad_button(&mut j, 0, Button::RightShoulder, true);
    assert_eq!(j.current(0), 0);
}

#[test]
fn mapper_skips_invalid_binding_names() {
    let mut c = Config::default();
    // Typo — not a real Keycode.
    c.keys
        .controller1
        .insert("A".to_string(), "NotAKey".to_string());
    let m = InputMapper::from_config(&c);
    // A is now unbound; the other 7 default bindings remain.
    assert_eq!(m.keyboard_binding_count(), 7);
    assert_eq!(m.keyboard_binding(Keycode::Z), None);
}

#[test]
fn mapper_custom_gamepad_binding_respected() {
    let mut c = Config::default();
    c.gamepad
        .buttons
        .insert("A".to_string(), "RightShoulder".to_string());
    let mut m = InputMapper::from_config(&c);

    let mut j = Joypad::new();
    m.handle_gamepad_button(&mut j, 0, Button::A, true); // A no longer bound
    assert_eq!(j.current(0), 0);
    m.handle_gamepad_button(&mut j, 0, Button::RightShoulder, true);
    assert_eq!(j.current(0), 1 << button::A);
}

#[test]
fn mapper_loaded_from_disk_config_works() {
    // End-to-end: write a custom config to disk, load it, build a mapper,
    // and verify the custom binding is honored.
    let dir = temp_dir("mapper_disk");
    let path = dir.join("config.toml");

    let mut c = Config::default();
    c.keys
        .controller1
        .insert("A".to_string(), "Space".to_string());
    c.save_to_path(&path).expect("save");

    let loaded = Config::load_from_path(&path).expect("load");
    let mut m = InputMapper::from_config(&loaded);

    let mut j = Joypad::new();
    m.handle_key(&mut j, Keycode::Z, true); // Z no longer bound
    assert_eq!(j.current(0), 0);
    m.handle_key(&mut j, Keycode::Space, true); // Space is now A
    assert_eq!(j.current(0), 1 << button::A);

    cleanup(&dir);
}

// ---------------------------------------------------------------------------
// Struct-level construction (covers the public serde types)
// ---------------------------------------------------------------------------

#[test]
fn keybindings_default_is_empty() {
    let kb = KeyBindings::default();
    assert!(kb.controller1.is_empty());
    assert!(kb.controller2.is_empty());
}

#[test]
fn gamepadbindings_default_is_empty() {
    let gp = GamepadBindings::default();
    assert!(gp.buttons.is_empty());
}

#[test]
fn config_with_empty_keys_yields_all_unbound() {
    let c = Config {
        audio_volume: 0.7,
        window_scale: 2,
        keys: KeyBindings::default(),
        gamepad: GamepadBindings::default(),
        audio_channels: ChannelVolumes::default(),
        region: "auto".to_string(),
        recent_roms: Vec::new(),
    };
    for b in 0..8u8 {
        assert_eq!(c.keycode_for(0, b), None);
        assert_eq!(c.keycode_for(1, b), None);
        assert_eq!(c.gamepad_button_for(b), None);
    }
    assert_eq!(c.audio_volume, 0.7);
    assert_eq!(c.window_scale, 2);
}

// ---- M31: per-channel APU volumes -------------------------------------

#[test]
fn channel_volumes_default_all_one() {
    let cv = ChannelVolumes::default();
    assert_eq!(cv.pulse1, 1.0);
    assert_eq!(cv.pulse2, 1.0);
    assert_eq!(cv.triangle, 1.0);
    assert_eq!(cv.noise, 1.0);
    assert_eq!(cv.dmc, 1.0);
}

#[test]
fn channel_volumes_as_array_is_in_apu_index_order() {
    let cv = ChannelVolumes {
        pulse1: 0.1,
        pulse2: 0.2,
        triangle: 0.3,
        noise: 0.4,
        dmc: 0.5,
    };
    assert_eq!(cv.as_array(), [0.1, 0.2, 0.3, 0.4, 0.5]);
}

#[test]
fn config_default_has_full_volume_channels() {
    let c = Config::default();
    assert_eq!(c.audio_channels.as_array(), [1.0; 5]);
}

#[test]
fn channel_volumes_round_trip_through_toml() {
    let mut c = Config::default();
    c.audio_channels.pulse1 = 0.8;
    c.audio_channels.pulse2 = 0.6;
    c.audio_channels.triangle = 0.4;
    c.audio_channels.noise = 0.2;
    c.audio_channels.dmc = 0.0;
    let text = c.to_toml().expect("serialize");
    let parsed = Config::from_toml(&text).expect("parse");
    assert!((parsed.audio_channels.pulse1 - 0.8).abs() < 1e-6);
    assert!((parsed.audio_channels.pulse2 - 0.6).abs() < 1e-6);
    assert!((parsed.audio_channels.triangle - 0.4).abs() < 1e-6);
    assert!((parsed.audio_channels.noise - 0.2).abs() < 1e-6);
    assert_eq!(parsed.audio_channels.dmc, 0.0);
}

#[test]
fn partial_audio_channels_uses_defaults_for_missing_fields() {
    let text = r#"
[audio_channels]
pulse1 = 0.5
dmc = 0.25
"#;
    let c = Config::from_toml(text).expect("parse");
    assert!((c.audio_channels.pulse1 - 0.5).abs() < 1e-6);
    assert_eq!(c.audio_channels.pulse2, 1.0); // default
    assert_eq!(c.audio_channels.triangle, 1.0); // default
    assert_eq!(c.audio_channels.noise, 1.0); // default
    assert!((c.audio_channels.dmc - 0.25).abs() < 1e-6);
}

#[test]
fn missing_audio_channels_section_uses_all_defaults() {
    let text = "audio_volume = 0.5\n";
    let c = Config::from_toml(text).expect("parse");
    assert_eq!(c.audio_channels.as_array(), [1.0; 5]);
}
