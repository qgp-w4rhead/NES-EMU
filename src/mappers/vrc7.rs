//! VRC7 — iNES mapper 85 (Konami VRC7 with YM2413 OPLL audio).
//!
//! The VRC7 is Konami's VRC7 ASIC: flexible PRG/CHR banking, a CPU-clocked
//! IRQ timer, and an on-chip YM2413 OPLL FM synthesiser providing 9 channels
//! of expansion audio. Used by *Lagrange Point* and *Tiny Toon Adventures 2*.
//!
//! # Register layout
//!
//! Register decode uses A0, A2, A3, A4, A5, and A12-A15, so the effective
//! register index is `addr & 0xF03D`.
//!
//! | Address   | Function                                       |
//! |-----------|------------------------------------------------|
//! | `$8000`   | 8 KB PRG bank at `$8000-$9FFF`                 |
//! | `$8008`   | 8 KB PRG bank at `$A000-$BFFF`                 |
//! | `$9000`   | 8 KB PRG bank at `$C000-$DFFF`                 |
//! | `$9010`   | OPLL address latch (write-only)                |
//! | `$9030`   | OPLL data write (write-only)                   |
//! | `$B000`   | Mirroring (bit 0: 0=vertical, 1=horizontal)    |
//! | `$C000-$C00C` | CHR banks 0-3 (1 KB each, step 4)         |
//! | `$D000-$D00C` | CHR banks 4-7 (1 KB each, step 4)         |
//! | `$E000`   | IRQ latch (reload value)                       |
//! | `$E008`   | IRQ control (bit 0=enable, bit 1=ack+enable)  |
//! | `$E010`   | IRQ acknowledge                               |
//!
//! `$E000-$FFFF` is fixed to the last 8 KB PRG bank. `$6000-$7FFF` is 8 KB
//! PRG-RAM (optionally battery-backed).
//!
//! See: https://www.nesdev.org/wiki/VRC7

use super::opll::Opll;
use super::{Mapper, Mirroring};

/// PRG-ROM 8 KB bank size.
const PRG_8K_SIZE: usize = 8 * 1024;
/// CHR 1 KB bank size.
const CHR_1K_SIZE: usize = 1024;
/// PRG-RAM size (8 KB at `$6000-$7FFF`).
const PRG_RAM_SIZE: usize = 8 * 1024;

/// VRC7 cartridge state.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Vrc7 {
    prg_rom: Vec<u8>,
    chr: Vec<u8>,
    chr_is_ram: bool,
    prg_ram: Vec<u8>,
    has_battery: bool,
    /// Three 8 KB PRG bank registers (`$8000`, `$A000`, `$C000` windows).
    prg_banks: [u8; 3],
    /// Eight 1 KB CHR bank registers.
    chr_banks: [u8; 8],
    mirror_horizontal: bool,
    /// 16-bit IRQ latch (reload value).
    irq_latch: u16,
    /// Running 16-bit IRQ counter.
    irq_counter: u16,
    /// Toggle for the two-byte IRQ latch load.
    irq_latch_high: bool,
    irq_enable: bool,
    irq_pending: bool,
    /// On-chip YM2413 OPLL FM synthesiser.
    opll: Opll,
}

impl Vrc7 {
    /// Construct a VRC7 mapper from parsed PRG/CHR data.
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
            prg_banks: [0u8; 3],
            chr_banks: [0u8; 8],
            mirror_horizontal: matches!(mirroring, Mirroring::Horizontal),
            irq_latch: 0,
            irq_counter: 0,
            irq_latch_high: false,
            irq_enable: false,
            irq_pending: false,
            opll: Opll::new(),
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
        let idx = bank * PRG_8K_SIZE + offset;
        self.prg_rom.get(idx).copied().unwrap_or(0)
    }

    /// Decode a CPU address into a register index (`addr & 0xF03D`).
    fn decode(addr: u16) -> u16 {
        addr & 0xF03D
    }
}

impl Mapper for Vrc7 {
    fn read_prg(&self, addr: u16) -> u8 {
        if (0x6000..0x8000).contains(&addr) {
            let idx = (addr as usize - 0x6000) & (PRG_RAM_SIZE - 1);
            return self.prg_ram.get(idx).copied().unwrap_or(0);
        }
        let local = (addr - 0x8000) as usize;
        let slot = local / PRG_8K_SIZE; // 0..=3
        let offset = local & (PRG_8K_SIZE - 1);
        if slot < 3 {
            let bank = (self.prg_banks[slot] as usize) % self.prg_8k_count();
            self.prg_read_bank(bank, offset)
        } else {
            // $E000-$FFFF: fixed last 8 KB bank.
            let last = self.prg_8k_count().saturating_sub(1);
            self.prg_read_bank(last, offset)
        }
    }

    fn write_prg(&mut self, addr: u16, value: u8) {
        if (0x6000..0x8000).contains(&addr) {
            let idx = (addr as usize - 0x6000) & (PRG_RAM_SIZE - 1);
            if let Some(slot) = self.prg_ram.get_mut(idx) {
                *slot = value;
            }
            return;
        }
        match Self::decode(addr) {
            0x8000 => self.prg_banks[0] = value & 0x3F,
            0x8008 => self.prg_banks[1] = value & 0x3F,
            0x9000 => self.prg_banks[2] = value & 0x3F,
            0x9010 => self.opll.write_addr(value),
            0x9030 => self.opll.write_data(value),
            0xB000 => self.mirror_horizontal = (value & 0x01) != 0,
            0xC000 => self.chr_banks[0] = value,
            0xC004 => self.chr_banks[1] = value,
            0xC008 => self.chr_banks[2] = value,
            0xC00C => self.chr_banks[3] = value,
            0xD000 => self.chr_banks[4] = value,
            0xD004 => self.chr_banks[5] = value,
            0xD008 => self.chr_banks[6] = value,
            0xD00C => self.chr_banks[7] = value,
            0xE000 => {
                // IRQ latch: two writes load low then high byte.
                if !self.irq_latch_high {
                    self.irq_latch = (self.irq_latch & 0xFF00) | value as u16;
                    self.irq_latch_high = true;
                } else {
                    self.irq_latch = (self.irq_latch & 0x00FF) | ((value as u16) << 8);
                    self.irq_latch_high = false;
                }
            }
            0xE008 => {
                if value & 0x02 != 0 {
                    self.irq_enable = true;
                    self.irq_pending = false;
                    self.irq_counter = self.irq_latch;
                } else if value & 0x01 != 0 {
                    self.irq_enable = true;
                    self.irq_counter = self.irq_latch;
                } else {
                    self.irq_enable = false;
                }
            }
            0xE010 => {
                // Acknowledge: clear pending, keep enable state.
                self.irq_pending = false;
            }
            _ => {}
        }
    }

    fn read_chr(&self, addr: u16) -> u8 {
        let slot = (addr as usize) / CHR_1K_SIZE;
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

    /// Clock the VRC7 16-bit IRQ counter by `cpu_cycles` CPU cycles and
    /// advance the OPLL audio state by `cpu_cycles / 2` APU cycles.
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
        // OPLL runs at APU clock = CPU clock / 2.
        self.opll.clock(cpu_cycles / 2);
    }

    /// Mixed expansion-audio sample in `[-1.0, 1.0]` (M35).
    /// See: https://www.nesdev.org/wiki/VRC7_audio
    fn expansion_audio_sample(&self) -> f32 {
        const VRC7_GAIN: f32 = 0.6;
        self.opll.sample() * VRC7_GAIN
    }

    fn save_state(&self) -> super::MapperState {
        super::MapperState::Vrc7(self.clone())
    }

    fn restore_state(&mut self, state: super::MapperState) {
        match state {
            super::MapperState::Vrc7(m) => *self = m,
            _ => panic!("Vrc7::restore_state: expected Vrc7 variant"),
        }
    }
}
