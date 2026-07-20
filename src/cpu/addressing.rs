//! 6502 addressing modes.
//!
//! All 13 addressing modes supported by the 6502. Each `am_*` helper
//! advances the program counter past any operand bytes and returns the
//! effective address together with a `page_crossed` flag. The flag is
//! `true` only for the indexed modes that incur a page-crossing cycle
//! penalty (`absolute,X`, `absolute,Y`, `indirect,Y`); it is `false` for
//! every other mode. Opcode handlers in [`super::opcodes`] use the flag to
//! add the +1 cycle penalty on read operations.
//!
//! # Dummy reads (M24)
//!
//! Several addressing modes perform a spurious read before the real access.
//! These *dummy reads* trigger side effects on bus devices with read-sensitive
//! registers (PPU `$2002` VBlank-clear, `$2004` OAMADDR-increment, `$2007`
//! VRAM-advance) and on certain mappers. The `_read` variants issue a dummy
//! read at the page-wrap address only when a page boundary is crossed; the
//! `_rmw` variants issue an unconditional dummy read (RMW and store opcodes
//! always pay the dummy-read cycle). Zero-page,X/Y variants (`_dr`) always
//! issue a dummy read at the unindexed base address.
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

/// The result of resolving an addressing mode for an instruction.
///
/// Opcode handlers match on this to decide where the operand lives.
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

impl Cpu {
    /// Resolve the effective address for `mode`, advancing PC past any
    /// operand bytes. Returns `Operand::None` for implied,
    /// `Operand::Accumulator` for accumulator mode, and `Operand::Address`
    /// for all memory modes (including immediate and relative).
    ///
    /// The page-cross flag from the indexed modes is discarded here; opcode
    /// handlers that need it call the `am_*_indexed` helpers directly.
    pub fn resolve(&mut self, bus: &mut Bus, mode: AddrMode) -> Operand {
        match mode {
            AddrMode::Implied => Operand::None,
            AddrMode::Accumulator => Operand::Accumulator,
            AddrMode::Immediate => Operand::Address(self.am_immediate()),
            AddrMode::ZeroPage => Operand::Address(self.am_zero_page(bus)),
            AddrMode::ZeroPageX => Operand::Address(self.am_zero_page_x(bus)),
            AddrMode::ZeroPageY => Operand::Address(self.am_zero_page_y(bus)),
            AddrMode::Absolute => Operand::Address(self.am_absolute(bus)),
            AddrMode::AbsoluteX => Operand::Address(self.am_absolute_x(bus).0),
            AddrMode::AbsoluteY => Operand::Address(self.am_absolute_y(bus).0),
            AddrMode::Indirect => Operand::Address(self.am_indirect(bus)),
            AddrMode::IndirectX => Operand::Address(self.am_indirect_x(bus)),
            AddrMode::IndirectY => Operand::Address(self.am_indirect_y(bus).0),
            AddrMode::Relative => Operand::Address(self.am_relative(bus)),
        }
    }

    /// Immediate: the operand byte follows the opcode. The effective
    /// "address" is PC itself; PC then advances past the byte. The opcode
    /// handler reads the operand from this address.
    fn am_immediate(&mut self) -> u16 {
        let addr = self.pc;
        self.pc = self.pc.wrapping_add(1);
        addr
    }

    /// Zero-page: the operand byte is a zero-page address (`$00..=$FF`).
    fn am_zero_page(&mut self, bus: &mut Bus) -> u16 {
        self.fetch_byte(bus) as u16
    }

    /// Zero-page,X: `(operand + X) & 0xFF` — wraps within the zero page.
    fn am_zero_page_x(&mut self, bus: &mut Bus) -> u16 {
        let base = self.fetch_byte(bus);
        base.wrapping_add(self.x) as u16
    }

    /// Zero-page,Y: `(operand + Y) & 0xFF` — wraps within the zero page.
    fn am_zero_page_y(&mut self, bus: &mut Bus) -> u16 {
        let base = self.fetch_byte(bus);
        base.wrapping_add(self.y) as u16
    }

    /// Absolute: the 16-bit operand is the effective address.
    pub(crate) fn am_absolute(&mut self, bus: &mut Bus) -> u16 {
        self.fetch_word(bus)
    }

    /// Absolute,X: `operand + X` (16-bit wraparound). Returns the effective
    /// address and whether the index crossed a page boundary (penalty cycle
    /// applied by read opcodes in [`super::opcodes`]).
    pub(crate) fn am_absolute_x(&mut self, bus: &mut Bus) -> (u16, bool) {
        let base = self.fetch_word(bus);
        let eff = base.wrapping_add(self.x as u16);
        (eff, (base & 0xFF00) != (eff & 0xFF00))
    }

    /// Absolute,Y: `operand + Y` (16-bit wraparound). Returns the effective
    /// address and page-cross flag.
    pub(crate) fn am_absolute_y(&mut self, bus: &mut Bus) -> (u16, bool) {
        let base = self.fetch_word(bus);
        let eff = base.wrapping_add(self.y as u16);
        (eff, (base & 0xFF00) != (eff & 0xFF00))
    }

    /// Indirect: `JMP ($1000)` — read a 16-bit pointer at the operand address.
    ///
    /// **6502 page-wrap bug**: if the pointer's low byte is `$FF`, the high
    /// byte is fetched from the *same* page's `$00` rather than the next
    /// page. E.g. `JMP ($30FF)` reads lo from `$30FF` and hi from `$3000`
    /// (not `$3100`).
    ///
    /// See: https://www.nesdev.org/6502.txt — "JMP indirect" bug.
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
    pub(crate) fn am_indirect_x(&mut self, bus: &mut Bus) -> u16 {
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
    /// (16-bit wraparound). Returns the effective address and page-cross
    /// flag.
    pub(crate) fn am_indirect_y(&mut self, bus: &mut Bus) -> (u16, bool) {
        let zp = self.fetch_byte(bus);
        let lo = bus.read(zp as u16);
        let hi = bus.read(zp.wrapping_add(1) as u16);
        let base = (lo as u16) | ((hi as u16) << 8);
        let eff = base.wrapping_add(self.y as u16);
        (eff, (base & 0xFF00) != (eff & 0xFF00))
    }

    /// Relative: branch target = PC (after fetching the offset byte) plus
    /// the signed offset. The offset is treated as a signed 8-bit value.
    pub(crate) fn am_relative(&mut self, bus: &mut Bus) -> u16 {
        let offset = self.fetch_byte(bus) as i8;
        self.pc.wrapping_add(offset as u16)
    }

    // ---- Dummy-read variants (M24) --------------------------------------
    //
    // The 6502 performs a spurious "dummy" read at the page-wrap address
    // before the real access on indexed addressing modes. For *read*
    // opcodes (LDA/LDX/LDY/AND/ORA/EOR/ADC/SBC/CMP) this dummy read only
    // occurs when a page boundary is crossed — the single read in the
    // no-cross case IS the real read. For *RMW* (INC/DEC/ASL/LSR/ROL/ROR)
    // and *store* (STA/STX/STY) opcodes the dummy read is unconditional.
    //
    // Zero-page,X/Y modes always perform a dummy read at the unindexed
    // base address (the zero-page operand byte before adding X/Y), because
    // the 6502 reads the base address as a stepping stone before computing
    // the wrapped indexed address.
    //
    // These dummy reads are observable on devices with read side-effects:
    // PPU PPUSTATUS ($2002) clears VBlank on read, OAMDATA ($2004)
    // increments OAMADDR, PPUDATA ($2007) advances the VRAM address. Some
    // mappers also have read side-effects. Issuing the dummy read here
    // ensures those side-effects fire at the correct cycle.
    //
    // See: https://www.nesdev.org/6502.txt — "Dummy reads"
    // See: https://www.nesdev.org/wiki/CPU_memory_map

    /// Zero-page,X with a dummy read at the unindexed base address.
    /// Used by all zero-page,X opcodes (reads, RMW, stores).
    pub(crate) fn am_zero_page_x_dr(&mut self, bus: &mut Bus) -> u16 {
        let base = self.fetch_byte(bus);
        // Dummy read at the base (unindexed) zero-page address.
        let _ = bus.read(base as u16);
        base.wrapping_add(self.x) as u16
    }

    /// Zero-page,Y with a dummy read at the unindexed base address.
    /// Used by all zero-page,Y opcodes (reads, RMW, stores).
    pub(crate) fn am_zero_page_y_dr(&mut self, bus: &mut Bus) -> u16 {
        let base = self.fetch_byte(bus);
        // Dummy read at the base (unindexed) zero-page address.
        let _ = bus.read(base as u16);
        base.wrapping_add(self.y) as u16
    }

    /// Absolute,X for *read* opcodes: issues a dummy read at the page-wrap
    /// address only when a page boundary is crossed. Returns the effective
    /// address and the page-cross flag (the flag still drives the +1 cycle
    /// penalty).
    pub(crate) fn am_absolute_x_read(&mut self, bus: &mut Bus) -> (u16, bool) {
        let base = self.fetch_word(bus);
        let eff = base.wrapping_add(self.x as u16);
        let page_cross = (base & 0xFF00) != (eff & 0xFF00);
        if page_cross {
            // Dummy read at the page-wrap address (high byte from base,
            // low byte from the indexed offset).
            let dummy_addr = (base & 0xFF00) | (eff & 0x00FF);
            let _ = bus.read(dummy_addr);
        }
        (eff, page_cross)
    }

    /// Absolute,X for *RMW and store* opcodes: unconditionally issues a
    /// dummy read at the page-wrap address. Returns the effective address.
    pub(crate) fn am_absolute_x_rmw(&mut self, bus: &mut Bus) -> u16 {
        let base = self.fetch_word(bus);
        let eff = base.wrapping_add(self.x as u16);
        // RMW and store opcodes always pay the dummy-read cycle, even
        // without a page cross.
        let dummy_addr = (base & 0xFF00) | (eff & 0x00FF);
        let _ = bus.read(dummy_addr);
        eff
    }

    /// Absolute,Y for *read* opcodes: dummy read on page cross. Returns
    /// the effective address and the page-cross flag.
    pub(crate) fn am_absolute_y_read(&mut self, bus: &mut Bus) -> (u16, bool) {
        let base = self.fetch_word(bus);
        let eff = base.wrapping_add(self.y as u16);
        let page_cross = (base & 0xFF00) != (eff & 0xFF00);
        if page_cross {
            let dummy_addr = (base & 0xFF00) | (eff & 0x00FF);
            let _ = bus.read(dummy_addr);
        }
        (eff, page_cross)
    }

    /// Absolute,Y for *store* opcodes: unconditional dummy read. Returns
    /// the effective address.
    pub(crate) fn am_absolute_y_rmw(&mut self, bus: &mut Bus) -> u16 {
        let base = self.fetch_word(bus);
        let eff = base.wrapping_add(self.y as u16);
        let dummy_addr = (base & 0xFF00) | (eff & 0x00FF);
        let _ = bus.read(dummy_addr);
        eff
    }

    /// Indirect,Y for *read* opcodes: dummy read on page cross. Returns
    /// the effective address and the page-cross flag.
    pub(crate) fn am_indirect_y_read(&mut self, bus: &mut Bus) -> (u16, bool) {
        let zp = self.fetch_byte(bus);
        let lo = bus.read(zp as u16);
        let hi = bus.read(zp.wrapping_add(1) as u16);
        let base = (lo as u16) | ((hi as u16) << 8);
        let eff = base.wrapping_add(self.y as u16);
        let page_cross = (base & 0xFF00) != (eff & 0xFF00);
        if page_cross {
            let dummy_addr = (base & 0xFF00) | (eff & 0x00FF);
            let _ = bus.read(dummy_addr);
        }
        (eff, page_cross)
    }

    /// Indirect,Y for *store* opcodes: unconditional dummy read. Returns
    /// the effective address.
    pub(crate) fn am_indirect_y_rmw(&mut self, bus: &mut Bus) -> u16 {
        let zp = self.fetch_byte(bus);
        let lo = bus.read(zp as u16);
        let hi = bus.read(zp.wrapping_add(1) as u16);
        let base = (lo as u16) | ((hi as u16) << 8);
        let eff = base.wrapping_add(self.y as u16);
        let dummy_addr = (base & 0xFF00) | (eff & 0x00FF);
        let _ = bus.read(dummy_addr);
        eff
    }
}
