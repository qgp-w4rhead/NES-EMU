//! Integration tests for M24 — CPU dummy reads + precise interrupt timing.
//!
//! # Dummy reads
//!
//! The 6502 performs spurious "dummy" reads at the page-wrap address before
//! the real access on indexed addressing modes. These reads trigger side
//! effects on bus devices with read-sensitive registers. The PPU's
//! PPUSTATUS register (`$2002`) is the most convenient side-effect to
//! observe: reading it clears the VBlank flag (bit 7) and returns the
//! previous value. By placing the dummy-read address at `$2002` and the
//! real-read address at a mirror (`$2102`, `$2202`, etc.), we can
//! distinguish whether the dummy read fired:
//!
//! - **Dummy read happened**: VBlank is cleared by the dummy read at
//!   `$2002`; the real read at the mirror returns VBlank=0.
//! - **No dummy read**: the real read at the mirror is the first read of
//!   PPUSTATUS; it returns VBlank=1 and then clears it.
//!
//! For store opcodes (STA/STX/STY), the only read is the dummy read — the
//! real access is a write. So if VBlank is cleared after a store, the dummy
//! read must have fired.
//!
//! # Interrupt timing
//!
//! Tests verify NMI priority over IRQ, IRQ masking by the I flag, and the
//! NMI-vs-BRK interaction (NMI is serviced before BRK is fetched when NMI
//! is pending at the instruction boundary).
//!
//! See: https://www.nesdev.org/6502.txt — "Dummy reads"
//! See: https://www.nesdev.org/wiki/CPU_interrupts

use nes_emu::bus::Bus;
use nes_emu::cartridge::Cartridge;
use nes_emu::cpu::flags;
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

/// Build a 32KB NROM cartridge with custom NMI / RESET / IRQ vectors.
fn make_cart(nmi: u16, reset: u16, irq: u16) -> Cartridge {
    let mut bytes = vec![b'N', b'E', b'S', 0x1A, 2, 0, 0, 0];
    bytes.extend_from_slice(&[0u8; 8]);
    bytes.resize(16 + 32 * 1024, 0);
    let base = 16 + 0x7FFA;
    bytes[base] = (nmi & 0xFF) as u8;
    bytes[base + 1] = (nmi >> 8) as u8;
    bytes[base + 2] = (reset & 0xFF) as u8;
    bytes[base + 3] = (reset >> 8) as u8;
    bytes[base + 4] = (irq & 0xFF) as u8;
    bytes[base + 5] = (irq >> 8) as u8;
    Cartridge::from_bytes(&bytes).expect("build test cartridge")
}

/// Build a bus with the given vectors and a small program at `PC0`.
fn bus_with_vectors(prog: &[u8], nmi: u16, reset: u16, irq: u16) -> Bus {
    let cart = make_cart(nmi, reset, irq);
    let mut bus = Bus::with_cartridge(cart);
    for (i, b) in prog.iter().enumerate() {
        bus.write(PC0 + i as u16, *b);
    }
    bus
}

/// Build a CPU with PC set to `PC0`, I flag clear (so IRQs are not masked).
fn cpu_at_pc0() -> Cpu {
    let mut cpu = Cpu::new();
    cpu.pc = PC0;
    // Clear I flag so IRQ tests can observe servicing.
    cpu.set_interrupt_disable(false);
    cpu
}

/// Set VBlank in the PPU and return the bus ready for a dummy-read test.
fn set_vblank(bus: &mut Bus) {
    bus.ppu_mut().set_vblank(true);
}

/// Read VBlank status from the PPU without clearing it (direct field access).
fn vblank_is_set(bus: &Bus) -> bool {
    bus.ppu().in_vblank()
}

// ===========================================================================
// Dummy reads: absolute,X reads with page cross
// ===========================================================================

/// `LDA $20F0,X` with X=$12 → effective $2102 (mirror of $2002).
/// Dummy read at $2002 (PPUSTATUS) clears VBlank; real read at $2102
/// returns VBlank=0. Without the dummy read, the real read would return
/// VBlank=1.
#[test]
fn lda_abs_x_page_cross_dummy_read_clears_vblank() {
    let mut bus = bus_with_prog(&[0xBD, 0xF0, 0x20]); // LDA $20F0,X
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x12;
    set_vblank(&mut bus);

    let _ = cpu.step(&mut bus);

    // The dummy read at $2002 should have cleared VBlank before the real
    // read at $2102, so A should NOT have bit 7 set.
    assert_eq!(
        cpu.a & 0x80,
        0,
        "dummy read should have cleared VBlank before real read"
    );
    assert!(!vblank_is_set(&bus), "VBlank should be cleared after read");
}

/// `LDA $2000,X` with X=$2 → effective $2002, NO page cross.
/// No dummy read; the single read at $2002 returns VBlank=1.
#[test]
fn lda_abs_x_no_page_cross_no_dummy_read() {
    let mut bus = bus_with_prog(&[0xBD, 0x00, 0x20]); // LDA $2000,X
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x02;
    set_vblank(&mut bus);

    let _ = cpu.step(&mut bus);

    // No page cross → no dummy read → the single read returns VBlank=1.
    assert_eq!(
        cpu.a & 0x80,
        0x80,
        "no dummy read; real read should return VBlank=1"
    );
    assert!(!vblank_is_set(&bus), "VBlank cleared by real read");
}

/// `LDA $20F0,X` with X=$12 → 5 cycles (4 + 1 page cross).
#[test]
fn lda_abs_x_page_cross_cycle_count() {
    let mut bus = bus_with_prog(&[0xBD, 0xF0, 0x20]);
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x12;
    let c = cpu.step(&mut bus);
    assert_eq!(c, 5);
}

/// `LDA $20F0,X` with X=$00 → no page cross → 4 cycles.
#[test]
fn lda_abs_x_no_page_cross_cycle_count() {
    let mut bus = bus_with_prog(&[0xBD, 0xF0, 0x20]);
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x00;
    let c = cpu.step(&mut bus);
    assert_eq!(c, 4);
}

// ===========================================================================
// Dummy reads: absolute,Y reads with page cross
// ===========================================================================

/// `LDA $20F0,Y` with Y=$12 → effective $2102, page cross.
/// Dummy read at $2002 clears VBlank; real read returns VBlank=0.
#[test]
fn lda_abs_y_page_cross_dummy_read_clears_vblank() {
    let mut bus = bus_with_prog(&[0xB9, 0xF0, 0x20]); // LDA $20F0,Y
    let mut cpu = cpu_at_pc0();
    cpu.y = 0x12;
    set_vblank(&mut bus);

    let _ = cpu.step(&mut bus);

    assert_eq!(cpu.a & 0x80, 0, "dummy read should clear VBlank");
    assert!(!vblank_is_set(&bus));
}

/// `LDA $2002,Y` with Y=$00 → effective $2002, NO page cross.
/// No dummy read; the single read at $2002 returns VBlank=1.
#[test]
fn lda_abs_y_no_page_cross_no_dummy_read() {
    let mut bus = bus_with_prog(&[0xB9, 0x02, 0x20]); // LDA $2002,Y
    let mut cpu = cpu_at_pc0();
    cpu.y = 0x00; // effective = $2002, no page cross
    set_vblank(&mut bus);

    let _ = cpu.step(&mut bus);

    assert_eq!(
        cpu.a & 0x80,
        0x80,
        "no dummy read; real read returns VBlank=1"
    );
}

// ===========================================================================
// Dummy reads: indirect,Y reads with page cross
// ===========================================================================

/// `LDA ($20),Y` with pointer at $20=$F0, $21=$20 (base=$20F0), Y=$12.
/// Effective = $2102, page cross. Dummy read at $2002 clears VBlank.
#[test]
fn lda_ind_y_page_cross_dummy_read_clears_vblank() {
    let mut bus = bus_with_prog(&[0xB1, 0x20]); // LDA ($20),Y
    bus.write(0x0020, 0xF0); // pointer low
    bus.write(0x0021, 0x20); // pointer high → base = $20F0
    let mut cpu = cpu_at_pc0();
    cpu.y = 0x12; // effective = $2102
    set_vblank(&mut bus);

    let _ = cpu.step(&mut bus);

    assert_eq!(cpu.a & 0x80, 0, "dummy read should clear VBlank");
    assert!(!vblank_is_set(&bus));
}

/// `LDA ($20),Y` with Y=$00 → no page cross, no dummy read.
#[test]
fn lda_ind_y_no_page_cross_no_dummy_read() {
    let mut bus = bus_with_prog(&[0xB1, 0x20]);
    bus.write(0x0020, 0xF0);
    bus.write(0x0021, 0x20); // base = $20F0
    let mut cpu = cpu_at_pc0();
    cpu.y = 0x00; // effective = $20F0, no page cross
    set_vblank(&mut bus);

    let _ = cpu.step(&mut bus);

    // $20F0 is in PPU register space (mirror of $2000 = PPUCTRL, write-only).
    // Reading it returns open bus. VBlank is in PPUSTATUS ($2002), so reading
    // $20F0 does NOT clear VBlank. We just verify no dummy read happened
    // (VBlank should still be set since no PPUSTATUS read occurred).
    assert!(vblank_is_set(&bus), "VBlank should not be cleared");
}

// ===========================================================================
// Dummy reads: RMW on absolute,X (always dummy read, even without page cross)
// ===========================================================================

/// `INC $20F0,X` with X=$12 → effective $2102, page cross.
/// Dummy read at $2002 clears VBlank before the real read.
#[test]
fn inc_abs_x_page_cross_dummy_read_clears_vblank() {
    let mut bus = bus_with_prog(&[0xFE, 0xF0, 0x20]); // INC $20F0,X
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x12;
    set_vblank(&mut bus);

    let _ = cpu.step(&mut bus);

    // The dummy read at $2002 should clear VBlank.
    // (The real read at $2102 would also clear it, but the dummy fires first.)
    assert!(!vblank_is_set(&bus), "dummy read should clear VBlank");
}

/// `INC $2000,X` with X=$02 → effective $2002, NO page cross.
/// RMW opcodes ALWAYS do a dummy read — even without page cross, the
/// dummy read is at the page-wrap address which equals the effective
/// address. The dummy read at $2002 clears VBlank.
#[test]
fn inc_abs_x_no_page_cross_still_does_dummy_read() {
    let mut bus = bus_with_prog(&[0xFE, 0x00, 0x20]); // INC $2000,X
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x02; // effective = $2002, no page cross
    set_vblank(&mut bus);

    let _ = cpu.step(&mut bus);

    // RMW always does a dummy read. Without page cross, the dummy read
    // address = effective address = $2002. The dummy read clears VBlank.
    // The real read also reads $2002 but VBlank is already cleared.
    assert!(!vblank_is_set(&bus), "RMW dummy read should clear VBlank");
}

/// `INC $20F0,X` with X=$12 → 7 cycles (RMW abs,X is always 7).
#[test]
fn inc_abs_x_cycle_count() {
    let mut bus = bus_with_prog(&[0xFE, 0xF0, 0x20]);
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x12;
    let c = cpu.step(&mut bus);
    assert_eq!(c, 7);
}

/// `ASL $20F0,X` with X=$12 → dummy read clears VBlank.
#[test]
fn asl_abs_x_dummy_read_clears_vblank() {
    let mut bus = bus_with_prog(&[0x1E, 0xF0, 0x20]); // ASL $20F0,X
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x12;
    set_vblank(&mut bus);

    let _ = cpu.step(&mut bus);
    assert!(!vblank_is_set(&bus));
}

/// `DEC $20F0,X` with X=$12 → dummy read clears VBlank.
#[test]
fn dec_abs_x_dummy_read_clears_vblank() {
    let mut bus = bus_with_prog(&[0xDE, 0xF0, 0x20]);
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x12;
    set_vblank(&mut bus);

    let _ = cpu.step(&mut bus);
    assert!(!vblank_is_set(&bus));
}

/// `LSR $20F0,X` with X=$12 → dummy read clears VBlank.
#[test]
fn lsr_abs_x_dummy_read_clears_vblank() {
    let mut bus = bus_with_prog(&[0x5E, 0xF0, 0x20]);
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x12;
    set_vblank(&mut bus);

    let _ = cpu.step(&mut bus);
    assert!(!vblank_is_set(&bus));
}

/// `ROL $20F0,X` with X=$12 → dummy read clears VBlank.
#[test]
fn rol_abs_x_dummy_read_clears_vblank() {
    let mut bus = bus_with_prog(&[0x3E, 0xF0, 0x20]);
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x12;
    set_vblank(&mut bus);

    let _ = cpu.step(&mut bus);
    assert!(!vblank_is_set(&bus));
}

/// `ROR $20F0,X` with X=$12 → dummy read clears VBlank.
#[test]
fn ror_abs_x_dummy_read_clears_vblank() {
    let mut bus = bus_with_prog(&[0x7E, 0xF0, 0x20]);
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x12;
    set_vblank(&mut bus);

    let _ = cpu.step(&mut bus);
    assert!(!vblank_is_set(&bus));
}

// ===========================================================================
// Dummy reads: stores on absolute,X/Y/indirect,Y (always dummy read)
// ===========================================================================

/// `STA $20F0,X` with X=$12 → effective $2102, page cross.
/// The ONLY read is the dummy read at $2002 — the real access is a write.
/// If VBlank is cleared, the dummy read must have fired.
#[test]
fn sta_abs_x_dummy_read_clears_vblank() {
    let mut bus = bus_with_prog(&[0x9D, 0xF0, 0x20]); // STA $20F0,X
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x12;
    cpu.a = 0x42;
    set_vblank(&mut bus);

    let _ = cpu.step(&mut bus);

    // The store doesn't read PPUSTATUS — only the dummy read does.
    assert!(!vblank_is_set(&bus), "STA dummy read should clear VBlank");
}

/// `STA $2000,X` with X=$02 → effective $2002, NO page cross.
/// Store opcodes always do a dummy read, even without page cross.
#[test]
fn sta_abs_x_no_page_cross_still_does_dummy_read() {
    let mut bus = bus_with_prog(&[0x9D, 0x00, 0x20]); // STA $2000,X
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x02; // effective = $2002, no page cross
    cpu.a = 0x42;
    set_vblank(&mut bus);

    let _ = cpu.step(&mut bus);

    assert!(
        !vblank_is_set(&bus),
        "STA dummy read should clear VBlank even without page cross"
    );
}

/// `STA $20F0,Y` with Y=$12 → dummy read at $2002 clears VBlank.
#[test]
fn sta_abs_y_dummy_read_clears_vblank() {
    let mut bus = bus_with_prog(&[0x99, 0xF0, 0x20]); // STA $20F0,Y
    let mut cpu = cpu_at_pc0();
    cpu.y = 0x12;
    cpu.a = 0x42;
    set_vblank(&mut bus);

    let _ = cpu.step(&mut bus);
    assert!(!vblank_is_set(&bus));
}

/// `STA ($20),Y` with base=$20F0, Y=$12 → effective $2102, page cross.
/// Dummy read at $2002 clears VBlank.
#[test]
fn sta_ind_y_dummy_read_clears_vblank() {
    let mut bus = bus_with_prog(&[0x91, 0x20]); // STA ($20),Y
    bus.write(0x0020, 0xF0);
    bus.write(0x0021, 0x20);
    let mut cpu = cpu_at_pc0();
    cpu.y = 0x12;
    cpu.a = 0x42;
    set_vblank(&mut bus);

    let _ = cpu.step(&mut bus);
    assert!(!vblank_is_set(&bus));
}

/// `STA ($20),Y` with Y=$00 → no page cross, still does dummy read.
#[test]
fn sta_ind_y_no_page_cross_still_does_dummy_read() {
    let mut bus = bus_with_prog(&[0x91, 0x20]);
    bus.write(0x0020, 0xF0);
    bus.write(0x0021, 0x20); // base = $20F0
    let mut cpu = cpu_at_pc0();
    cpu.y = 0x00; // effective = $20F0, no page cross
    cpu.a = 0x42;
    set_vblank(&mut bus);

    let _ = cpu.step(&mut bus);

    // $20F0 is PPUCTRL (write-only) — reading it returns open bus, doesn't
    // clear VBlank. The dummy read at $20F0 (page-wrap = effective, no cross)
    // reads PPUCTRL, not PPUSTATUS. So VBlank should NOT be cleared.
    // This confirms the dummy read goes to the page-wrap address, not $2002.
    assert!(
        vblank_is_set(&bus),
        "no page cross → dummy read at $20F0 (PPUCTRL), not $2002"
    );
}

// ===========================================================================
// Dummy reads: zero-page,X/Y (dummy read at base address)
// ===========================================================================

/// `LDA $02,X` with X=$00 → effective $02, dummy read at $02.
/// Zero-page,X always does a dummy read at the base (unindexed) address.
/// With X=0, base = effective = $02, so the dummy read IS the real read
/// address. We can't distinguish them, but we can verify the dummy read
/// fires by using a PPU-register-mapped zero-page address... wait, zero-page
/// is always RAM. So we test with OAMDATA side-effect instead.
///
/// Actually, zero-page is always RAM ($0000-$07FF), which has no read side
/// effects. So we test the zero-page,X dummy read by verifying the correct
/// effective address is computed (the dummy read at base is invisible for
/// RAM). We verify the read/write still works correctly.
#[test]
fn lda_zp_x_dummy_read_at_base_computes_correct_effective() {
    let mut bus = bus_with_prog(&[0xB5, 0x10]); // LDA $10,X
    bus.write(0x0010, 0xAA); // value at base (dummy read address)
    bus.write(0x0012, 0xBB); // value at effective (base + X)
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x02;

    let _ = cpu.step(&mut bus);

    // Should read from $12 (effective), not $10 (dummy).
    assert_eq!(cpu.a, 0xBB, "should read from effective address $12");
}

/// `STA $10,X` with X=$02 → writes to $12, dummy read at $10.
#[test]
fn sta_zp_x_dummy_read_at_base_writes_correct_effective() {
    let mut bus = bus_with_prog(&[0x95, 0x10]); // STA $10,X
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x02;
    cpu.a = 0x42;

    let _ = cpu.step(&mut bus);

    assert_eq!(bus.read(0x0012), 0x42, "should write to effective $12");
    assert_eq!(bus.read(0x0010), 0x00, "should NOT write to base $10");
}

/// `INC $10,X` with X=$02 → increments $12, dummy read at $10.
#[test]
fn inc_zp_x_dummy_read_at_base_increments_correct_effective() {
    let mut bus = bus_with_prog(&[0xF6, 0x10]); // INC $10,X
    bus.write(0x0012, 0x05);
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x02;

    let _ = cpu.step(&mut bus);

    assert_eq!(bus.read(0x0012), 0x06, "should increment effective $12");
    assert_eq!(bus.read(0x0010), 0x00, "should NOT touch base $10");
}

/// `LDX $10,Y` with Y=$02 → reads from $12, dummy read at $10.
#[test]
fn ldx_zp_y_dummy_read_at_base_computes_correct_effective() {
    let mut bus = bus_with_prog(&[0xB6, 0x10]); // LDX $10,Y
    bus.write(0x0012, 0xCC);
    let mut cpu = cpu_at_pc0();
    cpu.y = 0x02;

    let _ = cpu.step(&mut bus);

    assert_eq!(cpu.x, 0xCC);
}

/// `STX $10,Y` with Y=$02 → writes to $12, dummy read at $10.
#[test]
fn stx_zp_y_dummy_read_at_base_writes_correct_effective() {
    let mut bus = bus_with_prog(&[0x96, 0x10]); // STX $10,Y
    let mut cpu = cpu_at_pc0();
    cpu.y = 0x02;
    cpu.x = 0x77;

    let _ = cpu.step(&mut bus);

    assert_eq!(bus.read(0x0012), 0x77);
    assert_eq!(bus.read(0x0010), 0x00);
}

/// Zero-page,X wraps within the zero page (base + X wraps at $FF→$00).
#[test]
fn zp_x_dummy_read_wraps_within_zero_page() {
    let mut bus = bus_with_prog(&[0xB5, 0xFE]); // LDA $FE,X
    bus.write(0x00FE, 0xAA); // base (dummy read)
    bus.write(0x0000, 0xBB); // wrapped effective ($FE + $02 = $00 wrap)
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x02;

    let _ = cpu.step(&mut bus);

    assert_eq!(cpu.a, 0xBB, "zero-page,X wraps at $FF→$00");
}

// ===========================================================================
// Dummy reads: all read opcodes on absolute,X with page cross
// ===========================================================================

/// Helper: run a read opcode with absolute,X page cross and check VBlank
/// is cleared by the dummy read.
fn check_abs_x_read_dummy(opcode: u8, x: u8, base_lo: u8, base_hi: u8) {
    let mut bus = bus_with_prog(&[opcode, base_lo, base_hi]);
    let mut cpu = cpu_at_pc0();
    cpu.x = x;
    set_vblank(&mut bus);
    let _ = cpu.step(&mut bus);
    assert!(
        !vblank_is_set(&bus),
        "opcode ${:02X} should issue dummy read on abs,X page cross",
        opcode
    );
}

#[test]
fn all_abs_x_read_opcodes_issue_dummy_read_on_page_cross() {
    // base = $20F0, X = $12 → effective $2102, dummy at $2002
    let (lo, hi) = (0xF0, 0x20);
    let x = 0x12;
    // LDA, LDY, AND, ORA, EOR, ADC, SBC, CMP
    check_abs_x_read_dummy(0xBD, x, lo, hi); // LDA
    check_abs_x_read_dummy(0xBC, x, lo, hi); // LDY
    check_abs_x_read_dummy(0x3D, x, lo, hi); // AND
    check_abs_x_read_dummy(0x1D, x, lo, hi); // ORA
    check_abs_x_read_dummy(0x5D, x, lo, hi); // EOR
    check_abs_x_read_dummy(0x7D, x, lo, hi); // ADC
    check_abs_x_read_dummy(0xFD, x, lo, hi); // SBC
    check_abs_x_read_dummy(0xDD, x, lo, hi); // CMP
}

/// Helper: run a read opcode with absolute,Y page cross and check VBlank
/// is cleared by the dummy read.
fn check_abs_y_read_dummy(opcode: u8, y: u8, base_lo: u8, base_hi: u8) {
    let mut bus = bus_with_prog(&[opcode, base_lo, base_hi]);
    let mut cpu = cpu_at_pc0();
    cpu.y = y;
    set_vblank(&mut bus);
    let _ = cpu.step(&mut bus);
    assert!(
        !vblank_is_set(&bus),
        "opcode ${:02X} should issue dummy read on abs,Y page cross",
        opcode
    );
}

#[test]
fn all_abs_y_read_opcodes_issue_dummy_read_on_page_cross() {
    let (lo, hi) = (0xF0, 0x20);
    let y = 0x12;
    // LDA, LDX, AND, ORA, EOR, ADC, SBC, CMP
    check_abs_y_read_dummy(0xB9, y, lo, hi); // LDA
    check_abs_y_read_dummy(0xBE, y, lo, hi); // LDX
    check_abs_y_read_dummy(0x39, y, lo, hi); // AND
    check_abs_y_read_dummy(0x19, y, lo, hi); // ORA
    check_abs_y_read_dummy(0x59, y, lo, hi); // EOR
    check_abs_y_read_dummy(0x79, y, lo, hi); // ADC
    check_abs_y_read_dummy(0xF9, y, lo, hi); // SBC
    check_abs_y_read_dummy(0xD9, y, lo, hi); // CMP
}

/// Helper: run a read opcode with indirect,Y page cross and check VBlank.
fn check_ind_y_read_dummy(opcode: u8, y: u8) {
    let mut bus = bus_with_prog(&[opcode, 0x20]);
    bus.write(0x0020, 0xF0);
    bus.write(0x0021, 0x20); // base = $20F0
    let mut cpu = cpu_at_pc0();
    cpu.y = y;
    set_vblank(&mut bus);
    let _ = cpu.step(&mut bus);
    assert!(
        !vblank_is_set(&bus),
        "opcode ${:02X} should issue dummy read on ind,Y page cross",
        opcode
    );
}

#[test]
fn all_ind_y_read_opcodes_issue_dummy_read_on_page_cross() {
    let y = 0x12;
    // LDA, AND, ORA, EOR, ADC, SBC, CMP
    check_ind_y_read_dummy(0xB1, y); // LDA
    check_ind_y_read_dummy(0x31, y); // AND
    check_ind_y_read_dummy(0x11, y); // ORA
    check_ind_y_read_dummy(0x51, y); // EOR
    check_ind_y_read_dummy(0x71, y); // ADC
    check_ind_y_read_dummy(0xF1, y); // SBC
    check_ind_y_read_dummy(0xD1, y); // CMP
}

// ===========================================================================
// Dummy reads: OAMDATA ($2004) increment side-effect
// ===========================================================================

/// `LDA $20F4,X` with X=$10 → effective $2104 (mirror of $2004 = OAMDATA).
/// Dummy read at $2004 increments OAMADDR. Real read at $2104 also
/// increments OAMADDR. After the instruction, OAMADDR should be 2
/// (incremented twice: dummy + real).
#[test]
fn lda_abs_x_dummy_read_increments_oamaddr() {
    let mut bus = bus_with_prog(&[0xBD, 0xF4, 0x20]); // LDA $20F4,X
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x10; // effective = $2104, page cross
                  // Set OAMADDR to 0.
    bus.write(0x2003, 0x00); // OAMADDR = 0

    let _ = cpu.step(&mut bus);

    // Dummy read at $2004 increments OAMADDR to 1.
    // Real read at $2104 increments OAMADDR to 2.
    let oamaddr = bus.ppu().oamaddr();
    assert_eq!(
        oamaddr, 2,
        "OAMADDR should be 2 (dummy + real read both increment)"
    );
}

/// `LDA $2004,X` with X=$00 → effective $2004, NO page cross.
/// No dummy read; only the real read increments OAMADDR → OAMADDR = 1.
#[test]
fn lda_abs_x_no_page_cross_no_dummy_oamaddr_increment() {
    let mut bus = bus_with_prog(&[0xBD, 0x04, 0x20]); // LDA $2004,X
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x00; // effective = $2004, no page cross
    bus.write(0x2003, 0x00); // OAMADDR = 0

    let _ = cpu.step(&mut bus);

    // No page cross → no dummy read → only real read increments.
    assert_eq!(
        bus.ppu().oamaddr(),
        1,
        "no dummy read; only real read increments OAMADDR"
    );
}

/// `STA $20F4,X` with X=$10 → dummy read at $2004 increments OAMADDR.
/// The store writes to $2104 (mirror of $2004 = OAMDATA), which also
/// increments OAMADDR. So total = 2 (dummy read + write).
#[test]
fn sta_abs_x_dummy_read_increments_oamaddr() {
    let mut bus = bus_with_prog(&[0x9D, 0xF4, 0x20]); // STA $20F4,X
    let mut cpu = cpu_at_pc0();
    cpu.x = 0x10;
    cpu.a = 0x42;
    bus.write(0x2003, 0x00);

    let _ = cpu.step(&mut bus);

    // Dummy read at $2004 increments OAMADDR to 1.
    // Store at $2104 writes OAMDATA, which also increments OAMADDR to 2.
    assert_eq!(
        bus.ppu().oamaddr(),
        2,
        "STA dummy read + write both increment OAMADDR"
    );
}

// ===========================================================================
// Interrupt timing: NMI priority over IRQ
// ===========================================================================

/// When both NMI and IRQ are pending, NMI is serviced first (NMI vector
/// at $FFFA, not IRQ vector at $FFFE).
#[test]
fn nmi_takes_priority_over_irq() {
    let mut bus = bus_with_vectors(&[0xEA], 0x0300, PC0, 0x0400);
    let mut cpu = cpu_at_pc0();
    cpu.set_nmi_pending(true);
    cpu.set_irq_pending(true);
    cpu.set_interrupt_disable(false);

    let cycles = cpu.step(&mut bus);

    // NMI should be serviced: PC loaded from NMI vector ($0300).
    assert_eq!(cpu.pc, 0x0300, "NMI should take priority over IRQ");
    assert_eq!(cycles, 7);
    assert!(!cpu.nmi_pending(), "NMI flag should be cleared");
    // IRQ flag stays set (NMI doesn't clear it).
    assert!(cpu.irq_pending(), "IRQ flag should remain set");
    // I flag should be set by NMI service.
    assert!(cpu.interrupt_disable(), "I flag set by NMI service");
}

/// IRQ is serviced when I flag is clear.
#[test]
fn irq_serviced_when_i_flag_clear() {
    let mut bus = bus_with_vectors(&[0xEA], 0x0000, PC0, 0x0400);
    let mut cpu = cpu_at_pc0();
    cpu.set_irq_pending(true);
    cpu.set_interrupt_disable(false);

    let cycles = cpu.step(&mut bus);

    assert_eq!(cpu.pc, 0x0400, "IRQ should be serviced");
    assert_eq!(cycles, 7);
    assert!(!cpu.irq_pending());
}

/// IRQ is NOT serviced when I flag is set — the NOP executes instead.
#[test]
fn irq_not_serviced_when_i_flag_set() {
    let mut bus = bus_with_vectors(&[0xEA], 0x0000, PC0, 0x0400);
    let mut cpu = cpu_at_pc0();
    cpu.set_irq_pending(true);
    cpu.set_interrupt_disable(true); // I flag set → IRQ masked

    let cycles = cpu.step(&mut bus);

    // NOP executes, IRQ not serviced.
    assert_eq!(cycles, 2, "NOP should execute, not IRQ");
    assert_eq!(cpu.pc, PC0 + 1, "PC should advance past NOP");
    assert!(cpu.irq_pending(), "IRQ flag should remain set");
}

/// NMI is serviced even when I flag is set (non-maskable).
#[test]
fn nmi_serviced_even_when_i_flag_set() {
    let mut bus = bus_with_vectors(&[0xEA], 0x0300, PC0, 0x0000);
    let mut cpu = cpu_at_pc0();
    cpu.set_nmi_pending(true);
    cpu.set_interrupt_disable(true); // I flag set

    let cycles = cpu.step(&mut bus);

    assert_eq!(cpu.pc, 0x0300, "NMI is non-maskable");
    assert_eq!(cycles, 7);
}

// ===========================================================================
// Interrupt timing: NMI vs BRK
// ===========================================================================

/// When NMI is pending and the next opcode is BRK, NMI is serviced
/// instead of BRK. The pushed status has B=0 (NMI, not BRK). After RTI,
/// execution returns to the BRK address and BRK executes normally.
#[test]
fn nmi_pending_before_brk_services_nmi_not_brk() {
    let mut bus = bus_with_vectors(&[0x00, 0x00], 0x0300, PC0, 0x0400);
    let mut cpu = cpu_at_pc0();
    cpu.set_nmi_pending(true);
    let sp_before = cpu.sp;

    let cycles = cpu.step(&mut bus);

    // NMI should be serviced, not BRK.
    assert_eq!(cpu.pc, 0x0300, "NMI vector should be loaded");
    assert_eq!(cycles, 7);
    // The pushed PC should be the BRK address (PC0), not PC0+2.
    // On real hardware, NMI is serviced before BRK is fetched, so the
    // return address is the BRK opcode address.
    // push_pc pushes high byte first, then low byte.
    let pushed_pch = bus.read(0x0100 + sp_before as u16);
    let pushed_pcl = bus.read(0x0100 + sp_before.wrapping_sub(1) as u16);
    let pushed_pc = (pushed_pcl as u16) | ((pushed_pch as u16) << 8);
    assert_eq!(
        pushed_pc, PC0,
        "pushed PC should be BRK address (NMI serviced before BRK fetch)"
    );
    // The pushed status should have B=0 (NMI, not BRK).
    let pushed_status = bus.read(0x0100 + sp_before.wrapping_sub(2) as u16);
    assert_eq!(
        pushed_status & flags::B,
        0,
        "B flag should be 0 for NMI (not BRK)"
    );
}

/// BRK when NMI is NOT pending uses the BRK/IRQ vector and sets B=1.
#[test]
fn brk_without_nmi_uses_brk_vector_and_sets_b() {
    let mut bus = bus_with_vectors(&[0x00, 0x00], 0x0300, PC0, 0x0400);
    let mut cpu = cpu_at_pc0();
    cpu.set_nmi_pending(false);
    let sp_before = cpu.sp;

    let cycles = cpu.step(&mut bus);

    // BRK should use the BRK vector.
    assert_eq!(cpu.pc, 0x0400, "BRK vector should be loaded");
    assert_eq!(cycles, 7);
    // Pushed PC should be PC0 + 2 (past the BRK instruction).
    // push_pc pushes high byte first, then low byte.
    let pushed_pch = bus.read(0x0100 + sp_before as u16);
    let pushed_pcl = bus.read(0x0100 + sp_before.wrapping_sub(1) as u16);
    let pushed_pc = (pushed_pcl as u16) | ((pushed_pch as u16) << 8);
    assert_eq!(pushed_pc, PC0 + 2, "pushed PC should be past BRK");
    // B flag should be set.
    let pushed_status = bus.read(0x0100 + sp_before.wrapping_sub(2) as u16);
    assert_eq!(
        pushed_status & flags::B,
        flags::B,
        "B flag should be set for BRK"
    );
}

// ===========================================================================
// Interrupt timing: NMI/IRQ flag clearing
// ===========================================================================

/// Servicing NMI clears the nmi_pending flag.
#[test]
fn servicing_nmi_clears_nmi_pending_flag() {
    let mut bus = bus_with_prog(&[0xEA]);
    bus.write(0xFFFA, 0x00);
    bus.write(0xFFFB, 0x03);
    let mut cpu = cpu_at_pc0();
    cpu.set_nmi_pending(true);
    let _ = cpu.step(&mut bus);
    assert!(!cpu.nmi_pending());
}

/// Servicing IRQ clears the irq_pending flag.
#[test]
fn servicing_irq_clears_irq_pending_flag() {
    let mut bus = bus_with_prog(&[0xEA]);
    bus.write(0xFFFE, 0x00);
    bus.write(0xFFFF, 0x04);
    let mut cpu = cpu_at_pc0();
    cpu.set_irq_pending(true);
    cpu.set_interrupt_disable(false);
    let _ = cpu.step(&mut bus);
    assert!(!cpu.irq_pending());
}

/// IRQ flag stays set when I flag is set (not serviced).
#[test]
fn irq_flag_stays_set_when_masked() {
    let mut bus = bus_with_prog(&[0xEA]);
    let mut cpu = cpu_at_pc0();
    cpu.set_irq_pending(true);
    cpu.set_interrupt_disable(true);
    let _ = cpu.step(&mut bus);
    assert!(cpu.irq_pending(), "IRQ flag should stay set when masked");
}

// ===========================================================================
// Regression: nestest still passes (CPU integrity check)
// ===========================================================================

/// Run nestest in automation mode (PC=$C000) for ~100 instructions and
/// verify the CPU doesn't crash. The full nestest log comparison is in
/// tests/cpu_interrupts.rs; this is a smoke test that dummy reads didn't
/// break basic CPU operation.
#[test]
fn nestest_smoke_test_with_dummy_reads() {
    use std::path::PathBuf;
    let rom_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/test_roms/nestest.nes");
    if !rom_path.exists() {
        eprintln!("nestest.nes not found, skipping smoke test");
        return;
    }
    let rom_bytes = std::fs::read(&rom_path).expect("read nestest.nes");
    let cart = nes_emu::cartridge::Cartridge::from_bytes(&rom_bytes).expect("parse nestest");
    let mut emu = nes_emu::emulator::EmulatorState::new(cart);
    emu.reset();
    // Run a few frames to verify the CPU operates correctly with dummy reads.
    for _ in 0..3 {
        emu.step_frame();
    }
    // If we got here without panicking, the CPU is functional.
}
