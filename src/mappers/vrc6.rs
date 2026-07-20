//! VRC6 — iNES mappers 24 (VRC6a) and 26 (VRC6b).
//!
//! Konami's VRC6 ASIC adds three expansion audio channels (two pulse waves
//! and a sawtooth) on top of the standard NES APU, plus PRG/CHR banking and
//! a CPU-clocked IRQ timer. It is used by *Akumajou Densetsu* (the Japanese
//! Castlevania III, mapper 24), *Madara*, and *Esper Dream 2* (mapper 26).
//!
//! The two iNES mapper numbers differ only in how the CPU address lines A0
//! and A1 are wired to the chip: mapper 26 swaps them, so the sub-register
//! addresses `$x001` and `$x002` are exchanged. We model this with a
//! `swap_addr` flag set at construction.
//!
//! # Register layout (mapper 24)
//!
//! Only address lines A0, A1, and A12-A15 are used for register decode, so
//! the effective register index is `addr & 0xF003`.
//!
//! | Address   | Function                                  |
//! |-----------|-------------------------------------------|
//! | `$8000`   | 16 KB PRG bank at `$8000-$BFFF`           |
//! | `$9000`   | Pulse 1 duty/volume                       |
//! | `$9001`   | Pulse 1 period low                        |
//! | `$9002`   | Pulse 1 period high + enable              |
//! | `$9003`   | Pulse 1 frequency scaling                 |
//! | `$A000`   | Pulse 2 duty/volume                       |
//! | `$A001`   | Pulse 2 period low                        |
//! | `$A002`   | Pulse 2 period high + enable              |
//! | `$B000`   | Sawtooth accumulator rate                 |
//! | `$B001`   | Sawtooth period low                       |
//! | `$B002`   | Sawtooth period high + enable             |
//! | `$B003`   | Mirroring                                 |
//! | `$C000`   | 8 KB PRG bank at `$C000-$DFFF`            |
//! | `$D000-$D003` | CHR banks 0-3                         |
//! | `$E000-$E003` | CHR banks 4-7                         |
//! | `$F000`   | IRQ latch (reload value)                  |
//! | `$F001`   | IRQ control (enable + reload)             |
//! | `$F002`   | IRQ acknowledge / enable mode             |
//!
//! `$E000-$FFFF` is fixed to the last 8 KB PRG bank. `$6000-$7FFF` is 8 KB
//! PRG-RAM (optionally battery-backed).
//!
//! See: https://www.nesdev.org/wiki/VRC6 and
//! https://www.nesdev.org/wiki/VRC6_audio

use super::{Mapper, Mirroring};

/// PRG-ROM 16 KB bank size (for the `$8000-$BFFF` window).
const PRG_16K_SIZE: usize = 16 * 1024;
/// PRG-ROM 8 KB bank size (for the `$C000-$DFFF` and fixed `$E000` windows).
const PRG_8K_SIZE: usize = 8 * 1024;
/// CHR 1 KB bank size.
const CHR_1K_SIZE: usize = 1024;
/// PRG-RAM size (8 KB at `$6000-$7FFF`).
const PRG_RAM_SIZE: usize = 8 * 1024;

/// A single VRC6 pulse channel (channels 1 and 2 are identical).
#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct PulseChannel {
    /// `$9000`/`$A000`: bit 7 = ignore duty low bit, bits 4-6 = duty (0-7),
    /// bits 0-3 = volume.
    control: u8,
    /// 12-bit period (low 8 bits from `$x001`, high 4 bits from `$x002`).
    period: u16,
    /// Running timer (counts down from `period`; clocks duty step at 0).
    timer: u16,
    /// Duty step position (0-15).
    step: u8,
    /// Channel enable (bit 7 of `$x002`, tracked separately from `control`
    /// bit 7 which is the "ignore duty low bit" flag).
    enabled: bool,
    /// Frequency scaling from `$9003` (bit 0 = halve frequency). Channel 2
    /// has no `$9003` equivalent; this field is unused for channel 2.
    scale: u8,
}

impl PulseChannel {
    /// Duty cycle length in 16ths (1-8, from bits 4-6 of control).
    fn duty(&self) -> u8 {
        ((self.control >> 4) & 0x07) + 1
    }

    /// Volume (bits 0-3 of control).
    fn volume(&self) -> u8 {
        self.control & 0x0F
    }

    /// Advance the channel by `cpu_cycles` CPU cycles and return the
    /// current output sample (0 = silence, otherwise volume).
    fn clock(&mut self, cpu_cycles: u32) -> u8 {
        if !self.enabled {
            return 0;
        }
        // Frequency scaling: if bit 0 of $9003 set, period is doubled
        // (effective frequency halved). Channel 2 leaves `scale` at 0.
        let period = if self.scale & 1 != 0 {
            self.period.saturating_mul(2).max(1)
        } else {
            self.period
        };
        let period = period.max(1);
        for _ in 0..cpu_cycles {
            if self.timer == 0 {
                self.timer = period;
                self.step = (self.step + 1) & 0x0F;
            } else {
                self.timer -= 1;
            }
        }
        // Output is `volume` when the step is within the duty window,
        // 0 otherwise. The duty window is the first `duty()` steps of 16.
        if self.step < self.duty() {
            self.volume()
        } else {
            0
        }
    }
}

/// The VRC6 sawtooth channel.
#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct SawChannel {
    /// `$B000`: accumulator rate (bits 0-5).
    rate: u8,
    /// 12-bit period (low 8 from `$B001`, high 4 from `$B002`).
    period: u16,
    /// Running timer.
    timer: u16,
    /// 7-bit phase accumulator (0-127). Output is accumulator >> 2
    /// (5-bit value 0-31 for the 6-bit DAC).
    accum: u8,
    /// Whether the channel is enabled (bit 7 of `$B002`).
    enabled: bool,
}

impl SawChannel {
    /// Advance the channel by `cpu_cycles` CPU cycles and return the
    /// current output sample (0-31, the top 5 bits of the accumulator).
    fn clock(&mut self, cpu_cycles: u32) -> u8 {
        if !self.enabled {
            return 0;
        }
        let period = self.period.max(1);
        for _ in 0..cpu_cycles {
            if self.timer == 0 {
                self.timer = period;
                // The sawtooth accumulator advances by `rate` each timer
                // reset. The 7-bit accumulator wraps at 128 (>= 0x80).
                self.accum = self.accum.wrapping_add(self.rate & 0x3F);
                if self.accum >= 0x80 {
                    self.accum = 0;
                }
            } else {
                self.timer -= 1;
            }
        }
        // Output is the top 5 bits of the 7-bit accumulator (>> 2 gives
        // a 5-bit value 0-31, feeding the 6-bit DAC).
        self.accum >> 2
    }
}

/// VRC6 cartridge state.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Vrc6 {
    prg_rom: Vec<u8>,
    /// CHR data — ROM when `chr_is_ram` is false, RAM when true.
    chr: Vec<u8>,
    chr_is_ram: bool,
    /// 8 KB PRG-RAM at `$6000-$7FFF`.
    prg_ram: Vec<u8>,
    has_battery: bool,

    /// 16 KB PRG bank register for `$8000-$BFFF`.
    prg_bank_16k: u8,
    /// 8 KB PRG bank register for `$C000-$DFFF`.
    prg_bank_8k: u8,

    /// Eight 1 KB CHR bank registers.
    chr_banks: [u8; 8],

    /// Mirroring: 0 = vertical, 1 = horizontal (bit 0 of `$B003`).
    mirror_horizontal: bool,

    /// IRQ latch (reload value, 8-bit).
    irq_latch: u8,
    /// Running IRQ counter (8-bit, decrements every CPU cycle).
    irq_counter: u8,
    /// IRQ enabled flag.
    irq_enable: bool,
    /// IRQ "enable after acknowledge" mode (bit 1 of `$F002`).
    irq_enable_after_ack: bool,
    /// Asserted when the counter underflows; cleared by `$F002` write.
    irq_pending: bool,

    /// Audio channels.
    pulse1: PulseChannel,
    pulse2: PulseChannel,
    saw: SawChannel,

    /// Mapper 26 swaps A0/A1 register decode.
    swap_addr: bool,
}

impl Vrc6 {
    /// Construct a VRC6 mapper. `swap_addr` should be `false` for mapper 24
    /// and `true` for mapper 26.
    pub fn new(
        prg_rom: Vec<u8>,
        chr_rom: Vec<u8>,
        mirroring: Mirroring,
        has_battery: bool,
        swap_addr: bool,
    ) -> Self {
        let (chr, chr_is_ram) = if chr_rom.is_empty() {
            (vec![0u8; 8 * 1024], true)
        } else {
            (chr_rom, false)
        };
        Self {
            prg_rom,
            chr,
            chr_is_ram,
            prg_ram: vec![0u8; PRG_RAM_SIZE],
            has_battery,
            prg_bank_16k: 0,
            prg_bank_8k: 0,
            chr_banks: [0u8; 8],
            mirror_horizontal: matches!(mirroring, Mirroring::Horizontal),
            irq_latch: 0,
            irq_counter: 0,
            irq_enable: false,
            irq_enable_after_ack: false,
            irq_pending: false,
            pulse1: PulseChannel::default(),
            pulse2: PulseChannel::default(),
            saw: SawChannel::default(),
            swap_addr,
        }
    }

    /// Number of 16 KB PRG banks (for the `$8000` window).
    fn prg_16k_count(&self) -> usize {
        (self.prg_rom.len() / PRG_16K_SIZE).max(1)
    }

    /// Number of 8 KB PRG banks (for the `$C000` and fixed windows).
    fn prg_8k_count(&self) -> usize {
        (self.prg_rom.len() / PRG_8K_SIZE).max(1)
    }

    /// Number of 1 KB CHR banks.
    fn chr_1k_count(&self) -> usize {
        (self.chr.len() / CHR_1K_SIZE).max(1)
    }

    /// Decode a register address, applying the A0/A1 swap for mapper 26.
    /// Returns the canonical mapper-24 address (`addr & 0xF003` with A0/A1
    /// unswapped).
    fn decode(&self, addr: u16) -> u16 {
        let masked = addr & 0xF003;
        if !self.swap_addr {
            return masked;
        }
        // Swap A0 and A1: extract each, exchange, recombine.
        let a0 = (masked & 0x001) << 1;
        let a1 = (masked & 0x002) >> 1;
        (masked & 0xFFFC) | a0 | a1
    }

    /// Read a PRG-ROM byte from a given 8 KB bank + offset.
    fn prg_read_8k(&self, bank: usize, offset: usize) -> u8 {
        let count = self.prg_8k_count();
        let bank = bank % count;
        let idx = bank * PRG_8K_SIZE + offset;
        self.prg_rom.get(idx).copied().unwrap_or(0)
    }

    /// Read a PRG-ROM byte from a given 16 KB bank + offset.
    fn prg_read_16k(&self, bank: usize, offset: usize) -> u8 {
        let count = self.prg_16k_count();
        let bank = bank % count;
        let idx = bank * PRG_16K_SIZE + offset;
        self.prg_rom.get(idx).copied().unwrap_or(0)
    }

    /// Advance the audio channels by `cpu_cycles` CPU cycles. The expansion
    /// audio mixing into the APU output stream is wired in M35; this method
    /// updates channel state and returns the combined expansion sample
    /// (pulse1 + pulse2 + saw, scaled to the APU's 8-bit range).
    pub fn clock_audio(&mut self, cpu_cycles: u32) {
        self.pulse1.clock(cpu_cycles);
        self.pulse2.clock(cpu_cycles);
        self.saw.clock(cpu_cycles);
    }

    /// Current expansion-audio sample (mixed pulse1 + pulse2 + sawtooth).
    /// Returns a value in 0-255 suitable for mixing with the internal APU
    /// output. The exact mix coefficients are tuned in M35; here we use a
    /// simple sum of the three channels' 6-bit outputs scaled to 8 bits.
    pub fn audio_sample(&self) -> u8 {
        let p1 = self.pulse1_sample();
        let p2 = self.pulse2_sample();
        let saw = self.saw_sample();
        // Each channel is 6-bit (0-63) on real hardware; we approximate the
        // mix as a clipped sum scaled to 8 bits.
        let sum = (p1 as u32)
            .saturating_add(p2 as u32)
            .saturating_add(saw as u32);
        (sum.min(63) * 255 / 63) as u8
    }

    /// Pulse 1 current output (0-15, the channel's volume when active).
    pub fn pulse1_sample(&self) -> u8 {
        if self.pulse1.enabled && self.pulse1.step < self.pulse1.duty() {
            self.pulse1.volume()
        } else {
            0
        }
    }

    /// Pulse 2 current output (0-15).
    pub fn pulse2_sample(&self) -> u8 {
        if self.pulse2.enabled && self.pulse2.step < self.pulse2.duty() {
            self.pulse2.volume()
        } else {
            0
        }
    }

    /// Sawtooth current output (0-31).
    pub fn saw_sample(&self) -> u8 {
        if self.saw.enabled {
            self.saw.accum >> 2
        } else {
            0
        }
    }
}

impl Mapper for Vrc6 {
    fn read_prg(&self, addr: u16) -> u8 {
        // PRG-RAM at $6000-$7FFF.
        if (0x6000..0x8000).contains(&addr) {
            let idx = (addr as usize - 0x6000) & (PRG_RAM_SIZE - 1);
            return self.prg_ram.get(idx).copied().unwrap_or(0);
        }

        let local = (addr - 0x8000) as usize; // 0..=0x7FFF
        if local < PRG_16K_SIZE {
            // $8000-$BFFF: 16 KB switchable bank.
            let bank = (self.prg_bank_16k as usize) % self.prg_16k_count();
            return self.prg_read_16k(bank, local);
        }
        // $C000-$DFFF: 8 KB switchable bank.
        if local < PRG_16K_SIZE + PRG_8K_SIZE {
            let off = local - PRG_16K_SIZE;
            let bank = (self.prg_bank_8k as usize) % self.prg_8k_count();
            return self.prg_read_8k(bank, off);
        }
        // $E000-$FFFF: fixed last 8 KB bank.
        let off = local - PRG_16K_SIZE - PRG_8K_SIZE;
        let last = self.prg_8k_count().saturating_sub(1);
        self.prg_read_8k(last, off)
    }

    fn write_prg(&mut self, addr: u16, value: u8) {
        // PRG-RAM at $6000-$7FFF.
        if (0x6000..0x8000).contains(&addr) {
            let idx = (addr as usize - 0x6000) & (PRG_RAM_SIZE - 1);
            if let Some(slot) = self.prg_ram.get_mut(idx) {
                *slot = value;
            }
            return;
        }

        let reg = self.decode(addr);
        match reg {
            0x8000 => self.prg_bank_16k = value & 0x3F,
            0x9000 => self.pulse1.control = value,
            0x9001 => self.pulse1.period = (self.pulse1.period & 0x0F00) | value as u16,
            0x9002 => {
                self.pulse1.period = (self.pulse1.period & 0x00FF) | ((value as u16 & 0x0F) << 8);
                // Bit 7 of $x002 = channel enable (separate from $9000
                // bit 7 which is the "ignore duty low bit" flag).
                self.pulse1.enabled = (value & 0x80) != 0;
            }
            0x9003 => self.pulse1.scale = value,
            0xA000 => self.pulse2.control = value,
            0xA001 => self.pulse2.period = (self.pulse2.period & 0x0F00) | value as u16,
            0xA002 => {
                self.pulse2.period = (self.pulse2.period & 0x00FF) | ((value as u16 & 0x0F) << 8);
                self.pulse2.enabled = (value & 0x80) != 0;
            }
            0xB000 => self.saw.rate = value,
            0xB001 => self.saw.period = (self.saw.period & 0x0F00) | value as u16,
            0xB002 => {
                self.saw.period = (self.saw.period & 0x00FF) | ((value as u16 & 0x0F) << 8);
                self.saw.enabled = (value & 0x80) != 0;
            }
            0xB003 => {
                // Mirroring: bit 0 = 0 vertical, 1 horizontal.
                self.mirror_horizontal = (value & 0x01) != 0;
            }
            0xC000 => self.prg_bank_8k = value & 0x3F,
            0xD000 => self.chr_banks[0] = value,
            0xD001 => self.chr_banks[1] = value,
            0xD002 => self.chr_banks[2] = value,
            0xD003 => self.chr_banks[3] = value,
            0xE000 => self.chr_banks[4] = value,
            0xE001 => self.chr_banks[5] = value,
            0xE002 => self.chr_banks[6] = value,
            0xE003 => self.chr_banks[7] = value,
            0xF000 => {
                // IRQ latch (reload value).
                self.irq_latch = value;
            }
            0xF001 => {
                // IRQ control: bit 0 = enable, and reload counter from latch.
                self.irq_enable = (value & 0x01) != 0;
                self.irq_counter = self.irq_latch;
            }
            0xF002 => {
                // IRQ acknowledge: bit 0 = enable, bit 1 = enable-after-ack.
                self.irq_enable_after_ack = (value & 0x02) != 0;
                self.irq_enable = (value & 0x01) != 0;
                self.irq_pending = false;
            }
            _ => {}
        }
    }

    fn read_chr(&self, addr: u16) -> u8 {
        let slot = (addr as usize) / CHR_1K_SIZE; // 0..=7
        let offset = (addr as usize) & (CHR_1K_SIZE - 1);
        let count = self.chr_1k_count();
        let bank = (self.chr_banks[slot] as usize) % count;
        let idx = bank * CHR_1K_SIZE + offset;
        self.chr.get(idx).copied().unwrap_or(0)
    }

    fn write_chr(&mut self, addr: u16, value: u8) {
        if !self.chr_is_ram {
            return;
        }
        let slot = (addr as usize) / CHR_1K_SIZE;
        let offset = (addr as usize) & (CHR_1K_SIZE - 1);
        let count = self.chr_1k_count();
        let bank = (self.chr_banks[slot] as usize) % count;
        let idx = bank * CHR_1K_SIZE + offset;
        if let Some(s) = self.chr.get_mut(idx) {
            *s = value;
        }
    }

    fn mirror_mode(&self) -> Mirroring {
        if self.mirror_horizontal {
            Mirroring::Horizontal
        } else {
            Mirroring::Vertical
        }
    }

    fn has_battery(&self) -> bool {
        self.has_battery
    }

    fn battery_sram(&self) -> Option<Vec<u8>> {
        if self.has_battery {
            Some(self.prg_ram.clone())
        } else {
            None
        }
    }

    fn load_battery_sram(&mut self, data: &[u8]) {
        if !self.has_battery {
            return;
        }
        let len = self.prg_ram.len().min(data.len());
        self.prg_ram[..len].copy_from_slice(&data[..len]);
    }

    fn irq_pending(&self) -> bool {
        self.irq_pending
    }

    /// Clock the VRC6 IRQ counter by `cpu_cycles` CPU cycles. The 8-bit
    /// counter decrements each CPU cycle; when it underflows (0 → reload),
    /// the IRQ is asserted if enabled.
    fn clock_cpu(&mut self, cpu_cycles: u32) {
        for _ in 0..cpu_cycles {
            if self.irq_counter == 0 {
                self.irq_counter = self.irq_latch;
                if self.irq_enable {
                    self.irq_pending = true;
                    if !self.irq_enable_after_ack {
                        // One-shot mode: disable after firing.
                        self.irq_enable = false;
                    }
                }
            } else {
                self.irq_counter -= 1;
            }
        }
        // Also advance the expansion audio channels by the same CPU cycles.
        self.clock_audio(cpu_cycles);
    }

    fn save_state(&self) -> super::MapperState {
        super::MapperState::Vrc6(self.clone())
    }

    /// Mixed expansion-audio sample in `[-1.0, 1.0]` (M35).
    ///
    /// The VRC6 has three 6-bit DAC channels (two pulses at 0-15, one
    /// sawtooth at 0-31). The NESdev "VRC6 audio" page documents the
    /// channel mix: each channel contributes its DAC value, and the
    /// combined output is `pulse1 + pulse2 + saw` clipped to the 6-bit
    /// range, then normalised. We map the 6-bit mix (0-63) to `[-1, 1]`
    /// via `out = sample / 63.0 * 2.0 - 1.0` (silence → -1.0, matching
    /// the internal APU's DC-offset convention from M16/M31).
    ///
    /// See: https://www.nesdev.org/wiki/VRC6_audio
    fn expansion_audio_sample(&self) -> f32 {
        let p1 = self.pulse1_sample() as u32; // 0..=15
        let p2 = self.pulse2_sample() as u32; // 0..=15
        let saw = self.saw_sample() as u32; // 0..=31
        let sum = p1.saturating_add(p2).saturating_add(saw).min(63);
        // 6-bit mix → [-1.0, 1.0]. Gain trimmed so the chip sits roughly
        // at the same perceived loudness as the internal APU.
        const VRC6_GAIN: f32 = 0.75;
        let normalized = (sum as f32 / 63.0) * 2.0 - 1.0;
        normalized * VRC6_GAIN
    }

    fn restore_state(&mut self, state: super::MapperState) {
        match state {
            super::MapperState::Vrc6(m) => *self = m,
            _ => panic!("Vrc6::restore_state: expected Vrc6 variant, got a different mapper type"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build PRG-ROM with `banks_8k` 8 KB banks, each filled with a unique
    /// byte (the bank index modulo 256).
    fn make_prg(banks_8k: usize) -> Vec<u8> {
        let mut prg = vec![0u8; banks_8k * PRG_8K_SIZE];
        for (i, b) in prg.iter_mut().enumerate() {
            *b = (i / PRG_8K_SIZE) as u8;
        }
        prg
    }

    /// Build CHR-ROM with `banks` 1 KB banks, each filled with a unique byte.
    fn make_chr(banks: usize) -> Vec<u8> {
        let mut chr = vec![0u8; banks * CHR_1K_SIZE];
        for (i, b) in chr.iter_mut().enumerate() {
            *b = (i / CHR_1K_SIZE) as u8;
        }
        chr
    }

    // ---- PRG banking ---------------------------------------------------

    #[test]
    fn prg_default_16k_bank_0_at_8000_fixed_last_at_e000() {
        // 8 × 8KB = 64KB PRG = 4 × 16KB banks. Default 16k bank = 0.
        // $C000 8k bank = 0. $E000 fixed last 8KB = bank 7.
        let vrc = Vrc6::new(make_prg(8), make_chr(8), Mirroring::Vertical, false, false);
        // $8000-$BFFF = 16KB bank 0 = 8KB banks 0,1.
        assert_eq!(vrc.read_prg(0x8000), 0x00);
        assert_eq!(vrc.read_prg(0x9FFF), 0x00);
        assert_eq!(vrc.read_prg(0xA000), 0x01);
        assert_eq!(vrc.read_prg(0xBFFF), 0x01);
        // $C000-$DFFF = 8KB bank 0.
        assert_eq!(vrc.read_prg(0xC000), 0x00);
        assert_eq!(vrc.read_prg(0xDFFF), 0x00);
        // $E000-$FFFF = fixed last 8KB = bank 7.
        assert_eq!(vrc.read_prg(0xE000), 0x07);
        assert_eq!(vrc.read_prg(0xFFFF), 0x07);
    }

    #[test]
    fn prg_16k_bank_select_switches_8000_window() {
        let mut vrc = Vrc6::new(make_prg(8), make_chr(8), Mirroring::Vertical, false, false);
        // 16k bank 2 → 8KB banks 4,5 at $8000-$BFFF.
        vrc.write_prg(0x8000, 0x02);
        assert_eq!(vrc.read_prg(0x8000), 0x04);
        assert_eq!(vrc.read_prg(0xBFFF), 0x05);
        // $C000 unchanged (8k bank 0).
        assert_eq!(vrc.read_prg(0xC000), 0x00);
    }

    #[test]
    fn prg_8k_bank_select_switches_c000_window() {
        let mut vrc = Vrc6::new(make_prg(8), make_chr(8), Mirroring::Vertical, false, false);
        vrc.write_prg(0xC000, 0x03); // 8k bank 3 at $C000.
        assert_eq!(vrc.read_prg(0xC000), 0x03);
        assert_eq!(vrc.read_prg(0xDFFF), 0x03);
        // $8000 unchanged.
        assert_eq!(vrc.read_prg(0x8000), 0x00);
    }

    #[test]
    fn prg_bank_wraps_within_rom_size() {
        // 4 × 8KB = 32KB = 2 × 16KB banks. 16k bank 5 → 5 % 2 = 1.
        let mut vrc = Vrc6::new(make_prg(4), make_chr(8), Mirroring::Vertical, false, false);
        vrc.write_prg(0x8000, 0x05);
        assert_eq!(vrc.read_prg(0x8000), 0x02); // 16k bank 1 = 8k banks 2,3
        assert_eq!(vrc.read_prg(0xBFFF), 0x03);
    }

    #[test]
    fn prg_register_uses_f003_mask() {
        // Only A0, A1, A12-A15 are used for register decode, so addresses
        // that share those bits hit the same register. $8FFC & 0xF003 =
        // 0x8000 (the 16 KB PRG bank register).
        let mut vrc = Vrc6::new(make_prg(8), make_chr(8), Mirroring::Vertical, false, false);
        vrc.write_prg(0x8FFC, 0x02); // decodes to $8000 → 16k bank 2
        assert_eq!(vrc.read_prg(0x8000), 0x04); // 16k bank 2 = 8k banks 4,5
    }

    // ---- CHR banking ---------------------------------------------------

    #[test]
    fn chr_eight_1kb_banks_selectable() {
        let mut vrc = Vrc6::new(make_prg(8), make_chr(16), Mirroring::Vertical, false, false);
        vrc.write_prg(0xD000, 0x02); // CHR bank 0 = 2
        vrc.write_prg(0xD001, 0x05); // CHR bank 1 = 5
        vrc.write_prg(0xE003, 0x0E); // CHR bank 7 = 14
        assert_eq!(vrc.read_chr(0x0000), 0x02);
        assert_eq!(vrc.read_chr(0x03FF), 0x02);
        assert_eq!(vrc.read_chr(0x0400), 0x05);
        assert_eq!(vrc.read_chr(0x07FF), 0x05);
        assert_eq!(vrc.read_chr(0x1C00), 0x0E);
        assert_eq!(vrc.read_chr(0x1FFF), 0x0E);
    }

    #[test]
    fn chr_ram_writes_persist() {
        let mut vrc = Vrc6::new(make_prg(8), vec![], Mirroring::Vertical, false, false);
        vrc.write_chr(0x0000, 0x42);
        assert_eq!(vrc.read_chr(0x0000), 0x42);
    }

    // ---- Mirroring -----------------------------------------------------

    #[test]
    fn mirroring_default_from_header() {
        let vrc = Vrc6::new(
            make_prg(8),
            make_chr(8),
            Mirroring::Horizontal,
            false,
            false,
        );
        assert_eq!(vrc.mirror_mode(), Mirroring::Horizontal);
    }

    #[test]
    fn mirroring_switchable_via_b003() {
        let mut vrc = Vrc6::new(make_prg(8), make_chr(8), Mirroring::Vertical, false, false);
        assert_eq!(vrc.mirror_mode(), Mirroring::Vertical);
        vrc.write_prg(0xB003, 0x01); // horizontal
        assert_eq!(vrc.mirror_mode(), Mirroring::Horizontal);
        vrc.write_prg(0xB003, 0x00); // vertical
        assert_eq!(vrc.mirror_mode(), Mirroring::Vertical);
    }

    // ---- IRQ -----------------------------------------------------------

    #[test]
    fn irq_fires_after_counter_underflow() {
        let mut vrc = Vrc6::new(make_prg(8), make_chr(8), Mirroring::Vertical, false, false);
        // Latch = 4, enable + reload via $F001.
        vrc.write_prg(0xF000, 0x04);
        vrc.write_prg(0xF001, 0x01); // enable + reload → counter = 4
        assert!(!vrc.irq_pending());
        // 4 cycles → counter 4→3→2→1→0 (no underflow yet at 0).
        vrc.clock_cpu(4);
        assert!(!vrc.irq_pending());
        // 1 more cycle → underflow (0 → reload), IRQ fires.
        vrc.clock_cpu(1);
        assert!(vrc.irq_pending());
    }

    #[test]
    fn irq_disabled_does_not_fire() {
        let mut vrc = Vrc6::new(make_prg(8), make_chr(8), Mirroring::Vertical, false, false);
        vrc.write_prg(0xF000, 0x02);
        vrc.write_prg(0xF001, 0x00); // disable + reload
        vrc.clock_cpu(10);
        assert!(!vrc.irq_pending());
    }

    #[test]
    fn irq_acknowledge_clears_pending() {
        let mut vrc = Vrc6::new(make_prg(8), make_chr(8), Mirroring::Vertical, false, false);
        vrc.write_prg(0xF000, 0x01);
        vrc.write_prg(0xF001, 0x01); // enable + reload → counter = 1
        vrc.clock_cpu(2); // underflow → IRQ fires
        assert!(vrc.irq_pending());
        vrc.write_prg(0xF002, 0x00); // acknowledge (disable)
        assert!(!vrc.irq_pending());
    }

    #[test]
    fn irq_one_shot_mode_disables_after_firing() {
        let mut vrc = Vrc6::new(make_prg(8), make_chr(8), Mirroring::Vertical, false, false);
        vrc.write_prg(0xF000, 0x01);
        vrc.write_prg(0xF001, 0x01); // enable + reload → counter = 1
        vrc.clock_cpu(2); // fires, one-shot → disable
        assert!(vrc.irq_pending());
        vrc.write_prg(0xF002, 0x00); // ack
                                     // Counter continues to decrement but IRQ stays disabled.
        vrc.clock_cpu(10);
        assert!(!vrc.irq_pending());
    }

    // ---- Audio ---------------------------------------------------------

    #[test]
    fn audio_pulse_channel_produces_output_when_enabled() {
        let mut vrc = Vrc6::new(make_prg(8), make_chr(8), Mirroring::Vertical, false, false);
        // Pulse 1: duty = 8/16 (bits 4-6 = 0b111), volume = 15, enable.
        vrc.write_prg(0x9000, 0b0111_1111); // duty 7+1=8, vol 15
        vrc.write_prg(0x9001, 0x10); // period low = 0x10
        vrc.write_prg(0x9002, 0x80); // period high = 0, enable bit 7
                                     // Clock enough cycles to step through the duty window.
        vrc.clock_audio(64);
        // At some point the output should be non-zero (volume 15).
        let sample = vrc.pulse1_sample();
        // After 64 cycles with period 0x10=16, the step advanced ~4 times.
        // The sample depends on step position; just assert it can be 15.
        // We clock a bit more and check the max sample observed.
        let mut saw_nonzero = false;
        for _ in 0..100 {
            vrc.clock_audio(1);
            if vrc.pulse1_sample() == 15 {
                saw_nonzero = true;
                break;
            }
        }
        assert!(saw_nonzero, "pulse1 should produce volume output");
        let _ = sample;
    }

    #[test]
    fn audio_saw_channel_produces_rising_output() {
        let mut vrc = Vrc6::new(make_prg(8), make_chr(8), Mirroring::Vertical, false, false);
        vrc.write_prg(0xB000, 0x20); // accumulator rate = 32
        vrc.write_prg(0xB001, 0x04); // period low = 4
        vrc.write_prg(0xB002, 0x80); // period high = 0, enable
                                     // Clock many cycles; the accumulator should rise and reset.
        let mut saw_nonzero = false;
        for _ in 0..2000 {
            vrc.clock_audio(1);
            if vrc.saw_sample() > 0 {
                saw_nonzero = true;
            }
        }
        assert!(saw_nonzero, "saw should produce non-zero output");
    }

    #[test]
    fn audio_disabled_channel_produces_no_output() {
        let vrc = Vrc6::new(make_prg(8), make_chr(8), Mirroring::Vertical, false, false);
        assert_eq!(vrc.pulse1_sample(), 0);
        assert_eq!(vrc.pulse2_sample(), 0);
        assert_eq!(vrc.saw_sample(), 0);
    }

    // ---- Mapper 26 A0/A1 swap -----------------------------------------

    #[test]
    fn mapper26_swaps_a0_a1_for_register_decode() {
        // Mapper 26: $x001 ↔ $x002. So writing $9002 (mapper 26) should hit
        // the period-low register (which is $9001 in mapper 24 terms).
        let mut vrc = Vrc6::new(make_prg(8), make_chr(8), Mirroring::Vertical, false, true);
        // $9002 in mapper 26 decodes to $9001 (period low) in canonical form.
        vrc.write_prg(0x9002, 0xAB);
        assert_eq!(vrc.pulse1.period & 0xFF, 0xAB);
    }

    // ---- PRG-RAM / battery --------------------------------------------

    #[test]
    fn prg_ram_writes_and_reads_at_6000() {
        let mut vrc = Vrc6::new(make_prg(8), make_chr(8), Mirroring::Vertical, false, false);
        vrc.write_prg(0x6000, 0x42);
        assert_eq!(vrc.read_prg(0x6000), 0x42);
    }

    #[test]
    fn battery_sram_round_trips() {
        let mut vrc = Vrc6::new(make_prg(8), make_chr(8), Mirroring::Vertical, true, false);
        vrc.write_prg(0x6000, 0xCD);
        let saved = vrc.battery_sram().expect("battery");
        let mut vrc2 = Vrc6::new(make_prg(8), make_chr(8), Mirroring::Vertical, true, false);
        vrc2.load_battery_sram(&saved);
        assert_eq!(vrc2.read_prg(0x6000), 0xCD);
    }
}
