//! MMC2 — mapper 9.
//!
//! MMC2 is Nintendo's mapper used by *Punch-Out!!*. Its defining feature is
//! **CHR bank latching**: the cartridge exposes four 4 KB CHR bank registers
//! but only two are active at a time. The PPU selects which bank is visible
//! in each 4 KB slot by reading from specific CHR addresses (`$0FD8`/`$0FE8`
//! for the left slot, `$1FD8`/`$1FE8` for the right slot). This lets a single
//! 4 KB CHR window show two different pattern-table halves depending on which
//! tile the PPU is fetching — Punch-Out!! uses this to swap the boxer's
//! large sprite graphics mid-frame without CPU intervention.
//!
//! # Features
//!
//! - **PRG-ROM**: 8 KB switchable bank at `$8000-$9FFF` (register `$A000`),
//!   fixed last 24 KB at `$A000-$FFFF`.
//! - **CHR-ROM**: 4 KB banks with latching (see below).
//! - **PRG-RAM**: 1 KB at `$6000-$7FFF` (mirrored within the 8 KB range),
//!   optionally battery-backed.
//! - **Mirroring**: fixed from the iNES header (horizontal or vertical).
//!
//! # Register layout
//!
//! | Address     | Function                              |
//! |-------------|---------------------------------------|
//! | `$A000`     | PRG bank 0 (`$8000-$9FFF`)            |
//! | `$B000`     | CHR bank 0 (left slot, latch 0)       |
//! | `$B001`     | CHR bank 1 (left slot, latch 1)       |
//! | `$B002`     | CHR bank 2 (right slot, latch 0)      |
//! | `$B003`     | CHR bank 3 (right slot, latch 1)      |
//!
//! The active 4 KB CHR bank for the left slot (`$0000-$0FFF`) is bank 0 or
//! bank 1 depending on the latch state, toggled by PPU reads from `$0FD8`
//! (→ bank 0) or `$0FE8` (→ bank 1). The right slot (`$1000-$1FFF`) uses
//! bank 2 / bank 3, toggled by reads from `$1FD8` / `$1FE8`.
//!
//! See: https://www.nesdev.org/wiki/MMC2

use super::{Mapper, Mirroring};

/// PRG-ROM bank size (8 KB).
const PRG_BANK_SIZE: usize = 8 * 1024;
/// CHR 4 KB bank size (MMC2 switches CHR in 4 KB units).
const CHR_4K_SIZE: usize = 4 * 1024;
/// PRG-RAM size (1 KB — mirrored across the 8 KB `$6000-$7FFF` window).
const PRG_RAM_SIZE: usize = 1024;

/// CHR latch trigger address ranges. A PPU read from any address in
/// `$0FD8-$0FDF` selects left-bank 0; `$0FE8-$0FEF` selects left-bank 1.
/// Similarly `$1FD8-$1FDF` → right-bank 0, `$1FE8-$1FEF` → right-bank 1.
/// See: https://www.nesdev.org/wiki/MMC2
const LATCH_LEFT_BANK0_LO: u16 = 0x0FD8;
const LATCH_LEFT_BANK0_HI: u16 = 0x0FDF;
const LATCH_LEFT_BANK1_LO: u16 = 0x0FE8;
const LATCH_LEFT_BANK1_HI: u16 = 0x0FEF;
const LATCH_RIGHT_BANK0_LO: u16 = 0x1FD8;
const LATCH_RIGHT_BANK0_HI: u16 = 0x1FDF;
const LATCH_RIGHT_BANK1_LO: u16 = 0x1FE8;
const LATCH_RIGHT_BANK1_HI: u16 = 0x1FEF;

/// MMC2 cartridge state.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Mmc2 {
    prg_rom: Vec<u8>,
    /// CHR data — ROM when `chr_is_ram` is false, RAM when true. MMC2 carts
    /// are CHR-ROM in practice, but CHR-RAM is supported for completeness.
    chr: Vec<u8>,
    chr_is_ram: bool,
    /// 1 KB PRG-RAM (mirrored across `$6000-$7FFF`).
    prg_ram: Vec<u8>,
    has_battery: bool,

    /// PRG bank register (selects the 8 KB bank at `$8000-$9FFF`).
    prg_bank: u8,

    /// Four 4 KB CHR bank registers (R0-R3).
    chr_banks: [u8; 4],
    /// Latch state for the left 4 KB slot (`$0000-$0FFF`): 0 → R0, 1 → R1.
    latch_left: u8,
    /// Latch state for the right 4 KB slot (`$1000-$1FFF`): 0 → R2, 1 → R3.
    latch_right: u8,

    /// Fixed mirroring from the iNES header.
    mirroring: Mirroring,
}

impl Mmc2 {
    /// Construct an MMC2 mapper from parsed PRG/CHR data.
    ///
    /// `chr_rom` is empty for CHR-RAM carts (iNES `chr_rom_banks == 0`); in
    /// that case an 8 KB RAM buffer is allocated. The initial latches are 0
    /// (R0 / R2 active), matching the power-on state.
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
            prg_bank: 0,
            chr_banks: [0u8; 4],
            latch_left: 0,
            latch_right: 0,
            mirroring,
        }
    }

    /// Number of 8 KB PRG banks.
    fn prg_bank_count(&self) -> usize {
        (self.prg_rom.len() / PRG_BANK_SIZE).max(1)
    }

    /// Number of 4 KB CHR banks.
    fn chr_bank_count(&self) -> usize {
        (self.chr.len() / CHR_4K_SIZE).max(1)
    }

    /// Read a PRG-ROM byte from a given 8 KB bank + offset.
    fn prg_read_bank(&self, bank: usize, offset: usize) -> u8 {
        let count = self.prg_bank_count();
        let bank = bank % count;
        let idx = bank * PRG_BANK_SIZE + offset;
        self.prg_rom.get(idx).copied().unwrap_or(0)
    }

    /// The active 4 KB CHR bank register for the left slot (`$0000-$0FFF`).
    fn left_bank(&self) -> u8 {
        if self.latch_left == 0 {
            self.chr_banks[0]
        } else {
            self.chr_banks[1]
        }
    }

    /// The active 4 KB CHR bank register for the right slot (`$1000-$1FFF`).
    fn right_bank(&self) -> u8 {
        if self.latch_right == 0 {
            self.chr_banks[2]
        } else {
            self.chr_banks[3]
        }
    }

    /// Update the CHR latches based on a PPU read address. Reads from
    /// `$0FD8-$0FDF`/`$0FE8-$0FEF` toggle the left latch; reads from
    /// `$1FD8-$1FDF`/`$1FE8-$1FEF` toggle the right latch. Other addresses
    /// leave the latches unchanged.
    fn update_latches(&mut self, addr: u16) {
        if (LATCH_LEFT_BANK0_LO..=LATCH_LEFT_BANK0_HI).contains(&addr) {
            self.latch_left = 0;
        } else if (LATCH_LEFT_BANK1_LO..=LATCH_LEFT_BANK1_HI).contains(&addr) {
            self.latch_left = 1;
        } else if (LATCH_RIGHT_BANK0_LO..=LATCH_RIGHT_BANK0_HI).contains(&addr) {
            self.latch_right = 0;
        } else if (LATCH_RIGHT_BANK1_LO..=LATCH_RIGHT_BANK1_HI).contains(&addr) {
            self.latch_right = 1;
        }
    }

    /// Tick the CHR latches in response to a PPU read from `addr`. This is
    /// the read side-effect that selects which 4 KB CHR bank is active in
    /// each slot. The bus calls this from its CHR-read wrapper so that the
    /// `&self` `read_chr` can stay side-effect-free for unit tests.
    ///
    /// Returns `true` if a latch changed.
    pub fn tick_latch(&mut self, addr: u16) -> bool {
        let prev_left = self.latch_left;
        let prev_right = self.latch_right;
        self.update_latches(addr);
        prev_left != self.latch_left || prev_right != self.latch_right
    }
}

impl Mapper for Mmc2 {
    fn read_prg(&self, addr: u16) -> u8 {
        // PRG-RAM at $6000-$7FFF (1 KB, mirrored).
        if (0x6000..0x8000).contains(&addr) {
            let idx = (addr as usize - 0x6000) & (PRG_RAM_SIZE - 1);
            return self.prg_ram.get(idx).copied().unwrap_or(0);
        }

        // $8000-$FFFF: 8 KB switchable at $8000, fixed last 24 KB at $A000.
        let local = (addr - 0x8000) as usize; // 0..=0x7FFF
        let count = self.prg_bank_count();

        if local < PRG_BANK_SIZE {
            // $8000-$9FFF: switchable bank.
            let bank = (self.prg_bank as usize) % count;
            return self.prg_read_bank(bank, local);
        }

        // $A000-$FFFF: fixed last 24 KB = last 3 × 8 KB banks.
        let fixed_offset = local - PRG_BANK_SIZE; // 0..=0x5FFF
        let fixed_bank_base = count.saturating_sub(3); // first of the last 3 banks
        let bank = fixed_bank_base + (fixed_offset / PRG_BANK_SIZE);
        let offset = fixed_offset & (PRG_BANK_SIZE - 1);
        self.prg_read_bank(bank, offset)
    }

    fn write_prg(&mut self, addr: u16, value: u8) {
        // PRG-RAM at $6000-$7FFF (1 KB, mirrored).
        if (0x6000..0x8000).contains(&addr) {
            let idx = (addr as usize - 0x6000) & (PRG_RAM_SIZE - 1);
            if let Some(slot) = self.prg_ram.get_mut(idx) {
                *slot = value;
            }
            return;
        }

        // Register writes by address.
        match addr {
            0xA000 => self.prg_bank = value & 0x0F,
            0xB000 => self.chr_banks[0] = value & 0x3F,
            0xB001 => self.chr_banks[1] = value & 0x3F,
            0xB002 => self.chr_banks[2] = value & 0x3F,
            0xB003 => self.chr_banks[3] = value & 0x3F,
            _ => {}
        }
    }

    fn read_chr(&self, addr: u16) -> u8 {
        let bank = if (addr as usize) < CHR_4K_SIZE {
            self.left_bank()
        } else {
            self.right_bank()
        };
        let count = self.chr_bank_count();
        let bank = (bank as usize) % count;
        let offset = (addr as usize) & (CHR_4K_SIZE - 1);
        let idx = bank * CHR_4K_SIZE + offset;
        self.chr.get(idx).copied().unwrap_or(0)
    }

    fn read_chr_latched(&mut self, addr: u16) -> u8 {
        // Update the CHR bank latches based on the read address, then
        // return the byte from the now-active bank. This is the MMC2's
        // defining side-effect: PPU pattern-fetches at `$0FD8`/`$0FE8`/
        // `$1FD8`/`$1FE8` (and their 8-byte ranges) select which 4 KB
        // CHR bank is visible in each slot.
        self.update_latches(addr);
        self.read_chr(addr)
    }

    fn write_chr(&mut self, addr: u16, value: u8) {
        if !self.chr_is_ram {
            return;
        }
        let bank = if (addr as usize) < CHR_4K_SIZE {
            self.left_bank()
        } else {
            self.right_bank()
        };
        let count = self.chr_bank_count();
        let bank = (bank as usize) % count;
        let offset = (addr as usize) & (CHR_4K_SIZE - 1);
        let idx = bank * CHR_4K_SIZE + offset;
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
        super::MapperState::Mmc2(self.clone())
    }

    fn restore_state(&mut self, state: super::MapperState) {
        match state {
            super::MapperState::Mmc2(m) => *self = m,
            _ => panic!("Mmc2::restore_state: expected Mmc2 variant, got a different mapper type"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build PRG-ROM with `banks` 8 KB banks, each filled with a unique
    /// byte (the bank index).
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

    // ---- PRG banking ---------------------------------------------------

    #[test]
    fn prg_default_bank_0_at_8000_fixed_last_24k_at_a000() {
        // 4 × 8KB banks (32 KB). Default R0=0 → $8000 = bank 0.
        // $A000-$FFFF = fixed last 24KB = banks 1,2,3.
        let mmc = Mmc2::new(make_prg(4), make_chr(4), Mirroring::Horizontal, false);
        assert_eq!(mmc.read_prg(0x8000), 0x00); // switchable bank 0
        assert_eq!(mmc.read_prg(0x9FFF), 0x00);
        assert_eq!(mmc.read_prg(0xA000), 0x01); // fixed bank 1
        assert_eq!(mmc.read_prg(0xBFFF), 0x01);
        assert_eq!(mmc.read_prg(0xC000), 0x02); // fixed bank 2
        assert_eq!(mmc.read_prg(0xDFFF), 0x02);
        assert_eq!(mmc.read_prg(0xE000), 0x03); // fixed bank 3
        assert_eq!(mmc.read_prg(0xFFFF), 0x03);
    }

    #[test]
    fn prg_bank_select_switches_8000_window() {
        let mut mmc = Mmc2::new(make_prg(4), make_chr(4), Mirroring::Horizontal, false);
        mmc.write_prg(0xA000, 0x02); // R0 = bank 2
        assert_eq!(mmc.read_prg(0x8000), 0x02);
        assert_eq!(mmc.read_prg(0x9FFF), 0x02);
        // Fixed region unchanged.
        assert_eq!(mmc.read_prg(0xA000), 0x01);
        assert_eq!(mmc.read_prg(0xE000), 0x03);
    }

    #[test]
    fn prg_bank_number_wraps_within_rom_size() {
        // 4 banks. R0 = 6 → 6 % 4 = 2.
        let mut mmc = Mmc2::new(make_prg(4), make_chr(4), Mirroring::Horizontal, false);
        mmc.write_prg(0xA000, 0x06);
        assert_eq!(mmc.read_prg(0x8000), 0x02);
    }

    #[test]
    fn prg_bank_register_masked_to_low_4_bits() {
        let mut mmc = Mmc2::new(make_prg(4), make_chr(4), Mirroring::Horizontal, false);
        mmc.write_prg(0xA000, 0xFF); // & 0x0F = 0x0F → 15 % 4 = 3
        assert_eq!(mmc.read_prg(0x8000), 0x03);
    }

    #[test]
    fn prg_register_only_responds_to_a000() {
        let mut mmc = Mmc2::new(make_prg(4), make_chr(4), Mirroring::Horizontal, false);
        // Writes to other addresses in $8000-$FFFF should not change PRG bank.
        mmc.write_prg(0x8000, 0x02);
        assert_eq!(mmc.read_prg(0x8000), 0x00); // still bank 0
        mmc.write_prg(0xA000, 0x02);
        assert_eq!(mmc.read_prg(0x8000), 0x02); // now bank 2
    }

    // ---- PRG-RAM -------------------------------------------------------

    #[test]
    fn prg_ram_writes_and_reads_at_6000() {
        let mut mmc = Mmc2::new(make_prg(4), make_chr(4), Mirroring::Horizontal, false);
        mmc.write_prg(0x6000, 0x42);
        mmc.write_prg(0x7FFF, 0x99);
        assert_eq!(mmc.read_prg(0x6000), 0x42);
        assert_eq!(mmc.read_prg(0x7FFF), 0x99);
    }

    #[test]
    fn prg_ram_is_1kb_mirrored_across_6000_7fff() {
        // 1KB PRG-RAM mirrors 8× across the 8KB $6000-$7FFF window.
        let mut mmc = Mmc2::new(make_prg(4), make_chr(4), Mirroring::Horizontal, false);
        mmc.write_prg(0x6000, 0x77);
        assert_eq!(mmc.read_prg(0x6400), 0x77); // mirror (0x400 = 1KB)
        assert_eq!(mmc.read_prg(0x7C00), 0x77); // mirror
    }

    // ---- CHR latching --------------------------------------------------

    #[test]
    fn chr_default_left_bank_0_right_bank_0() {
        // Default latches: left=0 → R0, right=0 → R2. Both R0 and R2
        // default to 0, so both slots read CHR bank 0.
        let mmc = Mmc2::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        assert_eq!(mmc.read_chr(0x0000), 0x00); // R0 = bank 0
        assert_eq!(mmc.read_chr(0x0FFF), 0x00);
        assert_eq!(mmc.read_chr(0x1000), 0x00); // R2 = bank 0
        assert_eq!(mmc.read_chr(0x1FFF), 0x00);
    }

    #[test]
    fn chr_bank_registers_select_4kb_window() {
        let mut mmc = Mmc2::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        mmc.write_prg(0xB000, 0x04); // R0 = 4
        mmc.write_prg(0xB002, 0x06); // R2 = 6
        assert_eq!(mmc.read_chr(0x0000), 0x04);
        assert_eq!(mmc.read_chr(0x0FFF), 0x04);
        assert_eq!(mmc.read_chr(0x1000), 0x06);
        assert_eq!(mmc.read_chr(0x1FFF), 0x06);
    }

    #[test]
    fn chr_latch_left_toggles_between_r0_and_r1() {
        let mut mmc = Mmc2::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        mmc.write_prg(0xB000, 0x01); // R0 = 1
        mmc.write_prg(0xB001, 0x05); // R1 = 5
                                     // Default latch_left = 0 → R0 = bank 1.
        assert_eq!(mmc.read_chr(0x0000), 0x01);
        // Read $0FE8 → latch_left = 1 → R1 = bank 5.
        assert!(mmc.tick_latch(0x0FE8));
        assert_eq!(mmc.read_chr(0x0000), 0x05);
        // Read $0FD8 → latch_left = 0 → R0 = bank 1.
        assert!(mmc.tick_latch(0x0FD8));
        assert_eq!(mmc.read_chr(0x0000), 0x01);
    }

    #[test]
    fn chr_latch_right_toggles_between_r2_and_r3() {
        let mut mmc = Mmc2::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        mmc.write_prg(0xB002, 0x02); // R2 = 2
        mmc.write_prg(0xB003, 0x07); // R3 = 7
                                     // Default latch_right = 0 → R2 = bank 2.
        assert_eq!(mmc.read_chr(0x1000), 0x02);
        // Read $1FE8 → latch_right = 1 → R3 = bank 7.
        assert!(mmc.tick_latch(0x1FE8));
        assert_eq!(mmc.read_chr(0x1000), 0x07);
        // Read $1FD8 → latch_right = 0 → R2 = bank 2.
        assert!(mmc.tick_latch(0x1FD8));
        assert_eq!(mmc.read_chr(0x1000), 0x02);
    }

    #[test]
    fn chr_latch_addresses_outside_triggers_do_not_toggle() {
        let mut mmc = Mmc2::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        mmc.write_prg(0xB000, 0x01);
        mmc.write_prg(0xB001, 0x05);
        // Random address — no toggle.
        assert!(!mmc.tick_latch(0x0800));
        assert_eq!(mmc.latch_left, 0);
        // $0FD9 (one off) — no toggle.
        assert!(!mmc.tick_latch(0x0FD9));
        assert_eq!(mmc.latch_left, 0);
    }

    #[test]
    fn chr_bank_register_masked_to_low_5_bits() {
        let mut mmc = Mmc2::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        mmc.write_prg(0xB000, 0xFF); // & 0x1F = 0x1F → 31 % 8 = 7
        assert_eq!(mmc.read_chr(0x0000), 0x07);
    }

    #[test]
    fn chr_rom_writes_ignored() {
        let mut mmc = Mmc2::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        mmc.write_chr(0x0000, 0x42);
        assert_eq!(mmc.read_chr(0x0000), 0x00); // unchanged
    }

    #[test]
    fn chr_ram_writes_persist() {
        let mut mmc = Mmc2::new(make_prg(4), vec![], Mirroring::Horizontal, false);
        mmc.write_chr(0x0000, 0x42);
        assert_eq!(mmc.read_chr(0x0000), 0x42);
    }

    // ---- Mirroring / battery -------------------------------------------

    #[test]
    fn mirroring_is_fixed_from_header() {
        let mmc = Mmc2::new(make_prg(4), make_chr(4), Mirroring::Vertical, false);
        assert_eq!(mmc.mirror_mode(), Mirroring::Vertical);
    }

    #[test]
    fn battery_flag_reported() {
        let mmc = Mmc2::new(make_prg(4), make_chr(4), Mirroring::Horizontal, true);
        assert!(mmc.has_battery());
    }

    #[test]
    fn battery_sram_round_trips() {
        let mut mmc = Mmc2::new(make_prg(4), make_chr(4), Mirroring::Horizontal, true);
        mmc.write_prg(0x6000, 0xAB);
        let saved = mmc.battery_sram().expect("battery");
        let mut mmc2 = Mmc2::new(make_prg(4), make_chr(4), Mirroring::Horizontal, true);
        mmc2.load_battery_sram(&saved);
        assert_eq!(mmc2.read_prg(0x6000), 0xAB);
    }
}
