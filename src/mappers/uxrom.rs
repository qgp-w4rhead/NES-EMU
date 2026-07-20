//! UxROM — mapper 2.
//!
//! UxROM is Nintendo's simplest PRG bank-switching board. The CPU address
//! space `$8000-$BFFF` is a switchable 16 KB PRG-ROM bank, while
//! `$C000-$FFFF` is hard-wired to the last 16 KB bank (so the RESET and
//! interrupt vectors are always reachable). CHR is a single 8 KB window of
//! either ROM or RAM. There is no PRG-RAM and no IRQ source.
//!
//! The bank register is written anywhere in `$8000-$FFFF`; the low bits
//! select the bank (typically bits 0-3, allowing up to 16 banks = 256 KB).
//! Mirroring is fixed by the cartridge solder pads (horizontal or vertical
//! per the iNES header) and cannot be changed at runtime.
//!
//! Games using UxROM include *Castlevania*, *Mega Man*, *Duck Tales*, and
//! *Contra*.
//!
//! See: https://www.nesdev.org/wiki/UxROM

use super::{Mapper, Mirroring};

/// PRG-ROM bank size (16 KB).
const PRG_BANK_SIZE: usize = 16 * 1024;
/// CHR bank size (8 KB — UxROM has a single CHR window).
const CHR_SIZE: usize = 8 * 1024;

/// UxROM cartridge state.
pub struct Uxrom {
    prg_rom: Vec<u8>,
    /// CHR data — ROM when `chr_is_ram` is false, RAM when true.
    chr: Vec<u8>,
    chr_is_ram: bool,
    mirroring: Mirroring,
    has_battery: bool,
    /// Currently selected 16 KB PRG bank at `$8000-$BFFF`.
    bank: u8,
}

impl Uxrom {
    /// Construct a UxROM mapper from parsed PRG/CHR data.
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
            (vec![0u8; CHR_SIZE], true)
        } else {
            (chr_rom, false)
        };

        Self {
            prg_rom,
            chr,
            chr_is_ram,
            mirroring,
            has_battery,
            bank: 0,
        }
    }

    /// Number of 16 KB PRG banks.
    fn prg_bank_count(&self) -> usize {
        (self.prg_rom.len() / PRG_BANK_SIZE).max(1)
    }
}

impl Mapper for Uxrom {
    fn read_prg(&self, addr: u16) -> u8 {
        // No PRG-RAM on UxROM — $6000-$7FFF reads return 0 (open bus filler).
        if addr < 0x8000 {
            return 0x00;
        }

        let local = (addr - 0x8000) as usize; // 0..=0x7FFF
        let offset = local & (PRG_BANK_SIZE - 1);
        let count = self.prg_bank_count();

        if local < PRG_BANK_SIZE {
            // $8000-$BFFF: switchable bank.
            let bank = (self.bank as usize) % count;
            let idx = bank * PRG_BANK_SIZE + offset;
            self.prg_rom.get(idx).copied().unwrap_or(0)
        } else {
            // $C000-$FFFF: fixed last bank.
            let last = count.saturating_sub(1);
            let idx = last * PRG_BANK_SIZE + offset;
            self.prg_rom.get(idx).copied().unwrap_or(0)
        }
    }

    fn write_prg(&mut self, addr: u16, value: u8) {
        // $6000-$7FFF: no PRG-RAM, ignore.
        if addr < 0x8000 {
            return;
        }
        // Any write to $8000-$FFFF latches the low bits as the bank number.
        // The bank register is typically 4 bits wide (bits 0-3); mask the
        // value to the actual bank count at read time so oversized ROMs and
        // sub-16-bank ROMs both work.
        self.bank = value;
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
        self.mirroring
    }

    fn has_battery(&self) -> bool {
        self.has_battery
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build PRG-ROM with `banks` 16 KB banks, each filled with a unique
    /// byte (the bank index) so bank selection is observable.
    fn make_prg(banks: usize) -> Vec<u8> {
        let mut prg = vec![0u8; banks * PRG_BANK_SIZE];
        for (i, b) in prg.iter_mut().enumerate() {
            *b = (i / PRG_BANK_SIZE) as u8;
        }
        prg
    }

    #[test]
    fn default_bank_0_at_8000_last_bank_at_c000() {
        // 4 banks (64 KB). Bank 0 = 0x00, bank 3 = 0x03.
        let ux = Uxrom::new(make_prg(4), vec![], Mirroring::Horizontal, false);
        assert_eq!(ux.read_prg(0x8000), 0x00);
        assert_eq!(ux.read_prg(0xBFFF), 0x00);
        assert_eq!(ux.read_prg(0xC000), 0x03);
        assert_eq!(ux.read_prg(0xFFFF), 0x03);
    }

    #[test]
    fn bank_select_switches_8000_window() {
        let mut ux = Uxrom::new(make_prg(4), vec![], Mirroring::Horizontal, false);
        ux.write_prg(0x8000, 0x02);
        assert_eq!(ux.read_prg(0x8000), 0x02);
        assert_eq!(ux.read_prg(0xBFFF), 0x02);
        // $C000 stays fixed at last bank.
        assert_eq!(ux.read_prg(0xC000), 0x03);
        assert_eq!(ux.read_prg(0xFFFF), 0x03);
    }

    #[test]
    fn bank_select_via_any_address_in_8000_ffff() {
        let mut ux = Uxrom::new(make_prg(4), vec![], Mirroring::Horizontal, false);
        // Writing to $C000 should still latch the bank register.
        ux.write_prg(0xC000, 0x01);
        assert_eq!(ux.read_prg(0x8000), 0x01);
        assert_eq!(ux.read_prg(0xBFFF), 0x01);
    }

    #[test]
    fn bank_number_wraps_within_rom_size() {
        // 2 banks (32 KB). bank=5 → 5 % 2 = 1.
        let mut ux = Uxrom::new(make_prg(2), vec![], Mirroring::Horizontal, false);
        ux.write_prg(0x8000, 0x05);
        assert_eq!(ux.read_prg(0x8000), 0x01);
        // $C000 = last bank (bank 1).
        assert_eq!(ux.read_prg(0xC000), 0x01);
    }

    #[test]
    fn prg_ram_region_reads_zero() {
        let ux = Uxrom::new(make_prg(4), vec![], Mirroring::Horizontal, false);
        assert_eq!(ux.read_prg(0x6000), 0x00);
        assert_eq!(ux.read_prg(0x7FFF), 0x00);
    }

    #[test]
    fn prg_ram_region_writes_ignored() {
        let mut ux = Uxrom::new(make_prg(4), vec![], Mirroring::Horizontal, false);
        ux.write_prg(0x6000, 0xFF);
        // Should not affect bank selection or PRG-ROM reads.
        assert_eq!(ux.read_prg(0x6000), 0x00);
        assert_eq!(ux.read_prg(0x8000), 0x00);
    }

    #[test]
    fn chr_ram_writes_persist() {
        let mut ux = Uxrom::new(make_prg(4), vec![], Mirroring::Vertical, false);
        ux.write_chr(0x0000, 0x42);
        assert_eq!(ux.read_chr(0x0000), 0x42);
    }

    #[test]
    fn chr_rom_writes_ignored() {
        let mut ux = Uxrom::new(
            make_prg(4),
            vec![0xAA; CHR_SIZE],
            Mirroring::Vertical,
            false,
        );
        ux.write_chr(0x0000, 0x11);
        assert_eq!(ux.read_chr(0x0000), 0xAA);
    }

    #[test]
    fn chr_rom_reads_return_data() {
        let mut chr = vec![0u8; CHR_SIZE];
        for (i, b) in chr.iter_mut().enumerate() {
            *b = (i as u8).wrapping_add(0x40);
        }
        let ux = Uxrom::new(make_prg(4), chr, Mirroring::Vertical, false);
        assert_eq!(ux.read_chr(0x0000), 0x40);
        assert_eq!(ux.read_chr(0x0001), 0x41);
    }

    #[test]
    fn mirror_mode_reported_correctly() {
        let ux = Uxrom::new(make_prg(4), vec![], Mirroring::Vertical, false);
        assert_eq!(ux.mirror_mode(), Mirroring::Vertical);
    }

    #[test]
    fn mirror_mode_unchanged_by_writes() {
        // UxROM has fixed mirroring — writes must not change it.
        let mut ux = Uxrom::new(make_prg(4), vec![], Mirroring::Horizontal, false);
        ux.write_prg(0x8000, 0x02);
        assert_eq!(ux.mirror_mode(), Mirroring::Horizontal);
    }

    #[test]
    fn battery_flag_reported() {
        let ux = Uxrom::new(make_prg(4), vec![], Mirroring::Horizontal, true);
        assert!(ux.has_battery());
    }
}
