//! Debug tools — CPU debugger, disassembler, breakpoints, PPU / memory
//! viewers, and a trace logger.
//!
//! The debug layer is purely a *consumer* of the emulation core: it never
//! mutates emulator state on its own (the only mutation it performs is on
//! its own breakpoint / pause bookkeeping, and — for the memory viewer's
//! `poke` operation — explicit user-requested edits). All reads it makes
//! against the bus go through [`crate::bus::Bus::peek`] (CPU space) or the
//! PPU's immutable accessors + `Cartridge::read_chr` (PPU space), all of
//! which are side-effect-free, so attaching a debugger cannot perturb the
//! emulated machine.
//!
//! See: https://www.nesdev.org/wiki/CPU_registers (status flag bit layout)
//! See: https://www.nesdev.org/6502.txt (opcode table used by the disassembler)
//! See: https://www.nesdev.org/wiki/PPU_memory_layout (PPU viewer regions)
//! See: https://www.nesdev.org/wiki/Tracing (trace log format)

pub mod cpu_debugger;
pub mod disasm;
pub mod hotkeys;
pub mod memory_viewer;
pub mod overlay;
pub mod ppu_viewer;
pub mod trace_logger;

pub use cpu_debugger::{Breakpoint, CpuDebugger, RegisterSnapshot};
pub use disasm::{disassemble_at, disassemble_window, DisassembledInstruction};
pub use hotkeys::{handle_debugger_key, DebugHotkeys, DEFAULT_TRACE_LOG_PATH};
pub use memory_viewer::{MemoryRegion, MemoryViewer};
pub use overlay::print_debug_overlay;
pub use ppu_viewer::{PpuView, PpuViewer};
pub use trace_logger::TraceLogger;
