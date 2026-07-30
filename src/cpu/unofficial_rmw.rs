//! Unofficial 6502 opcodes (M33) — RMW-combo + unstable store dispatch.
//!
//! This module holds the second half of the unofficial-opcode dispatch:
//! the read-modify-write combo opcodes (DCP, ISC, SLO, RLA, SRE, RRA) and
//! the unstable indexed stores (TAS, AHX, SHX, SHY). It is called from the
//! `_` fallback arm of [`super::unofficial::Cpu::execute_unofficial`] so
//! that the simple unofficial opcodes (NOP/LAX/SAX/immediate/LAS/KIL) and
//! the combo/stores share a single dispatch tree rooted at
//! `execute_unofficial`.
//!
//! Each RMW-combo opcode does: resolve the effective address (using the
//! RMW addressing helper that issues the dummy read), apply a value
//! transform via [`Cpu::rmw`], then apply a register operation using the
//! transformed value. The combo helpers (`dcp`, `isc`, `slo`, `rla`, `sre`,
//! `rra`) live here; the immediate-combined-op and unstable-store
//! *helpers* (`anc`, `alr`, `arr`, `axs`, `xaa`, `tas_store`, `ahx_store`,
//! `shy_store`, `quirk_addr_*`) live in [`super::unofficial_special`].
//!
//! See: https://www.nesdev.org/wiki/CPU_unofficial_opcodes

use super::{Cpu, Operand};
use crate::bus::Bus;

impl Cpu {
    /// Dispatch the RMW-combo and unstable-store unofficial opcodes.
    /// Called from the `_` arm of [`super::unofficial::Cpu::execute_unofficial`].
    pub fn execute_unofficial_rmw(&mut self, bus: &mut Bus, opcode: u8) -> u8 {
        match opcode {
            // ---- DCP (DEC then CMP) ------------------------------------
            0xC7 => {
                let op = self.op_zp(bus);
                self.dcp(bus, op);
                5
            }
            0xD7 => {
                let op = self.op_zp_x(bus);
                self.dcp(bus, op);
                6
            }
            0xCF => {
                let op = self.op_abs(bus);
                self.dcp(bus, op);
                6
            }
            0xDF => {
                let op = self.op_abs_x(bus);
                self.dcp(bus, op);
                7
            }
            0xDB => {
                let op = self.op_abs_y(bus);
                self.dcp(bus, op);
                7
            }
            0xC3 => {
                let op = self.op_ind_x(bus);
                self.dcp(bus, op);
                8
            }
            0xD3 => {
                let op = self.op_ind_y(bus);
                self.dcp(bus, op);
                8
            }

            // ---- ISC (INC then SBC) ------------------------------------
            0xE7 => {
                let op = self.op_zp(bus);
                self.isc(bus, op);
                5
            }
            0xF7 => {
                let op = self.op_zp_x(bus);
                self.isc(bus, op);
                6
            }
            0xEF => {
                let op = self.op_abs(bus);
                self.isc(bus, op);
                6
            }
            0xFF => {
                let op = self.op_abs_x(bus);
                self.isc(bus, op);
                7
            }
            0xFB => {
                let op = self.op_abs_y(bus);
                self.isc(bus, op);
                7
            }
            0xE3 => {
                let op = self.op_ind_x(bus);
                self.isc(bus, op);
                8
            }
            0xF3 => {
                let op = self.op_ind_y(bus);
                self.isc(bus, op);
                8
            }

            // ---- SLO (ASL then ORA) ------------------------------------
            0x07 => {
                let op = self.op_zp(bus);
                self.slo(bus, op);
                5
            }
            0x17 => {
                let op = self.op_zp_x(bus);
                self.slo(bus, op);
                6
            }
            0x0F => {
                let op = self.op_abs(bus);
                self.slo(bus, op);
                6
            }
            0x1F => {
                let op = self.op_abs_x(bus);
                self.slo(bus, op);
                7
            }
            0x1B => {
                let op = self.op_abs_y(bus);
                self.slo(bus, op);
                7
            }
            0x03 => {
                let op = self.op_ind_x(bus);
                self.slo(bus, op);
                8
            }
            0x13 => {
                let op = self.op_ind_y(bus);
                self.slo(bus, op);
                8
            }

            // ---- RLA (ROL then AND) ------------------------------------
            0x27 => {
                let op = self.op_zp(bus);
                self.rla(bus, op);
                5
            }
            0x37 => {
                let op = self.op_zp_x(bus);
                self.rla(bus, op);
                6
            }
            0x2F => {
                let op = self.op_abs(bus);
                self.rla(bus, op);
                6
            }
            0x3F => {
                let op = self.op_abs_x(bus);
                self.rla(bus, op);
                7
            }
            0x3B => {
                let op = self.op_abs_y(bus);
                self.rla(bus, op);
                7
            }
            0x23 => {
                let op = self.op_ind_x(bus);
                self.rla(bus, op);
                8
            }
            0x33 => {
                let op = self.op_ind_y(bus);
                self.rla(bus, op);
                8
            }

            // ---- SRE (LSR then EOR) ------------------------------------
            0x47 => {
                let op = self.op_zp(bus);
                self.sre(bus, op);
                5
            }
            0x57 => {
                let op = self.op_zp_x(bus);
                self.sre(bus, op);
                6
            }
            0x4F => {
                let op = self.op_abs(bus);
                self.sre(bus, op);
                6
            }
            0x5F => {
                let op = self.op_abs_x(bus);
                self.sre(bus, op);
                7
            }
            0x5B => {
                let op = self.op_abs_y(bus);
                self.sre(bus, op);
                7
            }
            0x43 => {
                let op = self.op_ind_x(bus);
                self.sre(bus, op);
                8
            }
            0x53 => {
                let op = self.op_ind_y(bus);
                self.sre(bus, op);
                8
            }

            // ---- RRA (ROR then ADC) ------------------------------------
            0x67 => {
                let op = self.op_zp(bus);
                self.rra(bus, op);
                5
            }
            0x77 => {
                let op = self.op_zp_x(bus);
                self.rra(bus, op);
                6
            }
            0x6F => {
                let op = self.op_abs(bus);
                self.rra(bus, op);
                6
            }
            0x7F => {
                let op = self.op_abs_x(bus);
                self.rra(bus, op);
                7
            }
            0x7B => {
                let op = self.op_abs_y(bus);
                self.rra(bus, op);
                7
            }
            0x63 => {
                let op = self.op_ind_x(bus);
                self.rra(bus, op);
                8
            }
            0x73 => {
                let op = self.op_ind_y(bus);
                self.rra(bus, op);
                8
            }

            // ---- TAS / SHS (SP = A & X; store SP & (H+1)) --------------
            0x9B => {
                let base = self.fetch_word(bus);
                self.tas_store(bus, base);
                5
            }

            // ---- AHX / SHA (store A & X & (H+1)) -----------------------
            0x9F => {
                let base = self.fetch_word(bus);
                self.ahx_store(bus, base, self.a & self.x);
                5
            }
            0x93 => {
                // (ind),Y: base pointer read from zero page.
                let base = self.am_indirect_y_base(bus);
                self.ahx_store(bus, base, self.a & self.x);
                6
            }

            // ---- SHX / SXA (store X & (H+1)) ---------------------------
            0x9E => {
                let base = self.fetch_word(bus);
                self.ahx_store(bus, base, self.x);
                5
            }

            // ---- SHY / SYA (store Y & (H+1)) ---------------------------
            0x9C => {
                let base = self.fetch_word(bus);
                self.shy_store(bus, base);
                5
            }

            // Defensive fallback: any byte not matching an official or
            // unofficial opcode is treated as a 2-cycle NOP. (With both
            // the official and unofficial sets above, every byte in
            // 0x00..=0xFF is covered, so this arm is unreachable in
            // practice — but the hot loop must never panic.)
            _ => 2,
        }
    }

    // ---- RMW-combo helpers --------------------------------------------
    //
    // Each combo opcode does: read memory, apply a value transform (which
    // sets carry for the shift/rotate variants), write the transformed
    // value back, then apply a register operation using the transformed
    // value. The [`Cpu::rmw`] helper handles read + transform + writeback +
    // set_nz(new); the register op that follows overwrites N/Z with the
    // register result (and, for ADC/SBC, overwrites C/V with the
    // arithmetic result — which is the correct hardware behavior, since
    // e.g. RRA's ROR-set carry is *consumed* by the following ADC and
    // replaced by the ADC's carry-out).

    /// DCP — decrement memory then CMP A against the new value.
    /// C/Z/N from the compare; the DEC itself sets no flags.
    fn dcp(&mut self, bus: &mut Bus, op: Operand) {
        self.rmw(bus, op, Self::dec_value);
        let m = self.read_operand(bus, op);
        self.cmp(self.a, m);
    }

    /// ISC — increment memory then SBC A against the new value.
    fn isc(&mut self, bus: &mut Bus, op: Operand) {
        self.rmw(bus, op, Self::inc_value);
        let m = self.read_operand(bus, op);
        self.sbc(m);
    }

    /// SLO — ASL memory then ORA A with the new value. C from the ASL is
    /// preserved (ORA does not touch C).
    fn slo(&mut self, bus: &mut Bus, op: Operand) {
        self.rmw(bus, op, Self::asl_value);
        let m = self.read_operand(bus, op);
        self.ora(m);
    }

    /// RLA — ROL memory then AND A with the new value. C from the ROL is
    /// preserved.
    fn rla(&mut self, bus: &mut Bus, op: Operand) {
        self.rmw(bus, op, Self::rol_value);
        let m = self.read_operand(bus, op);
        self.and(m);
    }

    /// SRE — LSR memory then EOR A with the new value. C from the LSR is
    /// preserved.
    fn sre(&mut self, bus: &mut Bus, op: Operand) {
        self.rmw(bus, op, Self::lsr_value);
        let m = self.read_operand(bus, op);
        self.eor(m);
    }

    /// RRA — ROR memory then ADC A with the new value. The ROR-set carry
    /// is consumed by the ADC as its carry-in; the ADC's carry-out
    /// replaces C.
    fn rra(&mut self, bus: &mut Bus, op: Operand) {
        self.rmw(bus, op, Self::ror_value);
        let m = self.read_operand(bus, op);
        self.adc(m);
    }
}
