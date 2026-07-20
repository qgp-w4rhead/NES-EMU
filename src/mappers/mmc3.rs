//! MMC3 — mapper 4.
//!
//! MMC3 is Nintendo's most popular mapper, used by hundreds of games
//! including *Mega Man 2-6*, *Super Mario Bros 3*, *Castlevania III*, and
//! *Kirby's Adventure*. It provides:
//!
//! - **PRG-ROM banking**: 8 KB switchable banks (two switchable + one fixed
//!   16 KB window at the top), with a mode bit selecting which half is
//!   switchable.
//! - **CHR-ROM/RAM banking**: 1 KB and 2 KB banks (six 1 KB-equivalent
//!   registers), with a mode bit swapping the R0/R1 (2 KB) and R2-R5 (1 KB)
//!   groups between the left and right pattern-table halves.
//! - **8 KB PRG-RAM** at `$6000-$7FFF`, optionally battery-backed, gated by
//!   `$A001`.
//! - **Horizontal/vertical mirroring** switchable via `$A000`.
//! - **IRQ counter** clocked by the PPU A12 rising edge, used for raster
//!   effects (split-screen, status bars). The bus clocks this once per
//!   scanline during rendering at the approximate A12-rise cycle.
//!
//! # Register layout
//!
//! Writes to `$8000-$FFFF` are decoded by the high address bits and bit 0:
//!
//! | Address       | Bit 0 | Function                          |
//! |---------------|-------|-----------------------------------|
//! | `$8000-$9FFF` | 0     | Bank select (reg index, PRG/CHR mode) |
//! | `$8000-$9FFF` | 1     | Bank data (write to selected reg) |
//! | `$A000-$BFFF` | 0     | Mirroring (bit 0)                 |
//! | `$A000-$BFFF` | 1     | PRG-RAM enable / write protect    |
//! | `$C000-$DFFF` | 0     | IRQ latch (reload value)          |
//! | `$C000-$DFFF` | 1     | IRQ reload (force reload next clock) |
//! | `$E000-$FFFF` | 0     | IRQ disable + clear               |
//! | `$E000-$FFFF` | 1     | IRQ enable                        |
//!
//! The 8 bank registers (R0-R7) are: R0-R5 for CHR (R0,R1 = 2 KB; R2-R5 =
//! 1 KB), R6-R7 for PRG (8 KB each).
//!
//! See: https://www.nesdev.org/wiki/MMC3

use super::{Mapper, Mirroring};

/// PRG-ROM bank size (8 KB).
const PRG_BANK_SIZE: usize = 8 * 1024;
/// CHR 1 KB bank size (the finest CHR granularity MMC3 supports).
const CHR_1K_SIZE: usize = 1024;
/// CHR 2 KB bank size (for R0/R1).
const CHR_2K_SIZE: usize = 2 * 1024;
/// PRG-RAM size (8 KB at `$6000-$7FFF`).
const PRG_RAM_SIZE: usize = 8 * 1024;

/// MMC3 cartridge state.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Mmc3 {
    prg_rom: Vec<u8>,
    /// CHR data — ROM when `chr_is_ram` is false, RAM when true.
    chr: Vec<u8>,
    chr_is_ram: bool,
    /// 8 KB PRG-RAM at `$6000-$7FFF`.
    prg_ram: Vec<u8>,
    has_battery: bool,

    /// `$8000` bank-select register: bits 0-2 = target register index for
    /// the next `$8001` write, bit 6 = PRG mode, bit 7 = CHR mode.
    bank_select: u8,
    /// Bank register values R0-R7 (written via `$8001`).
    bank_values: [u8; 8],

    /// `$A000` mirroring: 0 = vertical, 1 = horizontal.
    mirroring: Mirroring,

    /// `$A001` PRG-RAM enable (bit 7).
    prg_ram_enable: bool,
    /// `$A001` write-protect (bit 6): when set, PRG-RAM writes are ignored.
    prg_ram_write_protect: bool,

    /// `$C000` IRQ latch — the reload value for the IRQ counter.
    irq_latch: u8,
    /// Running IRQ counter, decremented on each A12 rising edge.
    irq_counter: u8,
    /// Set by a `$C001` write; forces the counter to reload from the latch
    /// on the next clock.
    irq_reload_flag: bool,
    /// `$E001` enables / `$E000` disables IRQ generation.
    irq_enable: bool,
    /// Asserted when the counter triggers; stays asserted until `$E000`
    /// clears it. The bus/emulator polls this for the CPU IRQ line.
    irq_pending: bool,
}

impl Mmc3 {
    /// Construct an MMC3 mapper from parsed PRG/CHR data.
    ///
    /// The bank registers are zero-initialised, so the initial PRG mapping
    /// places bank 0 at `$8000` (mode 0) and the fixed last 16 KB at
    /// `$C000`. The initial mirroring comes from the iNES header. PRG-RAM
    /// is disabled by default (games enable it via `$A001`).
    pub fn new(
        prg_rom: Vec<u8>,
        chr_rom: Vec<u8>,
        mirroring: Mirroring,
        has_battery: bool,
    ) -> Self {
        let (chr, chr_is_ram) = if chr_rom.is_empty() {
            (vec![0u8; CHR_2K_SIZE * 4], true) // 8 KB CHR-RAM default
        } else {
            (chr_rom, false)
        };

        Self {
            prg_rom,
            chr,
            chr_is_ram,
            prg_ram: vec![0u8; PRG_RAM_SIZE],
            has_battery,
            bank_select: 0,
            bank_values: [0u8; 8],
            mirroring,
            prg_ram_enable: false,
            prg_ram_write_protect: false,
            irq_latch: 0,
            irq_counter: 0,
            irq_reload_flag: false,
            irq_enable: false,
            irq_pending: false,
        }
    }

    /// Number of 8 KB PRG banks.
    fn prg_bank_count(&self) -> usize {
        (self.prg_rom.len() / PRG_BANK_SIZE).max(1)
    }

    /// Number of 1 KB CHR banks.
    fn chr_1k_count(&self) -> usize {
        (self.chr.len() / CHR_1K_SIZE).max(1)
    }

    /// PRG mode (bit 6 of `$8000`): 0 = R6 at `$8000`, 1 = R6 at `$C000`.
    fn prg_mode(&self) -> u8 {
        (self.bank_select >> 6) & 1
    }

    /// CHR mode (bit 7 of `$8000`): 0 = R0/R1 at `$0000`, 1 = R0/R1 at `$1000`.
    fn chr_mode(&self) -> u8 {
        (self.bank_select >> 7) & 1
    }

    /// Read a PRG-ROM byte from a given 8 KB bank + offset.
    fn prg_read_bank(&self, bank: usize, offset: usize) -> u8 {
        let count = self.prg_bank_count();
        let bank = bank % count;
        let idx = bank * PRG_BANK_SIZE + offset;
        self.prg_rom.get(idx).copied().unwrap_or(0)
    }

    /// Compute the 1 KB CHR bank number for a given 1 KB slot (0..=7),
    /// applying the current CHR mode and R0-R5 register values.
    ///
    /// R0/R1 select 2 KB banks (LSB dropped → `& 0xFE`), so they fill two
    /// consecutive 1 KB slots. R2-R5 select individual 1 KB banks.
    fn chr_bank_for_slot(&self, slot: usize) -> usize {
        let r = &self.bank_values;
        match (self.chr_mode(), slot) {
            // CHR mode 0: R0(2KB)@$0000, R1(2KB)@$0800, R2-5(1KB)@$1000+
            (0, 0) => (r[0] & 0xFE) as usize,
            (0, 1) => (r[0] & 0xFE) as usize + 1,
            (0, 2) => (r[1] & 0xFE) as usize,
            (0, 3) => (r[1] & 0xFE) as usize + 1,
            (0, 4) => r[2] as usize,
            (0, 5) => r[3] as usize,
            (0, 6) => r[4] as usize,
            (0, 7) => r[5] as usize,
            // CHR mode 1: R2-5(1KB)@$0000, R0(2KB)@$1000, R1(2KB)@$1800
            (1, 0) => r[2] as usize,
            (1, 1) => r[3] as usize,
            (1, 2) => r[4] as usize,
            (1, 3) => r[5] as usize,
            (1, 4) => (r[0] & 0xFE) as usize,
            (1, 5) => (r[0] & 0xFE) as usize + 1,
            (1, 6) => (r[1] & 0xFE) as usize,
            (1, 7) => (r[1] & 0xFE) as usize + 1,
            _ => 0,
        }
    }

    /// Read/write a CHR byte at `addr` through the bank-mapping logic.
    fn chr_index(&self, addr: u16) -> usize {
        let addr = addr as usize;
        let slot = addr / CHR_1K_SIZE; // 0..=7
        let offset = addr & (CHR_1K_SIZE - 1);
        let bank = self.chr_bank_for_slot(slot);
        let count = self.chr_1k_count();
        (bank % count) * CHR_1K_SIZE + offset
    }
}

impl Mapper for Mmc3 {
    fn read_prg(&self, addr: u16) -> u8 {
        // PRG-RAM at $6000-$7FFF.
        if (0x6000..0x8000).contains(&addr) {
            if self.prg_ram_enable {
                let idx = (addr as usize - 0x6000) & (PRG_RAM_SIZE - 1);
                return self.prg_ram.get(idx).copied().unwrap_or(0);
            }
            return 0; // PRG-RAM disabled — open bus (approximated as 0).
        }

        // $8000-$FFFF: 4 × 8 KB slots.
        let local = (addr - 0x8000) as usize; // 0..=0x7FFF
        let slot = local / PRG_BANK_SIZE; // 0..=3
        let offset = local & (PRG_BANK_SIZE - 1);

        let count = self.prg_bank_count();
        let last = count.saturating_sub(1);
        let second_last = count.saturating_sub(2);

        let bank = match (slot, self.prg_mode()) {
            (0, 0) => (self.bank_values[6] as usize) % count, // R6
            (0, 1) => second_last,                            // fixed 2nd-last
            (1, _) => (self.bank_values[7] as usize) % count, // R7
            (2, 0) => second_last,                            // fixed 2nd-last
            (2, 1) => (self.bank_values[6] as usize) % count, // R6
            (3, _) => last,                                   // fixed last
            _ => 0,
        };

        self.prg_read_bank(bank, offset)
    }

    fn write_prg(&mut self, addr: u16, value: u8) {
        // PRG-RAM at $6000-$7FFF.
        if (0x6000..0x8000).contains(&addr) {
            if self.prg_ram_enable && !self.prg_ram_write_protect {
                let idx = (addr as usize - 0x6000) & (PRG_RAM_SIZE - 1);
                if let Some(slot) = self.prg_ram.get_mut(idx) {
                    *slot = value;
                }
            }
            return;
        }

        // $8000-$FFFF: MMC3 registers, decoded by high bits + bit 0.
        match addr {
            0x8000..=0x9FFF => {
                if addr & 1 == 0 {
                    // Bank select: bits 0-2 = reg index, bit 6 = PRG mode,
                    // bit 7 = CHR mode.
                    self.bank_select = value;
                } else {
                    // Bank data: write to the selected register.
                    let reg = (self.bank_select & 0x07) as usize;
                    self.bank_values[reg] = value;
                }
            }
            0xA000..=0xBFFF => {
                if addr & 1 == 0 {
                    // Mirroring: 0 = vertical, 1 = horizontal.
                    self.mirroring = if value & 1 == 0 {
                        Mirroring::Vertical
                    } else {
                        Mirroring::Horizontal
                    };
                } else {
                    // PRG-RAM enable (bit 7) + write protect (bit 6).
                    self.prg_ram_enable = (value & 0x80) != 0;
                    self.prg_ram_write_protect = (value & 0x40) != 0;
                }
            }
            0xC000..=0xDFFF => {
                if addr & 1 == 0 {
                    // IRQ latch (reload value).
                    self.irq_latch = value;
                } else {
                    // IRQ reload: force reload on next clock.
                    self.irq_reload_flag = true;
                }
            }
            0xE000..=0xFFFF => {
                if addr & 1 == 0 {
                    // IRQ disable + clear pending.
                    self.irq_enable = false;
                    self.irq_pending = false;
                } else {
                    // IRQ enable.
                    self.irq_enable = true;
                }
            }
            _ => {}
        }
    }

    fn read_chr(&self, addr: u16) -> u8 {
        let idx = self.chr_index(addr);
        self.chr.get(idx).copied().unwrap_or(0)
    }

    fn write_chr(&mut self, addr: u16, value: u8) {
        if !self.chr_is_ram {
            return; // CHR-ROM writes ignored.
        }
        let idx = self.chr_index(addr);
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

    fn irq_pending(&self) -> bool {
        self.irq_pending
    }

    /// Clock the IRQ counter by one A12 rising edge.
    ///
    /// If the reload flag is set, the counter is loaded from the latch and
    /// the flag is cleared. Otherwise, if the counter is already zero, it
    /// reloads from the latch and asserts the IRQ (if enabled). Otherwise
    /// the counter is decremented.
    ///
    /// See: https://www.nesdev.org/wiki/MMC3#IRQ
    fn clock_irq(&mut self) {
        if self.irq_reload_flag {
            self.irq_counter = self.irq_latch;
            self.irq_reload_flag = false;
        } else if self.irq_counter == 0 {
            self.irq_counter = self.irq_latch;
            if self.irq_enable {
                self.irq_pending = true;
            }
        } else {
            self.irq_counter -= 1;
        }
    }

    fn save_state(&self) -> super::MapperState {
        super::MapperState::Mmc3(self.clone())
    }

    fn restore_state(&mut self, state: super::MapperState) {
        match state {
            super::MapperState::Mmc3(m) => *self = m,
            _ => panic!("Mmc3::restore_state: expected Mmc3 variant, got a different mapper type"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build PRG-ROM with `banks` 8 KB banks, each filled with a unique
    /// byte (the bank index) so bank selection is observable.
    fn make_prg(banks: usize) -> Vec<u8> {
        let mut prg = vec![0u8; banks * PRG_BANK_SIZE];
        for (i, b) in prg.iter_mut().enumerate() {
            *b = (i / PRG_BANK_SIZE) as u8;
        }
        prg
    }

    /// Build CHR-ROM with `kb` 1 KB banks, each filled with a unique byte.
    fn make_chr(kb: usize) -> Vec<u8> {
        let mut chr = vec![0u8; kb * CHR_1K_SIZE];
        for (i, b) in chr.iter_mut().enumerate() {
            *b = (i / CHR_1K_SIZE) as u8;
        }
        chr
    }

    /// Write to an MMC3 register via the mapper's `write_prg` using the
    /// standard address decoding.
    fn write_reg(mmc: &mut Mmc3, addr: u16, value: u8) {
        mmc.write_prg(addr, value);
    }

    /// Select bank register `reg` (0-7) and write `value` to it, preserving
    /// the PRG/CHR mode bits (6 and 7) of the bank-select register.
    fn write_bank(mmc: &mut Mmc3, reg: u8, value: u8) {
        let mode_bits = mmc.bank_select & 0xC0;
        write_reg(mmc, 0x8000, mode_bits | (reg & 0x07));
        write_reg(mmc, 0x8001, value);
    }

    // ---- Register write decoding ----------------------------------------

    #[test]
    fn bank_select_stores_reg_index_and_mode_bits() {
        let mut mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        // $8000 bit layout: bit 7 = CHR mode, bit 6 = PRG mode,
        // bits 0-2 = register index. 0xC6 = CHR mode 1, PRG mode 1, reg 6.
        write_reg(&mut mmc, 0x8000, 0xC6);
        assert_eq!(mmc.bank_select, 0xC6);
        assert_eq!(mmc.prg_mode(), 1);
        assert_eq!(mmc.chr_mode(), 1);
        assert_eq!(mmc.bank_select & 0x07, 6);
    }

    #[test]
    fn bank_data_writes_to_selected_register() {
        let mut mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        write_bank(&mut mmc, 6, 0x02);
        write_bank(&mut mmc, 7, 0x03);
        assert_eq!(mmc.bank_values[6], 0x02);
        assert_eq!(mmc.bank_values[7], 0x03);
    }

    #[test]
    fn even_odd_address_selects_bank_select_vs_data() {
        let mut mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        // $8000 (even) = bank select → reg index 5.
        write_reg(&mut mmc, 0x8000, 0x05);
        // $8001 (odd) = bank data → writes to reg 5.
        write_reg(&mut mmc, 0x8001, 0x42);
        assert_eq!(mmc.bank_values[5], 0x42);
        // $9FFE (even, same range) = bank select.
        write_reg(&mut mmc, 0x9FFE, 0x07);
        // $9FFF (odd) = bank data → writes to reg 7.
        write_reg(&mut mmc, 0x9FFF, 0x99);
        assert_eq!(mmc.bank_values[7], 0x99);
    }

    // ---- PRG banking ---------------------------------------------------

    #[test]
    fn prg_mode_0_r6_at_8000_fixed_last_at_c000() {
        // 4 × 8KB banks (32 KB). Mode 0: R6@$8000, R7@$A000, fixed last@$C000+.
        let mut mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        // Default mode 0. Set R6 = 1 → bank 1 at $8000.
        write_bank(&mut mmc, 6, 0x01);
        // R7 = 2 → bank 2 at $A000.
        write_bank(&mut mmc, 7, 0x02);
        assert_eq!(mmc.read_prg(0x8000), 0x01); // R6 = bank 1
        assert_eq!(mmc.read_prg(0x9FFF), 0x01);
        assert_eq!(mmc.read_prg(0xA000), 0x02); // R7 = bank 2
        assert_eq!(mmc.read_prg(0xBFFF), 0x02);
        // $C000-$FFFF = fixed last 16KB = banks 2 and 3.
        assert_eq!(mmc.read_prg(0xC000), 0x02); // 2nd-last bank
        assert_eq!(mmc.read_prg(0xDFFF), 0x02);
        assert_eq!(mmc.read_prg(0xE000), 0x03); // last bank
        assert_eq!(mmc.read_prg(0xFFFF), 0x03);
    }

    #[test]
    fn prg_mode_1_r6_at_c000_fixed_2nd_last_at_8000() {
        let mut mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        // Set PRG mode 1: bit 6 of $8000 = 1. Reg index 0 (don't care).
        write_reg(&mut mmc, 0x8000, 0b0100_0000);
        assert_eq!(mmc.prg_mode(), 1);
        // R6 = 1 → bank 1 at $C000.
        write_bank(&mut mmc, 6, 0x01);
        // R7 = 2 → bank 2 at $A000.
        write_bank(&mut mmc, 7, 0x02);
        // $8000 = fixed 2nd-last (bank 2).
        assert_eq!(mmc.read_prg(0x8000), 0x02);
        assert_eq!(mmc.read_prg(0x9FFF), 0x02);
        // $A000 = R7 = bank 2.
        assert_eq!(mmc.read_prg(0xA000), 0x02);
        assert_eq!(mmc.read_prg(0xBFFF), 0x02);
        // $C000 = R6 = bank 1.
        assert_eq!(mmc.read_prg(0xC000), 0x01);
        assert_eq!(mmc.read_prg(0xDFFF), 0x01);
        // $E000 = fixed last (bank 3).
        assert_eq!(mmc.read_prg(0xE000), 0x03);
        assert_eq!(mmc.read_prg(0xFFFF), 0x03);
    }

    #[test]
    fn prg_bank_number_wraps_within_rom_size() {
        // 2 × 8KB banks (16 KB). R6 = 5 → 5 % 2 = 1.
        let mut mmc = Mmc3::new(make_prg(2), make_chr(8), Mirroring::Horizontal, false);
        write_bank(&mut mmc, 6, 0x05);
        assert_eq!(mmc.read_prg(0x8000), 0x01); // 5 % 2 = 1
    }

    #[test]
    fn prg_default_maps_bank_0_at_8000() {
        let mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        // No bank writes: R6=R7=0 → bank 0 at $8000 and $A000.
        assert_eq!(mmc.read_prg(0x8000), 0x00);
        assert_eq!(mmc.read_prg(0xA000), 0x00);
        // Fixed last 16KB at $C000: banks 2 and 3.
        assert_eq!(mmc.read_prg(0xC000), 0x02);
        assert_eq!(mmc.read_prg(0xE000), 0x03);
    }

    // ---- CHR banking ---------------------------------------------------

    #[test]
    fn chr_mode_0_r0_r1_2kb_at_left_r2_r5_1kb_at_right() {
        // 8 × 1KB CHR banks (8 KB).
        let mut mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        // Default CHR mode 0. R0 = 2 → 2KB bank at $0000 (1KB banks 2&3).
        write_bank(&mut mmc, 0, 0x02);
        // R1 = 4 → 2KB bank at $0800 (1KB banks 4&5).
        write_bank(&mut mmc, 1, 0x04);
        // R2 = 6 → 1KB bank at $1000.
        write_bank(&mut mmc, 2, 0x06);
        // R3 = 7 → 1KB bank at $1400.
        write_bank(&mut mmc, 3, 0x07);

        assert_eq!(mmc.read_chr(0x0000), 0x02); // R0 first half
        assert_eq!(mmc.read_chr(0x03FF), 0x02);
        assert_eq!(mmc.read_chr(0x0400), 0x03); // R0 second half
        assert_eq!(mmc.read_chr(0x07FF), 0x03);
        assert_eq!(mmc.read_chr(0x0800), 0x04); // R1 first half
        assert_eq!(mmc.read_chr(0x0BFF), 0x04);
        assert_eq!(mmc.read_chr(0x0C00), 0x05); // R1 second half
        assert_eq!(mmc.read_chr(0x0FFF), 0x05);
        assert_eq!(mmc.read_chr(0x1000), 0x06); // R2
        assert_eq!(mmc.read_chr(0x13FF), 0x06);
        assert_eq!(mmc.read_chr(0x1400), 0x07); // R3
        assert_eq!(mmc.read_chr(0x17FF), 0x07);
    }

    #[test]
    fn chr_mode_1_swaps_r0_r1_to_right_r2_r5_to_left() {
        let mut mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        // Set CHR mode 1: bit 7 of $8000 = 1.
        write_reg(&mut mmc, 0x8000, 0b1000_0000);
        assert_eq!(mmc.chr_mode(), 1);
        // R0 = 2 → 2KB bank at $1000 (1KB banks 2&3).
        write_bank(&mut mmc, 0, 0x02);
        // R2 = 6 → 1KB bank at $0000.
        write_bank(&mut mmc, 2, 0x06);

        assert_eq!(mmc.read_chr(0x0000), 0x06); // R2 at $0000
        assert_eq!(mmc.read_chr(0x03FF), 0x06);
        assert_eq!(mmc.read_chr(0x1000), 0x02); // R0 first half at $1000
        assert_eq!(mmc.read_chr(0x13FF), 0x02);
        assert_eq!(mmc.read_chr(0x1400), 0x03); // R0 second half
        assert_eq!(mmc.read_chr(0x17FF), 0x03);
    }

    #[test]
    fn r0_r1_lsb_dropped_for_2kb_alignment() {
        let mut mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        // R0 = 3 (odd) → 2KB bank = 3 & 0xFE = 2 (1KB banks 2&3).
        write_bank(&mut mmc, 0, 0x03);
        assert_eq!(mmc.read_chr(0x0000), 0x02);
        assert_eq!(mmc.read_chr(0x03FF), 0x02);
        assert_eq!(mmc.read_chr(0x0400), 0x03);
        assert_eq!(mmc.read_chr(0x07FF), 0x03);
    }

    #[test]
    fn chr_bank_wraps_within_chr_size() {
        // 4 × 1KB CHR banks. R2 = 6 → 6 % 4 = 2.
        let mut mmc = Mmc3::new(make_prg(4), make_chr(4), Mirroring::Horizontal, false);
        write_bank(&mut mmc, 2, 0x06);
        assert_eq!(mmc.read_chr(0x1000), 0x02);
    }

    #[test]
    fn chr_mode_1_r1_2kb_at_1800() {
        // Exercise the (1, 6)/(1, 7) match arms: R1 in CHR mode 1 maps a
        // 2KB bank at $1800 (1KB slots 6 and 7).
        let mut mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        // Set CHR mode 1 (bit 7) and select R1.
        write_reg(&mut mmc, 0x8000, 0b1000_0000 | 1);
        write_reg(&mut mmc, 0x8001, 0x04); // R1 = 4 → 2KB bank (1KB banks 4&5)
        assert_eq!(mmc.read_chr(0x1800), 0x04); // slot 6
        assert_eq!(mmc.read_chr(0x1BFF), 0x04);
        assert_eq!(mmc.read_chr(0x1C00), 0x05); // slot 7
        assert_eq!(mmc.read_chr(0x1FFF), 0x05);
    }

    // ---- PRG-RAM -------------------------------------------------------

    #[test]
    fn prg_ram_disabled_by_default() {
        let mut mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        mmc.write_prg(0x6000, 0x42);
        assert_eq!(mmc.read_prg(0x6000), 0x00); // disabled → returns 0
    }

    #[test]
    fn prg_ram_enabled_via_a001() {
        let mut mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        // Enable PRG-RAM: bit 7 = 1, bit 6 = 0 (no write protect).
        write_reg(&mut mmc, 0xA001, 0x80);
        mmc.write_prg(0x6000, 0x42);
        mmc.write_prg(0x7FFF, 0x99);
        assert_eq!(mmc.read_prg(0x6000), 0x42);
        assert_eq!(mmc.read_prg(0x7FFF), 0x99);
    }

    #[test]
    fn prg_ram_write_protect_blocks_writes() {
        let mut mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        // Enable + write protect: bits 7 and 6 = 1.
        write_reg(&mut mmc, 0xA001, 0xC0);
        mmc.write_prg(0x6000, 0x42);
        // Write was blocked; read returns 0 (RAM is zero-initialised).
        assert_eq!(mmc.read_prg(0x6000), 0x00);
    }

    // ---- Mirroring -----------------------------------------------------

    #[test]
    fn mirroring_change_via_a000() {
        let mut mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Vertical, false);
        assert_eq!(mmc.mirror_mode(), Mirroring::Vertical);
        // bit 0 = 1 → horizontal.
        write_reg(&mut mmc, 0xA000, 0x01);
        assert_eq!(mmc.mirror_mode(), Mirroring::Horizontal);
        // bit 0 = 0 → vertical.
        write_reg(&mut mmc, 0xA000, 0x00);
        assert_eq!(mmc.mirror_mode(), Mirroring::Vertical);
    }

    // ---- IRQ counter ---------------------------------------------------

    #[test]
    fn irq_latch_set_via_c000() {
        let mut mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        write_reg(&mut mmc, 0xC000, 0x05);
        assert_eq!(mmc.irq_latch, 0x05);
    }

    #[test]
    fn irq_reload_flag_set_via_c001() {
        let mut mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        write_reg(&mut mmc, 0xC000, 0x05);
        write_reg(&mut mmc, 0xC001, 0x00); // any value
        assert!(mmc.irq_reload_flag);
        // Clocking reloads the counter from the latch.
        mmc.clock_irq();
        assert_eq!(mmc.irq_counter, 0x05);
        assert!(!mmc.irq_reload_flag);
    }

    #[test]
    fn irq_counter_decrements_on_clock() {
        let mut mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        write_reg(&mut mmc, 0xC000, 0x03);
        write_reg(&mut mmc, 0xC001, 0x00); // reload
        mmc.clock_irq(); // reload → counter = 3
        assert_eq!(mmc.irq_counter, 0x03);
        mmc.clock_irq(); // 3 → 2
        assert_eq!(mmc.irq_counter, 0x02);
        mmc.clock_irq(); // 2 → 1
        assert_eq!(mmc.irq_counter, 0x01);
        mmc.clock_irq(); // 1 → 0
        assert_eq!(mmc.irq_counter, 0x00);
    }

    #[test]
    fn irq_fires_when_counter_wraps_past_zero() {
        let mut mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        write_reg(&mut mmc, 0xC000, 0x02);
        write_reg(&mut mmc, 0xC001, 0x00); // reload
        write_reg(&mut mmc, 0xE001, 0x00); // enable IRQ
        mmc.clock_irq(); // reload → counter = 2
        mmc.clock_irq(); // 2 → 1
        mmc.clock_irq(); // 1 → 0
        assert!(!mmc.irq_pending); // not yet
        mmc.clock_irq(); // counter == 0 → reload + assert IRQ
        assert!(mmc.irq_pending);
        assert_eq!(mmc.irq_counter, 0x02); // reloaded from latch
    }

    #[test]
    fn irq_does_not_fire_when_disabled() {
        let mut mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        write_reg(&mut mmc, 0xC000, 0x01);
        write_reg(&mut mmc, 0xC001, 0x00); // reload
                                           // IRQ disabled by default.
        mmc.clock_irq(); // reload → counter = 1
        mmc.clock_irq(); // 1 → 0
        mmc.clock_irq(); // 0 → reload, but no IRQ (disabled)
        assert!(!mmc.irq_pending);
    }

    #[test]
    fn irq_disable_clears_pending() {
        let mut mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        write_reg(&mut mmc, 0xC000, 0x01);
        write_reg(&mut mmc, 0xC001, 0x00);
        write_reg(&mut mmc, 0xE001, 0x00); // enable
        mmc.clock_irq(); // reload → 1
        mmc.clock_irq(); // 1 → 0
        mmc.clock_irq(); // 0 → reload + IRQ
        assert!(mmc.irq_pending);
        write_reg(&mut mmc, 0xE000, 0x00); // disable + clear
        assert!(!mmc.irq_pending);
        assert!(!mmc.irq_enable);
    }

    #[test]
    fn irq_enable_does_not_clear_pending() {
        let mut mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        mmc.irq_pending = true;
        write_reg(&mut mmc, 0xE001, 0x00); // enable
        assert!(mmc.irq_pending); // still pending
        assert!(mmc.irq_enable);
    }

    #[test]
    fn irq_latch_zero_fires_every_clock_when_enabled() {
        // Latch = 0: counter stays 0, reloads to 0, asserts IRQ every clock.
        let mut mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        write_reg(&mut mmc, 0xC000, 0x00); // latch = 0
        write_reg(&mut mmc, 0xC001, 0x00); // reload
        write_reg(&mut mmc, 0xE001, 0x00); // enable
        mmc.clock_irq(); // reload → 0 (no IRQ on the reload clock)
        assert!(!mmc.irq_pending);
        mmc.clock_irq(); // counter == 0 → reload + IRQ
        assert!(mmc.irq_pending);
        // Clear and clock again → fires again immediately.
        write_reg(&mut mmc, 0xE000, 0x00); // disable + clear
        assert!(!mmc.irq_pending);
        write_reg(&mut mmc, 0xE001, 0x00); // re-enable
        mmc.clock_irq(); // counter == 0 → reload + IRQ
        assert!(mmc.irq_pending);
    }

    #[test]
    fn irq_stays_asserted_until_e000_clear() {
        let mut mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        write_reg(&mut mmc, 0xC000, 0x01);
        write_reg(&mut mmc, 0xC001, 0x00);
        write_reg(&mut mmc, 0xE001, 0x00);
        mmc.clock_irq(); // reload → 1
        mmc.clock_irq(); // 1 → 0
        mmc.clock_irq(); // 0 → IRQ
                         // IRQ stays asserted across multiple clocks (level-triggered).
        mmc.clock_irq(); // 0 → reload + IRQ again
        mmc.clock_irq(); // 1 → 0
        assert!(mmc.irq_pending);
        write_reg(&mut mmc, 0xE000, 0x00);
        assert!(!mmc.irq_pending);
    }

    // ---- Battery flag --------------------------------------------------

    #[test]
    fn battery_flag_reported() {
        let mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, true);
        assert!(mmc.has_battery());
    }

    // ---- CHR-RAM -------------------------------------------------------

    #[test]
    fn chr_ram_writes_persist() {
        let mut mmc = Mmc3::new(make_prg(4), vec![], Mirroring::Horizontal, false);
        assert!(mmc.chr_is_ram);
        mmc.write_chr(0x0000, 0x42);
        assert_eq!(mmc.read_chr(0x0000), 0x42);
    }

    #[test]
    fn chr_rom_writes_ignored() {
        let mut mmc = Mmc3::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        assert!(!mmc.chr_is_ram);
        mmc.write_chr(0x0000, 0x42);
        // Bank 0 fill byte is 0; write was ignored.
        assert_eq!(mmc.read_chr(0x0000), 0x00);
    }
}
