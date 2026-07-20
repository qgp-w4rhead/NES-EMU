//! 6502 CPU core — registers, status flags, addressing modes, and the full
//! 151-official-opcode instruction set.
//!
//! This module implements the MOS 6502 CPU as used in the NES (Ricoh 2A03).
//! The NES variant lacks decimal mode (the D flag exists but has no effect
//! on arithmetic). M4 covered the register file, flag helpers, and all 13
//! addressing modes. M5 adds the complete official opcode set with correct
//! cycle costs (including page-crossing penalties). M6 adds interrupt
//! handling (NMI / IRQ / RESET) with correct vector fetches and stack
//! push sequence, plus `nestest.nes` automation-mode validation.
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
    /// Pending non-maskable interrupt. Set by an external device (the PPU
    /// VBlank in M10, or a test harness) and serviced at the start of the
    /// next [`Cpu::step`]. NMI is non-maskable and always takes priority
    /// over IRQ.
    pub nmi_pending: bool,
    /// Pending maskable interrupt. Set by an external device (APU frame
    /// counter, mapper IRQ, or a test harness) and serviced at the start of
    /// the next [`Cpu::step`] only when the I flag is clear.
    pub irq_pending: bool,
}

impl Cpu {
    /// Construct a CPU in a simplified power-on state: registers zeroed,
    /// SP at `$FD`, and only the I and U flags set.
    ///
    /// This leaves PC at `$0000`; callers should either invoke
    /// [`Cpu::reset`] to load PC from the RESET vector at `$FFFC/$FFFD`
    /// (the normal boot path) or set `pc` manually for test harnesses
    /// such as `nestest` automation mode (PC = `$C000`).
    pub fn new() -> Self {
        Self {
            a: 0,
            x: 0,
            y: 0,
            sp: 0xFD,
            pc: 0,
            status: flags::U | flags::I,
            nmi_pending: false,
            irq_pending: false,
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
    /// Before fetching the opcode, any pending interrupt is serviced:
    /// [`Cpu::nmi_pending`] takes priority (non-maskable), then
    /// [`Cpu::irq_pending`] is serviced only when the I flag is clear. An
    /// interrupt service consumes 7 cycles and pushes PC + status onto the
    /// stack before loading the new PC from the relevant vector.
    ///
    /// See: https://www.nesdev.org/6502.txt — cycle counts per opcode.
    /// See: https://www.nesdev.org/wiki/CPU_interrupts
    pub fn step(&mut self, bus: &mut Bus) -> u8 {
        // NMI is non-maskable and always wins over IRQ.
        if self.nmi_pending {
            self.nmi_pending = false;
            self.nmi(bus);
            return 7;
        }
        // IRQ is masked by the I flag.
        if self.irq_pending && !self.interrupt_disable() {
            self.irq_pending = false;
            self.irq(bus);
            return 7;
        }
        let opcode = self.fetch_byte(bus);
        self.execute(bus, opcode)
    }
}

/// Vector addresses for each interrupt type.
///
/// See: https://www.nesdev.org/wiki/CPU_interrupts
pub mod vectors {
    /// NMI vector (low byte at `$FFFA`, high byte at `$FFFB`).
    pub const NMI: u16 = 0xFFFA;
    /// RESET vector (low byte at `$FFFC`, high byte at `$FFFD`).
    pub const RESET: u16 = 0xFFFC;
    /// IRQ / BRK vector (low byte at `$FFFE`, high byte at `$FFFF`).
    pub const IRQ: u16 = 0xFFFE;
}

impl Cpu {
    /// Perform a non-maskable interrupt (NMI). Pushes PC and status (with
    /// B clear and U set) onto the stack, sets the I flag, and loads PC
    /// from the NMI vector at `$FFFA/$FFFB`. Consumes 7 CPU cycles.
    ///
    /// NMI is non-maskable — it is serviced regardless of the I flag. The
    /// caller is responsible for timing: on real hardware NMI is asserted
    /// by the PPU at VBlank and sampled at the end of an instruction.
    ///
    /// See: https://www.nesdev.org/wiki/CPU_interrupts#NMI
    pub fn nmi(&mut self, bus: &mut Bus) {
        self.service_interrupt(bus, vectors::NMI);
    }

    /// Perform a maskable interrupt (IRQ). Pushes PC and status (with B
    /// clear and U set) onto the stack, sets the I flag, and loads PC from
    /// the IRQ/BRK vector at `$FFFE/$FFFF`. Consumes 7 CPU cycles.
    ///
    /// The caller must check the I flag before calling this — on real
    /// hardware IRQ is masked when I is set. [`Cpu::step`] performs this
    /// check automatically; direct callers should too.
    ///
    /// See: https://www.nesdev.org/wiki/CPU_interrupts#IRQ
    pub fn irq(&mut self, bus: &mut Bus) {
        self.service_interrupt(bus, vectors::IRQ);
    }

    /// Perform the RESET sequence. Loads PC from the RESET vector at
    /// `$FFFC/$FFFD`, sets the I flag, and resets SP to `$FD`. Does NOT
    /// push anything onto the stack (RESET does not preserve a return
    /// address). Consumes 7 CPU cycles on real hardware.
    ///
    /// This is the normal boot path: after constructing a [`Cpu`] and
    /// loading a cartridge into the [`Bus`], call `reset` to point PC at
    /// the cartridge's entry point.
    ///
    /// See: https://www.nesdev.org/wiki/CPU_interrupts#RESET
    pub fn reset(&mut self, bus: &Bus) {
        // The 6502 RESET sequence decrements SP by 3 (without pushing) and
        // loads PC from the RESET vector. We model the end state directly:
        // SP = $FD, I set, U set, PC from $FFFC/$FFFD.
        self.sp = 0xFD;
        self.set_interrupt_disable(true);
        self.status |= flags::U;
        self.pc = self.read_vector(bus, vectors::RESET);
    }

    /// Shared body of NMI and IRQ: push PC (high then low), push status
    /// (B clear, U set), set I, load PC from `vector`.
    ///
    /// The pushed PC is the address of the *next* instruction (the PC value
    /// at the moment the interrupt is serviced, which is already past the
    /// last fully-executed instruction). RTI restores this value.
    fn service_interrupt(&mut self, bus: &mut Bus, vector: u16) {
        self.push_pc(bus, self.pc);
        // B is cleared for hardware interrupts (NMI/IRQ); U is always set
        // in the pushed status copy.
        self.push_status(bus, false);
        self.set_interrupt_disable(true);
        self.pc = self.read_vector(bus, vector);
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}
