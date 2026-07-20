//! MMC1 — mapper 1.
//!
//! MMC1 is Nintendo's first bank-switching mapper. It uses a serial 5-bit
//! shift register to load four internal registers that control PRG banking
//! (16 KB or 32 KB mode), CHR banking (4 KB or 8 KB mode), and nametable
//! mirroring. It also provides 8 KB of PRG-RAM at `$6000-$7FFF` (optionally
//! battery-backed).
//!
//! Games using MMC1 include *The Legend of Zelda*, *Metroid*, *Castlevania*,
//! and *Kid Icarus*.
//!
//! # Register layout
//!
//! The 5-bit shift register is filled serially: each write to `$8000-$FFFF`
//! shifts the data bit 0 into the register (first write → bit 0, fifth write
//! → bit 4). After the fifth write, the accumulated 5 bits are copied to one
//! of four internal registers selected by the write address, and the shift
//! register is cleared. A write with bit 7 set resets the shift register and
//! forces PRG bank mode 3 (fix last bank at `$C000`).
//!
//! | Register | Address range   | Function                          |
//! |----------|-----------------|-----------------------------------|
//! | 0        | `$8000-$9FFF`   | Control (mirroring, PRG/CHR mode) |
//! | 1        | `$A000-$BFFF`   | CHR bank 0                        |
//! | 2        | `$C000-$DFFF`   | CHR bank 1                        |
//! | 3        | `$E000-$FFFF`   | PRG bank                           |
//!
//! See: https://www.nesdev.org/wiki/MMC1

use super::{Mapper, Mirroring};

/// PRG-ROM bank size (16 KB).
const PRG_BANK_SIZE: usize = 16 * 1024;
/// CHR-ROM/RAM bank size for 4 KB mode.
const CHR_4K_SIZE: usize = 4 * 1024;
/// CHR-ROM/RAM bank size for 8 KB mode.
const CHR_8K_SIZE: usize = 8 * 1024;
/// PRG-RAM size (8 KB at `$6000-$7FFF`).
const PRG_RAM_SIZE: usize = 8 * 1024;

/// Reset value for the control register's PRG mode bits (mode 3 = fix last
/// bank at `$C000`, switch 16 KB at `$8000`). Combined with the header
/// mirroring bits at construction.
const CONTROL_PRG_MODE_3: u8 = 0b01100;

/// MMC1 cartridge state.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Mmc1 {
    prg_rom: Vec<u8>,
    /// CHR data — ROM when `chr_is_ram` is false, RAM when true.
    chr: Vec<u8>,
    chr_is_ram: bool,
    /// 8 KB PRG-RAM at `$6000-$7FFF`.
    prg_ram: Vec<u8>,
    has_battery: bool,

    /// 5-bit serial shift register accumulator.
    shift_reg: u8,
    /// Number of bits written to the shift register since last reset (0..=5).
    shift_count: u8,

    // Internal 5-bit registers.
    /// Register 0 (`$8000`): mirroring + PRG/CHR mode.
    control: u8,
    /// Register 1 (`$A000`): CHR bank 0.
    chr_bank_0: u8,
    /// Register 2 (`$C000`): CHR bank 1.
    chr_bank_1: u8,
    /// Register 3 (`$E000`): PRG bank.
    prg_bank: u8,
}

impl Mmc1 {
    /// Construct an MMC1 mapper from parsed PRG/CHR data.
    ///
    /// The control register is initialised so that PRG mode = 3 (fix last
    /// bank at `$C000`, switch 16 KB at `$8000`) and CHR mode = 0 (8 KB),
    /// with the mirroring bits derived from the iNES header. This matches
    /// the reset state described on the NESdev wiki.
    pub fn new(
        prg_rom: Vec<u8>,
        chr_rom: Vec<u8>,
        mirroring: Mirroring,
        has_battery: bool,
    ) -> Self {
        let (chr, chr_is_ram) = if chr_rom.is_empty() {
            (vec![0u8; CHR_8K_SIZE], true)
        } else {
            (chr_rom, false)
        };

        // Encode the header mirroring into the control register's bits 0-1.
        let mirr_bits = match mirroring {
            Mirroring::SingleScreen(0) => 0b00,
            Mirroring::SingleScreen(_) => 0b01, // 1ScB for any non-zero NT
            Mirroring::Vertical => 0b10,
            // Horizontal, FourScreen (not applicable to MMC1) → horizontal.
            Mirroring::Horizontal | Mirroring::FourScreen => 0b11,
        };
        let control = CONTROL_PRG_MODE_3 | mirr_bits;

        Self {
            prg_rom,
            chr,
            chr_is_ram,
            prg_ram: vec![0u8; PRG_RAM_SIZE],
            has_battery,
            shift_reg: 0,
            shift_count: 0,
            control,
            chr_bank_0: 0,
            chr_bank_1: 0,
            prg_bank: 0,
        }
    }

    /// Number of 16 KB PRG banks.
    fn prg_bank_count(&self) -> usize {
        (self.prg_rom.len() / PRG_BANK_SIZE).max(1)
    }

    /// Number of 4 KB CHR banks.
    fn chr_bank_count(&self) -> usize {
        (self.chr.len() / CHR_4K_SIZE).max(1)
    }

    /// PRG bank mode (bits 2-3 of the control register).
    fn prg_mode(&self) -> u8 {
        (self.control >> 2) & 0b11
    }

    /// CHR bank mode (bit 4 of the control register): 0 = 8 KB, 1 = 4 KB.
    fn chr_mode(&self) -> u8 {
        (self.control >> 4) & 1
    }

    /// Read a PRG-ROM byte from a given bank + offset.
    fn prg_read_bank(&self, bank: usize, offset: usize) -> u8 {
        let count = self.prg_bank_count();
        let bank = bank % count;
        let idx = bank * PRG_BANK_SIZE + offset;
        self.prg_rom.get(idx).copied().unwrap_or(0)
    }

    /// Write to the serial port, handling shift-register accumulation and
    /// register commit. See module docs for the protocol.
    fn serial_write(&mut self, addr: u16, value: u8) {
        // Bit 7 set: reset shift register and force PRG mode 3.
        if value & 0x80 != 0 {
            self.shift_reg = 0;
            self.shift_count = 0;
            // Preserve mirroring (bits 0-1) and CHR mode (bit 4); set PRG
            // mode bits (2-3) to 11 (mode 3).
            self.control = (self.control & 0b10011) | CONTROL_PRG_MODE_3;
            return;
        }

        // Shift bit 0 into the register: first write → bit 0, fifth → bit 4.
        self.shift_reg = (self.shift_reg >> 1) | ((value & 1) << 4);
        self.shift_count += 1;

        if self.shift_count == 5 {
            // Select the target register from bits 13-14 of the address.
            let reg = ((addr >> 13) & 0b11) as u8;
            match reg {
                0 => self.control = self.shift_reg,
                1 => self.chr_bank_0 = self.shift_reg,
                2 => self.chr_bank_1 = self.shift_reg,
                3 => self.prg_bank = self.shift_reg,
                _ => {}
            }
            self.shift_reg = 0;
            self.shift_count = 0;
        }
    }
}

impl Mapper for Mmc1 {
    fn read_prg(&self, addr: u16) -> u8 {
        // PRG-RAM at $6000-$7FFF.
        if (0x6000..0x8000).contains(&addr) {
            let idx = (addr as usize - 0x6000) & (PRG_RAM_SIZE - 1);
            return self.prg_ram.get(idx).copied().unwrap_or(0);
        }

        let local = (addr - 0x8000) as usize; // 0..=0x7FFF
        let in_low = local < PRG_BANK_SIZE; // $8000-$BFFF
        let offset = local & (PRG_BANK_SIZE - 1);

        match self.prg_mode() {
            0 | 1 => {
                // 32 KB mode: select a 32 KB window using prg_bank & 0x0E.
                let bank = (self.prg_bank & 0x0E) as usize;
                self.prg_read_bank(bank, local)
            }
            2 => {
                // Fix first bank at $8000, switch 16 KB at $C000.
                if in_low {
                    self.prg_read_bank(0, offset)
                } else {
                    let bank = (self.prg_bank & 0x0F) as usize;
                    self.prg_read_bank(bank, offset)
                }
            }
            _ => {
                // Mode 3 (default): fix last bank at $C000, switch 16 KB at $8000.
                if in_low {
                    let bank = (self.prg_bank & 0x0F) as usize;
                    self.prg_read_bank(bank, offset)
                } else {
                    let last = self.prg_bank_count().saturating_sub(1);
                    self.prg_read_bank(last, offset)
                }
            }
        }
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

        // $8000-$FFFF: serial port.
        self.serial_write(addr, value);
    }

    fn read_chr(&self, addr: u16) -> u8 {
        let addr = addr as usize;
        let count = self.chr_bank_count();

        if self.chr_mode() == 0 {
            // 8 KB mode: chr_bank_0 & 0x1E selects the 8 KB bank.
            let bank = ((self.chr_bank_0 & 0x1E) as usize) % count;
            let idx = bank * CHR_4K_SIZE + (addr & (CHR_8K_SIZE - 1));
            self.chr.get(idx).copied().unwrap_or(0)
        } else {
            // 4 KB mode: separate banks for $0000-$0FFF and $1000-$1FFF.
            if addr < CHR_4K_SIZE {
                let bank = ((self.chr_bank_0 & 0x1F) as usize) % count;
                let idx = bank * CHR_4K_SIZE + addr;
                self.chr.get(idx).copied().unwrap_or(0)
            } else {
                let bank = ((self.chr_bank_1 & 0x1F) as usize) % count;
                let idx = bank * CHR_4K_SIZE + (addr - CHR_4K_SIZE);
                self.chr.get(idx).copied().unwrap_or(0)
            }
        }
    }

    fn write_chr(&mut self, addr: u16, value: u8) {
        if !self.chr_is_ram {
            return; // CHR-ROM writes ignored.
        }

        let addr = addr as usize;
        let count = self.chr_bank_count();

        if self.chr_mode() == 0 {
            let bank = ((self.chr_bank_0 & 0x1E) as usize) % count;
            let idx = bank * CHR_4K_SIZE + (addr & (CHR_8K_SIZE - 1));
            if let Some(slot) = self.chr.get_mut(idx) {
                *slot = value;
            }
        } else if addr < CHR_4K_SIZE {
            let bank = ((self.chr_bank_0 & 0x1F) as usize) % count;
            let idx = bank * CHR_4K_SIZE + addr;
            if let Some(slot) = self.chr.get_mut(idx) {
                *slot = value;
            }
        } else {
            let bank = ((self.chr_bank_1 & 0x1F) as usize) % count;
            let idx = bank * CHR_4K_SIZE + (addr - CHR_4K_SIZE);
            if let Some(slot) = self.chr.get_mut(idx) {
                *slot = value;
            }
        }
    }

    fn mirror_mode(&self) -> Mirroring {
        match self.control & 0b11 {
            0 => Mirroring::SingleScreen(0), // 1ScA
            1 => Mirroring::SingleScreen(1), // 1ScB
            2 => Mirroring::Vertical,
            _ => Mirroring::Horizontal,
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

    fn save_state(&self) -> super::MapperState {
        super::MapperState::Mmc1(self.clone())
    }

    fn restore_state(&mut self, state: super::MapperState) {
        match state {
            super::MapperState::Mmc1(m) => *self = m,
            _ => panic!("Mmc1::restore_state: expected Mmc1 variant, got a different mapper type"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build PRG-ROM with `banks` 16 KB banks, each filled with a unique
    /// byte so bank selection is observable.
    fn make_prg(banks: usize) -> Vec<u8> {
        let mut prg = vec![0u8; banks * PRG_BANK_SIZE];
        for (i, b) in prg.iter_mut().enumerate() {
            *b = (i / PRG_BANK_SIZE) as u8;
        }
        prg
    }

    /// Build CHR-ROM with `banks` 4 KB banks, each filled with a unique byte.
    fn make_chr(banks: usize) -> Vec<u8> {
        let mut chr = vec![0u8; banks * CHR_4K_SIZE];
        for (i, b) in chr.iter_mut().enumerate() {
            *b = (i / CHR_4K_SIZE) as u8;
        }
        chr
    }

    /// Write a 5-bit value to an MMC1 register at `addr` by serially
    /// shifting each bit (LSB first) across 5 writes.
    fn write_reg(mmc: &mut Mmc1, addr: u16, value: u8) {
        for i in 0..5 {
            mmc.write_prg(addr, (value >> i) & 1);
        }
    }

    // ---- Shift register / serial port --------------------------------

    #[test]
    fn shift_register_accumulates_5_bits_lsb_first() {
        let mut mmc = Mmc1::new(make_prg(4), make_chr(2), Mirroring::Horizontal, false);
        // Write bits 1,0,1,1,0 (LSB first) to register 3 ($E000).
        // Expected 5-bit value: bit0=1, bit1=0, bit2=1, bit3=1, bit4=0 = 0b01101 = 0x0D
        mmc.write_prg(0xE000, 1);
        mmc.write_prg(0xE000, 0);
        mmc.write_prg(0xE000, 1);
        mmc.write_prg(0xE000, 1);
        mmc.write_prg(0xE000, 0);
        assert_eq!(mmc.prg_bank, 0x0D);
        // Shift register should be cleared after commit.
        assert_eq!(mmc.shift_count, 0);
        assert_eq!(mmc.shift_reg, 0);
    }

    #[test]
    fn shift_register_routes_to_correct_register_by_address() {
        let mut mmc = Mmc1::new(make_prg(4), make_chr(2), Mirroring::Horizontal, false);

        // Write 0b11111 (all 1s) to register 0 ($8000).
        for _ in 0..5 {
            mmc.write_prg(0x8000, 1);
        }
        assert_eq!(mmc.control, 0x1F);

        // Write 0b00000 to register 1 ($A000).
        for _ in 0..5 {
            mmc.write_prg(0xA000, 0);
        }
        assert_eq!(mmc.chr_bank_0, 0x00);

        // Write 0b10101 to register 2 ($C000).
        mmc.write_prg(0xC000, 1);
        mmc.write_prg(0xC000, 0);
        mmc.write_prg(0xC000, 1);
        mmc.write_prg(0xC000, 0);
        mmc.write_prg(0xC000, 1);
        assert_eq!(mmc.chr_bank_1, 0b10101);

        // Write 0b01010 to register 3 ($E000).
        mmc.write_prg(0xE000, 0);
        mmc.write_prg(0xE000, 1);
        mmc.write_prg(0xE000, 0);
        mmc.write_prg(0xE000, 1);
        mmc.write_prg(0xE000, 0);
        assert_eq!(mmc.prg_bank, 0b01010);
    }

    #[test]
    fn bit7_set_resets_shift_register_and_prg_mode() {
        let mut mmc = Mmc1::new(make_prg(4), make_chr(2), Mirroring::Horizontal, false);
        // Accumulate 3 bits, then reset.
        mmc.write_prg(0xE000, 1);
        mmc.write_prg(0xE000, 1);
        mmc.write_prg(0xE000, 1);
        assert_eq!(mmc.shift_count, 3);
        // Write with bit 7 set → reset.
        mmc.write_prg(0xE000, 0x80);
        assert_eq!(mmc.shift_count, 0);
        assert_eq!(mmc.shift_reg, 0);
        // PRG mode should be 3 (bits 2-3 = 11).
        assert_eq!(mmc.prg_mode(), 3);
    }

    #[test]
    fn bit7_reset_preserves_mirroring_and_chr_mode() {
        let mut mmc = Mmc1::new(make_prg(4), make_chr(2), Mirroring::Vertical, false);
        // Set CHR mode to 1 (4 KB) and mirroring to horizontal via control reg.
        // control = 0b1_11_11 = 0x1F (CHR 4KB, PRG mode 3, horizontal).
        write_reg(&mut mmc, 0x8000, 0x1F);
        assert_eq!(mmc.control, 0x1F);
        assert_eq!(mmc.chr_mode(), 1);

        // Now write bit 7 set → should preserve CHR mode and mirroring.
        mmc.write_prg(0x8000, 0x80);
        assert_eq!(mmc.chr_mode(), 1);
        assert_eq!(mmc.mirror_mode(), Mirroring::Horizontal);
        // PRG mode forced to 3.
        assert_eq!(mmc.prg_mode(), 3);
    }

    #[test]
    fn only_bit0_is_shifted_high_bits_ignored() {
        let mut mmc = Mmc1::new(make_prg(4), make_chr(2), Mirroring::Horizontal, false);
        // Write 0xFE (bit 0 = 0) five times → register should be 0.
        for _ in 0..5 {
            mmc.write_prg(0xE000, 0xFE);
        }
        assert_eq!(mmc.prg_bank, 0);
        // Write 0x01 (bit 0 = 1) five times → register should be 0x1F.
        for _ in 0..5 {
            mmc.write_prg(0xE000, 0x01);
        }
        assert_eq!(mmc.prg_bank, 0x1F);
    }

    // ---- PRG banking --------------------------------------------------

    #[test]
    fn prg_mode_3_fixes_last_bank_at_c000() {
        // 4 banks (64 KB). Bank 0 = 0x00, bank 1 = 0x01, etc.
        let mut mmc = Mmc1::new(make_prg(4), make_chr(2), Mirroring::Horizontal, false);
        // Default: PRG mode 3. Select switchable bank = 1 at $8000.
        write_reg(&mut mmc, 0xE000, 0x01);
        // $8000-$BFFF = bank 1 (fill 0x01).
        assert_eq!(mmc.read_prg(0x8000), 0x01);
        assert_eq!(mmc.read_prg(0xBFFF), 0x01);
        // $C000-$FFFF = fixed last bank (bank 3, fill 0x03).
        assert_eq!(mmc.read_prg(0xC000), 0x03);
        assert_eq!(mmc.read_prg(0xFFFF), 0x03);
    }

    #[test]
    fn prg_mode_2_fixes_first_bank_at_8000() {
        let mut mmc = Mmc1::new(make_prg(4), make_chr(2), Mirroring::Horizontal, false);
        // Set PRG mode 2: control bits 2-3 = 10, mirroring = horizontal (11).
        // control = 0b0_10_11 = 0x0B.
        write_reg(&mut mmc, 0x8000, 0x0B);
        assert_eq!(mmc.prg_mode(), 2);

        // Select switchable bank = 2 at $C000.
        write_reg(&mut mmc, 0xE000, 0x02);
        // $8000-$BFFF = fixed bank 0 (fill 0x00).
        assert_eq!(mmc.read_prg(0x8000), 0x00);
        assert_eq!(mmc.read_prg(0xBFFF), 0x00);
        // $C000-$FFFF = bank 2 (fill 0x02).
        assert_eq!(mmc.read_prg(0xC000), 0x02);
        assert_eq!(mmc.read_prg(0xFFFF), 0x02);
    }

    #[test]
    fn prg_mode_0_swaps_32kb_window() {
        // 4 banks (64 KB). 32 KB mode uses prg_bank & 0x0E.
        let mut mmc = Mmc1::new(make_prg(4), make_chr(2), Mirroring::Horizontal, false);
        // Set PRG mode 0: control bits 2-3 = 00, mirroring = horizontal (11).
        // control = 0b0_00_11 = 0x03.
        write_reg(&mut mmc, 0x8000, 0x03);
        assert_eq!(mmc.prg_mode(), 0);

        // prg_bank = 2 → 32 KB window = banks 2&3 (prg_bank & 0x0E = 2).
        write_reg(&mut mmc, 0xE000, 0x02);
        // $8000 = bank 2 (0x02), $C000 = bank 3 (0x03).
        assert_eq!(mmc.read_prg(0x8000), 0x02);
        assert_eq!(mmc.read_prg(0xBFFF), 0x02);
        assert_eq!(mmc.read_prg(0xC000), 0x03);
        assert_eq!(mmc.read_prg(0xFFFF), 0x03);
    }

    #[test]
    fn prg_bank_number_wraps_within_rom_size() {
        // 2 banks (32 KB). In mode 3, prg_bank & 0x0F is masked to valid range.
        let mut mmc = Mmc1::new(make_prg(2), make_chr(2), Mirroring::Horizontal, false);
        // prg_bank = 5 → 5 % 2 = 1 → bank 1 at $8000.
        write_reg(&mut mmc, 0xE000, 0x05);
        // $8000 = bank 1 (0x01), $C000 = last bank (bank 1, 0x01).
        assert_eq!(mmc.read_prg(0x8000), 0x01);
        assert_eq!(mmc.read_prg(0xC000), 0x01);
    }

    // ---- PRG-RAM ------------------------------------------------------

    #[test]
    fn prg_ram_writes_and_reads_at_6000() {
        let mut mmc = Mmc1::new(make_prg(4), make_chr(2), Mirroring::Horizontal, false);
        mmc.write_prg(0x6000, 0x42);
        mmc.write_prg(0x7FFF, 0x99);
        assert_eq!(mmc.read_prg(0x6000), 0x42);
        assert_eq!(mmc.read_prg(0x7FFF), 0x99);
        // $6000 mirrors within the 8 KB range — writing $6000+0x2000=$8000
        // would hit PRG-ROM, not RAM. Verify $6000-$7FFF is exactly 8 KB.
        mmc.write_prg(0x6000, 0x11);
        assert_eq!(mmc.read_prg(0x6000), 0x11);
    }

    #[test]
    fn prg_ram_does_not_interfere_with_serial_writes() {
        let mut mmc = Mmc1::new(make_prg(4), make_chr(2), Mirroring::Horizontal, false);
        // Writing to $6000 should NOT shift the serial register.
        mmc.write_prg(0x6000, 0x01);
        assert_eq!(mmc.shift_count, 0);
        // Writing to $8000 should shift.
        mmc.write_prg(0x8000, 0x01);
        assert_eq!(mmc.shift_count, 1);
    }

    // ---- CHR banking --------------------------------------------------

    #[test]
    fn chr_8kb_mode_selects_8kb_window() {
        // 4 CHR banks (16 KB). 8 KB mode uses chr_bank_0 & 0x1E.
        let mut mmc = Mmc1::new(make_prg(4), make_chr(4), Mirroring::Horizontal, false);
        // Default CHR mode = 0 (8 KB). Set chr_bank_0 = 2 → window = banks 2&3.
        write_reg(&mut mmc, 0xA000, 0x02);
        // $0000 = bank 2 (0x02), $1000 = bank 3 (0x03).
        assert_eq!(mmc.read_chr(0x0000), 0x02);
        assert_eq!(mmc.read_chr(0x0FFF), 0x02);
        assert_eq!(mmc.read_chr(0x1000), 0x03);
        assert_eq!(mmc.read_chr(0x1FFF), 0x03);
    }

    #[test]
    fn chr_4kb_mode_selects_independent_banks() {
        let mut mmc = Mmc1::new(make_prg(4), make_chr(4), Mirroring::Horizontal, false);
        // Set CHR mode 1 (4 KB): control bit 4 = 1, PRG mode 3, horizontal.
        // control = 0b1_11_11 = 0x1F.
        write_reg(&mut mmc, 0x8000, 0x1F);
        assert_eq!(mmc.chr_mode(), 1);

        // chr_bank_0 = 1 → $0000-$0FFF = bank 1 (0x01).
        write_reg(&mut mmc, 0xA000, 0x01);
        // chr_bank_1 = 3 → $1000-$1FFF = bank 3 (0x03).
        write_reg(&mut mmc, 0xC000, 0x03);
        assert_eq!(mmc.read_chr(0x0000), 0x01);
        assert_eq!(mmc.read_chr(0x0FFF), 0x01);
        assert_eq!(mmc.read_chr(0x1000), 0x03);
        assert_eq!(mmc.read_chr(0x1FFF), 0x03);
    }

    #[test]
    fn chr_ram_writes_persist_in_4kb_mode() {
        let mut mmc = Mmc1::new(make_prg(4), vec![], Mirroring::Horizontal, false);
        // CHR-RAM (8 KB = 2 × 4 KB banks). Enable 4 KB mode.
        // control = 0b1_11_11 = 0x1F.
        write_reg(&mut mmc, 0x8000, 0x1F);
        // chr_bank_0 = 0, chr_bank_1 = 1.
        write_reg(&mut mmc, 0xA000, 0x00);
        write_reg(&mut mmc, 0xC000, 0x01);
        mmc.write_chr(0x0000, 0xAA);
        mmc.write_chr(0x1000, 0xBB);
        assert_eq!(mmc.read_chr(0x0000), 0xAA);
        assert_eq!(mmc.read_chr(0x1000), 0xBB);
    }

    #[test]
    fn chr_rom_writes_ignored() {
        let mut mmc = Mmc1::new(make_prg(4), make_chr(4), Mirroring::Horizontal, false);
        mmc.write_chr(0x0000, 0xFF);
        // Bank 0 fill = 0x00, write should be ignored.
        assert_eq!(mmc.read_chr(0x0000), 0x00);
    }

    // ---- Mirroring ----------------------------------------------------

    #[test]
    fn mirroring_single_screen_a() {
        let mut mmc = Mmc1::new(make_prg(4), make_chr(2), Mirroring::Horizontal, false);
        // control bits 0-1 = 00 (1ScA). PRG mode 3, CHR 8KB.
        // control = 0b0_11_00 = 0x0C.
        write_reg(&mut mmc, 0x8000, 0x0C);
        assert_eq!(mmc.mirror_mode(), Mirroring::SingleScreen(0));
    }

    #[test]
    fn mirroring_single_screen_b() {
        let mut mmc = Mmc1::new(make_prg(4), make_chr(2), Mirroring::Horizontal, false);
        // control bits 0-1 = 01 (1ScB). control = 0b0_11_01 = 0x0D.
        write_reg(&mut mmc, 0x8000, 0x0D);
        assert_eq!(mmc.mirror_mode(), Mirroring::SingleScreen(1));
    }

    #[test]
    fn mirroring_vertical() {
        let mut mmc = Mmc1::new(make_prg(4), make_chr(2), Mirroring::Horizontal, false);
        // control bits 0-1 = 10 (vertical). control = 0b0_11_10 = 0x0E.
        write_reg(&mut mmc, 0x8000, 0x0E);
        assert_eq!(mmc.mirror_mode(), Mirroring::Vertical);
    }

    #[test]
    fn mirroring_horizontal() {
        let mut mmc = Mmc1::new(make_prg(4), make_chr(2), Mirroring::Horizontal, false);
        // control bits 0-1 = 11 (horizontal). control = 0b0_11_11 = 0x0F.
        write_reg(&mut mmc, 0x8000, 0x0F);
        assert_eq!(mmc.mirror_mode(), Mirroring::Horizontal);
    }

    #[test]
    fn header_mirroring_initializes_control() {
        // Vertical header → control bits 0-1 = 10.
        let mmc = Mmc1::new(make_prg(4), make_chr(2), Mirroring::Vertical, false);
        assert_eq!(mmc.mirror_mode(), Mirroring::Vertical);
        // Horizontal header → control bits 0-1 = 11.
        let mmc2 = Mmc1::new(make_prg(4), make_chr(2), Mirroring::Horizontal, false);
        assert_eq!(mmc2.mirror_mode(), Mirroring::Horizontal);
        // PRG mode should be 3 in both cases.
        assert_eq!(mmc.prg_mode(), 3);
        assert_eq!(mmc2.prg_mode(), 3);
    }

    // ---- Battery ------------------------------------------------------

    #[test]
    fn battery_flag_reported() {
        let mmc = Mmc1::new(make_prg(4), make_chr(2), Mirroring::Horizontal, true);
        assert!(mmc.has_battery());
        let mmc2 = Mmc1::new(make_prg(4), make_chr(2), Mirroring::Horizontal, false);
        assert!(!mmc2.has_battery());
    }

    // ---- Reset behavior -----------------------------------------------

    #[test]
    fn reset_via_bit7_restores_prg_mode_3_default() {
        let mut mmc = Mmc1::new(make_prg(4), make_chr(2), Mirroring::Horizontal, false);
        // Switch to PRG mode 0: control = 0b0_00_11 = 0x03.
        write_reg(&mut mmc, 0x8000, 0x03);
        assert_eq!(mmc.prg_mode(), 0);
        // Reset via bit 7.
        mmc.write_prg(0x8000, 0x80);
        assert_eq!(mmc.prg_mode(), 3);
    }

    #[test]
    fn incomplete_shift_does_not_commit() {
        let mut mmc = Mmc1::new(make_prg(4), make_chr(2), Mirroring::Horizontal, false);
        // Only 4 writes — should not commit to any register.
        mmc.write_prg(0xE000, 1);
        mmc.write_prg(0xE000, 1);
        mmc.write_prg(0xE000, 1);
        mmc.write_prg(0xE000, 1);
        assert_eq!(mmc.prg_bank, 0); // unchanged from init
        assert_eq!(mmc.shift_count, 4);
        // 5th write commits.
        mmc.write_prg(0xE000, 1);
        assert_eq!(mmc.prg_bank, 0x1F);
        assert_eq!(mmc.shift_count, 0);
    }
}
