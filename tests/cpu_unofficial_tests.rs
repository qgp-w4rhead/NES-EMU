//! Integration tests for the unofficial / illegal 6502 opcodes (M33).
//!
//! Each opcode category is exercised end-to-end through a real `Bus` +
//! `Cpu::step`: NOP variants, LAX, SAX, the RMW-combo opcodes (DCP, ISC,
//! SLO, RLA, SRE, RRA), the immediate combined ops (ANC, ALR, ARR, AXS,
//! XAA), the unstable indexed stores (TAS, AHX, SHX, SHY), LAS, and KIL.
//! Cycle counts and flag behavior are checked against the NESdev reference
//! (<https://www.nesdev.org/wiki/CPU_unofficial_opcodes>).
//!
//! Programs are laid down in CPU RAM at `$0200` and the PC is pointed at
//! the start. Operands that must live at specific addresses are written
//! separately.

use nes_emu::bus::Bus;
use nes_emu::cpu::Cpu;

/// Start address for test programs in RAM.
const PC0: u16 = 0x0200;

fn bus_with_prog(prog: &[u8]) -> Bus {
    let mut bus = Bus::new();
    for (i, b) in prog.iter().enumerate() {
        bus.write(PC0 + i as u16, *b);
    }
    bus
}

fn cpu_at_pc0() -> Cpu {
    let mut cpu = Cpu::new();
    cpu.pc = PC0;
    cpu
}

fn step_once(prog: &[u8]) -> (u8, Cpu, Bus) {
    let mut bus = bus_with_prog(prog);
    let mut cpu = cpu_at_pc0();
    let cycles = cpu.step(&mut bus);
    (cycles, cpu, bus)
}

// ===========================================================================
// NOP variants
// ===========================================================================

#[test]
fn nop_implied_variants_advance_pc_and_cost_2_cycles() {
    for &op in &[0x1A, 0x3A, 0x5A, 0x7A, 0xDA, 0xFA] {
        let (c, cpu, _) = step_once(&[op]);
        assert_eq!(c, 2, "opcode ${op:02X}: expected 2 cycles");
        assert_eq!(cpu.pc, PC0 + 1, "opcode ${op:02X}: PC should advance by 1");
    }
}

#[test]
fn nop_immediate_variants_consume_operand_and_cost_2_cycles() {
    for &op in &[0x80, 0x82, 0x89, 0xC2, 0xE2] {
        let (c, cpu, _) = step_once(&[op, 0x42]);
        assert_eq!(c, 2, "opcode ${op:02X}: expected 2 cycles");
        assert_eq!(cpu.pc, PC0 + 2, "opcode ${op:02X}: PC should advance by 2");
    }
}

#[test]
fn nop_zero_page_variants_cost_3_cycles() {
    for &op in &[0x04, 0x44, 0x64] {
        let (c, cpu, _) = step_once(&[op, 0x10]);
        assert_eq!(c, 3, "opcode ${op:02X}: expected 3 cycles");
        assert_eq!(cpu.pc, PC0 + 2);
    }
}

#[test]
fn nop_zero_page_x_variants_cost_4_cycles() {
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x05;
    let mut bus = bus_with_prog(&[0x14, 0x10]);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 4);
    assert_eq!(cpu.pc, PC0 + 2);
}

#[test]
fn nop_absolute_costs_4_cycles() {
    let (c, cpu, _) = step_once(&[0x0C, 0x00, 0x03]);
    assert_eq!(c, 4);
    assert_eq!(cpu.pc, PC0 + 3);
}

#[test]
fn nop_absolute_x_no_page_cross_costs_4_cycles() {
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x10;
    let mut bus = bus_with_prog(&[0x1C, 0x00, 0x02]);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 4);
    assert_eq!(cpu.pc, PC0 + 3);
}

#[test]
fn nop_absolute_x_page_cross_costs_5_cycles() {
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x05;
    // base $02FF + X=$05 -> $0304, page cross.
    let mut bus = bus_with_prog(&[0x1C, 0xFF, 0x02]);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 5);
}

#[test]
fn nop_variants_have_no_side_effects() {
    // Zero-page NOP must not change memory.
    let mut bus = bus_with_prog(&[0x04, 0x10]);
    bus.write(0x0010, 0xAA);
    let mut cpu = cpu_at_pc0();
    cpu.step(&mut bus);
    assert_eq!(bus.read(0x0010), 0xAA);
}

// ===========================================================================
// LAX (load A and X)
// ===========================================================================

#[test]
fn lax_zero_page_loads_a_and_x() {
    let mut bus = bus_with_prog(&[0xA7, 0x10]);
    bus.write(0x0010, 0x42);
    let mut cpu = cpu_at_pc0();
    let c = cpu.step(&mut bus);
    assert_eq!(c, 3);
    assert_eq!(cpu.a, 0x42);
    assert_eq!(cpu.x, 0x42);
    assert!(!cpu.zero());
    assert!(!cpu.negative());
}

#[test]
fn lax_zero_page_y_loads_a_and_x() {
    let mut cpu = cpu_at_pc0();
    cpu.y = 0x04;
    let mut bus = bus_with_prog(&[0xB7, 0x10]);
    bus.write(0x0014, 0x77);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 4);
    assert_eq!(cpu.a, 0x77);
    assert_eq!(cpu.x, 0x77);
}

#[test]
fn lax_absolute_loads_a_and_x() {
    let mut bus = bus_with_prog(&[0xAF, 0x00, 0x03]);
    bus.write(0x0300, 0x80);
    let mut cpu = cpu_at_pc0();
    let c = cpu.step(&mut bus);
    assert_eq!(c, 4);
    assert_eq!(cpu.a, 0x80);
    assert_eq!(cpu.x, 0x80);
    assert!(cpu.negative());
}

#[test]
fn lax_absolute_y_page_cross_costs_5() {
    let mut cpu = cpu_at_pc0();
    cpu.y = 0x05;
    let mut bus = bus_with_prog(&[0xBF, 0xFF, 0x02]);
    bus.write(0x0304, 0x99);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 5);
    assert_eq!(cpu.a, 0x99);
    assert_eq!(cpu.x, 0x99);
}

#[test]
fn lax_indirect_x_loads_a_and_x() {
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x04;
    // Pointer at ($10 + X=4) = $14..$15.
    let mut bus = bus_with_prog(&[0xA3, 0x10]);
    bus.write(0x0014, 0x50);
    bus.write(0x0015, 0x03);
    bus.write(0x0350, 0xCC);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 6);
    assert_eq!(cpu.a, 0xCC);
    assert_eq!(cpu.x, 0xCC); // X unchanged by the load (it was 0x04 before, now 0xCC)
}

#[test]
fn lax_indirect_y_page_cross_costs_6() {
    let mut cpu = cpu_at_pc0();
    cpu.y = 0x10;
    let mut bus = bus_with_prog(&[0xB3, 0x10]);
    bus.write(0x0010, 0xF0);
    bus.write(0x0011, 0x02);
    bus.write(0x0300, 0xDD); // $02F0 + Y=$10 = $0300 (page cross)
    let c = cpu.step(&mut bus);
    assert_eq!(c, 6);
    assert_eq!(cpu.a, 0xDD);
}

#[test]
fn lax_zero_value_sets_zero_flag() {
    let mut bus = bus_with_prog(&[0xA7, 0x10]);
    bus.write(0x0010, 0x00);
    let mut cpu = cpu_at_pc0();
    cpu.step(&mut bus);
    assert!(cpu.zero());
}

// ===========================================================================
// SAX (store A & X)
// ===========================================================================

#[test]
fn sax_zero_page_stores_a_and_x() {
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xF0;
    cpu.x = 0x0F;
    let mut bus = bus_with_prog(&[0x87, 0x20]);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 3);
    assert_eq!(bus.read(0x0020), 0x00); // 0xF0 & 0x0F = 0x00
}

#[test]
fn sax_zero_page_y_stores_a_and_x() {
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xFF;
    cpu.x = 0x3C;
    cpu.y = 0x04;
    let mut bus = bus_with_prog(&[0x97, 0x20]);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 4);
    assert_eq!(bus.read(0x0024), 0x3C); // 0xFF & 0x3C = 0x3C
}

#[test]
fn sax_absolute_stores_a_and_x() {
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xAA;
    cpu.x = 0x55;
    let mut bus = bus_with_prog(&[0x8F, 0x00, 0x05]);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 4);
    assert_eq!(bus.read(0x0500), 0x00); // 0xAA & 0x55 = 0x00
}

#[test]
fn sax_indirect_x_stores_a_and_x() {
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x70;
    cpu.x = 0x04;
    let mut bus = bus_with_prog(&[0x83, 0x10]);
    bus.write(0x0014, 0x00);
    bus.write(0x0015, 0x06);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 6);
    // 0x70 & 0x04 = 0x00
    assert_eq!(bus.read(0x0600), 0x00);
}

#[test]
fn sax_does_not_modify_flags() {
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xFF;
    cpu.x = 0x0F;
    cpu.set_zero(false);
    cpu.set_negative(false);
    let mut bus = bus_with_prog(&[0x87, 0x20]);
    cpu.step(&mut bus);
    assert!(!cpu.zero());
    assert!(!cpu.negative());
}

// ===========================================================================
// DCP (DEC then CMP)
// ===========================================================================

#[test]
fn dcp_zero_page_decrements_and_compares() {
    // A=$10, M=$11 -> M=$10, CMP A=$10 vs M=$10 -> Z set, C set.
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x10;
    let mut bus = bus_with_prog(&[0xC7, 0x30]);
    bus.write(0x0030, 0x11);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 5);
    assert_eq!(bus.read(0x0030), 0x10);
    assert!(cpu.zero());
    assert!(cpu.carry()); // A >= M
}

#[test]
fn dcp_absolute_x_costs_7() {
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x50;
    cpu.x = 0x04;
    let mut bus = bus_with_prog(&[0xDF, 0x00, 0x03]);
    bus.write(0x0304, 0x60);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 7);
    assert_eq!(bus.read(0x0304), 0x5F);
    // A=$50 vs M=$5F -> A < M -> C clear, Z clear, N set (bit7 of $F1)
    assert!(!cpu.carry());
    assert!(!cpu.zero());
}

#[test]
fn dcp_indirect_y_costs_8() {
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x00;
    cpu.y = 0x10;
    let mut bus = bus_with_prog(&[0xD3, 0x10]);
    bus.write(0x0010, 0x00);
    bus.write(0x0011, 0x04);
    bus.write(0x0410, 0x01);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 8);
    // M was $01, decremented to $00. CMP A=$00 vs M=$00 -> Z, C set.
    assert_eq!(bus.read(0x0410), 0x00);
    assert!(cpu.zero());
    assert!(cpu.carry());
}

#[test]
fn dcp_decrement_wraps_from_zero_to_ff() {
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xFF;
    let mut bus = bus_with_prog(&[0xC7, 0x30]);
    bus.write(0x0030, 0x00);
    cpu.step(&mut bus);
    assert_eq!(bus.read(0x0030), 0xFF);
    // A=$FF vs M=$FF -> Z set, C set.
    assert!(cpu.zero());
    assert!(cpu.carry());
}

#[test]
fn dcp_absolute_costs_6_cycles() {
    // DCP absolute (0xCF) — 6 cycles per NESdev.
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x10;
    let mut bus = bus_with_prog(&[0xCF, 0x00, 0x05]);
    bus.write(0x0500, 0x11);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 6);
    assert_eq!(bus.read(0x0500), 0x10);
    // A=$10 vs M=$10 -> Z set, C set.
    assert!(cpu.zero());
    assert!(cpu.carry());
}

#[test]
fn isc_absolute_costs_6_cycles() {
    // ISC absolute (0xEF) — 6 cycles per NESdev.
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x10;
    cpu.set_carry(true);
    let mut bus = bus_with_prog(&[0xEF, 0x00, 0x05]);
    bus.write(0x0500, 0x0F);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 6);
    assert_eq!(bus.read(0x0500), 0x10);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.carry());
}

#[test]
fn slo_absolute_costs_6_cycles() {
    // SLO absolute (0x0F) — 6 cycles per NESdev.
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x01;
    let mut bus = bus_with_prog(&[0x0F, 0x00, 0x05]);
    bus.write(0x0500, 0x01);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 6);
    assert_eq!(bus.read(0x0500), 0x02);
    assert_eq!(cpu.a, 0x03);
}

// ===========================================================================
// ISC (INC then SBC)
// ===========================================================================

#[test]
fn isc_zero_page_increments_and_subtracts() {
    // A=$10, M=$0F, C=1 -> M=$10, SBC: A=$10 - $10 - 0 = $00, C set, Z set.
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x10;
    cpu.set_carry(true);
    let mut bus = bus_with_prog(&[0xE7, 0x30]);
    bus.write(0x0030, 0x0F);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 5);
    assert_eq!(bus.read(0x0030), 0x10);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.carry()); // no borrow
    assert!(cpu.zero());
}

#[test]
fn isc_absolute_y_costs_7() {
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x00;
    cpu.y = 0x10;
    cpu.set_carry(false);
    let mut bus = bus_with_prog(&[0xFB, 0x00, 0x04]);
    bus.write(0x0410, 0xFE);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 7);
    // M=$FE -> $FF. SBC: A=$00 - $FF - 1 (no carry) = $00 - $00 = $00 with borrow.
    // A = (0x00 + ~0xFF + 0) = 0x00, C clear (borrow).
    assert_eq!(bus.read(0x0410), 0xFF);
    assert_eq!(cpu.a, 0x00);
    assert!(!cpu.carry());
}

#[test]
fn isc_increment_wraps_from_ff_to_zero() {
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x10;
    cpu.set_carry(true);
    let mut bus = bus_with_prog(&[0xE7, 0x30]);
    bus.write(0x0030, 0xFF);
    cpu.step(&mut bus);
    assert_eq!(bus.read(0x0030), 0x00);
    // SBC A=$10 - $00 - 0 = $10, C set.
    assert_eq!(cpu.a, 0x10);
    assert!(cpu.carry());
}

// ===========================================================================
// SLO (ASL then ORA)
// ===========================================================================

#[test]
fn slo_zero_page_shifts_and_ors() {
    // A=$01, M=$01 -> ASL M=$02, C=0. ORA: A=$01|$02=$03.
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x01;
    let mut bus = bus_with_prog(&[0x07, 0x30]);
    bus.write(0x0030, 0x01);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 5);
    assert_eq!(bus.read(0x0030), 0x02);
    assert_eq!(cpu.a, 0x03);
    assert!(!cpu.carry()); // bit 7 of $01 was 0
}

#[test]
fn slo_asl_sets_carry_from_bit_7() {
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x00;
    let mut bus = bus_with_prog(&[0x07, 0x30]);
    bus.write(0x0030, 0x80);
    cpu.step(&mut bus);
    assert_eq!(bus.read(0x0030), 0x00);
    assert!(cpu.carry()); // bit 7 was 1
    assert_eq!(cpu.a, 0x00);
}

#[test]
fn slo_indirect_x_costs_8() {
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x0F;
    cpu.x = 0x04;
    let mut bus = bus_with_prog(&[0x03, 0x10]);
    bus.write(0x0014, 0x00);
    bus.write(0x0015, 0x05);
    bus.write(0x0500, 0x01);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 8);
    assert_eq!(bus.read(0x0500), 0x02);
    assert_eq!(cpu.a, 0x0F | 0x02);
}

// ===========================================================================
// RLA (ROL then AND)
// ===========================================================================

#[test]
fn rla_zero_page_rotates_and_ands() {
    // A=$02, M=$01, C=0 -> ROL M=$02, C=0. AND: A=$02&$02=$02.
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x02;
    cpu.set_carry(false);
    let mut bus = bus_with_prog(&[0x27, 0x30]);
    bus.write(0x0030, 0x01);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 5);
    assert_eq!(bus.read(0x0030), 0x02);
    assert_eq!(cpu.a, 0x02);
}

#[test]
fn rla_rol_uses_carry_in() {
    // A=$FF, M=$00, C=1 -> ROL M=$01, C=0. AND: A=$FF&$01=$01.
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xFF;
    cpu.set_carry(true);
    let mut bus = bus_with_prog(&[0x27, 0x30]);
    bus.write(0x0030, 0x00);
    cpu.step(&mut bus);
    assert_eq!(bus.read(0x0030), 0x01);
    assert_eq!(cpu.a, 0x01);
}

#[test]
fn rla_absolute_x_costs_7() {
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x80;
    cpu.x = 0x04;
    cpu.set_carry(false);
    let mut bus = bus_with_prog(&[0x3F, 0x00, 0x03]);
    bus.write(0x0304, 0x40);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 7);
    // ROL $40 (C=0) -> $80, C=0. AND A=$80 & $80 = $80.
    assert_eq!(bus.read(0x0304), 0x80);
    assert_eq!(cpu.a, 0x80);
}

// ===========================================================================
// SRE (LSR then EOR)
// ===========================================================================

#[test]
fn sre_zero_page_shifts_and_eors() {
    // A=$03, M=$02 -> LSR M=$01, C=0. EOR: A=$03^$01=$02.
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x03;
    let mut bus = bus_with_prog(&[0x47, 0x30]);
    bus.write(0x0030, 0x02);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 5);
    assert_eq!(bus.read(0x0030), 0x01);
    assert_eq!(cpu.a, 0x02);
    assert!(!cpu.carry());
}

#[test]
fn sre_lsr_sets_carry_from_bit_0() {
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x00;
    let mut bus = bus_with_prog(&[0x47, 0x30]);
    bus.write(0x0030, 0x03);
    cpu.step(&mut bus);
    assert_eq!(bus.read(0x0030), 0x01);
    assert!(cpu.carry()); // bit 0 was 1
}

// ===========================================================================
// RRA (ROR then ADC)
// ===========================================================================

#[test]
fn rra_zero_page_rotates_and_adds() {
    // A=$01, M=$01, C=0 -> ROR M=$00, C=1. ADC: A=$01+$00+1=$02, C=0.
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x01;
    cpu.set_carry(false);
    let mut bus = bus_with_prog(&[0x67, 0x30]);
    bus.write(0x0030, 0x01);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 5);
    assert_eq!(bus.read(0x0030), 0x00);
    assert_eq!(cpu.a, 0x02);
    // After ADC, C = carry-out of $01+$00+1 = $02, no overflow -> C=0.
    assert!(!cpu.carry());
}

#[test]
fn rra_ror_uses_carry_in() {
    // A=$00, M=$00, C=1 -> ROR M=$80, C=0. ADC: A=$00+$80+0=$80, C=0.
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x00;
    cpu.set_carry(true);
    let mut bus = bus_with_prog(&[0x67, 0x30]);
    bus.write(0x0030, 0x00);
    cpu.step(&mut bus);
    assert_eq!(bus.read(0x0030), 0x80);
    assert_eq!(cpu.a, 0x80);
}

#[test]
fn rra_indirect_y_costs_8() {
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x10;
    cpu.y = 0x10;
    cpu.set_carry(false);
    let mut bus = bus_with_prog(&[0x73, 0x10]);
    bus.write(0x0010, 0x00);
    bus.write(0x0011, 0x04);
    bus.write(0x0410, 0x02);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 8);
    // ROR $02 (C=0) -> $01, C=0. ADC A=$10+$01+0=$11.
    assert_eq!(bus.read(0x0410), 0x01);
    assert_eq!(cpu.a, 0x11);
}

// ===========================================================================
// ANC (AND then copy N to C)
// ===========================================================================

#[test]
fn anc_and_sets_carry_to_negative_bit() {
    // Both 0x0B and 0x2B are ANC.
    for &op in &[0x0B, 0x2B] {
        let mut cpu = cpu_at_pc0();
        cpu.a = 0xF0;
        let mut bus = bus_with_prog(&[op, 0x80]);
        let c = cpu.step(&mut bus);
        assert_eq!(c, 2, "opcode ${op:02X}");
        assert_eq!(cpu.a, 0x80); // 0xF0 & 0x80 = 0x80
        assert!(cpu.negative());
        assert!(cpu.carry()); // C = bit 7 of A = 1
    }
}

#[test]
fn anc_clears_carry_when_bit7_clear() {
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x70;
    cpu.set_carry(true);
    let mut bus = bus_with_prog(&[0x0B, 0xFF]);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x70);
    assert!(!cpu.carry());
    assert!(!cpu.negative());
}

// ===========================================================================
// ALR (AND then LSR)
// ===========================================================================

#[test]
fn alr_and_then_lsr() {
    // A=$FF, imm=$0F -> A=$0F. LSR: A=$07, C=1 (bit 0 of $0F was 1).
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xFF;
    let mut bus = bus_with_prog(&[0x4B, 0x0F]);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 2);
    assert_eq!(cpu.a, 0x07);
    assert!(cpu.carry());
}

#[test]
fn alr_even_result_clears_carry() {
    // A=$FF, imm=$0E -> A=$0E. LSR: A=$07, C=0 (bit 0 of $0E was 0).
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xFF;
    let mut bus = bus_with_prog(&[0x4B, 0x0E]);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x07);
    assert!(!cpu.carry());
}

// ===========================================================================
// ARR (AND then ROR, special V/C)
// ===========================================================================

#[test]
fn arr_and_then_ror_with_special_flags() {
    // A=$FF, imm=$C0, C=0 -> A&imm=$C0=1100_0000. ROR (C=0): A=$60=0110_0000.
    // C = bit 6 of $60 = 1. V = bit6 XOR bit5 = 1 XOR 1 = 0.
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xFF;
    cpu.set_carry(false);
    let mut bus = bus_with_prog(&[0x6B, 0xC0]);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 2);
    assert_eq!(cpu.a, 0x60);
    assert!(cpu.carry()); // C = bit 6 of result ($60) = 1
    assert!(!cpu.overflow()); // V = bit6 XOR bit5 = 1 XOR 1 = 0
}

#[test]
fn arr_carry_in_becomes_bit_7() {
    // A=$FF, imm=$7F, C=1 -> A&imm=$7F=0111_1111. ROR (C=1): A=$BF=1011_1111.
    // C = bit 6 of $BF = 0. V = bit6 XOR bit5 = 0 XOR 1 = 1.
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xFF;
    cpu.set_carry(true);
    let mut bus = bus_with_prog(&[0x6B, 0x7F]);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0xBF);
    assert!(!cpu.carry()); // C = bit 6 of $BF = 0
    assert!(cpu.overflow()); // V = bit6 XOR bit5 = 0 XOR 1 = 1
}

#[test]
fn arr_overflow_set_when_bit6_xor_bit5() {
    // A=$FF, imm=$80, C=0 -> A&imm=$80=1000_0000. ROR (C=0): A=$40=0100_0000.
    // C = bit 6 of $40 = 1. V = bit6 XOR bit5 = 1 XOR 0 = 1.
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xFF;
    cpu.set_carry(false);
    let mut bus = bus_with_prog(&[0x6B, 0x80]);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x40);
    assert!(cpu.carry());
    assert!(cpu.overflow());
}

// ===========================================================================
// AXS / SBX (subtract imm from A&X, store in X)
// ===========================================================================

#[test]
fn axs_subtracts_from_a_and_x() {
    // A=$F0, X=$0F -> A&X=$00. imm=$00 -> X=$00, C set (0>=0), Z set.
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xF0;
    cpu.x = 0x0F;
    let mut bus = bus_with_prog(&[0xCB, 0x00]);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 2);
    assert_eq!(cpu.x, 0x00);
    assert!(cpu.carry());
    assert!(cpu.zero());
    assert_eq!(cpu.a, 0xF0); // A unchanged
}

#[test]
fn axs_borrow_clears_carry() {
    // A=$FF, X=$FF -> A&X=$FF. imm=$01 -> X=$FE, C set (FF>=01).
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xFF;
    cpu.x = 0xFF;
    let mut bus = bus_with_prog(&[0xCB, 0x01]);
    cpu.step(&mut bus);
    assert_eq!(cpu.x, 0xFE);
    assert!(cpu.carry());
}

#[test]
fn axs_underflow_wraps_and_clears_carry() {
    // A&X=$00, imm=$01 -> X=$FF (wrap), C clear (0<1).
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x00;
    cpu.x = 0x00;
    let mut bus = bus_with_prog(&[0xCB, 0x01]);
    cpu.step(&mut bus);
    assert_eq!(cpu.x, 0xFF);
    assert!(!cpu.carry());
    assert!(cpu.negative());
}

// ===========================================================================
// XAA (unstable: A = (A | magic) & X & imm)
// ===========================================================================

#[test]
fn xaa_computes_a_and_x_and_imm() {
    // With XAA_MAGIC=0: A = A & X & imm.
    // A=$FF, X=$0F, imm=$0C -> A=$0F & $0C = $0C.
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xFF;
    cpu.x = 0x0F;
    let mut bus = bus_with_prog(&[0x8B, 0x0C]);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 2);
    assert_eq!(cpu.a, 0x0C);
}

#[test]
fn xaa_zero_result_sets_zero_flag() {
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xFF;
    cpu.x = 0x0F;
    let mut bus = bus_with_prog(&[0x8B, 0xF0]);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.zero());
}

// ===========================================================================
// TAS / SHS (SP = A & X; store SP & (H+1))
// ===========================================================================

#[test]
fn tas_sets_sp_and_stores() {
    // A=$FF, X=$0F -> SP=$0F. base=$0300, H=$03, H+1=$04.
    // value = $0F & $04 = $04. Store at $0300 + Y.
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xFF;
    cpu.x = 0x0F;
    cpu.y = 0x10;
    let mut bus = bus_with_prog(&[0x9B, 0x00, 0x03]);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 5);
    assert_eq!(cpu.sp, 0x0F);
    assert_eq!(bus.read(0x0310), 0x04);
}

#[test]
fn tas_page_cross_quirk_diverges_from_normal_addr() {
    // Pick inputs where the quirk address differs from the normal eff addr.
    // base=$02F0, Y=$20 -> normal eff=$0310 (page cross). H=$02, H+1=$03.
    // A=$F7, X=$FF -> SP=$F7. value=$F7 & $03 = $03.
    // Quirk: store_addr high byte = value=$03, low byte = $10 -> $0310.
    // (Here value=$03 happens to equal H+1=$03, so add a second case below
    // where value != H+1 to truly distinguish quirk from normal.)
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xF7;
    cpu.x = 0xFF;
    cpu.y = 0x20;
    let mut bus = bus_with_prog(&[0x9B, 0xF0, 0x02]);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 5);
    assert_eq!(cpu.sp, 0xF7);
    assert_eq!(bus.read(0x0310), 0x03);

    // Second case: value != H+1 so the quirk address diverges from the
    // normal eff addr. base=$02F0, Y=$20 -> normal eff=$0310. H=$02,
    // H+1=$03. A=$FF, X=$F0 -> SP=$F0. value=$F0 & $03 = $00. Quirk addr =
    // ($00 << 8) | $10 = $0010, NOT $0310. The store must land at $0010
    // and the normal eff addr $0310 must remain untouched.
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xFF;
    cpu.x = 0xF0;
    cpu.y = 0x20;
    let mut bus = bus_with_prog(&[0x9B, 0xF0, 0x02]);
    // Pre-fill both candidate addresses with sentinels so we can tell
    // which one was actually written.
    bus.write(0x0010, 0xAA);
    bus.write(0x0310, 0xBB);
    cpu.step(&mut bus);
    assert_eq!(cpu.sp, 0xF0);
    // Quirk addr $0010 was overwritten with value=$00.
    assert_eq!(
        bus.read(0x0010),
        0x00,
        "quirk addr $0010 should hold the store"
    );
    // Normal eff addr $0310 must NOT have been touched (still $BB).
    assert_eq!(bus.read(0x0310), 0xBB, "normal eff addr must be untouched");
}

// ===========================================================================
// AHX / SHA (store A & X & (H+1))
// ===========================================================================

#[test]
fn ahx_absolute_y_stores_a_and_x_and_h_plus_1() {
    // A=$FF, X=$F0, base=$0300, H=$03, H+1=$04.
    // value = $FF & $F0 & $04 = $00. Store at $0300 + Y=$10 = $0310.
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xFF;
    cpu.x = 0xF0;
    cpu.y = 0x10;
    let mut bus = bus_with_prog(&[0x9F, 0x00, 0x03]);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 5);
    assert_eq!(bus.read(0x0310), 0x00);
}

#[test]
fn ahx_indirect_y_stores() {
    // A=$FF, X=$FF, pointer at $10..$11 = $0300, Y=$10 -> eff=$0310.
    // H=$03, H+1=$04. value = $FF & $FF & $04 = $04.
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xFF;
    cpu.x = 0xFF;
    cpu.y = 0x10;
    let mut bus = bus_with_prog(&[0x93, 0x10]);
    bus.write(0x0010, 0x00);
    bus.write(0x0011, 0x03);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 6);
    assert_eq!(bus.read(0x0310), 0x04);
}

#[test]
fn ahx_absolute_y_page_cross_quirk_diverges() {
    // AHX abs,Y with a page cross where value != H+1, so the quirk addr
    // differs from the normal eff addr.
    // A=$F0, X=$FF -> reg=$F0. base=$02F0, Y=$20 -> normal eff=$0310.
    // H=$02, H+1=$03. value=$F0 & $03 = $00. Quirk addr = $0010.
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xF0;
    cpu.x = 0xFF;
    cpu.y = 0x20;
    let mut bus = bus_with_prog(&[0x9F, 0xF0, 0x02]);
    bus.write(0x0010, 0xAA);
    bus.write(0x0310, 0xBB);
    cpu.step(&mut bus);
    assert_eq!(
        bus.read(0x0010),
        0x00,
        "quirk addr $0010 should hold the store"
    );
    assert_eq!(bus.read(0x0310), 0xBB, "normal eff addr must be untouched");
}

// ===========================================================================
// SHX / SXA (store X & (H+1))
// ===========================================================================

#[test]
fn shx_absolute_y_stores_x_and_h_plus_1() {
    // X=$F0, base=$0300, H=$03, H+1=$04. value=$F0 & $04 = $00.
    // Store at $0300 + Y=$10 = $0310.
    let mut cpu = cpu_at_pc0();
    cpu.x = 0xF0;
    cpu.y = 0x10;
    let mut bus = bus_with_prog(&[0x9E, 0x00, 0x03]);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 5);
    assert_eq!(bus.read(0x0310), 0x00);
}

#[test]
fn shx_nonzero_value() {
    // X=$07, base=$0300, H=$03, H+1=$04. value=$07 & $04 = $04.
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x07;
    cpu.y = 0x10;
    let mut bus = bus_with_prog(&[0x9E, 0x00, 0x03]);
    cpu.step(&mut bus);
    assert_eq!(bus.read(0x0310), 0x04);
}

#[test]
fn shx_absolute_y_page_cross_quirk_diverges() {
    // SHX abs,Y with a page cross where value != H+1.
    // X=$F0, base=$02F0, Y=$20 -> normal eff=$0310. H=$02, H+1=$03.
    // value=$F0 & $03 = $00. Quirk addr = $0010 (NOT $0310).
    let mut cpu = cpu_at_pc0();
    cpu.x = 0xF0;
    cpu.y = 0x20;
    let mut bus = bus_with_prog(&[0x9E, 0xF0, 0x02]);
    bus.write(0x0010, 0xAA);
    bus.write(0x0310, 0xBB);
    cpu.step(&mut bus);
    assert_eq!(
        bus.read(0x0010),
        0x00,
        "quirk addr $0010 should hold the store"
    );
    assert_eq!(bus.read(0x0310), 0xBB, "normal eff addr must be untouched");
}

// ===========================================================================
// SHY / SYA (store Y & (H+1))
// ===========================================================================

#[test]
fn shy_absolute_x_stores_y_and_h_plus_1() {
    // Y=$07, base=$0300, H=$03, H+1=$04. value=$07 & $04 = $04.
    // Store at $0300 + X=$10 = $0310.
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x10;
    cpu.y = 0x07;
    let mut bus = bus_with_prog(&[0x9C, 0x00, 0x03]);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 5);
    assert_eq!(bus.read(0x0310), 0x04);
}

#[test]
fn shy_absolute_x_page_cross_quirk_diverges() {
    // SHY abs,X with a page cross where value != H+1. SHY is indexed by X.
    // Y=$F0, base=$02F0, X=$20 -> normal eff=$0310. H=$02, H+1=$03.
    // value=$F0 & $03 = $00. Quirk addr = $0010 (NOT $0310).
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x20;
    cpu.y = 0xF0;
    let mut bus = bus_with_prog(&[0x9C, 0xF0, 0x02]);
    bus.write(0x0010, 0xAA);
    bus.write(0x0310, 0xBB);
    cpu.step(&mut bus);
    assert_eq!(
        bus.read(0x0010),
        0x00,
        "quirk addr $0010 should hold the store"
    );
    assert_eq!(bus.read(0x0310), 0xBB, "normal eff addr must be untouched");
}

// ===========================================================================
// LAS / LAR (A = X = SP = M & SP)
// ===========================================================================

#[test]
fn las_loads_a_x_sp_from_memory_anded_with_sp() {
    // SP=$F0, M=$0F -> result=$00. A=X=SP=$00.
    let mut cpu = cpu_at_pc0();
    cpu.sp = 0xF0;
    cpu.y = 0x10;
    let mut bus = bus_with_prog(&[0xBB, 0x00, 0x03]);
    bus.write(0x0310, 0x0F);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 4);
    assert_eq!(cpu.a, 0x00);
    assert_eq!(cpu.x, 0x00);
    assert_eq!(cpu.sp, 0x00);
    assert!(cpu.zero());
}

#[test]
fn las_nonzero_result() {
    let mut cpu = cpu_at_pc0();
    cpu.sp = 0xFF;
    let mut bus = bus_with_prog(&[0xBB, 0x00, 0x03]);
    bus.write(0x0300, 0x80);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x80);
    assert_eq!(cpu.x, 0x80);
    assert_eq!(cpu.sp, 0x80);
    assert!(cpu.negative());
}

#[test]
fn las_absolute_y_page_cross_costs_5() {
    let mut cpu = cpu_at_pc0();
    cpu.sp = 0xFF;
    cpu.y = 0x05;
    let mut bus = bus_with_prog(&[0xBB, 0xFF, 0x02]);
    bus.write(0x0304, 0x10);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 5);
    assert_eq!(cpu.a, 0x10);
}

// ===========================================================================
// KIL / JAM / HLT
// ===========================================================================

#[test]
fn kil_halts_cpu_and_returns_one_cycle() {
    for &op in &[
        0x02, 0x12, 0x22, 0x32, 0x42, 0x52, 0x62, 0x72, 0x92, 0xB2, 0xD2, 0xF2,
    ] {
        let (c, cpu, _) = step_once(&[op]);
        assert_eq!(c, 1, "opcode ${op:02X}: expected 1 cycle");
        assert!(cpu.is_halted(), "opcode ${op:02X}: CPU should be halted");
    }
}

#[test]
fn kil_does_not_advance_pc_past_opcode() {
    let (_, cpu, _) = step_once(&[0x02]);
    // PC should be at PC0 + 1 (the KIL opcode byte was fetched, but no
    // further execution happens).
    assert_eq!(cpu.pc, PC0 + 1);
}

#[test]
fn kil_subsequent_step_is_no_op() {
    let mut bus = bus_with_prog(&[0x02, 0xEA, 0xEA]);
    let mut cpu = cpu_at_pc0();
    let c1 = cpu.step(&mut bus);
    assert_eq!(c1, 1);
    let pc_after_kil = cpu.pc;
    let c2 = cpu.step(&mut bus);
    assert_eq!(c2, 1);
    assert_eq!(cpu.pc, pc_after_kil, "PC must not advance while halted");
    assert!(cpu.is_halted());
}

#[test]
fn kil_revived_by_reset() {
    let mut bus = bus_with_prog(&[0x02]);
    // Set up RESET vector at $FFFC/$FFFD -> $0000.
    bus.write(0xFFFC, 0x00);
    bus.write(0xFFFD, 0x00);
    let mut cpu = cpu_at_pc0();
    cpu.step(&mut bus);
    assert!(cpu.is_halted());
    cpu.reset(&mut bus);
    assert!(!cpu.is_halted());
    assert_eq!(cpu.pc, 0x0000);
}

#[test]
fn kil_ignores_pending_nmi_and_irq() {
    // A halted CPU must not service NMI or IRQ — `step` returns 1 before
    // reaching the interrupt checks. PC must not jump to a vector.
    let mut bus = bus_with_prog(&[0x02]);
    // Set NMI vector at $FFFA/$FFFB -> $ABCD and IRQ vector -> $1234 so
    // we can detect if either was taken.
    bus.write(0xFFFA, 0xCD);
    bus.write(0xFFFB, 0xAB);
    bus.write(0xFFFE, 0x34);
    bus.write(0xFFFF, 0x12);
    let mut cpu = cpu_at_pc0();
    cpu.step(&mut bus);
    assert!(cpu.is_halted());
    let pc_after_kil = cpu.pc;

    // Raise both NMI and IRQ, then step — neither should be serviced.
    cpu.nmi_pending = true;
    cpu.irq_pending = true;
    let c = cpu.step(&mut bus);
    assert_eq!(c, 1, "halted step must return 1 cycle");
    assert_eq!(
        cpu.pc, pc_after_kil,
        "PC must not jump to an interrupt vector"
    );
    assert!(cpu.is_halted(), "CPU must remain halted");
    // The pending flags are NOT cleared by a halted step (no service ran).
    assert!(
        cpu.nmi_pending,
        "NMI pending must not be cleared while halted"
    );
    assert!(
        cpu.irq_pending,
        "IRQ pending must not be cleared while halted"
    );
}

// ===========================================================================
// Disassembler coverage for unofficial opcodes
// ===========================================================================

#[test]
fn disasm_decodes_unofficial_mnemonics() {
    use nes_emu::debug::disasm::disassemble_at;
    let mut bus = bus_with_prog(&[
        0xA7, 0x10, // LAX $10
        0x87, 0x20, // SAX $20
        0xC7, 0x30, // DCP $30
        0x0B, 0x42, // ANC #$42
        0x02, // KIL
    ]);
    let d0 = disassemble_at(&bus, PC0);
    assert_eq!(d0.text, "LAX $10");
    let d1 = disassemble_at(&bus, PC0 + 2);
    assert_eq!(d1.text, "SAX $20");
    let d2 = disassemble_at(&bus, PC0 + 4);
    assert_eq!(d2.text, "DCP $30");
    let d3 = disassemble_at(&bus, PC0 + 6);
    assert_eq!(d3.text, "ANC #$42");
    let d4 = disassemble_at(&bus, PC0 + 8);
    assert_eq!(d4.text, "KIL");
    let _ = &mut bus;
}

// ===========================================================================
// Save state version
// ===========================================================================

#[test]
fn save_state_version_is_six() {
    use nes_emu::save_state::SAVE_STATE_VERSION;
    assert_eq!(SAVE_STATE_VERSION, 6);
}
