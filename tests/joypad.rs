//! Integration tests for M13 — joypad input via `$4016`/`$4017`.
//!
//! These tests exercise the full path from the [`Bus`] register interface
//! through the [`Joypad`](nes_emu::joypad::Joypad) shift register, plus a
//! small ROM-driven test that has the CPU poll `$4016` and write the
//! result to RAM so we can observe the value the game would see.

#![allow(unused_assignments)]

use nes_emu::bus::Bus;
use nes_emu::cartridge::{Cartridge, PRG_ROM_UNIT};
use nes_emu::cpu::Cpu;
use nes_emu::emulator::EmulatorState;
use nes_emu::joypad::{button, Joypad};

/// iNES header magic.
const INES_MAGIC: [u8; 4] = [b'N', b'E', b'S', 0x1A];

/// Build an iNES image from raw PRG bytes (16 KB per bank) and optional
/// CHR bytes (8 KB per bank). Vectors come from the last 6 bytes of PRG.
fn make_ines(prg_banks: u8, chr_banks: u8, flags6: u8, prg: &[u8], chr: &[u8]) -> Vec<u8> {
    let prg_size = prg_banks as usize * PRG_ROM_UNIT;
    let chr_size = chr_banks as usize * 8 * 1024;
    let mut buf = Vec::with_capacity(16 + prg_size + chr_size);
    buf.extend_from_slice(&INES_MAGIC);
    buf.push(prg_banks);
    buf.push(chr_banks);
    buf.push(flags6);
    buf.push(0); // flags7
    buf.extend_from_slice(&[0u8; 8]);
    buf.extend_from_slice(prg);
    buf.resize(16 + prg_size, 0);
    buf.extend_from_slice(chr);
    buf.resize(16 + prg_size + chr_size, 0);
    buf
}

/// Read 8 button bits from controller `c` through the bus at `$4016`/`$4017`.
fn read_controller(bus: &mut Bus, controller: usize) -> [u8; 8] {
    let reg = if controller == 0 { 0x4016 } else { 0x4017 };
    // Standard polling sequence: strobe on, strobe off, read 8 times.
    bus.write(0x4016, 1);
    bus.write(0x4016, 0);
    let mut bits = [0u8; 8];
    for slot in bits.iter_mut() {
        *slot = bus.read(reg) & 1;
    }
    bits
}

// ---- Bus routing: $4016 / $4017 -------------------------------------------

#[test]
fn bus_4016_read_returns_button_bit0() {
    let mut bus = Bus::new();
    bus.joypad_mut().press(0, button::A);
    let bits = read_controller(&mut bus, 0);
    // A is read first → bit 0 = 1, rest = 0.
    assert_eq!(bits, [1, 0, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn bus_4017_read_returns_controller2_buttons() {
    let mut bus = Bus::new();
    bus.joypad_mut().press(1, button::B);
    let bits = read_controller(&mut bus, 1);
    // B is the 2nd bit → index 1.
    assert_eq!(bits, [0, 1, 0, 0, 0, 0, 0, 0]);
}

#[test]
fn bus_full_polling_order_controller1() {
    let mut bus = Bus::new();
    // Press A, Select, Up, Right on controller 1.
    bus.joypad_mut().press(0, button::A);
    bus.joypad_mut().press(0, button::SELECT);
    bus.joypad_mut().press(0, button::UP);
    bus.joypad_mut().press(0, button::RIGHT);

    let bits = read_controller(&mut bus, 0);
    // Order: A, B, Select, Start, Up, Down, Left, Right.
    assert_eq!(bits, [1, 0, 1, 0, 1, 0, 0, 1]);
}

#[test]
fn bus_no_buttons_returns_all_zeros() {
    let mut bus = Bus::new();
    let bits1 = read_controller(&mut bus, 0);
    let bits2 = read_controller(&mut bus, 1);
    assert_eq!(bits1, [0; 8]);
    assert_eq!(bits2, [0; 8]);
}

#[test]
fn bus_reads_beyond_eight_return_one_in_bit0() {
    let mut bus = Bus::new();
    bus.write(0x4016, 1);
    bus.write(0x4016, 0);
    // 8 button bits (all 0).
    for _ in 0..8 {
        assert_eq!(bus.read(0x4016) & 1, 0);
    }
    // Subsequent reads return 1 in bit 0.
    for _ in 0..4 {
        assert_eq!(bus.read(0x4016) & 1, 1);
    }
}

#[test]
fn bus_strobe_high_read_returns_live_a() {
    let mut bus = Bus::new();
    bus.joypad_mut().press(0, button::A);
    bus.write(0x4016, 1); // strobe high
    assert_eq!(bus.read(0x4016) & 1, 1);
    // Release A while strobe is high → reads reflect live state.
    bus.joypad_mut().release(0, button::A);
    assert_eq!(bus.read(0x4016) & 1, 0);
}

#[test]
fn bus_open_bus_bits_preserved_on_4016_read() {
    let mut bus = Bus::new();
    // Write a value with bits 1-7 set to $4016 (strobe bit 0 = 0).
    bus.write(0x4016, 0xC0);
    // Read $4016: bit 0 = joypad (0, no buttons), bits 1-7 = open bus = 0xC0.
    let v = bus.read(0x4016);
    assert_eq!(v, 0xC0);
}

#[test]
fn bus_open_bus_bits_preserved_on_4017_read() {
    let mut bus = Bus::new();
    // Write 0xC0 to $4017 (APU frame counter latch — M14/M16).
    bus.write(0x4017, 0xC0);
    // Read $4017: bit 0 = joypad (0), bits 1-7 = open bus = 0xC0.
    let v = bus.read(0x4017);
    assert_eq!(v, 0xC0);
}

#[test]
fn bus_4016_write_latches_open_bus_and_strobes() {
    let mut bus = Bus::new();
    bus.joypad_mut().press(0, button::A);
    // Write 0x01 → strobe high, open bus = 0x01.
    bus.write(0x4016, 0x01);
    // Write 0x00 → strobe falling edge (snapshot), open bus = 0x00.
    bus.write(0x4016, 0x00);
    // Read returns A=1 in bit 0, open bus bits 1-7 = 0.
    assert_eq!(bus.read(0x4016), 1);
}

#[test]
fn bus_two_controllers_independent_through_bus() {
    let mut bus = Bus::new();
    bus.joypad_mut().press(0, button::A);
    bus.joypad_mut().press(1, button::START);

    let b1 = read_controller(&mut bus, 0);
    let b2 = read_controller(&mut bus, 1);
    assert_eq!(b1, [1, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(b2, [0, 0, 0, 1, 0, 0, 0, 0]);
}

#[test]
fn bus_re_strobe_resets_counter() {
    let mut bus = Bus::new();
    bus.joypad_mut().press(0, button::A);
    let _ = read_controller(&mut bus, 0); // exhausts the shift register
                                          // Re-strobe and read again — A should come back.
    let bits = read_controller(&mut bus, 0);
    assert_eq!(bits[0], 1);
}

#[test]
fn bus_joypad_accessors_return_state() {
    let mut bus = Bus::new();
    bus.joypad_mut().press(0, button::A);
    assert_eq!(bus.joypad().current(0), 1 << button::A);
}

#[test]
fn bus_4016_strobe_does_not_affect_4017_latch() {
    let mut bus = Bus::new();
    bus.write(0x4017, 0xAB);
    bus.write(0x4016, 1); // strobe on
    bus.write(0x4016, 0); // strobe off
                          // $4017 open bus should still be 0xAB.
    assert_eq!(bus.read(0x4017) & 0xFE, 0xAB & 0xFE);
}

// ---- CPU-driven polling via a real ROM ------------------------------------

/// 6502 opcode constants used to build test programs.
mod op {
    pub const SEI: u8 = 0x78;
    pub const CLD: u8 = 0xD8;
    pub const LDA_IMM: u8 = 0xA9;
    pub const STA_ABS: u8 = 0x8D;
    pub const LDX_IMM: u8 = 0xA2;
    pub const INX: u8 = 0xE8;
    pub const BNE: u8 = 0xD0;
    pub const JMP_ABS: u8 = 0x4C;
    pub const STA_ZPX: u8 = 0x95;
    pub const LDA_ABS: u8 = 0xAD;
}

/// IO register address (CPU-side).
const JOY1: u16 = 0x4016;

/// Build a ROM that polls controller 1 and stores the 8 button bits to
/// `$0000..$0007` (in read order: A, B, Select, Start, Up, Down, Left,
/// Right). Then loops forever. The NMI is left disabled so the polling
/// loop runs uninterrupted.
fn make_poll_rom() -> Vec<u8> {
    let mut prg = vec![0u8; PRG_ROM_UNIT];
    let mut p = 0usize;

    // reset: SEI CLD
    prg[p] = op::SEI;
    p += 1;
    prg[p] = op::CLD;
    p += 1;

    // LDA #$01 ; STA $4016  (strobe on)
    prg[p] = op::LDA_IMM;
    p += 1;
    prg[p] = 0x01;
    p += 1;
    prg[p] = op::STA_ABS;
    p += 1;
    prg[p] = JOY1 as u8;
    p += 1;
    prg[p] = (JOY1 >> 8) as u8;
    p += 1;

    // LDA #$00 ; STA $4016  (strobe off → snapshot)
    prg[p] = op::LDA_IMM;
    p += 1;
    prg[p] = 0x00;
    p += 1;
    prg[p] = op::STA_ABS;
    p += 1;
    prg[p] = JOY1 as u8;
    p += 1;
    prg[p] = (JOY1 >> 8) as u8;
    p += 1;

    // LDX #$00 (index into $0000..$0007)
    prg[p] = op::LDX_IMM;
    p += 1;
    prg[p] = 0x00;
    p += 1;

    // loop: LDA $4016 ; AND #$01 ; STA $00,X ; INX ; CPX #$08 ; BNE loop
    let loop_start = p;
    prg[p] = op::LDA_ABS;
    p += 1;
    prg[p] = JOY1 as u8;
    p += 1;
    prg[p] = (JOY1 >> 8) as u8;
    p += 1;
    // AND #$01 — opcode 0x29
    prg[p] = 0x29;
    p += 1;
    prg[p] = 0x01;
    p += 1;
    // STA $00,X (zero-page,X)
    prg[p] = op::STA_ZPX;
    p += 1;
    prg[p] = 0x00;
    p += 1;
    // INX
    prg[p] = op::INX;
    p += 1;
    // CPX #$08 — opcode 0xE0
    prg[p] = 0xE0;
    p += 1;
    prg[p] = 0x08;
    p += 1;
    // BNE loop_start
    prg[p] = op::BNE;
    p += 1;
    prg[p] = (loop_start as i16 - (p as i16 + 1)) as i8 as u8;
    p += 1;

    // After the loop, JMP self (halt).
    let halt = p;
    prg[p] = op::JMP_ABS;
    p += 1;
    prg[p] = halt as u8;
    p += 1;
    prg[p] = (halt >> 8) as u8;
    p += 1;

    // Vectors: RESET → $C000 (NROM-128 mirror of $8000).
    let reset_off = 0x3FFC;
    prg[reset_off] = 0x00;
    prg[reset_off + 1] = 0xC0;
    // NMI / IRQ → $0000 (unused, NMI disabled).
    prg[0x3FFA] = 0x00;
    prg[0x3FFB] = 0x00;
    prg[0x3FFE] = 0x00;
    prg[0x3FFF] = 0x00;

    make_ines(1, 0, 0, &prg, &[])
}

/// Run the CPU for `cycles` cycles against `bus`, returning the cycle count.
fn run_cpu_cycles(cpu: &mut Cpu, bus: &mut Bus, cycles: u32) -> u32 {
    let mut total = 0u32;
    while total < cycles {
        total += cpu.step(bus) as u32;
    }
    total
}

#[test]
fn cpu_poll_rom_reads_no_buttons_as_zero() {
    let rom = make_poll_rom();
    let cart = Cartridge::from_bytes(&rom).expect("load poll ROM");
    let mut bus = Bus::with_cartridge(cart);
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);
    // Run enough cycles to complete the polling loop (well under 200 cycles).
    run_cpu_cycles(&mut cpu, &mut bus, 500);
    // $0000..$0007 should all be 0 (no buttons pressed).
    for i in 0..8 {
        assert_eq!(bus.read(i), 0, "button bit {i} should be 0");
    }
}

#[test]
fn cpu_poll_rom_reads_pressed_buttons_correctly() {
    let rom = make_poll_rom();
    let cart = Cartridge::from_bytes(&rom).expect("load poll ROM");
    let mut bus = Bus::with_cartridge(cart);
    // Press A, Start, Up on controller 1 before the CPU runs.
    bus.joypad_mut().press(0, button::A);
    bus.joypad_mut().press(0, button::START);
    bus.joypad_mut().press(0, button::UP);
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);
    run_cpu_cycles(&mut cpu, &mut bus, 500);
    // Order in $0000..$0007: A, B, Select, Start, Up, Down, Left, Right.
    assert_eq!(bus.read(0), 1, "A");
    assert_eq!(bus.read(1), 0, "B");
    assert_eq!(bus.read(2), 0, "Select");
    assert_eq!(bus.read(3), 1, "Start");
    assert_eq!(bus.read(4), 1, "Up");
    assert_eq!(bus.read(5), 0, "Down");
    assert_eq!(bus.read(6), 0, "Left");
    assert_eq!(bus.read(7), 0, "Right");
}

#[test]
fn cpu_poll_rom_all_buttons_pressed() {
    let rom = make_poll_rom();
    let cart = Cartridge::from_bytes(&rom).expect("load poll ROM");
    let mut bus = Bus::with_cartridge(cart);
    for b in 0..8 {
        bus.joypad_mut().press(0, b);
    }
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);
    run_cpu_cycles(&mut cpu, &mut bus, 500);
    for i in 0..8 {
        assert_eq!(bus.read(i), 1, "button bit {i} should be 1");
    }
}

#[test]
fn cpu_poll_rom_button_release_after_strobe_not_seen() {
    let rom = make_poll_rom();
    let cart = Cartridge::from_bytes(&rom).expect("load poll ROM");
    let mut bus = Bus::with_cartridge(cart);
    bus.joypad_mut().press(0, button::A);
    let mut cpu = Cpu::new();
    cpu.reset(&mut bus);
    // Run past the strobe-off write but before the read loop completes,
    // then release A. The snapshot was already taken, so A should still
    // read as 1. We run a small number of cycles to land mid-poll.
    run_cpu_cycles(&mut cpu, &mut bus, 20);
    bus.joypad_mut().release(0, button::A);
    run_cpu_cycles(&mut cpu, &mut bus, 500);
    // A was pressed at snapshot time → bit 0 = 1.
    assert_eq!(bus.read(0), 1, "A should be 1 (frozen at strobe)");
}

// ---- Emulator integration -------------------------------------------------

#[test]
fn emulator_exposes_joypad_through_bus() {
    let rom = make_poll_rom();
    let cart = Cartridge::from_bytes(&rom).expect("load poll ROM");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    emu.bus_mut().joypad_mut().press(0, button::A);
    assert_eq!(emu.bus().joypad().current(0), 1 << button::A);
}

#[test]
fn emulator_step_frame_does_not_clear_joypad_state() {
    let rom = make_poll_rom();
    let cart = Cartridge::from_bytes(&rom).expect("load poll ROM");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    emu.bus_mut().joypad_mut().press(0, button::A);
    emu.step_frame();
    // The joypad state should persist across frames (the main loop does
    // not clear it — the host input layer manages button state).
    assert_eq!(emu.bus().joypad().current(0), 1 << button::A);
}

// ---- Joypad unit (re-exported here for cross-module coverage) -------------

#[test]
fn joypad_strobe_falling_edge_snapshots_state() {
    let mut j = Joypad::new();
    j.press(0, button::A);
    j.write_strobe(1);
    j.write_strobe(0);
    j.release(0, button::A);
    // Snapshot frozen with A=1.
    assert_eq!(j.read(0), 1);
}

#[test]
fn joypad_read_order_matches_nesdev() {
    let order = [
        button::A,
        button::B,
        button::SELECT,
        button::START,
        button::UP,
        button::DOWN,
        button::LEFT,
        button::RIGHT,
    ];
    for (idx, &btn) in order.iter().enumerate() {
        let mut j = Joypad::new();
        j.press(0, btn);
        j.write_strobe(1);
        j.write_strobe(0);
        for i in 0..8 {
            let bit = j.read(0);
            assert_eq!(bit, if i == idx { 1 } else { 0 });
        }
    }
}
