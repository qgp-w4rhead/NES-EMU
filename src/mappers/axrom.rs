//! AxROM — mapper 7.
//!
//! AxROM switches PRG-ROM in 32 KB windows: the entire `$8000-$FFFF` range
//! is mapped to a single selectable bank. The bank register also controls
//! single-screen nametable mirroring — bit 4 selects which of the two
//! physical nametable pages is visible at all four NT slots. CHR is a
//! single 8 KB window of either ROM or RAM. There is no PRG-RAM and no
//! IRQ source.
//!
//! Games using AxROM include *Battletoads*, *Paperboy*, *Jeopardy!*, and
//! *Marble Madness*.
//!
//! # Register layout
//!
//! Writes to `$8000-$FFFF` latch the bank register:
//!
//! ```text
//! 7  bit  0
//! ---- ----
//! xxxM xPPP
//!    |  |||
//!    |  +++- Select 32 KB PRG-ROM bank for $8000-$FFFF
//!    +------ Select 1 KB VRAM page for all 4 nametables (single-screen)
//! ```
//!
//! See: https://www.nesdev.org/wiki/AxROM

use super::{Mapper, Mirroring};

/// PRG-ROM bank size (32 KB — AxROM switches the whole `$8000-$FFFF` range).
const PRG_BANK_SIZE: usize = 32 * 1024;
/// CHR bank size (8 KB — AxROM has a single CHR window).
const CHR_SIZE: usize = 8 * 1024;

/// AxROM cartridge state.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Axrom {
    prg_rom: Vec<u8>,
    /// CHR data — ROM when `chr_is_ram` is false, RAM when true.
    chr: Vec<u8>,
    chr_is_ram: bool,
    has_battery: bool,
    /// Currently selected 32 KB PRG bank (bits 0-2 of the register).
    prg_bank: u8,
    /// Current single-screen nametable page (bit 4 of the register): 0 or 1.
    mirror_nt: u8,
}

impl Axrom {
    /// Construct an AxROM mapper from parsed PRG/CHR data.
    ///
    /// `chr_rom` is empty for CHR-RAM carts (iNES `chr_rom_banks == 0`); in
    /// that case an 8 KB RAM buffer is allocated. The initial mirroring is
    /// single-screen NT 0 (the AxROM power-on default).
    pub fn new(
        prg_rom: Vec<u8>,
        chr_rom: Vec<u8>,
        _mirroring: Mirroring,
        has_battery: bool,
    ) -> Self {
        let (chr, chr_is_ram) = if chr_rom.is_empty() {
            (vec![0u8; CHR_SIZE], true)
        } else {
            (chr_rom, false)
        };

        Self {
            prg_rom,
            chr,
            chr_is_ram,
            has_battery,
            prg_bank: 0,
            mirror_nt: 0,
        }
    }

    /// Number of 32 KB PRG banks.
    fn prg_bank_count(&self) -> usize {
        (self.prg_rom.len() / PRG_BANK_SIZE).max(1)
    }
}

impl Mapper for Axrom {
    fn read_prg(&self, addr: u16) -> u8 {
        // No PRG-RAM on AxROM — $6000-$7FFF reads return 0.
        if addr < 0x8000 {
            return 0x00;
        }

        let local = (addr - 0x8000) as usize; // 0..=0x7FFF
        let count = self.prg_bank_count();
        let bank = (self.prg_bank as usize) % count;
        let idx = bank * PRG_BANK_SIZE + local;
        self.prg_rom.get(idx).copied().unwrap_or(0)
    }

    fn write_prg(&mut self, addr: u16, value: u8) {
        // $6000-$7FFF: no PRG-RAM, ignore.
        if addr < 0x8000 {
            return;
        }
        // Bit 4 selects the single-screen nametable page (0 or 1).
        self.mirror_nt = (value >> 4) & 1;
        // Bits 0-2 select the 32 KB PRG bank.
        self.prg_bank = value & 0x07;
    }

    fn read_chr(&self, addr: u16) -> u8 {
        let idx = (addr as usize) % self.chr.len().max(1);
        self.chr.get(idx).copied().unwrap_or(0)
    }

    fn write_chr(&mut self, addr: u16, value: u8) {
        if self.chr_is_ram {
            let idx = (addr as usize) % self.chr.len();
            if let Some(slot) = self.chr.get_mut(idx) {
                *slot = value;
            }
        }
        // CHR-ROM writes are ignored.
    }

    fn mirror_mode(&self) -> Mirroring {
        // AxROM is always single-screen; bit 4 of the bank register selects
        // which nametable page (0 or 1) is visible at all four NT slots.
        Mirroring::SingleScreen(self.mirror_nt)
    }

    fn has_battery(&self) -> bool {
        self.has_battery
    }

    fn save_state(&self) -> super::MapperState {
        super::MapperState::Axrom(self.clone())
    }

    fn restore_state(&mut self, state: super::MapperState) {
        match state {
            super::MapperState::Axrom(m) => *self = m,
            _ => {
                panic!("Axrom::restore_state: expected Axrom variant, got a different mapper type")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build PRG-ROM with `banks` 32 KB banks, each filled with a unique
    /// byte (the bank index).
    fn make_prg(banks: usize) -> Vec<u8> {
        let mut prg = vec![0u8; banks * PRG_BANK_SIZE];
        for (i, b) in prg.iter_mut().enumerate() {
            *b = (i / PRG_BANK_SIZE) as u8;
        }
        prg
    }

    #[test]
    fn default_bank_0_across_whole_window() {
        // 4 banks (128 KB). Bank 0 = 0x00, bank 3 = 0x03.
        let ax = Axrom::new(make_prg(4), vec![], Mirroring::Horizontal, false);
        assert_eq!(ax.read_prg(0x8000), 0x00);
        assert_eq!(ax.read_prg(0xBFFF), 0x00);
        assert_eq!(ax.read_prg(0xC000), 0x00);
        assert_eq!(ax.read_prg(0xFFFF), 0x00);
    }

    #[test]
    fn bank_select_switches_whole_32k_window() {
        let mut ax = Axrom::new(make_prg(4), vec![], Mirroring::Horizontal, false);
        ax.write_prg(0x8000, 0x02);
        assert_eq!(ax.read_prg(0x8000), 0x02);
        assert_eq!(ax.read_prg(0xBFFF), 0x02);
        assert_eq!(ax.read_prg(0xC000), 0x02);
        assert_eq!(ax.read_prg(0xFFFF), 0x02);
    }

    #[test]
    fn bank_select_via_any_address_in_8000_ffff() {
        let mut ax = Axrom::new(make_prg(4), vec![], Mirroring::Horizontal, false);
        // Writing to $FFFF should still latch the bank register.
        ax.write_prg(0xFFFF, 0x01);
        assert_eq!(ax.read_prg(0x8000), 0x01);
    }

    #[test]
    fn bank_number_wraps_within_rom_size() {
        // 2 banks (64 KB). bank=5 → 5 % 2 = 1.
        let mut ax = Axrom::new(make_prg(2), vec![], Mirroring::Horizontal, false);
        ax.write_prg(0x8000, 0x05);
        assert_eq!(ax.read_prg(0x8000), 0x01);
        assert_eq!(ax.read_prg(0xFFFF), 0x01);
    }

    #[test]
    fn bank_select_uses_low_3_bits() {
        // 8 banks (256 KB). Writing 0x0F should select bank 7 (bits 0-2).
        let mut ax = Axrom::new(make_prg(8), vec![], Mirroring::Horizontal, false);
        ax.write_prg(0x8000, 0x0F);
        assert_eq!(ax.read_prg(0x8000), 0x07);
    }

    #[test]
    fn prg_ram_region_reads_zero() {
        let ax = Axrom::new(make_prg(4), vec![], Mirroring::Horizontal, false);
        assert_eq!(ax.read_prg(0x6000), 0x00);
        assert_eq!(ax.read_prg(0x7FFF), 0x00);
    }

    #[test]
    fn prg_ram_region_writes_ignored() {
        let mut ax = Axrom::new(make_prg(4), vec![], Mirroring::Horizontal, false);
        ax.write_prg(0x6000, 0xFF);
        assert_eq!(ax.read_prg(0x6000), 0x00);
        // Bank selection unchanged.
        assert_eq!(ax.read_prg(0x8000), 0x00);
    }

    #[test]
    fn default_mirroring_is_single_screen_nt0() {
        let ax = Axrom::new(make_prg(4), vec![], Mirroring::Horizontal, false);
        assert_eq!(ax.mirror_mode(), Mirroring::SingleScreen(0));
    }

    #[test]
    fn bit4_selects_single_screen_nt1() {
        let mut ax = Axrom::new(make_prg(4), vec![], Mirroring::Horizontal, false);
        // Bit 4 set, bank bits = 0.
        ax.write_prg(0x8000, 0x10);
        assert_eq!(ax.mirror_mode(), Mirroring::SingleScreen(1));
        // Bank still 0.
        assert_eq!(ax.read_prg(0x8000), 0x00);
    }

    #[test]
    fn mirroring_and_bank_independent() {
        let mut ax = Axrom::new(make_prg(4), vec![], Mirroring::Horizontal, false);
        // Bit 4 set + bank 2.
        ax.write_prg(0x8000, 0x12);
        assert_eq!(ax.mirror_mode(), Mirroring::SingleScreen(1));
        assert_eq!(ax.read_prg(0x8000), 0x02);
        // Bit 4 clear + bank 3.
        ax.write_prg(0x8000, 0x03);
        assert_eq!(ax.mirror_mode(), Mirroring::SingleScreen(0));
        assert_eq!(ax.read_prg(0x8000), 0x03);
    }

    #[test]
    fn chr_ram_writes_persist() {
        let mut ax = Axrom::new(make_prg(4), vec![], Mirroring::Vertical, false);
        ax.write_chr(0x0000, 0x42);
        assert_eq!(ax.read_chr(0x0000), 0x42);
    }

    #[test]
    fn chr_rom_writes_ignored() {
        let mut ax = Axrom::new(
            make_prg(4),
            vec![0xAA; CHR_SIZE],
            Mirroring::Vertical,
            false,
        );
        ax.write_chr(0x0000, 0x11);
        assert_eq!(ax.read_chr(0x0000), 0xAA);
    }

    #[test]
    fn battery_flag_reported() {
        let ax = Axrom::new(make_prg(4), vec![], Mirroring::Horizontal, true);
        assert!(ax.has_battery());
    }
}
