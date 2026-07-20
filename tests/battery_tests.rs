//! Battery-backed PRG-RAM persistence integration tests (M21).
//!
//! These tests exercise the full battery-SRAM pipeline:
//!
//! - `Cartridge::battery_sram` / `load_battery_sram` delegation to mappers.
//! - `EmulatorState::has_battery` / `battery_sram` / `load_battery_sram`.
//! - End-to-end persistence: write PRG-RAM via the bus, dump it through
//!   `battery::save_for_rom`, reload it through `battery::load_for_rom`,
//!   and verify the PRG-RAM contents are restored in a fresh emulator.
//! - Non-battery mappers (NROM, UxROM, CNROM, AxROM) return `None` and
//!   ignore `load_battery_sram`.
//! - MMC1 and MMC3 battery variants round-trip PRG-RAM.
//! - `load_battery_sram` handles oversized and undersized payloads.
//!
//! See: https://www.nesdev.org/wiki/INES#Flags_6
//! See: https://www.nesdev.org/wiki/MMC1#PRG-RAM
//! See: https://www.nesdev.org/wiki/MMC3#PRG-RAM

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use nes_emu::battery;
use nes_emu::cartridge::Cartridge;
use nes_emu::emulator::EmulatorState;

/// Global counter for unique temp dirs (avoids pulling in a `tempfile`
/// dev-dependency).
static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Create a unique temp directory for a single test.
fn temp_dir() -> PathBuf {
    let n = TEST_DIR_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    let path = std::env::temp_dir().join(format!("nes-emu-battery-it-{pid}-{n}"));
    std::fs::create_dir_all(&path).expect("create temp dir");
    path
}

/// Build an in-memory iNES image. `flags6` controls mirroring/battery/trainer
/// and the mapper low nibble; `flags7` supplies the mapper high nibble. PRG
/// is filled with `0xEA` (NOP) and the RESET vector points to `$C000`.
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
    let reset_off = 16 + 0x3FFC;
    if reset_off + 1 < buf.len() {
        buf[reset_off] = 0x00;
        buf[reset_off + 1] = 0xC0;
    }
    buf
}

/// MMC1 battery cart: mapper 1, 16KB PRG, 8KB CHR-RAM, battery flag set.
fn make_mmc1_battery_cart() -> Cartridge {
    // flags6: mapper low nibble = 1, battery bit (0x02) set → 0x12.
    let bytes = make_ines(1, 0, 0x12, 0x00);
    Cartridge::from_bytes(&bytes).expect("build MMC1 battery cart")
}

/// MMC1 non-battery cart: mapper 1, no battery flag.
fn make_mmc1_no_battery_cart() -> Cartridge {
    let bytes = make_ines(1, 0, 0x10, 0x00);
    Cartridge::from_bytes(&bytes).expect("build MMC1 non-battery cart")
}

/// MMC3 battery cart: mapper 4, 32KB PRG, 8KB CHR-RAM, battery flag set.
fn make_mmc3_battery_cart() -> Cartridge {
    // flags6: mapper low nibble = 4, battery bit (0x02) set → 0x42.
    let bytes = make_ines(2, 0, 0x42, 0x00);
    Cartridge::from_bytes(&bytes).expect("build MMC3 battery cart")
}

/// NROM-128 cart: mapper 0, no battery.
fn make_nrom_cart() -> Cartridge {
    let bytes = make_ines(1, 0, 0x00, 0x00);
    Cartridge::from_bytes(&bytes).expect("build NROM cart")
}

/// UxROM cart: mapper 2, no battery.
fn make_uxrom_cart() -> Cartridge {
    let bytes = make_ines(4, 0, 0x20, 0x00);
    Cartridge::from_bytes(&bytes).expect("build UxROM cart")
}

/// CNROM cart: mapper 3, no battery.
fn make_cnrom_cart() -> Cartridge {
    let bytes = make_ines(1, 4, 0x30, 0x00);
    Cartridge::from_bytes(&bytes).expect("build CNROM cart")
}

/// AxROM cart: mapper 7, no battery.
fn make_axrom_cart() -> Cartridge {
    let bytes = make_ines(4, 0, 0x70, 0x00);
    Cartridge::from_bytes(&bytes).expect("build AxROM cart")
}

// ---- Cartridge-level: battery flag detection ---------------------------

#[test]
fn mmc1_battery_cart_reports_has_battery() {
    let cart = make_mmc1_battery_cart();
    assert!(cart.has_battery());
}

#[test]
fn mmc1_non_battery_cart_reports_no_battery() {
    let cart = make_mmc1_no_battery_cart();
    assert!(!cart.has_battery());
}

#[test]
fn mmc3_battery_cart_reports_has_battery() {
    let cart = make_mmc3_battery_cart();
    assert!(cart.has_battery());
}

#[test]
fn nrom_cart_reports_no_battery() {
    let cart = make_nrom_cart();
    assert!(!cart.has_battery());
}

// ---- Cartridge-level: battery_sram returns None / Some ----------------

#[test]
fn non_battery_mappers_return_none_for_battery_sram() {
    // NROM, UxROM, CNROM, AxROM have no PRG-RAM and no battery flag.
    assert!(make_nrom_cart().battery_sram().is_none());
    assert!(make_uxrom_cart().battery_sram().is_none());
    assert!(make_cnrom_cart().battery_sram().is_none());
    assert!(make_axrom_cart().battery_sram().is_none());
}

#[test]
fn mmc1_non_battery_returns_none_for_battery_sram() {
    // MMC1 has PRG-RAM, but without the battery flag we should not persist
    // it — `battery_sram` returns None.
    let cart = make_mmc1_no_battery_cart();
    assert!(cart.battery_sram().is_none());
}

#[test]
fn mmc1_battery_returns_some_prg_ram() {
    let cart = make_mmc1_battery_cart();
    let sram = cart.battery_sram().expect("battery cart → Some");
    // MMC1 PRG-RAM is 8 KB.
    assert_eq!(sram.len(), 8 * 1024);
    // Fresh cart → zeroed PRG-RAM.
    assert!(sram.iter().all(|&b| b == 0));
}

#[test]
fn mmc3_battery_returns_some_prg_ram() {
    let cart = make_mmc3_battery_cart();
    let sram = cart.battery_sram().expect("battery cart → Some");
    assert_eq!(sram.len(), 8 * 1024);
    assert!(sram.iter().all(|&b| b == 0));
}

// ---- Cartridge-level: load_battery_sram -------------------------------

#[test]
fn load_battery_sram_populates_mmc1_prg_ram() {
    let mut cart = make_mmc1_battery_cart();
    let payload: Vec<u8> = (0..8192u32).map(|i| (i & 0xFF) as u8).collect();
    cart.load_battery_sram(&payload);
    let sram = cart.battery_sram().expect("Some");
    assert_eq!(sram, payload);
}

#[test]
fn load_battery_sram_populates_mmc3_prg_ram() {
    let mut cart = make_mmc3_battery_cart();
    let payload = vec![0xA5; 8 * 1024];
    cart.load_battery_sram(&payload);
    let sram = cart.battery_sram().expect("Some");
    assert_eq!(sram, payload);
}

#[test]
fn load_battery_sram_ignored_on_non_battery_cart() {
    let mut cart = make_nrom_cart();
    cart.load_battery_sram(&[0xFF; 8192]);
    // NROM has no PRG-RAM — battery_sram stays None.
    assert!(cart.battery_sram().is_none());
}

#[test]
fn load_battery_sram_ignored_on_mmc1_non_battery() {
    let mut cart = make_mmc1_no_battery_cart();
    cart.load_battery_sram(&[0x77; 8192]);
    // Without the battery flag, the mapper refuses to load (and refuses to
    // expose) PRG-RAM contents.
    assert!(cart.battery_sram().is_none());
}

#[test]
fn load_battery_sram_truncates_oversized_payload() {
    let mut cart = make_mmc1_battery_cart();
    // 16 KB payload into 8 KB PRG-RAM — only the first 8 KB should land.
    let payload: Vec<u8> = (0..16384u32).map(|i| (i & 0xFF) as u8).collect();
    cart.load_battery_sram(&payload);
    let sram = cart.battery_sram().expect("Some");
    assert_eq!(sram.len(), 8 * 1024);
    // First 8 KB matches the payload's first 8 KB.
    assert_eq!(sram, payload[..8 * 1024]);
}

#[test]
fn load_battery_sram_partial_payload_leaves_rest_untouched() {
    let mut cart = make_mmc1_battery_cart();
    // Pre-seed PRG-RAM with a marker byte so we can tell what was untouched.
    cart.load_battery_sram(&[0xCD; 8 * 1024]);
    // Now load a 1 KB partial payload of 0xAB.
    cart.load_battery_sram(&[0xAB; 1024]);
    let sram = cart.battery_sram().expect("Some");
    // First 1 KB overwritten.
    assert!(sram[..1024].iter().all(|&b| b == 0xAB));
    // Remaining 7 KB untouched (still 0xCD).
    assert!(sram[1024..].iter().all(|&b| b == 0xCD));
}

// ---- EmulatorState-level ----------------------------------------------

#[test]
fn emulator_has_battery_reflects_cartridge() {
    let emu_battery = EmulatorState::new(make_mmc1_battery_cart());
    assert!(emu_battery.has_battery());

    let emu_plain = EmulatorState::new(make_nrom_cart());
    assert!(!emu_plain.has_battery());
}

#[test]
fn emulator_battery_sram_round_trip() {
    let mut emu = EmulatorState::new(make_mmc1_battery_cart());
    emu.reset();
    let payload = vec![0x5A; 8 * 1024];
    emu.load_battery_sram(&payload);
    let sram = emu.battery_sram().expect("Some");
    assert_eq!(sram, payload);
}

#[test]
fn emulator_battery_sram_none_without_cartridge() {
    // Construct an emulator and remove the cartridge — has_battery should
    // be false and battery_sram should be None.
    let mut emu = EmulatorState::new(make_mmc1_battery_cart());
    let _ = emu.bus_mut().remove_cartridge();
    assert!(!emu.has_battery());
    assert!(emu.battery_sram().is_none());
    // load_battery_sram should be a no-op (no cartridge to load into).
    emu.load_battery_sram(&[0xFF; 8192]);
}

// ---- End-to-end: PRG-RAM visible via the bus --------------------------

#[test]
fn mmc1_prg_ram_writes_visible_via_bus_after_load() {
    // Load SRAM into the cartridge, build the emulator, and verify the
    // PRG-RAM region ($6000-$7FFF) is readable through the bus.
    let mut cart = make_mmc1_battery_cart();
    let mut payload = vec![0u8; 8 * 1024];
    for (i, b) in payload.iter_mut().enumerate() {
        *b = (i.wrapping_mul(7) & 0xFF) as u8;
    }
    cart.load_battery_sram(&payload);

    let mut emu = EmulatorState::new(cart);
    emu.reset();

    // Read a few PRG-RAM addresses through the bus.
    assert_eq!(emu.bus_mut().read(0x6000), payload[0]);
    assert_eq!(emu.bus_mut().read(0x6001), payload[1]);
    assert_eq!(emu.bus_mut().read(0x7FFF), payload[8 * 1024 - 1]);
}

#[test]
fn mmc3_prg_ram_writes_visible_via_bus_after_load() {
    // MMC3 PRG-RAM is gated by $A001 bit 7 — enable it before reading.
    let mut cart = make_mmc3_battery_cart();
    let payload = vec![0x3C; 8 * 1024];
    cart.load_battery_sram(&payload);

    let mut emu = EmulatorState::new(cart);
    emu.reset();
    // Enable PRG-RAM via $A001 write (bit 7 set, bit 6 clear = not write-protected).
    emu.bus_mut().write(0xA001, 0x80);

    assert_eq!(emu.bus_mut().read(0x6000), 0x3C);
    assert_eq!(emu.bus_mut().read(0x7FFF), 0x3C);
}

#[test]
fn mmc3_battery_sram_returns_dump_even_when_prg_ram_disabled() {
    // The battery dump represents the underlying SRAM cell contents, which
    // survive power-off regardless of the $A001 enable bit. `battery_sram()`
    // must therefore return the loaded dump even when `prg_ram_enable` is
    // false (the default reset state). Likewise `load_battery_sram` must
    // restore the SRAM without needing the enable bit set.
    let mut cart = make_mmc3_battery_cart();
    let payload = vec![0x7E; 8 * 1024];
    cart.load_battery_sram(&payload);

    // Build the emulator but do NOT write $A001 — prg_ram_enable stays false.
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    // Bus read at $6000 returns 0 (PRG-RAM disabled) — confirms the gate is
    // closed by default.
    assert_eq!(emu.bus_mut().read(0x6000), 0x00);
    // But the raw dump must still reflect the loaded payload — the battery
    // dump is independent of the enable gate.
    let sram = emu.battery_sram().expect("Some");
    assert_eq!(sram, payload);
}

#[test]
fn mmc3_battery_sram_independent_of_write_protect() {
    // With $A001 bit 6 set (write-protect), CPU writes to $6000-$7FFF are
    // ignored, but the underlying SRAM (and thus battery_sram()) must still
    // be readable and load_battery_sram must still restore it.
    let mut cart = make_mmc3_battery_cart();
    let payload = vec![0x55; 8 * 1024];
    cart.load_battery_sram(&payload);

    let mut emu = EmulatorState::new(cart);
    emu.reset();
    // Enable PRG-RAM but also set write-protect (bit 6).
    emu.bus_mut().write(0xA001, 0xC0);
    // CPU write should be ignored (write-protected) — bus read still returns
    // the loaded 0x55, not 0xFF.
    emu.bus_mut().write(0x6000, 0xFF);
    assert_eq!(emu.bus_mut().read(0x6000), 0x55);
    // battery_sram() returns the underlying SRAM, unchanged by the rejected write.
    let sram = emu.battery_sram().expect("Some");
    assert!(sram.iter().all(|&b| b == 0x55));
}

// ---- End-to-end: file persistence via battery module ------------------

#[test]
fn end_to_end_mmc1_persistence_via_files() {
    let dir = temp_dir();
    let rom_path = dir.join("zelda.nes");
    // Write a minimal ROM image to disk so the sidecar path is derived
    // from a real file path.
    let rom_bytes = make_ines(1, 0, 0x12, 0x00);
    std::fs::write(&rom_path, &rom_bytes).unwrap();

    // Boot #1: load ROM, write PRG-RAM via the bus, save SRAM to disk.
    let mut cart = Cartridge::from_path(&rom_path).expect("load ROM");
    assert!(cart.has_battery());
    cart.load_battery_sram(&[0xEE; 8 * 1024]);
    let mut emu1 = EmulatorState::new(cart);
    emu1.reset();
    // Mutate PRG-RAM through the bus so the save reflects bus-visible state.
    emu1.bus_mut().write(0x6000, 0x11);
    emu1.bus_mut().write(0x6001, 0x22);
    emu1.bus_mut().write(0x7FFF, 0x33);

    let sram = emu1.battery_sram().expect("Some");
    battery::save_for_rom(&rom_path, &sram).expect("save");

    // Boot #2: load ROM fresh, load SRAM from disk, verify PRG-RAM matches.
    let mut cart2 = Cartridge::from_path(&rom_path).expect("load ROM #2");
    assert!(cart2.has_battery());
    let loaded = battery::load_for_rom(&rom_path)
        .expect("load")
        .expect("Some");
    cart2.load_battery_sram(&loaded);
    let mut emu2 = EmulatorState::new(cart2);
    assert_eq!(emu2.bus_mut().read(0x6000), 0x11);
    assert_eq!(emu2.bus_mut().read(0x6001), 0x22);
    assert_eq!(emu2.bus_mut().read(0x7FFF), 0x33);
}

#[test]
fn end_to_end_mmc3_persistence_via_files() {
    let dir = temp_dir();
    let rom_path = dir.join("smb3.nes");
    let rom_bytes = make_ines(2, 0, 0x42, 0x00);
    std::fs::write(&rom_path, &rom_bytes).unwrap();

    // Boot #1: write PRG-RAM, save SRAM.
    let cart = Cartridge::from_path(&rom_path).expect("load ROM");
    assert!(cart.has_battery());
    let mut emu1 = EmulatorState::new(cart);
    emu1.reset();
    // Enable + unprotect PRG-RAM, then write a pattern.
    emu1.bus_mut().write(0xA001, 0x80);
    for addr in 0x6000..=0x600F {
        emu1.bus_mut().write(addr, (addr & 0xFF) as u8);
    }
    let sram = emu1.battery_sram().expect("Some");
    battery::save_for_rom(&rom_path, &sram).expect("save");

    // Boot #2: load SRAM, verify pattern is restored.
    let mut cart2 = Cartridge::from_path(&rom_path).expect("load ROM #2");
    let loaded = battery::load_for_rom(&rom_path)
        .expect("load")
        .expect("Some");
    cart2.load_battery_sram(&loaded);
    let mut emu2 = EmulatorState::new(cart2);
    emu2.reset();
    emu2.bus_mut().write(0xA001, 0x80); // enable PRG-RAM reads
    for addr in 0x6000..=0x600F {
        assert_eq!(
            emu2.bus_mut().read(addr),
            (addr & 0xFF) as u8,
            "addr {addr:04X}"
        );
    }
}

#[test]
fn no_sidecar_first_run_loads_none() {
    // First run: no .nessram file exists → load_for_rom returns Ok(None).
    let dir = temp_dir();
    let rom_path = dir.join("fresh.nes");
    std::fs::write(&rom_path, make_ines(1, 0, 0x12, 0x00)).unwrap();
    let loaded = battery::load_for_rom(&rom_path).expect("load");
    assert!(loaded.is_none());
}

#[test]
fn non_battery_cart_does_not_persist() {
    // Sanity: a non-battery cart's battery_sram() is None, so the main loop
    // would skip the save. Verify the accessor chain returns None end-to-end.
    let emu = EmulatorState::new(make_nrom_cart());
    assert!(!emu.has_battery());
    assert!(emu.battery_sram().is_none());
}

// ---- Determinism: SRAM load does not affect ROM/CHR -------------------

#[test]
fn loading_sram_does_not_corrupt_prg_rom() {
    let mut cart = make_mmc1_battery_cart();
    // Capture a PRG-ROM byte before loading SRAM.
    let prg_before = cart.read_prg(0x8000);
    cart.load_battery_sram(&[0xFF; 8 * 1024]);
    let prg_after = cart.read_prg(0x8000);
    assert_eq!(
        prg_before, prg_after,
        "PRG-ROM must not change on SRAM load"
    );
}

#[test]
fn loading_sram_does_not_affect_chr() {
    let mut cart = make_mmc1_battery_cart();
    let chr_before = cart.read_chr(0x0000);
    cart.load_battery_sram(&[0xAA; 8 * 1024]);
    let chr_after = cart.read_chr(0x0000);
    assert_eq!(chr_before, chr_after, "CHR must not change on SRAM load");
}

// ---- Save-state interaction: SRAM survives save/restore ---------------

#[test]
fn sram_survives_save_state_round_trip() {
    // The save-state system (M20) already serialises the full mapper state
    // including PRG-RAM. Verify that a save-state round-trip preserves
    // battery-loaded SRAM contents, so the two persistence mechanisms do
    // not conflict.
    let mut emu = EmulatorState::new(make_mmc1_battery_cart());
    emu.reset();
    emu.load_battery_sram(&[0x99; 8 * 1024]);

    let saved = emu.save_state().expect("save state");
    let mut restored = EmulatorState::new(make_mmc1_battery_cart());
    restored.reset();
    restored.load_state(&saved).expect("load state");

    let sram = restored.battery_sram().expect("Some");
    assert!(
        sram.iter().all(|&b| b == 0x99),
        "SRAM should survive save-state round-trip"
    );
}

// ---- battery module path derivation -----------------------------------

#[test]
fn sram_path_for_rom_matches_expected_layout() {
    let rom = PathBuf::from("/games/foo.nes");
    assert_eq!(
        battery::sram_path_for_rom(&rom),
        PathBuf::from("/games/foo.nessram")
    );
}
