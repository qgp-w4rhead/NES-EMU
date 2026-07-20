//! MMC5 — iNES mapper 5.
//!
//! MMC5 is Nintendo's largest NES mapper, used by *Castlevania III*
//! (US/EU, the Japanese version uses VRC6), *Laser Invasion*, *Uchuu
//! Keibitai SDF*, and *Metal Slader Glory*. It provides:
//!
//! - **PRG-ROM banking**: 8 KB / 16 KB / 32 KB switchable windows with
//!   several modes (we implement the common 8 KB mode: three switchable
//!   8 KB banks at `$8000-$DFFF` plus a fixed last 8 KB bank at
//!   `$E000-$FFFF`).
//! - **PRG-RAM**: up to 64 KB, banked in 8 KB windows at `$6000-$7FFF`
//!   (via `$5113`) and optionally within the PRG-ROM space. Battery-
//!   backed on most carts.
//! - **CHR-ROM/RAM banking**: 1 KB / 2 KB / 4 KB banks (we implement 1 KB
//!   mode: eight 1 KB banks via `$5120-$5127`).
//! - **Hardware multiplier**: writing the two 8-bit operands to `$5205`
//!   and `$5206` makes the 16-bit product available at `$5205` (low) and
//!   `$5206` (high).
//! - **Switchable nametable mirroring**: `$5105` configures each of the
//!   four nametable slots independently (2 bits per slot).
//! - **Scanline IRQ**: `$5203` holds a scanline number; when the PPU
//!   reaches that scanline and the IRQ is enabled (`$5204` bit 7), an IRQ
//!   is asserted. The bus clocks this once per scanline via `clock_irq`.
//! - **Split-screen**: `$5200-$5202` configure a hardware vertical split
//!   (split tile, split Y-scroll, split CHR bank). The registers are
//!   stored here; full PPU split rendering integration is a future
//!   enhancement (the PPU does not yet query the mapper for split state).
//!
//! See: https://www.nesdev.org/wiki/MMC5

use super::{Mapper, Mirroring};

/// PRG-ROM 8 KB bank size.
const PRG_BANK_SIZE: usize = 8 * 1024;
/// CHR 1 KB bank size.
const CHR_1K_SIZE: usize = 1024;
/// PRG-RAM size (64 KB — the maximum MMC5 supports; banked in 8 KB windows).
const PRG_RAM_SIZE: usize = 64 * 1024;
/// PRG-RAM window size (8 KB at `$6000-$7FFF`).
const PRG_RAM_WINDOW: usize = 8 * 1024;

/// MMC5 cartridge state.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Mmc5 {
    prg_rom: Vec<u8>,
    /// CHR data — ROM when `chr_is_ram` is false, RAM when true.
    chr: Vec<u8>,
    chr_is_ram: bool,
    /// Up to 64 KB PRG-RAM, banked into `$6000-$7FFF` via `$5113`.
    prg_ram: Vec<u8>,
    has_battery: bool,

    /// `$5100` PRG bank mode (0-3). We fully implement mode 3 (8 KB banks);
    /// other modes fall back to 8 KB behaviour with the mode stored.
    prg_mode: u8,
    /// `$5101` CHR bank mode (0=8K, 1=16K, 2=32K, 3=1K per NESdev). We
    /// implement 1 KB mode (3); other modes are stored but fall back to
    /// 1 KB behaviour.
    chr_mode: u8,
    /// `$5102` PRG-RAM protect register 1 (must be `0b10` for writes).
    prg_ram_protect1: u8,
    /// `$5103` PRG-RAM protect register 2 (must be `0b01` for writes).
    prg_ram_protect2: u8,
    /// `$5105` nametable mirroring: 2 bits per NT slot (4 slots → 8 bits).
    nt_mirroring: u8,
    /// `$5106` fill-mode tile.
    fill_tile: u8,
    /// `$5107` fill-mode attribute.
    fill_attr: u8,

    /// `$5113` PRG-RAM bank for `$6000-$7FFF` (8 KB window into 64 KB).
    prg_ram_bank: u8,
    /// `$5114`-`$5117` PRG-ROM bank registers for the four 8 KB slots at
    /// `$8000-$FFFF` (mode 3). Bit 7 set selects PRG-RAM instead of ROM.
    prg_banks: [u8; 4],

    /// `$5120`-`$5127` CHR bank registers (1 KB each, for `$0000-$1FFF`).
    chr_banks: [u8; 8],
    /// `$5128`-`$512B` extra CHR banks (sprite-only when bg/sprite split).
    chr_banks_ex: [u8; 4],
    /// `$5130` CHR bank upper bits (bit 0 = upper bit of CHR bank number).
    chr_upper: u8,

    /// `$5200` split control.
    split_control: u8,
    /// `$5201` split Y-scroll.
    split_y_scroll: u8,
    /// `$5202` split bank.
    split_bank: u8,

    /// `$5203` IRQ scanline number.
    irq_scanline: u8,
    /// `$5204` IRQ control (bit 7 = enable) + status (bit 7 = pending).
    irq_control: u8,
    /// Running scanline counter (incremented by `clock_irq` once per
    /// scanline). Compared to `irq_scanline` to trigger the IRQ.
    scanline_counter: u8,
    /// Whether the IRQ is currently asserted.
    irq_pending: bool,
    /// Whether we are "in VBlank" (set when scanline counter passes 240).
    in_vblank: bool,

    /// `$5205` multiplier operand A.
    mult_a: u8,
    /// `$5206` multiplier operand B.
    mult_b: u8,
}

impl Mmc5 {
    /// Construct an MMC5 mapper from parsed PRG/CHR data.
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
        // Encode the header mirroring into $5105. Each NT slot gets 2 bits.
        // Horizontal = 01 01 00 00? The MMC5 $5105 encoding per NESdev:
        //   00 = NT0, 01 = NT1, 10 = NT2, 11 = NT3 for each slot.
        // Horizontal mirroring = NT0,NT1,NT0,NT1 → 01 00 01 00 = 0x44.
        // Vertical mirroring = NT0,NT0,NT1,NT1 → 00 00 01 01 = 0x50? Let's
        // compute: slot0=NT0(00), slot1=NT0(00), slot2=NT1(01), slot3=NT1(01)
        // → bits: slot0 bits 0-1, slot1 bits 2-3, slot2 bits 4-5, slot3 bits 6-7.
        // Vertical = 00 00 01 01 = 0b01010000 = 0x50.
        // Horizontal = slot0=NT0(00), slot1=NT1(01), slot2=NT0(00), slot3=NT1(01)
        // → 0b01000100 = 0x44.
        let nt_mirroring = match mirroring {
            Mirroring::Horizontal => 0x44,
            Mirroring::Vertical => 0x50,
            Mirroring::FourScreen => 0xE4, // NT0,NT1,NT2,NT3 → 11 10 01 00 = 0xE4
            Mirroring::SingleScreen(n) => {
                let bits = n & 3;
                bits | (bits << 2) | (bits << 4) | (bits << 6)
            }
        };
        Self {
            prg_rom,
            chr,
            chr_is_ram,
            prg_ram: vec![0u8; PRG_RAM_SIZE],
            has_battery,
            prg_mode: 3,
            chr_mode: 3,
            prg_ram_protect1: 0,
            prg_ram_protect2: 0,
            nt_mirroring,
            fill_tile: 0,
            fill_attr: 0,
            prg_ram_bank: 0,
            prg_banks: [0u8; 4], // slots 0-2 switchable; slot 3 fixed last
            chr_banks: [0u8; 8],
            chr_banks_ex: [0u8; 4],
            chr_upper: 0,
            split_control: 0,
            split_y_scroll: 0,
            split_bank: 0,
            irq_scanline: 0,
            irq_control: 0,
            scanline_counter: 0,
            irq_pending: false,
            in_vblank: false,
            mult_a: 0,
            mult_b: 0,
        }
    }

    /// Number of 8 KB PRG-ROM banks.
    fn prg_bank_count(&self) -> usize {
        (self.prg_rom.len() / PRG_BANK_SIZE).max(1)
    }

    /// Number of 1 KB CHR banks.
    fn chr_1k_count(&self) -> usize {
        (self.chr.len() / CHR_1K_SIZE).max(1)
    }

    /// Read a PRG-ROM byte from a given 8 KB bank + offset.
    fn prg_read_bank(&self, bank: usize, offset: usize) -> u8 {
        let count = self.prg_bank_count();
        let bank = bank % count;
        let idx = bank * PRG_BANK_SIZE + offset;
        self.prg_rom.get(idx).copied().unwrap_or(0)
    }

    /// Decode the nametable mirroring from `$5105` into the public
    /// `Mirroring` enum. MMC5 supports per-slot mapping, which the simple
    /// `Mirroring` enum cannot fully express; we approximate by inspecting
    /// the four 2-bit slots and returning the closest standard mode. For
    /// single-screen configurations (all slots equal) we return
    /// `SingleScreen(n)`; for horizontal/vertical patterns we return the
    /// matching mode; otherwise we fall back to horizontal.
    fn decode_mirroring(&self) -> Mirroring {
        let s0 = self.nt_mirroring & 0x03;
        let s1 = (self.nt_mirroring >> 2) & 0x03;
        let s2 = (self.nt_mirroring >> 4) & 0x03;
        let s3 = (self.nt_mirroring >> 6) & 0x03;
        if s0 == s1 && s1 == s2 && s2 == s3 {
            return Mirroring::SingleScreen(s0);
        }
        // Horizontal: NT0,NT1,NT0,NT1 → s0=0,s1=1,s2=0,s3=1.
        if s0 == 0 && s1 == 1 && s2 == 0 && s3 == 1 {
            return Mirroring::Horizontal;
        }
        // Vertical: NT0,NT0,NT1,NT1 → s0=0,s1=0,s2=1,s3=1.
        if s0 == 0 && s1 == 0 && s2 == 1 && s3 == 1 {
            return Mirroring::Vertical;
        }
        // Four-screen: NT0,NT1,NT2,NT3.
        if s0 == 0 && s1 == 1 && s2 == 2 && s3 == 3 {
            return Mirroring::FourScreen;
        }
        // Fallback: treat as horizontal.
        Mirroring::Horizontal
    }

    /// The 16-bit hardware multiplier product (`$5205` low, `$5206` high).
    fn product(&self) -> u16 {
        (self.mult_a as u16) * (self.mult_b as u16)
    }

    /// Whether PRG-RAM writes are allowed: `$5102 == 0b10` AND `$5103 == 0b01`.
    /// See: https://www.nesdev.org/wiki/MMC5#PRG_RAM_Protect_(.245102.2F.245103)
    fn prg_ram_writes_allowed(&self) -> bool {
        self.prg_ram_protect1 == 0b10 && self.prg_ram_protect2 == 0b01
    }
}

impl Mapper for Mmc5 {
    fn read_prg(&self, addr: u16) -> u8 {
        // Hardware multiplier reads at $5205/$5206.
        if addr == 0x5205 {
            return (self.product() & 0xFF) as u8;
        }
        if addr == 0x5206 {
            return ((self.product() >> 8) & 0xFF) as u8;
        }
        // IRQ status read at $5204 (bit 7 = in-VBlank / IRQ fired).
        if addr == 0x5204 {
            let mut status = self.irq_control & 0x80;
            if self.in_vblank {
                status |= 0x40;
            }
            return status;
        }

        // PRG-RAM at $6000-$7FFF (8 KB window banked via $5113).
        if (0x6000..0x8000).contains(&addr) {
            let bank = (self.prg_ram_bank as usize) % (PRG_RAM_SIZE / PRG_RAM_WINDOW);
            let idx = bank * PRG_RAM_WINDOW + ((addr as usize - 0x6000) & (PRG_RAM_WINDOW - 1));
            return self.prg_ram.get(idx).copied().unwrap_or(0);
        }

        // $8000-$FFFF: 4 × 8 KB slots (mode 3). Slot 3 ($E000) is fixed
        // to the last PRG-ROM bank; slots 0-2 are switchable and may
        // select PRG-RAM when bit 7 of the bank register is set.
        let local = (addr - 0x8000) as usize;
        let slot = local / PRG_BANK_SIZE; // 0..=3
        let offset = local & (PRG_BANK_SIZE - 1);

        if slot == 3 {
            // Fixed last PRG-ROM bank.
            let last = self.prg_bank_count().saturating_sub(1);
            return self.prg_read_bank(last, offset);
        }

        let reg = self.prg_banks[slot];
        if reg & 0x80 != 0 {
            // Bit 7 set: PRG-RAM bank (low bits select RAM bank).
            let ram_bank = (reg & 0x7F) as usize % (PRG_RAM_SIZE / PRG_BANK_SIZE);
            let idx = ram_bank * PRG_BANK_SIZE + offset;
            return self.prg_ram.get(idx).copied().unwrap_or(0);
        }

        // PRG-ROM bank.
        let count = self.prg_bank_count();
        let bank = (reg as usize) % count;
        self.prg_read_bank(bank, offset)
    }

    fn write_prg(&mut self, addr: u16, value: u8) {
        // Multiplier operands.
        if addr == 0x5205 {
            self.mult_a = value;
            return;
        }
        if addr == 0x5206 {
            self.mult_b = value;
            return;
        }
        // IRQ control / status.
        if addr == 0x5204 {
            self.irq_control = value & 0x80;
            // Writing $5204 with bit 7 clear acknowledges the IRQ.
            if value & 0x80 == 0 {
                self.irq_pending = false;
            }
            return;
        }
        if addr == 0x5203 {
            self.irq_scanline = value;
            return;
        }
        // Split registers.
        if addr == 0x5200 {
            self.split_control = value;
            return;
        }
        if addr == 0x5201 {
            self.split_y_scroll = value;
            return;
        }
        if addr == 0x5202 {
            self.split_bank = value;
            return;
        }
        // PRG/CHR mode + mirroring + fill.
        match addr {
            0x5100 => {
                self.prg_mode = value & 0x03;
                return;
            }
            0x5101 => {
                self.chr_mode = value & 0x03;
                return;
            }
            0x5102 => {
                self.prg_ram_protect1 = value & 0x03;
                return;
            }
            0x5103 => {
                self.prg_ram_protect2 = value & 0x03;
                return;
            }
            0x5104 => {
                // Extended NT mode (VRAM configuration). Stored but not
                // fully wired into the PPU's NT mapping yet.
                return;
            }
            0x5105 => {
                self.nt_mirroring = value;
                return;
            }
            0x5106 => {
                self.fill_tile = value;
                return;
            }
            0x5107 => {
                self.fill_attr = value;
                return;
            }
            0x5113 => {
                self.prg_ram_bank = value;
                return;
            }
            0x5114 => {
                self.prg_banks[0] = value;
                return;
            }
            0x5115 => {
                self.prg_banks[1] = value;
                return;
            }
            0x5116 => {
                self.prg_banks[2] = value;
                return;
            }
            0x5117 => {
                self.prg_banks[3] = value;
                return;
            }
            0x5120 => {
                self.chr_banks[0] = value;
                return;
            }
            0x5121 => {
                self.chr_banks[1] = value;
                return;
            }
            0x5122 => {
                self.chr_banks[2] = value;
                return;
            }
            0x5123 => {
                self.chr_banks[3] = value;
                return;
            }
            0x5124 => {
                self.chr_banks[4] = value;
                return;
            }
            0x5125 => {
                self.chr_banks[5] = value;
                return;
            }
            0x5126 => {
                self.chr_banks[6] = value;
                return;
            }
            0x5127 => {
                self.chr_banks[7] = value;
                return;
            }
            0x5128 => {
                self.chr_banks_ex[0] = value;
                return;
            }
            0x5129 => {
                self.chr_banks_ex[1] = value;
                return;
            }
            0x512A => {
                self.chr_banks_ex[2] = value;
                return;
            }
            0x512B => {
                self.chr_banks_ex[3] = value;
                return;
            }
            0x5130 => {
                self.chr_upper = value & 0x01;
                return;
            }
            _ => {}
        }

        // PRG-RAM writes at $6000-$7FFF (gated by $5102 == 0b10 AND
        // $5103 == 0b01, per NESdev).
        if (0x6000..0x8000).contains(&addr) && self.prg_ram_writes_allowed() {
            let bank = (self.prg_ram_bank as usize) % (PRG_RAM_SIZE / PRG_RAM_WINDOW);
            let idx = bank * PRG_RAM_WINDOW + ((addr as usize - 0x6000) & (PRG_RAM_WINDOW - 1));
            if let Some(slot) = self.prg_ram.get_mut(idx) {
                *slot = value;
            }
            return;
        }

        // PRG-RAM writes at $8000-$DFFF when a switchable slot's bank
        // register has bit 7 set (selecting PRG-RAM). Slot 3 ($E000) is
        // fixed ROM and not writable.
        if (0x8000..0xE000).contains(&addr) && self.prg_ram_writes_allowed() {
            let local = (addr - 0x8000) as usize;
            let slot = local / PRG_BANK_SIZE; // 0..=2
            let offset = local & (PRG_BANK_SIZE - 1);
            let reg = self.prg_banks[slot];
            if reg & 0x80 != 0 {
                let ram_bank = (reg & 0x7F) as usize % (PRG_RAM_SIZE / PRG_BANK_SIZE);
                let idx = ram_bank * PRG_BANK_SIZE + offset;
                if let Some(s) = self.prg_ram.get_mut(idx) {
                    *s = value;
                }
            }
        }
    }

    fn read_chr(&self, addr: u16) -> u8 {
        let slot = (addr as usize) / CHR_1K_SIZE; // 0..=7
        let offset = (addr as usize) & (CHR_1K_SIZE - 1);
        let count = self.chr_1k_count();
        // Combine the 8-bit bank register with the upper bit from $5130.
        let bank_reg = self.chr_banks[slot];
        let bank = ((((self.chr_upper as u16) << 8) | bank_reg as u16) as usize) % count;
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
        let bank_reg = self.chr_banks[slot];
        let bank = ((((self.chr_upper as u16) << 8) | bank_reg as u16) as usize) % count;
        let idx = bank * CHR_1K_SIZE + offset;
        if let Some(s) = self.chr.get_mut(idx) {
            *s = value;
        }
    }

    fn mirror_mode(&self) -> Mirroring {
        self.decode_mirroring()
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

    /// Clock the MMC5 scanline IRQ counter by one step. The bus calls this
    /// once per scanline (via the PPU A12 hook). The internal scanline
    /// counter increments; when it matches `$5203` and the IRQ is enabled
    /// (`$5204` bit 7), the IRQ is asserted. The counter resets at the
    /// prerender scanline (start of a new frame).
    fn clock_irq(&mut self) {
        self.scanline_counter = self.scanline_counter.wrapping_add(1);
        if self.scanline_counter >= 240 {
            self.in_vblank = true;
        }
        if self.scanline_counter == self.irq_scanline && (self.irq_control & 0x80) != 0 {
            self.irq_pending = true;
        }
    }

    fn reset_scanline_counter(&mut self) {
        self.scanline_counter = 0;
        self.in_vblank = false;
    }

    fn save_state(&self) -> super::MapperState {
        super::MapperState::Mmc5(self.clone())
    }

    fn restore_state(&mut self, state: super::MapperState) {
        match state {
            super::MapperState::Mmc5(m) => *self = m,
            _ => panic!("Mmc5::restore_state: expected Mmc5 variant, got a different mapper type"),
        }
    }
}

impl Mmc5 {
    /// Current split-screen configuration (read-only access for the PPU to
    /// query in a future enhancement). Returns `(control, y_scroll, bank)`.
    pub fn split_config(&self) -> (u8, u8, u8) {
        (self.split_control, self.split_y_scroll, self.split_bank)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build PRG-ROM with `banks` 8 KB banks, each filled with a unique byte.
    fn make_prg(banks: usize) -> Vec<u8> {
        let mut prg = vec![0u8; banks * PRG_BANK_SIZE];
        for (i, b) in prg.iter_mut().enumerate() {
            *b = (i / PRG_BANK_SIZE) as u8;
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
    fn prg_default_bank_0_slots_fixed_last_at_e000() {
        // 4 × 8KB banks. Default: slots 0-2 = bank 0, slot 3 = fixed last.
        let mmc = Mmc5::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        assert_eq!(mmc.read_prg(0x8000), 0x00); // slot 0 = bank 0
        assert_eq!(mmc.read_prg(0x9FFF), 0x00);
        assert_eq!(mmc.read_prg(0xA000), 0x00); // slot 1 = bank 0
        assert_eq!(mmc.read_prg(0xC000), 0x00); // slot 2 = bank 0
        assert_eq!(mmc.read_prg(0xE000), 0x03); // slot 3 = fixed last bank
        assert_eq!(mmc.read_prg(0xFFFF), 0x03);
    }

    #[test]
    fn prg_bank_select_switches_8k_windows() {
        let mut mmc = Mmc5::new(make_prg(8), make_chr(8), Mirroring::Horizontal, false);
        mmc.write_prg(0x5114, 0x02); // slot 0 = bank 2
        mmc.write_prg(0x5115, 0x03); // slot 1 = bank 3
        mmc.write_prg(0x5116, 0x04); // slot 2 = bank 4
                                     // Slot 3 ($E000) is fixed to the last bank (7 for 8 banks).
        assert_eq!(mmc.read_prg(0x8000), 0x02);
        assert_eq!(mmc.read_prg(0x9FFF), 0x02);
        assert_eq!(mmc.read_prg(0xA000), 0x03);
        assert_eq!(mmc.read_prg(0xC000), 0x04);
        assert_eq!(mmc.read_prg(0xE000), 0x07);
        assert_eq!(mmc.read_prg(0xFFFF), 0x07);
    }

    #[test]
    fn prg_bank_wraps_within_rom_size() {
        let mut mmc = Mmc5::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        mmc.write_prg(0x5114, 0x06); // 6 % 4 = 2
        assert_eq!(mmc.read_prg(0x8000), 0x02);
    }

    // ---- PRG-RAM -------------------------------------------------------

    #[test]
    fn prg_ram_banked_at_6000_via_5113() {
        let mut mmc = Mmc5::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        // Enable writes: protect bits 0-1 must be 0b11.
        mmc.write_prg(0x5102, 0x02);
        mmc.write_prg(0x5103, 0x01);
        // Bank 0 at $6000.
        mmc.write_prg(0x6000, 0x42);
        assert_eq!(mmc.read_prg(0x6000), 0x42);
        // Switch to bank 1 and write a different value.
        mmc.write_prg(0x5113, 0x01);
        mmc.write_prg(0x6000, 0x99);
        assert_eq!(mmc.read_prg(0x6000), 0x99);
        // Switch back to bank 0 — original value preserved.
        mmc.write_prg(0x5113, 0x00);
        assert_eq!(mmc.read_prg(0x6000), 0x42);
    }

    #[test]
    fn prg_ram_writes_blocked_when_protect_not_set() {
        let mut mmc = Mmc5::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        // protect = 0 (default) → writes blocked.
        mmc.write_prg(0x6000, 0x42);
        assert_eq!(mmc.read_prg(0x6000), 0x00);
    }

    #[test]
    fn prg_ram_bank_selectable_in_prg_window() {
        // Bit 7 of a PRG bank register selects PRG-RAM.
        let mut mmc = Mmc5::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        mmc.write_prg(0x5102, 0x02);
        mmc.write_prg(0x5103, 0x01); // enable RAM writes
                                     // Map PRG-RAM bank 0 at $8000 by setting prg_banks[0] = 0x80.
        mmc.write_prg(0x5114, 0x80);
        mmc.write_prg(0x8000, 0x77);
        assert_eq!(mmc.read_prg(0x8000), 0x77);
    }

    // ---- Hardware multiplier ------------------------------------------

    #[test]
    fn multiplier_product_low_at_5205() {
        let mut mmc = Mmc5::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        mmc.write_prg(0x5205, 0x07); // A = 7
        mmc.write_prg(0x5206, 0x09); // B = 9 → product = 63 = 0x003F
        assert_eq!(mmc.read_prg(0x5205), 0x3F);
        assert_eq!(mmc.read_prg(0x5206), 0x00);
    }

    #[test]
    fn multiplier_product_high_at_5206() {
        let mut mmc = Mmc5::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        mmc.write_prg(0x5205, 0xFF); // A = 255
        mmc.write_prg(0x5206, 0xFF); // B = 255 → product = 65025 = 0xFE01
        assert_eq!(mmc.read_prg(0x5205), 0x01);
        assert_eq!(mmc.read_prg(0x5206), 0xFE);
    }

    #[test]
    fn multiplier_zero_operand() {
        let mut mmc = Mmc5::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        mmc.write_prg(0x5205, 0x00);
        mmc.write_prg(0x5206, 0x42);
        assert_eq!(mmc.read_prg(0x5205), 0x00);
        assert_eq!(mmc.read_prg(0x5206), 0x00);
    }

    // ---- CHR banking ---------------------------------------------------

    #[test]
    fn chr_eight_1kb_banks_selectable() {
        let mut mmc = Mmc5::new(make_prg(4), make_chr(16), Mirroring::Horizontal, false);
        mmc.write_prg(0x5120, 0x02); // CHR 0 = 2
        mmc.write_prg(0x5127, 0x0F); // CHR 7 = 15
        assert_eq!(mmc.read_chr(0x0000), 0x02);
        assert_eq!(mmc.read_chr(0x03FF), 0x02);
        assert_eq!(mmc.read_chr(0x1C00), 0x0F);
        assert_eq!(mmc.read_chr(0x1FFF), 0x0F);
    }

    #[test]
    fn chr_upper_bit_extends_bank_number() {
        let mut mmc = Mmc5::new(make_prg(4), make_chr(16), Mirroring::Horizontal, false);
        mmc.write_prg(0x5130, 0x01); // upper bit = 1
        mmc.write_prg(0x5120, 0x00); // CHR 0 = 0x100 = 256 % 16 = 0
                                     // With 16 1KB banks, bank 0x100 % 16 = 0.
        assert_eq!(mmc.read_chr(0x0000), 0x00);
    }

    #[test]
    fn chr_ram_writes_persist() {
        let mut mmc = Mmc5::new(make_prg(4), vec![], Mirroring::Horizontal, false);
        mmc.write_chr(0x0000, 0x42);
        assert_eq!(mmc.read_chr(0x0000), 0x42);
    }

    // ---- Mirroring -----------------------------------------------------

    #[test]
    fn mirroring_default_horizontal_from_header() {
        let mmc = Mmc5::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        assert_eq!(mmc.mirror_mode(), Mirroring::Horizontal);
    }

    #[test]
    fn mirroring_default_vertical_from_header() {
        let mmc = Mmc5::new(make_prg(4), make_chr(8), Mirroring::Vertical, false);
        assert_eq!(mmc.mirror_mode(), Mirroring::Vertical);
    }

    #[test]
    fn mirroring_switchable_via_5105() {
        let mut mmc = Mmc5::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        // Vertical: 0x50.
        mmc.write_prg(0x5105, 0x50);
        assert_eq!(mmc.mirror_mode(), Mirroring::Vertical);
        // Single-screen NT2: all slots = 2 → 0b10 10 10 10 = 0xAA.
        mmc.write_prg(0x5105, 0xAA);
        assert_eq!(mmc.mirror_mode(), Mirroring::SingleScreen(2));
    }

    // ---- Scanline IRQ --------------------------------------------------

    #[test]
    fn irq_fires_when_scanline_counter_matches() {
        let mut mmc = Mmc5::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        mmc.write_prg(0x5203, 0x05); // target scanline = 5
        mmc.write_prg(0x5204, 0x80); // enable IRQ
                                     // Clock 5 scanlines: counter goes 1,2,3,4,5 → fire at 5.
        for _ in 0..5 {
            mmc.clock_irq();
        }
        assert!(mmc.irq_pending());
    }

    #[test]
    fn irq_disabled_does_not_fire() {
        let mut mmc = Mmc5::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        mmc.write_prg(0x5203, 0x05);
        // Don't enable.
        for _ in 0..10 {
            mmc.clock_irq();
        }
        assert!(!mmc.irq_pending());
    }

    #[test]
    fn irq_acknowledged_by_writing_5204_clear() {
        let mut mmc = Mmc5::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        mmc.write_prg(0x5203, 0x02);
        mmc.write_prg(0x5204, 0x80);
        for _ in 0..2 {
            mmc.clock_irq();
        }
        assert!(mmc.irq_pending());
        mmc.write_prg(0x5204, 0x00); // clear
        assert!(!mmc.irq_pending());
    }

    // ---- Split registers ----------------------------------------------

    #[test]
    fn split_registers_stored_and_readable() {
        let mut mmc = Mmc5::new(make_prg(4), make_chr(8), Mirroring::Horizontal, false);
        mmc.write_prg(0x5200, 0x01);
        mmc.write_prg(0x5201, 0x80);
        mmc.write_prg(0x5202, 0x42);
        assert_eq!(mmc.split_config(), (0x01, 0x80, 0x42));
    }

    // ---- Battery -------------------------------------------------------

    #[test]
    fn battery_sram_round_trips() {
        let mut mmc = Mmc5::new(make_prg(4), make_chr(8), Mirroring::Horizontal, true);
        mmc.write_prg(0x5102, 0x02);
        mmc.write_prg(0x5103, 0x01); // enable RAM writes
        mmc.write_prg(0x6000, 0xAB);
        let saved = mmc.battery_sram().expect("battery");
        let mut mmc2 = Mmc5::new(make_prg(4), make_chr(8), Mirroring::Horizontal, true);
        mmc2.load_battery_sram(&saved);
        mmc2.write_prg(0x5102, 0x02);
        mmc2.write_prg(0x5103, 0x01);
        assert_eq!(mmc2.read_prg(0x6000), 0xAB);
    }
}
