//! 6502 CPU core — registers, status flags, and addressing modes.
//!
//! This module implements the MOS 6502 CPU as used in the NES (Ricoh 2A03).
//! The NES variant lacks decimal mode (the D flag exists but has no effect
//! on arithmetic). Milestone 4 covers the register file, flag helpers, and
//! all 13 addressing modes. Opcode implementations arrive in M5; interrupt
//! handling (NMI / IRQ / RESET) in M6.
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

/// Status flag bit masks for the P register.
///
/// See: https://www.nesdev.org/wiki/Status_flags
mod flags {
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

/// The 13 addressing modes supported by the 6502.
///
/// See: https://www.nesdev.org/wiki/CPU_addressing_modes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddrMode {
    /// Implied — no operand (e.g. `DEX`, `TAX`, `CLC`).
    Implied,
    /// Accumulator — operand is the A register (e.g. `ASL A`, `LSR A`).
    Accumulator,
    /// Immediate — operand byte follows the opcode (e.g. `LDA #$44`).
    Immediate,
    /// Zero-page — operand is a zero-page address (e.g. `LDA $44`).
    ZeroPage,
    /// Zero-page,X — `(operand + X) & 0xFF` (e.g. `LDA $44,X`).
    ZeroPageX,
    /// Zero-page,Y — `(operand + Y) & 0xFF` (e.g. `LDX $44,Y`).
    ZeroPageY,
    /// Absolute — 16-bit operand is the address (e.g. `LDA $4400`).
    Absolute,
    /// Absolute,X — `operand + X` (e.g. `LDA $4400,X`).
    AbsoluteX,
    /// Absolute,Y — `operand + Y` (e.g. `LDA $4400,Y`).
    AbsoluteY,
    /// Indirect — `JMP ($1000)`; reads a 16-bit pointer at the operand
    /// address (subject to the 6502 page-wrap bug).
    Indirect,
    /// Indirect,X — `(operand,X)`; zero-page indexed indirect
    /// (e.g. `LDA ($20,X)`).
    IndirectX,
    /// Indirect,Y — `(operand),Y`; indirect indexed (e.g. `LDA ($20),Y`).
    IndirectY,
    /// Relative — branch target = PC + signed offset (e.g. `BNE $10`).
    Relative,
}

/// The result of resolving an addressing mode for an instruction.
///
/// M5 opcode handlers match on this to decide where the operand lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operand {
    /// No operand — implied addressing (e.g. `DEX`, `TAX`).
    None,
    /// Operand is the accumulator register (e.g. `ASL A`).
    Accumulator,
    /// Operand lives at this memory address. Covers immediate (the operand
    /// byte's address), zero-page, absolute, all indexed/indirect modes, and
    /// relative (the branch target address).
    Address(u16),
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
    fn set_flag(&mut self, flag: u8, v: bool) {
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
    fn fetch_byte(&mut self, bus: &Bus) -> u8 {
        let b = bus.read(self.pc);
        self.pc = self.pc.wrapping_add(1);
        b
    }

    /// Read a little-endian 16-bit word at PC and advance PC by two.
    fn fetch_word(&mut self, bus: &Bus) -> u16 {
        let lo = self.fetch_byte(bus) as u16;
        let hi = self.fetch_byte(bus) as u16;
        lo | (hi << 8)
    }

    // ---- addressing mode resolution ----------------------------------

    /// Resolve the effective address for `mode`, advancing PC past any
    /// operand bytes. Returns `Operand::None` for implied,
    /// `Operand::Accumulator` for accumulator mode, and `Operand::Address`
    /// for all memory modes (including immediate and relative).
    pub fn resolve(&mut self, bus: &Bus, mode: AddrMode) -> Operand {
        match mode {
            AddrMode::Implied => Operand::None,
            AddrMode::Accumulator => Operand::Accumulator,
            AddrMode::Immediate => Operand::Address(self.am_immediate(bus)),
            AddrMode::ZeroPage => Operand::Address(self.am_zero_page(bus)),
            AddrMode::ZeroPageX => Operand::Address(self.am_zero_page_x(bus)),
            AddrMode::ZeroPageY => Operand::Address(self.am_zero_page_y(bus)),
            AddrMode::Absolute => Operand::Address(self.am_absolute(bus)),
            AddrMode::AbsoluteX => Operand::Address(self.am_absolute_x(bus)),
            AddrMode::AbsoluteY => Operand::Address(self.am_absolute_y(bus)),
            AddrMode::Indirect => Operand::Address(self.am_indirect(bus)),
            AddrMode::IndirectX => Operand::Address(self.am_indirect_x(bus)),
            AddrMode::IndirectY => Operand::Address(self.am_indirect_y(bus)),
            AddrMode::Relative => Operand::Address(self.am_relative(bus)),
        }
    }

    /// Immediate: the operand byte follows the opcode. The effective
    /// "address" is PC itself; PC then advances past the byte. The opcode
    /// handler reads the operand from this address.
    fn am_immediate(&mut self, _bus: &Bus) -> u16 {
        let addr = self.pc;
        self.pc = self.pc.wrapping_add(1);
        addr
    }

    /// Zero-page: the operand byte is a zero-page address (`$00..=$FF`).
    fn am_zero_page(&mut self, bus: &Bus) -> u16 {
        self.fetch_byte(bus) as u16
    }

    /// Zero-page,X: `(operand + X) & 0xFF` — wraps within the zero page.
    fn am_zero_page_x(&mut self, bus: &Bus) -> u16 {
        let base = self.fetch_byte(bus);
        base.wrapping_add(self.x) as u16
    }

    /// Zero-page,Y: `(operand + Y) & 0xFF` — wraps within the zero page.
    fn am_zero_page_y(&mut self, bus: &Bus) -> u16 {
        let base = self.fetch_byte(bus);
        base.wrapping_add(self.y) as u16
    }

    /// Absolute: the 16-bit operand is the effective address.
    fn am_absolute(&mut self, bus: &Bus) -> u16 {
        self.fetch_word(bus)
    }

    /// Absolute,X: `operand + X` (16-bit wraparound). Page-crossing penalty
    /// cycles are applied in M5's opcode dispatch, not here.
    fn am_absolute_x(&mut self, bus: &Bus) -> u16 {
        let base = self.fetch_word(bus);
        base.wrapping_add(self.x as u16)
    }

    /// Absolute,Y: `operand + Y` (16-bit wraparound).
    fn am_absolute_y(&mut self, bus: &Bus) -> u16 {
        let base = self.fetch_word(bus);
        base.wrapping_add(self.y as u16)
    }

    /// Indirect: `JMP ($1000)` — read a 16-bit pointer at the operand address.
    ///
    /// **6502 page-wrap bug**: if the pointer's low byte is `$FF`, the high
    /// byte is fetched from the *same* page's `$00` rather than the next
    /// page. E.g. `JMP ($30FF)` reads lo from `$30FF` and hi from `$3000`
    /// (not `$3100`).
    ///
    /// See: https://www.nesdev.org/6502.txt — "JMP indirect" bug.
    fn am_indirect(&mut self, bus: &Bus) -> u16 {
        let ptr = self.fetch_word(bus);
        let lo = bus.read(ptr);
        // High byte comes from the same page (low byte wraps within it).
        let hi_addr = (ptr & 0xFF00) | ((ptr as u8).wrapping_add(1) as u16);
        let hi = bus.read(hi_addr);
        (lo as u16) | ((hi as u16) << 8)
    }

    /// Indirect,X: `(operand,X)` — zero-page indexed indirect.
    ///
    /// The pointer location is `(operand + X) & 0xFF` (wraps in zero page).
    /// The 16-bit pointer is read from two consecutive zero-page addresses
    /// (the second also wraps within zero page).
    fn am_indirect_x(&mut self, bus: &Bus) -> u16 {
        let zp = self.fetch_byte(bus);
        let ptr = zp.wrapping_add(self.x);
        let lo = bus.read(ptr as u16);
        let hi = bus.read(ptr.wrapping_add(1) as u16);
        (lo as u16) | ((hi as u16) << 8)
    }

    /// Indirect,Y: `(operand),Y` — indirect indexed.
    ///
    /// The 16-bit base pointer is read from zero-page `operand` and
    /// `operand+1` (the second wraps within zero page), then Y is added
    /// (16-bit wraparound).
    fn am_indirect_y(&mut self, bus: &Bus) -> u16 {
        let zp = self.fetch_byte(bus);
        let lo = bus.read(zp as u16);
        let hi = bus.read(zp.wrapping_add(1) as u16);
        let base = (lo as u16) | ((hi as u16) << 8);
        base.wrapping_add(self.y as u16)
    }

    /// Relative: branch target = PC (after fetching the offset byte) plus
    /// the signed offset. The offset is treated as a signed 8-bit value.
    fn am_relative(&mut self, bus: &Bus) -> u16 {
        let offset = self.fetch_byte(bus) as i8;
        self.pc.wrapping_add(offset as u16)
    }
}

impl Default for Cpu {
    fn default() -> Self {
        Self::new()
    }
}
