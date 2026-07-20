//! Debug tools — CPU debugger, disassembler, breakpoints, and (in M28)
//! PPU / memory viewers and a trace logger.
//!
//! The debug layer is purely a *consumer* of the emulation core: it never
//! mutates emulator state on its own (the only mutation it performs is on
//! its own breakpoint / pause bookkeeping). All reads it makes against the
//! bus go through [`crate::bus::Bus::peek`], which is side-effect-free, so
//! attaching a debugger cannot perturb the emulated machine.
//!
//! See: https://www.nesdev.org/wiki/CPU_registers (status flag bit layout)
//! See: https://www.nesdev.org/6502.txt (opcode table used by the disassembler)

pub mod cpu_debugger;
pub mod disasm;
pub mod overlay;

pub use cpu_debugger::{Breakpoint, CpuDebugger, RegisterSnapshot};
pub use disasm::{disassemble_at, disassemble_window, DisassembledInstruction};
pub use overlay::print_debug_overlay;
