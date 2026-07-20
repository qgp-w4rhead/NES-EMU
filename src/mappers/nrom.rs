//! NROM — mapper 0.
//!
//! The simplest NES board: no bank switching, no extra registers, no IRQ.
//! PRG-ROM is either 16 KB (mirrored into both halves of `$8000-$FFFF`) or
//! 32 KB (mapped linearly across `$8000-$FFFF`). CHR is 8 KB of either
//! ROM (when the iNES header reports `chr_rom_banks >= 1`) or RAM (when
//! `chr_rom_banks == 0`).
//!
//! PRG-RAM at `$6000-$7FFF` is not present on stock NROM boards; reads
//! return `0x00` (open bus filler) and writes are ignored.
//!
//! See: https://www.nesdev.org/wiki/NROM

use super::{Mapper, Mirroring};

/// NROM cartridge state.
pub struct Nrom {
    prg_rom: Vec<u8>,
    /// CHR data — ROM when `chr_is_ram` is false, RAM when true.
    chr: Vec<u8>,
    chr_is_ram: bool,
    mirroring: Mirroring,
    has_battery: bool,
}

impl Nrom {
    /// Construct an NROM mapper from the parsed PRG/CHR data.
    ///
    /// `chr_rom` is empty for CHR-RAM carts (iNES `chr_rom_banks == 0`);
    /// in that case an 8 KB RAM buffer is allocated.
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
            mirroring,
            has_battery,
        }
    }

    /// Mask a CPU PRG address into the PRG-ROM index range.
    ///
    /// For 16 KB PRG the high bit is dropped (the 16 KB bank mirrors at
    /// `$C000`); for 32 KB the address is taken modulo 32 KB directly.
    fn prg_index(&self, addr: u16) -> usize {
        // Addresses passed in are in `$6000..=$FFFF`. PRG-ROM lives at
        // `$8000..=$FFFF`, so subtract `$8000` first. `$6000-$7FFF` (PRG-RAM
        // region) is handled separately in `read_prg` / `write_prg`.
        let local = (addr - 0x8000) as usize;
        let bank_size = self.prg_rom.len();
        if bank_size == 0 {
            return 0;
        }
        local % bank_size
    }
}

impl Mapper for Nrom {
    fn read_prg(&self, addr: u16) -> u8 {
        // PRG-RAM region ($6000-$7FFF) — not present on stock NROM.
        if addr < 0x8000 {
            return 0x00;
        }
        let idx = self.prg_index(addr);
        self.prg_rom.get(idx).copied().unwrap_or(0)
    }

    fn write_prg(&mut self, addr: u16, _value: u8) {
        // NROM has no writable PRG-RAM and no bank-switch registers.
        // Silently ignore writes (including the $6000-$7FFF range).
        let _ = addr;
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

    fn make_prg(size_kb: usize, fill: u8) -> Vec<u8> {
        vec![fill; size_kb * 1024]
    }

    #[test]
    fn nrom_32k_reads_linearly() {
        let prg = make_prg(32, 0xCD);
        let nrom = Nrom::new(prg, vec![0; 8 * 1024], Mirroring::Horizontal, false);
        assert_eq!(nrom.read_prg(0x8000), 0xCD);
        assert_eq!(nrom.read_prg(0xC000), 0xCD);
        assert_eq!(nrom.read_prg(0xFFFF), 0xCD);
    }

    #[test]
    fn nrom_16k_mirrors_high_half() {
        let mut prg = make_prg(16, 0);
        for (i, b) in prg.iter_mut().enumerate() {
            *b = (i & 0xFF) as u8;
        }
        let nrom = Nrom::new(prg, vec![], Mirroring::Horizontal, false);
        assert_eq!(nrom.read_prg(0x8000), 0x00);
        assert_eq!(nrom.read_prg(0xC000), 0x00); // mirror of $8000
        assert_eq!(nrom.read_prg(0xC001), 0x01);
        assert_eq!(nrom.read_prg(0xFFFF), 0xFF);
    }

    #[test]
    fn prg_ram_region_reads_zero() {
        let nrom = Nrom::new(make_prg(16, 0xFF), vec![], Mirroring::Horizontal, false);
        assert_eq!(nrom.read_prg(0x6000), 0x00);
        assert_eq!(nrom.read_prg(0x7FFF), 0x00);
    }

    #[test]
    fn chr_ram_writes_persist() {
        let mut nrom = Nrom::new(make_prg(16, 0), vec![], Mirroring::Vertical, false);
        nrom.write_chr(0x0000, 0x42);
        assert_eq!(nrom.read_chr(0x0000), 0x42);
    }

    #[test]
    fn chr_rom_writes_ignored() {
        let mut nrom = Nrom::new(
            make_prg(16, 0),
            vec![0xAA; 8 * 1024],
            Mirroring::Vertical,
            false,
        );
        nrom.write_chr(0x0000, 0x11);
        assert_eq!(nrom.read_chr(0x0000), 0xAA);
    }

    #[test]
    fn mirror_mode_reported_correctly() {
        let nrom = Nrom::new(make_prg(16, 0), vec![], Mirroring::Vertical, false);
        assert_eq!(nrom.mirror_mode(), Mirroring::Vertical);
    }

    #[test]
    fn battery_flag_reported() {
        let nrom = Nrom::new(make_prg(16, 0), vec![], Mirroring::Horizontal, true);
        assert!(nrom.has_battery());
    }
}
