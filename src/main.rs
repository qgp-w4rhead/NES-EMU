//! NES emulator entry point.
//!
//! Milestone 12 scope: load an iNES ROM from `--rom <path>`, build the
//! `EmulatorState`, and run a frame-locked main loop that steps the
//! emulator one NTSC frame per vsync and presents the PPU framebuffer via
//! SDL2.
//!
//! Milestone 22 added: configurable key bindings + gamepad support. The
//! bindings are loaded from `config.toml` (in the CWD by default, or
//! `--config <path>`). On first run, a default `config.toml` is written so
//! the user has a template to edit. Both keyboard and gamepad events are
//! routed through `InputMapper` and applied to the joypad simultaneously.
//!
//! See: https://www.nesdev.org/wiki/PPU — native NES resolution is 256x240.
//! See: https://www.nesdev.org/wiki/Cycle_reference — ~29,830 CPU cycles
//! per NTSC frame at 60.0988 Hz.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use sdl2::controller::GameController;
use sdl2::event::Event;
use sdl2::keyboard::Keycode;

use nes_emu::audio::AudioOutput;
use nes_emu::battery;
use nes_emu::cartridge::Cartridge;
use nes_emu::config::Config;
use nes_emu::emulator::EmulatorState;
use nes_emu::input::InputMapper;
use nes_emu::video::Video;

/// Application entry point. Returns a process exit code so that SDL2 or
/// initialization errors are reported cleanly without panicking in the
/// hot path (per tech-stack.md: "No panics in hot paths").
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("nes-emu: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Print usage information to stderr.
fn print_usage() {
    eprintln!("usage: nes-emu --rom <path-to-.nes> [--config <path-to-config.toml>]");
}

/// Parsed command-line arguments.
struct CliArgs {
    rom_path: String,
    config_path: Option<String>,
}

/// Parse `--rom <path>` and optional `--config <path>` from the argument
/// list. Returns the parsed args or an error message.
fn parse_args(args: &[String]) -> Result<CliArgs, String> {
    let mut rom_path: Option<String> = None;
    let mut config_path: Option<String> = None;
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        if arg == "--rom" {
            rom_path = Some(
                iter.next()
                    .ok_or_else(|| "--rom requires a path argument".to_string())?
                    .clone(),
            );
        } else if let Some(rest) = arg.strip_prefix("--rom=") {
            rom_path = Some(rest.to_string());
        } else if arg == "--config" {
            config_path = Some(
                iter.next()
                    .ok_or_else(|| "--config requires a path argument".to_string())?
                    .clone(),
            );
        } else if let Some(rest) = arg.strip_prefix("--config=") {
            config_path = Some(rest.to_string());
        }
    }
    let rom_path = rom_path.ok_or_else(|| {
        print_usage();
        "missing --rom <path>".to_string()
    })?;
    Ok(CliArgs {
        rom_path,
        config_path,
    })
}

/// Resolve the config file path: explicit `--config` wins, otherwise
/// default to `./config.toml` in the current working directory.
fn resolve_config_path(explicit: Option<&str>) -> PathBuf {
    match explicit {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from("config.toml"),
    }
}

/// Initialize SDL2, load the ROM, build the emulator, and drive the
/// frame-locked main loop until the user requests exit (ESC, window-close,
/// or Q on the window).
fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let cli = parse_args(&args)?;
    let rom_path = cli.rom_path;
    let config_path = resolve_config_path(cli.config_path.as_deref());

    // Load user config (M22). A missing file is not an error — we fall
    // back to defaults and write a template `config.toml` so the user has
    // something to edit. Parse errors are reported but non-fatal: the
    // emulator boots with defaults so a malformed config never blocks the
    // user from playing.
    let config = match Config::load_from_path(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("nes-emu: warning: {e}; using default config");
            Config::default()
        }
    };
    // First-run: write a default config.toml template if none exists.
    if let Err(e) = Config::ensure_default_file(&config_path) {
        eprintln!("nes-emu: warning: could not write default config: {e}");
    }

    let cartridge = Cartridge::from_path(&rom_path)
        .map_err(|e| format!("failed to load ROM '{rom_path}': {e}"))?;

    // Battery-backed PRG-RAM persistence (M21): if the cartridge advertises
    // battery-backed SRAM and a `.nessram` sidecar file exists next to the
    // ROM, load its contents into the cartridge's PRG-RAM before booting.
    // A missing sidecar is not an error — the game simply starts with
    // zeroed PRG-RAM (first run). Load errors are reported but non-fatal:
    // the game still boots with empty SRAM so the user is not blocked.
    let rom_path_ref = std::path::Path::new(&rom_path);
    let mut cartridge = cartridge;
    if cartridge.has_battery() {
        match battery::load_for_rom(rom_path_ref) {
            Ok(Some(data)) => cartridge.load_battery_sram(&data),
            Ok(None) => {}
            Err(e) => eprintln!("nes-emu: warning: could not read battery SRAM: {e}"),
        }
    }

    let mut emulator = EmulatorState::new(cartridge);
    emulator.reset();

    let sdl_context = sdl2::init()?;
    let video_subsystem = sdl_context.video()?;
    let audio_subsystem = sdl_context.audio()?;
    let game_controller_subsystem = sdl_context.game_controller()?;

    // Window scale from config (M22). Falls back to the video module's
    // default if the config value is zero or out of a sane range.
    let scale = if (1..=8).contains(&config.window_scale) {
        config.window_scale
    } else {
        nes_emu::video::DEFAULT_SCALE
    };
    let mut video = Video::new(&video_subsystem, scale)?;
    let mut audio = AudioOutput::new(&audio_subsystem)?;
    // Apply master volume from config (M22). Clamped to [0.0, 1.0] inside
    // `set_volume`.
    audio.set_volume(config.audio_volume);
    let mut event_pump = sdl_context.event_pump()?;

    // Open all connected SDL2 game controllers (M22). Each open controller
    // is held in `gamepads` for the lifetime of the loop so SDL2 keeps
    // reporting its events. We also build `gamepad_index_map`, which maps
    // SDL2 joystick **instance IDs** (the `which` field of
    // `ControllerButtonDown`/`Up` events) to a sequential NES controller
    // index (0, 1, ...). This is necessary because instance IDs are
    // monotonically increasing and not reused — after a disconnect/reconnect
    // a gamepad gets a higher ID, so we cannot pass `which` directly as a
    // controller index. See: https://wiki.libsdl.org/SDL_GameControllerOpen
    let mut gamepads: Vec<GameController> = Vec::new();
    let mut gamepad_index_map: HashMap<u32, usize> = HashMap::new();
    let num_joysticks = game_controller_subsystem.num_joysticks().unwrap_or(0);
    let mut next_seq: usize = 0;
    for i in 0..num_joysticks {
        if game_controller_subsystem.is_game_controller(i) {
            match game_controller_subsystem.open(i) {
                Ok(c) => {
                    let instance_id = c.instance_id();
                    let seq = next_seq;
                    next_seq += 1;
                    eprintln!(
                        "nes-emu: gamepad {} (instance {}) connected: {}",
                        seq,
                        instance_id,
                        c.name()
                    );
                    gamepad_index_map.insert(instance_id, seq);
                    gamepads.push(c);
                }
                Err(e) => {
                    eprintln!("nes-emu: warning: could not open gamepad {i}: {e}");
                }
            }
        }
    }
    if gamepads.is_empty() {
        eprintln!("nes-emu: no gamepads detected; keyboard only");
    }

    let mut mapper = InputMapper::from_config(&config);

    'running: loop {
        // Drain all pending events each frame; ESC, window-close, and the
        // conventional 'Q' key all terminate the loop. NES controller
        // buttons are wired to the joypad via `InputMapper` (M22). Both
        // keyboard and gamepad events are routed through the same mapper
        // so they work simultaneously.
        for event in event_pump.poll_iter() {
            match event {
                Event::Quit { .. } => break 'running,
                Event::KeyDown {
                    keycode: Some(Keycode::Escape),
                    ..
                }
                | Event::KeyDown {
                    keycode: Some(Keycode::Q),
                    ..
                } => break 'running,
                Event::KeyDown {
                    keycode: Some(k), ..
                } => mapper.handle_key(emulator.bus_mut().joypad_mut(), k, true),
                Event::KeyUp {
                    keycode: Some(k), ..
                } => mapper.handle_key(emulator.bus_mut().joypad_mut(), k, false),
                Event::ControllerButtonDown { which, button, .. } => {
                    if let Some(&seq) = gamepad_index_map.get(&which) {
                        mapper.handle_gamepad_button(
                            emulator.bus_mut().joypad_mut(),
                            seq,
                            button,
                            true,
                        );
                    }
                }
                Event::ControllerButtonUp { which, button, .. } => {
                    if let Some(&seq) = gamepad_index_map.get(&which) {
                        mapper.handle_gamepad_button(
                            emulator.bus_mut().joypad_mut(),
                            seq,
                            button,
                            false,
                        );
                    }
                }
                // Gamepad hot-plug: open the newly added controller and
                // assign it the next sequential NES controller index. The
                // `which` field here is the joystick **device index** (not
                // instance ID) — that's what `GameControllerSubsystem::open`
                // expects. After opening, we record the mapping from the
                // new controller's instance ID to its sequential index.
                Event::ControllerDeviceAdded { which, .. } => {
                    match game_controller_subsystem.open(which) {
                        Ok(c) => {
                            let instance_id = c.instance_id();
                            let seq = next_seq;
                            next_seq += 1;
                            eprintln!(
                                "nes-emu: gamepad {} (instance {}) hot-plugged: {}",
                                seq,
                                instance_id,
                                c.name()
                            );
                            gamepad_index_map.insert(instance_id, seq);
                            gamepads.push(c);
                        }
                        Err(e) => {
                            eprintln!(
                                "nes-emu: warning: could not open hot-plugged gamepad {which}: {e}"
                            );
                        }
                    }
                }
                // Gamepad hot-unplug: drop it from the index map. The
                // `GameController` itself stays in `gamepads` (dropping it
                // would close the device); SDL2 simply stops sending events
                // for it. We keep the sequential slot reserved so a later
                // hot-plug doesn't renumber existing controllers.
                Event::ControllerDeviceRemoved { which, .. } => {
                    if gamepad_index_map.remove(&which).is_some() {
                        eprintln!("nes-emu: gamepad (instance {which}) removed");
                    }
                }
                _ => {}
            }
        }

        // Step one full NTSC frame (CPU + PPU + APU in lockstep), then
        // present the rendered framebuffer and queue the audio samples.
        // SDL2's vsynced renderer paces the loop to the monitor refresh
        // rate (~60 Hz).
        emulator.step_frame();
        video.present(emulator.framebuffer())?;
        let samples = emulator.take_audio_samples();
        if !samples.is_empty() {
            // Audio output is best-effort: if the queue is full (e.g. the
            // audio device is slow), drop the samples rather than blocking
            // the emulation loop.
            let _ = audio.push_samples(&samples);
        }
    }

    // Battery-backed PRG-RAM persistence (M21): on exit, dump the
    // cartridge's PRG-RAM to the `.nessram` sidecar file next to the ROM so
    // the game's save data survives. Save errors are reported but non-fatal
    // — the emulator is already shutting down.
    if emulator.has_battery() {
        if let Some(sram) = emulator.battery_sram() {
            if let Err(e) = battery::save_for_rom(rom_path_ref, &sram) {
                eprintln!("nes-emu: warning: could not save battery SRAM: {e}");
            }
        }
    }

    Ok(())
}
