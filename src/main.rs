//! NES emulator entry point.
//!
//! Loads an iNES ROM from `--rom <path>`, builds the `EmulatorState`, and
//! runs a frame-locked main loop that steps the emulator one frame per
//! vsync and presents the PPU framebuffer via SDL2. Key bindings and gamepad
//! mappings come from `config.toml` (M22). Debug hotkeys (M27/M28), UI
//! controls (M29), save-state/OSD (M30), audio volume/mute (M31), region
//! switching (M32), and ROM management (M34 — drag-and-drop, recent list,
//! ROM info overlay) are dispatched before joypad routing.
//!
//! See: https://www.nesdev.org/wiki/PPU (256x240) and
//! https://www.nesdev.org/wiki/Cycle_reference (~29,830 CPU cycles/NTSC frame).

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use sdl2::controller::GameController;
use sdl2::event::Event;
use sdl2::keyboard::Keycode;

use nes_emu::app::{load_rom, save_battery_sram};
use nes_emu::audio::AudioOutput;
use nes_emu::audio_hotkeys::AudioHotkeys;
use nes_emu::config::Config;
use nes_emu::debug::{handle_debugger_key, print_debug_overlay, CpuDebugger, DebugHotkeys};
use nes_emu::input::InputMapper;
use nes_emu::osd::game_name_from_path;
use nes_emu::region_hotkeys::RegionHotkeys;
use nes_emu::rom_manager::{RomManager, ROM_INFO_HOTKEY};
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
    let rom_path = PathBuf::from(cli.rom_path);
    let config_path = resolve_config_path(cli.config_path.as_deref());

    // Load user config (M22). Missing/invalid config is non-fatal.
    let mut config = match Config::load_from_path(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("nes-emu: warning: {e}; using default config");
            Config::default()
        }
    };
    if let Err(e) = Config::ensure_default_file(&config_path) {
        eprintln!("nes-emu: warning: could not write default config: {e}");
    }

    // M34: ROM manager — drag-and-drop queue, recent list, ROM info overlay.
    let mut rom_manager = RomManager::new(config.recent_roms.clone());

    // Initial ROM load (shared with drag-and-drop reload path).
    let loaded = load_rom(&rom_path, &config)?;
    let mut emulator = loaded.emulator;
    let mut current_rom_path = loaded.rom_path.clone();
    let mut current_rom_info = loaded.info;
    let new_recent = rom_manager.record_loaded_rom(&rom_path.to_string_lossy());
    config.recent_roms = new_recent;

    let sdl_context = sdl2::init()?;
    let video_subsystem = sdl_context.video()?;
    let audio_subsystem = sdl_context.audio()?;
    let game_controller_subsystem = sdl_context.game_controller()?;

    let scale = if (1..=8).contains(&config.window_scale) {
        config.window_scale
    } else {
        nes_emu::video::DEFAULT_SCALE
    };
    let mut video = Video::new(&video_subsystem, scale)?;
    let mut audio = AudioOutput::new(&audio_subsystem)?;
    audio.set_volume(config.audio_volume);
    let mut event_pump = sdl_context.event_pump()?;
    // M34: enable SDL2 file-drop events for drag-and-drop ROM loading.
    event_pump.enable_event(sdl2::event::EventType::DropFile);

    let (mut gamepads, mut gamepad_index_map) = open_initial_gamepads(&game_controller_subsystem);
    let mut next_seq: usize = gamepads.len();
    let mut mapper = InputMapper::from_config(&config);

    let mut debugger = CpuDebugger::new();
    let mut debug_hotkeys = DebugHotkeys::default();
    let mut ui_hotkeys = UiHotkeys::default();
    let mut save_state_hotkeys = SaveStateHotkeys::new(game_name_from_path(&current_rom_path));
    let mut audio_hotkeys = AudioHotkeys::new();
    let mut region_hotkeys = RegionHotkeys::new();

    'running: loop {
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
                // M34: drag-and-drop — queue the path; reload between frames.
                Event::DropFile { filename, .. } => {
                    rom_manager.queue_drop(PathBuf::from(filename));
                }
                Event::KeyDown {
                    keycode: Some(k),
                    keymod,
                    ..
                } => {
                    // Intercept UI/debugger/save-state/audio/region/ROM-info
                    // hotkeys (M27-M34) before joypad routing.
                    let consumed = ui_hotkeys.handle_key(&mut emulator, &mut video, k, keymod)
                        || handle_debugger_key(&mut debugger, k)
                        || debug_hotkeys.handle_key(emulator.bus(), k)
                        || save_state_hotkeys.handle_key(&mut emulator, k, keymod)
                        || audio_hotkeys.handle_key(emulator.bus_mut().apu_mut(), k, keymod)
                        || region_hotkeys.handle_key(&mut emulator, k, keymod)
                        || (k == ROM_INFO_HOTKEY && {
                            rom_manager.toggle_info_overlay();
                            true
                        });
                    if !consumed {
                        mapper.handle_key(emulator.bus_mut().joypad_mut(), k, true);
                    }
                }
                Event::KeyUp {
                    keycode: Some(k), ..
                } => {
                    mapper.handle_key(emulator.bus_mut().joypad_mut(), k, false);
                }
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
                // Gamepad hot-plug: `which` is the joystick device index.
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
                Event::ControllerDeviceRemoved { which, .. } => {
                    if gamepad_index_map.remove(&which).is_some() {
                        eprintln!("nes-emu: gamepad (instance {which}) removed");
                    }
                }
                _ => {}
            }
        }

        // M34: process a pending drag-and-drop reload between frames.
        if let Some(dropped_path) = rom_manager.take_pending_drop() {
            save_battery_sram(&emulator, &current_rom_path);
            match load_rom(&dropped_path, &config) {
                Ok(loaded) => {
                    emulator = loaded.emulator;
                    current_rom_path = loaded.rom_path.clone();
                    current_rom_info = loaded.info;
                    let new_recent = rom_manager.record_loaded_rom(&dropped_path.to_string_lossy());
                    config.recent_roms = new_recent;
                    save_state_hotkeys =
                        SaveStateHotkeys::new(game_name_from_path(&current_rom_path));
                    debugger = CpuDebugger::new();
                    eprintln!("nes-emu: loaded ROM: {}", current_rom_path.display());
                }
                Err(e) => eprintln!("nes-emu: {e}"),
            }
        }

        // Step the emulator. When paused, a pending single-step (F2) runs
        // one instruction. When running, execute a full frame (or
        // `FAST_FORWARD_FRAMES` frames if Tab is held), stopping early if
        // a breakpoint matches (F3). Intermediate fast-forward frames'
        // audio is drained to avoid flooding the queue.
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
                    emulator.step_frame_traced(&mut debugger, &mut debug_hotkeys.trace_logger);
                } else {
                    emulator.step_frame_debug(&mut debugger);
                }
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
        // M30/M34: present logic. When neither the OSD nor the ROM-info
        // overlay is enabled, present the raw framebuffer here. When
        // either is enabled, the present is deferred to the post-frame
        // hook (OSD) or the info-overlay blit below, so the overlay is
        // drawn *before* the present (a double-present would halve the
        // frame rate since the renderer is vsync-locked).
        let osd_on = save_state_hotkeys.osd_enabled();
        let info_on = rom_manager.info_overlay_enabled();
        if !osd_on && !info_on {
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
        // snapshot and does not present. Encapsulated in
        // `SaveStateHotkeys::post_frame` to keep main.rs compact.
        save_state_hotkeys.post_frame(&mut emulator, |fb| video.present(fb))?;

        // M34: ROM-info overlay — when enabled, blit the info lines into
        // the framebuffer and present. This runs after the save-state
        // post_frame hook so the OSD + info overlay stack correctly; if
        // the OSD already presented, we skip the present here to avoid a
        // double-present.
        if info_on {
            rom_manager.render_info_overlay(
                emulator.framebuffer_mut(),
                256,
                240,
                &current_rom_info,
            );
            if !osd_on {
                video.present(emulator.framebuffer())?;
            }
        }

        // While paused, render a console "overlay" — register snapshot +
        // disassembly window — to stderr each frame (M27 debug overlay).
        if debugger.is_paused() {
            print_debug_overlay(&emulator, &debugger);
        }
    }

    // Battery-backed PRG-RAM persistence (M21): on exit, dump the
    // cartridge's PRG-RAM to the `.nessram` sidecar file next to the ROM.
    // Save errors are non-fatal — the emulator is already shutting down.
    save_battery_sram(&emulator, &current_rom_path);

    // M34: persist the updated recent-ROM list to config.toml.
    if let Err(e) = config.save_to_path(&config_path) {
        eprintln!("nes-emu: warning: could not save config: {e}");
    }

    // M28: flush + close the trace log on exit so no lines are lost.
    debug_hotkeys.shutdown();

    // Keep `gamepads` alive until after the loop so the SDL2 controller
    // handles stay open for the duration of the session.
    drop(gamepads);

    Ok(())
}
