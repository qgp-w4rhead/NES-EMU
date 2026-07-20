//! Save state integration tests — round-trip integrity for the full
//! emulator state (M20).
//!
//! These tests build a minimal NROM cartridge, run the emulator for a few
//! frames, serialise the state with `EmulatorState::save_state`, restore it
//! into a fresh emulator with `EmulatorState::load_state`, and verify that
//! every component (CPU, RAM, PPU, APU, joypad, cartridge/mapper, audio)
//! matches the original. They also exercise the error paths (version
//! mismatch, corrupt data) and test save/restore across different mapper
//! types (MMC1, MMC3, UxROM, CNROM, AxROM).

use nes_emu::cartridge::Cartridge;
use nes_emu::emulator::EmulatorState;
use nes_emu::save_state::{SaveStateError, SAVE_STATE_VERSION};

/// Build a minimal NROM-128 cartridge (16 KB PRG, 8 KB CHR-RAM) whose
/// PRG is filled with `0xEA` (NOP) and whose RESET vector points to
/// `$C000`.
fn make_nop_cart() -> Cartridge {
    let mut bytes = vec![b'N', b'E', b'S', 0x1A, 1, 0, 0, 0];
    bytes.extend_from_slice(&[0u8; 8]); // remaining header
    bytes.resize(16 + 16 * 1024, 0xEA); // PRG filled with NOP
    let reset_off = 16 + 0x3FFC;
    bytes[reset_off] = 0x00;
    bytes[reset_off + 1] = 0xC0;
    Cartridge::from_bytes(&bytes).expect("build NOP cart")
}

/// Build an in-memory iNES image with the given mapper number and PRG/CHR
/// bank counts. PRG is filled with `0xEA` (NOP); the RESET vector points
/// to `$C000` (for 16K carts, mirrors `$8000`).
fn make_ines(prg_banks: u8, chr_banks: u8, flags6: u8, flags7: u8) -> Vec<u8> {
    let prg_size = prg_banks as usize * 16 * 1024;
    let chr_size = chr_banks as usize * 8 * 1024;
    let mut buf = Vec::with_capacity(16 + prg_size + chr_size);
    buf.extend_from_slice(&[b'N', b'E', b'S', 0x1A]);
    buf.push(prg_banks);
    buf.push(chr_banks);
    buf.push(flags6);
    buf.push(flags7);
    buf.extend_from_slice(&[0u8; 8]);
    buf.resize(16 + prg_size + chr_size, 0xEA);
    // Set RESET vector to $C000 (works for 16K mirror and 32K linear).
    let reset_off = 16 + 0x3FFC;
    if reset_off + 1 < buf.len() {
        buf[reset_off] = 0x00;
        buf[reset_off + 1] = 0xC0;
    }
    buf
}

// ---- Basic round-trip --------------------------------------------------

#[test]
fn save_state_round_trip_preserves_cpu_registers() {
    let mut emu = EmulatorState::new(make_nop_cart());
    emu.reset();
    // Run a few frames to get non-trivial CPU state.
    for _ in 0..3 {
        emu.step_frame();
    }

    let saved = emu.save_state().expect("save");
    let mut restored = EmulatorState::new(make_nop_cart());
    restored.reset();
    restored.load_state(&saved).expect("load");

    assert_eq!(restored.cpu().a, emu.cpu().a);
    assert_eq!(restored.cpu().x, emu.cpu().x);
    assert_eq!(restored.cpu().y, emu.cpu().y);
    assert_eq!(restored.cpu().sp, emu.cpu().sp);
    assert_eq!(restored.cpu().pc, emu.cpu().pc);
    assert_eq!(restored.cpu().status, emu.cpu().status);
}

#[test]
fn save_state_round_trip_preserves_ram() {
    let mut emu = EmulatorState::new(make_nop_cart());
    emu.reset();
    // Write some known values to RAM via the bus.
    emu.bus_mut().write(0x0000, 0x42);
    emu.bus_mut().write(0x0100, 0xAB);
    emu.bus_mut().write(0x07FF, 0xCD);

    let saved = emu.save_state().expect("save");
    let mut restored = EmulatorState::new(make_nop_cart());
    restored.reset();
    restored.load_state(&saved).expect("load");

    assert_eq!(restored.bus_mut().read(0x0000), 0x42);
    assert_eq!(restored.bus_mut().read(0x0100), 0xAB);
    assert_eq!(restored.bus_mut().read(0x07FF), 0xCD);
}

#[test]
fn save_state_round_trip_preserves_ppu_state() {
    let mut emu = EmulatorState::new(make_nop_cart());
    emu.reset();
    // Write some PPU state.
    emu.bus_mut().write(0x2000, 0x80); // PPUCTRL — NMI enable
    emu.bus_mut().write(0x2001, 0x1E); // PPUMASK — show bg+sprites
    emu.bus_mut().write(0x2003, 0x00); // OAMADDR
    emu.bus_mut().write(0x2004, 0x5A); // OAMDATA
    emu.bus_mut().write(0x2006, 0x21); // PPUADDR hi
    emu.bus_mut().write(0x2006, 0x00); // PPUADDR lo
    emu.bus_mut().write(0x2007, 0x77); // PPUDATA → VRAM $2100

    let saved = emu.save_state().expect("save");
    let mut restored = EmulatorState::new(make_nop_cart());
    restored.reset();
    restored.load_state(&saved).expect("load");

    assert_eq!(restored.bus().ppu().ppuctrl(), 0x80);
    assert_eq!(restored.bus().ppu().ppumask(), 0x1E);
    // OAM[0] should hold 0x5A.
    assert_eq!(restored.bus().ppu().oam()[0], 0x5A);
    // VRAM at $2100 should hold 0x77 (nametable byte).
    assert_eq!(restored.bus().ppu().read_nametable(0x2100), 0x77);
}

#[test]
fn save_state_round_trip_preserves_ppu_palette() {
    let mut emu = EmulatorState::new(make_nop_cart());
    emu.reset();
    // Write palette entries via $2006/$2007. Use $3F00 and $3F01 (not
    // $3F10, which mirrors $3F00).
    emu.bus_mut().write(0x2006, 0x3F);
    emu.bus_mut().write(0x2006, 0x00);
    emu.bus_mut().write(0x2007, 0x15);
    emu.bus_mut().write(0x2006, 0x3F);
    emu.bus_mut().write(0x2006, 0x01);
    emu.bus_mut().write(0x2007, 0x27);

    let saved = emu.save_state().expect("save");
    let mut restored = EmulatorState::new(make_nop_cart());
    restored.reset();
    restored.load_state(&saved).expect("load");

    assert_eq!(restored.bus().ppu().read_palette(0x3F00), 0x15);
    assert_eq!(restored.bus().ppu().read_palette(0x3F01), 0x27);
}

#[test]
fn save_state_round_trip_preserves_apu_state() {
    let mut emu = EmulatorState::new(make_nop_cart());
    emu.reset();
    // Write some APU registers. Enable pulse 1 BEFORE loading the length
    // counter — the length only loads when the channel is enabled.
    emu.bus_mut().write(0x4000, 0x3F); // Pulse 1: duty=0, constant vol, vol=15
    emu.bus_mut().write(0x4015, 0x01); // Enable pulse 1
    emu.bus_mut().write(0x4002, 0x77); // Pulse 1 timer low
    emu.bus_mut().write(0x4003, 0x08); // Pulse 1 length + timer high → length=254
    emu.bus_mut().write(0x4017, 0xC0); // 5-step mode, IRQ inhibit

    let saved = emu.save_state().expect("save");
    let mut restored = EmulatorState::new(make_nop_cart());
    restored.reset();
    restored.load_state(&saved).expect("load");

    // $4015 read returns the channel status — pulse 1 length should be
    // loaded (bit 0 set).
    let status = restored.bus_mut().read(0x4015);
    assert_eq!(
        status & 0x01,
        0x01,
        "pulse 1 length counter should be loaded"
    );
    // Open-bus latch for $4017 should be 0xC0.
    assert_eq!(restored.bus_mut().read(0x4017), 0xC0);
}

#[test]
fn save_state_round_trip_preserves_joypad_state() {
    let mut emu = EmulatorState::new(make_nop_cart());
    emu.reset();
    // Strobe + read to advance the joypad counter.
    emu.bus_mut().write(0x4016, 0x01);
    emu.bus_mut().write(0x4016, 0x00);
    emu.bus_mut().read(0x4016); // read 1 bit
    emu.bus_mut().read(0x4016); // read 2 bits

    let saved = emu.save_state().expect("save");
    let mut restored = EmulatorState::new(make_nop_cart());
    restored.reset();
    restored.load_state(&saved).expect("load");

    // The joypad read counter should be at 2 — the next 6 reads should
    // return button bits (all 0 since no buttons pressed), then 1s.
    for _ in 0..6 {
        assert_eq!(restored.bus_mut().read(0x4016), 0);
    }
    // After 8 total reads, subsequent reads return 1.
    assert_eq!(restored.bus_mut().read(0x4016), 1);
}

#[test]
fn save_state_round_trip_preserves_cartridge_prg_rom() {
    let mut emu = EmulatorState::new(make_nop_cart());
    emu.reset();
    let saved = emu.save_state().expect("save");

    let mut restored = EmulatorState::new(make_nop_cart());
    restored.reset();
    restored.load_state(&saved).expect("load");

    // PRG-ROM should be intact — $C000 reads 0xEA (NOP fill).
    assert_eq!(restored.bus_mut().read(0xC000), 0xEA);
    assert_eq!(restored.bus_mut().read(0xFFFF), 0xEA);
}

#[test]
fn save_state_round_trip_preserves_chr_ram() {
    // CHR-RAM cartridge (chr_rom_banks = 0).
    let mut emu = EmulatorState::new(make_nop_cart());
    emu.reset();
    // Write some CHR-RAM data via $2006/$2007 (CHR space $0000-$1FFF).
    emu.bus_mut().write(0x2006, 0x00);
    emu.bus_mut().write(0x2006, 0x00);
    emu.bus_mut().write(0x2007, 0x55);
    emu.bus_mut().write(0x2006, 0x1F);
    emu.bus_mut().write(0x2006, 0xFF);
    emu.bus_mut().write(0x2007, 0xAA);

    let saved = emu.save_state().expect("save");
    let mut restored = EmulatorState::new(make_nop_cart());
    restored.reset();
    restored.load_state(&saved).expect("load");

    // Read CHR-RAM back via the cartridge.
    let cart = restored.bus().cartridge().expect("cartridge present");
    assert_eq!(cart.read_chr(0x0000), 0x55);
    assert_eq!(cart.read_chr(0x1FFF), 0xAA);
}

#[test]
fn save_state_round_trip_preserves_audio_accumulator() {
    let mut emu = EmulatorState::new(make_nop_cart());
    emu.reset();
    emu.step_frame(); // Produces audio samples + advances the accumulator.

    let saved = emu.save_state().expect("save");
    let mut restored = EmulatorState::new(make_nop_cart());
    restored.reset();
    restored.load_state(&saved).expect("load");

    assert_eq!(restored.sample_accumulator(), emu.sample_accumulator());
    // Audio buffer lengths should match.
    assert_eq!(restored.audio_buffer().len(), emu.audio_buffer().len(),);
    // And the sample values should match.
    for (a, b) in emu
        .audio_buffer()
        .iter()
        .zip(restored.audio_buffer().iter())
    {
        assert_eq!(a, b);
    }
}

#[test]
fn save_state_round_trip_produces_identical_subsequent_frames() {
    let mut emu_a = EmulatorState::new(make_nop_cart());
    emu_a.reset();
    for _ in 0..5 {
        emu_a.step_frame();
    }

    let saved = emu_a.save_state().expect("save");

    // Restore into a fresh emulator.
    let mut emu_b = EmulatorState::new(make_nop_cart());
    emu_b.reset();
    emu_b.load_state(&saved).expect("load");

    // Run both forward one more frame and compare the framebuffers.
    emu_a.step_frame();
    emu_b.step_frame();

    let fb_a = emu_a.framebuffer();
    let fb_b = emu_b.framebuffer();
    assert_eq!(fb_a.len(), fb_b.len());
    for (i, (a, b)) in fb_a.iter().zip(fb_b.iter()).enumerate() {
        assert_eq!(a, b, "framebuffer pixel {i} differs after restore");
    }
}

// ---- Error paths -------------------------------------------------------

#[test]
fn load_state_rejects_version_mismatch() {
    let mut emu = EmulatorState::new(make_nop_cart());
    emu.reset();
    let mut saved = emu.save_state().expect("save");

    // Corrupt the version field (first 4 bytes of the bincode payload).
    // bincode encodes u32 as little-endian; flip the version to something
    // that will never match.
    saved[0] = 0xFF;
    saved[1] = 0xFF;
    saved[2] = 0xFF;
    saved[3] = 0xFF;

    let mut restored = EmulatorState::new(make_nop_cart());
    restored.reset();
    let err = restored.load_state(&saved).unwrap_err();
    assert!(
        matches!(err, SaveStateError::VersionMismatch { .. }),
        "expected VersionMismatch, got {err:?}",
    );
}

#[test]
fn load_state_rejects_truncated_data() {
    let mut emu = EmulatorState::new(make_nop_cart());
    emu.reset();
    let saved = emu.save_state().expect("save");

    // Truncate to just the version field (4 bytes).
    let truncated = &saved[..4];

    let mut restored = EmulatorState::new(make_nop_cart());
    restored.reset();
    let err = restored.load_state(truncated).unwrap_err();
    assert!(
        matches!(err, SaveStateError::Decode(_)),
        "expected Decode error, got {err:?}",
    );
}

#[test]
fn load_state_rejects_empty_data() {
    let mut restored = EmulatorState::new(make_nop_cart());
    restored.reset();
    let err = restored.load_state(&[]).unwrap_err();
    assert!(
        matches!(err, SaveStateError::Decode(_)),
        "expected Decode error, got {err:?}",
    );
}

#[test]
fn save_state_version_is_three() {
    // M32 bumped the version: the Ppu and Apu structs gained a `region`
    // field, which changes the bincode layout.
    assert_eq!(SAVE_STATE_VERSION, 3);
}

// ---- No-cartridge save/restore -----------------------------------------

#[test]
fn save_state_with_no_cartridge_round_trips() {
    // Build an emulator with no cartridge — use a NOP cart to construct,
    // then remove the cartridge before saving.
    let mut emu = EmulatorState::new(make_nop_cart());
    emu.reset();
    emu.bus_mut().remove_cartridge();

    let saved = emu.save_state().expect("save");
    let mut restored = EmulatorState::new(make_nop_cart());
    restored.reset();
    restored.bus_mut().remove_cartridge();
    restored.load_state(&saved).expect("load");

    // No cartridge → cartridge space reads 0.
    assert!(restored.bus().cartridge().is_none());
    assert_eq!(restored.bus_mut().read(0x8000), 0x00);
}

// ---- Cross-mapper save/restore -----------------------------------------

#[test]
fn save_state_round_trip_mmc1() {
    // Mapper 1 (MMC1): 32KB PRG, 0 CHR (CHR-RAM), battery flag set.
    let bytes = make_ines(2, 0, 0b0001_0010, 0);
    let cart = Cartridge::from_bytes(&bytes).expect("load MMC1 cart");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    // Switch PRG bank via MMC1 serial register: write 5 bits to $E000.
    emu.bus_mut().write(0xE000, 0b10000); // bit 0
    emu.bus_mut().write(0xE000, 0b10000); // bit 0
    emu.bus_mut().write(0xE000, 0b10000); // bit 0
    emu.bus_mut().write(0xE000, 0b10000); // bit 0
    emu.bus_mut().write(0xE000, 0b10001); // bit 1 (commit: prg_bank = 0b10000)

    let saved = emu.save_state().expect("save");
    let bytes2 = make_ines(2, 0, 0b0001_0010, 0);
    let cart2 = Cartridge::from_bytes(&bytes2).expect("load MMC1 cart 2");
    let mut restored = EmulatorState::new(cart2);
    restored.reset();
    restored.load_state(&saved).expect("load");

    // The MMC1 PRG bank register should be preserved.
    // After restoring, reading $8000 should reflect the banked PRG data.
    // Both emulators should read the same value at $8000.
    assert_eq!(restored.bus_mut().read(0x8000), emu.bus_mut().read(0x8000),);
}

#[test]
fn save_state_round_trip_mmc3() {
    // Mapper 4 (MMC3): 32KB PRG, 0 CHR (CHR-RAM).
    let bytes = make_ines(2, 0, 0b0100_0000, 0);
    let cart = Cartridge::from_bytes(&bytes).expect("load MMC3 cart");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    // Write some MMC3 registers: bank select + bank data.
    emu.bus_mut().write(0x8000, 0x06); // Select R6 (PRG bank at $8000)
    emu.bus_mut().write(0x8001, 0x03); // R6 = 3
    emu.bus_mut().write(0x8000, 0x07); // Select R7 (PRG bank at $A000)
    emu.bus_mut().write(0x8001, 0x01); // R7 = 1

    let saved = emu.save_state().expect("save");
    let bytes2 = make_ines(2, 0, 0b0100_0000, 0);
    let cart2 = Cartridge::from_bytes(&bytes2).expect("load MMC3 cart 2");
    let mut restored = EmulatorState::new(cart2);
    restored.reset();
    restored.load_state(&saved).expect("load");

    // PRG banking should be preserved — $8000 and $A000 should read the
    // same values in both emulators.
    assert_eq!(restored.bus_mut().read(0x8000), emu.bus_mut().read(0x8000),);
    assert_eq!(restored.bus_mut().read(0xA000), emu.bus_mut().read(0xA000),);
}

#[test]
fn save_state_round_trip_uxrom() {
    // Mapper 2 (UxROM): 32KB PRG (2 banks), 0 CHR (CHR-RAM).
    let bytes = make_ines(2, 0, 0b0010_0000, 0);
    let cart = Cartridge::from_bytes(&bytes).expect("load UxROM cart");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    // Select bank 1 at $8000.
    emu.bus_mut().write(0x8000, 0x01);

    let saved = emu.save_state().expect("save");
    let bytes2 = make_ines(2, 0, 0b0010_0000, 0);
    let cart2 = Cartridge::from_bytes(&bytes2).expect("load UxROM cart 2");
    let mut restored = EmulatorState::new(cart2);
    restored.reset();
    restored.load_state(&saved).expect("load");

    assert_eq!(restored.bus_mut().read(0x8000), emu.bus_mut().read(0x8000),);
}

#[test]
fn save_state_round_trip_cnrom() {
    // Mapper 3 (CNROM): 16KB PRG, 8KB CHR-RAM (0 CHR banks).
    let bytes = make_ines(1, 0, 0b0011_0000, 0);
    let cart = Cartridge::from_bytes(&bytes).expect("load CNROM cart");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    // Write CHR-RAM data.
    emu.bus_mut().write(0x2006, 0x00);
    emu.bus_mut().write(0x2006, 0x00);
    emu.bus_mut().write(0x2007, 0x99);

    let saved = emu.save_state().expect("save");
    let bytes2 = make_ines(1, 0, 0b0011_0000, 0);
    let cart2 = Cartridge::from_bytes(&bytes2).expect("load CNROM cart 2");
    let mut restored = EmulatorState::new(cart2);
    restored.reset();
    restored.load_state(&saved).expect("load");

    // CHR-RAM data should be preserved.
    let r_cart = restored.bus().cartridge().expect("cart present");
    assert_eq!(r_cart.read_chr(0x0000), 0x99);
}

#[test]
fn save_state_round_trip_axrom() {
    // Mapper 7 (AxROM): 32KB PRG, 0 CHR (CHR-RAM).
    let bytes = make_ines(2, 0, 0b0111_0000, 0);
    let cart = Cartridge::from_bytes(&bytes).expect("load AxROM cart");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    // Select PRG bank 1 + single-screen NT 1 (bit 4 = 1).
    emu.bus_mut().write(0x8000, 0b0001_0001);

    let saved = emu.save_state().expect("save");
    let bytes2 = make_ines(2, 0, 0b0111_0000, 0);
    let cart2 = Cartridge::from_bytes(&bytes2).expect("load AxROM cart 2");
    let mut restored = EmulatorState::new(cart2);
    restored.reset();
    restored.load_state(&saved).expect("load");

    // PRG bank + mirroring should be preserved.
    assert_eq!(restored.bus_mut().read(0x8000), emu.bus_mut().read(0x8000),);
    assert_eq!(
        restored.bus().cartridge().unwrap().mirror_mode(),
        emu.bus().cartridge().unwrap().mirror_mode(),
    );
}

// ---- Determinism -------------------------------------------------------

#[test]
fn save_state_is_deterministic() {
    let mut emu = EmulatorState::new(make_nop_cart());
    emu.reset();
    for _ in 0..3 {
        emu.step_frame();
    }

    let saved1 = emu.save_state().expect("save 1");
    let saved2 = emu.save_state().expect("save 2");

    // The same state should serialise to the same bytes every time.
    assert_eq!(saved1, saved2);
}

// ---- Deeper cross-mapper edge cases ------------------------------------

#[test]
fn save_state_round_trip_mmc1_mid_serial_register() {
    // MMC1's 5-bit serial shift register: partially written (3 of 5 bits)
    // but not yet committed. The shift_count and shift_reg must be
    // preserved so the remaining 2 writes complete the correct value.
    let bytes = make_ines(2, 0, 0b0001_0010, 0);
    let cart = Cartridge::from_bytes(&bytes).expect("load MMC1 cart");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    // Write 3 bits to $E000 (PRG bank register) without committing.
    emu.bus_mut().write(0xE000, 1); // bit 0 → sr bit 0
    emu.bus_mut().write(0xE000, 0); // bit 0 → sr bit 1
    emu.bus_mut().write(0xE000, 1); // bit 0 → sr bit 2
                                    // 3 of 5 bits written — not yet committed.

    let saved = emu.save_state().expect("save");
    let bytes2 = make_ines(2, 0, 0b0001_0010, 0);
    let cart2 = Cartridge::from_bytes(&bytes2).expect("load MMC1 cart 2");
    let mut restored = EmulatorState::new(cart2);
    restored.reset();
    restored.load_state(&saved).expect("load");

    // Complete the remaining 2 writes in both emulators.
    emu.bus_mut().write(0xE000, 0); // bit 3
    emu.bus_mut().write(0xE000, 0); // bit 4 → commit: sr = 0b00101 = 5
    restored.bus_mut().write(0xE000, 0);
    restored.bus_mut().write(0xE000, 0);

    // Both should now have the same PRG bank selected → same $8000 data.
    assert_eq!(restored.bus_mut().read(0x8000), emu.bus_mut().read(0x8000),);
}

#[test]
fn save_state_round_trip_mmc3_irq_counter() {
    // MMC3 IRQ counter state: latch, counter, enable, reload flag, pending.
    let bytes = make_ines(2, 0, 0b0100_0000, 0);
    let cart = Cartridge::from_bytes(&bytes).expect("load MMC3 cart");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    // Set up IRQ counter: latch = 5, reload, enable.
    emu.bus_mut().write(0xC000, 5); // IRQ latch = 5
    emu.bus_mut().write(0xC001, 0); // IRQ reload flag
    emu.bus_mut().write(0xE001, 0); // IRQ enable

    // Check that the mapper reports IRQ pending before any clocking
    // (reload flag set → counter reloads on next clock, not yet pending).
    assert!(!emu.bus().cart_irq_pending());

    let saved = emu.save_state().expect("save");
    let bytes2 = make_ines(2, 0, 0b0100_0000, 0);
    let cart2 = Cartridge::from_bytes(&bytes2).expect("load MMC3 cart 2");
    let mut restored = EmulatorState::new(cart2);
    restored.reset();
    restored.load_state(&saved).expect("load");

    // The IRQ latch should be preserved — reload and check.
    restored.bus_mut().write(0xC001, 0); // reload again
                                         // After reload + clock, the counter should be 5 (from the latch).
                                         // Both emulators should behave identically after the same number of
                                         // IRQ clocks.
    for _ in 0..6 {
        restored.bus_mut().cartridge_mut().unwrap().clock_irq();
        emu.bus_mut().cartridge_mut().unwrap().clock_irq();
    }
    // After 6 clocks (reload on 1st, then 5 decrements → 0 → reload + IRQ),
    // both should have IRQ pending.
    assert_eq!(
        restored.bus().cart_irq_pending(),
        emu.bus().cart_irq_pending(),
    );
}

#[test]
fn save_state_round_trip_mmc3_chr_banking() {
    // MMC3 CHR banking via $8000/$8001 — verify CHR-RAM data at banked
    // addresses survives round-trip.
    let bytes = make_ines(2, 0, 0b0100_0000, 0);
    let cart = Cartridge::from_bytes(&bytes).expect("load MMC3 cart");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    // Select CHR mode 1 (4KB) via $8000 bit 7, and set R2 = 1.
    emu.bus_mut().write(0x8000, 0b1000_0010); // CHR mode 1, select R2
    emu.bus_mut().write(0x8001, 0x01); // R2 = 1 → CHR $0800 = bank 1
                                       // Write CHR-RAM data at $0800 via $2006/$2007.
    emu.bus_mut().write(0x2006, 0x08);
    emu.bus_mut().write(0x2006, 0x00);
    emu.bus_mut().write(0x2007, 0x33);

    let saved = emu.save_state().expect("save");
    let bytes2 = make_ines(2, 0, 0b0100_0000, 0);
    let cart2 = Cartridge::from_bytes(&bytes2).expect("load MMC3 cart 2");
    let mut restored = EmulatorState::new(cart2);
    restored.reset();
    restored.load_state(&saved).expect("load");

    // CHR-RAM at $0800 should be preserved.
    let r_cart = restored.bus().cartridge().expect("cart present");
    assert_eq!(r_cart.read_chr(0x0800), 0x33);
}
