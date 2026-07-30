//! 6502 addressing modes — effective address computation with optional dummy reads.
//!
//! See: https://www.nesdev.org/wiki/CPU_addressing_modes
//! See: https://www.nesdev.org/6502.txt — "Dummy reads" section.

use super::Cpu;
use crate::bus::Bus;

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

/// Dummy-read behaviour for indexed addressing modes.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Dummy {
    /// No dummy read — used by `resolve()` / disassembler.
    None,
    /// Dummy read on page cross only — used by read opcodes (LDA/AND/etc.).
    Read,
    /// Unconditional dummy read — used by RMW and store opcodes.
    Rmw,
}

/// The result of resolving an addressing mode for an instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operand {
    None,
    Accumulator,
    Address(u16),
}

impl Cpu {
    /// Resolve the effective address for `mode` (no dummy reads).
    pub fn resolve(&mut self, bus: &mut Bus, mode: AddrMode) -> Operand {
        match mode {
            AddrMode::Implied => Operand::None,
            AddrMode::Accumulator => Operand::Accumulator,
            AddrMode::Immediate => Operand::Address(self.am_immediate()),
            AddrMode::ZeroPage => Operand::Address(self.am_zero_page(bus)),
            AddrMode::ZeroPageX => Operand::Address(self.am_zero_page_x(bus, Dummy::None)),
            AddrMode::ZeroPageY => Operand::Address(self.am_zero_page_y(bus, Dummy::None)),
            AddrMode::Absolute => Operand::Address(self.am_absolute(bus)),
            AddrMode::AbsoluteX => Operand::Address(self.am_absolute_x(bus, Dummy::None).0),
            AddrMode::AbsoluteY => Operand::Address(self.am_absolute_y(bus, Dummy::None).0),
            AddrMode::Indirect => Operand::Address(self.am_indirect(bus)),
            AddrMode::IndirectX => Operand::Address(self.am_indirect_x(bus)),
            AddrMode::IndirectY => Operand::Address(self.am_indirect_y(bus, Dummy::None).0),
            AddrMode::Relative => Operand::Address(self.am_relative(bus)),
        }
    }

    /// Immediate: the operand byte follows the opcode. The effective
    /// "address" is PC itself; PC then advances past the byte. The opcode
    /// handler reads the operand from this address.
    #[inline]
    fn am_immediate(&mut self) -> u16 {
        let addr = self.pc;
        self.pc = self.pc.wrapping_add(1);
        addr
    }

    /// Zero-page: the operand byte is a zero-page address (`$00..=$FF`).
    #[inline]
    fn am_zero_page(&mut self, bus: &mut Bus) -> u16 {
        self.fetch_byte(bus) as u16
    }

    /// Zero-page,X: `(operand + X) & 0xFF` — wraps within the zero page.
    /// `dummy` controls whether a dummy read fires at the unindexed base.
    #[inline]
    pub(crate) fn am_zero_page_x(&mut self, bus: &mut Bus, dummy: Dummy) -> u16 {
        let base = self.fetch_byte(bus);
        if dummy != Dummy::None {
            let _ = bus.read(base as u16);
        }
        base.wrapping_add(self.x) as u16
    }

    /// Zero-page,Y: `(operand + Y) & 0xFF` — wraps within the zero page.
    /// `dummy` controls whether a dummy read fires at the unindexed base.
    #[inline]
    pub(crate) fn am_zero_page_y(&mut self, bus: &mut Bus, dummy: Dummy) -> u16 {
        let base = self.fetch_byte(bus);
        if dummy != Dummy::None {
            let _ = bus.read(base as u16);
        }
        base.wrapping_add(self.y) as u16
    }

    /// Absolute: the 16-bit operand is the effective address.
    #[inline]
    pub(crate) fn am_absolute(&mut self, bus: &mut Bus) -> u16 {
        self.fetch_word(bus)
    }

    /// Absolute,X: `operand + X` (16-bit wraparound). Returns `(addr, page_cross)`.
    /// `dummy` controls dummy-read behaviour for read vs RMW/store opcodes.
    #[inline]
    pub(crate) fn am_absolute_x(&mut self, bus: &mut Bus, dummy: Dummy) -> (u16, bool) {
        let base = self.fetch_word(bus);
        let eff = base.wrapping_add(self.x as u16);
        let page_cross = (base & 0xFF00) != (eff & 0xFF00);
        if matches!(dummy, Dummy::Read) && page_cross || matches!(dummy, Dummy::Rmw) {
            let _ = bus.read((base & 0xFF00) | (eff & 0x00FF));
        }
        (eff, page_cross)
    }

    /// Absolute,Y: `operand + Y` (16-bit wraparound). Returns `(addr, page_cross)`.
    #[inline]
    pub(crate) fn am_absolute_y(&mut self, bus: &mut Bus, dummy: Dummy) -> (u16, bool) {
        let base = self.fetch_word(bus);
        let eff = base.wrapping_add(self.y as u16);
        let page_cross = (base & 0xFF00) != (eff & 0xFF00);
        if matches!(dummy, Dummy::Read) && page_cross || matches!(dummy, Dummy::Rmw) {
            let _ = bus.read((base & 0xFF00) | (eff & 0x00FF));
        }
        (eff, page_cross)
    }

    /// Indirect: `JMP ($1000)` — read a 16-bit pointer at the operand address.
    ///
    /// **6502 page-wrap bug**: if the pointer's low byte is `$FF`, the high
    /// byte is fetched from the *same* page's `$00` rather than the next
    /// page. E.g. `JMP ($30FF)` reads lo from `$30FF` and hi from `$3000`
    /// (not `$3100`).
    ///
    /// See: https://www.nesdev.org/6502.txt — "JMP indirect" bug.
    #[inline]
    pub(crate) fn am_indirect(&mut self, bus: &mut Bus) -> u16 {
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
    #[inline]
    pub(crate) fn am_indirect_x(&mut self, bus: &mut Bus) -> u16 {
        let zp = self.fetch_byte(bus);
        let ptr = zp.wrapping_add(self.x);
        let lo = bus.read(ptr as u16);
        let hi = bus.read(ptr.wrapping_add(1) as u16);
        (lo as u16) | ((hi as u16) << 8)
    }

    /// Indirect,Y: `(operand),Y` — indirect indexed. Returns `(addr, page_cross)`.
    #[inline]
    pub(crate) fn am_indirect_y(&mut self, bus: &mut Bus, dummy: Dummy) -> (u16, bool) {
        let zp = self.fetch_byte(bus);
        let lo = bus.read(zp as u16);
        let hi = bus.read(zp.wrapping_add(1) as u16);
        let base = (lo as u16) | ((hi as u16) << 8);
        let eff = base.wrapping_add(self.y as u16);
        let page_cross = (base & 0xFF00) != (eff & 0xFF00);
        if matches!(dummy, Dummy::Read) && page_cross || matches!(dummy, Dummy::Rmw) {
            let _ = bus.read((base & 0xFF00) | (eff & 0x00FF));
        }
        (eff, page_cross)
    }

    /// Relative: branch target = PC (after fetching the offset byte) plus
    /// the signed offset. The offset is treated as a signed 8-bit value.
    #[inline]
    pub(crate) fn am_relative(&mut self, bus: &mut Bus) -> u16 {
        let offset = self.fetch_byte(bus) as i8;
        self.pc.wrapping_add(offset as u16)
    }
}
