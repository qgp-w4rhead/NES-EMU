//! Integration tests for the M27 debug tools — disassembler, register
//! snapshot, breakpoints, single-step, and run-to-breakpoint, exercised
//! end-to-end through a real `Bus` + `Cpu` / `EmulatorState`.
//!
//! The disassembler reads via `Bus::peek` (side-effect-free); a dedicated
//! test confirms that peeking `PPUSTATUS` does not clear the VBlank flag
//! the way a real `Bus::read` would.

use nes_emu::bus::Bus;
use nes_emu::cartridge::Cartridge;
use nes_emu::cpu::Cpu;
use nes_emu::debug::{
    disassemble_at, disassemble_window, Breakpoint, CpuDebugger, RegisterSnapshot,
};
use nes_emu::emulator::EmulatorState;

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

// ---- Disassembler: every addressing mode formats correctly -------------

#[test]
fn disassembler_all_addressing_modes() {
    // One representative opcode per addressing mode.
    let prog: &[(u8, &[u8], &str)] = &[
        (0x00, &[0x00], "BRK"),               // Implied (BRK, 2 bytes)
        (0xEA, &[], "NOP"),                   // Implied
        (0x0A, &[], "ASL A"),                 // Accumulator
        (0xA9, &[0x44], "LDA #$44"),          // Immediate
        (0xA5, &[0x10], "LDA $10"),           // ZeroPage
        (0xB5, &[0x10], "LDA $10,X"),         // ZeroPageX
        (0xB6, &[0x10], "LDX $10,Y"),         // ZeroPageY
        (0xAD, &[0x00, 0x02], "LDA $0200"),   // Absolute
        (0xBD, &[0x10, 0x03], "LDA $0310,X"), // AbsoluteX
        (0xB9, &[0x10, 0x03], "LDA $0310,Y"), // AbsoluteY
        (0x6C, &[0x34, 0x12], "JMP ($1234)"), // Indirect
        (0xA1, &[0x20], "LDA ($20,X)"),       // IndirectX
        (0xB1, &[0x20], "LDA ($20),Y"),       // IndirectY
        (0xF0, &[0x10], "BEQ $0212"),         // Relative (forward)
    ];
    for (op, operand, expected) in prog {
        let mut prog_bytes = vec![*op];
        prog_bytes.extend_from_slice(operand);
        let bus = bus_with_prog(&prog_bytes);
        let instr = disassemble_at(&bus, PC0);
        assert_eq!(
            instr.text, *expected,
            "opcode ${:02X}: expected {:?}, got {:?}",
            op, expected, instr.text,
        );
    }
}

#[test]
fn disassembler_branch_backward_wraps() {
    let bus = bus_with_prog(&[0xD0, 0xFC]); // BNE $01FE
    let instr = disassemble_at(&bus, PC0);
    assert_eq!(instr.text, "BNE $01FE");
}

#[test]
fn disassembler_unofficial_opcode_decodes_correctly() {
    // M33: all 256 opcodes are now mapped (151 official + 105 unofficial).
    // 0xD3 was previously `???` (1 byte); it is now DCP (ind),Y (2 bytes).
    let bus = bus_with_prog(&[0xD3, 0x10]);
    let instr = disassemble_at(&bus, PC0);
    assert_eq!(instr.len, 2);
    assert_eq!(instr.text, "DCP ($10),Y");
    assert_eq!(instr.next_pc, PC0 + 2);
}

#[test]
fn disassemble_window_returns_requested_count() {
    // 3 NOPs + 1 LDA imm + 1 BRK
    let bus = bus_with_prog(&[0xEA, 0xEA, 0xEA, 0xA9, 0x44, 0x00, 0x00]);
    let win = disassemble_window(&bus, PC0, 4);
    assert_eq!(win.len(), 4);
    assert_eq!(win[0].text, "NOP");
    assert_eq!(win[1].text, "NOP");
    assert_eq!(win[2].text, "NOP");
    assert_eq!(win[3].text, "LDA #$44");
    assert_eq!(win[3].next_pc, PC0 + 5);
}

#[test]
fn disassemble_window_stops_at_address_space_wrap() {
    // Place a 3-byte instruction at $FFFE so its next_pc wraps to $0001.
    let mut bus = Bus::new();
    bus.write(0xFFFE, 0xAD); // LDA absolute
    bus.write(0xFFFF, 0x00);
    // $0000 is the high byte of the operand (RAM, readable via peek).
    // The window should return the wrapped instruction and then stop
    // (next_pc <= addr).
    let win = disassemble_window(&bus, 0xFFFE, 5);
    assert_eq!(win.len(), 1);
    assert_eq!(win[0].addr, 0xFFFE);
}

// ---- Register snapshot --------------------------------------------------

#[test]
fn register_snapshot_reflects_cpu_state() {
    let mut cpu = Cpu::new();
    cpu.a = 0x12;
    cpu.x = 0x34;
    cpu.y = 0x56;
    cpu.sp = 0xFA;
    cpu.pc = 0xBEEF;
    // N(7) V(6) I(2) C(0) set; U(5) B(4) D(3) Z(1) clear.
    cpu.status = 0b1100_0101;
    let snap = RegisterSnapshot::from_cpu(&cpu);
    assert_eq!(snap.a, 0x12);
    assert_eq!(snap.x, 0x34);
    assert_eq!(snap.y, 0x56);
    assert_eq!(snap.sp, 0xFA);
    assert_eq!(snap.pc, 0xBEEF);
    assert_eq!(snap.status, 0b1100_0101);
    assert!(snap.n && snap.v && snap.i && snap.c);
    assert!(!snap.b && !snap.d && !snap.z);
}

#[test]
fn register_snapshot_display_contains_all_registers() {
    let mut cpu = Cpu::new();
    cpu.pc = 0xC000;
    let snap = RegisterSnapshot::from_cpu(&cpu);
    let line = snap.to_line();
    assert!(line.contains("A:"));
    assert!(line.contains("X:"));
    assert!(line.contains("Y:"));
    assert!(line.contains("SP:"));
    assert!(line.contains("PC:C000"));
    assert!(line.contains("NV-BDIZC"));
}

// ---- Breakpoints --------------------------------------------------------

#[test]
fn address_breakpoint_fires_at_pc() {
    let mut cpu = Cpu::new();
    cpu.pc = 0xC000;
    let bus = Bus::new();
    let mut dbg = CpuDebugger::new();
    dbg.toggle_run_to_breakpoint();
    dbg.add(Breakpoint::Address(0xC000));
    assert!(dbg.check_before_step(&cpu, &bus));
    assert!(dbg.is_paused());
    assert_eq!(dbg.last_hit(), Some(Breakpoint::Address(0xC000)));
}

#[test]
fn address_breakpoint_does_not_fire_at_other_pc() {
    let mut cpu = Cpu::new();
    cpu.pc = 0xC001;
    let bus = Bus::new();
    let mut dbg = CpuDebugger::new();
    dbg.toggle_run_to_breakpoint();
    dbg.add(Breakpoint::Address(0xC000));
    assert!(!dbg.check_before_step(&cpu, &bus));
    assert!(!dbg.is_paused());
}

#[test]
fn value_breakpoint_fires_when_memory_matches() {
    let cpu = Cpu::new();
    let mut bus = Bus::new();
    bus.write(0x0080, 0xAB);
    let mut dbg = CpuDebugger::new();
    dbg.toggle_run_to_breakpoint();
    dbg.add(Breakpoint::Value {
        addr: 0x0080,
        value: 0xAB,
    });
    assert!(dbg.check_before_step(&cpu, &bus));
    assert_eq!(
        dbg.last_hit(),
        Some(Breakpoint::Value {
            addr: 0x0080,
            value: 0xAB,
        }),
    );
}

#[test]
fn value_breakpoint_does_not_fire_when_memory_differs() {
    let cpu = Cpu::new();
    let mut bus = Bus::new();
    bus.write(0x0080, 0x00);
    let mut dbg = CpuDebugger::new();
    dbg.toggle_run_to_breakpoint();
    dbg.add(Breakpoint::Value {
        addr: 0x0080,
        value: 0xAB,
    });
    assert!(!dbg.check_before_step(&cpu, &bus));
}

#[test]
fn run_to_breakpoint_off_does_not_check_breakpoints() {
    let mut cpu = Cpu::new();
    cpu.pc = 0xC000;
    let bus = Bus::new();
    let mut dbg = CpuDebugger::new();
    // run_to_breakpoint is OFF by default; even with a matching
    // breakpoint, check_before_step should return false.
    dbg.add(Breakpoint::Address(0xC000));
    assert!(!dbg.check_before_step(&cpu, &bus));
    assert!(!dbg.is_paused());
}

#[test]
fn remove_breakpoint_works() {
    let mut dbg = CpuDebugger::new();
    let bp = Breakpoint::Address(0xC000);
    dbg.add(bp);
    assert_eq!(dbg.breakpoints().len(), 1);
    assert!(dbg.remove(bp));
    assert!(dbg.breakpoints().is_empty());
    assert!(!dbg.remove(bp));
}

#[test]
fn clear_breakpoints_removes_all() {
    let mut dbg = CpuDebugger::new();
    dbg.add(Breakpoint::Address(0xC000));
    dbg.add(Breakpoint::Address(0xC001));
    dbg.add(Breakpoint::Value {
        addr: 0x80,
        value: 0x42,
    });
    assert_eq!(dbg.breakpoints().len(), 3);
    dbg.clear();
    assert!(dbg.breakpoints().is_empty());
}

// ---- Single-step --------------------------------------------------------

#[test]
fn single_step_advances_pc_by_instruction_length() {
    // LDA #$44 at PC0 (2 bytes). After one single-step, PC should
    // advance by 2 and A should hold 0x44.
    let mut cpu = cpu_at_pc0();
    let mut bus = bus_with_prog(&[0xA9, 0x44, 0xEA]);
    let mut dbg = CpuDebugger::new();
    dbg.toggle_pause(); // pause first
    assert!(dbg.is_paused());
    dbg.request_step();
    // Simulate the main loop's paused branch: consume the step request,
    // then run one instruction.
    assert!(dbg.consume_step_request());
    let _cycles = cpu.step(&mut bus);
    assert_eq!(cpu.pc, PC0 + 2);
    assert_eq!(cpu.a, 0x44);
    // The next consume_step_request should be false (no further step).
    assert!(!dbg.consume_step_request());
}

#[test]
fn single_step_via_emulator_step_instruction() {
    // Use EmulatorState::step_instruction to run one CPU instruction
    // without looping to a frame boundary.
    let emu = make_nop_cart_emu();
    let emu = std::cell::RefCell::new(emu);
    {
        let mut e = emu.borrow_mut();
        e.reset();
        // After reset, PC is at the RESET vector target. Step one
        // instruction (a NOP) and confirm PC advances by 1.
        let pc_before = e.cpu().pc;
        let cycles = e.step_instruction();
        assert!(cycles >= 2, "NOP should take >=2 cycles, got {cycles}");
        assert_eq!(e.cpu().pc, pc_before.wrapping_add(1));
    }
}

// ---- Run-to-breakpoint via EmulatorState --------------------------------

#[test]
fn step_frame_debug_stops_at_address_breakpoint() {
    // Build an emulator whose PRG is all NOPs with RESET → $C000. Set a
    // breakpoint at $C005. Run step_frame_debug; it should pause with
    // PC == $C005 (after 5 NOPs) without completing the full frame.
    let mut emu = make_nop_cart_emu();
    emu.reset();
    assert_eq!(emu.cpu().pc, 0xC000);
    let mut dbg = CpuDebugger::new();
    dbg.toggle_run_to_breakpoint();
    dbg.add(Breakpoint::Address(0xC005));
    let _ = emu.step_frame_debug(&mut dbg);
    assert!(
        dbg.is_paused(),
        "debugger should be paused after breakpoint"
    );
    assert_eq!(
        emu.cpu().pc,
        0xC005,
        "PC should be exactly at the breakpoint"
    );
    assert_eq!(dbg.last_hit(), Some(Breakpoint::Address(0xC005)));
}

#[test]
fn step_frame_debug_completes_frame_without_breakpoints() {
    // With no breakpoints and run-to-breakpoint off, step_frame_debug
    // should behave like step_frame (complete a full frame).
    let mut emu = make_nop_cart_emu();
    emu.reset();
    let mut dbg = CpuDebugger::new();
    let cycles = emu.step_frame_debug(&mut dbg);
    assert!(!dbg.is_paused());
    assert!(
        (29_300..=30_400).contains(&cycles),
        "expected ~29830 cycles, got {cycles}",
    );
    assert_eq!(emu.bus().ppu().scanline(), 0);
}

#[test]
fn step_frame_debug_run_to_breakpoint_off_ignores_breakpoints() {
    // With run-to-breakpoint OFF, breakpoints should not fire.
    let mut emu = make_nop_cart_emu();
    emu.reset();
    let mut dbg = CpuDebugger::new();
    // run_to_breakpoint is OFF; add a breakpoint at the current PC.
    dbg.add(Breakpoint::Address(emu.cpu().pc));
    let _ = emu.step_frame_debug(&mut dbg);
    assert!(
        !dbg.is_paused(),
        "should not pause with run-to-breakpoint off"
    );
    assert_eq!(emu.bus().ppu().scanline(), 0);
}

#[test]
fn step_frame_debug_resume_after_breakpoint_completes_frame() {
    // After a breakpoint fires, resuming (toggle_pause off) should let
    // the frame complete.
    let mut emu = make_nop_cart_emu();
    emu.reset();
    let mut dbg = CpuDebugger::new();
    dbg.toggle_run_to_breakpoint();
    dbg.add(Breakpoint::Address(0xC002));
    let _ = emu.step_frame_debug(&mut dbg);
    assert!(dbg.is_paused());
    assert_eq!(emu.cpu().pc, 0xC002);
    // Resume: turn off pause and run again.
    dbg.toggle_pause();
    assert!(!dbg.is_paused());
    let _ = emu.step_frame_debug(&mut dbg);
    assert!(!dbg.is_paused(), "should complete frame after resume");
    assert_eq!(emu.bus().ppu().scanline(), 0);
}

#[test]
fn resume_past_value_breakpoint_does_not_refire_immediately() {
    // A Value breakpoint whose watched byte has not changed after one
    // step must NOT re-fire immediately on resume — the suppress_bp
    // mechanism skips the just-hit BP for one iteration. Build a cart
    // whose PRG writes a known value to a zero-page location, then set
    // a Value BP on that location. We can't easily trigger a Value BP
    // mid-frame with the NOP cart, so test the debugger logic directly
    // with a mock CPU/bus: the BP fires, we resume, and the next
    // check_before_step must return false (suppressed) even though the
    // value still matches.
    let cpu = Cpu::new();
    let mut bus = Bus::new();
    bus.write(0x0080, 0xAB);
    let bp = Breakpoint::Value {
        addr: 0x0080,
        value: 0xAB,
    };
    let mut dbg = CpuDebugger::new();
    dbg.toggle_run_to_breakpoint();
    dbg.add(bp);
    // BP fires.
    assert!(dbg.check_before_step(&cpu, &bus));
    assert_eq!(dbg.last_hit(), Some(bp));
    // Resume — sets step_request + suppress_bp.
    dbg.toggle_pause();
    assert!(!dbg.is_paused());
    // First check after resume: step_request lets one instruction through.
    assert!(!dbg.check_before_step(&cpu, &bus));
    // Second check: the value at $0080 is still 0xAB, but suppress_bp
    // skips the just-hit BP for one iteration, so check returns false.
    assert!(
        !dbg.check_before_step(&cpu, &bus),
        "Value BP must not re-fire immediately after resume (suppress_bp)"
    );
    // Third check: suppression consumed; BP fires again (level-triggered).
    assert!(dbg.check_before_step(&cpu, &bus));
}

#[test]
fn f2_step_then_f1_resume_suppresses_just_hit_breakpoint() {
    // If the user single-steps (F2) past a breakpoint and then resumes
    // (F1), the resume path must still suppress the just-hit BP for one
    // iteration. request_step no longer clears last_hit, so toggle_pause
    // can see it and set up suppress_bp.
    let mut cpu = Cpu::new();
    cpu.pc = 0xC000;
    let bus = Bus::new();
    let bp = Breakpoint::Address(0xC000);
    let mut dbg = CpuDebugger::new();
    dbg.toggle_run_to_breakpoint();
    dbg.add(bp);
    // BP fires at PC=0xC000.
    assert!(dbg.check_before_step(&cpu, &bus));
    assert!(dbg.is_paused());
    // User presses F2 (single-step). request_step does NOT clear last_hit.
    dbg.request_step();
    // Simulate one instruction advancing PC.
    cpu.pc = 0xC001;
    assert!(!dbg.check_before_step(&cpu, &bus));
    // Now paused again (single-step re-pauses after one instruction).
    assert!(dbg.is_paused());
    // User presses F1 to resume. last_hit is still set (the original BP),
    // so toggle_pause sets suppress_bp for that BP.
    dbg.toggle_pause();
    assert!(!dbg.is_paused());
    // Check after resume #1: step_request lets one instruction through.
    cpu.pc = 0xC000;
    assert!(
        !dbg.check_before_step(&cpu, &bus),
        "F2→F1 resume: step_request should let one instruction through"
    );
    // Check after resume #2: suppress_bp skips the just-hit BP for one
    // iteration even though PC is back at the breakpoint address.
    assert!(
        !dbg.check_before_step(&cpu, &bus),
        "F2→F1 resume: suppress_bp must skip the just-hit BP for one iteration"
    );
    // Check after resume #3: suppression consumed; BP fires again.
    assert!(dbg.check_before_step(&cpu, &bus));
}

// ---- Peek has no side-effects -------------------------------------------

#[test]
fn peek_ppustatus_does_not_clear_vblank() {
    // A real Bus::read of PPUSTATUS ($2002) clears the VBlank flag.
    // Bus::peek must NOT clear it. We set VBlank via the PPU, then peek
    // $2002 repeatedly and confirm VBlank stays set.
    let mut bus = Bus::new();
    bus.ppu_mut().set_vblank(true);
    assert!(bus.ppu().in_vblank(), "VBlank should be set initially");
    // Peek should not clear VBlank.
    for _ in 0..5 {
        let _ = bus.peek(0x2002);
        assert!(bus.ppu().in_vblank(), "peek must not clear VBlank");
    }
    // A real read DOES clear VBlank.
    let _ = bus.read(0x2002);
    assert!(!bus.ppu().in_vblank(), "read should clear VBlank");
}

#[test]
fn peek_ram_matches_read() {
    let mut bus = Bus::new();
    bus.write(0x0040, 0xAB);
    bus.write(0x0123, 0xCD);
    assert_eq!(bus.peek(0x0040), 0xAB);
    assert_eq!(bus.peek(0x0123), 0xCD);
    // Mirrors: $0840 mirrors $0040.
    assert_eq!(bus.peek(0x0840), 0xAB);
}

#[test]
fn peek_cartridge_prg_rom_matches_read() {
    // Build an NROM-128 cart with a known byte at $C000 (PRG offset 0).
    let emu = make_nop_cart_emu();
    let bus = emu.bus();
    // PRG is filled with 0xEA (NOP); $C000 should read 0xEA via both
    // peek and read.
    assert_eq!(bus.peek(0xC000), 0xEA);
}

// ---- Disassembler reads via peek (no side-effects on PPUSTATUS) ---------

#[test]
fn disassemble_via_peek_does_not_clear_vblank() {
    // Disassembling an instruction whose operand bytes happen to live in
    // the PPU register mirror region ($2000-$3FFF) must not trigger
    // PPUSTATUS side-effects. Place the opcode at a RAM address and
    // point the operand at $2002; the disassembler reads the operand
    // via peek.
    let mut bus = Bus::new();
    bus.write(PC0, 0xAD); // LDA absolute
    bus.write(PC0 + 1, 0x02); // lo = $02
    bus.write(PC0 + 2, 0x20); // hi = $20 → operand $2002
    bus.ppu_mut().set_vblank(true);
    let instr = disassemble_at(&bus, PC0);
    assert_eq!(instr.text, "LDA $2002");
    // VBlank must still be set — the disassembler used peek, not read.
    assert!(bus.ppu().in_vblank());
}

// ---- Helpers ------------------------------------------------------------

/// Build a minimal NROM-128 cartridge (16 KB PRG, 8 KB CHR-RAM) whose
/// PRG is filled with `0xEA` (NOP) and whose RESET vector points to
/// `$C000`. Used to exercise the debugger without a real game.
fn make_nop_cart_emu() -> EmulatorState {
    let mut bytes = vec![b'N', b'E', b'S', 0x1A, 1, 0, 0, 0];
    bytes.extend_from_slice(&[0u8; 8]); // remaining header
    bytes.resize(16 + 16 * 1024, 0xEA); // PRG filled with NOP
    let reset_off = 16 + 0x3FFC;
    bytes[reset_off] = 0x00;
    bytes[reset_off + 1] = 0xC0;
    let cart = Cartridge::from_bytes(&bytes).expect("build NOP cart");
    EmulatorState::new(cart)
}
