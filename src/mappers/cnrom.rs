//! CNROM — mapper 3.
//!
//! CNROM is the CHR counterpart to UxROM: there is no PRG bank switching
//! (PRG-ROM is mapped linearly across `$8000-$FFFF`, with 16 KB mirroring
//! for single-bank carts), but CHR is bank-switched in 8 KB windows. The
//! bank register is written anywhere in `$8000-$FFFF`; the low 2 bits
//! select 1 of 4 CHR banks (32 KB max). There is no PRG-RAM and no IRQ.
//!
//! Games using CNROM include *Arkanoid*, *Cybernoid*, *Solomon's Key*, and
//! *Bomb Jack*.
//!
//! See: https://www.nesdev.org/wiki/CNROM

use super::{Mapper, Mirroring};

/// PRG-ROM bank size (16 KB). CNROM has no PRG banking, but 16 KB carts
/// mirror the single bank into both halves of `$8000-$FFFF`.
const PRG_BANK_SIZE: usize = 16 * 1024;
/// CHR bank size (8 KB — CNROM switches CHR in 8 KB units).
const CHR_BANK_SIZE: usize = 8 * 1024;

/// CNROM cartridge state.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Cnrom {
    prg_rom: Vec<u8>,
    /// CHR data — ROM when `chr_is_ram` is false, RAM when true.
    chr: Vec<u8>,
    chr_is_ram: bool,
    mirroring: Mirroring,
    has_battery: bool,
    /// Currently selected 8 KB CHR bank.
    chr_bank: u8,
}

impl Cnrom {
    /// Construct a CNROM mapper from parsed PRG/CHR data.
    ///
    /// `chr_rom` is empty for CHR-RAM carts (iNES `chr_rom_banks == 0`); in
    /// that case an 8 KB RAM buffer is allocated.
    pub fn new(
        prg_rom: Vec<u8>,
        chr_rom: Vec<u8>,
        mirroring: Mirroring,
        has_battery: bool,
    ) -> Self {
        let (chr, chr_is_ram) = if chr_rom.is_empty() {
            (vec![0u8; CHR_BANK_SIZE], true)
        } else {
            (chr_rom, false)
        };

        Self {
            prg_rom,
            chr,
            chr_is_ram,
            mirroring,
            has_battery,
            chr_bank: 0,
        }
    }

    /// Number of 8 KB CHR banks.
    fn chr_bank_count(&self) -> usize {
        (self.chr.len() / CHR_BANK_SIZE).max(1)
    }

    /// Mask a CPU PRG address into the PRG-ROM index range.
    /// For 16 KB PRG the high bit is dropped (mirror); for 32 KB the
    /// address is taken modulo 32 KB directly.
    fn prg_index(&self, addr: u16) -> usize {
        let local = (addr - 0x8000) as usize;
        let bank_size = self.prg_rom.len();
        if bank_size == 0 {
            return 0;
        }
        local % bank_size
    }
}

impl Mapper for Cnrom {
    fn read_prg(&self, addr: u16) -> u8 {
        // No PRG-RAM on CNROM — $6000-$7FFF reads return 0.
        if addr < 0x8000 {
            return 0x00;
        }
        let idx = self.prg_index(addr);
        self.prg_rom.get(idx).copied().unwrap_or(0)
    }

    fn write_prg(&mut self, addr: u16, value: u8) {
        // $6000-$7FFF: no PRG-RAM, ignore.
        if addr < 0x8000 {
            return;
        }
        // Any write to $8000-$FFFF latches the low bits as the CHR bank
        // number. The register is 2 bits wide (bits 0-1); mask to the
        // actual bank count at read time so sub-4-bank CHR-ROMs work.
        self.chr_bank = value;
    }

    fn read_chr(&self, addr: u16) -> u8 {
        let addr = addr as usize;
        let count = self.chr_bank_count();
        let bank = (self.chr_bank as usize) % count;
        let idx = bank * CHR_BANK_SIZE + (addr & (CHR_BANK_SIZE - 1));
        self.chr.get(idx).copied().unwrap_or(0)
    }

    fn write_chr(&mut self, addr: u16, value: u8) {
        if !self.chr_is_ram {
            return; // CHR-ROM writes ignored.
        }
        let addr = addr as usize;
        let count = self.chr_bank_count();
        let bank = (self.chr_bank as usize) % count;
        let idx = bank * CHR_BANK_SIZE + (addr & (CHR_BANK_SIZE - 1));
        if let Some(slot) = self.chr.get_mut(idx) {
            *slot = value;
        }
    }

    fn mirror_mode(&self) -> Mirroring {
        self.mirroring
    }

    fn has_battery(&self) -> bool {
        self.has_battery
    }

    fn save_state(&self) -> super::MapperState {
        super::MapperState::Cnrom(self.clone())
    }

    fn restore_state(&mut self, state: super::MapperState) {
        match state {
            super::MapperState::Cnrom(m) => *self = m,
            _ => {
                panic!("Cnrom::restore_state: expected Cnrom variant, got a different mapper type")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build PRG-ROM with `banks` 16 KB banks, each filled with a unique
    /// byte (the bank index).
    fn make_prg(banks: usize) -> Vec<u8> {
        let mut prg = vec![0u8; banks * PRG_BANK_SIZE];
        for (i, b) in prg.iter_mut().enumerate() {
            *b = (i / PRG_BANK_SIZE) as u8;
        }
        prg
    }

    /// Build CHR-ROM with `banks` 8 KB banks, each filled with a unique
    /// byte (the bank index).
    fn make_chr(banks: usize) -> Vec<u8> {
        let mut chr = vec![0u8; banks * CHR_BANK_SIZE];
        for (i, b) in chr.iter_mut().enumerate() {
            *b = (i / CHR_BANK_SIZE) as u8;
        }
        chr
    }

    #[test]
    fn prg_32k_reads_linearly() {
        let cn = Cnrom::new(make_prg(2), make_chr(2), Mirroring::Horizontal, false);
        assert_eq!(cn.read_prg(0x8000), 0x00);
        assert_eq!(cn.read_prg(0xBFFF), 0x00);
        assert_eq!(cn.read_prg(0xC000), 0x01);
        assert_eq!(cn.read_prg(0xFFFF), 0x01);
    }

    #[test]
    fn prg_16k_mirrors_high_half() {
        let cn = Cnrom::new(make_prg(1), make_chr(2), Mirroring::Horizontal, false);
        assert_eq!(cn.read_prg(0x8000), 0x00);
        assert_eq!(cn.read_prg(0xC000), 0x00); // mirror of $8000
        assert_eq!(cn.read_prg(0xFFFF), 0x00);
    }

    #[test]
    fn prg_ram_region_reads_zero() {
        let cn = Cnrom::new(make_prg(2), make_chr(2), Mirroring::Horizontal, false);
        assert_eq!(cn.read_prg(0x6000), 0x00);
        assert_eq!(cn.read_prg(0x7FFF), 0x00);
    }

    #[test]
    fn prg_ram_region_writes_ignored() {
        let mut cn = Cnrom::new(make_prg(2), make_chr(2), Mirroring::Horizontal, false);
        cn.write_prg(0x6000, 0xFF);
        assert_eq!(cn.read_prg(0x6000), 0x00);
        // PRG banking unchanged.
        assert_eq!(cn.read_prg(0x8000), 0x00);
    }

    #[test]
    fn chr_bank_select_switches_window() {
        // 4 × 8 KB CHR banks.
        let mut cn = Cnrom::new(make_prg(2), make_chr(4), Mirroring::Horizontal, false);
        // Default chr_bank = 0 → bank 0.
        assert_eq!(cn.read_chr(0x0000), 0x00);
        assert_eq!(cn.read_chr(0x1FFF), 0x00);
        // Select bank 2.
        cn.write_prg(0x8000, 0x02);
        assert_eq!(cn.read_chr(0x0000), 0x02);
        assert_eq!(cn.read_chr(0x1FFF), 0x02);
        // Select bank 3.
        cn.write_prg(0x8000, 0x03);
        assert_eq!(cn.read_chr(0x0000), 0x03);
    }

    #[test]
    fn chr_bank_select_uses_low_bits_only() {
        // 4 × 8 KB CHR banks. Writing 0xFF should select bank 3 (bits 0-1).
        let mut cn = Cnrom::new(make_prg(2), make_chr(4), Mirroring::Horizontal, false);
        cn.write_prg(0x8000, 0xFF);
        assert_eq!(cn.read_chr(0x0000), 0x03);
    }

    #[test]
    fn chr_bank_number_wraps_within_rom_size() {
        // 2 × 8 KB CHR banks. chr_bank=5 → 5 % 2 = 1.
        let mut cn = Cnrom::new(make_prg(2), make_chr(2), Mirroring::Horizontal, false);
        cn.write_prg(0x8000, 0x05);
        assert_eq!(cn.read_chr(0x0000), 0x01);
    }

    #[test]
    fn chr_rom_writes_ignored() {
        let mut cn = Cnrom::new(make_prg(2), make_chr(2), Mirroring::Horizontal, false);
        cn.write_chr(0x0000, 0x11);
        assert_eq!(cn.read_chr(0x0000), 0x00);
    }

    #[test]
    fn chr_ram_writes_persist() {
        let mut cn = Cnrom::new(make_prg(2), vec![], Mirroring::Vertical, false);
        cn.write_chr(0x0000, 0x42);
        assert_eq!(cn.read_chr(0x0000), 0x42);
    }

    #[test]
    fn mirror_mode_reported_correctly() {
        let cn = Cnrom::new(make_prg(2), make_chr(2), Mirroring::Vertical, false);
        assert_eq!(cn.mirror_mode(), Mirroring::Vertical);
    }

    #[test]
    fn mirror_mode_unchanged_by_writes() {
        // CNROM has fixed mirroring — writes must not change it.
        let mut cn = Cnrom::new(make_prg(2), make_chr(2), Mirroring::Horizontal, false);
        cn.write_prg(0x8000, 0x02);
        assert_eq!(cn.mirror_mode(), Mirroring::Horizontal);
    }

    #[test]
    fn battery_flag_reported() {
        let cn = Cnrom::new(make_prg(2), make_chr(2), Mirroring::Horizontal, true);
        assert!(cn.has_battery());
    }
}
