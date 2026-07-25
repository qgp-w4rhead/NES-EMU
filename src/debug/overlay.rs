//! Console debug overlay — prints the register snapshot and a
//! disassembly window to stderr while the debugger is paused.
//!
//! This is the M27 "overlay": a text-only debug view that works without
//! a GUI toolkit. A graphical egui on-screen overlay lands in M28
//! alongside the PPU / memory viewers.
//!
//! The overlay performs only side-effect-free reads via
//! [`crate::bus::Bus::peek`] (through the disassembler) and immutable
//! borrows of the CPU, so printing it cannot perturb emulator state.

use crate::debug::{disassemble_window, CpuDebugger, RegisterSnapshot};
use crate::emulator::EmulatorState;

/// Number of instructions to show in the disassembly window (the current
/// instruction plus the next 7).
const DISASM_WINDOW_SIZE: usize = 8;

/// Print the M27 debug "overlay" to stderr: register snapshot + a
/// disassembly window of the current instruction and the next 7, with a
/// `>` marker on the current instruction. Also lists active breakpoints.
///
/// This is the console equivalent of a graphical debug overlay; a real
/// on-screen egui overlay lands in M28.
pub fn print_debug_overlay(emulator: &EmulatorState, debugger: &CpuDebugger) {
    let cpu = emulator.cpu();
    let bus = emulator.bus();
    let snap = RegisterSnapshot::from_cpu(cpu);
    eprintln!("--- debug overlay (M27) ---");
    eprintln!("{}", snap.to_line());
    let window = disassemble_window(bus, cpu.pc, DISASM_WINDOW_SIZE);
    for (i, instr) in window.iter().enumerate() {
        let marker = if i == 0 { ">" } else { " " };
        eprintln!(
            "{marker} ${:04X}: {:02X} {:02X} {:02X}  {}",
            instr.addr, instr.bytes[0], instr.bytes[1], instr.bytes[2], instr.text,
        );
    }
    if !debugger.breakpoints().is_empty() {
        eprintln!("breakpoints:");
        for bp in debugger.breakpoints() {
            eprintln!("  {bp}");
        }
    }
    eprintln!(
        "run-to-breakpoint: {}",
        if debugger.run_to_breakpoint() {
            "ON"
        } else {
            "OFF"
        }
    );
    eprintln!("F1=help  F2=resume  F3=breakpoint  N=step");
}
