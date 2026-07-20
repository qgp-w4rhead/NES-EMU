//! Famicom Disk System (FDS) integration tests (M36).
//!
//! Exercises the FDS disk image parser, BIOS loader, FDS mapper (mapper
//! 20) with disk I/O registers, timer IRQ, expansion audio (wavetable +
//! modulator), and the cartridge loading path for `.fds` files.

use nes_emu::cartridge::{Cartridge, CartridgeError};
use nes_emu::fds::{self, FdsDisk, BIOS_SIZE, DISK_SIDE_SIZE, FDS_HEADER_SIZE, FDS_MAGIC};
use nes_emu::mappers::fds::Fds;
use nes_emu::mappers::fds_audio::FdsAudio;
use nes_emu::mappers::{Mapper, MapperState, Mirroring};
use nes_emu::save_state::SAVE_STATE_VERSION;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a minimal FDS disk image with the given number of disk sides.
/// Each side starts with a disk info block ($80) and is filled with a
/// recognizable pattern.
fn make_fds(disk_count: u8) -> Vec<u8> {
    let mut buf = Vec::with_capacity(FDS_HEADER_SIZE + disk_count as usize * DISK_SIDE_SIZE);
    buf.extend_from_slice(&FDS_MAGIC);
    buf.push(disk_count);
    buf.extend_from_slice(&[0xFF; 11]); // padding
    for i in 0..disk_count as usize {
        let mut side = vec![0u8; DISK_SIDE_SIZE];
        side[0] = 0x80; // disk info block type
        side[1..4].copy_from_slice(b"FDS");
        side[4] = 0x01; // disk type
        side[5] = 0x02; // game version
        side[6..14].copy_from_slice(b"TESTGAME"); // game name
        side[14..18].copy_from_slice(&[0u8; 4]); // game number
        side[18..20].copy_from_slice(&[0u8; 2]); // disk number
        side[20..22].copy_from_slice(&[0u8; 2]); // disk type
        side[22..56].copy_from_slice(&[0u8; 34]); // unused
                                                  // Fill the rest with a pattern that includes the side index.
        for (j, byte) in side.iter_mut().enumerate().take(DISK_SIDE_SIZE).skip(56) {
            *byte = ((i + j) & 0xFF) as u8;
        }
        buf.extend_from_slice(&side);
    }
    buf
}

/// Build a minimal 8 KB FDS BIOS ROM filled with NOPs, with the reset
/// vector pointing to $E000.
fn make_bios() -> Vec<u8> {
    let mut bios = vec![0xEAu8; BIOS_SIZE];
    // Reset vector at $FFFC/$FFFD → $E000.
    bios[0x1FFC] = 0x00;
    bios[0x1FFD] = 0xE0;
    // NMI vector at $FFFA/$FFFB → $E100.
    bios[0x1FFA] = 0x00;
    bios[0x1FFB] = 0xE1;
    bios
}

// ---------------------------------------------------------------------------
// FDS disk image parser
// ---------------------------------------------------------------------------

#[test]
fn fds_disk_parses_single_sided_image() {
    let data = make_fds(1);
    let disk = FdsDisk::parse(&data).expect("parse");
    assert_eq!(disk.disk_count(), 1);
    assert_eq!(disk.sides.len(), 1);
    assert_eq!(disk.sides[0].len(), DISK_SIDE_SIZE);
}

#[test]
fn fds_disk_parses_double_sided_image() {
    let data = make_fds(2);
    let disk = FdsDisk::parse(&data).expect("parse");
    assert_eq!(disk.disk_count(), 2);
    assert_eq!(disk.sides.len(), 2);
}

#[test]
fn fds_disk_rejects_bad_magic() {
    let mut data = make_fds(1);
    data[0] = b'X';
    assert!(matches!(
        FdsDisk::parse(&data),
        Err(CartridgeError::BadMagic)
    ));
}

#[test]
fn fds_disk_rejects_truncated_image() {
    let mut data = make_fds(1);
    data.truncate(FDS_HEADER_SIZE + 100);
    assert!(matches!(FdsDisk::parse(&data), Err(CartridgeError::Io(_))));
}

#[test]
fn fds_is_fds_file_detects_extension() {
    use std::path::Path;
    assert!(fds::is_fds_file(Path::new("game.fds")));
    assert!(fds::is_fds_file(Path::new("GAME.FDS")));
    assert!(!fds::is_fds_file(Path::new("game.nes")));
}

// ---------------------------------------------------------------------------
// FDS cartridge loading
// ---------------------------------------------------------------------------

#[test]
fn cartridge_from_fds_bytes_builds_mapper_20() {
    let disk_data = make_fds(1);
    let bios = make_bios();
    let cart = Cartridge::from_fds_bytes(&disk_data, bios).expect("build cartridge");
    assert_eq!(cart.header.mapper_number, 20);
    assert_eq!(cart.header.prg_rom_banks, 0);
    assert_eq!(cart.header.chr_rom_banks, 0);
    assert_eq!(cart.header.mirroring, Mirroring::Vertical);
}

#[test]
fn cartridge_from_fds_bytes_rejects_bad_disk() {
    let bad_disk = [0u8; 10];
    let bios = make_bios();
    let result = Cartridge::from_fds_bytes(&bad_disk, bios);
    assert!(matches!(result, Err(CartridgeError::TooShort)));
}

#[test]
fn cartridge_from_fds_bytes_rejects_bad_magic() {
    let mut bad_disk = make_fds(1);
    bad_disk[0] = b'X';
    let bios = make_bios();
    let result = Cartridge::from_fds_bytes(&bad_disk, bios);
    assert!(matches!(result, Err(CartridgeError::BadMagic)));
}

#[test]
fn fds_cartridge_prg_ram_writes_persist() {
    let disk_data = make_fds(1);
    let bios = make_bios();
    let mut cart = Cartridge::from_fds_bytes(&disk_data, bios).expect("cartridge");
    cart.write_prg(0x6000, 0x42);
    assert_eq!(cart.read_prg(0x6000), 0x42);
    cart.write_prg(0xDFFF, 0xAB);
    assert_eq!(cart.read_prg(0xDFFF), 0xAB);
}

#[test]
fn fds_cartridge_bios_readable_at_e000() {
    let disk_data = make_fds(1);
    let bios = make_bios();
    let cart = Cartridge::from_fds_bytes(&disk_data, bios).expect("cartridge");
    // BIOS is filled with NOPs (0xEA).
    assert_eq!(cart.read_prg(0xE000), 0xEA);
    assert_eq!(cart.read_prg(0xE001), 0xEA);
}

#[test]
fn fds_cartridge_reset_vector_points_to_e000() {
    let disk_data = make_fds(1);
    let bios = make_bios();
    let cart = Cartridge::from_fds_bytes(&disk_data, bios).expect("cartridge");
    assert_eq!(cart.read_prg(0xFFFC), 0x00);
    assert_eq!(cart.read_prg(0xFFFD), 0xE0);
}

#[test]
fn fds_cartridge_chr_ram_writes_persist() {
    let disk_data = make_fds(1);
    let bios = make_bios();
    let mut cart = Cartridge::from_fds_bytes(&disk_data, bios).expect("cartridge");
    cart.write_chr(0x0000, 0x55);
    assert_eq!(cart.read_chr(0x0000), 0x55);
    cart.write_chr(0x1FFF, 0xAA);
    assert_eq!(cart.read_chr(0x1FFF), 0xAA);
}

// ---------------------------------------------------------------------------
// FDS disk I/O registers
// ---------------------------------------------------------------------------

#[test]
fn fds_disk_read_returns_data_sequentially_via_mapper() {
    let disk_data = make_fds(1);
    let bios = make_bios();
    let mut cart = Cartridge::from_fds_bytes(&disk_data, bios).expect("cartridge");
    // Enable motor + read mode via $4025.
    cart.write_prg(0x4025, 0x20);
    // Read $4031 → first byte of disk data (block type $80).
    assert_eq!(cart.read_prg_mut(0x4031), 0x80);
    // Second read → 'F'.
    assert_eq!(cart.read_prg_mut(0x4031), b'F');
    // Third read → 'D'.
    assert_eq!(cart.read_prg_mut(0x4031), b'D');
}

#[test]
fn fds_disk_transfer_reset_resets_read_position() {
    let disk_data = make_fds(1);
    let bios = make_bios();
    let mut cart = Cartridge::from_fds_bytes(&disk_data, bios).expect("cartridge");
    cart.write_prg(0x4025, 0x20); // motor on
                                  // Read a few bytes.
    let _ = cart.read_prg_mut(0x4031);
    let _ = cart.read_prg_mut(0x4031);
    // Transfer reset (bit 0 of $4025).
    cart.write_prg(0x4025, 0x21);
    // Next read should be the first byte again.
    assert_eq!(cart.read_prg_mut(0x4031), 0x80);
}

#[test]
fn fds_disk_status_shows_motor_on() {
    let disk_data = make_fds(1);
    let bios = make_bios();
    let mut cart = Cartridge::from_fds_bytes(&disk_data, bios).expect("cartridge");
    cart.write_prg(0x4025, 0x20); // motor on
    let status = cart.read_prg_mut(0x4030);
    // bit 2 = disk ready (motor on + inserted).
    assert!(status & 0x04 != 0);
}

#[test]
fn fds_disk_status_shows_data_available() {
    let disk_data = make_fds(1);
    let bios = make_bios();
    let mut cart = Cartridge::from_fds_bytes(&disk_data, bios).expect("cartridge");
    cart.write_prg(0x4025, 0x20); // motor on, read mode
    let status = cart.read_prg_mut(0x4030);
    // bit 0 = data available.
    assert!(status & 0x01 != 0);
}

#[test]
fn fds_mirroring_switches_via_4025() {
    let disk_data = make_fds(1);
    let bios = make_bios();
    let mut cart = Cartridge::from_fds_bytes(&disk_data, bios).expect("cartridge");
    assert_eq!(cart.mirror_mode(), Mirroring::Vertical);
    cart.write_prg(0x4025, 0x02); // bit 1 = horizontal
    assert_eq!(cart.mirror_mode(), Mirroring::Horizontal);
}

// ---------------------------------------------------------------------------
// FDS timer IRQ
// ---------------------------------------------------------------------------

#[test]
fn fds_timer_irq_fires_after_countdown() {
    let disk_data = make_fds(1);
    let bios = make_bios();
    let mut cart = Cartridge::from_fds_bytes(&disk_data, bios).expect("cartridge");
    cart.write_prg(0x4020, 0x05); // latch low = 5
    cart.write_prg(0x4021, 0x00); // latch high = 0
    cart.write_prg(0x4023, 0x02); // enable timer I/O
    cart.write_prg(0x4022, 0x01); // enable timer
                                  // Counter=5: 5 decrements to 0, then 6th cycle fires IRQ.
    cart.clock_cpu(6);
    assert!(cart.irq_pending());
}

#[test]
fn fds_timer_irq_cleared_on_4030_read() {
    let disk_data = make_fds(1);
    let bios = make_bios();
    let mut cart = Cartridge::from_fds_bytes(&disk_data, bios).expect("cartridge");
    cart.write_prg(0x4020, 0x01);
    cart.write_prg(0x4021, 0x00);
    cart.write_prg(0x4023, 0x02);
    cart.write_prg(0x4022, 0x01);
    cart.clock_cpu(2);
    assert!(cart.irq_pending());
    let status = cart.read_prg_mut(0x4030);
    assert!(status & 0x80 != 0);
    assert!(!cart.irq_pending());
}

#[test]
fn fds_timer_disabled_does_not_fire() {
    let disk_data = make_fds(1);
    let bios = make_bios();
    let mut cart = Cartridge::from_fds_bytes(&disk_data, bios).expect("cartridge");
    cart.write_prg(0x4020, 0x01);
    cart.write_prg(0x4021, 0x00);
    // Don't enable timer.
    cart.clock_cpu(100);
    assert!(!cart.irq_pending());
}

// ---------------------------------------------------------------------------
// FDS expansion audio
// ---------------------------------------------------------------------------

#[test]
fn fds_audio_silent_when_frequency_zero() {
    let mut audio = FdsAudio::new();
    assert_eq!(audio.sample(), 0.0);
}

#[test]
fn fds_audio_wave_ram_writes_store_6bit() {
    let mut audio = FdsAudio::new();
    audio.write_register(0x4040, 0x3F);
    assert_eq!(audio.wave_ram()[0], 0x3F);
    audio.write_register(0x4040, 0xFF);
    assert_eq!(audio.wave_ram()[1], 0x3F); // masked to 6 bits
}

#[test]
fn fds_audio_frequency_is_12bit() {
    let mut audio = FdsAudio::new();
    audio.write_register(0x4082, 0x34);
    audio.write_register(0x4081, 0x05);
    assert_eq!(audio.freq_value(), 0x534);
}

#[test]
fn fds_audio_produces_output_with_wave_and_volume() {
    let mut audio = FdsAudio::new();
    // Fill wave RAM with max values.
    for _ in 0..64 {
        audio.write_register(0x4040, 0x3F);
    }
    // Set volume (envelope disabled, vol = 0x3F).
    audio.write_register(0x4080, 0x7F);
    // Set frequency.
    audio.write_register(0x4082, 0x00);
    audio.write_register(0x4081, 0x01);
    audio.clock(200);
    let s = audio.sample();
    // Wave = 0x3F (63), centered → (63 - 32) * 63 / 2016 > 0
    assert!(s > 0.0, "expected positive sample, got {s}");
}

#[test]
fn fds_expansion_audio_via_mapper() {
    let disk_data = make_fds(1);
    let bios = make_bios();
    let mut cart = Cartridge::from_fds_bytes(&disk_data, bios).expect("cartridge");
    // Set up wave RAM via $4040.
    for _ in 0..64 {
        cart.write_prg(0x4040, 0x3F);
    }
    // Set volume + frequency.
    cart.write_prg(0x4080, 0x7F);
    cart.write_prg(0x4082, 0x00);
    cart.write_prg(0x4081, 0x01);
    // Clock to advance audio state.
    cart.clock_cpu(200);
    // Sample should be positive (wave = 0x3F > 32).
    let s = cart.expansion_audio_sample();
    assert!(s > 0.0, "expected positive expansion sample, got {s}");
}

#[test]
fn fds_expansion_audio_silent_without_frequency() {
    let disk_data = make_fds(1);
    let bios = make_bios();
    let cart = Cartridge::from_fds_bytes(&disk_data, bios).expect("cartridge");
    // No frequency set → silence.
    assert_eq!(cart.expansion_audio_sample(), 0.0);
}

// ---------------------------------------------------------------------------
// FDS save state
// ---------------------------------------------------------------------------

#[test]
fn fds_save_state_version_is_six() {
    assert_eq!(SAVE_STATE_VERSION, 6);
}

#[test]
fn fds_save_state_round_trips_prg_ram() {
    let disk_data = make_fds(1);
    let bios = make_bios();
    let mut cart = Cartridge::from_fds_bytes(&disk_data, bios).expect("cartridge");
    cart.write_prg(0x6000, 0x99);
    cart.write_prg(0x8000, 0x88);
    let state = cart.save_mapper_state();
    // Build a new cartridge and restore.
    let mut cart2 = Cartridge::from_fds_bytes(&disk_data, make_bios()).expect("cartridge 2");
    cart2.restore_mapper_state(state);
    assert_eq!(cart2.read_prg(0x6000), 0x99);
    assert_eq!(cart2.read_prg(0x8000), 0x88);
}

#[test]
fn fds_save_state_round_trips_chr_ram() {
    let disk_data = make_fds(1);
    let bios = make_bios();
    let mut cart = Cartridge::from_fds_bytes(&disk_data, bios).expect("cartridge");
    cart.write_chr(0x1000, 0x77);
    let state = cart.save_mapper_state();
    let mut cart2 = Cartridge::from_fds_bytes(&disk_data, make_bios()).expect("cartridge 2");
    cart2.restore_mapper_state(state);
    assert_eq!(cart2.read_chr(0x1000), 0x77);
}

#[test]
fn fds_save_state_round_trips_timer_state() {
    let disk_data = make_fds(1);
    let bios = make_bios();
    let mut cart = Cartridge::from_fds_bytes(&disk_data, bios).expect("cartridge");
    cart.write_prg(0x4020, 0x10);
    cart.write_prg(0x4021, 0x00);
    cart.write_prg(0x4023, 0x02);
    cart.write_prg(0x4022, 0x01);
    let state = cart.save_mapper_state();
    let mut cart2 = Cartridge::from_fds_bytes(&disk_data, make_bios()).expect("cartridge 2");
    cart2.restore_mapper_state(state);
    // Timer should be enabled with latch 0x10. Counter starts at 0x10=16;
    // 16 decrements to 0, then the 17th cycle fires the IRQ.
    cart2.clock_cpu(17);
    assert!(cart2.irq_pending());
}

#[test]
fn fds_save_state_preserves_bios_after_restore() {
    let disk_data = make_fds(1);
    let bios = make_bios();
    let cart = Cartridge::from_fds_bytes(&disk_data, bios).expect("cartridge");
    let state = cart.save_mapper_state();
    // New cart with same BIOS — restore should keep BIOS readable.
    let mut cart2 = Cartridge::from_fds_bytes(&disk_data, make_bios()).expect("cartridge 2");
    cart2.restore_mapper_state(state);
    assert_eq!(cart2.read_prg(0xE000), 0xEA);
}

// ---------------------------------------------------------------------------
// FDS MapperState enum variant
// ---------------------------------------------------------------------------

#[test]
fn fds_mapper_state_variant_exists() {
    let fds = Fds::new(make_bios(), make_fds(1));
    let state = fds.save_state();
    assert!(matches!(state, MapperState::Fds(_)));
}

#[test]
fn fds_mapper_state_to_box_round_trips() {
    let mut fds = Fds::new(make_bios(), make_fds(1));
    fds.write_prg(0x6000, 0x42);
    let state = fds.save_state();
    let boxed = state.into_boxed_mapper();
    assert_eq!(boxed.read_prg(0x6000), 0x42);
}
