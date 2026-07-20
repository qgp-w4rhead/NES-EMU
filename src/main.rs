//! NES emulator entry point.
//!
//! Milestone 1 scope: initialize SDL2, open a 768x720 window (3x integer
//! scale of the NES's native 256x240 resolution), and run an event loop
//! that presents a blank black framebuffer each frame and exits when the
//! user presses ESC or closes the window.
//!
//! See: https://www.nesdev.org/wiki/PPU — native NES resolution is 256x240.

mod video;

use std::process::ExitCode;

use sdl2::event::Event;
use sdl2::keyboard::Keycode;

use video::{new_framebuffer, Video, DEFAULT_SCALE};

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

/// Initialize SDL2, build the window, and drive the event loop until the
/// user requests exit (ESC, window-close, or Q on the window).
fn run() -> Result<(), String> {
    let sdl_context = sdl2::init()?;
    let video_subsystem = sdl_context.video()?;

    let mut video = Video::new(&video_subsystem, DEFAULT_SCALE)?;
    let framebuffer = new_framebuffer();

    let mut event_pump = sdl_context.event_pump()?;

    'running: loop {
        // Drain all pending events each frame; ESC, window-close, and the
        // conventional 'Q' key all terminate the loop.
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
                _ => {}
            }
        }

        // Present the (currently blank) framebuffer. The framebuffer is
        // opaque black for M1; later milestones will fill it with real
        // PPU output.
        video.present(&framebuffer)?;
    }

    Ok(())
}
