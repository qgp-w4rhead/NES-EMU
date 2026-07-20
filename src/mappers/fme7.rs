//! FME-7 — iNES mapper 69 (Sunsoft 5B without the audio).
//!
//! FME-7 is Sunsoft's mapper used by *Gimmick!* (with the 5B audio), *Mr.
//! Gimmick*, *Hebereke*, and *Ufouria*. The FME-7 proper provides flexible
//! PRG/CHR banking, PRG-RAM, switchable mirroring, and a 16-bit CPU-clocked
//! IRQ timer. (The Sunsoft 5B variant adds YM2149 audio, covered in M35.)
//!
//! # Register layout
//!
//! FME-7 uses a command/data latch pair:
//!
//! | Address   | Function                                     |
//! |-----------|----------------------------------------------|
//! | `$8000`   | Command register (selects target for `$8001`)|
//! | `$8001`   | Data register (writes the selected register) |
//!
//! The command register (low 4 bits) selects one of 16 internal registers:
//!
//! | Cmd | Function                                     |
//! |-----|-----------------------------------------------|
//! | 0-7 | CHR bank 0-7 (1 KB each, `$0000-$1FFF`)       |
//! | 8   | PRG bank 0 (`$8000-$9FFF`)                    |
//! | 9   | PRG bank 1 (`$A000-$BFFF`)                    |
//! | 10  | PRG bank 2 (`$C000-$DFFF`)                    |
//! | 11  | PRG bank 3 (`$E000-$FFFF`)                    |
//! | 12  | Mirroring (bit 0: 0=vertical, 1=horizontal)   |
//! | 13  | PRG-RAM enable (bit 7) + write protect (bit 6)|
//! | 14  | IRQ latch (two writes: low byte, then high)   |
//! | 15  | IRQ control (bit 0=enable, bit 1=enable+ack)  |
//!
//! The IRQ is a 16-bit down-counter clocked once per CPU cycle. When it
//! reaches zero it reloads from the 16-bit latch and, if enabled, asserts
//! the CPU IRQ. Writing command 15 with bit 1 set acknowledges the pending
//! IRQ and enables it; bit 0 alone enables without acknowledging.
//!
//! See: https://www.nesdev.org/wiki/FME-7

use super::{Mapper, Mirroring};

/// PRG-ROM bank size (8 KB).
const PRG_BANK_SIZE: usize = 8 * 1024;
/// CHR 1 KB bank size.
const CHR_1K_SIZE: usize = 1024;
/// PRG-RAM size (8 KB at `$6000-$7FFF`).
const PRG_RAM_SIZE: usize = 8 * 1024;

/// FME-7 cartridge state.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Fme7 {
    prg_rom: Vec<u8>,
    /// CHR data — ROM when `chr_is_ram` is false, RAM when true.
    chr: Vec<u8>,
    chr_is_ram: bool,
    /// 8 KB PRG-RAM at `$6000-$7FFF`.
    prg_ram: Vec<u8>,
    has_battery: bool,

    /// Command register (low 4 bits select the target of `$8001` writes).
    command: u8,
    /// Four 8 KB PRG bank registers (commands 8-11).
    prg_banks: [u8; 4],
    /// Eight 1 KB CHR bank registers (commands 0-7).
    chr_banks: [u8; 8],
    /// Mirroring: 0 = vertical, 1 = horizontal (command 12, bit 0).
    mirror_horizontal: bool,
    /// PRG-RAM enable (command 13, bit 7).
    prg_ram_enable: bool,
    /// PRG-RAM write protect (command 13, bit 6).
    prg_ram_write_protect: bool,

    /// 16-bit IRQ latch (reload value), loaded via two writes to command 14.
    irq_latch: u16,
    /// Running 16-bit IRQ counter, decremented every CPU cycle.
    irq_counter: u16,
    /// Toggle for command 14: false → next write is low byte, true → high.
    irq_latch_high: bool,
    /// IRQ enabled flag.
    irq_enable: bool,
    /// Asserted when the counter hits 0; cleared by command 15 with bit 1.
    irq_pending: bool,
}

impl Fme7 {
    /// Construct an FME-7 mapper from parsed PRG/CHR data.
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
            command: 0,
            prg_banks: [0u8; 4],
            chr_banks: [0u8; 8],
            mirror_horizontal: matches!(mirroring, Mirroring::Horizontal),
            prg_ram_enable: false,
            prg_ram_write_protect: false,
            irq_latch: 0,
            irq_counter: 0,
            irq_latch_high: false,
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

    /// Read a PRG-ROM byte from a given 8 KB bank + offset.
    fn prg_read_bank(&self, bank: usize, offset: usize) -> u8 {
        let count = self.prg_bank_count();
        let bank = bank % count;
        let idx = bank * PRG_BANK_SIZE + offset;
        self.prg_rom.get(idx).copied().unwrap_or(0)
    }

    /// Write to the data register (`$8001`), dispatching by the current
    /// command register value.
    fn write_data(&mut self, value: u8) {
        match self.command & 0x0F {
            0..=7 => self.chr_banks[self.command as usize & 0x07] = value,
            8 => self.prg_banks[0] = value & 0x3F,
            9 => self.prg_banks[1] = value & 0x3F,
            10 => self.prg_banks[2] = value & 0x3F,
            11 => self.prg_banks[3] = value & 0x3F,
            12 => self.mirror_horizontal = (value & 0x01) != 0,
            13 => {
                self.prg_ram_enable = (value & 0x80) != 0;
                self.prg_ram_write_protect = (value & 0x40) != 0;
            }
            14 => {
                // IRQ latch: two writes load low then high byte.
                if !self.irq_latch_high {
                    self.irq_latch = (self.irq_latch & 0xFF00) | value as u16;
                    self.irq_latch_high = true;
                } else {
                    self.irq_latch = (self.irq_latch & 0x00FF) | ((value as u16) << 8);
                    self.irq_latch_high = false;
                }
            }
            15 => {
                // IRQ control: bit 0 = enable, bit 1 = enable + acknowledge.
                // Enabling (either bit) reloads the counter from the latch
                // so the first countdown is a full latch period.
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
            _ => {}
        }
    }
}

impl Mapper for Fme7 {
    fn read_prg(&self, addr: u16) -> u8 {
        // PRG-RAM at $6000-$7FFF.
        if (0x6000..0x8000).contains(&addr) {
            if self.prg_ram_enable {
                let idx = (addr as usize - 0x6000) & (PRG_RAM_SIZE - 1);
                return self.prg_ram.get(idx).copied().unwrap_or(0);
            }
            return 0;
        }

        // $8000-$FFFF: 4 × 8 KB slots.
        let local = (addr - 0x8000) as usize;
        let slot = local / PRG_BANK_SIZE; // 0..=3
        let offset = local & (PRG_BANK_SIZE - 1);
        let bank = (self.prg_banks[slot] as usize) % self.prg_bank_count();
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

        match addr {
            0x8000 => self.command = value,
            0x8001 => self.write_data(value),
            _ => {}
        }
    }

    fn read_chr(&self, addr: u16) -> u8 {
        let slot = (addr as usize) / CHR_1K_SIZE; // 0..=7
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

    /// Clock the FME-7 16-bit IRQ counter by `cpu_cycles` CPU cycles. The
    /// counter decrements every CPU cycle regardless of the enable flag;
    /// when it reaches 0 it reloads from the 16-bit latch and, if enabled,
    /// asserts the CPU IRQ (which then continues counting for the next
    /// period).
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
    }

    fn save_state(&self) -> super::MapperState {
        super::MapperState::Fme7(self.clone())
    }

    fn restore_state(&mut self, state: super::MapperState) {
        match state {
            super::MapperState::Fme7(m) => *self = m,
            _ => panic!("Fme7::restore_state: expected Fme7 variant, got a different mapper type"),
        }
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

    /// Write a command/data pair via the `$8000`/`$8001` latch.
    fn cmd(fme: &mut Fme7, command: u8, value: u8) {
        fme.write_prg(0x8000, command);
        fme.write_prg(0x8001, value);
    }

    // ---- PRG banking ---------------------------------------------------

    #[test]
    fn prg_default_all_banks_zero() {
        // 4 × 8KB banks. Default: all PRG bank regs = 0 → bank 0 everywhere.
        let fme = Fme7::new(make_prg(4), make_chr(8), Mirroring::Vertical, false);
        assert_eq!(fme.read_prg(0x8000), 0x00);
        assert_eq!(fme.read_prg(0xA000), 0x00);
        assert_eq!(fme.read_prg(0xC000), 0x00);
        assert_eq!(fme.read_prg(0xE000), 0x00);
    }

    #[test]
    fn prg_bank_select_switches_each_8k_window() {
        let mut fme = Fme7::new(make_prg(8), make_chr(8), Mirroring::Vertical, false);
        cmd(&mut fme, 8, 0x02); // PRG 0 = bank 2 at $8000
        cmd(&mut fme, 9, 0x03); // PRG 1 = bank 3 at $A000
        cmd(&mut fme, 10, 0x04); // PRG 2 = bank 4 at $C000
        cmd(&mut fme, 11, 0x05); // PRG 3 = bank 5 at $E000
        assert_eq!(fme.read_prg(0x8000), 0x02);
        assert_eq!(fme.read_prg(0x9FFF), 0x02);
        assert_eq!(fme.read_prg(0xA000), 0x03);
        assert_eq!(fme.read_prg(0xBFFF), 0x03);
        assert_eq!(fme.read_prg(0xC000), 0x04);
        assert_eq!(fme.read_prg(0xDFFF), 0x04);
        assert_eq!(fme.read_prg(0xE000), 0x05);
        assert_eq!(fme.read_prg(0xFFFF), 0x05);
    }

    #[test]
    fn prg_bank_wraps_within_rom_size() {
        // 4 banks. PRG 0 = 6 → 6 % 4 = 2.
        let mut fme = Fme7::new(make_prg(4), make_chr(8), Mirroring::Vertical, false);
        cmd(&mut fme, 8, 0x06);
        assert_eq!(fme.read_prg(0x8000), 0x02);
    }

    #[test]
    fn prg_bank_register_masked_to_low_4_bits() {
        let mut fme = Fme7::new(make_prg(8), make_chr(8), Mirroring::Vertical, false);
        cmd(&mut fme, 8, 0xFF); // & 0x0F = 0x0F → 15 % 8 = 7
        assert_eq!(fme.read_prg(0x8000), 0x07);
    }

    // ---- CHR banking ---------------------------------------------------

    #[test]
    fn chr_eight_1kb_banks_selectable() {
        let mut fme = Fme7::new(make_prg(8), make_chr(16), Mirroring::Vertical, false);
        cmd(&mut fme, 0, 0x02); // CHR 0 = 2
        cmd(&mut fme, 3, 0x09); // CHR 3 = 9
        cmd(&mut fme, 7, 0x0F); // CHR 7 = 15
        assert_eq!(fme.read_chr(0x0000), 0x02);
        assert_eq!(fme.read_chr(0x03FF), 0x02);
        assert_eq!(fme.read_chr(0x0C00), 0x09);
        assert_eq!(fme.read_chr(0x0FFF), 0x09);
        assert_eq!(fme.read_chr(0x1C00), 0x0F);
        assert_eq!(fme.read_chr(0x1FFF), 0x0F);
    }

    #[test]
    fn chr_ram_writes_persist() {
        let mut fme = Fme7::new(make_prg(8), vec![], Mirroring::Vertical, false);
        fme.write_chr(0x0000, 0x42);
        assert_eq!(fme.read_chr(0x0000), 0x42);
    }

    // ---- Mirroring -----------------------------------------------------

    #[test]
    fn mirroring_default_from_header() {
        let fme = Fme7::new(make_prg(8), make_chr(8), Mirroring::Horizontal, false);
        assert_eq!(fme.mirror_mode(), Mirroring::Horizontal);
    }

    #[test]
    fn mirroring_switchable_via_command_12() {
        let mut fme = Fme7::new(make_prg(8), make_chr(8), Mirroring::Vertical, false);
        cmd(&mut fme, 12, 0x01); // horizontal
        assert_eq!(fme.mirror_mode(), Mirroring::Horizontal);
        cmd(&mut fme, 12, 0x00); // vertical
        assert_eq!(fme.mirror_mode(), Mirroring::Vertical);
    }

    // ---- PRG-RAM -------------------------------------------------------

    #[test]
    fn prg_ram_disabled_reads_zero() {
        let fme = Fme7::new(make_prg(8), make_chr(8), Mirroring::Vertical, false);
        assert_eq!(fme.read_prg(0x6000), 0x00);
    }

    #[test]
    fn prg_ram_enable_via_command_13() {
        let mut fme = Fme7::new(make_prg(8), make_chr(8), Mirroring::Vertical, false);
        cmd(&mut fme, 13, 0x80); // enable PRG-RAM
        fme.write_prg(0x6000, 0x42);
        assert_eq!(fme.read_prg(0x6000), 0x42);
    }

    #[test]
    fn prg_ram_write_protect_blocks_writes() {
        let mut fme = Fme7::new(make_prg(8), make_chr(8), Mirroring::Vertical, false);
        cmd(&mut fme, 13, 0xC0); // enable + write protect
        fme.write_prg(0x6000, 0x42);
        assert_eq!(fme.read_prg(0x6000), 0x00); // write blocked
    }

    // ---- IRQ -----------------------------------------------------------

    #[test]
    fn irq_latch_loaded_via_two_writes() {
        let mut fme = Fme7::new(make_prg(8), make_chr(8), Mirroring::Vertical, false);
        // Command 14: write low byte 0x34, then high byte 0x12 → latch 0x1234.
        cmd(&mut fme, 14, 0x34);
        cmd(&mut fme, 14, 0x12);
        assert_eq!(fme.irq_latch, 0x1234);
    }

    #[test]
    fn irq_fires_after_counter_reaches_zero() {
        let mut fme = Fme7::new(make_prg(8), make_chr(8), Mirroring::Vertical, false);
        // Latch = 0x0005.
        cmd(&mut fme, 14, 0x05);
        cmd(&mut fme, 14, 0x00);
        // Enable via command 15 bit 0 — this reloads the counter to 5.
        cmd(&mut fme, 15, 0x01);
        assert!(!fme.irq_pending());
        // 5 cycles: 5→4→3→2→1→0. No fire yet (fire happens on the reload
        // when the counter is 0 and is clocked again).
        fme.clock_cpu(5);
        assert!(!fme.irq_pending());
        // 1 more: 0→reload(5) + fire.
        fme.clock_cpu(1);
        assert!(fme.irq_pending());
    }

    #[test]
    fn irq_disabled_does_not_fire() {
        let mut fme = Fme7::new(make_prg(8), make_chr(8), Mirroring::Vertical, false);
        cmd(&mut fme, 14, 0x01);
        cmd(&mut fme, 14, 0x00);
        // Don't enable.
        fme.clock_cpu(100);
        assert!(!fme.irq_pending());
    }

    #[test]
    fn irq_acknowledge_clears_pending() {
        let mut fme = Fme7::new(make_prg(8), make_chr(8), Mirroring::Vertical, false);
        cmd(&mut fme, 14, 0x01);
        cmd(&mut fme, 14, 0x00);
        cmd(&mut fme, 15, 0x01); // enable
        fme.clock_cpu(4); // fire
        assert!(fme.irq_pending());
        cmd(&mut fme, 15, 0x02); // enable + acknowledge
        assert!(!fme.irq_pending());
    }

    #[test]
    fn irq_continues_after_fire_when_enabled() {
        // Unlike VRC6 one-shot, FME-7 keeps firing every latch period while
        // enabled (the counter reloads and continues).
        let mut fme = Fme7::new(make_prg(8), make_chr(8), Mirroring::Vertical, false);
        cmd(&mut fme, 14, 0x02);
        cmd(&mut fme, 14, 0x00);
        cmd(&mut fme, 15, 0x01); // enable
        fme.clock_cpu(4); // first fire (0→reload 2→1→0→reload+fire)
        assert!(fme.irq_pending());
        cmd(&mut fme, 15, 0x02); // ack
                                 // Continue clocking; should fire again.
        fme.clock_cpu(4);
        assert!(fme.irq_pending());
    }

    // ---- Battery -------------------------------------------------------

    #[test]
    fn battery_sram_round_trips() {
        let mut fme = Fme7::new(make_prg(8), make_chr(8), Mirroring::Vertical, true);
        cmd(&mut fme, 13, 0x80); // enable PRG-RAM
        fme.write_prg(0x6000, 0xCD);
        let saved = fme.battery_sram().expect("battery");
        let mut fme2 = Fme7::new(make_prg(8), make_chr(8), Mirroring::Vertical, true);
        cmd(&mut fme2, 13, 0x80);
        fme2.load_battery_sram(&saved);
        assert_eq!(fme2.read_prg(0x6000), 0xCD);
    }
}
