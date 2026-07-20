//! Trace logger — writes a per-instruction execution trace to a file in
//! an FCEUX-style format, for console-based debugging.
//!
//! This is the M28 trace logger deliverable. When enabled, the main loop
//! calls [`TraceLogger::log_instruction`] after each CPU step; the
//! logger formats one line per instruction and writes it to a buffered
//! file. The format is:
//!
//! ```text
//! C000  78      SEI           A:00 X:00 Y:00 P:24 SP:FD CYC:0
//! ```
//!
//! Columns: PC, opcode bytes (padded to 6 chars), disassembly (padded to
//! 12 chars), A / X / Y / P / SP registers, and the cumulative CPU cycle
//! count since the trace started.
//!
//! The disassembly column is produced by the M27 disassembler
//! ([`crate::debug::disassemble_at`]), which reads via `Bus::peek` — so
//! logging is side-effect-free and cannot perturb the emulated machine.
//!
//! See: https://www.nesdev.org/wiki/Tracing
//! See: FCEUX trace format reference (https://fceux.com/web/help/fceux.html)

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::bus::Bus;
use crate::cpu::Cpu;
use crate::debug::disassemble_at;

/// A trace logger that writes one line per executed instruction to a
/// file. The logger owns a buffered writer; flushing happens on
/// [`TraceLogger::stop`] or when the buffer fills.
///
/// The logger is *opt-in*: when `enabled` is false (the default),
/// [`TraceLogger::log_instruction`] is a no-op. The main loop calls
/// `start` when the user presses F8 and `stop` when they press F8 again
/// (or when the emulator exits).
pub struct TraceLogger {
    /// Open file writer, or `None` when logging is stopped.
    writer: Option<BufWriter<File>>,
    /// Path of the currently-open log file (for the stop message).
    path: PathBuf,
    /// Whether logging is currently active. Distinct from `writer.is_some()`
    /// so the caller can query the state without borrowing the writer.
    enabled: bool,
    /// Cumulative CPU cycles since the trace started. Incremented by
    /// each `log_instruction` call.
    cycle_count: u64,
    /// Lines written so far (for the stop summary).
    line_count: u64,
}

impl Default for TraceLogger {
    fn default() -> Self {
        Self::new()
    }
}

impl TraceLogger {
    /// Construct a stopped logger with no open file.
    pub fn new() -> Self {
        Self {
            writer: None,
            path: PathBuf::new(),
            enabled: false,
            cycle_count: 0,
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

    /// Cumulative CPU cycles logged since `start`.
    pub fn cycle_count(&self) -> u64 {
        self.cycle_count
    }

    /// Number of instruction lines written since `start`.
    pub fn line_count(&self) -> u64 {
        self.line_count
    }

    /// Start logging to `path`. Opens (or truncates) the file and writes
    /// a header line. Returns an error if the file cannot be opened.
    /// Calling `start` while already enabled first stops the current
    /// trace (flushing the previous file).
    pub fn start<P: AsRef<Path>>(&mut self, path: P) -> std::io::Result<()> {
        if self.enabled {
            self.stop();
        }
        let file = File::create(path.as_ref())?;
        let mut writer = BufWriter::new(file);
        // Header line so the file is self-describing.
        writeln!(
            writer,
            "# nes-emu trace log — PC bytes disasm A X Y P SP CYC"
        )?;
        self.path = path.as_ref().to_path_buf();
        self.writer = Some(writer);
        self.enabled = true;
        self.cycle_count = 0;
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
        self.cycle_count = 0;
        self.path = PathBuf::new();
        lines
    }

    /// Log one instruction. Captures the disassembly at `cpu.pc` *before*
    /// the step (so the PC column reflects the instruction that was
    /// about to execute), then advances the cycle counter by `cycles`.
    /// No-op when logging is disabled.
    ///
    /// The caller passes the CPU and bus state captured *before* the
    /// `Cpu::step` call (or, equivalently, the state at the start of
    /// the instruction). `cycles` is the CPU cycle count returned by
    /// `Cpu::step` for this instruction.
    pub fn log_instruction(&mut self, cpu: &Cpu, bus: &Bus, cycles: u32) -> std::io::Result<()> {
        if !self.enabled {
            return Ok(());
        }
        let instr = disassemble_at(bus, cpu.pc);
        // Opcode bytes column: only the real instruction bytes, padded
        // to 8 chars (the width of "XX XX XX") with trailing spaces so
        // the register columns line up. Matches FCEUX's "bytes" column
        // convention — no phantom zero operands for short instructions.
        let mut bytes_col = String::new();
        for (i, &b) in instr.bytes[..instr.len as usize].iter().enumerate() {
            if i > 0 {
                bytes_col.push(' ');
            }
            bytes_col.push_str(&format!("{:02X}", b));
        }
        // Pad to 8 chars ("XX XX XX" = 8 chars wide).
        while bytes_col.len() < 8 {
            bytes_col.push(' ');
        }
        // Disasm column padded to a fixed width so the register columns
        // line up. 12 chars is enough for most instructions; longer ones
        // (e.g. `LDA $0310,X`) just push the columns slightly.
        let disasm_col = format!("{:<12}", instr.text);
        let line = format!(
            "{:04X}  {}  {}  A:{:02X} X:{:02X} Y:{:02X} P:{:02X} SP:{:02X} CYC:{}\n",
            cpu.pc,
            bytes_col,
            disasm_col,
            cpu.a,
            cpu.x,
            cpu.y,
            cpu.status,
            cpu.sp,
            self.cycle_count,
        );
        if let Some(w) = self.writer.as_mut() {
            w.write_all(line.as_bytes())?;
        }
        self.cycle_count = self.cycle_count.saturating_add(cycles as u64);
        self.line_count = self.line_count.saturating_add(1);
        Ok(())
    }

    /// Flush the underlying writer without stopping the trace. Useful
    /// for inspecting a partial trace while the emulator is still
    /// running. No-op when stopped.
    pub fn flush(&mut self) -> std::io::Result<()> {
        if let Some(w) = self.writer.as_mut() {
            w.flush()?;
        }
        Ok(())
    }
}

impl Drop for TraceLogger {
    fn drop(&mut self) {
        // Ensure the file is flushed even if the caller forgets to stop.
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::Cartridge;
    use crate::emulator::EmulatorState;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Monotonic counter for unique temp-file names within the test
    /// process. Avoids collisions between parallel test threads.
    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Build a minimal NROM-128 cartridge whose PRG is filled with NOP
    /// and whose RESET vector points at $C000.
    fn nop_cart() -> Cartridge {
        let mut bytes = vec![b'N', b'E', b'S', 0x1A, 1, 0, 0, 0];
        bytes.extend_from_slice(&[0u8; 8]);
        bytes.resize(16 + 16 * 1024, 0xEA);
        let reset_off = 16 + 0x3FFC;
        bytes[reset_off] = 0x00;
        bytes[reset_off + 1] = 0xC0;
        Cartridge::from_bytes(&bytes).expect("build NOP cart")
    }

    /// Create a unique temp file path under the system temp dir. The
    /// file is not opened; the caller is responsible for cleanup.
    fn temp_path(tag: &str) -> PathBuf {
        let n = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let mut p = std::env::temp_dir();
        p.push(format!("nes-emu-trace-{tag}-{pid}-{n}.log"));
        p
    }

    /// Remove a file if it exists (best-effort cleanup).
    fn cleanup(path: &Path) {
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn new_logger_is_stopped() {
        let l = TraceLogger::new();
        assert!(!l.is_enabled());
        assert_eq!(l.line_count(), 0);
    }

    #[test]
    fn start_opens_file_and_enables() {
        let path = temp_path("start");
        let mut l = TraceLogger::new();
        l.start(&path).expect("start");
        assert!(l.is_enabled());
        drop(l);
        let contents = std::fs::read_to_string(&path).expect("read back");
        assert!(contents.contains("# nes-emu trace log"));
        cleanup(&path);
    }

    #[test]
    fn log_instruction_writes_one_line_per_call() {
        let path = temp_path("lines");
        let mut l = TraceLogger::new();
        l.start(&path).expect("start");
        let mut emu = EmulatorState::new(nop_cart());
        emu.reset();
        // Log two instructions without actually stepping (the disasm
        // column reflects the current PC).
        l.log_instruction(emu.cpu(), emu.bus(), 2).expect("log1");
        l.log_instruction(emu.cpu(), emu.bus(), 2).expect("log2");
        assert_eq!(l.line_count(), 2);
        assert_eq!(l.cycle_count(), 4);
        drop(l);
        let contents = std::fs::read_to_string(&path).expect("read back");
        // Header + 2 instruction lines.
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines.len(), 3);
        // First instruction line: PC=$C000, opcode=$EA (NOP).
        assert!(lines[1].starts_with("C000  EA"));
        assert!(lines[1].contains("NOP"));
        assert!(lines[1].contains("CYC:0"));
        // Second line: cycle counter advanced to 2.
        assert!(lines[2].contains("CYC:2"));
        cleanup(&path);
    }

    #[test]
    fn stop_disables_and_resets_counters() {
        let path = temp_path("stop");
        let mut l = TraceLogger::new();
        l.start(&path).expect("start");
        let mut emu = EmulatorState::new(nop_cart());
        emu.reset();
        l.log_instruction(emu.cpu(), emu.bus(), 2).expect("log");
        let lines = l.stop();
        assert_eq!(lines, 1);
        assert!(!l.is_enabled());
        assert_eq!(l.line_count(), 0);
        assert_eq!(l.cycle_count(), 0);
        cleanup(&path);
    }

    #[test]
    fn log_instruction_is_noop_when_stopped() {
        let mut l = TraceLogger::new();
        let mut emu = EmulatorState::new(nop_cart());
        emu.reset();
        // Should not panic or write anything.
        l.log_instruction(emu.cpu(), emu.bus(), 2).expect("log");
        assert_eq!(l.line_count(), 0);
    }

    #[test]
    fn start_while_enabled_restarts_trace() {
        let path1 = temp_path("restart1");
        let path2 = temp_path("restart2");
        let mut l = TraceLogger::new();
        l.start(&path1).expect("start1");
        let mut emu = EmulatorState::new(nop_cart());
        emu.reset();
        l.log_instruction(emu.cpu(), emu.bus(), 2).expect("log1");
        // Restarting should flush path1 and start a fresh path2.
        l.start(&path2).expect("start2");
        assert_eq!(l.line_count(), 0);
        assert_eq!(l.cycle_count(), 0);
        assert!(l.is_enabled());
        drop(l);
        cleanup(&path1);
        cleanup(&path2);
    }
}
