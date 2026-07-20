//! 6502 CPU core — registers, status flags, addressing modes, and the full
//! 151-official-opcode instruction set.
//!
//! This module implements the MOS 6502 CPU as used in the NES (Ricoh 2A03).
//! The NES variant lacks decimal mode (the D flag exists but has no effect
//! on arithmetic). M4 covered the register file, flag helpers, and all 13
//! addressing modes. M5 adds the complete official opcode set with correct
//! cycle costs (including page-crossing penalties). Interrupt handling
//! (NMI / IRQ / RESET) lands in M6.
//!
//! # Register file
//!
//! | Register | Width | Description                          |
//! |----------|-------|--------------------------------------|
//! | A        | 8     | Accumulator                          |
//! | X        | 8     | X index register                     |
//! | Y        | 8     | Y index register                     |
//! | SP       | 8     | Stack pointer (offset into `$0100`)  |
//! | PC       | 16    | Program counter                      |
//! | P        | 8     | Status flags (`N V - B D I Z C`)     |
//!
//! # Status flags (P register)
//!
//! | Bit | Flag | Meaning                                                    |
//! |-----|------|------------------------------------------------------------|
//! | 7   | N    | Negative (set to bit 7 of last result)                    |
//! | 6   | V    | Overflow (signed arithmetic overflow)                     |
//! | 5   | -    | Unused (always reads as 1; pushed as 1 on stack)          |
//! | 4   | B    | Break (set when pushed by software IRQ/BRK; 0 for NMI/IRQ)|
//! | 3   | D    | Decimal mode (no effect on NES; always 0)                 |
//! | 2   | I    | Interrupt disable                                         |
//! | 1   | Z    | Zero (set when last result was 0)                         |
//! | 0   | C    | Carry                                                     |
//!
//! See: https://www.nesdev.org/wiki/CPU_registers
//! See: https://www.nesdev.org/6502.txt

#![allow(dead_code)]

use crate::bus::Bus;

pub mod addressing;
pub mod opcodes;

pub use addressing::{AddrMode, Operand};

/// Status flag bit masks for the P register.
///
/// See: https://www.nesdev.org/wiki/Status_flags
pub mod flags {
    /// Carry (bit 0).
    pub const C: u8 = 0b0000_0001;
    /// Zero (bit 1).
    pub const Z: u8 = 0b0000_0010;
    /// Interrupt disable (bit 2).
    pub const I: u8 = 0b0000_0100;
    /// Decimal mode (bit 3 — no effect on the NES 2A03).
    pub const D: u8 = 0b0000_1000;
    /// Break (bit 4 — only meaningful in the stack-pushed status copy).
    pub const B: u8 = 0b0001_0000;
    /// Unused (bit 5 — always reads as 1, pushed as 1).
    pub const U: u8 = 0b0010_0000;
    /// Overflow (bit 6).
    pub const V: u8 = 0b0100_0000;
    /// Negative (bit 7).
    pub const N: u8 = 0b1000_0000;
}

/// The 6502 CPU.
pub struct Cpu {
    /// Accumulator.
    pub a: u8,
    /// X index register.
    pub x: u8,
    /// Y index register.
    pub y: u8,
    /// Stack pointer. The 6502 stack lives at `$0100..=$01FF`; SP is the low
    /// byte of the next free stack location.
    pub sp: u8,
    /// Program counter.
    pub pc: u16,
    /// Status flags (P register).
    pub status: u8,
}

impl Cpu {
    /// Construct a CPU in a simplified power-on state: registers zeroed,
    /// SP at `$FD`, and only the I and U flags set.
    ///
    /// The real RESET sequence loads PC from `$FFFC/$FFFD` — that is handled
    /// in M6 when interrupt handling lands.
    pub fn new() -> Self {
        Self {
            a: 0,
            x: 0,
            y: 0,
            sp: 0xFD,
            pc: 0,
            status: flags::U | flags::I,
        }
    }

    // ---- flag helpers -------------------------------------------------

    /// Get the carry flag (bit 0).
    pub fn carry(&self) -> bool {
        (self.status & flags::C) != 0
    }
    /// Set or clear the carry flag.
    pub fn set_carry(&mut self, v: bool) {
        self.set_flag(flags::C, v);
    }
    /// Get the zero flag (bit 1).
    pub fn zero(&self) -> bool {
        (self.status & flags::Z) != 0
    }
    /// Set or clear the zero flag.
    pub fn set_zero(&mut self, v: bool) {
        self.set_flag(flags::Z, v);
    }
    /// Get the interrupt-disable flag (bit 2).
    pub fn interrupt_disable(&self) -> bool {
        (self.status & flags::I) != 0
    }
    /// Set or clear the interrupt-disable flag.
    pub fn set_interrupt_disable(&mut self, v: bool) {
        self.set_flag(flags::I, v);
    }
    /// Get the decimal flag (bit 3 — no effect on the NES).
    pub fn decimal(&self) -> bool {
        (self.status & flags::D) != 0
    }
    /// Set or clear the decimal flag.
    pub fn set_decimal(&mut self, v: bool) {
        self.set_flag(flags::D, v);
    }
    /// Get the overflow flag (bit 6).
    pub fn overflow(&self) -> bool {
        (self.status & flags::V) != 0
    }
    /// Set or clear the overflow flag.
    pub fn set_overflow(&mut self, v: bool) {
        self.set_flag(flags::V, v);
    }
    /// Get the negative flag (bit 7).
    pub fn negative(&self) -> bool {
        (self.status & flags::N) != 0
    }
    /// Set or clear the negative flag.
    pub fn set_negative(&mut self, v: bool) {
        self.set_flag(flags::N, v);
    }

    /// Set or clear a flag bit.
    pub(crate) fn set_flag(&mut self, flag: u8, v: bool) {
        if v {
            self.status |= flag;
        } else {
            self.status &= !flag;
        }
    }

    /// Set the N and Z flags from a result byte.
    ///
    /// Used by loads, arithmetic, and logic instructions: N mirrors bit 7
    /// of the result, Z is set when the result is zero.
    pub fn set_nz(&mut self, value: u8) {
        self.set_zero(value == 0);
        self.set_negative((value & 0x80) != 0);
    }

    // ---- operand fetch ------------------------------------------------

    /// Read a byte at PC and advance PC by one.
    pub(crate) fn fetch_byte(&mut self, bus: &Bus) -> u8 {
        let b = bus.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        b
    }

    /// Read a little-endian 16-bit word at PC and advance PC by two.
    pub(crate) fn fetch_word(&mut self, bus: &Bus) -> u16 {
        let lo = self.fetch_byte(bus) as u16;
        let hi = self.fetch_byte(bus) as u16;
        lo | (hi << 8)
    }

    // ---- stack access -------------------------------------------------

    /// Push a byte onto the stack (decrements SP, writes at `$0100+SP`).
    pub(crate) fn push(&mut self, bus: &mut Bus, value: u8) {
        let addr = 0x0100 | self.sp as u16;
        bus.write(addr, value);
        self.sp = self.sp.wrapping_sub(1);
    }

    /// Pull a byte from the stack (increments SP, reads at `$0100+SP`).
    pub(crate) fn pull(&mut self, bus: &mut Bus) -> u8 {
        self.sp = self.sp.wrapping_add(1);
        let addr = 0x0100 | self.sp as u16;
        bus.read(addr)
    }

    /// Push the program counter (high byte first, then low).
    pub(crate) fn push_pc(&mut self, bus: &mut Bus, pc: u16) {
        self.push(bus, (pc >> 8) as u8);
        self.push(bus, (pc & 0xFF) as u8);
    }

    /// Pull a 16-bit program counter (low byte first, then high).
    pub(crate) fn pull_pc(&mut self, bus: &mut Bus) -> u16 {
        let lo = self.pull(bus) as u16;
        let hi = self.pull(bus) as u16;
        lo | (hi << 8)
    }

    /// Push the status register to the stack. The pushed copy has the B and
    /// U bits set per the 6502 convention; `with_break` controls the B bit
    /// (set for BRK/PHP, clear for NMI/IRQ/RTI pushes from hardware).
    pub(crate) fn push_status(&mut self, bus: &mut Bus, with_break: bool) {
        let mut p = self.status | flags::U;
        if with_break {
            p |= flags::B;
        } else {
            p &= !flags::B;
        }
        self.push(bus, p);
    }

    /// Pull the status register from the stack. The B bit is discarded (it
    /// is always re-read as 0 from a pull; only the U bit is forced to 1).
    pub(crate) fn pull_status(&mut self, bus: &mut Bus) {
        let p = self.pull(bus);
        self.status = (p & !flags::B) | flags::U;
    }

    // ---- operand read/write helpers ----------------------------------

    /// Read the operand value for an addressing result.
    pub(crate) fn read_operand(&self, bus: &Bus, op: Operand) -> u8 {
        match op {
            Operand::None => 0,
            Operand::Accumulator => self.a,
            Operand::Address(a) => bus.read(a),
        }
    }

    /// Write a value to the operand location.
    pub(crate) fn write_operand(&mut self, bus: &mut Bus, op: Operand, value: u8) {
        match op {
            Operand::None => {}
            Operand::Accumulator => self.a = value,
            Operand::Address(a) => bus.write(a, value),
        }
    }

    // ---- single-instruction step -------------------------------------

    /// Fetch and execute one instruction, returning the number of CPU
    /// cycles consumed (including page-crossing penalties).
    ///
    /// See: https://www.nesdev.org/6502.txt — cycle counts per opcode.
    pub fn step(&mut self, bus: &mut Bus) -> u8 {
        let opcode = self.fetch_byte(bus);
        self.execute(bus, opcode)
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}
