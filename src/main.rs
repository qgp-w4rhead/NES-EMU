//! NES emulator entry point.
//!
//! Loads an iNES ROM from `--rom <path>`, builds the `EmulatorState`, and
//! runs a frame-locked main loop that steps the emulator one NTSC frame
//! per vsync and presents the PPU framebuffer via SDL2. Key bindings and
//! gamepad mappings come from `config.toml` (M22). Debug hotkeys (M27/M28),
//! UI controls (M29), save-state/OSD (M30), and per-channel audio
//! volume/mute (M31) are dispatched before joypad routing.
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
use nes_emu::audio_hotkeys::AudioHotkeys;
use nes_emu::battery;
use nes_emu::cartridge::Cartridge;
use nes_emu::config::Config;
use nes_emu::debug::{handle_debugger_key, print_debug_overlay, CpuDebugger, DebugHotkeys};
use nes_emu::emulator::EmulatorState;
use nes_emu::input::InputMapper;
use nes_emu::osd::game_name_from_path;
use nes_emu::save_state_hotkeys::SaveStateHotkeys;
use nes_emu::ui_hotkeys::UiHotkeys;
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

/// Open all connected SDL2 game controllers at startup, returning the
/// held controllers and a map from SDL2 instance ID → NES controller
/// index. See: https://wiki.libsdl.org/SDL_GameControllerOpen
fn open_initial_gamepads(
    game_controller_subsystem: &sdl2::GameControllerSubsystem,
) -> (Vec<GameController>, HashMap<u32, usize>) {
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
    (gamepads, gamepad_index_map)
}

/// Initialize SDL2, load the ROM, build the emulator, and drive the
/// frame-locked main loop until the user requests exit (ESC, window-close,
/// or Q on the window).
fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let cli = parse_args(&args)?;
    let rom_path = cli.rom_path;
    let config_path = resolve_config_path(cli.config_path.as_deref());

    // Load user config (M22). Missing/invalid config is non-fatal: we
    // fall back to defaults and write a template `config.toml`.
    let config = match Config::load_from_path(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("nes-emu: warning: {e}; using default config");
            Config::default()
        }
    };
    if let Err(e) = Config::ensure_default_file(&config_path) {
        eprintln!("nes-emu: warning: could not write default config: {e}");
    }

    let cartridge = Cartridge::from_path(&rom_path)
        .map_err(|e| format!("failed to load ROM '{rom_path}': {e}"))?;

    // Battery-backed PRG-RAM persistence (M21): load `.nessram` sidecar
    // if present. Missing file / errors are non-fatal.
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
    // M31: apply per-channel APU volumes from config.
    emulator
        .bus_mut()
        .apu_mut()
        .apply_channel_volumes(&config.audio_channels.as_array());

    let sdl_context = sdl2::init()?;
    let video_subsystem = sdl_context.video()?;
    let audio_subsystem = sdl_context.audio()?;
    let game_controller_subsystem = sdl_context.game_controller()?;

    // Window scale from config (M22); falls back to default if invalid.
    let scale = if (1..=8).contains(&config.window_scale) {
        config.window_scale
    } else {
        nes_emu::video::DEFAULT_SCALE
    };
    let mut video = Video::new(&video_subsystem, scale)?;
    let mut audio = AudioOutput::new(&audio_subsystem)?;
    audio.set_volume(config.audio_volume);
    let mut event_pump = sdl_context.event_pump()?;

    // Open all connected SDL2 game controllers (M22). `gamepad_index_map`
    // maps SDL2 instance IDs → sequential NES controller index.
    let (mut gamepads, mut gamepad_index_map) = open_initial_gamepads(&game_controller_subsystem);
    let mut next_seq: usize = gamepads.len();

    let mut mapper = InputMapper::from_config(&config);

    // CPU debugger (M27): F1 pause/resume, F2 single-step, F3 run-to-BP.
    let mut debugger = CpuDebugger::new();
    // M28 debug viewers: F4 PPU viewer, F6 memory dump, F8 trace logger,
    // PageUp/PageDown navigate, `[`/`]` switch CPU/PPU region.
    let mut debug_hotkeys = DebugHotkeys::default();
    // M29 UI controls: Ctrl+R reset, F9 screenshot, Alt+Enter fullscreen,
    // Tab fast-forward.
    let mut ui_hotkeys = UiHotkeys::default();
    // M30 save-state slots (F5/F7 + 1..=9,0), rewind (Backspace), OSD (F10).
    let game_name = game_name_from_path(rom_path_ref);
    let mut save_state_hotkeys = SaveStateHotkeys::new(game_name);
    // M31 per-channel APU volume/mute hotkeys (Alt+1..5, M, Up/Down, 0).
    let mut audio_hotkeys = AudioHotkeys::new();

    'running: loop {
        // Drain all pending events each frame; ESC / Q / window-close
        // terminate. NES buttons route through `InputMapper` (M22).
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
                    keycode: Some(k),
                    keymod,
                    ..
                } => {
                    // Intercept UI + debugger + viewer + save-state +
                    // audio hotkeys (M27-M31) before joypad routing.
                    // First dispatcher to consume wins; rest skipped.
                    let consumed = ui_hotkeys.handle_key(&mut emulator, &mut video, k, keymod)
                        || handle_debugger_key(&mut debugger, k)
                        || debug_hotkeys.handle_key(emulator.bus(), k)
                        || save_state_hotkeys.handle_key(&mut emulator, k, keymod)
                        || audio_hotkeys.handle_key(emulator.bus_mut().apu_mut(), k, keymod);
                    if !consumed {
                        mapper.handle_key(emulator.bus_mut().joypad_mut(), k, true);
                    }
                }
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
                // Gamepad hot-plug: `which` is the joystick device index
                // (not instance ID) — that's what `open` expects.
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
                // Gamepad hot-unplug: drop from the index map. The
                // `GameController` stays in `gamepads` (dropping it would
                // close the device); the slot stays reserved.
                Event::ControllerDeviceRemoved { which, .. } => {
                    if gamepad_index_map.remove(&which).is_some() {
                        eprintln!("nes-emu: gamepad (instance {which}) removed");
                    }
                }
                _ => {}
            }
        }

        // Step the emulator. When paused, re-present the current
        // framebuffer; a pending single-step (F2) runs one instruction.
        // When not paused, run a full frame via `step_frame_debug`,
        // which stops early if a breakpoint matches (F3). M29
        // fast-forward (Tab): run `FAST_FORWARD_FRAMES` frames per
        // vsync tick; intermediate frames' audio is drained to avoid
        // flooding the queue (last frame's samples are kept).
        if debugger.is_paused() {
            if debugger.consume_step_request() {
                emulator.step_instruction();
            }
        } else {
            let frames_this_tick = if ui_hotkeys.fast_forward() {
                nes_emu::ui_hotkeys::FAST_FORWARD_FRAMES
            } else {
                1
            };
            for i in 0..frames_this_tick {
                if debug_hotkeys.trace_enabled() {
                    // M28: trace logging — run via `step_frame_traced`.
                    emulator.step_frame_traced(&mut debugger, &mut debug_hotkeys.trace_logger);
                } else {
                    emulator.step_frame_debug(&mut debugger);
                }
                // Fast-forward: drain intermediate frames' audio; keep
                // the last frame's samples for the post-loop push.
                if ui_hotkeys.fast_forward() && i + 1 < frames_this_tick {
                    let _ = emulator.take_audio_samples();
                }
                if debugger.is_paused() {
                    if let Some(bp) = debugger.last_hit() {
                        eprintln!("nes-emu: breakpoint hit: {bp}");
                    }
                    break;
                }
            }
        }
        // M30: when the OSD is enabled, the overlay is blitted into the
        // framebuffer *before* the single `video.present` call (inside
        // `post_frame`), so we skip the present here to avoid a double-
        // present that would halve the frame rate (the renderer is
        // vsync-locked). When the OSD is off, we present here.
        if !save_state_hotkeys.osd_enabled() {
            video.present(emulator.framebuffer())?;
        }
        let samples = emulator.take_audio_samples();
        if !samples.is_empty() {
            // Audio output is best-effort: if the queue is full (e.g. the
            // audio device is slow), drop the samples rather than blocking
            // the emulation loop.
            let _ = audio.push_samples(&samples);
        }

        // M30: post-frame hook — capture a rewind snapshot, then (if the
        // OSD is enabled) blit the overlay into the framebuffer and
        // present. When the OSD is off, this only pushes the rewind
        // snapshot and does not present (the present already happened
        // above). Encapsulated in `SaveStateHotkeys::post_frame` to keep
        // main.rs under the 400-line limit.
        save_state_hotkeys.post_frame(&mut emulator, |fb| video.present(fb))?;

        // While paused, render a console "overlay" — register snapshot +
        // disassembly window — to stderr each frame. This is the M27
        // debug overlay (a graphical egui overlay lands in M28).
        if debugger.is_paused() {
            print_debug_overlay(&emulator, &debugger);
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

    // M28: flush + close the trace log on exit so no lines are lost.
    debug_hotkeys.shutdown();

    Ok(())
}
