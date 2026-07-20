//! NES region / TV system — NTSC, PAL, and Dendy (Famiclone) timing.
//!
//! The NES was sold in different regions with different video standards:
//!
//! | Region | Frame  | Scanlines | CPU clock  | PPU clock  | Palette |
//! |--------|--------|-----------|------------|------------|---------|
//! | NTSC   | 60 Hz  | 262       | 1.789 MHz  | 5.369 MHz  | 2C02    |
//! | PAL    | 50 Hz  | 312       | 1.663 MHz  | 5.320 MHz  | 2C07    |
//! | Dendy  | 50 Hz  | 312       | 1.790 MHz  | 5.320 MHz* | 2C02    |
//!
//! \* Dendy uses PAL frame length (312 scanlines, 50 Hz) but NTSC-style
//! PPU behaviour (NTSC palette, short VBlank) and an NTSC-speed CPU. It is
//! a Russian Famiclone that hybridises the two so PAL games run at the
//! correct speed while keeping NTSC-style video timing for game code.
//!
//! # APU frame counter
//!
//! The APU frame counter runs at the CPU clock rate and its period tracks
//! the video frame rate (~1/60 s NTSC, ~1/50 s PAL). The quarter/half-frame
//! thresholds in CPU cycles therefore differ between regions. Dendy uses
//! the NTSC APU frame counter thresholds (its CPU runs at NTSC speed and
//! its APU is NTSC-style), which means the frame counter cycles slightly
//! out of sync with the 50 Hz video frame — a known Dendy quirk that some
//! games tolerate and others exhibit as occasional audio glitches.
//!
//! See: https://www.nesdev.org/wiki/Cycle_reference
//! See: https://www.nesdev.org/wiki/APU_Frame_Counter
//! See: https://www.nesdev.org/wiki/PPU_rendering#Timing

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// NES region / TV system.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Region {
    /// NTSC — North America / Japan. 262 scanlines, ~60 Hz, 2C02 palette.
    Ntsc,
    /// PAL — Europe / Australia. 312 scanlines, ~50 Hz, 2C07 palette.
    Pal,
    /// Dendy — Russian Famiclone. 312 scanlines, ~50 Hz, NTSC palette +
    /// NTSC CPU speed. PAL timing with NTSC PPU behaviour.
    Dendy,
}

impl Default for Region {
    /// The NES was primarily an NTSC machine; default to NTSC when no
    /// region is specified (matches the historical behaviour of this
    /// emulator before M32 added region support).
    fn default() -> Self {
        Region::Ntsc
    }
}

impl Region {
    /// Total scanlines per frame.
    /// NTSC = 262, PAL/Dendy = 312.
    pub const fn scanlines_per_frame(self) -> u16 {
        match self {
            Region::Ntsc => 262,
            Region::Pal | Region::Dendy => 312,
        }
    }

    /// Scanline on which VBlank begins (NMI asserted at cycle 1).
    /// All regions use scanline 241.
    pub const fn scanline_vblank_start(self) -> u16 {
        241
    }

    /// Prerender scanline (VBlank cleared + status flags cleared at cycle 1).
    /// NTSC = 261, PAL/Dendy = 311.
    pub const fn scanline_prerender(self) -> u16 {
        match self {
            Region::Ntsc => 261,
            Region::Pal | Region::Dendy => 311,
        }
    }

    /// Number of PPU cycles per scanline. All regions use 341.
    pub const fn cycles_per_scanline(self) -> u16 {
        341
    }

    /// CPU clock rate in Hz.
    /// NTSC/Dendy = 1,789,773; PAL = 1,662,607.
    pub const fn cpu_clock_hz(self) -> f32 {
        match self {
            Region::Ntsc | Region::Dendy => 1_789_773.0,
            Region::Pal => 1_662_607.0,
        }
    }

    /// PPU clock rate in Hz (3× CPU clock).
    pub const fn ppu_clock_hz(self) -> f32 {
        match self {
            Region::Ntsc | Region::Dendy => 5_369_318.0,
            Region::Pal => 5_319_870.0,
        }
    }

    /// APU clock rate in Hz (CPU clock / 2).
    pub const fn apu_clock_hz(self) -> f32 {
        match self {
            Region::Ntsc | Region::Dendy => 894_886.5,
            Region::Pal => 831_303.5,
        }
    }

    /// Frame rate in Hz.
    /// NTSC ≈ 60.0988, PAL/Dendy ≈ 50.0070.
    pub fn frame_rate_hz(self) -> f32 {
        // frame_rate = ppu_clock / (scanlines * cycles_per_scanline)
        self.ppu_clock_hz()
            / (self.scanlines_per_frame() as f32 * self.cycles_per_scanline() as f32)
    }

    /// Whether this region uses the PAL palette (2C07).
    /// Dendy uses the NTSC palette despite PAL timing.
    pub const fn is_pal_palette(self) -> bool {
        matches!(self, Region::Pal)
    }

    /// CPU cycles per audio sample at 44.1 kHz.
    /// NTSC/Dendy ≈ 40.585; PAL ≈ 37.7.
    pub fn cpu_cycles_per_sample(self) -> f32 {
        self.cpu_clock_hz() / crate::audio::SAMPLE_RATE as f32
    }

    /// APU frame-counter 4-step mode thresholds in CPU cycles.
    /// Each tuple is `(cycle_threshold, quarter_frame, half_frame)`.
    ///
    /// NTSC: 7457, 14913, 22371, 29828 (IRQ at 29828).
    /// PAL:  8314, 16627, 24941, 33255 (IRQ at 33255).
    /// Dendy: NTSC thresholds (NTSC-style APU).
    ///
    /// See: https://www.nesdev.org/wiki/APU_Frame_Counter
    pub const fn apu_4step_thresholds(self) -> [(u32, bool, bool); 4] {
        match self {
            Region::Ntsc | Region::Dendy => [
                (7457, true, false),
                (14913, true, true),
                (22371, true, false),
                (29828, true, true),
            ],
            Region::Pal => [
                (8314, true, false),
                (16627, true, true),
                (24941, true, false),
                (33255, true, true),
            ],
        }
    }

    /// APU frame-counter 5-step mode thresholds in CPU cycles.
    /// No IRQ is raised in 5-step mode.
    ///
    /// NTSC: 7457, 14913, 22371, 37281.
    /// PAL:  8314, 16627, 24941, 41568.
    /// Dendy: NTSC thresholds.
    pub const fn apu_5step_thresholds(self) -> [(u32, bool, bool); 4] {
        match self {
            Region::Ntsc | Region::Dendy => [
                (7457, true, false),
                (14913, true, true),
                (22371, true, false),
                (37281, true, true),
            ],
            Region::Pal => [
                (8314, true, false),
                (16627, true, true),
                (24941, true, false),
                (41568, true, true),
            ],
        }
    }

    /// APU frame-counter reset point in CPU cycles (end of period).
    /// NTSC 4-step = 29830, PAL 4-step = 33257.
    /// NTSC 5-step = 37282, PAL 5-step = 41570.
    pub const fn apu_reset_at(self, mode_5step: bool) -> u32 {
        match self {
            Region::Ntsc | Region::Dendy => {
                if mode_5step {
                    37282
                } else {
                    29830
                }
            }
            Region::Pal => {
                if mode_5step {
                    41570
                } else {
                    33257
                }
            }
        }
    }

    /// APU 4-step mode IRQ threshold (the 4th step where IRQ is raised
    /// when not inhibited). Used by `step_frame_counter` to detect the
    /// IRQ edge.
    pub const fn apu_4step_irq_threshold(self) -> u32 {
        match self {
            Region::Ntsc | Region::Dendy => 29828,
            Region::Pal => 33255,
        }
    }

    /// Parse a region name string as used in `config.toml`.
    /// Accepts `"auto"`, `"ntsc"`, `"pal"`, `"dendy"` (case-insensitive).
    /// Returns `Ok(Some(region))` for a concrete region, `Ok(None)` for
    /// `"auto"`, or `Err` for an unknown name.
    pub fn from_config_str(s: &str) -> Result<Option<Region>, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Ok(None),
            "ntsc" => Ok(Some(Region::Ntsc)),
            "pal" => Ok(Some(Region::Pal)),
            "dendy" => Ok(Some(Region::Dendy)),
            other => Err(format!(
                "unknown region '{other}' (expected: auto, ntsc, pal, dendy)"
            )),
        }
    }

    /// Short name suitable for OSD display ("NTSC" / "PAL" / "DENDY").
    pub const fn short_name(self) -> &'static str {
        match self {
            Region::Ntsc => "NTSC",
            Region::Pal => "PAL",
            Region::Dendy => "DENDY",
        }
    }

    /// Cycle to the next region in the order Ntsc → Pal → Dendy → Ntsc.
    /// Used by the F11 runtime region-cycle hotkey.
    pub const fn cycle(self) -> Region {
        match self {
            Region::Ntsc => Region::Pal,
            Region::Pal => Region::Dendy,
            Region::Dendy => Region::Ntsc,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ntsc_timing() {
        assert_eq!(Region::Ntsc.scanlines_per_frame(), 262);
        assert_eq!(Region::Ntsc.scanline_prerender(), 261);
        assert_eq!(Region::Ntsc.scanline_vblank_start(), 241);
        assert_eq!(Region::Ntsc.cycles_per_scanline(), 341);
        assert!((Region::Ntsc.cpu_clock_hz() - 1_789_773.0).abs() < 1.0);
        assert!(!Region::Ntsc.is_pal_palette());
    }

    #[test]
    fn pal_timing() {
        assert_eq!(Region::Pal.scanlines_per_frame(), 312);
        assert_eq!(Region::Pal.scanline_prerender(), 311);
        assert_eq!(Region::Pal.scanline_vblank_start(), 241);
        assert_eq!(Region::Pal.cycles_per_scanline(), 341);
        assert!((Region::Pal.cpu_clock_hz() - 1_662_607.0).abs() < 1.0);
        assert!(Region::Pal.is_pal_palette());
    }

    #[test]
    fn dendy_timing() {
        // Dendy: PAL frame length, NTSC CPU speed, NTSC palette.
        assert_eq!(Region::Dendy.scanlines_per_frame(), 312);
        assert_eq!(Region::Dendy.scanline_prerender(), 311);
        assert!((Region::Dendy.cpu_clock_hz() - 1_789_773.0).abs() < 1.0);
        assert!(!Region::Dendy.is_pal_palette());
    }

    #[test]
    fn frame_rates_are_close_to_standard() {
        // NTSC ~60.10 Hz, PAL ~50.01 Hz. Dendy uses the NTSC PPU clock
        // (5.369 MHz) with the PAL frame length (312 scanlines), giving
        // ~50.47 Hz — a known Dendy quirk (the CPU runs slightly faster
        // than a true PAL NES).
        assert!((Region::Ntsc.frame_rate_hz() - 60.0988).abs() < 0.01);
        assert!((Region::Pal.frame_rate_hz() - 50.0070).abs() < 0.01);
        assert!((Region::Dendy.frame_rate_hz() - 50.4700).abs() < 0.01);
    }

    #[test]
    fn cpu_cycles_per_sample_differs_by_region() {
        // NTSC ≈ 40.585, PAL ≈ 37.7.
        assert!((Region::Ntsc.cpu_cycles_per_sample() - 40.585).abs() < 0.01);
        assert!(Region::Pal.cpu_cycles_per_sample() < Region::Ntsc.cpu_cycles_per_sample());
        // Dendy uses NTSC CPU speed → same as NTSC.
        assert!(
            (Region::Dendy.cpu_cycles_per_sample() - Region::Ntsc.cpu_cycles_per_sample()).abs()
                < 1e-6
        );
    }

    #[test]
    fn apu_4step_thresholds_match_region() {
        let ntsc = Region::Ntsc.apu_4step_thresholds();
        assert_eq!(ntsc[0].0, 7457);
        assert_eq!(ntsc[3].0, 29828);
        let pal = Region::Pal.apu_4step_thresholds();
        assert_eq!(pal[0].0, 8314);
        assert_eq!(pal[3].0, 33255);
        // Dendy uses NTSC thresholds.
        assert_eq!(
            Region::Dendy.apu_4step_thresholds(),
            Region::Ntsc.apu_4step_thresholds()
        );
    }

    #[test]
    fn apu_reset_at_differs_by_region_and_mode() {
        assert_eq!(Region::Ntsc.apu_reset_at(false), 29830);
        assert_eq!(Region::Ntsc.apu_reset_at(true), 37282);
        assert_eq!(Region::Pal.apu_reset_at(false), 33257);
        assert_eq!(Region::Pal.apu_reset_at(true), 41570);
        assert_eq!(Region::Dendy.apu_reset_at(false), 29830);
    }

    #[test]
    fn from_config_string_round_trip() {
        assert_eq!(Region::from_config_str("ntsc").unwrap(), Some(Region::Ntsc));
        assert_eq!(Region::from_config_str("PAL").unwrap(), Some(Region::Pal));
        assert_eq!(
            Region::from_config_str("Dendy").unwrap(),
            Some(Region::Dendy)
        );
        assert_eq!(Region::from_config_str("auto").unwrap(), None);
        assert!(Region::from_config_str("xyz").is_err());
    }

    #[test]
    fn cycle_rotates_through_all_regions() {
        assert_eq!(Region::Ntsc.cycle(), Region::Pal);
        assert_eq!(Region::Pal.cycle(), Region::Dendy);
        assert_eq!(Region::Dendy.cycle(), Region::Ntsc);
    }

    #[test]
    fn short_names() {
        assert_eq!(Region::Ntsc.short_name(), "NTSC");
        assert_eq!(Region::Pal.short_name(), "PAL");
        assert_eq!(Region::Dendy.short_name(), "DENDY");
    }

    #[test]
    fn default_is_ntsc() {
        assert_eq!(Region::default(), Region::Ntsc);
    }
}
