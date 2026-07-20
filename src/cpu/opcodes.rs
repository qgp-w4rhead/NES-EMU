//! 6502 opcode dispatch and instruction implementations — all 151 official
//! opcodes.
//!
//! [`Cpu::execute`] is the per-opcode dispatch table. Each arm resolves the
//! addressing mode, performs the operation, and returns the cycle count
//! (including page-crossing penalties). Handler methods are grouped by
//! category in `impl Cpu` blocks below:
//!
//! - loads / stores / register transfers
//! - stack operations
//! - arithmetic & logic (`ADC`, `SBC`, `AND`, `ORA`, `EOR`, `BIT`)
//! - compares (`CMP`, `CPX`, `CPY`)
//! - read-modify-write shifts/rotates (`ASL`, `LSR`, `ROL`, `ROR`)
//! - inc/dec (`INC`, `DEC`, `INX`, `INY`, `DEX`, `DEY`)
//! - branches (`BPL`, `BMI`, `BVC`, `BVS`, `BCC`, `BCS`, `BNE`, `BEQ`)
//! - jumps / subroutines (`JMP`, `JSR`, `RTS`, `RTI`, `BRK`)
//! - flag ops (`CLC`, `SEC`, `CLI`, `SEI`, `CLV`, `CLD`, `SED`)
//! - `NOP`
//!
//! # Cycle counts
//!
//! Cycle counts follow the 6502 reference (see
//! <https://www.nesdev.org/6502.txt> and the opcode table at
//! <https://www.nesdev.org/wiki/CPU_unofficial_opcodes> — official column).
//! Page-crossing penalties (+1 cycle) apply to *read* operations using
//! `absolute,X`, `absolute,Y`, or `indirect,Y` addressing. Store (`STA`,
//! `STX`, `STY`) and read-modify-write operations do not add a page-cross
//! penalty (RMW always pays the +1 via the dummy read regardless of page
//! cross). Branches add +1 cycle when taken and another +1 when the target
//! is on a different page than the instruction following the branch.
//!
//! # Dummy reads
//!
//! Several addressing modes perform a spurious read before the real access
//! (indexed reads on page cross, RMW reads). These have no observable
//! effect until bus devices with read side-effects (PPU registers, certain
//! mappers) are introduced in later milestones; the cycle counts here
//! already account for them. Actual dummy read bus cycles will be issued
//! when those devices land.

use super::flags;
use super::{Cpu, Operand};
use crate::bus::Bus;

impl Cpu {
    /// Decode and execute a single opcode. Returns the number of CPU cycles
    /// consumed (including page-crossing penalties). The opcode byte itself
    /// has already been fetched (and PC advanced past it) by `step`.
    pub fn execute(&mut self, bus: &mut Bus, opcode: u8) -> u8 {
        match opcode {
            // ---- LDA ----------------------------------------------------
            0xA9 => {
                let v = self.fetch_byte(bus);
                self.lda(v);
                2
            } // imm
            0xA5 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPage);
                let v = self.read_operand(bus, op);
                self.lda(v);
                3
            }
            0xB5 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPageX);
                let v = self.read_operand(bus, op);
                self.lda(v);
                4
            }
            0xAD => {
                let op = self.resolve(bus, super::AddrMode::Absolute);
                let v = self.read_operand(bus, op);
                self.lda(v);
                4
            }
            0xBD => {
                let (a, pc) = self.am_absolute_x(bus);
                let v = bus.read(a);
                self.lda(v);
                4 + u8::from(pc)
            }
            0xB9 => {
                let (a, pc) = self.am_absolute_y(bus);
                let v = bus.read(a);
                self.lda(v);
                4 + u8::from(pc)
            }
            0xA1 => {
                let a = self.am_indirect_x(bus);
                let v = bus.read(a);
                self.lda(v);
                6
            }
            0xB1 => {
                let (a, pc) = self.am_indirect_y(bus);
                let v = bus.read(a);
                self.lda(v);
                5 + u8::from(pc)
            }

            // ---- LDX ----------------------------------------------------
            0xA2 => {
                let v = self.fetch_byte(bus);
                self.ldx(v);
                2
            } // imm
            0xA6 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPage);
                let v = self.read_operand(bus, op);
                self.ldx(v);
                3
            }
            0xB6 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPageY);
                let v = self.read_operand(bus, op);
                self.ldx(v);
                4
            }
            0xAE => {
                let op = self.resolve(bus, super::AddrMode::Absolute);
                let v = self.read_operand(bus, op);
                self.ldx(v);
                4
            }
            0xBE => {
                let (a, pc) = self.am_absolute_y(bus);
                let v = bus.read(a);
                self.ldx(v);
                4 + u8::from(pc)
            }

            // ---- LDY ----------------------------------------------------
            0xA0 => {
                let v = self.fetch_byte(bus);
                self.ldy(v);
                2
            } // imm
            0xA4 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPage);
                let v = self.read_operand(bus, op);
                self.ldy(v);
                3
            }
            0xB4 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPageX);
                let v = self.read_operand(bus, op);
                self.ldy(v);
                4
            }
            0xAC => {
                let op = self.resolve(bus, super::AddrMode::Absolute);
                let v = self.read_operand(bus, op);
                self.ldy(v);
                4
            }
            0xBC => {
                let (a, pc) = self.am_absolute_x(bus);
                let v = bus.read(a);
                self.ldy(v);
                4 + u8::from(pc)
            }

            // ---- STA ----------------------------------------------------
            0x85 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPage);
                self.write_operand(bus, op, self.a);
                3
            }
            0x95 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPageX);
                self.write_operand(bus, op, self.a);
                4
            }
            0x8D => {
                let op = self.resolve(bus, super::AddrMode::Absolute);
                self.write_operand(bus, op, self.a);
                4
            }
            0x9D => {
                let (a, _) = self.am_absolute_x(bus);
                bus.write(a, self.a);
                5
            } // no page-cross penalty on store
            0x99 => {
                let (a, _) = self.am_absolute_y(bus);
                bus.write(a, self.a);
                5
            }
            0x81 => {
                let a = self.am_indirect_x(bus);
                bus.write(a, self.a);
                6
            }
            0x91 => {
                let (a, _) = self.am_indirect_y(bus);
                bus.write(a, self.a);
                6
            }

            // ---- STX ----------------------------------------------------
            0x86 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPage);
                self.write_operand(bus, op, self.x);
                3
            }
            0x96 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPageY);
                self.write_operand(bus, op, self.x);
                4
            }
            0x8E => {
                let op = self.resolve(bus, super::AddrMode::Absolute);
                self.write_operand(bus, op, self.x);
                4
            }

            // ---- STY ----------------------------------------------------
            0x84 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPage);
                self.write_operand(bus, op, self.y);
                3
            }
            0x94 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPageX);
                self.write_operand(bus, op, self.y);
                4
            }
            0x8C => {
                let op = self.resolve(bus, super::AddrMode::Absolute);
                self.write_operand(bus, op, self.y);
                4
            }

            // ---- register transfers ------------------------------------
            0xAA => {
                self.tax();
                2
            } // TAX
            0xA8 => {
                self.tay();
                2
            } // TAY
            0x8A => {
                self.txa();
                2
            } // TXA
            0x98 => {
                self.tya();
                2
            } // TYA
            0xBA => {
                self.tsx();
                2
            } // TSX
            0x9A => {
                self.set_sp_from_x();
                2
            } // TXS (no flags)

            // ---- stack --------------------------------------------------
            0x48 => {
                self.push(bus, self.a);
                3
            } // PHA
            0x08 => {
                self.push_status(bus, true);
                3
            } // PHP (B set)
            0x68 => {
                let v = self.pull(bus);
                self.lda(v);
                4
            } // PLA
            0x28 => {
                self.pull_status(bus);
                4
            } // PLP

            // ---- logic --------------------------------------------------
            0x29 => {
                let v = self.fetch_byte(bus);
                self.and(v);
                2
            } // AND imm
            0x25 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPage);
                self.and(self.read_operand(bus, op));
                3
            }
            0x35 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPageX);
                self.and(self.read_operand(bus, op));
                4
            }
            0x2D => {
                let op = self.resolve(bus, super::AddrMode::Absolute);
                self.and(self.read_operand(bus, op));
                4
            }
            0x3D => {
                let (a, pc) = self.am_absolute_x(bus);
                self.and(bus.read(a));
                4 + u8::from(pc)
            }
            0x39 => {
                let (a, pc) = self.am_absolute_y(bus);
                self.and(bus.read(a));
                4 + u8::from(pc)
            }
            0x21 => {
                let a = self.am_indirect_x(bus);
                self.and(bus.read(a));
                6
            }
            0x31 => {
                let (a, pc) = self.am_indirect_y(bus);
                self.and(bus.read(a));
                5 + u8::from(pc)
            }

            0x09 => {
                let v = self.fetch_byte(bus);
                self.ora(v);
                2
            } // ORA imm
            0x05 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPage);
                self.ora(self.read_operand(bus, op));
                3
            }
            0x15 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPageX);
                self.ora(self.read_operand(bus, op));
                4
            }
            0x0D => {
                let op = self.resolve(bus, super::AddrMode::Absolute);
                self.ora(self.read_operand(bus, op));
                4
            }
            0x1D => {
                let (a, pc) = self.am_absolute_x(bus);
                self.ora(bus.read(a));
                4 + u8::from(pc)
            }
            0x19 => {
                let (a, pc) = self.am_absolute_y(bus);
                self.ora(bus.read(a));
                4 + u8::from(pc)
            }
            0x01 => {
                let a = self.am_indirect_x(bus);
                self.ora(bus.read(a));
                6
            }
            0x11 => {
                let (a, pc) = self.am_indirect_y(bus);
                self.ora(bus.read(a));
                5 + u8::from(pc)
            }

            0x49 => {
                let v = self.fetch_byte(bus);
                self.eor(v);
                2
            } // EOR imm
            0x45 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPage);
                self.eor(self.read_operand(bus, op));
                3
            }
            0x55 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPageX);
                self.eor(self.read_operand(bus, op));
                4
            }
            0x4D => {
                let op = self.resolve(bus, super::AddrMode::Absolute);
                self.eor(self.read_operand(bus, op));
                4
            }
            0x5D => {
                let (a, pc) = self.am_absolute_x(bus);
                self.eor(bus.read(a));
                4 + u8::from(pc)
            }
            0x59 => {
                let (a, pc) = self.am_absolute_y(bus);
                self.eor(bus.read(a));
                4 + u8::from(pc)
            }
            0x41 => {
                let a = self.am_indirect_x(bus);
                self.eor(bus.read(a));
                6
            }
            0x51 => {
                let (a, pc) = self.am_indirect_y(bus);
                self.eor(bus.read(a));
                5 + u8::from(pc)
            }

            // ---- BIT ----------------------------------------------------
            0x24 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPage);
                self.bit(self.read_operand(bus, op));
                3
            }
            0x2C => {
                let op = self.resolve(bus, super::AddrMode::Absolute);
                self.bit(self.read_operand(bus, op));
                4
            }

            // ---- ADC ----------------------------------------------------
            0x69 => {
                let v = self.fetch_byte(bus);
                self.adc(v);
                2
            } // imm
            0x65 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPage);
                self.adc(self.read_operand(bus, op));
                3
            }
            0x75 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPageX);
                self.adc(self.read_operand(bus, op));
                4
            }
            0x6D => {
                let op = self.resolve(bus, super::AddrMode::Absolute);
                self.adc(self.read_operand(bus, op));
                4
            }
            0x7D => {
                let (a, pc) = self.am_absolute_x(bus);
                self.adc(bus.read(a));
                4 + u8::from(pc)
            }
            0x79 => {
                let (a, pc) = self.am_absolute_y(bus);
                self.adc(bus.read(a));
                4 + u8::from(pc)
            }
            0x61 => {
                let a = self.am_indirect_x(bus);
                self.adc(bus.read(a));
                6
            }
            0x71 => {
                let (a, pc) = self.am_indirect_y(bus);
                self.adc(bus.read(a));
                5 + u8::from(pc)
            }

            // ---- SBC ----------------------------------------------------
            0xE9 => {
                let v = self.fetch_byte(bus);
                self.sbc(v);
                2
            } // imm
            0xE5 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPage);
                self.sbc(self.read_operand(bus, op));
                3
            }
            0xF5 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPageX);
                self.sbc(self.read_operand(bus, op));
                4
            }
            0xED => {
                let op = self.resolve(bus, super::AddrMode::Absolute);
                self.sbc(self.read_operand(bus, op));
                4
            }
            0xFD => {
                let (a, pc) = self.am_absolute_x(bus);
                self.sbc(bus.read(a));
                4 + u8::from(pc)
            }
            0xF9 => {
                let (a, pc) = self.am_absolute_y(bus);
                self.sbc(bus.read(a));
                4 + u8::from(pc)
            }
            0xE1 => {
                let a = self.am_indirect_x(bus);
                self.sbc(bus.read(a));
                6
            }
            0xF1 => {
                let (a, pc) = self.am_indirect_y(bus);
                self.sbc(bus.read(a));
                5 + u8::from(pc)
            }

            // ---- CMP ----------------------------------------------------
            0xC9 => {
                let v = self.fetch_byte(bus);
                self.cmp(self.a, v);
                2
            } // imm
            0xC5 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPage);
                self.cmp(self.a, self.read_operand(bus, op));
                3
            }
            0xD5 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPageX);
                self.cmp(self.a, self.read_operand(bus, op));
                4
            }
            0xCD => {
                let op = self.resolve(bus, super::AddrMode::Absolute);
                self.cmp(self.a, self.read_operand(bus, op));
                4
            }
            0xDD => {
                let (a, pc) = self.am_absolute_x(bus);
                self.cmp(self.a, bus.read(a));
                4 + u8::from(pc)
            }
            0xD9 => {
                let (a, pc) = self.am_absolute_y(bus);
                self.cmp(self.a, bus.read(a));
                4 + u8::from(pc)
            }
            0xC1 => {
                let a = self.am_indirect_x(bus);
                self.cmp(self.a, bus.read(a));
                6
            }
            0xD1 => {
                let (a, pc) = self.am_indirect_y(bus);
                self.cmp(self.a, bus.read(a));
                5 + u8::from(pc)
            }

            // ---- CPX / CPY ----------------------------------------------
            0xE0 => {
                let v = self.fetch_byte(bus);
                self.cmp(self.x, v);
                2
            } // CPX imm
            0xE4 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPage);
                self.cmp(self.x, self.read_operand(bus, op));
                3
            }
            0xEC => {
                let op = self.resolve(bus, super::AddrMode::Absolute);
                self.cmp(self.x, self.read_operand(bus, op));
                4
            }

            0xC0 => {
                let v = self.fetch_byte(bus);
                self.cmp(self.y, v);
                2
            } // CPY imm
            0xC4 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPage);
                self.cmp(self.y, self.read_operand(bus, op));
                3
            }
            0xCC => {
                let op = self.resolve(bus, super::AddrMode::Absolute);
                self.cmp(self.y, self.read_operand(bus, op));
                4
            }

            // ---- INC / DEC memory --------------------------------------
            0xE6 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPage);
                self.rmw(bus, op, Self::inc_value);
                5
            }
            0xF6 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPageX);
                self.rmw(bus, op, Self::inc_value);
                6
            }
            0xEE => {
                let op = self.resolve(bus, super::AddrMode::Absolute);
                self.rmw(bus, op, Self::inc_value);
                6
            }
            0xFE => {
                let (a, _) = self.am_absolute_x(bus);
                let op = Operand::Address(a);
                self.rmw(bus, op, Self::inc_value);
                7
            }

            0xC6 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPage);
                self.rmw(bus, op, Self::dec_value);
                5
            }
            0xD6 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPageX);
                self.rmw(bus, op, Self::dec_value);
                6
            }
            0xCE => {
                let op = self.resolve(bus, super::AddrMode::Absolute);
                self.rmw(bus, op, Self::dec_value);
                6
            }
            0xDE => {
                let (a, _) = self.am_absolute_x(bus);
                let op = Operand::Address(a);
                self.rmw(bus, op, Self::dec_value);
                7
            }

            // ---- INX/INY/DEX/DEY ---------------------------------------
            0xE8 => {
                self.x = self.x.wrapping_add(1);
                self.set_nz(self.x);
                2
            } // INX
            0xC8 => {
                self.y = self.y.wrapping_add(1);
                self.set_nz(self.y);
                2
            } // INY
            0xCA => {
                self.x = self.x.wrapping_sub(1);
                self.set_nz(self.x);
                2
            } // DEX
            0x88 => {
                self.y = self.y.wrapping_sub(1);
                self.set_nz(self.y);
                2
            } // DEY

            // ---- ASL ----------------------------------------------------
            0x0A => {
                self.a = self.asl_value(self.a);
                self.set_nz(self.a);
                2
            } // accumulator
            0x06 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPage);
                self.rmw(bus, op, Self::asl_value);
                5
            }
            0x16 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPageX);
                self.rmw(bus, op, Self::asl_value);
                6
            }
            0x0E => {
                let op = self.resolve(bus, super::AddrMode::Absolute);
                self.rmw(bus, op, Self::asl_value);
                6
            }
            0x1E => {
                let (a, _) = self.am_absolute_x(bus);
                let op = Operand::Address(a);
                self.rmw(bus, op, Self::asl_value);
                7
            }

            // ---- LSR ----------------------------------------------------
            0x4A => {
                self.a = self.lsr_value(self.a);
                self.set_nz(self.a);
                2
            } // accumulator
            0x46 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPage);
                self.rmw(bus, op, Self::lsr_value);
                5
            }
            0x56 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPageX);
                self.rmw(bus, op, Self::lsr_value);
                6
            }
            0x4E => {
                let op = self.resolve(bus, super::AddrMode::Absolute);
                self.rmw(bus, op, Self::lsr_value);
                6
            }
            0x5E => {
                let (a, _) = self.am_absolute_x(bus);
                let op = Operand::Address(a);
                self.rmw(bus, op, Self::lsr_value);
                7
            }

            // ---- ROL ----------------------------------------------------
            0x2A => {
                self.a = self.rol_value(self.a);
                self.set_nz(self.a);
                2
            } // accumulator
            0x26 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPage);
                self.rmw(bus, op, Self::rol_value);
                5
            }
            0x36 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPageX);
                self.rmw(bus, op, Self::rol_value);
                6
            }
            0x2E => {
                let op = self.resolve(bus, super::AddrMode::Absolute);
                self.rmw(bus, op, Self::rol_value);
                6
            }
            0x3E => {
                let (a, _) = self.am_absolute_x(bus);
                let op = Operand::Address(a);
                self.rmw(bus, op, Self::rol_value);
                7
            }

            // ---- ROR ----------------------------------------------------
            0x6A => {
                self.a = self.ror_value(self.a);
                self.set_nz(self.a);
                2
            } // accumulator
            0x66 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPage);
                self.rmw(bus, op, Self::ror_value);
                5
            }
            0x76 => {
                let op = self.resolve(bus, super::AddrMode::ZeroPageX);
                self.rmw(bus, op, Self::ror_value);
                6
            }
            0x6E => {
                let op = self.resolve(bus, super::AddrMode::Absolute);
                self.rmw(bus, op, Self::ror_value);
                6
            }
            0x7E => {
                let (a, _) = self.am_absolute_x(bus);
                let op = Operand::Address(a);
                self.rmw(bus, op, Self::ror_value);
                7
            }

            // ---- branches ----------------------------------------------
            0x10 => self.branch(bus, false, flags::N), // BPL
            0x30 => self.branch(bus, true, flags::N),  // BMI
            0x50 => self.branch(bus, false, flags::V), // BVC
            0x70 => self.branch(bus, true, flags::V),  // BVS
            0x90 => self.branch(bus, false, flags::C), // BCC
            0xB0 => self.branch(bus, true, flags::C),  // BCS
            0xD0 => self.branch(bus, false, flags::Z), // BNE
            0xF0 => self.branch(bus, true, flags::Z),  // BEQ

            // ---- JMP / JSR / RTS / RTI / BRK ---------------------------
            0x4C => {
                let a = self.am_absolute(bus);
                self.pc = a;
                3
            } // JMP abs
            0x6C => {
                let a = self.am_indirect(bus);
                self.pc = a;
                5
            } // JMP (ind)
            0x20 => {
                // JSR abs
                let target = self.fetch_word(bus);
                // Push PC of last byte of instruction (target-1) per 6502 RTS semantics.
                self.push_pc(bus, self.pc.wrapping_sub(1));
                self.pc = target;
                6
            }
            0x60 => {
                // RTS
                let ret = self.pull_pc(bus).wrapping_add(1);
                self.pc = ret;
                6
            }
            0x40 => {
                // RTI
                self.pull_status(bus);
                self.pc = self.pull_pc(bus);
                6
            }
            0x00 => {
                // BRK
                // BRK is a 2-byte instruction (the padding byte is fetched).
                let _pad = self.fetch_byte(bus);
                self.push_pc(bus, self.pc);
                self.push_status(bus, true);
                self.set_interrupt_disable(true);
                self.pc = self.read_vector(bus, 0xFFFE);
                7
            }

            // ---- flag operations ---------------------------------------
            0x18 => {
                self.set_carry(false);
                2
            } // CLC
            0x38 => {
                self.set_carry(true);
                2
            } // SEC
            0x58 => {
                self.set_interrupt_disable(false);
                2
            } // CLI
            0x78 => {
                self.set_interrupt_disable(true);
                2
            } // SEI
            0xB8 => {
                self.set_overflow(false);
                2
            } // CLV
            0xD8 => {
                self.set_decimal(false);
                2
            } // CLD
            0xF8 => {
                self.set_decimal(true);
                2
            } // SED

            // ---- NOP ----------------------------------------------------
            0xEA => 2,

            // Unknown / unofficial opcodes are not implemented in Tier 1.
            // Treat as a 2-cycle NOP rather than panicking (the hot loop
            // must never panic). M6+ may surface this as a loadable error.
            _ => 2,
        }
    }

    // ---- helper: read a 16-bit interrupt vector ----------------------

    /// Read a little-endian 16-bit vector from CPU address space (used by
    /// BRK here; NMI/IRQ/RESET reuse this in M6).
    fn read_vector(&self, bus: &Bus, addr: u16) -> u16 {
        let lo = bus.read(addr) as u16;
        let hi = bus.read(addr.wrapping_add(1)) as u16;
        lo | (hi << 8)
    }

    // ---- helper: read-modify-write -----------------------------------

    /// Read the operand, apply `f` to produce a new value, write it back,
    /// and update N/Z flags from the new value. Used by `INC`, `DEC`,
    /// `ASL`, `LSR`, `ROL`, `ROR` for memory operands (accumulator forms
    /// bypass this and operate on `self.a` directly).
    fn rmw<F>(&mut self, bus: &mut Bus, op: Operand, f: F)
    where
        F: FnOnce(&mut Self, u8) -> u8,
    {
        let v = self.read_operand(bus, op);
        // Dummy read cycle already accounted for in the cycle count.
        let new = f(self, v);
        self.write_operand(bus, op, new);
        self.set_nz(new);
    }

    // ---- load / store / transfer handlers ----------------------------

    fn lda(&mut self, v: u8) {
        self.a = v;
        self.set_nz(v);
    }
    fn ldx(&mut self, v: u8) {
        self.x = v;
        self.set_nz(v);
    }
    fn ldy(&mut self, v: u8) {
        self.y = v;
        self.set_nz(v);
    }
    fn tax(&mut self) {
        self.x = self.a;
        self.set_nz(self.x);
    }
    fn tay(&mut self) {
        self.y = self.a;
        self.set_nz(self.y);
    }
    fn txa(&mut self) {
        self.a = self.x;
        self.set_nz(self.a);
    }
    fn tya(&mut self) {
        self.a = self.y;
        self.set_nz(self.a);
    }
    fn tsx(&mut self) {
        self.x = self.sp;
        self.set_nz(self.x);
    }
    /// TXS — transfer X to SP. Does NOT update flags.
    fn set_sp_from_x(&mut self) {
        self.sp = self.x;
    }

    // ---- logic handlers ---------------------------------------------

    fn and(&mut self, v: u8) {
        self.a &= v;
        self.set_nz(self.a);
    }
    fn ora(&mut self, v: u8) {
        self.a |= v;
        self.set_nz(self.a);
    }
    fn eor(&mut self, v: u8) {
        self.a ^= v;
        self.set_nz(self.a);
    }

    /// BIT — tests bits of A against memory. Z = (A & M) == 0; N = bit 7 of
    /// M; V = bit 6 of M. A is unchanged.
    ///
    /// See: https://www.nesdev.org/wiki/CPU_instructions#BIT
    fn bit(&mut self, m: u8) {
        let result = self.a & m;
        self.set_zero(result == 0);
        self.set_negative((m & 0x80) != 0);
        self.set_overflow((m & 0x40) != 0);
    }

    // ---- arithmetic -------------------------------------------------

    /// ADC — add with carry. The NES 2A03 has no decimal mode, so this is
    /// always binary addition. Overflow flag is set on signed overflow.
    ///
    /// See: https://www.nesdev.org/wiki/CPU_instructions#ADC
    fn adc(&mut self, m: u8) {
        let a = self.a as u16;
        let m = m as u16;
        let c = u16::from(self.carry());
        let sum = a + m + c;
        // Carry from bit 7.
        self.set_carry(sum > 0xFF);
        let result = (sum & 0xFF) as u8;
        // Overflow: signed overflow occurs when both operands have the same
        // sign and the result has a different sign.
        self.set_overflow(((a ^ m) & 0x80) == 0 && ((a ^ sum) & 0x80) != 0);
        self.a = result;
        self.set_nz(result);
    }

    /// SBC — subtract with carry. Implemented as `A + ~M + C`, which shares
    /// the ADC overflow/carry logic. Carry is set when no borrow occurred.
    ///
    /// See: https://www.nesdev.org/wiki/CPU_instructions#SBC
    fn sbc(&mut self, m: u8) {
        let a = self.a as u16;
        let m = (!m) as u16;
        let c = u16::from(self.carry());
        let sum = a + m + c;
        self.set_carry(sum > 0xFF);
        let result = (sum & 0xFF) as u8;
        self.set_overflow(((a ^ m) & 0x80) == 0 && ((a ^ sum) & 0x80) != 0);
        self.a = result;
        self.set_nz(result);
    }

    // ---- compare ----------------------------------------------------

    /// CMP/CPX/CPY — compare register `r` against `m`. Sets C (r >= m),
    /// Z (r == m), and N (bit 7 of r-m). The register is not modified.
    fn cmp(&mut self, r: u8, m: u8) {
        let diff = r.wrapping_sub(m);
        self.set_carry(r >= m);
        self.set_nz(diff);
    }

    // ---- shifts / rotates / inc / dec value transforms --------------

    /// ASL — arithmetic shift left. C = old bit 7; result = (v << 1) & 0xFF.
    fn asl_value(&mut self, v: u8) -> u8 {
        self.set_carry((v & 0x80) != 0);
        v << 1
    }
    /// LSR — logical shift right. C = old bit 0; result = v >> 1.
    fn lsr_value(&mut self, v: u8) -> u8 {
        self.set_carry((v & 0x01) != 0);
        v >> 1
    }
    /// ROL — rotate left through carry. C = old bit 7; bit 0 = old C.
    fn rol_value(&mut self, v: u8) -> u8 {
        let new_c = (v & 0x80) != 0;
        let result = (v << 1) | u8::from(self.carry());
        self.set_carry(new_c);
        result
    }
    /// ROR — rotate right through carry. C = old bit 0; bit 7 = old C.
    fn ror_value(&mut self, v: u8) -> u8 {
        let new_c = (v & 0x01) != 0;
        let result = (v >> 1) | (u8::from(self.carry()) << 7);
        self.set_carry(new_c);
        result
    }
    /// INC — increment a memory value (wrapping).
    fn inc_value(&mut self, v: u8) -> u8 {
        v.wrapping_add(1)
    }
    /// DEC — decrement a memory value (wrapping).
    fn dec_value(&mut self, v: u8) -> u8 {
        v.wrapping_sub(1)
    }

    // ---- branches ---------------------------------------------------

    /// Execute a branch. `cond_flag` is the flag bit to test; `branch_on_set`
    /// selects whether the branch is taken when the flag is set (BEQ/BMI/etc)
    /// or clear (BNE/BPL/etc). Returns cycle count: 2 base, +1 if taken,
    /// +1 if taken and the target is on a different page than the
    /// instruction following the branch.
    fn branch(&mut self, bus: &mut Bus, branch_on_set: bool, cond_flag: u8) -> u8 {
        let flag_set = (self.status & cond_flag) != 0;
        let take = flag_set == branch_on_set;
        if !take {
            // Still consume the offset byte.
            let _ = self.fetch_byte(bus);
            return 2;
        }
        let pc_before = self.pc;
        let target = self.am_relative(bus);
        let pc_after = pc_before.wrapping_add(1); // PC after fetching offset
        let page_cross = (pc_after & 0xFF00) != (target & 0xFF00);
        self.pc = target;
        2 + 1 + u8::from(page_cross)
    }
}
