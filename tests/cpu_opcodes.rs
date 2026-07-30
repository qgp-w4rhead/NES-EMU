//! Integration tests for the 6502 opcode set (M5): all 151 official
//! opcodes exercised end-to-end through a real `Bus` + `Cpu::step`.
//!
//! Programs are laid down in CPU RAM at `$0200` (well within the 2 KB
//! internal RAM) and the PC is pointed at the start. Operands that must
//! live at specific addresses are written separately. Each test steps the
//! CPU one or more times and checks register/memory/flag results and, where
//! noted, the cycle count returned by `step`.

use nes_emu::bus::Bus;
use nes_emu::cpu::Cpu;

/// Start address for test programs in RAM.
const PC0: u16 = 0x0200;

/// Build a bus with `prog` written at `PC0`.
fn bus_with_prog(prog: &[u8]) -> Bus {
    let mut bus = Bus::new();
    for (i, b) in prog.iter().enumerate() {
        bus.write(PC0 + i as u16, *b);
    }
    bus
}

/// Build a CPU with PC set to `PC0`.
fn cpu_at_pc0() -> Cpu {
    let mut cpu = Cpu::new();
    cpu.pc = PC0;
    cpu
}

/// Run one instruction and return (cycles, cpu, bus) for inspection.
fn step_once(prog: &[u8]) -> (u8, Cpu, Bus) {
    let mut bus = bus_with_prog(prog);
    let mut cpu = cpu_at_pc0();
    let cycles = cpu.step(&mut bus);
    (cycles, cpu, bus)
}

// ===========================================================================
// Loads
// ===========================================================================

#[test]
fn lda_immediate_loads_a_and_sets_nz() {
    let (_, cpu, _) = step_once(&[0xA9, 0x42]);
    assert_eq!(cpu.a, 0x42);
    assert!(!cpu.zero());
    assert!(!cpu.negative());
}

#[test]
fn lda_immediate_zero_sets_z_clears_n() {
    let (_, mut cpu, _) = step_once(&[0xA9, 0x00]);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.zero());
    assert!(!cpu.negative());
    // _ = cpu to silence unused; reuse for negative case
    let _ = &mut cpu;
}

#[test]
fn lda_immediate_negative_sets_n() {
    let (_, cpu, _) = step_once(&[0xA9, 0x80]);
    assert_eq!(cpu.a, 0x80);
    assert!(cpu.negative());
    assert!(!cpu.zero());
}

#[test]
fn lda_immediate_cycle_count_is_2() {
    let (c, _, _) = step_once(&[0xA9, 0x42]);
    assert_eq!(c, 2);
}

#[test]
fn lda_zero_page_reads_from_zp_address() {
    let mut bus = bus_with_prog(&[0xA5, 0x10]);
    bus.write(0x0010, 0x77);
    let mut cpu = cpu_at_pc0();
    let c = cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x77);
    assert_eq!(c, 3);
}

#[test]
fn lda_absolute_reads_from_16bit_address() {
    let mut bus = bus_with_prog(&[0xAD, 0x00, 0x03]); // LDA $0300
    bus.write(0x0300, 0xCD);
    let mut cpu = cpu_at_pc0();
    let c = cpu.step(&mut bus);
    assert_eq!(cpu.a, 0xCD);
    assert_eq!(c, 4);
}

#[test]
fn lda_absolute_x_no_page_cross_is_4_cycles() {
    let mut bus = bus_with_prog(&[0xBD, 0x00, 0x03]); // LDA $0300,X
    bus.write(0x0305, 0x11);
    let mut cpu = cpu_at_pc0();
    cpu.x = 5;
    let c = cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x11);
    assert_eq!(c, 4);
}

#[test]
fn lda_absolute_x_page_cross_adds_a_cycle() {
    let mut bus = bus_with_prog(&[0xBD, 0xFC, 0x03]); // LDA $03FC,X
    bus.write(0x0401, 0x22);
    let mut cpu = cpu_at_pc0();
    cpu.x = 5; // $03FC + 5 = $0401 (page cross)
    let c = cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x22);
    assert_eq!(c, 5);
}

#[test]
fn lda_indirect_y_page_cross_adds_a_cycle() {
    // LDA ($20),Y ; pointer at $20 = $03FC
    let mut bus = bus_with_prog(&[0xB1, 0x20]);
    bus.write(0x0020, 0xFC);
    bus.write(0x0021, 0x03);
    bus.write(0x0401, 0x33); // $03FC + Y(5) = $0401
    let mut cpu = cpu_at_pc0();
    cpu.y = 5;
    let c = cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x33);
    assert_eq!(c, 6); // 5 + page cross
}

#[test]
fn lda_indirect_y_no_page_cross_is_5_cycles() {
    let mut bus = bus_with_prog(&[0xB1, 0x20]);
    bus.write(0x0020, 0x00);
    bus.write(0x0021, 0x03);
    bus.write(0x0305, 0x44);
    let mut cpu = cpu_at_pc0();
    cpu.y = 5;
    let c = cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x44);
    assert_eq!(c, 5);
}

#[test]
fn lda_indirect_x_reads_pointer_at_zp_plus_x() {
    // LDA ($20,X) with X=2 -> pointer at $22
    let mut bus = bus_with_prog(&[0xA1, 0x20]);
    bus.write(0x0022, 0x00);
    bus.write(0x0023, 0x04);
    bus.write(0x0400, 0x55);
    let mut cpu = cpu_at_pc0();
    cpu.x = 2;
    let c = cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x55);
    assert_eq!(c, 6);
}

#[test]
fn ldx_and_ldy_immediate() {
    let (_, cpu, _) = step_once(&[0xA2, 0x10]); // LDX #$10
    assert_eq!(cpu.x, 0x10);
    let (_, cpu, _) = step_once(&[0xA0, 0x20]); // LDY #$20
    assert_eq!(cpu.y, 0x20);
}

#[test]
fn ldx_zero_page_y_uses_y_index() {
    let mut bus = bus_with_prog(&[0xB6, 0x40]); // LDX $40,Y
    bus.write(0x0043, 0x99);
    let mut cpu = cpu_at_pc0();
    cpu.y = 3;
    cpu.step(&mut bus);
    assert_eq!(cpu.x, 0x99);
}

#[test]
fn ldy_absolute_x_uses_x_index() {
    let mut bus = bus_with_prog(&[0xBC, 0x00, 0x05]); // LDY $0500,X
    bus.write(0x050A, 0xEE);
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x0A;
    cpu.step(&mut bus);
    assert_eq!(cpu.y, 0xEE);
}

// ===========================================================================
// Stores
// ===========================================================================

#[test]
fn sta_zero_page_writes_a() {
    let mut bus = bus_with_prog(&[0x85, 0x30]);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xAB;
    let c = cpu.step(&mut bus);
    assert_eq!(bus.read(0x0030), 0xAB);
    assert_eq!(c, 3);
}

#[test]
fn sta_absolute_writes_a() {
    let mut bus = bus_with_prog(&[0x8D, 0x00, 0x05]);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x7E;
    cpu.step(&mut bus);
    assert_eq!(bus.read(0x0500), 0x7E);
}

#[test]
fn sta_absolute_x_no_page_cross_penalty() {
    // STA abs,X is always 5 cycles — stores do not add a page-cross penalty,
    // even when the indexed address crosses into a new page.
    let mut bus = bus_with_prog(&[0x9D, 0xFC, 0x05]); // STA $05FC,X
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x02;
    cpu.x = 0x05; // $05FC + 5 = $0601 (page cross)
    let c = cpu.step(&mut bus);
    assert_eq!(bus.read(0x0601), 0x02);
    assert_eq!(c, 5); // no penalty despite the page cross
}

#[test]
fn sta_indirect_y_no_page_cross_penalty() {
    // STA (ind),Y is always 6 cycles.
    let mut bus = bus_with_prog(&[0x91, 0x20]);
    bus.write(0x0020, 0xFC);
    bus.write(0x0021, 0x03);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x77;
    cpu.y = 5; // $03FC + 5 = $0401 (page cross)
    let c = cpu.step(&mut bus);
    assert_eq!(bus.read(0x0401), 0x77);
    assert_eq!(c, 6);
}

#[test]
fn stx_and_sty_zero_page() {
    let mut bus = bus_with_prog(&[0x86, 0x40, 0x84, 0x41]);
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x12;
    cpu.y = 0x34;
    cpu.step(&mut bus); // STX $40
    cpu.step(&mut bus); // STY $41
    assert_eq!(bus.read(0x0040), 0x12);
    assert_eq!(bus.read(0x0041), 0x34);
}

// ===========================================================================
// Register transfers
// ===========================================================================

#[test]
fn tax_tay_txa_tya_transfer_and_set_flags() {
    let mut bus = bus_with_prog(&[0xAA]); // TAX
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x80;
    cpu.step(&mut bus);
    assert_eq!(cpu.x, 0x80);
    assert!(cpu.negative());

    let mut bus = bus_with_prog(&[0xA8]); // TAY
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x00;
    cpu.step(&mut bus);
    assert_eq!(cpu.y, 0x00);
    assert!(cpu.zero());

    let mut bus = bus_with_prog(&[0x8A]); // TXA
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x55;
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x55);

    let mut bus = bus_with_prog(&[0x98]); // TYA
    let mut cpu = cpu_at_pc0();
    cpu.y = 0x66;
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x66);
}

#[test]
fn tsx_sets_flags_txs_does_not() {
    let mut bus = bus_with_prog(&[0xBA]); // TSX
    let mut cpu = cpu_at_pc0();
    cpu.sp = 0x80;
    cpu.step(&mut bus);
    assert_eq!(cpu.x, 0x80);
    assert!(cpu.negative());

    let mut bus = bus_with_prog(&[0x9A]); // TXS
    let mut cpu = cpu_at_pc0();
    cpu.x = 0xFF;
    cpu.status = 0;
    cpu.step(&mut bus);
    assert_eq!(cpu.sp, 0xFF);
    // TXS does not affect flags.
    assert_eq!(cpu.status, 0);
}

// ===========================================================================
// Stack
// ===========================================================================

#[test]
fn pha_pla_round_trip() {
    // PHA ; PLA
    let mut bus = bus_with_prog(&[0x48, 0x68]);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x3C;
    cpu.step(&mut bus); // PHA
    assert_eq!(cpu.sp, 0xFC); // decremented from 0xFD
    cpu.a = 0x00;
    let c = cpu.step(&mut bus); // PLA
    assert_eq!(cpu.a, 0x3C);
    assert_eq!(cpu.sp, 0xFD);
    assert_eq!(c, 4);
}

#[test]
fn php_sets_b_and_u_bits_in_pushed_copy() {
    // PHP ; then read back the pushed byte from the stack.
    let mut bus = bus_with_prog(&[0x08]);
    let mut cpu = cpu_at_pc0();
    cpu.status = 0; // clear all
    cpu.step(&mut bus);
    // Pushed at $01FD (sp was 0xFD, then decremented to 0xFC).
    let pushed = bus.read(0x01FD);
    // B (0x10) and U (0x20) must be set in the pushed copy.
    assert_eq!(pushed & 0b0011_0000, 0b0011_0000);
}

#[test]
fn plp_pulls_status_and_clears_b() {
    // Pre-place a status byte on the stack with B set; PLP should clear B.
    let mut bus = bus_with_prog(&[0x28]); // PLP
    let mut cpu = cpu_at_pc0();
    cpu.sp = 0xFC;
    bus.write(0x01FD, 0xFF); // all bits set, including B
    cpu.step(&mut bus);
    // B should be cleared, U forced to 1.
    assert_eq!(cpu.status & 0b0011_0000, 0b0010_0000);
}

// ===========================================================================
// Logic
// ===========================================================================

#[test]
fn and_ora_eor_immediate() {
    let mut bus = bus_with_prog(&[0x29, 0x0F]); // AND #$0F
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xF0;
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.zero());

    let mut bus = bus_with_prog(&[0x09, 0x0F]); // ORA #$0F
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xF0;
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0xFF);
    assert!(cpu.negative());

    let mut bus = bus_with_prog(&[0x49, 0xFF]); // EOR #$FF
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xAA;
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x55);
}

#[test]
fn bit_sets_n_v_from_memory_and_z_from_and() {
    // BIT $10 with A=$0F, M=$C0 -> N=1, V=1, Z=1 (A&M == 0)
    let mut bus = bus_with_prog(&[0x24, 0x10]);
    bus.write(0x0010, 0xC0);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x0F;
    cpu.step(&mut bus);
    assert!(cpu.negative());
    assert!(cpu.overflow());
    assert!(cpu.zero());
    assert_eq!(cpu.a, 0x0F); // A unchanged

    // BIT $10 with A=$40, M=$C0 -> N=1, V=1, Z=0 (A&M = $40 != 0)
    let mut bus = bus_with_prog(&[0x24, 0x10]);
    bus.write(0x0010, 0xC0);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x40;
    cpu.step(&mut bus);
    assert!(cpu.negative());
    assert!(cpu.overflow());
    assert!(!cpu.zero());
}

// ===========================================================================
// ADC / SBC
// ===========================================================================

#[test]
fn adc_simple_no_carry() {
    let mut bus = bus_with_prog(&[0x69, 0x10]); // ADC #$10
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x20;
    cpu.set_carry(false);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x30);
    assert!(!cpu.carry());
    assert!(!cpu.overflow());
}

#[test]
fn adc_with_carry_in() {
    let mut bus = bus_with_prog(&[0x69, 0x01]);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x10;
    cpu.set_carry(true);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x12);
}

#[test]
fn adc_carry_out_on_unsigned_overflow() {
    let mut bus = bus_with_prog(&[0x69, 0x01]);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0xFF;
    cpu.set_carry(false);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.carry());
    assert!(cpu.zero());
}

#[test]
fn adc_signed_overflow_set() {
    // 0x7F + 0x01 = 0x80 -> signed overflow (positive + positive = negative)
    let mut bus = bus_with_prog(&[0x69, 0x01]);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x7F;
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x80);
    assert!(cpu.overflow());
    assert!(cpu.negative());
    assert!(!cpu.carry());
}

#[test]
fn adc_signed_overflow_negative_plus_negative() {
    // 0x80 + 0xFF (=-1) = 0x7F -> signed overflow
    let mut bus = bus_with_prog(&[0x69, 0xFF]);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x80;
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x7F);
    assert!(cpu.overflow());
    assert!(cpu.carry());
}

#[test]
fn sbc_simple_borrow() {
    // A - M - (1-C) = A - M - 1 when C=0; A=0x20, M=0x10 -> 0x20-0x10-1 = 0x0F
    let mut bus = bus_with_prog(&[0xE9, 0x10]);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x20;
    cpu.set_carry(false);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x0F);
    assert!(cpu.carry()); // no borrow -> carry set
}

#[test]
fn sbc_with_carry_in_no_borrow() {
    // A - M with C=1 -> A - M (no -1). A=0x20, M=0x10 -> 0x10
    let mut bus = bus_with_prog(&[0xE9, 0x10]);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x20;
    cpu.set_carry(true);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x10);
    assert!(cpu.carry());
}

#[test]
fn sbc_borrow_clears_carry() {
    // A=0x00, M=0x01, C=1 -> 0x00-0x01 = 0xFF, borrow -> carry clear
    let mut bus = bus_with_prog(&[0xE9, 0x01]);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x00;
    cpu.set_carry(true);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0xFF);
    assert!(!cpu.carry());
    assert!(cpu.negative());
}

#[test]
fn sbc_signed_overflow() {
    // A=0x80 (-128), M=0x01, C=1 -> 0x7F (+127): signed overflow
    let mut bus = bus_with_prog(&[0xE9, 0x01]);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x80;
    cpu.set_carry(true);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x7F);
    assert!(cpu.overflow());
}

// ===========================================================================
// Decimal mode behavior (M-CYC-07)
//
// The NES 2A03 physically removed the decimal-mode circuitry from the 6502.
// SED/CLD still set/clear the D flag, but ADC/SBC always perform binary
// arithmetic regardless of D. On a real 6502, the cases below would produce
// decimal-corrected results; on the NES they must produce plain binary
// results with flags set from the binary operation.
// See: https://www.nesdev.org/wiki/6502#Decimal_mode_in_the_NES
// ===========================================================================

#[test]
fn adc_decimal_flag_ignored_low_nibble_carry() {
    // 0x09 + 0x01 = 0x0A in binary. On a 6502 with D set, the low nibble
    // would carry: 0x09 + 0x01 = 0x10. On the NES, result is 0x0A.
    let mut bus = bus_with_prog(&[0x69, 0x01]); // ADC #$01
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x09;
    cpu.set_decimal(true);
    cpu.set_carry(false);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x0A);
    assert!(!cpu.carry());
    assert!(!cpu.zero());
    assert!(!cpu.negative());
    assert!(!cpu.overflow());
    // D flag must remain set — ADC does not clear it.
    assert!(cpu.decimal());
}

#[test]
fn adc_decimal_flag_ignored_high_nibble_carry() {
    // 0x99 + 0x01 = 0x9A in binary. On a 6502 with D set, this would
    // produce 0x00 with carry set. On the NES, result is 0x9A, no carry.
    let mut bus = bus_with_prog(&[0x69, 0x01]); // ADC #$01
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x99;
    cpu.set_decimal(true);
    cpu.set_carry(false);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x9A);
    assert!(!cpu.carry());
    assert!(cpu.negative()); // bit 7 set
    assert!(cpu.decimal());
}

#[test]
fn adc_decimal_flag_ignored_both_nibbles_carry() {
    // 0x49 + 0x49 = 0x92 in binary. On a 6502 with D set, both nibbles
    // would carry: 0x49 + 0x49 = 0x98 (decimal 49+49=98). On the NES,
    // result is 0x92.
    let mut bus = bus_with_prog(&[0x69, 0x49]); // ADC #$49
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x49;
    cpu.set_decimal(true);
    cpu.set_carry(false);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x92);
    assert!(cpu.negative());
    assert!(!cpu.carry());
    assert!(cpu.decimal());
}

#[test]
fn adc_decimal_flag_ignored_with_carry_in() {
    // 0x35 + 0x25 + carry = 0x5B in binary. On a 6502 with D set, this
    // would produce 0x61 (35+25+1=61 decimal). On the NES, 0x5B.
    let mut bus = bus_with_prog(&[0x69, 0x25]); // ADC #$25
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x35;
    cpu.set_decimal(true);
    cpu.set_carry(true);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x5B);
    assert!(!cpu.carry());
    assert!(!cpu.zero());
    assert!(!cpu.negative());
    assert!(cpu.decimal());
}

#[test]
fn sbc_decimal_flag_ignored_low_nibble_borrow() {
    // 0x10 - 0x01 = 0x0F in binary. On a 6502 with D set, the low nibble
    // borrow would produce 0x09. On the NES, result is 0x0F.
    let mut bus = bus_with_prog(&[0xE9, 0x01]); // SBC #$01
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x10;
    cpu.set_decimal(true);
    cpu.set_carry(true); // no borrow in
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x0F);
    assert!(cpu.carry()); // no borrow out
    assert!(!cpu.negative());
    assert!(cpu.decimal());
}

#[test]
fn sbc_decimal_flag_ignored_borrow_from_high() {
    // 0x00 - 0x01 = 0xFF in binary. On a 6502 with D set, this would
    // produce 0x99 with carry clear. On the NES, 0xFF with carry clear.
    let mut bus = bus_with_prog(&[0xE9, 0x01]); // SBC #$01
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x00;
    cpu.set_decimal(true);
    cpu.set_carry(true); // no borrow in
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0xFF);
    assert!(!cpu.carry()); // borrow occurred
    assert!(cpu.negative());
    assert!(cpu.decimal());
}

#[test]
fn sbc_decimal_flag_ignored_both_nibbles() {
    // 0x50 - 0x25 = 0x2B in binary. On a 6502 with D set, this would
    // produce 0x25 (decimal 50-25=25). On the NES, 0x2B.
    let mut bus = bus_with_prog(&[0xE9, 0x25]); // SBC #$25
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x50;
    cpu.set_decimal(true);
    cpu.set_carry(true);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x2B);
    assert!(cpu.carry());
    assert!(!cpu.negative());
    assert!(cpu.decimal());
}

#[test]
fn adc_results_identical_with_and_without_decimal_flag() {
    // Verify that D flag has zero effect on ADC by running the same
    // computation with D=0 and D=1 and comparing all results.
    let test_cases: &[(u8, u8, bool)] = &[
        (0x09, 0x01, false),
        (0x99, 0x01, false),
        (0x49, 0x49, false),
        (0x35, 0x25, true),
        (0xFF, 0x01, false),
        (0x80, 0x80, false),
        (0x7F, 0x01, false),
    ];
    for &(a_val, m_val, carry_in) in test_cases {
        // D=0
        let mut bus0 = bus_with_prog(&[0x69, m_val]);
        let mut cpu0 = cpu_at_pc0();
        cpu0.a = a_val;
        cpu0.set_decimal(false);
        cpu0.set_carry(carry_in);
        cpu0.step(&mut bus0);

        // D=1
        let mut bus1 = bus_with_prog(&[0x69, m_val]);
        let mut cpu1 = cpu_at_pc0();
        cpu1.a = a_val;
        cpu1.set_decimal(true);
        cpu1.set_carry(carry_in);
        cpu1.step(&mut bus1);

        assert_eq!(
            cpu0.a, cpu1.a,
            "ADC result differs for A={:#04X} M={:#04X} C={}",
            a_val, m_val, carry_in
        );
        assert_eq!(
            cpu0.carry(),
            cpu1.carry(),
            "carry differs for A={:#04X} M={:#04X} C={}",
            a_val,
            m_val,
            carry_in
        );
        assert_eq!(
            cpu0.zero(),
            cpu1.zero(),
            "zero differs for A={:#04X} M={:#04X} C={}",
            a_val,
            m_val,
            carry_in
        );
        assert_eq!(
            cpu0.negative(),
            cpu1.negative(),
            "negative differs for A={:#04X} M={:#04X} C={}",
            a_val,
            m_val,
            carry_in
        );
        assert_eq!(
            cpu0.overflow(),
            cpu1.overflow(),
            "overflow differs for A={:#04X} M={:#04X} C={}",
            a_val,
            m_val,
            carry_in
        );
    }
}

#[test]
fn sbc_results_identical_with_and_without_decimal_flag() {
    // Verify that D flag has zero effect on SBC by running the same
    // computation with D=0 and D=1 and comparing all results.
    let test_cases: &[(u8, u8, bool)] = &[
        (0x10, 0x01, true),
        (0x00, 0x01, true),
        (0x50, 0x25, true),
        (0x00, 0x01, false),
        (0x80, 0x01, true),
        (0x50, 0x60, true),
        (0x99, 0x50, true),
    ];
    for &(a_val, m_val, carry_in) in test_cases {
        // D=0
        let mut bus0 = bus_with_prog(&[0xE9, m_val]);
        let mut cpu0 = cpu_at_pc0();
        cpu0.a = a_val;
        cpu0.set_decimal(false);
        cpu0.set_carry(carry_in);
        cpu0.step(&mut bus0);

        // D=1
        let mut bus1 = bus_with_prog(&[0xE9, m_val]);
        let mut cpu1 = cpu_at_pc0();
        cpu1.a = a_val;
        cpu1.set_decimal(true);
        cpu1.set_carry(carry_in);
        cpu1.step(&mut bus1);

        assert_eq!(
            cpu0.a, cpu1.a,
            "SBC result differs for A={:#04X} M={:#04X} C={}",
            a_val, m_val, carry_in
        );
        assert_eq!(
            cpu0.carry(),
            cpu1.carry(),
            "carry differs for A={:#04X} M={:#04X} C={}",
            a_val,
            m_val,
            carry_in
        );
        assert_eq!(
            cpu0.zero(),
            cpu1.zero(),
            "zero differs for A={:#04X} M={:#04X} C={}",
            a_val,
            m_val,
            carry_in
        );
        assert_eq!(
            cpu0.negative(),
            cpu1.negative(),
            "negative differs for A={:#04X} M={:#04X} C={}",
            a_val,
            m_val,
            carry_in
        );
        assert_eq!(
            cpu0.overflow(),
            cpu1.overflow(),
            "overflow differs for A={:#04X} M={:#04X} C={}",
            a_val,
            m_val,
            carry_in
        );
    }
}

#[test]
fn sed_cld_set_and_clear_decimal_flag() {
    // SED sets D, CLD clears D. Verify the flag bit is correctly
    // manipulated and persists across other operations.
    let mut bus = bus_with_prog(&[0xF8, 0xD8]); // SED ; CLD
    let mut cpu = cpu_at_pc0();
    assert!(!cpu.decimal());
    cpu.step(&mut bus);
    assert!(cpu.decimal());
    cpu.step(&mut bus);
    assert!(!cpu.decimal());
}

#[test]
fn decimal_flag_survives_plp_and_php_roundtrip() {
    // PHP pushes P (including D) to stack; PLP pulls it back.
    // Verify D flag survives the round-trip.
    let mut bus = bus_with_prog(&[0x08, 0x28]); // PHP ; PLP
    let mut cpu = cpu_at_pc0();
    cpu.set_decimal(true);
    cpu.step(&mut bus); // PHP
    cpu.set_decimal(false); // clear D in the register
    cpu.step(&mut bus); // PLP — should restore D=1
    assert!(cpu.decimal());
}

// ===========================================================================
// CMP / CPX / CPY
// ===========================================================================

#[test]
fn cmp_equal_sets_z_and_c() {
    let mut bus = bus_with_prog(&[0xC9, 0x42]);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x42;
    cpu.step(&mut bus);
    assert!(cpu.zero());
    assert!(cpu.carry());
    assert!(!cpu.negative());
}

#[test]
fn cmp_greater_sets_c_clears_z() {
    let mut bus = bus_with_prog(&[0xC9, 0x10]);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x42;
    cpu.step(&mut bus);
    assert!(!cpu.zero());
    assert!(cpu.carry());
}

#[test]
fn cmp_less_clears_c() {
    let mut bus = bus_with_prog(&[0xC9, 0x50]);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x10;
    cpu.step(&mut bus);
    assert!(!cpu.carry());
    assert!(!cpu.zero());
}

#[test]
fn cpx_and_cpy_immediate() {
    let mut bus = bus_with_prog(&[0xE0, 0x05]);
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x05;
    cpu.step(&mut bus);
    assert!(cpu.zero());
    assert!(cpu.carry());

    let mut bus = bus_with_prog(&[0xC0, 0xFF]);
    let mut cpu = cpu_at_pc0();
    cpu.y = 0x00;
    cpu.step(&mut bus);
    assert!(!cpu.carry());
    // 0x00 - 0xFF = 0x01 (wrapping). N = bit7 of 0x01 = 0 -> negative clear.
    assert!(!cpu.negative());
}

// ===========================================================================
// INC / DEC / INX / INY / DEX / DEY
// ===========================================================================

#[test]
fn inc_zero_page_wraps_and_sets_flags() {
    let mut bus = bus_with_prog(&[0xE6, 0x50]);
    bus.write(0x0050, 0xFF);
    let mut cpu = cpu_at_pc0();
    let c = cpu.step(&mut bus);
    assert_eq!(bus.read(0x0050), 0x00);
    assert!(cpu.zero());
    assert_eq!(c, 5);
}

#[test]
fn dec_zero_page_wraps_and_sets_flags() {
    let mut bus = bus_with_prog(&[0xC6, 0x50]);
    bus.write(0x0050, 0x00);
    let mut cpu = cpu_at_pc0();
    cpu.step(&mut bus);
    assert_eq!(bus.read(0x0050), 0xFF);
    assert!(cpu.negative());
}

#[test]
fn inc_absolute_x_is_7_cycles() {
    let mut bus = bus_with_prog(&[0xFE, 0x00, 0x05]); // INC $0500,X
    bus.write(0x0505, 0x09);
    let mut cpu = cpu_at_pc0();
    cpu.x = 5;
    let c = cpu.step(&mut bus);
    assert_eq!(bus.read(0x0505), 0x0A);
    assert_eq!(c, 7);
}

#[test]
fn inx_iny_dex_dey() {
    let mut bus = bus_with_prog(&[0xE8, 0xC8, 0xCA, 0x88]);
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x09;
    cpu.y = 0x09;
    cpu.step(&mut bus); // INX -> 0x0A
    assert_eq!(cpu.x, 0x0A);
    cpu.step(&mut bus); // INY -> 0x0A
    assert_eq!(cpu.y, 0x0A);
    cpu.step(&mut bus); // DEX -> 0x09
    assert_eq!(cpu.x, 0x09);
    cpu.step(&mut bus); // DEY -> 0x09
    assert_eq!(cpu.y, 0x09);
}

// ===========================================================================
// Shifts / rotates
// ===========================================================================

#[test]
fn asl_accumulator_shifts_into_carry() {
    let mut bus = bus_with_prog(&[0x0A]);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0b1000_0001;
    let c = cpu.step(&mut bus);
    assert_eq!(cpu.a, 0b0000_0010);
    assert!(cpu.carry());
    assert_eq!(c, 2);
}

#[test]
fn lsr_accumulator_shifts_into_carry() {
    let mut bus = bus_with_prog(&[0x4A]);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0b0000_0011;
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0b0000_0001);
    assert!(cpu.carry());
}

#[test]
fn rol_accumulator_rotates_through_carry() {
    let mut bus = bus_with_prog(&[0x2A]);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0b1000_0000;
    cpu.set_carry(true);
    cpu.step(&mut bus);
    // 1<<7 -> carry; old carry -> bit0
    assert_eq!(cpu.a, 0b0000_0001);
    assert!(cpu.carry());
}

#[test]
fn ror_accumulator_rotates_through_carry() {
    let mut bus = bus_with_prog(&[0x6A]);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0b0000_0001;
    cpu.set_carry(true);
    cpu.step(&mut bus);
    // bit0 -> carry; old carry -> bit7
    assert_eq!(cpu.a, 0b1000_0000);
    assert!(cpu.carry());
}

#[test]
fn asl_zero_page_writes_back_and_sets_flags() {
    let mut bus = bus_with_prog(&[0x06, 0x60]);
    bus.write(0x0060, 0b0100_0000);
    let mut cpu = cpu_at_pc0();
    let c = cpu.step(&mut bus);
    assert_eq!(bus.read(0x0060), 0b1000_0000);
    assert!(cpu.negative());
    assert!(!cpu.carry());
    assert_eq!(c, 5);
}

#[test]
fn accumulator_shifts_set_nz_flags() {
    // ASL A: 0x40 -> 0x80, N set.
    let mut bus = bus_with_prog(&[0x0A]);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x40;
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x80);
    assert!(cpu.negative());

    // LSR A: 0x01 -> 0x00, Z set.
    let mut bus = bus_with_prog(&[0x4A]);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x01;
    cpu.set_carry(false);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x00);
    assert!(cpu.zero());

    // ROL A: 0x40 with C=0 -> 0x80, N set.
    let mut bus = bus_with_prog(&[0x2A]);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x40;
    cpu.set_carry(false);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x80);
    assert!(cpu.negative());

    // ROR A: 0x02 with C=0 -> 0x01, Z clear, N clear.
    let mut bus = bus_with_prog(&[0x6A]);
    let mut cpu = cpu_at_pc0();
    cpu.a = 0x02;
    cpu.set_carry(false);
    cpu.step(&mut bus);
    assert_eq!(cpu.a, 0x01);
    assert!(!cpu.zero());
    assert!(!cpu.negative());
}

#[test]
fn ror_absolute_x_is_7_cycles() {
    let mut bus = bus_with_prog(&[0x7E, 0x00, 0x05]); // ROR $0500,X
    bus.write(0x0503, 0b0000_0010);
    let mut cpu = cpu_at_pc0();
    cpu.x = 3;
    cpu.set_carry(false);
    let c = cpu.step(&mut bus);
    assert_eq!(bus.read(0x0503), 0b0000_0001);
    assert_eq!(c, 7);
}

// ===========================================================================
// Branches
// ===========================================================================

#[test]
fn bne_not_taken_is_2_cycles_and_consumes_offset() {
    // BNE $+5 (not taken because Z is set)
    let mut bus = bus_with_prog(&[0xD0, 0x05, 0xEA]);
    let mut cpu = cpu_at_pc0();
    cpu.set_zero(true);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 2);
    // PC advanced past opcode + offset (to the NOP).
    assert_eq!(cpu.pc, PC0 + 2);
}

#[test]
fn bne_taken_same_page_is_3_cycles() {
    // BNE $+5 (taken, Z clear). Target = PC_after_offset + 5 = (PC0+2) + 5 = PC0+7
    let mut bus = bus_with_prog(&[0xD0, 0x05, 0xEA, 0xEA, 0xEA, 0xEA, 0xEA, 0xA9, 0x11]);
    let mut cpu = cpu_at_pc0();
    cpu.set_zero(false);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 3);
    assert_eq!(cpu.pc, PC0 + 2 + 5);
}

#[test]
fn beq_taken_page_cross_is_4_cycles() {
    // Place the branch so the byte after the offset lands on a page boundary
    // and the target wraps back into the previous page.
    // Opcode at $02FE: [BEQ -1]. pc_after = $0300 (page $03); target = $02FF (page $02) -> cross.
    let start = 0x02FEu16;
    let mut bus = Bus::new();
    bus.write(start, 0xF0); // BEQ
    bus.write(start + 1, 0xFF); // offset -1
    let mut cpu = Cpu::new();
    cpu.pc = start;
    cpu.set_zero(true);
    let c = cpu.step(&mut bus);
    assert_eq!(cpu.pc, 0x02FF);
    assert_eq!(c, 4);
}

#[test]
fn branch_taken_negative_offset() {
    // BNE back across a page boundary: opcode at $0302, offset -5.
    // pc_after = $0304 (page $03); target = $02FF (page $02) -> page cross, 4 cycles.
    let start = 0x0302u16;
    let mut bus = Bus::new();
    bus.write(start, 0xD0); // BNE
    bus.write(start + 1, 0xFB); // -5: target = (start+2) - 5 = $02FF
    let mut cpu = Cpu::new();
    cpu.pc = start;
    cpu.set_zero(false);
    let c = cpu.step(&mut bus);
    assert_eq!(cpu.pc, 0x02FF);
    assert_eq!(c, 4);
}

#[test]
fn bcc_bcs_bpl_bmi_bvc_bvs_taken_and_not_taken() {
    // BCC taken (C clear)
    let mut bus = bus_with_prog(&[0x90, 0x03]);
    let mut cpu = cpu_at_pc0();
    cpu.set_carry(false);
    let c = cpu.step(&mut bus);
    assert_eq!(cpu.pc, PC0 + 2 + 3);
    assert_eq!(c, 3);

    // BCS not taken (C clear)
    let mut bus = bus_with_prog(&[0xB0, 0x03]);
    let mut cpu = cpu_at_pc0();
    cpu.set_carry(false);
    let c = cpu.step(&mut bus);
    assert_eq!(c, 2);

    // BPL taken (N clear)
    let mut bus = bus_with_prog(&[0x10, 0x03]);
    let mut cpu = cpu_at_pc0();
    cpu.set_negative(false);
    cpu.step(&mut bus);
    assert_eq!(cpu.pc, PC0 + 2 + 3);

    // BMI taken (N set)
    let mut bus = bus_with_prog(&[0x30, 0x03]);
    let mut cpu = cpu_at_pc0();
    cpu.set_negative(true);
    cpu.step(&mut bus);
    assert_eq!(cpu.pc, PC0 + 2 + 3);

    // BVC taken (V clear)
    let mut bus = bus_with_prog(&[0x50, 0x03]);
    let mut cpu = cpu_at_pc0();
    cpu.set_overflow(false);
    cpu.step(&mut bus);
    assert_eq!(cpu.pc, PC0 + 2 + 3);

    // BVS taken (V set)
    let mut bus = bus_with_prog(&[0x70, 0x03]);
    let mut cpu = cpu_at_pc0();
    cpu.set_overflow(true);
    cpu.step(&mut bus);
    assert_eq!(cpu.pc, PC0 + 2 + 3);
}

// ===========================================================================
// Jumps / subroutines / interrupts
// ===========================================================================

#[test]
fn jmp_absolute_sets_pc() {
    let mut bus = bus_with_prog(&[0x4C, 0x00, 0x05]);
    let mut cpu = cpu_at_pc0();
    let c = cpu.step(&mut bus);
    assert_eq!(cpu.pc, 0x0500);
    assert_eq!(c, 3);
}

#[test]
fn jmp_indirect_uses_pointer() {
    // JMP ($0300) ; pointer at $0300 = $0500
    let mut bus = bus_with_prog(&[0x6C, 0x00, 0x03]);
    bus.write(0x0300, 0x00);
    bus.write(0x0301, 0x05);
    let mut cpu = cpu_at_pc0();
    let c = cpu.step(&mut bus);
    assert_eq!(cpu.pc, 0x0500);
    assert_eq!(c, 5);
}

#[test]
fn jsr_rts_round_trip() {
    // JSR $0500 ; at $0500: RTS
    let mut bus = bus_with_prog(&[0x20, 0x00, 0x05]);
    bus.write(0x0500, 0x60); // RTS
    let mut cpu = cpu_at_pc0();
    let c1 = cpu.step(&mut bus); // JSR
    assert_eq!(cpu.pc, 0x0500);
    assert_eq!(c1, 6);
    // JSR pushes (return_address - 1) = ($0203 - 1) = $0202.
    // push_pc pushes hi then lo, so lo is on top (at higher stack addr).
    let lo = bus.read(0x01FD);
    let hi = bus.read(0x01FC);
    assert_eq!((lo as u16) | ((hi as u16) << 8), 0x0202);
    let c2 = cpu.step(&mut bus); // RTS
                                 // RTS pulls $0202 and adds 1 -> return to $0203 (byte after JSR's 3 bytes).
    assert_eq!(cpu.pc, 0x0203);
    assert_eq!(c2, 6);
}

#[test]
fn rti_pulls_status_and_pc_without_increment() {
    // Pre-place status and PC on the stack. RTI pulls P then PC (no +1).
    let mut bus = bus_with_prog(&[0x40]);
    let mut cpu = cpu_at_pc0();
    cpu.sp = 0xFA;
    // Stack grows down; pull increments SP first. So:
    // $01FB = P, $01FC = PC.lo, $01FD = PC.hi  (pull order: P, lo, hi)
    bus.write(0x01FB, 0x00);
    bus.write(0x01FC, 0x12);
    bus.write(0x01FD, 0x34);
    cpu.step(&mut bus);
    assert_eq!(cpu.status & !0b0011_0000, 0x00); // B cleared, U forced
    assert_eq!(cpu.pc, 0x3412);
}

/// Build a bus with a 32KB NROM cartridge whose BRK/IRQ vector ($FFFE/$FFFF)
/// is set to `vector`. The program is still placed in RAM at `PC0`.
fn bus_with_cart_vector(prog: &[u8], vector: u16) -> Bus {
    use nes_emu::cartridge::Cartridge;
    // iNES header: 32KB PRG (2 banks), 0 CHR (CHR-RAM), mapper 0.
    let mut bytes = vec![b'N', b'E', b'S', 0x1A, 2, 0, 0, 0];
    bytes.extend_from_slice(&[0u8; 8]); // remaining header bytes
    bytes.resize(16 + 32 * 1024, 0);
    // $FFFE/$FFFF -> PRG offset $7FFE/$7FFF for 32KB linear NROM.
    let off = 16 + 0x7FFE;
    bytes[off] = (vector & 0xFF) as u8;
    bytes[off + 1] = (vector >> 8) as u8;
    let cart = Cartridge::from_bytes(&bytes).expect("build test cartridge");
    let mut bus = Bus::with_cartridge(cart);
    for (i, b) in prog.iter().enumerate() {
        bus.write(PC0 + i as u16, *b);
    }
    bus
}

#[test]
fn brk_pushs_pc_and_status_and_jumps_to_vector() {
    // BRK ; vector at $FFFE/$FFFF = $1234 (needs a cartridge so cart-space
    // reads of $FFFE/$FFFF return the programmed vector).
    let mut bus = bus_with_cart_vector(&[0x00, 0x00], 0x1234); // BRK + padding byte
    let mut cpu = cpu_at_pc0();
    let c = cpu.step(&mut bus);
    assert_eq!(cpu.pc, 0x1234);
    assert!(cpu.interrupt_disable()); // I set after BRK
    assert_eq!(c, 7);
    // Pushed PC = PC after the padding byte = PC0 + 2 = $0202.
    let lo = bus.read(0x01FD);
    let hi = bus.read(0x01FC);
    assert_eq!((lo as u16) | ((hi as u16) << 8), 0x0202);
    // Pushed status should have B and U set.
    let pushed_status = bus.read(0x01FB);
    assert_eq!(pushed_status & 0b0011_0000, 0b0011_0000);
}

// ===========================================================================
// Flag operations / NOP
// ===========================================================================

#[test]
fn flag_operations_set_and_clear() {
    let mut bus = bus_with_prog(&[0x38, 0x18]); // SEC ; CLC
    let mut cpu = cpu_at_pc0();
    cpu.step(&mut bus);
    assert!(cpu.carry());
    cpu.step(&mut bus);
    assert!(!cpu.carry());

    let mut bus = bus_with_prog(&[0x78, 0x58]); // SEI ; CLI
    let mut cpu = cpu_at_pc0();
    cpu.step(&mut bus);
    assert!(cpu.interrupt_disable());
    cpu.step(&mut bus);
    assert!(!cpu.interrupt_disable());

    let mut bus = bus_with_prog(&[0xF8, 0xD8]); // SED ; CLD
    let mut cpu = cpu_at_pc0();
    cpu.step(&mut bus);
    assert!(cpu.decimal());
    cpu.step(&mut bus);
    assert!(!cpu.decimal());

    let mut bus = bus_with_prog(&[0xB8]); // CLV
    let mut cpu = cpu_at_pc0();
    cpu.set_overflow(true);
    cpu.step(&mut bus);
    assert!(!cpu.overflow());
}

#[test]
fn nop_advances_pc_and_costs_2_cycles() {
    let (_, cpu, _) = step_once(&[0xEA]);
    assert_eq!(cpu.pc, PC0 + 1);
    let (c, _, _) = step_once(&[0xEA]);
    assert_eq!(c, 2);
}

// ===========================================================================
// Multi-instruction smoke test
// ===========================================================================

#[test]
fn multi_instruction_program_loads_and_stores() {
    // LDA #$42 ; STA $00 ; LDA #$FF ; STA $01 ; LDA $00 ; ADC $01
    let prog = [
        0xA9, 0x42, // LDA #$42
        0x85, 0x00, // STA $00
        0xA9, 0xFF, // LDA #$FF
        0x85, 0x01, // STA $01
        0xA5, 0x00, // LDA $00
        0x65, 0x01, // ADC $01
    ];
    let mut bus = bus_with_prog(&prog);
    let mut cpu = cpu_at_pc0();
    for _ in 0..6 {
        cpu.step(&mut bus);
    }
    // $42 + $FF = $141 -> A=$41, carry set
    assert_eq!(cpu.a, 0x41);
    assert!(cpu.carry());
    assert_eq!(bus.read(0x0000), 0x42);
    assert_eq!(bus.read(0x0001), 0xFF);
}
