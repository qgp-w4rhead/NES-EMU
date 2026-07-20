//! Integration tests for the 6502 CPU core (M4): registers, flags, and all
//! 13 addressing modes.
//!
//! These exercise the public `Cpu::resolve` API end-to-end through a real
//! `Bus`. Operand bytes and indirect pointers are placed in CPU RAM
//! (`$0000-$07FF`) so they are readable without a loaded cartridge.

use nes_emu::bus::Bus;
use nes_emu::cpu::{AddrMode, Cpu, Operand};

/// A free RAM area used to lay down operand bytes for the program counter.
/// `$0200` is the top of the stack-friendly region and well within the
/// 2 KB internal RAM.
const OPC: u16 = 0x0200;

/// Build a bus with `bytes` written starting at `start` (CPU address space).
fn bus_with(start: u16, bytes: &[u8]) -> Bus {
    let mut bus = Bus::new();
    for (i, b) in bytes.iter().enumerate() {
        bus.write(start + i as u16, *b);
    }
    bus
}

/// Write a 16-bit little-endian pointer into RAM at `addr`.
fn write_ptr(bus: &mut Bus, addr: u16, value: u16) {
    bus.write(addr, (value & 0xFF) as u8);
    bus.write(addr + 1, (value >> 8) as u8);
}

// ---- flag helpers ---------------------------------------------------

#[test]
fn new_cpu_has_i_and_u_flags_set() {
    let cpu = Cpu::new();
    assert!(cpu.interrupt_disable());
    assert!(!cpu.decimal());
    assert!(!cpu.carry());
    // U bit (0x20) is set on power-on; B (0x10) is not.
    assert_eq!(cpu.status & 0b0011_0000, 0b0010_0000);
}

#[test]
fn set_and_clear_each_flag() {
    let mut cpu = Cpu::new();

    cpu.set_carry(true);
    assert!(cpu.carry());
    cpu.set_carry(false);
    assert!(!cpu.carry());

    cpu.set_zero(true);
    assert!(cpu.zero());
    cpu.set_zero(false);
    assert!(!cpu.zero());

    cpu.set_interrupt_disable(false);
    assert!(!cpu.interrupt_disable());
    cpu.set_interrupt_disable(true);
    assert!(cpu.interrupt_disable());

    cpu.set_decimal(true);
    assert!(cpu.decimal());
    cpu.set_decimal(false);
    assert!(!cpu.decimal());

    cpu.set_overflow(true);
    assert!(cpu.overflow());
    cpu.set_overflow(false);
    assert!(!cpu.overflow());

    cpu.set_negative(true);
    assert!(cpu.negative());
    cpu.set_negative(false);
    assert!(!cpu.negative());
}

#[test]
fn set_nz_sets_both_flags_from_value() {
    let mut cpu = Cpu::new();
    cpu.set_nz(0x00);
    assert!(cpu.zero());
    assert!(!cpu.negative());

    cpu.set_nz(0x80);
    assert!(!cpu.zero());
    assert!(cpu.negative());

    cpu.set_nz(0x7F);
    assert!(!cpu.zero());
    assert!(!cpu.negative());
}

// ---- implied & accumulator -----------------------------------------

#[test]
fn implied_returns_none_and_does_not_advance_pc() {
    let bus = Bus::new();
    let mut cpu = Cpu::new();
    cpu.pc = 0x0123;
    let op = cpu.resolve(&bus, AddrMode::Implied);
    assert_eq!(op, Operand::None);
    assert_eq!(cpu.pc, 0x0123);
}

#[test]
fn accumulator_returns_accumulator_and_does_not_advance_pc() {
    let bus = Bus::new();
    let mut cpu = Cpu::new();
    cpu.pc = 0x0123;
    let op = cpu.resolve(&bus, AddrMode::Accumulator);
    assert_eq!(op, Operand::Accumulator);
    assert_eq!(cpu.pc, 0x0123);
}

// ---- immediate ------------------------------------------------------

#[test]
fn immediate_returns_pc_then_advances_one() {
    let bus = Bus::new();
    let mut cpu = Cpu::new();
    cpu.pc = 0x0200;
    let op = cpu.resolve(&bus, AddrMode::Immediate);
    assert_eq!(op, Operand::Address(0x0200));
    assert_eq!(cpu.pc, 0x0201);
}

// ---- zero-page ------------------------------------------------------

#[test]
fn zero_page_returns_operand_and_advances_one() {
    let bus = bus_with(OPC, &[0x44]);
    let mut cpu = Cpu::new();
    cpu.pc = OPC;
    let op = cpu.resolve(&bus, AddrMode::ZeroPage);
    assert_eq!(op, Operand::Address(0x0044));
    assert_eq!(cpu.pc, OPC + 1);
}

// ---- zero-page,X ----------------------------------------------------

#[test]
fn zero_page_x_wraps_within_zero_page() {
    let bus = bus_with(OPC, &[0xF0]);
    let mut cpu = Cpu::new();
    cpu.pc = OPC;
    cpu.x = 0x20;
    let op = cpu.resolve(&bus, AddrMode::ZeroPageX);
    assert_eq!(op, Operand::Address(0x0010));
    assert_eq!(cpu.pc, OPC + 1);
}

#[test]
fn zero_page_x_wrap_around_at_ff() {
    // $FF + $01 = $00 (wraps in zero page, not into $0100).
    let bus = bus_with(OPC, &[0xFF]);
    let mut cpu = Cpu::new();
    cpu.pc = OPC;
    cpu.x = 0x01;
    let op = cpu.resolve(&bus, AddrMode::ZeroPageX);
    assert_eq!(op, Operand::Address(0x0000));
}

// ---- zero-page,Y ----------------------------------------------------

#[test]
fn zero_page_y_wraps_within_zero_page() {
    let bus = bus_with(OPC, &[0x10]);
    let mut cpu = Cpu::new();
    cpu.pc = OPC;
    cpu.y = 0x05;
    let op = cpu.resolve(&bus, AddrMode::ZeroPageY);
    assert_eq!(op, Operand::Address(0x0015));
    assert_eq!(cpu.pc, OPC + 1);
}

// ---- absolute -------------------------------------------------------

#[test]
fn absolute_returns_little_endian_word_and_advances_two() {
    let bus = bus_with(OPC, &[0x00, 0x44]);
    let mut cpu = Cpu::new();
    cpu.pc = OPC;
    let op = cpu.resolve(&bus, AddrMode::Absolute);
    assert_eq!(op, Operand::Address(0x4400));
    assert_eq!(cpu.pc, OPC + 2);
}

// ---- absolute,X -----------------------------------------------------

#[test]
fn absolute_x_adds_x_to_base() {
    let bus = bus_with(OPC, &[0x00, 0x44]);
    let mut cpu = Cpu::new();
    cpu.pc = OPC;
    cpu.x = 0x10;
    let op = cpu.resolve(&bus, AddrMode::AbsoluteX);
    assert_eq!(op, Operand::Address(0x4410));
    assert_eq!(cpu.pc, OPC + 2);
}

#[test]
fn absolute_x_wraps_at_16_bit_boundary() {
    let bus = bus_with(OPC, &[0xFF, 0xFF]);
    let mut cpu = Cpu::new();
    cpu.pc = OPC;
    cpu.x = 0x02;
    let op = cpu.resolve(&bus, AddrMode::AbsoluteX);
    assert_eq!(op, Operand::Address(0x0001));
}

// ---- absolute,Y -----------------------------------------------------

#[test]
fn absolute_y_adds_y_to_base() {
    let bus = bus_with(OPC, &[0x34, 0x12]);
    let mut cpu = Cpu::new();
    cpu.pc = OPC;
    cpu.y = 0x0C;
    let op = cpu.resolve(&bus, AddrMode::AbsoluteY);
    assert_eq!(op, Operand::Address(0x1240));
    assert_eq!(cpu.pc, OPC + 2);
}

#[test]
fn absolute_y_wraps_at_16_bit_boundary() {
    let bus = bus_with(OPC, &[0xFF, 0xFF]);
    let mut cpu = Cpu::new();
    cpu.pc = OPC;
    cpu.y = 0x02;
    let op = cpu.resolve(&bus, AddrMode::AbsoluteY);
    assert_eq!(op, Operand::Address(0x0001));
}

// ---- indirect (with page-wrap bug) ----------------------------------

#[test]
fn indirect_reads_pointer_at_operand_address() {
    // JMP ($0300) where $0300/$0301 contains $30/$40 -> $4030.
    // $0300 is in RAM (mirrors $0300 -> $0300 & 0x07FF = $0300).
    let mut bus = bus_with(OPC, &[0x00, 0x03]);
    write_ptr(&mut bus, 0x0300, 0x4030);
    let mut cpu = Cpu::new();
    cpu.pc = OPC;
    let op = cpu.resolve(&bus, AddrMode::Indirect);
    assert_eq!(op, Operand::Address(0x4030));
    assert_eq!(cpu.pc, OPC + 2);
}

#[test]
fn indirect_page_wrap_bug_high_byte_from_same_page() {
    // JMP ($07FF): lo from $07FF, hi from $0700 (NOT $0800).
    // $07FF is the last byte of physical RAM; $0700 is in the same page.
    // $0800 mirrors $0000, so without the bug we'd read the wrong value.
    let mut bus = bus_with(OPC, &[0xFF, 0x07]);
    bus.write(0x07FF, 0x80); // lo
    bus.write(0x0700, 0xAB); // hi (page-wrap bug reads here)
    bus.write(0x0000, 0xCD); // $0800 mirrors $0000 — should NOT be read
    let mut cpu = Cpu::new();
    cpu.pc = OPC;
    let op = cpu.resolve(&bus, AddrMode::Indirect);
    assert_eq!(op, Operand::Address(0xAB80));
}

#[test]
fn indirect_page_wrap_at_00ff() {
    // JMP ($00FF): lo from $00FF, hi from $0000.
    let mut bus = bus_with(OPC, &[0xFF, 0x00]);
    bus.write(0x00FF, 0x11);
    bus.write(0x0000, 0x22);
    let mut cpu = Cpu::new();
    cpu.pc = OPC;
    let op = cpu.resolve(&bus, AddrMode::Indirect);
    assert_eq!(op, Operand::Address(0x2211));
}

// ---- indirect,X -----------------------------------------------------

#[test]
fn indirect_x_reads_pointer_at_zp_plus_x() {
    // LDA ($20,X) with X=$04 -> pointer at $24/$25.
    // $24/$25 contains $00/$50 -> effective $5000.
    let mut bus = bus_with(OPC, &[0x20]);
    write_ptr(&mut bus, 0x0024, 0x5000);
    let mut cpu = Cpu::new();
    cpu.pc = OPC;
    cpu.x = 0x04;
    let op = cpu.resolve(&bus, AddrMode::IndirectX);
    assert_eq!(op, Operand::Address(0x5000));
    assert_eq!(cpu.pc, OPC + 1);
}

#[test]
fn indirect_x_pointer_location_wraps_in_zero_page() {
    // operand=$FE, X=$05 -> pointer at $03/$04 (wraps).
    let mut bus = bus_with(OPC, &[0xFE]);
    write_ptr(&mut bus, 0x0003, 0xABCD);
    let mut cpu = Cpu::new();
    cpu.pc = OPC;
    cpu.x = 0x05;
    let op = cpu.resolve(&bus, AddrMode::IndirectX);
    assert_eq!(op, Operand::Address(0xABCD));
}

// ---- indirect,Y -----------------------------------------------------

#[test]
fn indirect_y_adds_y_to_indirect_base() {
    // LDA ($20),Y with pointer at $20/$21 = $3000, Y=$10 -> $3010.
    let mut bus = bus_with(OPC, &[0x20]);
    write_ptr(&mut bus, 0x0020, 0x3000);
    let mut cpu = Cpu::new();
    cpu.pc = OPC;
    cpu.y = 0x10;
    let op = cpu.resolve(&bus, AddrMode::IndirectY);
    assert_eq!(op, Operand::Address(0x3010));
    assert_eq!(cpu.pc, OPC + 1);
}

#[test]
fn indirect_y_pointer_wraps_in_zero_page() {
    // operand=$FF -> pointer at $FF/$00 (wraps).
    let mut bus = bus_with(OPC, &[0xFF]);
    bus.write(0x00FF, 0x34);
    bus.write(0x0000, 0x12);
    let mut cpu = Cpu::new();
    cpu.pc = OPC;
    cpu.y = 0x00;
    let op = cpu.resolve(&bus, AddrMode::IndirectY);
    assert_eq!(op, Operand::Address(0x1234));
}

#[test]
fn indirect_y_base_wraps_at_16_bit_boundary() {
    // pointer = $FFFF, Y=$02 -> $0001 (16-bit wrap).
    let mut bus = bus_with(OPC, &[0x20]);
    write_ptr(&mut bus, 0x0020, 0xFFFF);
    let mut cpu = Cpu::new();
    cpu.pc = OPC;
    cpu.y = 0x02;
    let op = cpu.resolve(&bus, AddrMode::IndirectY);
    assert_eq!(op, Operand::Address(0x0001));
}

// ---- relative -------------------------------------------------------

#[test]
fn relative_positive_offset_adds_to_next_pc() {
    // Operand byte $10 -> target = (PC after fetch) + 0x10.
    let bus = bus_with(OPC, &[0x10]);
    let mut cpu = Cpu::new();
    cpu.pc = OPC;
    let op = cpu.resolve(&bus, AddrMode::Relative);
    // After fetch PC=OPC+1=0x0201; +0x10 = 0x0211.
    assert_eq!(op, Operand::Address(0x0211));
    assert_eq!(cpu.pc, OPC + 1);
}

#[test]
fn relative_negative_offset_subtracts_from_next_pc() {
    // Operand byte $FE (-2) -> target = (PC after fetch) - 2 = opcode addr.
    let bus = bus_with(OPC, &[0xFE]);
    let mut cpu = Cpu::new();
    cpu.pc = OPC;
    let op = cpu.resolve(&bus, AddrMode::Relative);
    // After fetch PC=0x0201; -2 = 0x01FF.
    assert_eq!(op, Operand::Address(0x01FF));
}

#[test]
fn relative_wraps_at_16_bit_boundary() {
    // PC=$0000 (operand byte location), offset=$FE (-2): after fetch
    // next_pc=$0001; $0001 - 2 wraps to $FFFF.
    let bus = bus_with(0x0000, &[0xFE]);
    let mut cpu = Cpu::new();
    cpu.pc = 0x0000;
    let op = cpu.resolve(&bus, AddrMode::Relative);
    assert_eq!(op, Operand::Address(0xFFFF));
}

// ---- default --------------------------------------------------------

#[test]
fn default_matches_new() {
    let a = Cpu::new();
    let b = Cpu::default();
    assert_eq!(a.status, b.status);
    assert_eq!(a.sp, b.sp);
}
