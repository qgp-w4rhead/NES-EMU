//! Debug hotkey dispatcher — owns the M28 debug viewer state (PPU
//! viewer, memory viewer, trace logger) and centralizes the F4 / F6 /
//! F8 / PageUp / PageDown / `[` / `]` key handling so `src/main.rs`
//! stays under the 400-line file-size limit.
//!
//! The debugger pause/step/breakpoint keys (F2 / N / F3) are handled
//! by [`handle_debugger_key`]. F1 (help overlay) is handled in
//! `src/main.rs` directly.
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
use crate::debug::{CpuDebugger, MemoryViewer, PpuViewer, PpuWriteLogger, RingTraceLogger, TraceLogger};

/// Default trace log file path (under ./logs/).
pub const DEFAULT_TRACE_LOG_PATH: &str = "logs/trace.log";

/// Default PPU write log file path (under ./logs/).
pub const DEFAULT_PPU_WRITE_LOG_PATH: &str = "logs/ppu_writes.log";

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
    /// Whether PPU write logging is active (T toggles). The actual
    /// logger lives on the bus; this flag tracks state for the hotkey.
    ppu_write_log_active: bool,
    /// Path the PPU write logger writes to when started via T.
    pub ppu_write_log_path: String,
    /// Ring buffer trace logger — keeps last N instructions in memory,
    /// flushed to trace.log on pause. Enabled via OSD menu debug option.
    pub ring_trace: RingTraceLogger,
    /// Whether ring buffer trace mode is enabled (toggled via OSD menu).
    pub ring_trace_enabled: bool,
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
            ppu_write_log_active: false,
            ppu_write_log_path: DEFAULT_PPU_WRITE_LOG_PATH.to_string(),
            ring_trace: RingTraceLogger::default(),
            ring_trace_enabled: false,
        }
    }

    /// Is trace logging currently active?
    pub fn trace_enabled(&self) -> bool {
        self.trace_logger.is_enabled()
    }

    /// Is PPU write logging currently active?
    pub fn ppu_write_log_enabled(&self) -> bool {
        self.ppu_write_log_active
    }

    /// Is ring buffer trace mode active?
    pub fn ring_trace_active(&self) -> bool {
        self.ring_trace_enabled && self.ring_trace.is_enabled()
    }

    /// Enable ring buffer trace mode (starts buffering).
    pub fn enable_ring_trace(&mut self) {
        if !self.ring_trace_enabled {
            self.ring_trace_enabled = true;
        }
        if !self.ring_trace.is_enabled() {
            self.ring_trace.start();
        }
    }

    /// Disable ring buffer trace mode (stops buffering, clears buffer).
    pub fn disable_ring_trace(&mut self) {
        self.ring_trace_enabled = false;
        self.ring_trace.stop();
    }

    /// Dump the ring buffer to the trace log path. Returns the number of
    /// lines written, or 0 if the ring buffer is not active.
    pub fn dump_ring_trace(&mut self) -> usize {
        if !self.ring_trace.is_enabled() {
            return 0;
        }
        match self.ring_trace.dump(&self.trace_log_path) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("nes-emu: ring trace dump failed: {e}");
                0
            }
        }
    }

    /// Handle a key-down event. Returns `true` if the key was consumed
    /// by a debug hotkey (and should *not* be forwarded to the joypad),
    /// `false` if it should be routed to the input mapper as usual.
    ///
    /// The `bus` borrow is immutable for viewer reads. The PPU write
    /// logger toggle requires `&mut Bus` to install the logger.
    pub fn handle_key(&mut self, bus: &mut Bus, key: Keycode) -> bool {
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
            // T: PPU write logger — toggle on/off. The logger lives on
            // the bus so it can intercept writes; this flag tracks state.
            Keycode::T => {
                if self.ppu_write_log_active {
                    if let Some(logger) = bus.ppu_write_logger_mut() {
                        let lines = logger.stop();
                        eprintln!(
                            "nes-emu: PPU write logging stopped ({lines} lines written to {})",
                            self.ppu_write_log_path
                        );
                    }
                    self.ppu_write_log_active = false;
                } else {
                    let mut logger = PpuWriteLogger::new();
                    match logger.start(&self.ppu_write_log_path) {
                        Ok(()) => {
                            bus.set_ppu_write_logger(logger);
                            self.ppu_write_log_active = true;
                            eprintln!("nes-emu: PPU write logging started → {}", self.ppu_write_log_path);
                        }
                        Err(e) => eprintln!("nes-emu: could not start PPU write log: {e}"),
                    }
                }
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

    /// Flush + close the trace log and PPU write log on emulator exit so
    /// no lines are lost. No-op when logging is stopped.
    pub fn shutdown(&mut self) {
        if self.trace_logger.is_enabled() {
            let lines = self.trace_logger.stop();
            eprintln!("nes-emu: trace logging stopped on exit ({lines} lines)");
        }
        // PPU write logger is on the bus; the caller should stop it via
        // `bus.ppu_write_logger_mut()` before shutdown. This flag tracks
        // whether it was active so the caller can check.
    }

    /// Stop the PPU write logger on the bus. Called by the main loop on
    /// exit so the bus-owned logger is properly flushed.
    pub fn shutdown_ppu_write_log(&mut self, bus: &mut Bus) {
        if self.ppu_write_log_active {
            if let Some(logger) = bus.ppu_write_logger_mut() {
                let lines = logger.stop();
                eprintln!("nes-emu: PPU write logging stopped on exit ({lines} lines)");
            }
            self.ppu_write_log_active = false;
        }
    }
}

/// Handle M27 debugger hotkeys (F2/F3/N). Returns `true` if the key
/// was consumed. Extracted from `src/main.rs` so the binary stays under
/// the 400-line file-size limit.
///
/// - **F2**: toggle pause/resume.
/// - **F3**: toggle run-to-breakpoint mode.
/// - **N**: single-step (only when paused; otherwise a no-op with a
///   stderr hint).
///
/// F1 is handled separately in `main.rs` as a help overlay toggle.
pub fn handle_debugger_key(debugger: &mut CpuDebugger, key: Keycode) -> bool {
    match key {
        Keycode::F2 => {
            debugger.toggle_pause();
            let state = if debugger.is_paused() {
                "paused"
            } else {
                "resumed"
            };
            eprintln!("nes-emu: debugger {state}");
            true
        }
        Keycode::N => {
            if debugger.is_paused() {
                debugger.request_step();
            } else {
                eprintln!("nes-emu: N single-step ignored (debugger not paused; press F2 first)");
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
