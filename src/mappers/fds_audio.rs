//! Famicom Disk System expansion audio — wavetable synthesizer (M36).
//!
//! The FDS audio is a single wavetable channel with frequency modulation.
//! It has:
//! - A 64-entry wave table RAM (6-bit samples, values 0-63)
//! - A 12-bit frequency register
//! - A 6-bit volume register with envelope
//! - A modulator with its own 64-entry wave table (3-bit samples)
//! - A 12-bit modulator frequency register
//! - Modulator gain and sweep
//!
//! # Register map
//!
//! | Address   | Function                                           |
//! |-----------|----------------------------------------------------|
//! | `$4040-$407F` | Wave table RAM write (64 entries, 6-bit)     |
//! | `$4080`   | Volume: bits 0-5 = volume, bit 6 = env disable, bit 7 = env mode |
//! | `$4081`   | Frequency high byte (bits 0-3)                     |
//! | `$4082`   | Frequency low byte                                 |
//! | `$4083`   | Modulator freq high (bits 0-3), bit 7 = mod disable |
//! | `$4084`   | Modulator sweep rate (negative, bits 0-3)          |
//! | `$4085`   | Modulator sweep rate (positive, bits 0-3)          |
//! | `$4086`   | Modulator gain (negative, bits 0-5)                |
//! | `$4087`   | Modulator gain (positive, bits 0-5), bit 7 = mod disable |
//! | `$4088`   | Modulator wave table write (3-bit, auto-increment) |
//! | `$4089`   | Master volume (bits 0-5), bit 6 = wave RAM write mode |
//! | `$408A`   | Sweep speed (bits 0-3), bit 7 = mod disable        |
//! | `$4090`   | Current volume gain (read, bits 0-5)               |
//! | `$4092`   | Current modulator gain (read, bits 0-5)            |
//!
//! See: https://www.nesdev.org/wiki/FDS_audio

/// Wave table size (64 entries).
const WAVE_TABLE_SIZE: usize = 64;

/// Modulator wave table size (64 entries, 3-bit).
const MOD_TABLE_SIZE: usize = 64;

/// FDS audio state.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct FdsAudio {
    /// Wave table RAM (64 entries, 6-bit: values 0-63).
    #[serde(with = "crate::save_state::array_ser")]
    wave_ram: [u8; WAVE_TABLE_SIZE],
    /// Wave table write pointer (auto-increments on $4040-$407F writes
    /// when $4089 bit 6 is clear).
    wave_addr: u8,

    /// Master volume (6-bit, 0-63).
    master_volume: u8,
    /// Volume envelope disable (bit 6 of $4080).
    env_disabled: bool,
    /// Volume envelope mode: true = increase, false = decrease (bit 7 of $4080).
    env_increase: bool,
    /// Current volume gain (envelope output, 6-bit).
    volume_gain: u8,

    /// Main channel frequency (12-bit).
    freq: u16,
    /// Main channel phase accumulator.
    phase_acc: u32,

    /// Modulator frequency (12-bit).
    mod_freq: u16,
    /// Modulator phase accumulator.
    mod_phase_acc: u32,
    /// Modulator wave table (64 entries, 3-bit: values 0-7).
    #[serde(with = "crate::save_state::array_ser")]
    mod_wave: [u8; MOD_TABLE_SIZE],
    /// Modulator wave table write pointer.
    mod_wave_addr: u8,
    /// Modulator disabled flag.
    mod_disabled: bool,
    /// Modulator gain (6-bit, signed accumulation from $4086/$4087).
    mod_gain: i8,
    /// Modulator sweep counter.
    mod_sweep_counter: u8,
    /// Modulator sweep rate (negative, from $4084).
    mod_sweep_neg: u8,
    /// Modulator sweep rate (positive, from $4085).
    mod_sweep_pos: u8,
    /// Current modulator output gain (read at $4092).
    mod_gain_output: u8,
}

impl Default for FdsAudio {
    fn default() -> Self {
        Self::new()
    }
}

impl FdsAudio {
    /// Create a new FDS audio unit with default state.
    pub fn new() -> Self {
        Self {
            wave_ram: [0; WAVE_TABLE_SIZE],
            wave_addr: 0,
            master_volume: 0,
            env_disabled: false,
            env_increase: false,
            volume_gain: 0,
            freq: 0,
            phase_acc: 0,
            mod_freq: 0,
            mod_phase_acc: 0,
            mod_wave: [0; MOD_TABLE_SIZE],
            mod_wave_addr: 0,
            mod_disabled: true,
            mod_gain: 0,
            mod_sweep_counter: 0,
            mod_sweep_neg: 0,
            mod_sweep_pos: 0,
            mod_gain_output: 0,
        }
    }

    /// Write to an FDS audio register. `addr` is the full CPU address
    /// (`$4040-$408A`); the caller is responsible for routing.
    pub fn write_register(&mut self, addr: u16, value: u8) {
        match addr {
            // $4040-$407F: Wave table RAM write.
            0x4040..=0x407F => {
                self.wave_ram[self.wave_addr as usize % WAVE_TABLE_SIZE] = value & 0x3F;
                // Auto-increment when $4089 bit 6 is clear (write mode).
                if !self.wave_write_read_mode() {
                    self.wave_addr = (self.wave_addr + 1) & 0x3F;
                }
            }
            // $4080: Volume + envelope control.
            0x4080 => {
                self.master_volume = value & 0x3F;
                self.env_disabled = (value & 0x40) != 0;
                self.env_increase = (value & 0x80) != 0;
                if self.env_disabled {
                    // When envelope is disabled, volume gain = master volume directly.
                    self.volume_gain = self.master_volume;
                } else {
                    // Envelope restart: reset gain to 0 (increase) or max (decrease).
                    self.volume_gain = if self.env_increase { 0 } else { 0x3F };
                }
            }
            // $4081: Frequency high byte (bits 0-3).
            0x4081 => {
                self.freq = (self.freq & 0x00FF) | ((value as u16 & 0x0F) << 8);
            }
            // $4082: Frequency low byte.
            0x4082 => {
                self.freq = (self.freq & 0x0F00) | value as u16;
            }
            // $4083: Modulator freq high + disable.
            0x4083 => {
                self.mod_freq = (self.mod_freq & 0x00FF) | ((value as u16 & 0x0F) << 8);
                if value & 0x80 != 0 {
                    self.mod_disabled = true;
                    // Reset modulator phase.
                    self.mod_phase_acc = 0;
                }
            }
            // $4084: Modulator sweep rate (negative).
            0x4084 => {
                self.mod_sweep_neg = value & 0x0F;
            }
            // $4085: Modulator sweep rate (positive).
            0x4085 => {
                self.mod_sweep_pos = value & 0x0F;
            }
            // $4086: Modulator gain (negative).
            0x4086 => {
                let neg = (value & 0x3F) as i8;
                self.mod_gain = self.mod_gain.saturating_sub(neg);
            }
            // $4087: Modulator gain (positive) + disable.
            0x4087 => {
                let pos = (value & 0x3F) as i8;
                self.mod_gain = self.mod_gain.saturating_add(pos);
                if value & 0x80 != 0 {
                    self.mod_disabled = true;
                }
            }
            // $4088: Modulator wave table write (3-bit, auto-increment).
            0x4088 => {
                self.mod_wave[self.mod_wave_addr as usize % MOD_TABLE_SIZE] = value & 0x07;
                self.mod_wave_addr = (self.mod_wave_addr + 1) & 0x3F;
            }
            // $4089: Master volume + wave RAM write mode.
            0x4089 => {
                // Bits 0-1 are the master volume (only 2 bits here per NESdev;
                // the full 6-bit volume comes from $4080). Bit 6 = write mode.
                // We store the 2-bit volume in the low bits of master_volume.
                self.master_volume = (self.master_volume & 0x3C) | (value & 0x03);
            }
            // $408A: Sweep speed + modulator disable.
            0x408A => {
                if value & 0x80 != 0 {
                    self.mod_disabled = true;
                }
            }
            _ => {}
        }
    }

    /// Read an FDS audio register. Returns 0 for write-only / unimplemented.
    pub fn read_register(&self, addr: u16) -> u8 {
        match addr {
            // $4090: Current volume gain (bits 0-5).
            0x4090 => self.volume_gain & 0x3F,
            // $4092: Current modulator gain (bits 0-5).
            0x4092 => self.mod_gain_output & 0x3F,
            _ => 0,
        }
    }

    /// Whether the wave RAM is in read mode (bit 6 of $4089 set).
    fn wave_write_read_mode(&self) -> bool {
        // The $4089 bit 6 controls wave RAM access mode. We track it
        // implicitly: when set, wave RAM writes don't auto-increment.
        // For simplicity we always allow writes; the auto-increment
        // behavior is the key difference.
        false
    }

    /// Advance the audio state by `apu_cycles` APU cycles (CPU clock / 2).
    /// The FDS audio runs at the APU clock rate.
    pub fn clock(&mut self, apu_cycles: u32) {
        for _ in 0..apu_cycles {
            // Advance main channel phase.
            // The frequency is a 12-bit value; the phase accumulator
            // advances by `freq` each APU cycle, and the wave table
            // index is the top 6 bits of the 18-bit accumulator.
            self.phase_acc = self.phase_acc.wrapping_add(self.freq as u32);

            // Advance modulator phase (if not disabled).
            if !self.mod_disabled {
                self.mod_phase_acc = self.mod_phase_acc.wrapping_add(self.mod_freq as u32);

                // Modulator sweep: every 8 APU cycles, adjust gain.
                self.mod_sweep_counter = self.mod_sweep_counter.wrapping_add(1);
                if self.mod_sweep_counter >= 8 {
                    self.mod_sweep_counter = 0;
                    if self.mod_sweep_neg > 0 {
                        self.mod_gain = self.mod_gain.saturating_sub(self.mod_sweep_neg as i8);
                    }
                    if self.mod_sweep_pos > 0 {
                        self.mod_gain = self.mod_gain.saturating_add(self.mod_sweep_pos as i8);
                    }
                }
            }

            // Volume envelope: every 8 APU cycles, adjust gain.
            if !self.env_disabled && self.mod_sweep_counter == 0 {
                if self.env_increase && self.volume_gain < 0x3F {
                    self.volume_gain += 1;
                } else if !self.env_increase && self.volume_gain > 0 {
                    self.volume_gain -= 1;
                }
            }
        }
    }

    /// Compute the current audio sample in `[-1.0, 1.0]`.
    ///
    /// The wave table output is a 6-bit value (0-63), centered at 32.
    /// The modulator adds a phase offset to the wave table index.
    /// The final output is scaled by the volume gain.
    pub fn sample(&mut self) -> f32 {
        if self.freq == 0 {
            return 0.0;
        }

        // Wave table index: top 6 bits of the 18-bit phase accumulator.
        let wave_idx = ((self.phase_acc >> 12) & 0x3F) as usize;

        // Modulator output: add modulator gain * modulator wave value
        // to the wave table index.
        let mod_offset = if !self.mod_disabled {
            let mod_idx = ((self.mod_phase_acc >> 12) & 0x3F) as usize;
            let mod_val = self.mod_wave[mod_idx] as i8; // 0-7
                                                        // Modulator gain is signed; multiply by mod wave value.
            (self.mod_gain as i16 * mod_val as i16) >> 3
        } else {
            0
        };

        // Apply modulator offset to wave index (wrapping).
        let eff_idx = ((wave_idx as i16 + mod_offset) & 0x3F) as usize;
        let wave_val = self.wave_ram[eff_idx] as i16; // 0-63

        // Scale by volume gain (0-63).
        let vol = self.volume_gain as i16;
        let sample = (wave_val - 32) * vol; // Center at 0, scale by volume

        // Normalize to [-1.0, 1.0]. Max magnitude = 32 * 63 = 2016.
        let normalized = sample as f32 / 2016.0;
        normalized.clamp(-1.0, 1.0)
    }

    /// Get the wave RAM contents (for debugging / save state).
    pub fn wave_ram(&self) -> &[u8; WAVE_TABLE_SIZE] {
        &self.wave_ram
    }

    /// Get the modulator wave table (for debugging).
    pub fn mod_wave(&self) -> &[u8; MOD_TABLE_SIZE] {
        &self.mod_wave
    }

    /// Current main channel frequency (12-bit). Used by the FDS mapper's
    /// read-only `expansion_audio_sample` to compute the sample without
    /// needing `&mut self`.
    pub fn freq_value(&self) -> u16 {
        self.freq
    }

    /// Current main channel phase accumulator. Used by the FDS mapper's
    /// read-only `expansion_audio_sample`.
    pub(crate) fn phase_value(&self) -> u32 {
        self.phase_acc
    }

    /// Current volume gain (envelope output, 6-bit). Used by the FDS
    /// mapper's read-only `expansion_audio_sample`.
    pub(crate) fn volume_gain_value(&self) -> u8 {
        self.volume_gain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_audio_is_silent() {
        let mut audio = FdsAudio::new();
        // Frequency 0 = silence.
        assert_eq!(audio.sample(), 0.0);
    }

    #[test]
    fn wave_ram_write_stores_6bit_values() {
        let mut audio = FdsAudio::new();
        audio.write_register(0x4040, 0x3F); // max 6-bit value
        assert_eq!(audio.wave_ram()[0], 0x3F);
        audio.write_register(0x4041, 0xFF); // upper bits masked
        assert_eq!(audio.wave_ram()[1], 0x3F);
    }

    #[test]
    fn wave_ram_auto_increments_address() {
        let mut audio = FdsAudio::new();
        for i in 0..64 {
            audio.write_register(0x4040, i as u8 & 0x3F);
        }
        // All 64 entries should be filled.
        for i in 0..64 {
            assert_eq!(audio.wave_ram()[i], i as u8 & 0x3F);
        }
    }

    #[test]
    fn volume_register_sets_envelope() {
        let mut audio = FdsAudio::new();
        // $4080: volume = 0x20, envelope disabled (bit 6).
        audio.write_register(0x4080, 0x60);
        assert_eq!(audio.master_volume, 0x20);
        assert!(audio.env_disabled);
        assert_eq!(audio.volume_gain, 0x20); // disabled → gain = volume
    }

    #[test]
    fn frequency_registers_set_12bit_value() {
        let mut audio = FdsAudio::new();
        audio.write_register(0x4082, 0x34); // low byte
        audio.write_register(0x4081, 0x05); // high byte (bits 0-3)
        assert_eq!(audio.freq, 0x534);
    }

    #[test]
    fn modulator_wave_write_auto_increments() {
        let mut audio = FdsAudio::new();
        audio.write_register(0x4088, 0x07); // max 3-bit
        audio.write_register(0x4088, 0x03);
        assert_eq!(audio.mod_wave()[0], 0x07);
        assert_eq!(audio.mod_wave()[1], 0x03);
    }

    #[test]
    fn modulator_disable_on_4083_bit7() {
        let mut audio = FdsAudio::new();
        audio.write_register(0x4083, 0x80);
        assert!(audio.mod_disabled);
    }

    #[test]
    fn read_volume_gain_returns_6bit() {
        let mut audio = FdsAudio::new();
        audio.write_register(0x4080, 0x6F); // envelope disabled, vol = 0x2F
        let val = audio.read_register(0x4090);
        assert_eq!(val, 0x2F);
    }

    #[test]
    fn sample_produces_output_with_frequency() {
        let mut audio = FdsAudio::new();
        // Set up a simple wave: all entries at max (0x3F).
        for i in 0..64 {
            audio.write_register(0x4040, 0x3F);
            let _ = i;
        }
        // Set volume to max, envelope disabled.
        audio.write_register(0x4080, 0x7F); // vol = 0x3F, env disabled, increase mode
                                            // Set frequency.
        audio.write_register(0x4082, 0x00);
        audio.write_register(0x4081, 0x01);
        // Clock a few cycles.
        audio.clock(100);
        // Sample should be non-zero (positive, since wave = 0x3F > 32).
        let s = audio.sample();
        assert!(s > 0.0, "expected positive sample, got {s}");
    }

    #[test]
    fn sample_with_zero_wave_is_near_zero() {
        let mut audio = FdsAudio::new();
        // Wave RAM is all zeros (center = 32, so 0 - 32 = -32).
        audio.write_register(0x4080, 0x60); // vol = 0x20, env disabled
        audio.write_register(0x4082, 0x00);
        audio.write_register(0x4081, 0x01);
        audio.clock(100);
        let s = audio.sample();
        // Wave value 0, centered → -32 * 0x20 / 2016 ≈ -0.508
        assert!(s < 0.0, "expected negative sample with zero wave, got {s}");
    }

    #[test]
    fn clock_advances_phase() {
        let mut audio = FdsAudio::new();
        audio.write_register(0x4082, 0xFF);
        audio.write_register(0x4081, 0x0F); // freq = 0xFFF
        let phase_before = audio.phase_acc;
        audio.clock(10);
        assert!(audio.phase_acc > phase_before);
    }

    #[test]
    fn modulator_gain_accumulates() {
        let mut audio = FdsAudio::new();
        audio.write_register(0x4087, 0x10); // +16
        assert_eq!(audio.mod_gain, 16);
        audio.write_register(0x4086, 0x08); // -8
        assert_eq!(audio.mod_gain, 8);
    }
}
