//! Namco 163 — iNES mapper 19 (Namco 163 with wavetable expansion audio).
//!
//! Flexible PRG/CHR banking, switchable mirroring, optional PRG-RAM, and
//! an on-chip 8-channel wavetable synthesiser. Wave RAM (240 bytes) at
//! `$4800-$4FFF`, address-latched via `$F800` (bit 7 = auto-inc). Channel
//! params at `$5000-$57FF` (8 ch × 8 bytes). Bank registers via
//! `addr & 0xF801`: `$8000-$B800` CHR 0-7, `$C000-$D800` PRG 0-3, `$E000`
//! mirroring, `$E800` PRG-RAM ctrl, `$F000` IRQ latch (2-byte load).
//!
//! See: https://www.nesdev.org/wiki/Namco_163

use super::{Mapper, Mirroring};

/// PRG-ROM 8 KB bank size.
const PRG_8K_SIZE: usize = 8 * 1024;
/// CHR 1 KB bank size.
const CHR_1K_SIZE: usize = 1024;
/// PRG-RAM size (8 KB at `$6000-$7FFF`).
const PRG_RAM_SIZE: usize = 8 * 1024;
/// Waveform RAM size (240 bytes usable; 256-byte address space).
const WAVE_RAM_SIZE: usize = 0x80;
/// Number of wavetable channels (8; channels 1-7 are 4-bit, ch 7-8 vary).
pub const WAVE_CHANNELS: usize = 8;

/// One Namco 163 wavetable channel.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default)]
struct WaveChan {
    freq: u16,
    length: u8,
    volume: u8,
    offset: u8,
    phase: u32,
    enabled: bool,
}

/// Namco 163 cartridge state.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Namco163 {
    prg_rom: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    prg_ram: Vec<u8>,
    has_battery: bool,
    prg_banks: [u8; 4],
    chr_banks: [u8; 8],
    mirror: Mirroring,
    prg_ram_enable: bool,
    prg_ram_write_protect: bool,
    irq_latch: u16,
    irq_counter: u16,
    irq_enable: bool,
    irq_pending: bool,
    /// Toggle for the two-byte IRQ latch load (via $F000).
    irq_latch_high: bool,
    /// Waveform RAM (128 bytes; 240 physical on real hardware).
    #[serde(with = "crate::save_state::array_ser")]
    wave_ram: [u8; WAVE_RAM_SIZE],
    /// Address latch for `$F800` (bits 0-6 = wave addr, bit 7 = auto-inc).
    wave_addr: u8,
    chans: [WaveChan; WAVE_CHANNELS],
}

impl Namco163 {
    pub fn new(
        prg_rom: Vec<u8>,
        chr_rom: Vec<u8>,
        mirroring: Mirroring,
        has_battery: bool,
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
            prg_banks: [0u8; 4],
            chr_banks: [0u8; 8],
            mirror: mirroring,
            prg_ram_enable: false,
            prg_ram_write_protect: false,
            irq_latch: 0,
            irq_counter: 0,
            irq_enable: false,
            irq_pending: false,
            irq_latch_high: false,
            wave_ram: [0; WAVE_RAM_SIZE],
            wave_addr: 0,
            chans: [WaveChan::default(); WAVE_CHANNELS],
        }
    }

    fn prg_8k_count(&self) -> usize {
        (self.prg_rom.len() / PRG_8K_SIZE).max(1)
    }
    fn chr_1k_count(&self) -> usize {
        (self.chr.len() / CHR_1K_SIZE).max(1)
    }

    fn prg_read_bank(&self, bank: usize, offset: usize) -> u8 {
        let count = self.prg_8k_count();
        let bank = bank % count;
        self.prg_rom
            .get(bank * PRG_8K_SIZE + offset)
            .copied()
            .unwrap_or(0)
    }

    /// Decode a bank-register CPU address (`addr & 0xF801`).
    fn decode(addr: u16) -> u16 {
        addr & 0xF801
    }

    /// Read the waveform byte at the current wave_addr. Note: the real
    /// N163 auto-increments on read too, but `read_prg` is `&self` so we
    /// only auto-increment on writes (the common case for wave loading).
    fn wave_read(&self) -> u8 {
        let a = (self.wave_addr & 0x7F) as usize;
        self.wave_ram[a]
    }

    /// Write the active channel's waveform byte and auto-increment.
    fn wave_write(&mut self, value: u8) {
        let a = (self.wave_addr & 0x7F) as usize;
        self.wave_ram[a] = value;
        if self.wave_addr & 0x80 != 0 {
            // Auto-increment when bit 7 of the latch is set.
            self.wave_addr = (self.wave_addr & 0x80) | ((self.wave_addr + 1) & 0x7F);
        }
    }

    /// Read a channel parameter byte (freq/length/volume/offset) at
    /// `$5000 + ch*8 + off`. The Namco 163 exposes channel registers
    /// interleaved into the `$5000-$5800` range; we model them as
    /// wave-RAM-relative reads of the channel-control region.
    fn chan_param_read(&self, addr: u16) -> u8 {
        // Channel registers live at $5000-$57FF. We decode ch = (addr-$5000)/8.
        let off = (addr - 0x5000) as usize;
        let ch = off / 8;
        let sub = off & 0x07;
        if ch >= WAVE_CHANNELS {
            return 0;
        }
        let c = &self.chans[ch];
        match sub {
            0 => (c.freq & 0xFF) as u8,
            1 => ((c.freq >> 8) & 0x0F) as u8 | ((c.length & 0x0F) << 4),
            2 => c.volume & 0x0F,
            3 => (c.offset & 0x60) | if c.enabled { 0x80 } else { 0 },
            _ => 0,
        }
    }

    /// Write a channel parameter byte.
    fn chan_param_write(&mut self, addr: u16, value: u8) {
        let off = (addr - 0x5000) as usize;
        let ch = off / 8;
        let sub = off & 0x07;
        if ch >= WAVE_CHANNELS {
            return;
        }
        let c = &mut self.chans[ch];
        match sub {
            0 => c.freq = (c.freq & 0x0F00) | value as u16,
            1 => {
                c.freq = (c.freq & 0x00FF) | ((value as u16 & 0x0F) << 8);
                c.length = (value >> 4) & 0x0F;
            }
            2 => c.volume = value & 0x0F,
            3 => {
                c.offset = value & 0x60;
                c.enabled = (value & 0x80) != 0;
            }
            _ => {}
        }
    }

    /// Advance one channel's phase by `cpu_cycles` and return its output
    /// sample (0-15, 4-bit wavetable value scaled by volume).
    fn chan_sample(&self, ch: usize) -> u8 {
        let c = &self.chans[ch];
        if !c.enabled {
            return 0;
        }
        // Waveform length: `length` is in 32-sample units (4 bytes = 8 nibbles).
        // The phase wraps within `length * 8` nibbles. The waveform offset
        // (in 256-byte units, but we use it as a 128-byte index base) sets
        // the wave RAM starting address for this channel.
        let length_nibbles = ((c.length as u32) + 1) * 8;
        let phase_nibble = (c.phase >> 16) % length_nibbles.max(1);
        // Each wave byte holds two 4-bit samples (low nibble first).
        let byte_idx = ((c.offset as u32 + phase_nibble / 2) & 0x7F) as usize;
        let byte = self.wave_ram[byte_idx];
        let nibble = if phase_nibble & 1 != 0 {
            byte >> 4
        } else {
            byte & 0x0F
        };
        // Scale by volume (0-15) → 0-15 output.
        (nibble * c.volume) / 15
    }
}

impl Mapper for Namco163 {
    fn read_prg(&self, addr: u16) -> u8 {
        if (0x6000..0x8000).contains(&addr) {
            if self.prg_ram_enable {
                let idx = (addr as usize - 0x6000) & (PRG_RAM_SIZE - 1);
                return self.prg_ram.get(idx).copied().unwrap_or(0);
            }
            return 0;
        }
        // Waveform RAM read at $4800-$4FFF.
        if (0x4800..0x5000).contains(&addr) {
            return self.wave_read();
        }
        // Channel parameter reads at $5000-$57FF (ch = (addr-$5000)/8).
        if (0x5000..0x5800).contains(&addr) {
            return self.chan_param_read(addr);
        }
        let local = (addr - 0x8000) as usize;
        let slot = local / PRG_8K_SIZE;
        let offset = local & (PRG_8K_SIZE - 1);
        let bank = (self.prg_banks[slot] as usize) % self.prg_8k_count();
        self.prg_read_bank(bank, offset)
    }

    fn write_prg(&mut self, addr: u16, value: u8) {
        if (0x6000..0x8000).contains(&addr) {
            if self.prg_ram_enable && !self.prg_ram_write_protect {
                let idx = (addr as usize - 0x6000) & (PRG_RAM_SIZE - 1);
                if let Some(s) = self.prg_ram.get_mut(idx) {
                    *s = value;
                }
            }
            return;
        }
        // Waveform RAM write at $4800-$4FFF (auto-increment per $F800 latch).
        if (0x4800..0x5000).contains(&addr) {
            self.wave_write(value);
            return;
        }
        // Channel parameter writes at $5000-$57FF (ch = (addr-$5000)/8).
        if (0x5000..0x5800).contains(&addr) {
            self.chan_param_write(addr, value);
            return;
        }
        match Self::decode(addr) {
            0x8000 => self.chr_banks[0] = value,
            0x8800 => self.chr_banks[1] = value,
            0x9000 => self.chr_banks[2] = value,
            0x9800 => self.chr_banks[3] = value,
            0xA000 => self.chr_banks[4] = value,
            0xA800 => self.chr_banks[5] = value,
            0xB000 => self.chr_banks[6] = value,
            0xB800 => self.chr_banks[7] = value,
            0xC000 => self.prg_banks[0] = value & 0x3F,
            0xC800 => self.prg_banks[1] = value & 0x3F,
            0xD000 => self.prg_banks[2] = value & 0x3F,
            0xD800 => self.prg_banks[3] = value & 0x3F,
            0xE000 => {
                self.mirror = match value & 0x03 {
                    0 => Mirroring::Vertical,
                    1 => Mirroring::Horizontal,
                    2 => Mirroring::SingleScreen(0),
                    _ => Mirroring::SingleScreen(1),
                };
            }
            0xE800 => {
                self.prg_ram_enable = (value & 0x80) != 0;
                self.prg_ram_write_protect = (value & 0x40) != 0;
            }
            0xF000 => {
                // IRQ latch: two writes load low then high byte.
                if !self.irq_latch_high {
                    self.irq_latch = (self.irq_latch & 0xFF00) | value as u16;
                    self.irq_latch_high = true;
                } else {
                    self.irq_latch = (self.irq_latch & 0x00FF) | ((value as u16) << 8);
                    self.irq_latch_high = false;
                }
            }
            0xF800 => {
                // Wave RAM address latch: bits 0-6 = address, bit 7 = auto-inc.
                self.wave_addr = value;
            }
            _ => {}
        }
    }

    fn read_chr(&self, addr: u16) -> u8 {
        let slot = (addr as usize) / CHR_1K_SIZE;
        let offset = (addr as usize) & (CHR_1K_SIZE - 1);
        let count = self.chr_1k_count();
        let bank = (self.chr_banks[slot] as usize) % count;
        self.chr
            .get(bank * CHR_1K_SIZE + offset)
            .copied()
            .unwrap_or(0)
    }

    fn write_chr(&mut self, addr: u16, value: u8) {
        if !self.chr_is_ram {
            return;
        }
        let slot = (addr as usize) / CHR_1K_SIZE;
        let offset = (addr as usize) & (CHR_1K_SIZE - 1);
        let count = self.chr_1k_count();
        let bank = (self.chr_banks[slot] as usize) % count;
        if let Some(s) = self.chr.get_mut(bank * CHR_1K_SIZE + offset) {
            *s = value;
        }
    }

    fn mirror_mode(&self) -> Mirroring {
        self.mirror
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

    /// Clock the IRQ counter and advance wavetable channel phases.
    fn clock_cpu(&mut self, cpu_cycles: u32) {
        for _ in 0..cpu_cycles {
            if self.irq_counter == 0 {
                self.irq_counter = self.irq_latch;
                if self.irq_enable {
                    self.irq_pending = true;
                }
            } else {
                self.irq_counter -= 1;
            }
        }
        // Namco 163 audio runs at CPU clock / 15 (one channel) up to CPU
        // clock / 1.875 (8 channels). We approximate with a per-cycle phase
        // advance scaled by the active channel count.
        let active: u32 = self.chans.iter().filter(|c| c.enabled).count() as u32;
        let active = active.max(1);
        for _ in 0..cpu_cycles {
            for ch in 0..WAVE_CHANNELS {
                let c = &mut self.chans[ch];
                if !c.enabled {
                    continue;
                }
                // Phase increment: freq * length, scaled by 1/active.
                let inc = (c.freq as u32) << 8;
                c.phase = c.phase.wrapping_add(inc / active);
            }
        }
    }

    /// Mixed expansion-audio sample in `[-1.0, 1.0]` (M35).
    /// Silence (no channels enabled) maps to 0.0.
    /// See: https://www.nesdev.org/wiki/Namco_163_audio
    fn expansion_audio_sample(&self) -> f32 {
        let mut sum: i32 = 0;
        for ch in 0..WAVE_CHANNELS {
            sum += self.chan_sample(ch) as i32;
        }
        // 8 channels * 15 max = 120 → normalise to [0,1] then scale by gain.
        const N163_GAIN: f32 = 0.4;
        (sum as f32 / 120.0).clamp(-1.0, 1.0) * N163_GAIN
    }

    fn save_state(&self) -> super::MapperState {
        super::MapperState::Namco163(self.clone())
    }

    fn restore_state(&mut self, state: super::MapperState) {
        match state {
            super::MapperState::Namco163(m) => *self = m,
            _ => panic!("Namco163::restore_state: expected Namco163 variant"),
        }
    }
}
