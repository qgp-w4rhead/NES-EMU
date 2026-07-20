//! Debug hotkey dispatcher — owns the M28 debug viewer state (PPU
//! viewer, memory viewer, trace logger) and centralizes the F4 / F6 /
//! F8 / PageUp / PageDown / `[` / `]` key handling so `src/main.rs`
//! stays under the 400-line file-size limit.
//!
//! The dispatcher is a *consumer* of the emulation core: every read it
//! makes goes through side-effect-free accessors (see
//! [`crate::debug::ppu_viewer`] and [`crate::debug::memory_viewer`]).
//! The only mutation it performs on emulator state is via
//! [`MemoryViewer::poke`], which is not exposed through a hotkey (it is
//! an API for future GUI / script integration).
//!
//! See: [`crate::debug`] for the individual viewer modules.

use sdl2::keyboard::Keycode;

use crate::bus::Bus;
use crate::debug::{CpuDebugger, MemoryViewer, PpuViewer, TraceLogger};

/// Default trace log file path (relative to the CWD).
pub const DEFAULT_TRACE_LOG_PATH: &str = "trace.log";

/// Bundle of M28 debug viewer state + the trace logger. Held by the
/// main loop; [`DebugHotkeys::handle_key`] dispatches key events to the
/// appropriate viewer.
pub struct DebugHotkeys {
    /// PPU viewer state (F4 cycles views).
    pub ppu_viewer: PpuViewer,
    /// Memory viewer state (F6 dumps, PageUp/PageDown navigate,
    /// `[`/`]` switch regions).
    pub mem_viewer: MemoryViewer,
    /// Trace logger (F8 toggles on/off).
    pub trace_logger: TraceLogger,
    /// Path the trace logger writes to when started via F8.
    pub trace_log_path: String,
}

impl Default for DebugHotkeys {
    fn default() -> Self {
        Self::new(DEFAULT_TRACE_LOG_PATH.to_string())
    }
}

impl DebugHotkeys {
    /// Construct a dispatcher with the given trace log path.
    pub fn new(trace_log_path: String) -> Self {
        Self {
            ppu_viewer: PpuViewer::new(),
            mem_viewer: MemoryViewer::new(),
            trace_logger: TraceLogger::new(),
            trace_log_path,
        }
    }

    /// Is trace logging currently active?
    pub fn trace_enabled(&self) -> bool {
        self.trace_logger.is_enabled()
    }

    /// Handle a key-down event. Returns `true` if the key was consumed
    /// by a debug hotkey (and should *not* be forwarded to the joypad),
    /// `false` if it should be routed to the input mapper as usual.
    ///
    /// The `bus` borrow is immutable — all viewer reads are
    /// side-effect-free. The trace logger's file I/O is independent of
    /// the bus.
    pub fn handle_key(&mut self, bus: &Bus, key: Keycode) -> bool {
        match key {
            // F4: PPU viewer — cycle through Nametables → PatternTable
            // (bank 0/1) → OAM → Palettes and print the current view.
            Keycode::F4 => {
                self.ppu_viewer.cycle();
                eprint!("{}", self.ppu_viewer.render(bus));
                true
            }
            // F6: Memory viewer — dump the current window.
            Keycode::F6 => {
                eprint!("{}", self.mem_viewer.dump(bus));
                true
            }
            // F8: Trace logger — toggle on/off.
            Keycode::F8 => {
                if self.trace_logger.is_enabled() {
                    let lines = self.trace_logger.stop();
                    eprintln!(
                        "nes-emu: trace logging stopped ({lines} lines written to {})",
                        self.trace_log_path
                    );
                } else {
                    match self.trace_logger.start(&self.trace_log_path) {
                        Ok(()) => {
                            eprintln!("nes-emu: trace logging started → {}", self.trace_log_path)
                        }
                        Err(e) => eprintln!("nes-emu: could not start trace log: {e}"),
                    }
                }
                true
            }
            // PageUp / PageDown: memory viewer navigation.
            Keycode::PageUp => {
                self.mem_viewer.page_up();
                eprint!("{}", self.mem_viewer.dump(bus));
                true
            }
            Keycode::PageDown => {
                self.mem_viewer.page_down();
                eprint!("{}", self.mem_viewer.dump(bus));
                true
            }
            // `[` / `]`: switch memory region (CPU ↔ PPU). Both keys
            // toggle (there are only two regions, so a single toggle
            // suffices).
            Keycode::LeftBracket | Keycode::RightBracket => {
                self.mem_viewer.toggle_region();
                eprint!("{}", self.mem_viewer.dump(bus));
                true
            }
            _ => false,
        }
    }

    /// Flush + close the trace log on emulator exit so no lines are
    /// lost. No-op when logging is stopped.
    pub fn shutdown(&mut self) {
        if self.trace_logger.is_enabled() {
            let lines = self.trace_logger.stop();
            eprintln!("nes-emu: trace logging stopped on exit ({lines} lines)");
        }
    }
}

/// Handle M27 debugger hotkeys (F1/F2/F3). Returns `true` if the key
/// was consumed. Extracted from `src/main.rs` so the binary stays under
/// the 400-line file-size limit.
///
/// - **F1**: toggle pause/resume.
/// - **F2**: single-step (only when paused; otherwise a no-op with a
///   stderr hint).
/// - **F3**: toggle run-to-breakpoint mode.
pub fn handle_debugger_key(debugger: &mut CpuDebugger, key: Keycode) -> bool {
    match key {
        Keycode::F1 => {
            debugger.toggle_pause();
            let state = if debugger.is_paused() {
                "paused"
            } else {
                "resumed"
            };
            eprintln!("nes-emu: debugger {state}");
            true
        }
        Keycode::F2 => {
            if debugger.is_paused() {
                debugger.request_step();
            } else {
                eprintln!("nes-emu: F2 single-step ignored (debugger not paused; press F1 first)");
            }
            true
        }
        Keycode::F3 => {
            debugger.toggle_run_to_breakpoint();
            let state = if debugger.run_to_breakpoint() {
                "ON"
            } else {
                "OFF"
            };
            eprintln!("nes-emu: run-to-breakpoint {state}");
            if debugger.run_to_breakpoint() && debugger.breakpoints().is_empty() {
                eprintln!(
                    "nes-emu: no breakpoints set — add some via the debugger API to use run-to-breakpoint"
                );
            }
            true
        }
        _ => false,
    }
}
