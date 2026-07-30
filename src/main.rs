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
use std::time::{Duration, Instant};

use sdl2::controller::GameController;
use sdl2::event::Event;
use sdl2::keyboard::Keycode;

use nes_emu::app::{load_rom, save_battery_sram};
use nes_emu::audio::AudioOutput;
use nes_emu::audio_hotkeys::AudioHotkeys;
use nes_emu::config::{Config, NES_BUTTON_NAMES};
use nes_emu::debug::{handle_debugger_key, print_debug_overlay, CpuDebugger, DebugHotkeys};
use nes_emu::input::InputMapper;
use nes_emu::menu::Menu;
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
    let mut overlay_shown = false;
    let mut debug_hotkeys = DebugHotkeys::default();
    let mut ui_hotkeys = UiHotkeys::default();
    ui_hotkeys.set_turbo_ratio(config.turbo_speed);
    let mut save_state_hotkeys = SaveStateHotkeys::with_capacity(
        game_name_from_path(&current_rom_path),
        config.rewind_capacity,
    );
    save_state_hotkeys.set_branching_enabled(config.rewind_branching);
    save_state_hotkeys.set_timeline_enabled(config.timeline_enabled);
    save_state_hotkeys.set_countdown_delay_ms(config.countdown_delay_ms);
    save_state_hotkeys.set_countdown_start_number(config.countdown_start_number);
    let mut audio_hotkeys = AudioHotkeys::new();
    let mut region_hotkeys = RegionHotkeys::new();

    // F1 help overlay toggle.
    let mut help_visible = false;

    // In-emulator OSD menu (F12 to toggle).
    let mut menu = Menu::new();
    menu.set_slant_corruption(config.debug.slant_corruption);
    menu.set_inaccurate_palette(config.debug.use_inaccurate_palette);
    emulator
        .bus_mut()
        .ppu_mut()
        .set_inaccurate_palette(config.debug.use_inaccurate_palette);
    menu.set_nmi_retrigger(config.debug.nmi_retrigger);
    emulator
        .bus_mut()
        .ppu_mut()
        .set_nmi_retrigger(config.debug.nmi_retrigger);
    menu.set_ring_buffer_trace_display(config.debug.ring_buffer_trace);
    if config.debug.ring_buffer_trace {
        debug_hotkeys.enable_ring_trace();
    }

    // Driver-independent frame pacer. `present_vsync()` (in `Video::new`)
    // is requested but is not guaranteed by every GPU driver/compositor —
    // if it silently fails to throttle, this loop would free-run far
    // faster than real time, causing audio samples to be queued much
    // faster than SDL2's audio thread can drain them (an ever-growing
    // backlog that manifests as audio falling further and further behind
    // video the longer the emulator runs unthrottled). Sleeping for the
    // remainder of each frame's duration here guarantees real-time pacing
    // regardless of whether vsync actually engaged.
    let mut last_tick = Instant::now();

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
                    // F1: toggle help overlay.
                    if k == Keycode::F1 {
                        help_visible = !help_visible;
                        continue;
                    }
                    // F12: toggle the in-emulator menu. When the menu is
                    // open, all key events go to the menu — no joypad or
                    // debug hotkey routing happens.
                    if k == Keycode::F12 {
                        if menu.is_open() {
                            menu.close();
                            // Apply pending key binding changes.
                            let bindings = menu.take_pending_bindings();
                            if !bindings.is_empty() {
                                for (ctrl, btn, kc) in &bindings {
                                    if ctrl == &0 {
                                        config.keys.controller1.insert(
                                            NES_BUTTON_NAMES[*btn as usize].to_string(),
                                            kc.name(),
                                        );
                                    } else {
                                        config.keys.controller2.insert(
                                            NES_BUTTON_NAMES[*btn as usize].to_string(),
                                            kc.name(),
                                        );
                                    }
                                }
                                mapper = InputMapper::from_config(&config);
                                eprintln!("nes-emu: key bindings updated from menu");
                            }
                            // Apply debug flag changes.
                            let want_slant = menu.slant_corruption();
                            emulator
                                .bus_mut()
                                .ppu_mut()
                                .set_slant_corruption(want_slant);
                            config.debug.slant_corruption = want_slant;
                            if want_slant {
                                eprintln!("nes-emu: slant corruption injected");
                            } else {
                                eprintln!("nes-emu: slant corruption cleared");
                            }
                            let want_inacc_pal = menu.inaccurate_palette();
                            emulator
                                .bus_mut()
                                .ppu_mut()
                                .set_inaccurate_palette(want_inacc_pal);
                            config.debug.use_inaccurate_palette = want_inacc_pal;
                            if want_inacc_pal {
                                eprintln!("nes-emu: inaccurate palette injected");
                            } else {
                                eprintln!("nes-emu: inaccurate palette cleared");
                            }
                            let want_nmi_retrigger = menu.nmi_retrigger();
                            emulator
                                .bus_mut()
                                .ppu_mut()
                                .set_nmi_retrigger(want_nmi_retrigger);
                            config.debug.nmi_retrigger = want_nmi_retrigger;
                            if want_nmi_retrigger {
                                eprintln!("nes-emu: NMI retrigger bug injected");
                            } else {
                                eprintln!("nes-emu: NMI retrigger bug cleared");
                            }
                            // Apply pending rewind capacity change.
                            if let Some(cap) = menu.take_pending_rewind_capacity() {
                                save_state_hotkeys.set_rewind_capacity(cap);
                                config.rewind_capacity = cap;
                                eprintln!("nes-emu: rewind capacity set to {cap}");
                            }
                            // Apply pending rewind branching change.
                            if let Some(enabled) = menu.take_pending_branching() {
                                save_state_hotkeys.set_branching_enabled(enabled);
                                config.rewind_branching = enabled;
                                eprintln!(
                                    "nes-emu: rewind branching {}",
                                    if enabled { "enabled" } else { "disabled" }
                                );
                            }
                            // Apply pending timeline enabled change.
                            if let Some(enabled) = menu.take_pending_timeline() {
                                save_state_hotkeys.set_timeline_enabled(enabled);
                                config.timeline_enabled = enabled;
                                eprintln!(
                                    "nes-emu: timeline {}",
                                    if enabled { "enabled" } else { "disabled" }
                                );
                            }
                            // Apply pending countdown delay change.
                            if let Some(ms) = menu.take_pending_countdown_delay() {
                                save_state_hotkeys.set_countdown_delay_ms(ms);
                                config.countdown_delay_ms = ms;
                                eprintln!("nes-emu: countdown delay set to {ms}ms");
                            }
                            // Apply pending countdown start number change.
                            if let Some(n) = menu.take_pending_countdown_start() {
                                save_state_hotkeys.set_countdown_start_number(n);
                                config.countdown_start_number = n;
                                eprintln!("nes-emu: countdown start set to {n}");
                            }
                            // Apply pending turbo speed change.
                            if let Some(idx) = menu.take_pending_turbo_index() {
                                let speed = nes_emu::ui_hotkeys::TURBO_SPEEDS[idx];
                                ui_hotkeys.set_turbo_ratio(speed);
                                config.turbo_speed = speed;
                                eprintln!("nes-emu: turbo speed set to {:.2}x", speed);
                            }
                            if let Some(want_rbt) = menu.take_pending_ring_buffer_trace() {
                                config.debug.ring_buffer_trace = want_rbt;
                                if want_rbt {
                                    debug_hotkeys.enable_ring_trace();
                                    eprintln!("nes-emu: ring buffer trace enabled (pauses dump to trace.log)");
                                } else {
                                    debug_hotkeys.disable_ring_trace();
                                    eprintln!("nes-emu: ring buffer trace disabled");
                                }
                            }
                        } else {
                            menu.set_rewind_capacity_display(save_state_hotkeys.rewind_capacity());
                            menu.set_branching_display(save_state_hotkeys.branching_enabled());
                            menu.set_timeline_display(save_state_hotkeys.timeline_enabled());
                            menu.set_countdown_delay_display(save_state_hotkeys.countdown_delay_ms());
                            menu.set_countdown_start_display(save_state_hotkeys.countdown_start_number());
                            // Find current turbo index from config value.
                            let turbo_idx = nes_emu::ui_hotkeys::TURBO_SPEEDS
                                .iter()
                                .position(|&s| s == config.turbo_speed)
                                .unwrap_or(nes_emu::ui_hotkeys::DEFAULT_TURBO_SPEED_INDEX);
                            menu.set_turbo_index_display(turbo_idx);
                            menu.set_ring_buffer_trace_display(config.debug.ring_buffer_trace);
                            menu.open();
                        }
                        continue;
                    }
                    if menu.is_open() {
                        menu.handle_key(k);
                        // If the menu was closed by handle_key (Escape or
                        // "Close Menu" item), apply pending bindings — same
                        // as the F12 close path above.
                        if !menu.is_open() {
                            let bindings = menu.take_pending_bindings();
                            if !bindings.is_empty() {
                                for (ctrl, btn, kc) in &bindings {
                                    if ctrl == &0 {
                                        config.keys.controller1.insert(
                                            NES_BUTTON_NAMES[*btn as usize].to_string(),
                                            kc.name(),
                                        );
                                    } else {
                                        config.keys.controller2.insert(
                                            NES_BUTTON_NAMES[*btn as usize].to_string(),
                                            kc.name(),
                                        );
                                    }
                                }
                                mapper = InputMapper::from_config(&config);
                                eprintln!("nes-emu: key bindings updated from menu");
                            }
                            let want_slant = menu.slant_corruption();
                            emulator
                                .bus_mut()
                                .ppu_mut()
                                .set_slant_corruption(want_slant);
                            config.debug.slant_corruption = want_slant;
                            let want_inacc_pal = menu.inaccurate_palette();
                            emulator
                                .bus_mut()
                                .ppu_mut()
                                .set_inaccurate_palette(want_inacc_pal);
                            config.debug.use_inaccurate_palette = want_inacc_pal;
                            let want_nmi_retrigger = menu.nmi_retrigger();
                            emulator
                                .bus_mut()
                                .ppu_mut()
                                .set_nmi_retrigger(want_nmi_retrigger);
                            config.debug.nmi_retrigger = want_nmi_retrigger;
                            if let Some(cap) = menu.take_pending_rewind_capacity() {
                                save_state_hotkeys.set_rewind_capacity(cap);
                                config.rewind_capacity = cap;
                                eprintln!("nes-emu: rewind capacity set to {cap}");
                            }
                            if let Some(enabled) = menu.take_pending_branching() {
                                save_state_hotkeys.set_branching_enabled(enabled);
                                config.rewind_branching = enabled;
                                eprintln!(
                                    "nes-emu: rewind branching {}",
                                    if enabled { "enabled" } else { "disabled" }
                                );
                            }
                            if let Some(enabled) = menu.take_pending_timeline() {
                                save_state_hotkeys.set_timeline_enabled(enabled);
                                config.timeline_enabled = enabled;
                                eprintln!(
                                    "nes-emu: timeline {}",
                                    if enabled { "enabled" } else { "disabled" }
                                );
                            }
                            if let Some(ms) = menu.take_pending_countdown_delay() {
                                save_state_hotkeys.set_countdown_delay_ms(ms);
                                config.countdown_delay_ms = ms;
                                eprintln!("nes-emu: countdown delay set to {ms}ms");
                            }
                            if let Some(n) = menu.take_pending_countdown_start() {
                                save_state_hotkeys.set_countdown_start_number(n);
                                config.countdown_start_number = n;
                                eprintln!("nes-emu: countdown start set to {n}");
                            }
                            if let Some(idx) = menu.take_pending_turbo_index() {
                                let speed = nes_emu::ui_hotkeys::TURBO_SPEEDS[idx];
                                ui_hotkeys.set_turbo_ratio(speed);
                                config.turbo_speed = speed;
                                eprintln!("nes-emu: turbo speed set to {:.2}x", speed);
                            }
                            if let Some(want_rbt) = menu.take_pending_ring_buffer_trace() {
                                config.debug.ring_buffer_trace = want_rbt;
                                if want_rbt {
                                    debug_hotkeys.enable_ring_trace();
                                    eprintln!("nes-emu: ring buffer trace enabled (pauses dump to trace.log)");
                                } else {
                                    debug_hotkeys.disable_ring_trace();
                                    eprintln!("nes-emu: ring buffer trace disabled");
                                }
                            }
                        }
                        continue;
                    }
                    // Intercept UI/debugger/save-state/audio/region/ROM-info
                    // hotkeys (M27-M34) before joypad routing.
                    // save_state_hotkeys is checked before ui_hotkeys so
                    // Space is consumed for rewind speed cycling when
                    // rewinding, instead of falling through to turbo.
                    // Also handle Shift key-down for seamless rewind→forward
                    // switching while Backspace is held.
                    if k == Keycode::LShift || k == Keycode::RShift {
                        save_state_hotkeys.shift_pressed();
                    }
                    // Tab skips the 3-2-1 countdown when active.
                    if k == Keycode::Tab && save_state_hotkeys.is_countdown_active() {
                        save_state_hotkeys.skip_countdown();
                        continue;
                    }
                    let consumed = save_state_hotkeys.handle_key(&mut emulator, k, keymod)
                        || ui_hotkeys.handle_key(&mut emulator, &mut video, k, keymod)
                        || handle_debugger_key(&mut debugger, k)
                        || debug_hotkeys.handle_key(emulator.bus_mut(), k)
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
                    // Backspace release: stop rewinding and/or forwarding.
                    // If paused at a branch diverge point, keep the pause
                    // state so the user can use Up/Down to select a branch
                    // and O to accept it. If paused at the beginning of a
                    // branch, stop rewinding but keep the pause state so
                    // the user can press O to return to the parent.
                    if k == Keycode::Backspace {
                        if save_state_hotkeys.is_paused_at_branch_point() {
                            // Don't auto-accept — let user select with
                            // Up/Down and press O to accept.
                            save_state_hotkeys.stop_rewind_keep_paused();
                        } else if save_state_hotkeys.is_paused_at_start() {
                            // Paused at beginning of branch: stop rewinding
                            // but keep paused_at_start so user can press O
                            // to return to the parent timeline.
                            save_state_hotkeys.stop_rewind_keep_paused();
                        } else if save_state_hotkeys.is_paused_at_return() {
                            // Paused at return point on parent: stop
                            // rewinding/forwarding but keep paused_at_return
                            // so user can continue scrubbing or press O
                            // to resume normal play.
                            save_state_hotkeys.stop_rewind_keep_paused();
                        } else {
                            if save_state_hotkeys.is_forwarding() {
                                save_state_hotkeys.stop_forward();
                                eprintln!(
                                    "nes-emu: forward stopped ({} snapshots in forward buffer)",
                                    save_state_hotkeys.active_forward_len()
                                );
                            }
                            if save_state_hotkeys.is_rewinding() {
                                save_state_hotkeys.stop_rewind();
                                eprintln!(
                                    "nes-emu: rewind stopped ({} snapshots)",
                                    save_state_hotkeys.active_rewind_len()
                                );
                            }
                        }
                    }
                    // Shift release: if Backspace is still held, switch
                    // from forwarding back to rewinding. If Backspace was
                    // already released, stop forwarding.
                    if k == Keycode::LShift || k == Keycode::RShift {
                        save_state_hotkeys.shift_released();
                    }
                    // Space release: stop turbo.
                    if k == Keycode::Space {
                        ui_hotkeys.stop_turbo();
                    }
                    if !menu.is_open() {
                        mapper.handle_key(emulator.bus_mut().joypad_mut(), k, false);
                    }
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
                    save_state_hotkeys = SaveStateHotkeys::with_capacity(
                        game_name_from_path(&current_rom_path),
                        config.rewind_capacity,
                    );
                    save_state_hotkeys.set_branching_enabled(config.rewind_branching);
                    save_state_hotkeys.set_timeline_enabled(config.timeline_enabled);
                    save_state_hotkeys.set_countdown_delay_ms(config.countdown_delay_ms);
                    save_state_hotkeys.set_countdown_start_number(config.countdown_start_number);
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
                overlay_shown = false;
            }
        } else if save_state_hotkeys.is_paused_at_branch() {
            // Paused at a branch point: do nothing, wait for user to
            // press o (accept) or p (deny).
        } else if save_state_hotkeys.is_countdown_active() {
            // 3-2-1 countdown active: pause emulation, advance the
            // countdown state machine. Tab skips the countdown.
            save_state_hotkeys.tick_countdown();
        } else if save_state_hotkeys.is_rewinding() {
            // Rewind: pop snapshots per vsync tick (speed-controlled)
            // instead of stepping forward. If the buffer is empty,
            // emulation pauses until the rewind key is released.
            save_state_hotkeys.rewind_step(&mut emulator);
        } else if save_state_hotkeys.is_forwarding() {
            // Forward: pop from the forward buffer to advance through
            // previously-rewound states. If empty, emulation pauses.
            save_state_hotkeys.forward_step(&mut emulator);
        } else if ui_hotkeys.turbo_held() {
            // Turbo (Spacebar held): run `turbo_ratio`× frames per tick.
            // Uses a fractional accumulator for sub-1× speeds.
            let frames_this_tick = ui_hotkeys.turbo_frame_count();
            for i in 0..frames_this_tick {
                if debug_hotkeys.ring_trace_active() {
                    emulator.step_frame_ring_traced(&mut debugger, &mut debug_hotkeys.ring_trace);
                } else if debug_hotkeys.trace_enabled() {
                    emulator.step_frame_traced(&mut debugger, &mut debug_hotkeys.trace_logger);
                } else {
                    emulator.step_frame_debug(&mut debugger);
                }
                if i + 1 < frames_this_tick {
                    let _ = emulator.take_audio_samples();
                }
                if debugger.is_paused() {
                    if debug_hotkeys.ring_trace_active() {
                        let n = debug_hotkeys.dump_ring_trace();
                        eprintln!("nes-emu: ring trace dumped {n} lines to trace.log");
                    }
                    if let Some(bp) = debugger.last_hit() {
                        eprintln!("nes-emu: breakpoint hit: {bp}");
                    }
                    break;
                }
            }
        } else {
            let frames_this_tick = if ui_hotkeys.fast_forward() {
                nes_emu::ui_hotkeys::FAST_FORWARD_FRAMES
            } else {
                1
            };
            for i in 0..frames_this_tick {
                if debug_hotkeys.ring_trace_active() {
                    emulator.step_frame_ring_traced(&mut debugger, &mut debug_hotkeys.ring_trace);
                } else if debug_hotkeys.trace_enabled() {
                    emulator.step_frame_traced(&mut debugger, &mut debug_hotkeys.trace_logger);
                } else {
                    emulator.step_frame_debug(&mut debugger);
                }
                if ui_hotkeys.fast_forward() && i + 1 < frames_this_tick {
                    let _ = emulator.take_audio_samples();
                }
                if debugger.is_paused() {
                    if debug_hotkeys.ring_trace_active() {
                        let n = debug_hotkeys.dump_ring_trace();
                        eprintln!("nes-emu: ring trace dumped {n} lines to trace.log");
                    }
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
        // Rewind/forward timeline overlay — shown on screen while
        // rewinding or forwarding (or fading out after release),
        // regardless of OSD state. Advance the animation first.
        save_state_hotkeys.tick_animation();
        save_state_hotkeys.render_rewind_overlay(emulator.framebuffer_mut(), 256, 240);
        // 3-2-1 countdown overlay — drawn on top of the timeline.
        save_state_hotkeys.render_countdown(emulator.framebuffer_mut(), 256, 240);
        let osd_on = save_state_hotkeys.osd_enabled();
        let info_on = rom_manager.info_overlay_enabled();
        let menu_on = menu.is_open();
        let help_on = help_visible;
        if !osd_on && !info_on && !menu_on && !help_on {
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

        // In-emulator menu overlay — when open, blit the menu into the
        // framebuffer and present. Runs last so it sits on top of all
        // other overlays.
        if menu_on {
            menu.render(emulator.framebuffer_mut(), 256, 240, &config);
            if !osd_on && !info_on {
                video.present(emulator.framebuffer())?;
            }
        }

        // F1 help overlay — lists all F-key bindings on screen.
        if help_on {
            let help_lines: &[&str] = &[
                "== NES-EMU HELP ==",
                "F1  Help (this screen)",
                "F2  Pause/Resume",
                "F3  Breakpoint toggle",
                "N   Single-step (when paused)",
                "F4  PPU viewer",
                "F5  Save state",
                "F6  Memory viewer",
                "F7  Load state",
                "F8  Trace log toggle",
                "T   PPU write log toggle",
                "F12 Menu (Debug: Ring Buffer Trace)",
                "F9  Screenshot",
                "F10 OSD toggle",
                "F11 Region cycle",
                "Tab Fast-forward 4x / Skip countdown",
                "Spc Turbo (hold)",
                "Bsp Rewind (hold)",
                "S+Bsp Forward (hold)",
                "Spc Rewind speed (while rewinding)",
                "Ctl+R Reset",
                "Alt+Enter Fullscreen",
                "1-0 Save slot select",
                "Alt+1-5 Audio channel",
                "Alt+M Mute channel",
                "Alt+Up/Dn Volume",
                "PgUp/Dn Mem navigate",
                "[/]  Mem region",
                "ESC/Q Quit",
            ];
            save_state_hotkeys
                .osd
                .render_forced(emulator.framebuffer_mut(), 256, 240, help_lines);
            if !osd_on && !info_on && !menu_on {
                video.present(emulator.framebuffer())?;
            }
        }

        // While paused, render a console "overlay" — register snapshot +
        // disassembly window — to stderr once per pause session (M27 debug
        // overlay). Reprinted after each single-step so the user sees the
        // updated state.
        if debugger.is_paused() {
            if !overlay_shown {
                if debug_hotkeys.ring_trace_active() {
                    let n = debug_hotkeys.dump_ring_trace();
                    eprintln!("nes-emu: ring trace dumped {n} lines to trace.log");
                }
                print_debug_overlay(&emulator, &debugger);
                overlay_shown = true;
            }
        } else {
            overlay_shown = false;
        }

        // Pace the loop to real time (see comment above `last_tick`).
        // Skipped while fast-forwarding (intentionally faster than
        // real-time) and while the debugger is paused (no frame ran).
        //
        // Windows `thread::sleep` has ~15.6 ms granularity by default, but
        // an NTSC frame is 16.64 ms — the mismatch causes irregular frame
        // spacing and periodic audio buffer underruns (audible pops every
        // ~1 s). To get sub-millisecond pacing we sleep for the bulk of the
        // wait, then spin-wait the final 2 ms using `Instant::elapsed()`.
        if !ui_hotkeys.fast_forward() && !debugger.is_paused() {
            let target = Duration::from_secs_f64(1.0 / emulator.region().frame_rate_hz() as f64);
            let elapsed = last_tick.elapsed();
            if elapsed < target {
                let remaining = target - elapsed;
                let spin_threshold = Duration::from_millis(2);
                if remaining > spin_threshold {
                    std::thread::sleep(remaining - spin_threshold);
                }
                while last_tick.elapsed() < target {
                    std::hint::spin_loop();
                }
            }
        }
        last_tick = Instant::now();
    }

    // Battery-backed PRG-RAM persistence (M21): on exit, dump the
    // cartridge's PRG-RAM to the `.nessram` sidecar file next to the ROM.
    // Save errors are non-fatal — the emulator is already shutting down.
    save_battery_sram(&emulator, &current_rom_path);

    // M34: persist the updated recent-ROM list to config.toml.
    if let Err(e) = config.save_to_path(&config_path) {
        eprintln!("nes-emu: warning: could not save config: {e}");
    }

    // M28: flush + close the trace log and PPU write log on exit.
    debug_hotkeys.shutdown_ppu_write_log(emulator.bus_mut());
    debug_hotkeys.shutdown();

    // Keep `gamepads` alive until after the loop so the SDL2 controller
    // handles stay open for the duration of the session.
    drop(gamepads);

    Ok(())
}
