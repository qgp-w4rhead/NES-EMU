//! NES emulator entry point.
//!
//! Milestone 12 scope: load an iNES ROM from `--rom <path>`, build the
//! `EmulatorState`, and run a frame-locked main loop that steps the
//! emulator one NTSC frame per vsync and presents the PPU framebuffer via
//! SDL2.
//!
//! See: https://www.nesdev.org/wiki/PPU — native NES resolution is 256x240.
//! See: https://www.nesdev.org/wiki/Cycle_reference — ~29,830 CPU cycles
//! per NTSC frame at 60.0988 Hz.

use std::process::ExitCode;

use sdl2::event::Event;
use sdl2::keyboard::Keycode;

use nes_emu::cartridge::Cartridge;
use nes_emu::emulator::EmulatorState;
use nes_emu::input::handle_key;
use nes_emu::video::{Video, DEFAULT_SCALE};

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
    eprintln!("usage: nes-emu --rom <path-to-.nes>");
}

/// Parse `--rom <path>` from the argument list. Returns the path or an
/// error message.
fn parse_rom_path(args: &[String]) -> Result<String, String> {
    let mut iter = args.iter().skip(1);
    while let Some(arg) = iter.next() {
        if arg == "--rom" {
            return iter
                .next()
                .ok_or_else(|| "--rom requires a path argument".to_string())
                .cloned();
        }
        if let Some(rest) = arg.strip_prefix("--rom=") {
            return Ok(rest.to_string());
        }
    }
    Err("missing --rom <path>".to_string())
}

/// Initialize SDL2, load the ROM, build the emulator, and drive the
/// frame-locked main loop until the user requests exit (ESC, window-close,
/// or Q on the window).
fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let rom_path = parse_rom_path(&args).inspect_err(|_e| {
        print_usage();
    })?;

    let cartridge = Cartridge::from_path(&rom_path)
        .map_err(|e| format!("failed to load ROM '{rom_path}': {e}"))?;

    let mut emulator = EmulatorState::new(cartridge);
    emulator.reset();

    let sdl_context = sdl2::init()?;
    let video_subsystem = sdl_context.video()?;

    let mut video = Video::new(&video_subsystem, DEFAULT_SCALE)?;
    let mut event_pump = sdl_context.event_pump()?;

    'running: loop {
        // Drain all pending events each frame; ESC, window-close, and the
        // conventional 'Q' key all terminate the loop. NES controller
        // buttons are wired to the joypad via `handle_key` (M13).
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
                } => handle_key(emulator.bus_mut().joypad_mut(), k, true),
                Event::KeyUp {
                    keycode: Some(k), ..
                } => handle_key(emulator.bus_mut().joypad_mut(), k, false),
                _ => {}
            }
        }

        // Step one full NTSC frame (CPU + PPU in lockstep), then present
        // the rendered framebuffer. SDL2's vsynced renderer paces the
        // loop to the monitor refresh rate (~60 Hz).
        emulator.step_frame();
        video.present(emulator.framebuffer())?;
    }

    Ok(())
}
