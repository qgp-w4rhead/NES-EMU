//! User configuration — key bindings, gamepad bindings, audio volume, window scale.
//!
//! Persisted as `config.toml`; missing entries are treated as unbound.

use std::collections::BTreeMap;
use std::path::Path;

use sdl2::controller::Button;
use sdl2::keyboard::Keycode;
use serde::{Deserialize, Serialize};

/// NES button names in canonical order, indexed by the
/// [`crate::joypad::button`] bit indices (`A`=0 .. `Right`=7).
pub const NES_BUTTON_NAMES: [&str; 8] =
    ["A", "B", "Select", "Start", "Up", "Down", "Left", "Right"];

/// Default keyboard bindings for controller 1 — WASD for the D-pad,
/// `L`/`K` for the right-hand action buttons, and `H`/`G` for Select/Start.
const DEFAULT_KB1: [(&str, &str); 8] = [
    ("A", "L"),
    ("B", "K"),
    ("Select", "H"),
    ("Start", "G"),
    ("Up", "W"),
    ("Down", "S"),
    ("Left", "A"),
    ("Right", "D"),
];

/// Default gamepad bindings — Xbox-style layout where the face buttons map
/// directly (A→A, B→B), Back/Start map to Select/Start, and the D-pad maps
/// to the controller D-pad. The host-side names are the canonical SDL2
/// game-controller mapping strings (lowercase, as returned by
/// `Button::string()`): `a`, `b`, `back`, `start`, `dpup`, `dpdown`,
/// `dpleft`, `dpright`. `Button::from_string` is case-insensitive, so
/// users may also write `"A"` / `"B"` / `"Back"` / `"Start"` in
/// `config.toml`, but the D-pad names must be the SDL2 short forms
/// (`"dpup"`, not `"DPadUp"`).
const DEFAULT_GAMEPAD: [(&str, &str); 8] = [
    ("A", "a"),
    ("B", "b"),
    ("Select", "back"),
    ("Start", "start"),
    ("Up", "dpup"),
    ("Down", "dpdown"),
    ("Left", "dpleft"),
    ("Right", "dpright"),
];

/// Look up the NES button bit index for a canonical button name.
///
/// Returns `None` for unrecognized names. Comparison is case-sensitive and
/// matches the exact strings in [`NES_BUTTON_NAMES`].
pub fn button_index_from_name(name: &str) -> Option<u8> {
    NES_BUTTON_NAMES
        .iter()
        .position(|n| *n == name)
        .map(|i| i as u8)
}

/// Return the canonical NES button name for a button bit index, or `None`
/// if the index is out of range.
pub fn button_name(index: u8) -> Option<&'static str> {
    NES_BUTTON_NAMES.get(index as usize).copied()
}

/// Keyboard bindings for both NES controllers.
///
/// Each map is keyed by NES button name (e.g. `"A"`, `"Up"`) and valued by
/// SDL2 `Keycode` name (e.g. `"Z"`, `"Up"`). Missing or empty-string entries
/// are unbound.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct KeyBindings {
    /// Controller 1 (`$4016`) bindings.
    #[serde(default)]
    pub controller1: BTreeMap<String, String>,
    /// Controller 2 (`$4017`) bindings — empty by default.
    #[serde(default)]
    pub controller2: BTreeMap<String, String>,
}

/// Gamepad bindings — shared by all connected gamepads. Gamepad `N` maps to
/// NES controller `N` (clamped to 0 or 1).
///
/// The `buttons` field is flattened in TOML, so bindings go directly under
/// `[gamepad]` rather than `[gamepad.buttons]`:
///
/// ```toml
/// [gamepad]
/// A = "a"
/// B = "b"
/// ```
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct GamepadBindings {
    /// Map from NES button name to SDL2 `GameControllerButton` name.
    /// Flattened into the `[gamepad]` table in TOML.
    #[serde(default, flatten)]
    pub buttons: BTreeMap<String, String>,
}

/// Per-channel APU volume scalars (M31). Each value is in `[0.0, 1.0]`;
/// `0.0` silences that channel at the mix stage. Configured in
/// `config.toml` under `[audio_channels]`:
///
/// ```toml
/// [audio_channels]
/// pulse1 = 1.0
/// pulse2 = 1.0
/// triangle = 1.0
/// noise = 1.0
/// dmc = 1.0
/// ```
///
/// Missing fields default to `1.0` (full volume). At runtime the volumes
/// can be adjusted via `Alt+Up`/`Alt+Down` hotkeys (see
/// [`crate::audio_hotkeys::AudioHotkeys`]); `Alt+0` resets all channels
/// to unmuted with volume `1.0` (not to the config values).
///
/// See: https://www.nesdev.org/wiki/APU_Mixer
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ChannelVolumes {
    /// Pulse channel 1 (`$4000-$4003`) volume scalar.
    #[serde(default = "default_channel_volume")]
    pub pulse1: f32,
    /// Pulse channel 2 (`$4004-$4007`) volume scalar.
    #[serde(default = "default_channel_volume")]
    pub pulse2: f32,
    /// Triangle channel (`$4008-$400B`) volume scalar.
    #[serde(default = "default_channel_volume")]
    pub triangle: f32,
    /// Noise channel (`$400C-$400F`) volume scalar.
    #[serde(default = "default_channel_volume")]
    pub noise: f32,
    /// DMC channel (`$4010-$4013`) volume scalar.
    #[serde(default = "default_channel_volume")]
    pub dmc: f32,
}

fn default_channel_volume() -> f32 {
    1.0
}

impl Default for ChannelVolumes {
    fn default() -> Self {
        Self {
            pulse1: 1.0,
            pulse2: 1.0,
            triangle: 1.0,
            noise: 1.0,
            dmc: 1.0,
        }
    }
}

impl ChannelVolumes {
    /// Return the five volumes in APU channel-index order
    /// `[pulse1, pulse2, triangle, noise, dmc]` (matching
    /// [`crate::apu::Apu::apply_channel_volumes`]).
    pub fn as_array(&self) -> [f32; 5] {
        [
            self.pulse1,
            self.pulse2,
            self.triangle,
            self.noise,
            self.dmc,
        ]
    }
}

/// Debug / diagnostics settings, configured under `[debug]` in `config.toml`.
///
/// All flags default to `false` (accurate emulation). These are intended for
/// testing and visual regression purposes — they intentionally introduce
/// known bugs so their visual effects can be reproduced deterministically.
///
/// ```toml
/// [debug]
/// slant_corruption = false
/// ring_buffer_trace = false
/// use_inaccurate_palette = false
/// nmi_retrigger = false
/// ```
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct DebugConfig {
    /// Reproduce the fine-X scroll corruption bug (fixed). When `true`,
    /// `increment_h_scroll` mutates the PPUSCROLL `fine_x` register during
    /// rendering (incrementing it every 8 cycles and wrapping into coarse X)
    /// instead of incrementing coarse X directly. This causes each scanline
    /// to start with a progressively shifted fine X offset, producing the
    /// characteristic 45-degree slant on all rendered pixels.
    #[serde(default)]
    pub slant_corruption: bool,
    /// Enable ring buffer trace mode. When `true`, the emulator keeps the
    /// last 10,000 CPU instructions in memory instead of writing every
    /// instruction to `trace.log`. When the user pauses (F2), the buffer
    /// is flushed to `trace.log`, giving a focused window around the
    /// moment of interest rather than millions of lines from boot.
    #[serde(default)]
    pub ring_buffer_trace: bool,
    /// Reproduce the inaccurate NES palette bug (fixed). When `true`,
    /// the old incorrect RGB values are used for NTSC color conversion,
    /// producing wrong colors such as yellow pipes in Mario Bros.
    #[serde(default)]
    pub use_inaccurate_palette: bool,
    /// Reproduce the NMI retrigger bug (fixed). When `true`, every
    /// PPUCTRL write with bit 7 set during VBlank triggers an NMI
    /// (the old buggy behavior), instead of only the rising edge 0→1.
    /// This causes spurious extra NMIs that waste VBlank time on
    /// redundant OAM DMAs, forcing VRAM writes to spill into visible
    /// scanlines where scroll increments corrupt the target addresses.
    #[serde(default)]
    pub nmi_retrigger: bool,
}

/// Top-level user configuration.
///
/// All fields default to sensible values; a missing `config.toml` yields
/// [`Config::default`] which is immediately usable.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    /// Master audio volume, `0.0` (muted) to `1.0` (full). Values outside
    /// this range are clamped at apply time.
    #[serde(default = "default_audio_volume")]
    pub audio_volume: f32,

    /// Integer window scale factor (`1`, `2`, `3`, ...). The native NES
    /// resolution is 256x240; the window is `256*scale` by `240*scale`.
    #[serde(default = "default_window_scale")]
    pub window_scale: u32,

    /// Keyboard bindings for controllers 1 and 2.
    #[serde(default)]
    pub keys: KeyBindings,

    /// Gamepad bindings (applied to every connected gamepad).
    #[serde(default)]
    pub gamepad: GamepadBindings,

    /// Per-channel APU volume scalars (M31). Configured under
    /// `[audio_channels]` in `config.toml`. Defaults to full volume
    /// (`1.0`) on all five channels.
    #[serde(default)]
    pub audio_channels: ChannelVolumes,

    /// TV system / region override (M32). Accepts `"auto"`, `"ntsc"`,
    /// `"pal"`, or `"dendy"` (case-insensitive). `"auto"` (the default)
    /// auto-detects from the iNES header's TV-system hint (byte 9),
    /// falling back to NTSC when no hint is present.
    #[serde(default = "default_region")]
    pub region: String,

    /// Recent ROM file paths (M34), most-recent-first. Capped at 10
    /// entries by the ROM manager; persisted to `config.toml` as a TOML
    /// array of strings. Updated by the main loop after each successful
    /// ROM load (initial `--rom`, drag-and-drop, or recent-list
    /// selection).
    #[serde(default)]
    pub recent_roms: Vec<String>,

    /// Debug / diagnostics settings (under `[debug]` in `config.toml`).
    /// Defaults to all-off so emulation is accurate unless explicitly
    /// configured otherwise.
    #[serde(default)]
    pub debug: DebugConfig,

    /// Rewind buffer capacity in snapshots (under `[rewind]` in
    /// `config.toml`). Each snapshot is ~13–30 KB; with
    /// `REWIND_INTERVAL = 2`, `capacity` snapshots cover
    /// `capacity * 2 / 60` seconds of NTSC gameplay. Default is 300
    /// (~10 s). Maximum is 60000 (~33 min). Clamped at load time.
    #[serde(default = "default_rewind_capacity")]
    pub rewind_capacity: usize,

    /// Turbo speed multiplier for the Spacebar hold-to-accelerate
    /// feature. Accepts `0.25`, `0.5`, `2.0`, `3.0`, or `4.0`.
    /// Default is `2.0` (double speed).
    #[serde(default = "default_turbo_speed")]
    pub turbo_speed: f32,
}

fn default_region() -> String {
    "auto".to_string()
}

fn default_audio_volume() -> f32 {
    1.0
}

fn default_window_scale() -> u32 {
    3
}

fn default_rewind_capacity() -> usize {
    crate::save_state::DEFAULT_REWIND_CAPACITY
}

fn default_turbo_speed() -> f32 {
    crate::ui_hotkeys::TURBO_SPEEDS[crate::ui_hotkeys::DEFAULT_TURBO_SPEED_INDEX]
}

impl Default for Config {
    fn default() -> Self {
        let mut keys = KeyBindings::default();
        for (btn, key) in DEFAULT_KB1 {
            keys.controller1.insert(btn.to_string(), key.to_string());
        }
        // Controller 2 starts empty — add bindings in config.toml.

        let mut gamepad = GamepadBindings::default();
        for (btn, gp) in DEFAULT_GAMEPAD {
            gamepad.buttons.insert(btn.to_string(), gp.to_string());
        }

        Self {
            audio_volume: default_audio_volume(),
            window_scale: default_window_scale(),
            keys,
            gamepad,
            audio_channels: ChannelVolumes::default(),
            region: default_region(),
            recent_roms: Vec::new(),
            debug: DebugConfig::default(),
            rewind_capacity: default_rewind_capacity(),
            turbo_speed: default_turbo_speed(),
        }
    }
}

/// Error returned by config load/save operations.
#[derive(Debug)]
pub enum ConfigError {
    /// The TOML file could not be read or parsed.
    Parse(String),
    /// The file could not be written.
    Write(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Parse(m) => write!(f, "config parse error: {m}"),
            ConfigError::Write(m) => write!(f, "config write error: {m}"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Load a `Config` from a TOML file at `path`.
    ///
    /// A **missing** file is not an error — it yields `Config::default()`,
    /// so the emulator can boot on first run before any config exists. A
    /// present-but-unparseable file is an error (the user should know their
    /// config is malformed rather than silently getting defaults).
    pub fn load_from_path(path: &Path) -> Result<Self, ConfigError> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(ConfigError::Parse(format!("read {path:?}: {e}"))),
        };
        Self::from_toml(&text)
    }

    /// Parse a `Config` from a TOML string.
    pub fn from_toml(text: &str) -> Result<Self, ConfigError> {
        toml::from_str(text).map_err(|e| ConfigError::Parse(e.to_string()))
    }

    /// Serialize the config to a pretty-printed TOML string.
    pub fn to_toml(&self) -> Result<String, ConfigError> {
        toml::to_string_pretty(self).map_err(|e| ConfigError::Write(e.to_string()))
    }

    /// Save the config to a TOML file at `path`. Creates parent directories
    /// if needed.
    pub fn save_to_path(&self, path: &Path) -> Result<(), ConfigError> {
        let text = self.to_toml()?;
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| ConfigError::Write(format!("mkdir {parent:?}: {e}")))?;
            }
        }
        std::fs::write(path, text).map_err(|e| ConfigError::Write(format!("write {path:?}: {e}")))
    }

    /// If no config file exists at `path`, write the default config there
    /// so the user has a template to edit. Returns `true` if a file was
    /// created, `false` if one already existed. Errors are surfaced to the
    /// caller (e.g. permission denied) — the emulator should still boot
    /// with in-memory defaults in that case.
    pub fn ensure_default_file(path: &Path) -> Result<bool, ConfigError> {
        if path.exists() {
            return Ok(false);
        }
        Self::default().save_to_path(path)?;
        Ok(true)
    }

    /// Look up the SDL2 `Keycode` bound to a NES button on a given
    /// controller (`0` or `1`). Returns `None` if the controller has no
    /// binding for that button, or if the bound name is not a valid
    /// `Keycode` (silently ignored — protects the emulator from typos in
    /// `config.toml`).
    pub fn keycode_for(&self, controller: usize, btn: u8) -> Option<Keycode> {
        let map = match controller {
            0 => &self.keys.controller1,
            1 => &self.keys.controller2,
            _ => return None,
        };
        let name = button_name(btn)?;
        let key_name = map.get(name)?;
        if key_name.is_empty() {
            return None;
        }
        Keycode::from_name(key_name)
    }

    /// Look up the SDL2 `GameControllerButton` bound to a NES button.
    /// Returns `None` if unbound or if the name is not a valid button.
    pub fn gamepad_button_for(&self, btn: u8) -> Option<Button> {
        let name = button_name(btn)?;
        let btn_name = self.gamepad.buttons.get(name)?;
        if btn_name.is_empty() {
            return None;
        }
        Button::from_string(btn_name)
    }

    /// Resolve the configured region (M32). If `config.region` is
    /// `"auto"` (the default), the `hint` (typically the cartridge's
    /// [`crate::cartridge::InesHeader::region_hint`]) is used; if the
    /// hint is `None`, NTSC is the fallback. If `config.region` is a
    /// concrete region name (`"ntsc"` / `"pal"` / `"dendy"`), that
    /// region is used and the hint is ignored. An invalid region string
    /// falls back to NTSC (with the hint still honoured as a secondary
    /// fallback).
    pub fn resolve_region(&self, hint: Option<crate::region::Region>) -> crate::region::Region {
        match crate::region::Region::from_config_str(&self.region) {
            Ok(Some(r)) => r,
            Ok(None) => hint.unwrap_or(crate::region::Region::Ntsc),
            Err(_) => {
                // Invalid config string — fall back to hint, then NTSC.
                // The caller should have warned about the invalid string
                // at load time; here we just pick a sane default.
                hint.unwrap_or(crate::region::Region::Ntsc)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::joypad::button;

    /// Build a `BTreeMap` from literal `(name, value)` pairs.
    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        for (k, v) in pairs {
            m.insert((*k).to_string(), (*v).to_string());
        }
        m
    }

    #[test]
    fn default_config_has_standard_kb1_bindings() {
        let c = Config::default();
        assert_eq!(
            c.keycode_for(0, button::A),
            Some(Keycode::L),
            "default A should be L"
        );
        assert_eq!(c.keycode_for(0, button::B), Some(Keycode::K));
        assert_eq!(c.keycode_for(0, button::SELECT), Some(Keycode::H));
        assert_eq!(c.keycode_for(0, button::START), Some(Keycode::G));
        assert_eq!(c.keycode_for(0, button::UP), Some(Keycode::W));
        assert_eq!(c.keycode_for(0, button::DOWN), Some(Keycode::S));
        assert_eq!(c.keycode_for(0, button::LEFT), Some(Keycode::A));
        assert_eq!(c.keycode_for(0, button::RIGHT), Some(Keycode::D));
    }

    #[test]
    fn default_config_has_empty_kb2() {
        let c = Config::default();
        for b in 0..8u8 {
            assert_eq!(
                c.keycode_for(1, b),
                None,
                "controller 2 button {b} should be unbound by default"
            );
        }
    }

    #[test]
    fn default_config_has_standard_gamepad_bindings() {
        let c = Config::default();
        assert_eq!(c.gamepad_button_for(button::A), Some(Button::A));
        assert_eq!(c.gamepad_button_for(button::B), Some(Button::B));
        assert_eq!(c.gamepad_button_for(button::SELECT), Some(Button::Back));
        assert_eq!(c.gamepad_button_for(button::START), Some(Button::Start));
        assert_eq!(c.gamepad_button_for(button::UP), Some(Button::DPadUp));
        assert_eq!(c.gamepad_button_for(button::DOWN), Some(Button::DPadDown));
        assert_eq!(c.gamepad_button_for(button::LEFT), Some(Button::DPadLeft));
        assert_eq!(c.gamepad_button_for(button::RIGHT), Some(Button::DPadRight));
    }

    #[test]
    fn default_audio_volume_and_window_scale() {
        let c = Config::default();
        assert_eq!(c.audio_volume, 1.0);
        assert_eq!(c.window_scale, 3);
    }

    #[test]
    fn button_index_round_trip() {
        for (i, &name) in NES_BUTTON_NAMES.iter().enumerate() {
            assert_eq!(button_index_from_name(name), Some(i as u8));
            assert_eq!(button_name(i as u8), Some(name));
        }
    }

    #[test]
    fn button_index_unknown_name_returns_none() {
        assert_eq!(button_index_from_name("Foo"), None);
        assert_eq!(button_index_from_name(""), None);
        assert_eq!(button_index_from_name("a"), None); // case-sensitive
    }

    #[test]
    fn button_name_out_of_range_returns_none() {
        assert_eq!(button_name(8), None);
        assert_eq!(button_name(255), None);
    }

    #[test]
    fn keycode_for_invalid_controller_returns_none() {
        let c = Config::default();
        assert_eq!(c.keycode_for(2, button::A), None);
        assert_eq!(c.keycode_for(99, button::A), None);
    }

    #[test]
    fn keycode_for_invalid_button_returns_none() {
        let c = Config::default();
        assert_eq!(c.keycode_for(0, 8), None);
    }

    #[test]
    fn empty_string_binding_is_unbound() {
        let mut c = Config::default();
        c.keys.controller1.insert("A".to_string(), "".to_string());
        assert_eq!(c.keycode_for(0, button::A), None);
    }

    #[test]
    fn invalid_keycode_name_is_unbound() {
        let mut c = Config::default();
        c.keys
            .controller1
            .insert("A".to_string(), "NotARealKey".to_string());
        // Invalid name → None (silently ignored, no panic).
        assert_eq!(c.keycode_for(0, button::A), None);
    }

    #[test]
    fn invalid_gamepad_button_name_is_unbound() {
        let mut c = Config::default();
        c.gamepad
            .buttons
            .insert("A".to_string(), "NotARealButton".to_string());
        assert_eq!(c.gamepad_button_for(button::A), None);
    }

    #[test]
    fn toml_round_trip_preserves_bindings() {
        let c = Config::default();
        let text = c.to_toml().expect("serialize");
        let parsed = Config::from_toml(&text).expect("parse");
        assert_eq!(parsed.keycode_for(0, button::A), Some(Keycode::L));
        assert_eq!(parsed.keycode_for(0, button::START), Some(Keycode::G));
        assert_eq!(parsed.gamepad_button_for(button::A), Some(Button::A));
        assert_eq!(parsed.gamepad_button_for(button::UP), Some(Button::DPadUp));
        assert_eq!(parsed.audio_volume, 1.0);
        assert_eq!(parsed.window_scale, 3);
    }

    #[test]
    fn toml_round_trip_preserves_custom_bindings() {
        let mut c = Config::default();
        // Remap A to Return, B to Space on controller 1.
        c.keys
            .controller1
            .insert("A".to_string(), "Return".to_string());
        c.keys
            .controller1
            .insert("B".to_string(), "Space".to_string());
        // Add a controller 2 binding.
        c.keys.controller2.insert("A".to_string(), "Q".to_string());
        // Remap gamepad Select to LeftShoulder.
        c.gamepad
            .buttons
            .insert("Select".to_string(), "LeftShoulder".to_string());

        let text = c.to_toml().expect("serialize");
        let parsed = Config::from_toml(&text).expect("parse");
        assert_eq!(parsed.keycode_for(0, button::A), Some(Keycode::Return));
        assert_eq!(parsed.keycode_for(0, button::B), Some(Keycode::Space));
        assert_eq!(parsed.keycode_for(1, button::A), Some(Keycode::Q));
        assert_eq!(
            parsed.gamepad_button_for(button::SELECT),
            Some(Button::LeftShoulder)
        );
        // Untouched bindings remain.
        assert_eq!(parsed.keycode_for(0, button::START), Some(Keycode::G));
    }

    #[test]
    fn partial_config_uses_defaults_for_missing_top_level_fields() {
        // Only specify keys.controller1.A — everything else should default.
        let text = r#"
[keys.controller1]
A = "Return"
"#;
        let c = Config::from_toml(text).expect("parse");
        assert_eq!(c.keycode_for(0, button::A), Some(Keycode::Return));
        // Missing top-level fields default.
        assert_eq!(c.audio_volume, 1.0);
        assert_eq!(c.window_scale, 3);
        // Missing keys.controller1.B → unbound (not defaulted — partial map).
        assert_eq!(c.keycode_for(0, button::B), None);
        // Missing gamepad section → empty map → all unbound.
        assert_eq!(c.gamepad_button_for(button::A), None);
    }

    #[test]
    fn empty_toml_uses_all_defaults() {
        let c = Config::from_toml("").expect("parse empty");
        assert_eq!(c.audio_volume, 1.0);
        assert_eq!(c.window_scale, 3);
        // Empty keys/gamepad maps → everything unbound.
        assert_eq!(c.keycode_for(0, button::A), None);
        assert_eq!(c.gamepad_button_for(button::A), None);
    }

    #[test]
    fn malformed_toml_is_an_error() {
        let text = "this is not = valid = toml =";
        assert!(Config::from_toml(text).is_err());
    }

    #[test]
    fn load_missing_file_returns_default() {
        let path = std::env::temp_dir().join(format!(
            "nes-emu-config-nonexistent-{}.toml",
            std::process::id()
        ));
        // Make sure it really doesn't exist.
        let _ = std::fs::remove_file(&path);
        let c = Config::load_from_path(&path).expect("missing file is not an error");
        assert_eq!(c.keycode_for(0, button::A), Some(Keycode::L));
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = std::env::temp_dir().join(format!("nes-emu-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let _ = std::fs::remove_file(&path);

        let mut c = Config::default();
        c.keys
            .controller1
            .insert("A".to_string(), "Return".to_string());
        c.save_to_path(&path).expect("save");
        assert!(path.exists());

        let loaded = Config::load_from_path(&path).expect("load");
        assert_eq!(loaded.keycode_for(0, button::A), Some(Keycode::Return));
        assert_eq!(loaded.keycode_for(0, button::B), Some(Keycode::K));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn ensure_default_file_creates_then_is_idempotent() {
        let dir = std::env::temp_dir().join(format!("nes-emu-ensure-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let _ = std::fs::remove_file(&path);

        // First call creates the file.
        let created = Config::ensure_default_file(&path).expect("ensure first");
        assert!(created, "first call should create the file");
        assert!(path.exists());

        // Second call is a no-op.
        let created2 = Config::ensure_default_file(&path).expect("ensure second");
        assert!(!created2, "second call should not recreate");

        // The written file parses back to defaults.
        let loaded = Config::load_from_path(&path).expect("load");
        assert_eq!(loaded.keycode_for(0, button::A), Some(Keycode::L));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn custom_kb2_bindings_round_trip() {
        let mut c = Config::default();
        c.keys.controller2 = map(&[
            ("A", "U"),
            ("B", "I"),
            ("Select", "J"),
            ("Start", "K"),
            ("Up", "W"),
            ("Down", "X"), // note: conflicts with kb1 B but that's fine — different controller
            ("Left", "A"), // note: conflicts with kb1 Select but different controller
            ("Right", "D"),
        ]);
        let text = c.to_toml().expect("serialize");
        let parsed = Config::from_toml(&text).expect("parse");
        assert_eq!(parsed.keycode_for(1, button::A), Some(Keycode::U));
        assert_eq!(parsed.keycode_for(1, button::RIGHT), Some(Keycode::D));
    }

    #[test]
    fn debug_config_defaults_to_off() {
        let c = Config::default();
        assert!(
            !c.debug.slant_corruption,
            "slant_corruption defaults to false"
        );
    }

    #[test]
    fn debug_slant_corruption_round_trips_through_toml() {
        let mut c = Config::default();
        c.debug.slant_corruption = true;
        let text = c.to_toml().expect("serialize");
        let parsed = Config::from_toml(&text).expect("parse");
        assert!(parsed.debug.slant_corruption, "slant_corruption preserved");
    }

    #[test]
    fn debug_slant_corruption_parsed_from_toml() {
        let text = r#"
[debug]
slant_corruption = true
"#;
        let c = Config::from_toml(text).expect("parse");
        assert!(c.debug.slant_corruption);
    }
}
