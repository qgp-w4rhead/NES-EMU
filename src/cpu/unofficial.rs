//! Unofficial / illegal 6502 opcodes (M33) — dispatch root.
//!
//! The 6502 has 151 official opcodes and 105 unofficial ("illegal") opcodes
//! that are the result of PLA decode don't-cares. Many commercial and
//! homebrew NES titles rely on them for compact code, so a complete
//! emulator must implement them.
//!
//! This module is the dispatch root: [`Cpu::execute_unofficial`] handles
//! the "simple" unofficial opcodes (NOP variants, LAX, SAX, the immediate
//! combined ops ANC/ALR/ARR/AXS/XAA, LAS, and KIL) and delegates the
//! RMW-combo opcodes (DCP/ISC/SLO/RLA/SRE/RRA) and the unstable indexed
//! stores (TAS/AHX/SHX/SHY) to [`super::unofficial_rmw::Cpu::execute_unofficial_rmw`].
//! The combo/imm/store *helpers* live in [`super::unofficial_special`].
//!
//! # Cycle counts
//!
//! Cycle counts follow the NESdev reference table at
//! <https://www.nesdev.org/wiki/CPU_unofficial_opcodes>. RMW-combo opcodes
//! use the RMW addressing helpers and so inherit the unconditional
//! dummy-read cycle (no page-cross penalty on the indexed variants — the
//! +1 is already paid). NOP absolute,X is the exception: it is a
//! *read*-style NOP and adds the +1 page-cross penalty.
//!
//! # Unstable opcodes
//!
//! XAA, LAS, TAS, AHX, SHX, and SHY are documented as "highly unstable" on
//! real silicon — their exact behavior varies by chip revision,
//! temperature, and prior bus state. The implementations here follow the
//! most commonly emulated model (per NESdev wiki) so that the majority of
//! test ROMs and commercial titles behave correctly. Determinism is
//! preserved: the same inputs always produce the same outputs on this
//! emulator.
//!
//! See: https://www.nesdev.org/wiki/CPU_unofficial_opcodes
//! See: https://www.nesdev.org/6502.txt — "Unofficial opcodes" section.

use super::Cpu;
use crate::bus::Bus;

impl Cpu {
    /// Decode and execute an unofficial opcode. Called from the fallback
    /// arm of [`super::opcodes::Cpu::execute`] for any opcode byte that does
    /// not match an official instruction. Returns the cycle count. Any byte
    /// that matches neither an official nor an unofficial opcode (there are
    /// none in the 0x00..=0xFF space once both sets are covered, but the
    /// fallback is kept defensive) returns a 2-cycle NOP so the hot loop
    /// never panics.
    ///
    /// See: https://www.nesdev.org/wiki/CPU_unofficial_opcodes
    pub fn execute_unofficial(&mut self, bus: &mut Bus, opcode: u8) -> u8 {
        match opcode {
            // ---- NOP variants ------------------------------------------
            // Implied NOPs (2 cycles): 0x1A, 0x3A, 0x5A, 0x7A, 0xDA, 0xFA.
            0x1A | 0x3A | 0x5A | 0x7A | 0xDA | 0xFA => 2,
            // Immediate NOPs (2 cycles): 0x80, 0x82, 0x89, 0xC2, 0xE2.
            0x80 | 0x82 | 0x89 | 0xC2 | 0xE2 => {
                let _ = self.fetch_byte(bus);
                2
            }
            // Zero-page NOPs (3 cycles): 0x04, 0x44, 0x64.
            0x04 | 0x44 | 0x64 => {
                let _ = self.fetch_byte(bus);
                3
            }
            // Zero-page,X NOPs (4 cycles): 0x14, 0x34, 0x54, 0x74, 0xD4, 0xF4.
            0x14 | 0x34 | 0x54 | 0x74 | 0xD4 | 0xF4 => {
                let _ = self.am_zero_page_x_dr(bus);
                4
            }
            // Absolute NOP (4 cycles): 0x0C.
            0x0C => {
                let _ = self.fetch_word(bus);
                4
            }
            // Absolute,X NOPs (4 + page-cross cycles): 0x1C, 0x3C, 0x5C,
            // 0x7C, 0xDC, 0xFC. These are *read*-style NOPs: the dummy read
            // fires on page cross and the +1 penalty applies.
            0x1C | 0x3C | 0x5C | 0x7C | 0xDC | 0xFC => {
                let (_, pc) = self.am_absolute_x_read(bus);
                4 + u8::from(pc)
            }

            // ---- LAX (load A and X) ------------------------------------
            0xA7 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPage);
                self.lax(self.read_operand(bus, op));
                3
            }
            0xB7 => {
                let a = self.am_zero_page_y_dr(bus);
                self.lax(bus.read(a));
                4
            }
            0xAF => {
                let op = self.resolve(bus, super::AddrMode::Absolute);
                self.lax(self.read_operand(bus, op));
                4
            }
            0xBF => {
                let (a, pc) = self.am_absolute_y_read(bus);
                self.lax(bus.read(a));
                4 + u8::from(pc)
            }
            0xA3 => {
                let a = self.am_indirect_x(bus);
                self.lax(bus.read(a));
                6
            }
            0xB3 => {
                let (a, pc) = self.am_indirect_y_read(bus);
                self.lax(bus.read(a));
                5 + u8::from(pc)
            }

            // ---- SAX (store A & X) -------------------------------------
            0x87 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPage);
                self.write_operand(bus, op, self.a & self.x);
                3
            }
            0x97 => {
                let a = self.am_zero_page_y_dr(bus);
                bus.write(a, self.a & self.x);
                4
            }
            0x8F => {
                let op = self.resolve(bus, super::AddrMode::Absolute);
                self.write_operand(bus, op, self.a & self.x);
                4
            }
            0x83 => {
                let a = self.am_indirect_x(bus);
                bus.write(a, self.a & self.x);
                6
            }

            // ---- ANC (AND then copy N to C) ----------------------------
            // 0x0B and 0x2B are both ANC. A = A & imm; C = N = bit 7 of A.
            0x0B | 0x2B => {
                let v = self.fetch_byte(bus);
                self.anc(v);
                2
            }

            // ---- ALR (AND then LSR) ------------------------------------
            // A = (A & imm) >> 1; C = old bit 0 of (A & imm).
            0x4B => {
                let v = self.fetch_byte(bus);
                self.alr(v);
                2
            }

            // ---- ARR (AND then ROR, special V/C) -----------------------
            0x6B => {
                let v = self.fetch_byte(bus);
                self.arr(v);
                2
            }

            // ---- AXS / SBX (subtract imm from A&X, store in X) ---------
            0xCB => {
                let v = self.fetch_byte(bus);
                self.axs(v);
                2
            }

            // ---- XAA (unstable: A = (A | magic) & X & imm) -------------
            0x8B => {
                let v = self.fetch_byte(bus);
                self.xaa(v);
                2
            }

            // ---- LAS / LAR (A = X = SP = M & SP) -----------------------
            0xBB => {
                let (a, pc) = self.am_absolute_y_read(bus);
                let v = bus.read(a);
                let result = v & self.sp;
                self.a = result;
                self.x = result;
                self.sp = result;
                self.set_nz(result);
                4 + u8::from(pc)
            }

            // ---- KIL / JAM / HLT (halt the CPU) ------------------------
            // 0x02, 0x12, 0x22, 0x32, 0x42, 0x52, 0x62, 0x72, 0x92, 0xB2,
            // 0xD2, 0xF2. The CPU freezes; only RESET revives it. We
            // consume 1 cycle and set the halted flag — `step` becomes a
            // 1-cycle no-op thereafter so the PPU/APU keep advancing.
            0x02 | 0x12 | 0x22 | 0x32 | 0x42 | 0x52 | 0x62 | 0x72 | 0x92 | 0xB2 | 0xD2 | 0xF2 => {
                self.halted = true;
                1
            }

            // ---- RMW-combo + unstable stores ---------------------------
            // DCP, ISC, SLO, RLA, SRE, RRA, TAS, AHX, SHX, SHY are handled
            // by the sibling dispatch in `unofficial_rmw.rs`.
            _ => self.execute_unofficial_rmw(bus, opcode),
        }
    }

    /// LAX — load A and X simultaneously from `m`. Sets N/Z from `m`.
    fn lax(&mut self, m: u8) {
        self.a = m;
        self.x = m;
        self.set_nz(m);
    }
}
