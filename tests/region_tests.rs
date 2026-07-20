//! PAL / Dendy region integration tests (M32).
//!
//! These tests exercise the region-aware paths across the PPU, APU,
//! emulator, cartridge, config, and save-state modules:
//!
//! - iNES header byte 9 is parsed into `InesHeader::tv_system` and
//!   `region_hint()` returns the expected `Region`.
//! - `EmulatorState::new_with_region` propagates the region to the PPU
//!   (scanline count + prerender scanline) and APU (frame-counter
//!   thresholds).
//! - A PAL emulator runs a full 312-scanline frame and produces one NMI.
//! - A Dendy emulator runs a 312-scanline frame with the NTSC palette.
//! - `EmulatorState::set_region` mid-run propagates to PPU + APU.
//! - `Config::resolve_region` honours an explicit override over the
//!   cartridge hint, and falls back to the hint for `"auto"`.
//! - The PAL palette (`PAL_PALETTE`) differs from `NES_PALETTE` at
//!   colour `$0D` (PAL `$0D` is not pure black).
//! - `nes_color_to_argb_for` selects the right palette per region.
//! - A save state round-trips the region through the PPU's serialised
//!   `region` field (the EmulatorState's region is restored from the
//!   PPU on load).

use nes_emu::apu::Apu;
use nes_emu::cartridge::Cartridge;
use nes_emu::config::Config;
use nes_emu::emulator::EmulatorState;
use nes_emu::ppu::render::{nes_color_to_argb, nes_color_to_argb_for, NES_PALETTE, PAL_PALETTE};
use nes_emu::ppu::Ppu;
use nes_emu::region::Region;
use nes_emu::save_state::SAVE_STATE_VERSION;

/// Build a minimal NROM-128 cartridge (16 KB PRG filled with NOPs,
/// 8 KB CHR-RAM) with an optional TV-system byte at header offset 9.
fn make_cart(tv_system_byte: u8) -> Cartridge {
    let mut bytes = vec![b'N', b'E', b'S', 0x1A, 1, 0, 0, 0];
    // bytes 8..15 are reserved; byte 9 is the TV system hint.
    bytes.extend_from_slice(&[0u8; 8]);
    bytes[9] = tv_system_byte;
    bytes.resize(16 + 16 * 1024, 0xEA);
    let reset_off = 16 + 0x3FFC;
    bytes[reset_off] = 0x00;
    bytes[reset_off + 1] = 0xC0;
    Cartridge::from_bytes(&bytes).expect("build NOP cart")
}

// ---------------------------------------------------------------------------
// iNES header region hint
// ---------------------------------------------------------------------------

#[test]
fn ines_header_tv_system_zero_yields_no_hint() {
    let cart = make_cart(0);
    assert_eq!(cart.header.tv_system, 0);
    assert_eq!(cart.header.region_hint(), None);
}

#[test]
fn ines_header_tv_system_one_yields_pal() {
    let cart = make_cart(1);
    assert_eq!(cart.header.tv_system, 1);
    assert_eq!(cart.header.region_hint(), Some(Region::Pal));
}

#[test]
fn ines_header_tv_system_two_yields_dendy() {
    let cart = make_cart(2);
    assert_eq!(cart.header.tv_system, 2);
    assert_eq!(cart.header.region_hint(), Some(Region::Dendy));
}

#[test]
fn ines_header_tv_system_three_yields_no_hint() {
    // 3 = dual-region; we fall back to NTSC.
    let cart = make_cart(3);
    assert_eq!(cart.header.tv_system, 3);
    assert_eq!(cart.header.region_hint(), None);
}

#[test]
fn ines_header_tv_system_masks_to_low_two_bits() {
    // High bits of byte 9 are reserved and should be ignored.
    let cart = make_cart(0x05);
    assert_eq!(cart.header.tv_system, 1);
    assert_eq!(cart.header.region_hint(), Some(Region::Pal));
}

// ---------------------------------------------------------------------------
// EmulatorState region propagation
// ---------------------------------------------------------------------------

#[test]
fn new_with_region_propagates_to_ppu_and_apu() {
    let cart = make_cart(0);
    let emu = EmulatorState::new_with_region(cart, Region::Pal);
    assert_eq!(emu.region(), Region::Pal);
    assert_eq!(emu.bus().ppu().region(), Region::Pal);
    assert_eq!(emu.bus().apu().region(), Region::Pal);
}

#[test]
fn new_defaults_to_ntsc() {
    let cart = make_cart(0);
    let emu = EmulatorState::new(cart);
    assert_eq!(emu.region(), Region::Ntsc);
    assert_eq!(emu.bus().ppu().region(), Region::Ntsc);
}

#[test]
fn set_region_propagates_to_ppu_and_apu() {
    let cart = make_cart(0);
    let mut emu = EmulatorState::new(cart);
    emu.set_region(Region::Dendy);
    assert_eq!(emu.region(), Region::Dendy);
    assert_eq!(emu.bus().ppu().region(), Region::Dendy);
    assert_eq!(emu.bus().apu().region(), Region::Dendy);
}

#[test]
fn set_region_clamps_scanline_into_new_range() {
    // Start in PAL, advance the PPU past scanline 261, then switch to
    // NTSC (262 scanlines, prerender 261). The scanline should be
    // clamped so the stepper doesn't get stuck past the new prerender.
    let cart = make_cart(0);
    let mut emu = EmulatorState::new_with_region(cart, Region::Pal);
    // Advance ~280 scanlines worth of PPU cycles (well past 261).
    emu.bus_mut().step_ppu(280 * 341);
    let sl_before = emu.bus().ppu().scanline();
    assert!(sl_before > 261);
    emu.set_region(Region::Ntsc);
    let sl_after = emu.bus().ppu().scanline();
    assert!(
        sl_after < 262,
        "scanline {sl_after} should be < 262 after clamp"
    );
}

// ---------------------------------------------------------------------------
// PPU region-aware timing
// ---------------------------------------------------------------------------

#[test]
fn ppu_pal_frame_is_312_scanlines() {
    let mut ppu = Ppu::new();
    ppu.set_region(Region::Pal);
    // Enable NMI via PPUCTRL bit 7 so VBlank actually requests it.
    ppu.write_register(0x2000, 0x80);
    let mut nmis = 0;
    // 312 * 341 = 106392 cycles per PAL frame.
    for _ in 0..(312 * 341) {
        if ppu.step() {
            nmis += 1;
        }
    }
    assert_eq!(nmis, 1, "expected exactly one NMI per PAL frame");
    // After one full frame the PPU should be back at (0, 0).
    assert_eq!(ppu.scanline(), 0);
    assert_eq!(ppu.cycle(), 0);
}

#[test]
fn ppu_dendy_frame_is_312_scanlines() {
    let mut ppu = Ppu::new();
    ppu.set_region(Region::Dendy);
    ppu.write_register(0x2000, 0x80);
    let mut nmis = 0;
    for _ in 0..(312 * 341) {
        if ppu.step() {
            nmis += 1;
        }
    }
    assert_eq!(nmis, 1);
}

#[test]
fn ppu_pal_prerender_is_scanline_311() {
    let mut ppu = Ppu::new();
    ppu.set_region(Region::Pal);
    // Step to the start of scanline 311 (prerender) and verify VBlank
    // is cleared there.
    ppu.set_vblank(true);
    // 311 * 341 + 1 cycles to reach cycle 1 of scanline 311.
    for _ in 0..(311 * 341 + 1) {
        ppu.step();
    }
    assert_eq!(ppu.scanline(), 311);
    assert_eq!(ppu.cycle(), 1);
    assert!(
        !ppu.in_vblank(),
        "VBlank should be cleared at PAL prerender"
    );
}

#[test]
fn ppu_ntsc_prerender_is_scanline_261() {
    let mut ppu = Ppu::new();
    ppu.set_region(Region::Ntsc);
    ppu.set_vblank(true);
    for _ in 0..(261 * 341 + 1) {
        ppu.step();
    }
    assert_eq!(ppu.scanline(), 261);
    assert!(!ppu.in_vblank());
}

// ---------------------------------------------------------------------------
// APU region-aware frame counter
// ---------------------------------------------------------------------------

#[test]
fn apu_pal_4step_irq_at_33255() {
    let mut apu = Apu::new();
    apu.set_region(Region::Pal);
    // 4-step mode, IRQ not inhibited. The $4017 write imposes a 4-cycle
    // reset delay, so we step a few extra cycles past the threshold.
    apu.write_frame_counter(0x00);
    apu.step(33260, |_| 0);
    assert!(
        apu.irq_pending(),
        "PAL APU should raise IRQ near cycle 33255"
    );
}

#[test]
fn apu_pal_4step_no_irq_before_33255() {
    let mut apu = Apu::new();
    apu.set_region(Region::Pal);
    apu.write_frame_counter(0x00);
    // Step just short of the PAL threshold (accounting for the 4-cycle
    // reset delay, the effective threshold is 33255 + 4 = 33259).
    apu.step(33250, |_| 0);
    assert!(
        !apu.irq_pending(),
        "PAL APU should not raise IRQ before 33255"
    );
}

#[test]
fn apu_ntsc_4step_irq_at_29828() {
    let mut apu = Apu::new();
    apu.set_region(Region::Ntsc);
    apu.write_frame_counter(0x00);
    apu.step(29833, |_| 0);
    assert!(apu.irq_pending());
}

#[test]
fn apu_dendy_uses_ntsc_thresholds() {
    let mut apu = Apu::new();
    apu.set_region(Region::Dendy);
    apu.write_frame_counter(0x00);
    apu.step(29833, |_| 0);
    assert!(apu.irq_pending(), "Dendy APU should use NTSC IRQ threshold");
}

#[test]
fn apu_pal_5step_no_irq() {
    let mut apu = Apu::new();
    apu.set_region(Region::Pal);
    // Write $4017 with bit 7 set → 5-step mode.
    apu.write_frame_counter(0x80);
    // Step past the PAL 5-step period (41570). No IRQ should fire.
    apu.step(41580, |_| 0);
    assert!(!apu.irq_pending(), "5-step mode never raises IRQ");
}

#[test]
fn apu_pal_reset_wraps_and_fires_again() {
    let mut apu = Apu::new();
    apu.set_region(Region::Pal);
    apu.write_frame_counter(0x00);
    // Step past the PAL 4-step reset point (33257). First IRQ fires at
    // 33255; the counter wraps at 33257. The $4017 write imposes a
    // 4-cycle reset delay, so the effective frame_cycle after step
    // (33260) is 33256 (33260 - 4 reset delay).
    apu.step(33260, |_| 0);
    assert!(apu.irq_pending());
    // Clear the IRQ by reading $4015 (read_status).
    apu.read_status();
    assert!(!apu.irq_pending());
    // Step one more cycle to push past the reset point (33257). The
    // counter wraps to 0.
    apu.step(2, |_| 0);
    // Now step in small increments until the next IRQ threshold (33255).
    // Stepping 33255 more cycles crosses the threshold in the new period.
    apu.step(33255, |_| 0);
    assert!(
        apu.irq_pending(),
        "PAL frame counter should wrap at 33257 and re-fire"
    );
}

// ---------------------------------------------------------------------------
// Palette selection
// ---------------------------------------------------------------------------

#[test]
fn pal_palette_differs_from_ntsc_at_color_0e() {
    // NTSC $0E is pure black; PAL $0E is a very dark grey/blue (the
    // 2C07 colour generator produces a slightly lifted black level).
    assert_eq!(NES_PALETTE[0x0E], [0x00, 0x00, 0x00]);
    assert_ne!(PAL_PALETTE[0x0E], [0x00, 0x00, 0x00]);
}

#[test]
fn pal_palette_has_64_entries() {
    assert_eq!(PAL_PALETTE.len(), 64);
}

#[test]
fn nes_color_to_argb_for_ntsc_matches_legacy() {
    for i in 0u8..=0x3F {
        assert_eq!(nes_color_to_argb_for(i, Region::Ntsc), nes_color_to_argb(i),);
    }
}

#[test]
fn nes_color_to_argb_for_dendy_uses_ntsc_palette() {
    for i in 0u8..=0x3F {
        assert_eq!(
            nes_color_to_argb_for(i, Region::Dendy),
            nes_color_to_argb_for(i, Region::Ntsc),
        );
    }
}

#[test]
fn nes_color_to_argb_for_pal_uses_pal_palette() {
    // $0E differs between palettes, so PAL should pick the PAL value.
    assert_ne!(
        nes_color_to_argb_for(0x0E, Region::Pal),
        nes_color_to_argb_for(0x0E, Region::Ntsc),
    );
}

#[test]
fn nes_color_to_argb_for_masks_high_bits() {
    // Indices >= 0x40 should be masked to 6 bits.
    assert_eq!(
        nes_color_to_argb_for(0x40, Region::Ntsc),
        nes_color_to_argb_for(0x00, Region::Ntsc),
    );
    assert_eq!(
        nes_color_to_argb_for(0x80, Region::Pal),
        nes_color_to_argb_for(0x00, Region::Pal),
    );
}

// ---------------------------------------------------------------------------
// Config region resolution
// ---------------------------------------------------------------------------

#[test]
fn config_resolve_region_auto_uses_hint() {
    let cfg = Config::default(); // region = "auto"
    assert_eq!(cfg.resolve_region(Some(Region::Pal)), Region::Pal);
    assert_eq!(cfg.resolve_region(Some(Region::Dendy)), Region::Dendy);
}

#[test]
fn config_resolve_region_auto_falls_back_to_ntsc() {
    let cfg = Config::default();
    assert_eq!(cfg.resolve_region(None), Region::Ntsc);
}

#[test]
fn config_resolve_region_override_wins_over_hint() {
    let cfg = Config {
        region: "pal".to_string(),
        ..Config::default()
    };
    // Even with an NTSC hint, the override wins.
    assert_eq!(cfg.resolve_region(Some(Region::Ntsc)), Region::Pal);
}

#[test]
fn config_resolve_region_override_ntsc() {
    let cfg = Config {
        region: "ntsc".to_string(),
        ..Config::default()
    };
    assert_eq!(cfg.resolve_region(Some(Region::Pal)), Region::Ntsc);
}

#[test]
fn config_resolve_region_override_dendy() {
    let cfg = Config {
        region: "dendy".to_string(),
        ..Config::default()
    };
    assert_eq!(cfg.resolve_region(None), Region::Dendy);
}

#[test]
fn config_resolve_region_invalid_falls_back_to_hint() {
    let cfg = Config {
        region: "xyz".to_string(),
        ..Config::default()
    };
    assert_eq!(cfg.resolve_region(Some(Region::Pal)), Region::Pal);
    assert_eq!(cfg.resolve_region(None), Region::Ntsc);
}

#[test]
fn config_resolve_region_case_insensitive() {
    let cfg = Config {
        region: "PAL".to_string(),
        ..Config::default()
    };
    assert_eq!(cfg.resolve_region(None), Region::Pal);
}

#[test]
fn config_default_region_is_auto() {
    let cfg = Config::default();
    assert_eq!(cfg.region, "auto");
}

// ---------------------------------------------------------------------------
// Save state region round-trip
// ---------------------------------------------------------------------------

#[test]
fn save_state_version_is_six() {
    // M35 bumped the version: new mapper variants (Vrc7, Namco163) and
    // Fme7 gained a ym2149 field (Sunsoft 5B audio).
    assert_eq!(SAVE_STATE_VERSION, 6);
}

#[test]
fn save_state_round_trips_pal_region() {
    let cart = make_cart(1);
    let mut emu = EmulatorState::new_with_region(cart, Region::Pal);
    emu.reset();
    // Run a few frames so the PPU/APU state is non-trivial.
    for _ in 0..3 {
        emu.step_frame();
    }
    let saved = emu.save_state().expect("serialize");
    // Load into a fresh NTSC emulator — the region should be restored
    // from the PPU's serialised `region` field.
    let cart2 = make_cart(0);
    let mut emu2 = EmulatorState::new_with_region(cart2, Region::Ntsc);
    emu2.reset();
    emu2.load_state(&saved).expect("deserialize + apply");
    assert_eq!(
        emu2.region(),
        Region::Pal,
        "region should be restored from save state"
    );
    assert_eq!(emu2.bus().ppu().region(), Region::Pal);
    assert_eq!(emu2.bus().apu().region(), Region::Pal);
}

#[test]
fn save_state_round_trips_dendy_region() {
    let cart = make_cart(2);
    let mut emu = EmulatorState::new_with_region(cart, Region::Dendy);
    emu.reset();
    emu.step_frame();
    let saved = emu.save_state().expect("serialize");
    let cart2 = make_cart(0);
    let mut emu2 = EmulatorState::new_with_region(cart2, Region::Ntsc);
    emu2.reset();
    emu2.load_state(&saved).expect("deserialize + apply");
    assert_eq!(emu2.region(), Region::Dendy);
}

// ---------------------------------------------------------------------------
// Emulator end-to-end: PAL frame produces ~882 audio samples
// ---------------------------------------------------------------------------

#[test]
fn pal_emulator_runs_full_frame_without_crash() {
    let cart = make_cart(1);
    let mut emu = EmulatorState::new_with_region(cart, Region::Pal);
    emu.reset();
    emu.step_frame();
    // A PAL frame at 44.1 kHz / 50 Hz ≈ 882 samples.
    let samples = emu.take_audio_samples();
    assert!(
        samples.len() >= 800 && samples.len() <= 1000,
        "expected ~882 samples, got {}",
        samples.len()
    );
}

#[test]
fn dendy_emulator_runs_full_frame_without_crash() {
    let cart = make_cart(2);
    let mut emu = EmulatorState::new_with_region(cart, Region::Dendy);
    emu.reset();
    emu.step_frame();
    let samples = emu.take_audio_samples();
    assert!(
        !samples.is_empty(),
        "Dendy frame should produce audio samples"
    );
}

#[test]
fn ntsc_emulator_still_runs_262_scanline_frame() {
    // Regression: NTSC should still produce a 262-scanline frame after
    // the M32 region-aware changes.
    let cart = make_cart(0);
    let mut emu = EmulatorState::new_with_region(cart, Region::Ntsc);
    emu.reset();
    emu.step_frame();
    let samples = emu.take_audio_samples();
    // NTSC: ~735 samples/frame (44100/60).
    assert!(
        samples.len() >= 700 && samples.len() <= 800,
        "expected ~735 samples, got {}",
        samples.len()
    );
}

#[test]
fn pal_frame_yields_more_samples_than_ntsc() {
    // PAL CPU clock is slower → more CPU cycles per sample, but the PAL
    // video frame is longer in CPU cycles (312*341*3 = 319176 vs
    // 262*341*3 = 268086). The longer frame dominates, so PAL yields
    // more audio samples per video frame than NTSC (~882 vs ~735).
    let cart = make_cart(0);
    let mut emu_ntsc = EmulatorState::new_with_region(cart, Region::Ntsc);
    emu_ntsc.reset();
    emu_ntsc.step_frame();
    let n_ntsc = emu_ntsc.take_audio_samples().len();

    let cart2 = make_cart(0);
    let mut emu_pal = EmulatorState::new_with_region(cart2, Region::Pal);
    emu_pal.reset();
    emu_pal.step_frame();
    let n_pal = emu_pal.take_audio_samples().len();

    assert!(
        n_pal > n_ntsc,
        "PAL frame should yield more samples ({n_pal}) than NTSC ({n_ntsc})"
    );
}
