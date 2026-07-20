//! Integration tests for the APU pulse channels (M14).
//!
//! Exercises the bus register routing (`$4000-$4007`, `$4015`), the
//! `Apu`/`PulseChannel` public API, and the interaction between the
//! emulator frame loop and APU timer ticking.

use nes_emu::apu::Apu;
use nes_emu::bus::Bus;

// ---- Bus register routing ----------------------------------------------

#[test]
fn bus_pulse1_register_writes_route_to_apu() {
    let mut bus = Bus::new();
    // Enable pulse 1 via $4015 so the length load is not suppressed.
    bus.write(0x4015, 0x01);
    // $4000: duty 2, constant volume 0x0A.
    bus.write(0x4000, 0b1001_1010);
    // $4002: timer low = 0x34.
    bus.write(0x4002, 0x34);
    // $4003: length index 1 (length 254) + timer high = 0x02.
    bus.write(0x4003, (1 << 3) | 0x02);

    let p = bus.apu().pulse1();
    assert_eq!(p.timer_period(), 0x234);
    assert_eq!(p.length_counter(), 254);
    // sample() with duty 2 seq 0 = 0 → silent; advance to seq 1 via tick.
    assert_eq!(p.sample(), 0);
}

#[test]
fn bus_pulse2_register_writes_route_to_apu() {
    let mut bus = Bus::new();
    bus.write(0x4015, 0x02);
    bus.write(0x4004, 0b1101_0001); // duty 3, const vol 1
    bus.write(0x4006, 0x10);
    bus.write(0x4007, 0x00); // period = 16, length index 0 = 10
    let p = bus.apu().pulse2();
    assert_eq!(p.timer_period(), 0x010);
    assert_eq!(p.length_counter(), 10);
    // duty 3 seq 0 bit = 1 → audible at volume 1.
    assert_eq!(p.sample(), 1);
}

#[test]
fn bus_4015_write_enables_and_disables_pulse_channels() {
    let mut bus = Bus::new();
    bus.write(0x4015, 0x03);
    bus.write(0x4003, 0x00); // load pulse 1 length = 10
    bus.write(0x4007, 0x00); // load pulse 2 length = 10
    assert_eq!(bus.read(0x4015) & 0x03, 0x03);
    // Disable pulse 1 → its length counter is forced to 0.
    bus.write(0x4015, 0x02);
    assert_eq!(bus.read(0x4015) & 0x03, 0x02);
}

#[test]
fn bus_4015_read_reflects_length_counter_status() {
    let mut bus = Bus::new();
    // No length loaded → status bits 0,1 are 0.
    bus.write(0x4015, 0x03);
    assert_eq!(bus.read(0x4015) & 0x03, 0x00);
    // Load pulse 1 length.
    bus.write(0x4003, 0x00);
    assert_eq!(bus.read(0x4015) & 0x03, 0x01);
    // Load pulse 2 length.
    bus.write(0x4007, 0x00);
    assert_eq!(bus.read(0x4015) & 0x03, 0x03);
}

#[test]
fn bus_4015_read_preserves_open_bus_upper_bits() {
    let mut bus = Bus::new();
    // Write 0xE0 to $4015 — bits 5-7 latched on open bus; bits 0,1 enable
    // the pulse channels (no length loaded → status 0).
    bus.write(0x4015, 0xE3);
    let r = bus.read(0x4015);
    assert_eq!(r & 0xE0, 0xE0);
    assert_eq!(r & 0x1F, 0x00);
}

#[test]
fn bus_4000_reads_return_open_bus_latch() {
    let mut bus = Bus::new();
    // $4000 is a write-only pulse register; reads return the open-bus
    // latch (the last value written).
    bus.write(0x4000, 0xAB);
    assert_eq!(bus.read(0x4000), 0xAB);
    bus.write(0x4005, 0xCD);
    assert_eq!(bus.read(0x4005), 0xCD);
}

#[test]
fn bus_4008_through_400f_open_bus_latch_preserved_alongside_channel_routing() {
    let mut bus = Bus::new();
    // $4008-$400F route to the triangle/noise channels (M15) AND latch
    // the open bus (so reads return the last written value, matching real
    // open-bus behavior for write-only registers).
    bus.write(0x4008, 0x12);
    assert_eq!(bus.read(0x4008), 0x12);
    bus.write(0x400F, 0x34);
    assert_eq!(bus.read(0x400F), 0x34);
}

#[test]
fn bus_pulse1_and_pulse2_are_independent() {
    let mut bus = Bus::new();
    bus.write(0x4015, 0x03);
    bus.write(0x4000, 0b1001_0001); // pulse 1: const vol 1
    bus.write(0x4002, 0x10);
    bus.write(0x4003, 0x00);
    bus.write(0x4004, 0b1001_1111); // pulse 2: const vol 15
    bus.write(0x4006, 0x10);
    bus.write(0x4007, 0x00);
    // Advance both sequences to position 1 (duty 2 bit 1 = 1).
    bus.apu_mut().pulse1_mut().tick();
    bus.apu_mut().pulse2_mut().tick();
    assert_eq!(bus.apu().pulse1().sample(), 1);
    assert_eq!(bus.apu().pulse2().sample(), 15);
}

// ---- Apu top-level behavior --------------------------------------------

#[test]
fn apu_step_ticks_each_pulse_timer_at_half_cpu_rate() {
    let mut apu = Apu::new();
    apu.write_status(0x01);
    apu.pulse1_mut().write_register(2, 0x00);
    apu.pulse1_mut().write_register(3, 0x00); // period 0
    let s0 = apu.pulse1().sequence();
    // 5 CPU cycles = 2 APU cycles (with 1 CPU cycle left in the accumulator).
    apu.step(5);
    assert_eq!(apu.pulse1().sequence(), (s0 + 2) & 7);
    // The remaining 1 CPU cycle completes on the next step's first APU cycle.
    apu.step(1);
    assert_eq!(apu.pulse1().sequence(), (s0 + 3) & 7);
}

#[test]
fn apu_clock_quarter_frame_clocks_envelopes() {
    let mut apu = Apu::new();
    apu.write_status(0x01);
    // Pulse 1: envelope mode, volume 0 (divider period 1).
    apu.pulse1_mut().write_register(0, 0x00);
    apu.pulse1_mut().write_register(3, 0x00); // start envelope, length 10
    apu.clock_quarter_frame(); // start → decay 15
    assert_eq!(apu.pulse1().envelope_decay(), 15);
    apu.clock_quarter_frame(); // decay 15 → 14
    assert_eq!(apu.pulse1().envelope_decay(), 14);
}

#[test]
fn apu_clock_half_frame_clocks_length_and_sweep() {
    let mut apu = Apu::new();
    apu.write_status(0x01);
    apu.pulse1_mut().write_register(2, 0x00);
    apu.pulse1_mut().write_register(3, 0x01); // period 0x100, length index 0 = 10
    let len0 = apu.pulse1().length_counter();
    // Sweep: enabled, period 0, shift 1, add.
    apu.pulse1_mut().write_register(1, 0b1000_0001);
    apu.clock_half_frame();
    // Length decremented and sweep applied (divider 0 → apply + reload).
    assert_eq!(apu.pulse1().length_counter(), len0 - 1);
    assert_eq!(apu.pulse1().timer_period(), 0x100 + (0x100 >> 1));
}

#[test]
fn apu_mix_outputs_pulse_samples() {
    let mut apu = Apu::new();
    apu.write_status(0x03);
    // Pulse 1: duty 3, const vol 10, period 16, length 10.
    apu.pulse1_mut().write_register(0, 0b1101_1010);
    apu.pulse1_mut().write_register(2, 0x10);
    apu.pulse1_mut().write_register(3, 0x00);
    // Pulse 2 silent (no length loaded).
    assert_eq!(apu.mix(), 10);
}

// ---- Waveform frequency via tick ---------------------------------------

#[test]
fn pulse_waveform_advances_one_step_per_period_plus_one_ticks() {
    let mut bus = Bus::new();
    bus.write(0x4015, 0x01);
    bus.write(0x4000, 0b1001_1111); // duty 2, const vol 15
    bus.write(0x4002, 0x08); // period = 8
    bus.write(0x4003, 0x00); // length 10, seq 0, timer 0
                             // After $4003 timer is 0; one tick reloads to 8 and advances to seq 1.
    bus.apu_mut().pulse1_mut().tick();
    assert_eq!(bus.apu().pulse1().sequence(), 1);
    // 8 ticks to count down 8→0, then 1 tick to reload + advance to seq 2.
    for _ in 0..9 {
        bus.apu_mut().pulse1_mut().tick();
    }
    assert_eq!(bus.apu().pulse1().sequence(), 2);
}

#[test]
fn pulse_output_frequency_matches_period() {
    // For a pulse channel with period P (>= 8), one full 8-step waveform
    // cycle takes 8 * (P + 1) APU cycles. With P = 8 that is 72 APU cycles
    // = 144 CPU cycles. Verify the sequence returns to its start after
    // exactly that many APU ticks.
    let mut apu = Apu::new();
    apu.write_status(0x01);
    apu.pulse1_mut().write_register(0, 0b1001_1111);
    apu.pulse1_mut().write_register(2, 0x08);
    apu.pulse1_mut().write_register(3, 0x00);
    let start = apu.pulse1().sequence();
    let period: u32 = 8;
    let apu_cycles = 8 * (period + 1);
    for _ in 0..apu_cycles {
        apu.pulse1_mut().tick();
    }
    assert_eq!(apu.pulse1().sequence(), start);
}

// ---- Sweep via bus -----------------------------------------------------

#[test]
fn bus_sweep_changes_pitch_over_half_frames() {
    let mut bus = Bus::new();
    bus.write(0x4015, 0x01);
    bus.write(0x4002, 0x00);
    bus.write(0x4003, 0x01); // period = 0x100
                             // Sweep: enabled, period 0 (1 half-frame), shift 1, add.
    bus.write(0x4001, 0b1000_0001);
    let initial = bus.apu().pulse1().timer_period();
    bus.apu_mut().clock_half_frame();
    assert_eq!(bus.apu().pulse1().timer_period(), initial + (initial >> 1));
}

#[test]
fn bus_sweep_negate_pulse2_subtracts_extra_one() {
    let mut bus = Bus::new();
    bus.write(0x4015, 0x02);
    bus.write(0x4006, 0x00);
    bus.write(0x4007, 0x01); // pulse 2 period = 0x100
                             // Sweep: enabled, period 0, negate, shift 1.
    bus.write(0x4005, 0b1000_1001);
    let initial = bus.apu().pulse2().timer_period();
    bus.apu_mut().clock_half_frame();
    // Pulse 2 negate: target = period - (period>>1) - 1.
    assert_eq!(
        bus.apu().pulse2().timer_period(),
        initial - (initial >> 1) - 1
    );
}

// ---- Length counter via bus --------------------------------------------

#[test]
fn bus_length_counter_halts_when_halt_flag_set() {
    let mut bus = Bus::new();
    bus.write(0x4015, 0x01);
    // $4000 bit 5 = halt.
    bus.write(0x4000, 0x20);
    bus.write(0x4003, 0x00); // length = 10
    let start = bus.apu().pulse1().length_counter();
    bus.apu_mut().clock_half_frame();
    assert_eq!(bus.apu().pulse1().length_counter(), start);
}

#[test]
fn bus_length_counter_silences_channel_after_counting_down() {
    let mut bus = Bus::new();
    bus.write(0x4015, 0x01);
    // duty 3 (seq 0 bit 1), const vol 15, period 16, length index 0 = 10.
    bus.write(0x4000, 0b1101_1111);
    bus.write(0x4002, 0x10);
    bus.write(0x4003, 0x00);
    assert_eq!(bus.apu().pulse1().sample(), 15);
    for _ in 0..10 {
        bus.apu_mut().clock_half_frame();
    }
    assert_eq!(bus.apu().pulse1().length_counter(), 0);
    assert_eq!(bus.apu().pulse1().sample(), 0);
}

// ---- Emulator integration ----------------------------------------------

#[test]
fn emulator_exposes_apu_and_ticks_it_per_frame() {
    use nes_emu::cartridge::Cartridge;
    use nes_emu::emulator::EmulatorState;

    // Minimal NROM-128 cart filled with NOPs; RESET → $C000.
    let mut bytes = vec![b'N', b'E', b'S', 0x1A, 1, 0, 0, 0];
    bytes.extend_from_slice(&[0u8; 8]);
    bytes.resize(16 + 16 * 1024, 0xEA);
    let reset_off = 16 + 0x3FFC;
    bytes[reset_off] = 0x00;
    bytes[reset_off + 1] = 0xC0;
    let cart = Cartridge::from_bytes(&bytes).expect("build cart");

    let mut emu = EmulatorState::new(cart);
    emu.reset();
    // Configure pulse 1: duty 3, const vol 15, period 16, length 10.
    emu.bus_mut().write(0x4015, 0x01);
    emu.bus_mut().write(0x4000, 0b1101_1111);
    emu.bus_mut().write(0x4002, 0x10);
    emu.bus_mut().write(0x4003, 0x00);
    let seq_before = emu.bus().apu().pulse1().sequence();
    emu.step_frame();
    // One frame ≈ 29,830 CPU cycles ≈ 14,915 APU cycles. With period 16
    // the sequencer advances ~14,915 / 17 ≈ 877 times — far more than 8,
    // so the sequence must have moved on from its starting position.
    assert_ne!(
        emu.bus().apu().pulse1().sequence(),
        seq_before,
        "APU timer should have advanced during step_frame"
    );
}

#[test]
fn emulator_apu_state_survives_multiple_frames() {
    use nes_emu::cartridge::Cartridge;
    use nes_emu::emulator::EmulatorState;

    let mut bytes = vec![b'N', b'E', b'S', 0x1A, 1, 0, 0, 0];
    bytes.extend_from_slice(&[0u8; 8]);
    bytes.resize(16 + 16 * 1024, 0xEA);
    let reset_off = 16 + 0x3FFC;
    bytes[reset_off] = 0x00;
    bytes[reset_off + 1] = 0xC0;
    let cart = Cartridge::from_bytes(&bytes).expect("build cart");

    let mut emu = EmulatorState::new(cart);
    emu.reset();
    emu.bus_mut().write(0x4015, 0x01);
    emu.bus_mut().write(0x4003, 0x00); // load length 10
    let len0 = emu.bus().apu().pulse1().length_counter();
    assert_eq!(len0, 10);
    for _ in 0..3 {
        emu.step_frame();
    }
    // Length counter is not clocked (frame counter lands in M16), so it
    // should still be loaded. The channel remains enabled.
    assert_eq!(emu.bus().apu().pulse1().length_counter(), 10);
    assert!(emu.bus().apu().pulse1().enabled());
}

// ===========================================================================
// Triangle + noise channels (M15)
// ===========================================================================

// ---- Bus register routing: triangle -------------------------------------

#[test]
fn bus_triangle_register_writes_route_to_apu() {
    let mut bus = Bus::new();
    bus.write(0x4015, 0x04); // enable triangle
                             // $4008: halt clear, linear reload 0x2A.
    bus.write(0x4008, 0x2A);
    // $400A: timer low = 0x34.
    bus.write(0x400A, 0x34);
    // $400B: length index 1 (length 254) + timer high = 0x02.
    bus.write(0x400B, (1 << 3) | 0x02);
    assert_eq!(bus.apu().triangle().timer_period(), 0x234);
    assert_eq!(bus.apu().triangle().length_counter(), 254);
    // Linear counter reload value stored; counter itself starts at 0
    // until the quarter-frame clock reloads it (start flag set by $400B).
    bus.apu_mut().clock_quarter_frame();
    assert_eq!(bus.apu().triangle().linear_counter(), 0x2A);
}

#[test]
fn bus_triangle_4009_write_is_open_bus_only() {
    let mut bus = Bus::new();
    // $4009 is unused — the write latches the open bus but does not
    // affect the triangle channel state.
    bus.write(0x4009, 0xFF);
    assert_eq!(bus.read(0x4009), 0xFF);
    assert_eq!(bus.apu().triangle().timer_period(), 0);
}

#[test]
fn bus_triangle_reads_return_open_bus_latch() {
    let mut bus = Bus::new();
    bus.write(0x4008, 0x5A);
    assert_eq!(bus.read(0x4008), 0x5A);
    bus.write(0x400B, 0x3C);
    assert_eq!(bus.read(0x400B), 0x3C);
}

// ---- Bus register routing: noise ----------------------------------------

#[test]
fn bus_noise_register_writes_route_to_apu() {
    let mut bus = Bus::new();
    bus.write(0x4015, 0x08); // enable noise
                             // $400C: halt clear, constant volume 0x0A.
    bus.write(0x400C, 0b0001_1010);
    // $400E: mode 1, period index 5.
    bus.write(0x400E, 0x80 | 0x05);
    // $400F: length index 1 (length 254).
    bus.write(0x400F, 1 << 3);
    let n = bus.apu().noise();
    assert_eq!(n.period_index(), 5);
    assert_eq!(n.timer_period(), 96); // NOISE_PERIOD_TABLE[5]
    assert!(n.mode());
    assert_eq!(n.length_counter(), 254);
}

#[test]
fn bus_noise_400d_write_is_open_bus_only() {
    let mut bus = Bus::new();
    bus.write(0x400D, 0xFF);
    assert_eq!(bus.read(0x400D), 0xFF);
    // Noise state unaffected.
    assert_eq!(bus.apu().noise().timer_period(), 4); // default period index 0
}

#[test]
fn bus_noise_reads_return_open_bus_latch() {
    let mut bus = Bus::new();
    bus.write(0x400C, 0x5A);
    assert_eq!(bus.read(0x400C), 0x5A);
    bus.write(0x400F, 0x3C);
    assert_eq!(bus.read(0x400F), 0x3C);
}

// ---- $4015 status: triangle + noise bits --------------------------------

#[test]
fn bus_4015_write_enables_and_disables_triangle_and_noise() {
    let mut bus = Bus::new();
    bus.write(0x4015, 0x0C); // enable triangle + noise
    bus.write(0x400B, 0x00); // load triangle length = 10
    bus.write(0x400F, 0x00); // load noise length = 10
    assert_eq!(bus.read(0x4015) & 0x0C, 0x0C);
    // Disable triangle → its length counter is forced to 0.
    bus.write(0x4015, 0x08);
    assert_eq!(bus.read(0x4015) & 0x0C, 0x08);
}

#[test]
fn bus_4015_read_reflects_triangle_and_noise_length_status() {
    let mut bus = Bus::new();
    bus.write(0x4015, 0x0C);
    assert_eq!(bus.read(0x4015) & 0x0C, 0x00); // no length loaded
    bus.write(0x400B, 0x00); // triangle length = 10
    assert_eq!(bus.read(0x4015) & 0x0C, 0x04);
    bus.write(0x400F, 0x00); // noise length = 10
    assert_eq!(bus.read(0x4015) & 0x0C, 0x0C);
}

#[test]
fn bus_4015_read_preserves_open_bus_upper_bits_with_all_channels() {
    let mut bus = Bus::new();
    bus.write(0x4015, 0xEF); // bits 5-7 latched; bits 0-3 enable channels
    let r = bus.read(0x4015);
    assert_eq!(r & 0xE0, 0xE0);
    assert_eq!(r & 0x1F, 0x00); // no length loaded
}

// ---- $4010-$4013 remain open bus (DMC, M16) -----------------------------

#[test]
fn bus_4010_through_4013_remain_open_bus() {
    let mut bus = Bus::new();
    bus.write(0x4010, 0x12);
    assert_eq!(bus.read(0x4010), 0x12);
    bus.write(0x4013, 0x34);
    assert_eq!(bus.read(0x4013), 0x34);
}

// ---- Triangle waveform frequency via tick --------------------------------

#[test]
fn triangle_waveform_advances_one_step_per_period_plus_one_ticks() {
    let mut bus = Bus::new();
    bus.write(0x4015, 0x04);
    bus.write(0x4008, 0x7F); // linear reload 127
    bus.write(0x400A, 0x08); // period = 8
    bus.write(0x400B, 0x00); // length 10, seq 0, timer 0, linear start
    bus.apu_mut().clock_quarter_frame(); // start linear counter → 127
                                         // After $400B timer is 0; one tick reloads to 8 and advances to seq 1.
    bus.apu_mut().triangle_mut().tick();
    assert_eq!(bus.apu().triangle().sequence(), 1);
    // 8 ticks to count down 8→0, then 1 tick to reload + advance to seq 2.
    for _ in 0..9 {
        bus.apu_mut().triangle_mut().tick();
    }
    assert_eq!(bus.apu().triangle().sequence(), 2);
}

#[test]
fn triangle_output_frequency_matches_period() {
    // One full 32-step waveform cycle takes 32 * (P + 1) APU cycles.
    // With P = 8 that is 288 APU cycles. Verify the sequence returns to
    // its start after exactly that many APU ticks.
    let mut apu = Apu::new();
    apu.write_status(0x04);
    apu.triangle_mut().write_register(0, 0x7F); // linear reload 127
    apu.triangle_mut().write_register(2, 0x08); // period = 8
    apu.triangle_mut().write_register(3, 0x00); // length 10, start
    apu.clock_quarter_frame(); // start linear counter
    let start = apu.triangle().sequence();
    let period: u32 = 8;
    let apu_cycles = 32 * (period + 1);
    for _ in 0..apu_cycles {
        apu.triangle_mut().tick();
    }
    assert_eq!(apu.triangle().sequence(), start);
}

#[test]
fn triangle_sample_outputs_sequence_values() {
    let mut bus = Bus::new();
    bus.write(0x4015, 0x04);
    bus.write(0x4008, 0x7F); // linear reload 127
    bus.write(0x400A, 0x00); // period 0 → advance every tick
    bus.write(0x400B, 0x00); // length 10, start
    bus.apu_mut().clock_quarter_frame(); // start linear counter
                                         // The triangle sample should follow the 32-step sequence.
    let expected: [u8; 32] = [
        15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11,
        12, 13, 14, 15,
    ];
    for &v in expected.iter() {
        assert_eq!(bus.apu().triangle().sample(), v);
        bus.apu_mut().triangle_mut().tick();
    }
}

// ---- Triangle linear counter via quarter-frame --------------------------

#[test]
fn bus_triangle_linear_counter_decrements_via_quarter_frame() {
    let mut bus = Bus::new();
    bus.write(0x4015, 0x04);
    bus.write(0x4008, 0x03); // linear reload 3
    bus.write(0x400B, 0x00); // start
    bus.apu_mut().clock_quarter_frame(); // start → 3
    assert_eq!(bus.apu().triangle().linear_counter(), 3);
    bus.apu_mut().clock_quarter_frame(); // 3 → 2
    assert_eq!(bus.apu().triangle().linear_counter(), 2);
    bus.apu_mut().clock_quarter_frame(); // 2 → 1
    bus.apu_mut().clock_quarter_frame(); // 1 → 0
    assert_eq!(bus.apu().triangle().linear_counter(), 0);
    // With linear counter 0 the triangle channel is silenced.
    assert_eq!(bus.apu().triangle().sample(), 0);
}

#[test]
fn bus_triangle_halt_keeps_linear_counter_reloaded() {
    let mut bus = Bus::new();
    bus.write(0x4015, 0x04);
    bus.write(0x4008, 0x80 | 0x03); // halt + linear reload 3
    bus.write(0x400B, 0x00); // start
    bus.apu_mut().clock_quarter_frame(); // start → 3
    bus.apu_mut().clock_quarter_frame(); // halt → reload → 3
    bus.apu_mut().clock_quarter_frame();
    assert_eq!(bus.apu().triangle().linear_counter(), 3);
}

// ---- Triangle length counter via half-frame ------------------------------

#[test]
fn bus_triangle_length_counter_silences_channel_after_countdown() {
    let mut bus = Bus::new();
    bus.write(0x4015, 0x04);
    bus.write(0x4008, 0x7F); // linear reload 127
    bus.write(0x400A, 0x00); // period 0
    bus.write(0x400B, 0x00); // length 10, start
    bus.apu_mut().clock_quarter_frame(); // start linear counter
    assert_eq!(bus.apu().triangle().sample(), 15); // seq 0 = 15
    for _ in 0..10 {
        bus.apu_mut().clock_half_frame();
    }
    assert_eq!(bus.apu().triangle().length_counter(), 0);
    assert_eq!(bus.apu().triangle().sample(), 0);
}

// ---- Noise LFSR + envelope via bus --------------------------------------

#[test]
fn bus_noise_lfsr_shifts_on_timer_reload() {
    let mut bus = Bus::new();
    bus.write(0x4015, 0x08);
    bus.write(0x400E, 0x00); // mode 0, period index 0 → period 4
                             // LFSR starts at 1; first tick (timer 0 → reload 4) shifts it.
    bus.apu_mut().noise_mut().tick();
    assert_eq!(bus.apu().noise().lfsr(), 0x4000);
}

#[test]
fn bus_noise_sample_gated_by_lfsr_bit0() {
    let mut bus = Bus::new();
    bus.write(0x4015, 0x08);
    bus.write(0x400C, 0b0001_1111); // const vol 15
    bus.write(0x400F, 0x00); // length 10
                             // LFSR = 1 (bit 0 set) → sample 0.
    assert_eq!(bus.apu().noise().sample(), 0);
    // Tick to shift (timer 0 → reload + shift). LFSR → 0x4000 (bit 0 = 0).
    bus.apu_mut().noise_mut().tick();
    assert_eq!(bus.apu().noise().sample(), 15);
}

#[test]
fn bus_noise_envelope_decays_via_quarter_frame() {
    let mut bus = Bus::new();
    bus.write(0x4015, 0x08);
    bus.write(0x400C, 0x00); // envelope mode, volume 0
    bus.write(0x400F, 0x00); // start envelope, length 10
    bus.apu_mut().clock_quarter_frame(); // start → decay 15
    assert_eq!(bus.apu().noise().envelope_decay(), 15);
    bus.apu_mut().clock_quarter_frame(); // decay 15 → 14
    assert_eq!(bus.apu().noise().envelope_decay(), 14);
}

#[test]
fn bus_noise_length_counter_halts_when_halt_flag_set() {
    let mut bus = Bus::new();
    bus.write(0x4015, 0x08);
    bus.write(0x400C, 0x20); // halt
    bus.write(0x400F, 0x00); // length 10
    let start = bus.apu().noise().length_counter();
    bus.apu_mut().clock_half_frame();
    assert_eq!(bus.apu().noise().length_counter(), start);
}

#[test]
fn bus_noise_period_change_does_not_reset_running_timer() {
    let mut bus = Bus::new();
    bus.write(0x4015, 0x08);
    bus.write(0x400E, 0x00); // period index 0 → period 4
    bus.apu_mut().noise_mut().tick(); // timer 0 → reload(4) + shift; timer = 4
    bus.apu_mut().noise_mut().tick(); // 4 → 3
    assert_eq!(bus.apu().noise().timer(), 3);
    bus.write(0x400E, 0x01); // period index 1 → period 8
    assert_eq!(bus.apu().noise().timer_period(), 8);
    assert_eq!(bus.apu().noise().timer(), 3); // running timer unchanged
    bus.apu_mut().noise_mut().tick(); // 3 → 2 (still counting down)
    assert_eq!(bus.apu().noise().timer(), 2);
}

// ---- Apu top-level with all four channels -------------------------------

#[test]
fn apu_mix_includes_all_four_channels() {
    let mut apu = Apu::new();
    apu.write_status(0x0F); // enable pulse1, pulse2, triangle, noise
                            // Pulse 1: duty 3, const vol 5, period 16, length 10.
    apu.pulse1_mut().write_register(0, 0b1101_0101);
    apu.pulse1_mut().write_register(2, 0x10);
    apu.pulse1_mut().write_register(3, 0x00);
    // Pulse 2: silent (no length loaded).
    // Triangle: period 0, length 10, linear 127.
    apu.triangle_mut().write_register(0, 0x7F);
    apu.triangle_mut().write_register(2, 0x00);
    apu.triangle_mut().write_register(3, 0x00);
    apu.clock_quarter_frame(); // start triangle linear counter
                               // Noise: const vol 3, length 10. LFSR bit 0 = 1 → silent until shift.
    apu.noise_mut().write_register(0, 0b0001_0011);
    apu.noise_mut().write_register(3, 0x00);
    // Pulse 1 at seq 0 (duty 3 bit 0 = 1) → 5. Triangle at seq 0 → 15.
    // Noise silent (LFSR bit 0 = 1). Mix = 5 + 15 = 20 → clamped to 15.
    assert_eq!(apu.pulse1().sample(), 5);
    assert_eq!(apu.triangle().sample(), 15);
    assert_eq!(apu.noise().sample(), 0);
    assert_eq!(apu.mix(), 15);
}

#[test]
fn apu_step_ticks_triangle_and_noise_at_half_cpu_rate() {
    let mut apu = Apu::new();
    apu.write_status(0x04); // enable triangle
    apu.triangle_mut().write_register(2, 0x00);
    apu.triangle_mut().write_register(3, 0x00); // period 0
    let s0 = apu.triangle().sequence();
    apu.step(2); // one APU cycle → one triangle tick
    assert_eq!(apu.triangle().sequence(), (s0 + 1) & 0x1F);
    apu.step(1); // not enough for a full APU cycle
    assert_eq!(apu.triangle().sequence(), (s0 + 1) & 0x1F);
    apu.step(1); // completes the second APU cycle
    assert_eq!(apu.triangle().sequence(), (s0 + 2) & 0x1F);
}

// ---- Emulator integration: triangle + noise -----------------------------

#[test]
fn emulator_exposes_triangle_and_noise_channels() {
    use nes_emu::cartridge::Cartridge;
    use nes_emu::emulator::EmulatorState;

    let mut bytes = vec![b'N', b'E', b'S', 0x1A, 1, 0, 0, 0];
    bytes.extend_from_slice(&[0u8; 8]);
    bytes.resize(16 + 16 * 1024, 0xEA);
    let reset_off = 16 + 0x3FFC;
    bytes[reset_off] = 0x00;
    bytes[reset_off + 1] = 0xC0;
    let cart = Cartridge::from_bytes(&bytes).expect("build cart");

    let mut emu = EmulatorState::new(cart);
    emu.reset();
    // Configure triangle: period 16, length 10, linear 127.
    emu.bus_mut().write(0x4015, 0x04);
    emu.bus_mut().write(0x4008, 0x7F);
    emu.bus_mut().write(0x400A, 0x10);
    emu.bus_mut().write(0x400B, 0x00);
    // Configure noise: const vol 10, length 10, period index 4.
    emu.bus_mut().write(0x4015, 0x0C);
    emu.bus_mut().write(0x400C, 0b0001_1010);
    emu.bus_mut().write(0x400E, 0x04);
    emu.bus_mut().write(0x400F, 0x00);
    let tri_seq_before = emu.bus().apu().triangle().sequence();
    let noise_lfsr_before = emu.bus().apu().noise().lfsr();
    emu.step_frame();
    // One frame ≈ 14,915 APU cycles. Triangle with period 16 advances
    // ~14,915 / 17 ≈ 877 times — far more than 32, so the sequence must
    // have moved. Noise LFSR also shifts many times.
    assert_ne!(
        emu.bus().apu().triangle().sequence(),
        tri_seq_before,
        "triangle timer should have advanced during step_frame"
    );
    assert_ne!(
        emu.bus().apu().noise().lfsr(),
        noise_lfsr_before,
        "noise LFSR should have shifted during step_frame"
    );
}

#[test]
fn emulator_triangle_and_noise_state_survives_multiple_frames() {
    use nes_emu::cartridge::Cartridge;
    use nes_emu::emulator::EmulatorState;

    let mut bytes = vec![b'N', b'E', b'S', 0x1A, 1, 0, 0, 0];
    bytes.extend_from_slice(&[0u8; 8]);
    bytes.resize(16 + 16 * 1024, 0xEA);
    let reset_off = 16 + 0x3FFC;
    bytes[reset_off] = 0x00;
    bytes[reset_off + 1] = 0xC0;
    let cart = Cartridge::from_bytes(&bytes).expect("build cart");

    let mut emu = EmulatorState::new(cart);
    emu.reset();
    emu.bus_mut().write(0x4015, 0x0C);
    emu.bus_mut().write(0x400B, 0x00); // triangle length 10
    emu.bus_mut().write(0x400F, 0x00); // noise length 10
    assert_eq!(emu.bus().apu().triangle().length_counter(), 10);
    assert_eq!(emu.bus().apu().noise().length_counter(), 10);
    for _ in 0..3 {
        emu.step_frame();
    }
    // Length counters are not clocked (frame counter lands in M16), so
    // they should still be loaded. The channels remain enabled.
    assert_eq!(emu.bus().apu().triangle().length_counter(), 10);
    assert_eq!(emu.bus().apu().noise().length_counter(), 10);
    assert!(emu.bus().apu().triangle().enabled());
    assert!(emu.bus().apu().noise().enabled());
}
