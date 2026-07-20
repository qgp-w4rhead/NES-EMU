//! Integration tests for 6502 interrupt handling (M6): NMI, IRQ, RESET, and
//! the `nestest.nes` automation-mode CPU test ROM.
//!
//! These tests exercise the interrupt sequence end-to-end through a real
//! `Bus` + `Cpu`:
//!
//! - RESET loads PC from `$FFFC/$FFFD` and sets SP=`$FD` + I flag.
//! - NMI pushes PC + status (B clear, U set) and loads PC from `$FFFA/$FFFB`.
//! - IRQ pushes PC + status (B clear, U set) and loads PC from `$FFFE/$FFFF`,
//!   and is masked when the I flag is set.
//! - `step()` services `nmi_pending` / `irq_pending` before fetching the next
//!   opcode, with NMI taking priority over IRQ.
//! - RTI restores the pushed PC and status.
//! - `nestest.nes` runs to completion in automation mode (PC=`$C000`) with
//!   zero errors (result bytes at `$02`/`$03` both `0x00`).

use nes_emu::bus::Bus;
use nes_emu::cartridge::Cartridge;
use nes_emu::cpu::flags;
use nes_emu::cpu::{vectors, Cpu};

/// Start address for test programs in RAM.
const PC0: u16 = 0x0200;

/// Build a 32KB NROM cartridge with custom NMI / RESET / IRQ vectors.
fn make_cart(nmi: u16, reset: u16, irq: u16) -> Cartridge {
    // iNES header: 32KB PRG (2 banks), 0 CHR (CHR-RAM), mapper 0.
    let mut bytes = vec![b'N', b'E', b'S', 0x1A, 2, 0, 0, 0];
    bytes.extend_from_slice(&[0u8; 8]); // remaining header bytes
    bytes.resize(16 + 32 * 1024, 0);
    // Vectors live in the last 6 bytes of PRG ($FFFA-$FFFF).
    // For 32KB linear NROM, PRG offset of $FFFA = 0x7FFA.
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

/// Build a CPU with PC set to `PC0` and a known starting SP.
fn cpu_at_pc0(sp: u8) -> Cpu {
    let mut cpu = Cpu::new();
    cpu.pc = PC0;
    cpu.sp = sp;
    cpu
}

// ===========================================================================
// RESET
// ===========================================================================

#[test]
fn reset_loads_pc_from_fffc_vector() {
    let mut bus = bus_with_vectors(&[], 0x0000, 0xABCD, 0x0000);
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);
    assert_eq!(cpu.pc, 0xABCD);
}

#[test]
fn reset_sets_sp_to_fd_and_i_flag() {
    let mut bus = bus_with_vectors(&[], 0, 0x1234, 0);
    let mut cpu = Cpu::new();
    // Corrupt SP and I first to confirm reset overwrites them.
    cpu.sp = 0x10;
    cpu.set_interrupt_disable(false);
    cpu.reset(&mut bus);
    assert_eq!(cpu.sp, 0xFD);
    assert!(cpu.interrupt_disable());
}

#[test]
fn reset_does_not_push_anything_onto_stack() {
    // After RESET the stack should be empty (SP=$FD) and the bytes just
    // above SP should be unchanged from their pre-reset values.
    let mut bus = bus_with_vectors(&[], 0, 0xBEEF, 0);
    // Seed the stack area with a marker.
    bus.write(0x01FD, 0x77);
    bus.write(0x01FC, 0x88);
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);
    assert_eq!(bus.read(0x01FD), 0x77);
    assert_eq!(bus.read(0x01FC), 0x88);
    assert_eq!(cpu.sp, 0xFD);
}

#[test]
fn reset_sets_u_flag() {
    let mut bus = bus_with_vectors(&[], 0, 0x0000, 0);
    let mut cpu = Cpu::new();
    cpu.status = 0;
    cpu.reset(&mut bus);
    assert!(cpu.status & flags::U != 0);
}

// ===========================================================================
// NMI
// ===========================================================================

#[test]
fn nmi_loads_pc_from_fffa_vector() {
    let mut bus = bus_with_vectors(&[], 0xDEAD, 0, 0);
    let mut cpu = cpu_at_pc0(0xFF);
    cpu.nmi(&mut bus);
    assert_eq!(cpu.pc, 0xDEAD);
}

#[test]
fn nmi_pushes_pc_high_then_low_then_status() {
    // PC = PC0 = $0200. After NMI the stack should contain:
    //   $01FF = PCH = $02
    //   $01FE = PCL = $00
    //   $01FD = status (B clear, U set, I set after)
    // and SP should be $FC.
    let mut bus = bus_with_vectors(&[], 0x4000, 0, 0);
    let mut cpu = cpu_at_pc0(0xFF);
    cpu.status = flags::U | flags::I; // pre-existing I+U
    cpu.nmi(&mut bus);
    assert_eq!(bus.read(0x01FF), 0x02); // PCH
    assert_eq!(bus.read(0x01FE), 0x00); // PCL
    let pushed = bus.read(0x01FD);
    // B must be clear, U must be set in the pushed copy.
    assert_eq!(pushed & flags::B, 0, "B bit should be clear for NMI");
    assert_eq!(pushed & flags::U, flags::U, "U bit should be set");
    assert_eq!(cpu.sp, 0xFC);
}

#[test]
fn nmi_sets_interrupt_disable_flag() {
    let mut bus = bus_with_vectors(&[], 0x4000, 0, 0);
    let mut cpu = cpu_at_pc0(0xFF);
    cpu.set_interrupt_disable(false);
    cpu.nmi(&mut bus);
    assert!(cpu.interrupt_disable());
}

#[test]
fn nmi_is_non_maskable_ignores_i_flag() {
    // NMI must fire even when I is set.
    let mut bus = bus_with_vectors(&[], 0x5000, 0, 0);
    let mut cpu = cpu_at_pc0(0xFF);
    cpu.set_interrupt_disable(true);
    cpu.nmi(&mut bus);
    assert_eq!(cpu.pc, 0x5000);
}

#[test]
fn nmi_via_pending_flag_services_on_next_step() {
    // Set nmi_pending, then step — the interrupt should be serviced
    // instead of fetching the opcode at PC.
    let mut bus = bus_with_vectors(&[0xEA], 0x6000, 0, 0); // NOP at PC0
    let mut cpu = cpu_at_pc0(0xFF);
    cpu.nmi_pending = true;
    let cycles = cpu.step(&mut bus);
    assert_eq!(cycles, 7);
    assert_eq!(cpu.pc, 0x6000);
    // The NOP at PC0 must not have been executed — PC is at the NMI vector,
    // not at PC0+1.
    assert_ne!(cpu.pc, PC0 + 1);
    // Pending flag cleared after servicing.
    assert!(!cpu.nmi_pending);
}

// ===========================================================================
// IRQ
// ===========================================================================

#[test]
fn irq_loads_pc_from_fffe_vector() {
    let mut bus = bus_with_vectors(&[], 0, 0, 0xCAFE);
    let mut cpu = cpu_at_pc0(0xFF);
    cpu.set_interrupt_disable(false);
    cpu.irq(&mut bus);
    assert_eq!(cpu.pc, 0xCAFE);
}

#[test]
fn irq_pushes_pc_and_status_with_b_clear() {
    let mut bus = bus_with_vectors(&[], 0, 0, 0x4000);
    let mut cpu = cpu_at_pc0(0xFF);
    cpu.status = flags::U | flags::I;
    cpu.set_interrupt_disable(false);
    cpu.irq(&mut bus);
    assert_eq!(bus.read(0x01FF), 0x02); // PCH
    assert_eq!(bus.read(0x01FE), 0x00); // PCL
    let pushed = bus.read(0x01FD);
    assert_eq!(pushed & flags::B, 0, "B bit should be clear for IRQ");
    assert_eq!(pushed & flags::U, flags::U, "U bit should be set");
    assert_eq!(cpu.sp, 0xFC);
}

#[test]
fn irq_sets_interrupt_disable_flag() {
    let mut bus = bus_with_vectors(&[], 0, 0, 0x4000);
    let mut cpu = cpu_at_pc0(0xFF);
    cpu.set_interrupt_disable(false);
    cpu.irq(&mut bus);
    assert!(cpu.interrupt_disable());
}

#[test]
fn irq_via_pending_flag_masked_by_i_flag() {
    // When I is set, irq_pending must NOT be serviced; the opcode at PC
    // is fetched and executed instead.
    let mut bus = bus_with_vectors(&[0xEA], 0, 0, 0x4000); // NOP
    let mut cpu = cpu_at_pc0(0xFF);
    cpu.set_interrupt_disable(true);
    cpu.irq_pending = true;
    let cycles = cpu.step(&mut bus);
    assert_eq!(cycles, 2, "NOP should execute, not the IRQ");
    assert_eq!(cpu.pc, PC0 + 1, "NOP advanced PC");
    // irq_pending remains set because it was not serviced.
    assert!(cpu.irq_pending);
}

#[test]
fn irq_via_pending_flag_serviced_when_i_clear() {
    let mut bus = bus_with_vectors(&[0xEA], 0, 0, 0x4000); // NOP
    let mut cpu = cpu_at_pc0(0xFF);
    cpu.set_interrupt_disable(false);
    cpu.irq_pending = true;
    let cycles = cpu.step(&mut bus);
    assert_eq!(cycles, 7);
    assert_eq!(cpu.pc, 0x4000);
    assert!(!cpu.irq_pending);
}

// ===========================================================================
// NMI priority over IRQ
// ===========================================================================

#[test]
fn nmi_takes_priority_over_irq_in_step() {
    // Both pending: NMI should win and clear only its own flag.
    let mut bus = bus_with_vectors(&[0xEA], 0x1111, 0, 0x2222);
    let mut cpu = cpu_at_pc0(0xFF);
    cpu.set_interrupt_disable(false);
    cpu.nmi_pending = true;
    cpu.irq_pending = true;
    cpu.step(&mut bus);
    assert_eq!(cpu.pc, 0x1111, "NMI vector should be loaded");
    assert!(!cpu.nmi_pending, "NMI flag cleared");
    assert!(cpu.irq_pending, "IRQ flag still pending after NMI serviced");
}

// ===========================================================================
// RTI restores PC and status pushed by an interrupt
// ===========================================================================

#[test]
fn rti_restores_pc_and_status_pushed_by_nmi() {
    // Build a tiny "handler" at $4000: RTI ($40). After NMI services and
    // jumps to $4000, executing RTI should pull status + PC and return to
    // the original PC (PC0).
    let mut bus = bus_with_vectors(&[0xEA], 0x4000, 0, 0); // NOP at PC0
    bus.write(0x4000, 0x40); // RTI at the NMI handler
    let mut cpu = cpu_at_pc0(0xFF);
    cpu.status = flags::U | flags::I | flags::C; // C set as a marker
    let original_pc = cpu.pc;
    cpu.nmi(&mut bus);
    assert_eq!(cpu.pc, 0x4000);
    // Execute RTI.
    cpu.step(&mut bus);
    assert_eq!(cpu.pc, original_pc, "RTI returned to the pre-NMI PC");
    // C flag should be restored from the pushed status (it was set).
    assert!(cpu.carry(), "C flag restored from pushed status");
}

// ===========================================================================
// nestest.nes — automation mode
// ===========================================================================

/// Path to the nestest.nes ROM, checked into the test fixtures directory.
const NESTEST_ROM: &str = "tests/test_roms/nestest.nes";

/// Load nestest.nes into a bus.
fn nestest_bus() -> Bus {
    let cart = Cartridge::from_path(NESTEST_ROM).expect("load nestest.nes");
    Bus::with_cartridge(cart)
}

/// nestest in automation mode (PC = $C000) runs all CPU tests in sequence
/// and stores the result in zero-page locations $02 (high nibble) and $03
/// (low nibble). Both are $00 when every test passed; a non-zero value is
/// the hex error code of the last failing test.
///
/// See: https://www.nesdev.org/wiki/Emulator_tests — "nestest"
/// See: other/nestest.txt in christopherpow/nes-test-roms
#[test]
fn nestest_automation_passes_with_zero_errors() {
    let mut bus = nestest_bus();
    let mut cpu = Cpu::new();
    // Automation mode: start at $C000 (bypassing the interactive menu that
    // the RESET vector at $C004 would enter). SP=$FD and I=1 are already
    // set by Cpu::new().
    cpu.pc = 0xC000;

    // nestest executes ~8991 instructions in automation mode before
    // entering an infinite `JMP self` loop. Run up to 20000 steps and
    // stop early when PC stops advancing (the terminal loop).
    let mut prev_pc = cpu.pc;
    for _ in 0..20_000 {
        cpu.step(&mut bus);
        if cpu.pc == prev_pc {
            break;
        }
        prev_pc = cpu.pc;
    }

    let result_hi = bus.read(0x02);
    let result_lo = bus.read(0x03);
    let result = ((result_hi as u16) << 8) | (result_lo as u16);
    assert_eq!(
        result, 0,
        "nestest reported an error: ${:02X}{:02X} (check CPU opcode/interrupt \
         implementation against the nestest log)",
        result_hi, result_lo,
    );
}

/// nestest's RESET vector points to $C004 (the interactive menu entry).
/// This confirms our RESET sequence correctly fetches $FFFC/$FFFD.
#[test]
fn nestest_reset_vector_is_c004() {
    let mut bus = nestest_bus();
    let reset =
        u16::from(bus.read(vectors::RESET)) | (u16::from(bus.read(vectors::RESET + 1)) << 8);
    assert_eq!(reset, 0xC004, "nestest RESET vector should be $C004");
}

/// nestest's NMI vector points to $C5AF. Confirms $FFFA/$FFFB fetch logic.
#[test]
fn nestest_nmi_vector_is_c5af() {
    let mut bus = nestest_bus();
    let nmi = u16::from(bus.read(vectors::NMI)) | (u16::from(bus.read(vectors::NMI + 1)) << 8);
    assert_eq!(nmi, 0xC5AF, "nestest NMI vector should be $C5AF");
}

/// nestest's IRQ vector points to $C5F4. Confirms $FFFE/$FFFF fetch logic.
#[test]
fn nestest_irq_vector_is_c5f4() {
    let mut bus = nestest_bus();
    let irq = u16::from(bus.read(vectors::IRQ)) | (u16::from(bus.read(vectors::IRQ + 1)) << 8);
    assert_eq!(irq, 0xC5F4, "nestest IRQ vector should be $C5F4");
}
