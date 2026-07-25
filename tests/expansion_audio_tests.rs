//! Expansion audio integration tests (M35).
//!
//! Exercises the four expansion-audio chips added in M35:
//! - VRC6 (mapper 24/26) — 2 pulses + sawtooth
//! - VRC7 (mapper 85) — YM2413 OPLL FM synthesis
//! - Sunsoft 5B (mapper 69, FME-7 + YM2149) — 3 tone + noise PSG
//! - Namco 163 (mapper 19) — 8-channel wavetable
//!
//! And the emulator-level mix path that combines the internal APU output
//! with `bus.expansion_audio_sample()`.

use nes_emu::cartridge::Cartridge;
use nes_emu::emulator::EmulatorState;
use nes_emu::mappers::fme7::Fme7;
use nes_emu::mappers::namco163::Namco163;
use nes_emu::mappers::vrc6::Vrc6;
use nes_emu::mappers::vrc7::Vrc7;
use nes_emu::mappers::Mapper;
use nes_emu::mappers::Mirroring;
use nes_emu::save_state::SAVE_STATE_VERSION;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a minimal iNES cartridge image with the given mapper number and
/// PRG/CHR bank counts. PRG is filled with NOPs (0xEA) and the reset
/// vector points to $C000.
fn make_ines(mapper: u8, prg_banks: usize, chr_banks: usize) -> Vec<u8> {
    let mut bytes = vec![b'N', b'E', b'S', 0x1A];
    bytes.push(prg_banks as u8);
    bytes.push(chr_banks as u8);
    // byte 6: bits 4-7 = low nibble of mapper number
    bytes.push((mapper & 0x0F) << 4);
    // byte 7: bits 4-7 = high nibble of mapper number
    bytes.push(mapper & 0xF0);
    bytes.extend_from_slice(&[0u8; 8]);
    let prg_size = prg_banks * 16 * 1024;
    let chr_size = chr_banks * 8 * 1024;
    bytes.resize(16 + prg_size, 0xEA);
    // Reset vector → $C000.
    let reset_off = 16 + 0x3FFC;
    if reset_off + 1 < bytes.len() {
        bytes[reset_off] = 0x00;
        bytes[reset_off + 1] = 0xC0;
    }
    bytes.resize(16 + prg_size + chr_size, 0x00);
    bytes
}

fn make_prg(banks_8k: usize) -> Vec<u8> {
    let mut prg = vec![0u8; banks_8k * 8 * 1024];
    for (i, b) in prg.iter_mut().enumerate() {
        *b = (i / (8 * 1024)) as u8;
    }
    prg
}

fn make_chr(banks_1k: usize) -> Vec<u8> {
    let mut chr = vec![0u8; banks_1k * 1024];
    for (i, b) in chr.iter_mut().enumerate() {
        *b = (i / 1024) as u8;
    }
    chr
}

// ---------------------------------------------------------------------------
// VRC6 (mapper 24)
// ---------------------------------------------------------------------------

#[test]
fn vrc6_expansion_audio_sample_silence_when_disabled() {
    let vrc = Vrc6::new(make_prg(4), make_chr(8), Mirroring::Vertical, false, false);
    // All channels disabled → sample should be at the silence floor.
    // VRC6_GAIN (0.75) scales the -1.0 DC-offset silence convention.
    let s = vrc.expansion_audio_sample();
    assert!(s.abs() < 1.0, "sample in range, got {s}");
    assert!(
        (s - (-0.75)).abs() < 1e-3,
        "silence should be -0.75 (gain*dc), got {s}"
    );
}

#[test]
fn vrc6_expansion_audio_sample_nonzero_when_pulse_enabled() {
    let mut vrc = Vrc6::new(make_prg(4), make_chr(8), Mirroring::Vertical, false, false);
    // Pulse 1: volume 15, duty 4/16, enable.
    vrc.write_prg(0x9000, 0x4F); // duty=4 (bits 4-6=100), volume=15
    vrc.write_prg(0x9001, 0x10); // period low
    vrc.write_prg(0x9002, 0x80 | 0x01); // enable + period high=1
    vrc.clock_cpu(2000);
    let s = vrc.expansion_audio_sample();
    assert!(
        s.abs() > 0.0,
        "enabled pulse should produce non-zero sample, got {s}"
    );
}

// ---------------------------------------------------------------------------
// VRC7 (mapper 85) — OPLL FM synthesis
// ---------------------------------------------------------------------------

#[test]
fn vrc7_expansion_audio_silence_when_no_key_on() {
    let vrc = Vrc7::new(make_prg(8), make_chr(8), Mirroring::Vertical, false);
    let s = vrc.expansion_audio_sample();
    assert!(s.abs() < 1e-6, "no key-on should be silent (0.0), got {s}");
}

#[test]
fn vrc7_opll_key_on_produces_sound() {
    let mut vrc = Vrc7::new(make_prg(8), make_chr(8), Mirroring::Vertical, false);
    // Channel 0: instrument 0, max volume (0 = loudest).
    vrc.write_prg(0x9010, 0x30);
    vrc.write_prg(0x9030, 0x00);
    // Fnum low = 0x40.
    vrc.write_prg(0x9010, 0x20);
    vrc.write_prg(0x9030, 0x40);
    // Fnum high (bit 0) + block=1 + key-on (bit 4).
    vrc.write_prg(0x9010, 0x40);
    vrc.write_prg(0x9030, 0x12); // bit4=1 key-on, bits1-3=001 block=1, bit0=0
                                 // Clock in small increments and collect samples — a single sample can
                                 // land on a sine zero crossing, so we check the max over several.
    let mut max_abs = 0.0_f32;
    for _ in 0..100 {
        vrc.clock_cpu(97); // prime step to avoid phase aliasing
        max_abs = max_abs.max(vrc.expansion_audio_sample().abs());
    }
    assert!(
        max_abs > 0.0,
        "key-on should produce non-zero output, max={max_abs}"
    );
}

#[test]
fn vrc7_opll_key_off_silences_channel() {
    let mut vrc = Vrc7::new(make_prg(8), make_chr(8), Mirroring::Vertical, false);
    vrc.write_prg(0x9010, 0x30);
    vrc.write_prg(0x9030, 0x00);
    vrc.write_prg(0x9010, 0x20);
    vrc.write_prg(0x9030, 0x40);
    vrc.write_prg(0x9010, 0x40);
    vrc.write_prg(0x9030, 0x12);
    vrc.clock_cpu(4000);
    // Key off: clear bit 4.
    vrc.write_prg(0x9010, 0x40);
    vrc.write_prg(0x9030, 0x02);
    vrc.clock_cpu(20000); // release to silence
    let s = vrc.expansion_audio_sample();
    assert!(s.abs() < 1e-2, "key-off should silence channel, got {s}");
}

#[test]
fn vrc7_prg_banking_switches_8k_windows() {
    let mut vrc = Vrc7::new(make_prg(8), make_chr(8), Mirroring::Vertical, false);
    // Bank 0 at $8000 = 0, bank 1 at $A000 = 1, bank 2 at $C000 = 2.
    vrc.write_prg(0x8000, 0);
    vrc.write_prg(0x8008, 1);
    vrc.write_prg(0x9000, 2);
    assert_eq!(vrc.read_prg(0x8000), 0);
    assert_eq!(vrc.read_prg(0xA000), 1);
    assert_eq!(vrc.read_prg(0xC000), 2);
    // $E000-$FFFF fixed to last bank (7).
    assert_eq!(vrc.read_prg(0xE000), 7);
}

#[test]
fn vrc7_irq_fires_after_counter_reaches_zero() {
    let mut vrc = Vrc7::new(make_prg(8), make_chr(8), Mirroring::Vertical, false);
    // Load latch = 0x0010 (16), enable.
    vrc.write_prg(0xE000, 0x10); // low byte
    vrc.write_prg(0xE000, 0x00); // high byte
    vrc.write_prg(0xE008, 0x03); // enable + ack (bit 1)
    assert!(!vrc.irq_pending());
    vrc.clock_cpu(20);
    assert!(vrc.irq_pending(), "IRQ should fire after counter hits 0");
}

// ---------------------------------------------------------------------------
// Sunsoft 5B (FME-7 + YM2149)
// ---------------------------------------------------------------------------

#[test]
fn fme7_ym2149_silence_when_all_channels_off() {
    let fme = Fme7::new(make_prg(4), make_chr(8), Mirroring::Vertical, false);
    let s = fme.expansion_audio_sample();
    // YM2149 silence: all channels off → sum=0 → 0.0 (no DC offset).
    assert!(s.abs() < 1e-6, "silence should be 0.0, got {s}");
}

#[test]
fn fme7_ym2149_tone_channel_produces_sound() {
    let mut fme = Fme7::new(make_prg(4), make_chr(8), Mirroring::Vertical, false);
    // Address latch $C000, data $E000.
    // Reg 0 (ch A period low) = 0x40, reg 1 (high) = 0x01.
    fme.write_prg(0xC000, 0x00);
    fme.write_prg(0xE000, 0x40);
    fme.write_prg(0xC000, 0x01);
    fme.write_prg(0xE000, 0x01);
    // Reg 7 (enable): clear bit 0 → tone A ON.
    fme.write_prg(0xC000, 0x07);
    fme.write_prg(0xE000, 0x3E);
    // Reg 8 (ch A volume) = 10.
    fme.write_prg(0xC000, 0x08);
    fme.write_prg(0xE000, 0x0A);
    fme.clock_cpu(4000);
    let s = fme.expansion_audio_sample();
    assert!(s.abs() > 0.0, "tone channel should produce sound, got {s}");
}

#[test]
fn fme7_ym2149_noise_channel_produces_sound() {
    let mut fme = Fme7::new(make_prg(4), make_chr(8), Mirroring::Vertical, false);
    // Short noise period.
    fme.write_prg(0xC000, 0x06);
    fme.write_prg(0xE000, 0x01);
    // Enable noise on channel A (clear bit 3).
    fme.write_prg(0xC000, 0x07);
    fme.write_prg(0xE000, 0x36);
    // Volume 12.
    fme.write_prg(0xC000, 0x08);
    fme.write_prg(0xE000, 0x0C);
    // Clock and sample multiple times — a single sample may land on a
    // noise zero crossing, so check the max over several.
    let mut max_abs = 0.0_f32;
    for _ in 0..50 {
        fme.clock_cpu(200);
        max_abs = max_abs.max(fme.expansion_audio_sample().abs());
    }
    assert!(
        max_abs > 0.0,
        "noise channel should produce sound, max={max_abs}"
    );
}

// ---------------------------------------------------------------------------
// Namco 163 (mapper 19)
// ---------------------------------------------------------------------------

#[test]
fn namco163_silence_when_no_channels_enabled() {
    let n = Namco163::new(make_prg(4), make_chr(8), Mirroring::Vertical, false);
    let s = n.expansion_audio_sample();
    assert!(s.abs() < 1e-6, "silence should be 0.0, got {s}");
}

#[test]
fn namco163_wavetable_channel_produces_sound() {
    let mut n = Namco163::new(make_prg(4), make_chr(8), Mirroring::Vertical, false);
    // Write a waveform into wave RAM at $4800: alternating high/low nibbles.
    // Set the address latch via $F800 (bits 0-6 = addr, bit 7 = auto-inc).
    n.write_prg(0xF800, 0x80); // addr=0, auto-increment on
    for i in 0..16u8 {
        // Write bytes with non-zero low nibbles (the first nibble read is
        // the low nibble of byte 0).
        n.write_prg(0x4800, if i % 2 == 0 { 0x0F } else { 0xF0 });
    }
    // Configure channel 0: freq, length, volume, offset, enable.
    // Channel 0 registers at $5000 + ch*8 = $5000.
    n.write_prg(0x5000, 0x40); // freq low = 0x40
    n.write_prg(0x5001, 0x14); // freq high=0x04 (bits 0-3), length=1 (bits 4-7)
    n.write_prg(0x5002, 0x0F); // volume = 15
    n.write_prg(0x5003, 0x80); // enable (bit 7), offset=0
    n.clock_cpu(8000);
    let s = n.expansion_audio_sample();
    assert!(
        s.abs() > 0.0,
        "wavetable channel should produce sound, got {s}"
    );
}

#[test]
fn namco163_wave_ram_address_latch_auto_increments() {
    let mut n = Namco163::new(make_prg(4), make_chr(8), Mirroring::Vertical, false);
    // Latch addr=0, auto-inc on.
    n.write_prg(0xF800, 0x80);
    n.write_prg(0x4800, 0xAB); // writes to addr 0, auto-inc → addr=1
    n.write_prg(0x4800, 0xCD); // writes to addr 1, auto-inc → addr=2
                               // Re-latch to addr=1 to read back the second byte.
    n.write_prg(0xF800, 0x01);
    assert_eq!(n.read_prg(0x4800), 0xCD);
    // Re-latch to addr=0 to read the first byte.
    n.write_prg(0xF800, 0x00);
    assert_eq!(n.read_prg(0x4800), 0xAB);
}

#[test]
fn namco163_prg_banking_switches_8k_windows() {
    let mut n = Namco163::new(make_prg(8), make_chr(8), Mirroring::Vertical, false);
    n.write_prg(0xC000, 1);
    n.write_prg(0xC800, 2);
    n.write_prg(0xD000, 3);
    n.write_prg(0xD800, 4);
    assert_eq!(n.read_prg(0x8000), 1);
    assert_eq!(n.read_prg(0xA000), 2);
    assert_eq!(n.read_prg(0xC000), 3);
    assert_eq!(n.read_prg(0xE000), 4);
}

#[test]
fn namco163_mirroring_switchable() {
    let mut n = Namco163::new(make_prg(4), make_chr(8), Mirroring::Vertical, false);
    n.write_prg(0xE000, 0x01); // horizontal
    assert_eq!(n.mirror_mode(), Mirroring::Horizontal);
    n.write_prg(0xE000, 0x00); // vertical
    assert_eq!(n.mirror_mode(), Mirroring::Vertical);
    n.write_prg(0xE000, 0x02); // single-screen A
    assert_eq!(n.mirror_mode(), Mirroring::SingleScreen(0));
    n.write_prg(0xE000, 0x03); // single-screen B
    assert_eq!(n.mirror_mode(), Mirroring::SingleScreen(1));
}

// ---------------------------------------------------------------------------
// Emulator-level mix
// ---------------------------------------------------------------------------

#[test]
fn emulator_mixes_expansion_audio_for_vrc7_cart() {
    let bytes = make_ines(85, 4, 1);
    let cart = Cartridge::from_bytes(&bytes).expect("build VRC7 cart");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    // Trigger a VRC7 key-on via the bus so the expansion chip makes sound.
    emu.bus_mut().write(0x9010, 0x30);
    emu.bus_mut().write(0x9030, 0x00);
    emu.bus_mut().write(0x9010, 0x20);
    emu.bus_mut().write(0x9030, 0x40);
    emu.bus_mut().write(0x9010, 0x40);
    emu.bus_mut().write(0x9030, 0x12);
    // Step enough frames to let the VRC7 produce oscillating audio.
    // The DC blocker removes constant DC offsets, so we need enough
    // frames for the OPLL to start producing actual AC audio.
    for _ in 0..10 {
        emu.step_frame();
    }
    let samples = emu.take_audio_samples();
    // We should have produced some samples; at least one should be non-silent
    // (the expansion chip + internal APU mix).
    assert!(!samples.is_empty(), "should produce audio samples");
    let max_abs = samples.iter().map(|s| s.abs()).fold(0.0_f32, f32::max);
    assert!(
        max_abs > 0.0,
        "some sample should be non-zero, max={max_abs}"
    );
}

#[test]
fn emulator_expansion_audio_zero_for_non_audio_cart() {
    // NROM (mapper 0) has no expansion audio → sample is 0.0.
    let bytes = make_ines(0, 1, 1);
    let cart = Cartridge::from_bytes(&bytes).expect("build NROM cart");
    let emu = EmulatorState::new(cart);
    assert_eq!(emu.bus().expansion_audio_sample(), 0.0);
}

// ---------------------------------------------------------------------------
// Save state version
// ---------------------------------------------------------------------------

#[test]
fn save_state_version_is_eight() {
    assert_eq!(SAVE_STATE_VERSION, 8);
}
