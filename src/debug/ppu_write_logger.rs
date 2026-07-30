//! PPU register write logger — records every PPU register write and
//! OAM DMA with the PPU scanline/cycle at the time of the write, to a
//! file for offline timing-race analysis.
//!
//! When enabled, the bus calls [`PpuWriteLogger::log_write`] on every
//! PPU register write (`$2000-$2007`) and [`PpuWriteLogger::log_oam_dma`]
//! on every OAM DMA (`$4014`). Each line includes the register name,
//! value, PPU scanline, PPU cycle, and the CPU PC at the time of the
//! write (when available), making it possible to correlate PPU write
//! timing with the CPU instruction trace.
//!
//! The log format is:
//! ```text
//! # ppu write log — reg value scanline cycle [pc]
//! PPUCTRL  80  SL:241 CYC:1   PC:C004
//! PPUSCROLL 00  SL:241 CYC:3   PC:C008
//! OAMDMA   02  SL:241 CYC:5   PC:C00C
//! ```
//!
//! See: https://www.nesdev.org/wiki/PPU_registers

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

/// Register name lookup for PPU registers 0-7.
const PPU_REG_NAMES: [&str; 8] = [
    "PPUCTRL",
    "PPUMASK",
    "PPUSTATUS",
    "OAMADDR",
    "OAMDATA",
    "PPUSCROLL",
    "PPUADDR",
    "PPUDATA",
];

/// A PPU register write logger that writes one line per PPU register
/// write or OAM DMA to a file. The logger owns a buffered writer;
/// flushing happens on [`PpuWriteLogger::stop`] or when the buffer fills.
///
/// The logger is *opt-in*: when `enabled` is false (the default),
/// [`PpuWriteLogger::log_write`] and [`PpuWriteLogger::log_oam_dma`] are
/// no-ops.
pub struct PpuWriteLogger {
    /// Open file writer, or `None` when logging is stopped.
    writer: Option<BufWriter<File>>,
    /// Path of the currently-open log file (for the stop message).
    path: PathBuf,
    /// Whether logging is currently active.
    enabled: bool,
    /// Lines written so far (for the stop summary).
    line_count: u64,
}

impl Default for PpuWriteLogger {
    fn default() -> Self {
        Self::new()
    }
}

impl PpuWriteLogger {
    /// Construct a stopped logger with no open file.
    pub fn new() -> Self {
        Self {
            writer: None,
            path: PathBuf::new(),
            enabled: false,
            line_count: 0,
        }
    }

    /// Is logging currently active?
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Path of the currently-open log file, or an empty path when stopped.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Number of lines written since `start`.
    pub fn line_count(&self) -> u64 {
        self.line_count
    }

    /// Start logging to `path`. Opens (or truncates) the file and writes
    /// a header line. Returns an error if the file cannot be opened.
    /// Calling `start` while already enabled first stops the current
    /// log (flushing the previous file).
    pub fn start<P: AsRef<Path>>(&mut self, path: P) -> std::io::Result<()> {
        if self.enabled {
            self.stop();
        }
        if let Some(parent) = path.as_ref().parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let file = File::create(path.as_ref())?;
        let mut writer = BufWriter::new(file);
        writeln!(writer, "# ppu write log — reg value scanline cycle [pc]")?;
        self.path = path.as_ref().to_path_buf();
        self.writer = Some(writer);
        self.enabled = true;
        self.line_count = 0;
        Ok(())
    }

    /// Stop logging. Flushes and closes the file. Returns the number of
    /// lines written. No-op when already stopped.
    pub fn stop(&mut self) -> u64 {
        if let Some(mut w) = self.writer.take() {
            let _ = w.flush();
        }
        self.enabled = false;
        let lines = self.line_count;
        self.line_count = 0;
        self.path = PathBuf::new();
        lines
    }

    /// Log a PPU register write. `reg` is the de-mirrored register index
    /// (0-7), `value` is the written byte, `scanline` and `cycle` are the
    /// current PPU position, and `pc` is the CPU program counter (or
    /// `None` if not available). No-op when logging is disabled.
    pub fn log_write(
        &mut self,
        reg: u16,
        value: u8,
        scanline: u16,
        cycle: u16,
        vram_addr: u16,
        pc: Option<u16>,
    ) {
        if !self.enabled {
            return;
        }
        let name = PPU_REG_NAMES[(reg & 0x07) as usize];
        let pc_str = match pc {
            Some(p) => format!("  PC:{:04X}", p),
            None => String::new(),
        };
        let line = format!(
            "{:<10} {:02X}  SL:{:>3} CYC:{:>3} V:{:04X}{}\n",
            name, value, scanline, cycle, vram_addr, pc_str
        );
        if let Some(w) = self.writer.as_mut() {
            let _ = w.write_all(line.as_bytes());
        }
        self.line_count = self.line_count.saturating_add(1);
    }

    /// Log an OAM DMA ($4014) write. `page` is the DMA page value,
    /// `scanline` and `cycle` are the current PPU position.
    pub fn log_oam_dma(
        &mut self,
        page: u8,
        scanline: u16,
        cycle: u16,
        _vram_addr: u16,
        pc: Option<u16>,
    ) {
        if !self.enabled {
            return;
        }
        let pc_str = match pc {
            Some(p) => format!("  PC:{:04X}", p),
            None => String::new(),
        };
        let line = format!(
            "OAMDMA     {:02X}  SL:{:>3} CYC:{:>3}{}\n",
            page, scanline, cycle, pc_str
        );
        if let Some(w) = self.writer.as_mut() {
            let _ = w.write_all(line.as_bytes());
        }
        self.line_count = self.line_count.saturating_add(1);
    }

    /// Flush the underlying writer without stopping the log. No-op when
    /// stopped.
    pub fn flush(&mut self) -> std::io::Result<()> {
        if let Some(w) = self.writer.as_mut() {
            w.flush()?;
        }
        Ok(())
    }
}
