//! Famicom Disk System — iNES mapper 20 (FDS RAM adapter + disk drive).
//!
//! The FDS uses a RAM adapter that plugs into the cartridge slot. It
//! provides:
//! - 32 KB PRG-RAM at `$6000-$DFFF` (writable; loaded from disk by BIOS)
//! - 8 KB BIOS ROM at `$E000-$FFFF` (loaded from `disksys.rom`)
//! - 8 KB CHR-RAM at `$0000-$1FFF`
//! - FDS expansion audio (wavetable + modulator)
//! - Disk drive I/O registers at `$4020-$4033`
//! - Timer IRQ at `$4020-$4022`
//!
//! # Register map
//!
//! | Address | R/W | Function                                              |
//! |---------|-----|-------------------------------------------------------|
//! | `$4020` | W   | Timer IRQ latch low byte                              |
//! | `$4021` | W   | Timer IRQ latch high byte                             |
//! | `$4022` | W   | Timer IRQ enable                                      |
//! | `$4023` | W   | Master I/O enable (bit 0 = disk, bit 1 = timer)       |
//! | `$4024` | W   | Disk data write                                       |
//! | `$4025` | W   | Disk control (motor, transfer, mirroring)             |
//! | `$4030` | R   | Disk status (bit 7 = timer IRQ flag)                  |
//! | `$4031` | R   | Disk data read                                        |
//! | `$4032` | R   | Disk status 2 (disk inserted flags)                   |
//! | `$4033` | R   | Battery status                                        |
//!
//! See: https://www.nesdev.org/wiki/Famicom_Disk_System
//! See: https://www.nesdev.org/wiki/FDS_disk_format

use super::fds_audio::FdsAudio;
use super::{Mapper, Mirroring};

/// PRG-RAM size (32 KB at `$6000-$DFFF`).
const PRG_RAM_SIZE: usize = 32 * 1024;

/// CHR-RAM size (8 KB).
const CHR_RAM_SIZE: usize = 8 * 1024;

/// BIOS ROM size (8 KB at `$E000-$FFFF`).
const BIOS_SIZE: usize = 8 * 1024;

/// FDS cartridge state — RAM adapter + disk drive + audio.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Fds {
    /// 32 KB PRG-RAM at `$6000-$DFFF`.
    #[serde(with = "crate::save_state::array_ser")]
    prg_ram: [u8; PRG_RAM_SIZE],
    /// 8 KB BIOS ROM at `$E000-$FFFF`.
    #[serde(skip)]
    bios: Vec<u8>,
    /// 8 KB CHR-RAM.
    #[serde(with = "crate::save_state::array_ser")]
    chr_ram: [u8; CHR_RAM_SIZE],

    /// Raw disk data (all sides concatenated).
    #[serde(skip)]
    disk_data: Vec<u8>,
    /// Current disk read position (byte offset into `disk_data`).
    disk_read_pos: usize,
    /// Current disk write position.
    disk_write_pos: usize,
    /// Disk motor on flag ($4025 bit 5).
    disk_motor_on: bool,
    /// Disk transfer reset flag ($4025 bit 0).
    disk_transfer_reset: bool,
    /// Disk read/write mode ($4025 bit 2: 0 = read, 1 = write).
    disk_write_mode: bool,
    /// Disk data available flag (set when data is ready to read).
    disk_data_available: bool,
    /// Disk inserted flag.
    disk_inserted: bool,
    /// Last byte written to $4024 (disk data write).
    disk_write_latch: u8,
    /// Master I/O enable: disk drive (bit 0).
    io_enable_disk: bool,
    /// Master I/O enable: timer IRQ (bit 1).
    io_enable_timer: bool,

    /// Timer IRQ latch (reload value, 16-bit).
    timer_latch: u16,
    /// Timer IRQ counter (counts down).
    timer_counter: u16,
    /// Timer IRQ enable flag.
    timer_enable: bool,
    /// Timer IRQ pending flag.
    timer_irq_pending: bool,

    /// Mirroring: true = horizontal, false = vertical ($4025 bit 1).
    mirror_horizontal: bool,

    /// FDS expansion audio (wavetable + modulator).
    audio: FdsAudio,
}

impl Fds {
    /// Construct an FDS mapper from BIOS ROM and disk data.
    ///
    /// The BIOS is 8 KB (`disksys.rom`); disk data is the raw `.fds`
    /// file contents (header + disk sides). PRG-RAM and CHR-RAM start
    /// zeroed.
    pub fn new(bios: Vec<u8>, disk_data: Vec<u8>) -> Self {
        let bios = if bios.len() >= BIOS_SIZE {
            bios[..BIOS_SIZE].to_vec()
        } else {
            let mut padded = vec![0u8; BIOS_SIZE];
            padded[..bios.len()].copy_from_slice(&bios);
            padded
        };

        Self {
            prg_ram: [0u8; PRG_RAM_SIZE],
            bios,
            chr_ram: [0u8; CHR_RAM_SIZE],
            disk_data,
            disk_read_pos: 0,
            disk_write_pos: 0,
            disk_motor_on: false,
            disk_transfer_reset: false,
            disk_write_mode: false,
            disk_data_available: false,
            disk_inserted: true,
            disk_write_latch: 0,
            io_enable_disk: true,
            io_enable_timer: true,
            timer_latch: 0,
            timer_counter: 0,
            timer_enable: false,
            timer_irq_pending: false,
            mirror_horizontal: false,
            audio: FdsAudio::new(),
        }
    }

    /// Handle a write to a disk I/O register ($4020-$4025).
    fn write_disk_register(&mut self, addr: u16, value: u8) {
        match addr {
            // $4020: Timer IRQ latch low byte.
            0x4020 => {
                self.timer_latch = (self.timer_latch & 0xFF00) | value as u16;
            }
            // $4021: Timer IRQ latch high byte.
            0x4021 => {
                self.timer_latch = (self.timer_latch & 0x00FF) | ((value as u16) << 8);
            }
            // $4022: Timer IRQ enable.
            0x4022 => {
                self.timer_enable = (value & 0x01) != 0;
                if !self.timer_enable {
                    self.timer_irq_pending = false;
                }
                // Reload counter on enable.
                if self.timer_enable {
                    self.timer_counter = self.timer_latch;
                }
            }
            // $4023: Master I/O enable.
            0x4023 => {
                self.io_enable_disk = (value & 0x01) != 0;
                self.io_enable_timer = (value & 0x02) != 0;
                if !self.io_enable_timer {
                    self.timer_irq_pending = false;
                }
            }
            // $4024: Disk data write.
            0x4024 => {
                self.disk_write_latch = value;
                if self.disk_write_mode && self.io_enable_disk {
                    // Write to disk.
                    if self.disk_write_pos < self.disk_data.len() {
                        self.disk_data[self.disk_write_pos] = value;
                        self.disk_write_pos += 1;
                    }
                }
            }
            // $4025: Disk control register.
            0x4025 => {
                if !self.io_enable_disk {
                    return;
                }
                // bit 0: Transfer reset.
                let reset = (value & 0x01) != 0;
                if reset && !self.disk_transfer_reset {
                    // Reset transfer: reset read/write position.
                    self.disk_read_pos = 0;
                    self.disk_write_pos = 0;
                    self.disk_data_available = false;
                }
                self.disk_transfer_reset = reset;

                // bit 1: Mirroring (0 = vertical, 1 = horizontal).
                self.mirror_horizontal = (value & 0x02) != 0;

                // bit 2: Read/Write mode (0 = read, 1 = write).
                self.disk_write_mode = (value & 0x04) != 0;

                // bit 5: Motor on.
                self.disk_motor_on = (value & 0x20) != 0;

                // When motor is on and in read mode, data becomes available.
                if self.disk_motor_on && !self.disk_write_mode {
                    self.disk_data_available = self.disk_read_pos < self.disk_data.len();
                }
            }
            _ => {}
        }
    }

    /// Handle a read from a disk I/O register ($4030-$4033).
    fn read_disk_register(&mut self, addr: u16) -> u8 {
        match addr {
            // $4030: Disk status.
            0x4030 => {
                let mut status = 0u8;
                // bit 0: Disk data (read bit — we model this as data available).
                if self.disk_data_available {
                    status |= 0x01;
                }
                // bit 2: Disk ready (motor on and disk inserted).
                if self.disk_motor_on && self.disk_inserted {
                    status |= 0x04;
                }
                // bit 4: Disk reset (transfer reset active).
                if self.disk_transfer_reset {
                    status |= 0x10;
                }
                // bit 7: Timer IRQ flag.
                if self.timer_irq_pending {
                    status |= 0x80;
                }
                // Reading $4030 clears the timer IRQ flag.
                self.timer_irq_pending = false;
                status
            }
            // $4031: Disk data read.
            0x4031 => {
                if self.disk_read_pos < self.disk_data.len() {
                    let byte = self.disk_data[self.disk_read_pos];
                    self.disk_read_pos += 1;
                    // Update data available flag.
                    self.disk_data_available = self.disk_read_pos < self.disk_data.len();
                    byte
                } else {
                    0x00
                }
            }
            // $4032: Disk status 2 (disk inserted flags).
            // bit 0 = 0 means disk 0 inserted; bit 1,2 for disks 1,2.
            0x4032 => {
                if self.disk_inserted {
                    0x00 // bit 0 clear = disk inserted
                } else {
                    0x01
                }
            }
            // $4033: Battery status (bit 7 = battery good).
            0x4033 => 0x80,
            _ => 0x00,
        }
    }

    /// Check if an address is in the FDS audio register range ($4040-$408A).
    fn is_audio_register(addr: u16) -> bool {
        matches!(addr, 0x4040..=0x408A)
    }

    /// Check if an address is a disk I/O register ($4020-$4033).
    fn is_disk_register(addr: u16) -> bool {
        matches!(addr, 0x4020..=0x4033)
    }
}

impl Mapper for Fds {
    fn read_prg(&self, addr: u16) -> u8 {
        // $6000-$DFFF: 32 KB PRG-RAM.
        if (0x6000..0xE000).contains(&addr) {
            let idx = (addr as usize - 0x6000) & (PRG_RAM_SIZE - 1);
            return self.prg_ram[idx];
        }
        // $E000-$FFFF: 8 KB BIOS ROM.
        if addr >= 0xE000 {
            let idx = (addr as usize - 0xE000) & (BIOS_SIZE - 1);
            return self.bios.get(idx).copied().unwrap_or(0);
        }
        // $4020-$4033 and $4040-$408A are handled by `read_prg_mut`
        // (they have read side-effects). This `&self` path returns 0
        // for those ranges — the bus always calls `read_prg_mut`.
        0x00
    }

    /// Read with side effects — handles FDS disk I/O registers ($4030-
    /// $4033) and audio read registers ($4090, $4092) which mutate
    /// internal state on read (e.g. $4031 advances the disk read
    /// pointer, $4030 clears the timer IRQ flag).
    fn read_prg_mut(&mut self, addr: u16) -> u8 {
        // PRG-RAM and BIOS reads have no side effects.
        if (0x6000..0xE000).contains(&addr) {
            let idx = (addr as usize - 0x6000) & (PRG_RAM_SIZE - 1);
            return self.prg_ram[idx];
        }
        if addr >= 0xE000 {
            let idx = (addr as usize - 0xE000) & (BIOS_SIZE - 1);
            return self.bios.get(idx).copied().unwrap_or(0);
        }
        // Disk I/O registers ($4020-$4033).
        if Self::is_disk_register(addr) {
            return self.read_disk_register(addr);
        }
        // Audio read registers ($4090, $4092).
        if matches!(addr, 0x4090 | 0x4092) {
            return self.audio.read_register(addr);
        }
        0x00
    }

    fn write_prg(&mut self, addr: u16, value: u8) {
        // $6000-$DFFF: PRG-RAM (writable).
        if (0x6000..0xE000).contains(&addr) {
            let idx = (addr as usize - 0x6000) & (PRG_RAM_SIZE - 1);
            self.prg_ram[idx] = value;
            return;
        }
        // $4020-$4033: Disk I/O registers.
        if Self::is_disk_register(addr) {
            self.write_disk_register(addr, value);
            return;
        }
        // $4040-$408A: FDS audio registers.
        if Self::is_audio_register(addr) {
            self.audio.write_register(addr, value);
        }
        // BIOS ROM area ($E000-$FFFF) is read-only.
    }

    fn read_chr(&self, addr: u16) -> u8 {
        let idx = (addr as usize) & (CHR_RAM_SIZE - 1);
        self.chr_ram[idx]
    }

    fn write_chr(&mut self, addr: u16, value: u8) {
        let idx = (addr as usize) & (CHR_RAM_SIZE - 1);
        self.chr_ram[idx] = value;
    }

    fn mirror_mode(&self) -> Mirroring {
        if self.mirror_horizontal {
            Mirroring::Horizontal
        } else {
            Mirroring::Vertical
        }
    }

    fn irq_pending(&self) -> bool {
        self.timer_irq_pending
    }

    /// Clock the FDS timer and audio by `cpu_cycles` CPU cycles.
    /// The timer counts down every CPU cycle; the audio runs at APU
    /// clock = CPU clock / 2.
    fn clock_cpu(&mut self, cpu_cycles: u32) {
        // Timer IRQ: counts down every CPU cycle when enabled.
        // The IRQ fires when the counter reaches 0 and is reloaded
        // from the latch (matching the FDS timer behavior).
        if self.timer_enable && self.io_enable_timer {
            for _ in 0..cpu_cycles {
                if self.timer_counter == 0 {
                    self.timer_counter = self.timer_latch;
                    self.timer_irq_pending = true;
                } else {
                    self.timer_counter -= 1;
                }
            }
        }
        // Audio runs at APU clock = CPU clock / 2.
        self.audio.clock(cpu_cycles / 2);
    }

    /// FDS expansion audio sample (wavetable + modulator).
    fn expansion_audio_sample(&self) -> f32 {
        // We need &mut self for sample() (it reads phase accumulators
        // that are advanced by clock()). Since expansion_audio_sample
        // takes &self, we return the last computed sample. The audio
        // state is advanced by clock_cpu; the sample is read here.
        // We use a simple approach: compute the sample from current state
        // without mutation. The phase accumulators are advanced by
        // clock_cpu, so the sample reflects the current phase.
        //
        // To avoid the &mut requirement, we duplicate the sample logic
        // here as a read-only computation.
        if self.audio_freq() == 0 {
            return 0.0;
        }
        let wave_idx = ((self.audio_phase() >> 12) & 0x3F) as usize;
        let wave_val = self.audio_wave_val(wave_idx) as i16;
        let vol = self.audio_volume() as i16;
        let sample = (wave_val - 32) * vol;
        let normalized = sample as f32 / 2016.0;
        normalized.clamp(-1.0, 1.0)
    }

    fn save_state(&self) -> super::MapperState {
        super::MapperState::Fds(self.clone())
    }

    fn restore_state(&mut self, state: super::MapperState) {
        match state {
            super::MapperState::Fds(m) => {
                // Preserve BIOS and disk_data (they are #[serde(skip)]
                // and not part of the serialised state). We keep the
                // current BIOS and disk_data, restoring only the
                // mutable state.
                let bios = std::mem::take(&mut self.bios);
                let disk_data = std::mem::take(&mut self.disk_data);
                *self = m;
                self.bios = bios;
                self.disk_data = disk_data;
            }
            _ => panic!("Fds::restore_state: expected Fds variant"),
        }
    }
}

// ---- Read-only accessors for expansion_audio_sample (&self) ----

impl Fds {
    fn audio_freq(&self) -> u16 {
        // Access the private freq field via a method on FdsAudio.
        // Since FdsAudio doesn't expose freq, we read it here via
        // a helper. We add a pub(crate) accessor to FdsAudio.
        self.audio.freq_value()
    }

    fn audio_phase(&self) -> u32 {
        self.audio.phase_value()
    }

    fn audio_wave_val(&self, idx: usize) -> u8 {
        self.audio.wave_ram()[idx]
    }

    fn audio_volume(&self) -> u8 {
        self.audio.volume_gain_value()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a minimal FDS mapper with a small BIOS for testing.
    fn make_fds() -> Fds {
        let mut bios = vec![0u8; BIOS_SIZE];
        // Set reset vector to $E000.
        bios[0x1FFC] = 0x00;
        bios[0x1FFD] = 0xE0;
        // Fill with NOPs.
        for b in &mut bios[..0x1FFC] {
            *b = 0xEA;
        }
        let disk_data = vec![0x80, b'F', b'D', b'S', 0x00]; // minimal disk header
        Fds::new(bios, disk_data)
    }

    #[test]
    fn prg_ram_reads_zero_initially() {
        let fds = make_fds();
        assert_eq!(fds.read_prg(0x6000), 0);
        assert_eq!(fds.read_prg(0xDFFF), 0);
    }

    #[test]
    fn prg_ram_writes_persist() {
        let mut fds = make_fds();
        fds.write_prg(0x6000, 0x42);
        assert_eq!(fds.read_prg(0x6000), 0x42);
        fds.write_prg(0xDFFF, 0xAB);
        assert_eq!(fds.read_prg(0xDFFF), 0xAB);
    }

    #[test]
    fn bios_reads_return_bios_data() {
        let fds = make_fds();
        // BIOS is filled with NOPs (0xEA) except the reset vector.
        assert_eq!(fds.read_prg(0xE000), 0xEA);
        assert_eq!(fds.read_prg(0xE001), 0xEA);
    }

    #[test]
    fn bios_reset_vector_at_fffc() {
        let fds = make_fds();
        // $FFFC = BIOS offset 0x1FFC = 0x00, $FFFD = 0xE0 → reset to $E000.
        assert_eq!(fds.read_prg(0xFFFC), 0x00);
        assert_eq!(fds.read_prg(0xFFFD), 0xE0);
    }

    #[test]
    fn chr_ram_writes_persist() {
        let mut fds = make_fds();
        fds.write_chr(0x0000, 0x55);
        assert_eq!(fds.read_chr(0x0000), 0x55);
        fds.write_chr(0x1FFF, 0xAA);
        assert_eq!(fds.read_chr(0x1FFF), 0xAA);
    }

    #[test]
    fn default_mirroring_is_vertical() {
        let fds = make_fds();
        assert_eq!(fds.mirror_mode(), Mirroring::Vertical);
    }

    #[test]
    fn mirroring_switches_via_4025() {
        let mut fds = make_fds();
        fds.write_prg(0x4025, 0x02); // bit 1 = horizontal
        assert_eq!(fds.mirror_mode(), Mirroring::Horizontal);
        fds.write_prg(0x4025, 0x00); // bit 1 clear = vertical
        assert_eq!(fds.mirror_mode(), Mirroring::Vertical);
    }

    #[test]
    fn timer_irq_latch_and_enable() {
        let mut fds = make_fds();
        fds.write_prg(0x4020, 0x10); // latch low
        fds.write_prg(0x4021, 0x00); // latch high → latch = 0x0010
        fds.write_prg(0x4023, 0x02); // enable timer I/O
        fds.write_prg(0x4022, 0x01); // enable timer IRQ
        assert!(fds.timer_enable);
        assert_eq!(fds.timer_latch, 0x0010);
        assert_eq!(fds.timer_counter, 0x0010);
    }

    #[test]
    fn timer_irq_fires_after_countdown() {
        let mut fds = make_fds();
        fds.write_prg(0x4020, 0x05); // latch low = 5
        fds.write_prg(0x4021, 0x00); // latch high = 0
        fds.write_prg(0x4023, 0x02); // enable timer I/O
        fds.write_prg(0x4022, 0x01); // enable timer
                                     // Counter starts at 5. After 5 decrements it reaches 0; the
                                     // 6th cycle detects 0 and fires the IRQ (reloading the latch).
        fds.clock_cpu(6);
        assert!(fds.irq_pending());
    }

    #[test]
    fn timer_irq_cleared_on_4030_read() {
        let mut fds = make_fds();
        fds.write_prg(0x4020, 0x01);
        fds.write_prg(0x4021, 0x00);
        fds.write_prg(0x4023, 0x02);
        fds.write_prg(0x4022, 0x01);
        // Latch=1: counter=1→0 (1 cycle), then 0 detected → IRQ (2 cycles).
        fds.clock_cpu(2);
        assert!(fds.irq_pending());
        // Read $4030 → clears IRQ flag.
        let status = fds.read_disk_register(0x4030);
        assert!(status & 0x80 != 0); // bit 7 was set
        assert!(!fds.irq_pending()); // now cleared
    }

    #[test]
    fn disk_read_returns_data_sequentially() {
        let mut fds = make_fds();
        // disk_data = [0x80, 'F', 'D', 'S', 0x00]
        fds.write_prg(0x4025, 0x20); // motor on, read mode
        let b0 = fds.read_disk_register(0x4031);
        assert_eq!(b0, 0x80);
        let b1 = fds.read_disk_register(0x4031);
        assert_eq!(b1, b'F');
        let b2 = fds.read_disk_register(0x4031);
        assert_eq!(b2, b'D');
    }

    #[test]
    fn disk_transfer_reset_resets_position() {
        let mut fds = make_fds();
        fds.write_prg(0x4025, 0x20); // motor on
                                     // Read a few bytes.
        let _ = fds.read_disk_register(0x4031);
        let _ = fds.read_disk_register(0x4031);
        assert!(fds.disk_read_pos > 0);
        // Transfer reset.
        fds.write_prg(0x4025, 0x21); // bit 0 = reset
        assert_eq!(fds.disk_read_pos, 0);
    }

    #[test]
    fn disk_status_shows_motor_and_data() {
        let mut fds = make_fds();
        fds.write_prg(0x4025, 0x20); // motor on
        let status = fds.read_disk_register(0x4030);
        // bit 2 = disk ready (motor on + inserted).
        assert!(status & 0x04 != 0);
        // bit 0 = data available.
        assert!(status & 0x01 != 0);
    }

    #[test]
    fn audio_register_writes_route_to_audio() {
        let mut fds = make_fds();
        // Write to wave RAM via $4040.
        fds.write_prg(0x4040, 0x3F);
        assert_eq!(fds.audio.wave_ram()[0], 0x3F);
    }

    #[test]
    fn expansion_audio_sample_silent_when_freq_zero() {
        let fds = make_fds();
        assert_eq!(fds.expansion_audio_sample(), 0.0);
    }

    #[test]
    fn expansion_audio_sample_nonzero_with_freq() {
        let mut fds = make_fds();
        // Set up wave RAM with a pattern.
        for _ in 0..64 {
            fds.write_prg(0x4040, 0x3F);
        }
        // Set volume (envelope disabled, vol = 0x3F).
        fds.write_prg(0x4080, 0x7F);
        // Set frequency.
        fds.write_prg(0x4082, 0x00);
        fds.write_prg(0x4081, 0x01);
        // Clock audio.
        fds.clock_cpu(200);
        let s = fds.expansion_audio_sample();
        // Wave value 0x3F, centered → (0x3F - 32) * 0x3F / 2016 > 0
        assert!(s > 0.0, "expected positive sample, got {s}");
    }

    #[test]
    fn save_restore_preserves_prg_ram() {
        let mut fds = make_fds();
        fds.write_prg(0x6000, 0x99);
        fds.write_prg(0x8000, 0x88);
        let state = fds.save_state();
        let mut fds2 = make_fds();
        fds2.restore_state(state);
        assert_eq!(fds2.read_prg(0x6000), 0x99);
        assert_eq!(fds2.read_prg(0x8000), 0x88);
    }

    #[test]
    fn save_restore_preserves_chr_ram() {
        let mut fds = make_fds();
        fds.write_chr(0x1000, 0x77);
        let state = fds.save_state();
        let mut fds2 = make_fds();
        fds2.restore_state(state);
        assert_eq!(fds2.read_chr(0x1000), 0x77);
    }

    #[test]
    fn save_restore_preserves_bios_and_disk() {
        let fds = make_fds();
        // fds2 starts with the same BIOS so we can verify the restored
        // state (which preserves fds2's own BIOS, since BIOS is
        // #[serde(skip)]).
        let mut fds2 = make_fds();
        let state = fds.save_state();
        fds2.restore_state(state);
        // BIOS should be readable (preserved from fds2's own BIOS).
        assert_eq!(fds2.read_prg(0xE000), 0xEA);
        // Disk data should be preserved (restored from the saved state).
        fds2.write_prg(0x4025, 0x20); // motor on
        assert_eq!(fds2.read_disk_register(0x4031), 0x80);
    }
}
