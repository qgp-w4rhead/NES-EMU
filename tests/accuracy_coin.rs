//! AccuracyCoin test suite — 141 NES accuracy tests from the
//! AprAccuracyCoinUnattended ROM by Chris Siebert (MIT).
//!
//! Source: <https://github.com/erspicu/AprAccuracyCoinUnattended>
//! Original AccuracyCoin: <https://github.com/100thCoin/AccuracyCoin>
//!
//! The unattended ROM boots, runs all 141 tests without controller input,
//! and writes a completion block to CPU RAM that a test harness can poll:
//!
//! | Address       | Meaning                          |
//! |---------------|----------------------------------|
//! | `$07F0`–`$07F2` | magic bytes `DE B0 61`         |
//! | `$07F3`       | tests passed                     |
//! | `$07F4`       | tests total                      |
//! | `$07F5`       | tests skipped                    |
//! | `$0300`–`$04FF` | per-test result bytes          |
//!
//! After writing the block the ROM disables NMI and halts in a tight loop.
//!
//! The full suite completes at frame ~4,870 (~81 s of console time).
//! Accuracy tests are marked `#[ignore]` to avoid breaking CI — run with:
//!
//! ```text
//! cargo test -- --ignored accuracy_coin
//! ```
//!
//! See `README_org.md` in the upstream repo for the full list of tests,
//! error codes, and success codes.

use nes_emu::cartridge::Cartridge;
use nes_emu::emulator::EmulatorState;

// ---------------------------------------------------------------------------
// ROM paths
// ---------------------------------------------------------------------------

/// Full 141-test suite (NROM: 2×16 KB PRG + 8 KB CHR).
const ACCURACY_COIN_ROM: &str = "tests/test_roms/AccuracyCoin.nes";
/// Isolated Open Bus test (CPU Behavior page).
const OPENBUS_ROM: &str = "tests/test_roms/AccuracyCoin_OpenBus.nes";
/// Isolated Internal Data Bus test (test #141 — the final test).
const INTERNAL_DATA_BUS_ROM: &str = "tests/test_roms/AccuracyCoin_InternalDataBus.nes";
/// Isolated $2007 Stress test (PPU Misc page, 341-sample stress).
const STRESS_2007_ROM: &str = "tests/test_roms/AccuracyCoin_2007Stress.nes";
/// Isolated LAE unofficial opcode test ($BB absolute,Y).
const UNOP_LAE_ROM: &str = "tests/test_roms/AccuracyCoin_UnOpLAE.nes";
/// Minimal $2007 hang convergence probe.
const HANG_PROBE_ROM: &str = "tests/test_roms/AccuracyCoin_2007HangProbe.nes";

// ---------------------------------------------------------------------------
// Completion block addresses (in CPU RAM, $0000–$07FF)
// ---------------------------------------------------------------------------

/// Magic bytes at `$07F0`–`$07F2` signalling suite completion.
const MAGIC_ADDR: usize = 0x07F0;
/// Tests-passed tally at `$07F3`.
const PASS_ADDR: usize = 0x07F3;
/// Tests-total tally at `$07F4`.
const TOTAL_ADDR: usize = 0x07F4;
/// Tests-skipped tally at `$07F5`.
const SKIP_ADDR: usize = 0x07F5;
/// Per-test result bytes at `$0300`–`$04FF`.
const PER_TEST_BASE: usize = 0x0300;
const PER_TEST_END: usize = 0x0500;

/// Expected magic bytes.
const MAGIC: [u8; 3] = [0xDE, 0xB0, 0x61];

/// Expected total test count (141 real tests; 5 DRAW tests are skipped
/// by `AutomaticallyRunEveryTestInROM`).
const EXPECTED_TOTAL: u8 = 141;

/// Kill guard for the full suite: 1.5× the measured ~4,870 completion frame.
const MAX_FRAMES_FULL: u32 = 7_305;
/// Kill guard for isolated ROMs (they complete in far fewer frames).
const MAX_FRAMES_ISOLATED: u32 = 3_000;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parsed completion block written by the ROM to CPU RAM.
struct CompletionBlock {
    /// Frame number at which the magic bytes appeared.
    frame: u32,
    /// Number of tests that passed (`$07F3`).
    passed: u8,
    /// Total tests run (`$07F4`).
    total: u8,
    /// Tests skipped (`$07F5`).
    skipped: u8,
    /// Raw per-test result bytes (`$0300`–`$04FF`).
    per_test: Vec<u8>,
}

impl CompletionBlock {
    /// Returns indices and values of per-test bytes that look like failures
    /// (non-zero and non-one, assuming 0 = not-run and 1 = pass).
    fn failures(&self) -> Vec<(usize, u8)> {
        self.per_test
            .iter()
            .enumerate()
            .filter(|(_, &v)| v != 0 && v != 1)
            .map(|(i, &v)| (i, v))
            .collect()
    }
}

impl std::fmt::Display for CompletionBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}/{} passed, {} skipped (frame {})",
            self.passed, self.total, self.skipped, self.frame,
        )
    }
}

/// Load a ROM, reset, and run frames until the completion-block magic
/// bytes appear at `$07F0`. Panics with diagnostic info if the ROM does
/// not complete within `max_frames`.
fn run_until_completion(rom_path: &str, max_frames: u32) -> CompletionBlock {
    let cart = Cartridge::from_path(rom_path).unwrap_or_else(|e| {
        panic!(
            "failed to load {rom_path}: {e}\n\
             hint: ROMs can be downloaded from\n\
             https://github.com/erspicu/AprAccuracyCoinUnattended"
        )
    });
    let mut emu = EmulatorState::new(cart);
    emu.reset();

    for frame in 0..max_frames {
        emu.step_frame();

        let ram = emu.bus().ram();
        if ram[MAGIC_ADDR] == MAGIC[0]
            && ram[MAGIC_ADDR + 1] == MAGIC[1]
            && ram[MAGIC_ADDR + 2] == MAGIC[2]
        {
            return CompletionBlock {
                frame,
                passed: ram[PASS_ADDR],
                total: ram[TOTAL_ADDR],
                skipped: ram[SKIP_ADDR],
                per_test: ram[PER_TEST_BASE..PER_TEST_END].to_vec(),
            };
        }
    }

    // Did not complete — dump state for debugging.
    let ram = emu.bus().ram();
    panic!(
        "ROM did not complete within {max_frames} frames.\n\
         CPU PC: ${:04X}, SP: ${:02X}\n\
         PPU scanline: {}, cycle: {}\n\
         RAM[$07F0-$07F5]: {:02X?}\n\
         RAM[$0300-$030F]: {:02X?}",
        emu.cpu().pc,
        emu.cpu().sp,
        emu.bus().ppu().scanline(),
        emu.bus().ppu().cycle(),
        &ram[MAGIC_ADDR..=SKIP_ADDR],
        &ram[PER_TEST_BASE..PER_TEST_BASE + 16],
    );
}

/// Assert that a completion block reports all tests passed.
fn assert_all_passed(result: &CompletionBlock, rom_name: &str) {
    assert!(
        result.total > 0,
        "{rom_name}: total is 0 — ROM did not run any tests ({result})",
    );
    if result.passed != result.total {
        let failures = result.failures();
        panic!(
            "{rom_name}: {result} — expected {} passed.\n\
             Failing test indices (offset from $0300): {:?}",
            result.total, failures,
        );
    }
}

// ---------------------------------------------------------------------------
// Tests: ROM loading (fast, always run)
// ---------------------------------------------------------------------------

/// Verify the full-suite ROM loads and has the expected iNES header
/// (NROM, 32 KB PRG, 8 KB CHR, mapper 0).
#[test]
fn accuracy_coin_loads() {
    let cart = Cartridge::from_path(ACCURACY_COIN_ROM).expect("load AccuracyCoin.nes");
    assert_eq!(cart.header.mapper_number, 0, "should be NROM (mapper 0)");
    assert_eq!(
        cart.header.prg_rom_banks, 2,
        "should have 2 PRG banks (32 KB)"
    );
    assert_eq!(
        cart.header.chr_rom_banks, 1,
        "should have 1 CHR bank (8 KB)"
    );
}

/// Verify all isolated ROMs also load correctly.
#[test]
fn accuracy_coin_isolated_roms_load() {
    for rom in [
        OPENBUS_ROM,
        INTERNAL_DATA_BUS_ROM,
        STRESS_2007_ROM,
        UNOP_LAE_ROM,
        HANG_PROBE_ROM,
    ] {
        let cart = Cartridge::from_path(rom).unwrap_or_else(|e| panic!("load {rom}: {e}"));
        assert_eq!(
            cart.header.mapper_number, 0,
            "{rom}: should be NROM (mapper 0)"
        );
    }
}

/// Verify the emulator can boot the full-suite ROM and run several frames
/// without crashing.
#[test]
fn accuracy_coin_boots_and_runs() {
    let cart = Cartridge::from_path(ACCURACY_COIN_ROM).expect("load AccuracyCoin.nes");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    for _ in 0..10 {
        emu.step_frame();
    }
    assert_eq!(emu.bus().ppu().scanline(), 0, "should be at frame boundary");
}

// ---------------------------------------------------------------------------
// Tests: full 141-test suite (slow, ignored)
// ---------------------------------------------------------------------------

/// Full 141-test AccuracyCoin suite.
///
/// The ROM boots, runs all 141 tests unattended, and writes the completion
/// block to CPU RAM. This test verifies that all tests pass.
///
/// Marked `#[ignore]` because it takes ~4,870 frames (~30–60 s in debug
/// mode). Run with:
///
/// ```text
/// cargo test -- --ignored accuracy_coin_full_suite
/// ```
#[test]
#[ignore]
fn accuracy_coin_full_suite() {
    let result = run_until_completion(ACCURACY_COIN_ROM, MAX_FRAMES_FULL);

    assert_eq!(
        result.total, EXPECTED_TOTAL,
        "expected {EXPECTED_TOTAL} tests, got {} ({result})",
        result.total,
    );
    assert_all_passed(&result, "AccuracyCoin");
    assert_eq!(
        result.skipped, 0,
        "no tests should be skipped on NTSC ({result})",
    );

    println!("AccuracyCoin full suite: {result}");
}

// ---------------------------------------------------------------------------
// Tests: isolated ROMs (faster, but still ignored to avoid breaking CI
// if the emulator has accuracy issues)
// ---------------------------------------------------------------------------

/// Isolated Open Bus test (CPU Behavior page — 9 sub-tests covering open
/// bus reads, data bus persistence, and controller open bus bits).
#[test]
#[ignore]
fn accuracy_coin_openbus() {
    let result = run_until_completion(OPENBUS_ROM, MAX_FRAMES_ISOLATED);
    assert_all_passed(&result, "OpenBus");
    println!("OpenBus: {result}");
}

/// Isolated Internal Data Bus test (test #141 — the final test in the
/// suite, covering open bus and DMC DMA / $4015 data-bus isolation).
#[test]
#[ignore]
fn accuracy_coin_internal_data_bus() {
    let result = run_until_completion(INTERNAL_DATA_BUS_ROM, MAX_FRAMES_ISOLATED);
    assert_all_passed(&result, "InternalDataBus");
    println!("InternalDataBus: {result}");
}

/// Isolated $2007 Stress test (PPU Misc page — 341-sample analog ALE+RD
/// feedback oscillation test).
///
/// This test is sensitive to CPU/PPU entry timing.
#[test]
#[ignore]
fn accuracy_coin_2007_stress() {
    let result = run_until_completion(STRESS_2007_ROM, MAX_FRAMES_ISOLATED);
    assert_all_passed(&result, "2007Stress");
    println!("2007Stress: {result}");
}

/// Isolated LAE unofficial opcode test ($BB absolute,Y — the analog
/// three-way merge).
#[test]
#[ignore]
fn accuracy_coin_unop_lae() {
    let result = run_until_completion(UNOP_LAE_ROM, MAX_FRAMES_ISOLATED);
    assert_all_passed(&result, "UnOpLAE");
    println!("UnOpLAE: {result}");
}

/// Minimal $2007 hang convergence probe — jumps straight to the dot-$F6
/// phase that formerly hung.
#[test]
#[ignore]
fn accuracy_coin_2007_hang_probe() {
    let result = run_until_completion(HANG_PROBE_ROM, MAX_FRAMES_ISOLATED);
    assert_all_passed(&result, "2007HangProbe");
    println!("2007HangProbe: {result}");
}
