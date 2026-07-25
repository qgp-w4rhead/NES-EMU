//! CPU debugger — register snapshot, breakpoints, and pause/step logic.
//!
//! Pure logic (side-effect-free reads via `Bus::peek`); driven by F1/F2/F3 hotkeys.

use crate::bus::Bus;
use crate::cpu::flags;
use crate::cpu::Cpu;

/// A snapshot of the CPU register file and status flags, captured for the
/// debugger's register view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RegisterSnapshot {
    /// Accumulator.
    pub a: u8,
    /// X index register.
    pub x: u8,
    /// Y index register.
    pub y: u8,
    /// Stack pointer (low byte; stack lives at `$0100..=$01FF`).
    pub sp: u8,
    /// Program counter.
    pub pc: u16,
    /// Raw status register byte (P).
    pub status: u8,
    /// Negative flag (bit 7).
    pub n: bool,
    /// Overflow flag (bit 6).
    pub v: bool,
    /// Break flag (bit 4 — only meaningful in the stack-pushed copy).
    pub b: bool,
    /// Decimal flag (bit 3 — no effect on the NES 2A03).
    pub d: bool,
    /// Interrupt-disable flag (bit 2).
    pub i: bool,
    /// Zero flag (bit 1).
    pub z: bool,
    /// Carry flag (bit 0).
    pub c: bool,
}

impl RegisterSnapshot {
    /// Capture a snapshot from a CPU.
    pub fn from_cpu(cpu: &Cpu) -> Self {
        let s = cpu.status;
        Self {
            a: cpu.a,
            x: cpu.x,
            y: cpu.y,
            sp: cpu.sp,
            pc: cpu.pc,
            status: s,
            n: (s & flags::N) != 0,
            v: (s & flags::V) != 0,
            b: (s & flags::B) != 0,
            d: (s & flags::D) != 0,
            i: (s & flags::I) != 0,
            z: (s & flags::Z) != 0,
            c: (s & flags::C) != 0,
        }
    }

    /// Format the snapshot as a single-line register dump, e.g.
    /// `A:44 X:05 Y:00 SP:FD PC:C000 P:NV-BDIZC N--BD-IC` (the second
    /// column mirrors the symbolic row, with `-` for clear flags).
    pub fn to_line(&self) -> String {
        // Two status-flag rows: the symbolic bit names and the live state.
        let sym = "NV-BDIZC";
        let mut live = String::with_capacity(8);
        for (i, ch) in sym.chars().enumerate() {
            let bit = 7 - i;
            if (self.status >> bit) & 1 == 1 {
                live.push(ch);
            } else {
                live.push('-');
            }
        }
        format!(
            "A:{:02X} X:{:02X} Y:{:02X} SP:{:02X} PC:{:04X} P:{} {}",
            self.a, self.x, self.y, self.sp, self.pc, sym, live,
        )
    }
}

impl std::fmt::Display for RegisterSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_line())
    }
}

/// A breakpoint condition.
///
/// - [`Breakpoint::Address`] fires when the CPU is about to fetch an
///   instruction at the given address (checked before each `Cpu::step`).
/// - [`Breakpoint::Value`] fires when the byte at `addr` equals `value`
///   (checked before each `Cpu::step`, so the breakpoint fires before the
///   instruction that would have observed the value runs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Breakpoint {
    /// Pause when the PC reaches this address.
    Address(u16),
    /// Pause when the byte at `addr` equals `value`.
    Value { addr: u16, value: u8 },
}

impl Breakpoint {
    /// Returns `true` if this breakpoint is currently satisfied.
    ///
    /// `Address` breakpoints match against `cpu.pc`. `Value` breakpoints
    /// read `addr` via [`Bus::peek`] (no side-effects).
    pub fn matches(&self, cpu: &Cpu, bus: &Bus) -> bool {
        match *self {
            Breakpoint::Address(a) => cpu.pc == a,
            Breakpoint::Value { addr, value } => bus.peek(addr) == value,
        }
    }
}

impl std::fmt::Display for Breakpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Breakpoint::Address(a) => write!(f, "ADDR ${a:04X}"),
            Breakpoint::Value { addr, value } => {
                write!(f, "VALUE ${addr:04X}==${value:02X}")
            }
        }
    }
}

/// The CPU debugger — breakpoint set + pause / step bookkeeping.
///
/// The main loop drives it like this:
///
/// 1. Before each `Cpu::step`, call [`CpuDebugger::check_before_step`].
///    If it returns `true`, the loop stops stepping (the debugger has
///    either been paused by the user or hit a breakpoint).
/// 2. When the user presses F2 (single-step), call
///    [`CpuDebugger::consume_step_request`]; if it returns `true`, run
///    exactly one `Cpu::step` then pause again.
/// 3. F1 toggles [`CpuDebugger::toggle_pause`]; F3 toggles
///    [`CpuDebugger::toggle_run_to_breakpoint`].
pub struct CpuDebugger {
    /// Active breakpoints.
    breakpoints: Vec<Breakpoint>,
    /// True when the emulator is paused (no CPU stepping until resumed).
    paused: bool,
    /// True when the user requested a single-step (run one instruction,
    /// then re-pause). Consumed by [`consume_step_request`].
    step_request: bool,
    /// True when run-to-breakpoint mode is active (the loop runs at full
    /// speed but stops as soon as a breakpoint matches). When false, the
    /// loop runs at full speed and breakpoints are ignored.
    run_to_breakpoint: bool,
    /// The breakpoint that last fired, if any. Cleared on resume.
    last_hit: Option<Breakpoint>,
    /// A breakpoint to suppress for one `check_before_step` iteration.
    /// Set when resuming from a breakpoint hit so the just-fired BP does
    /// not re-fire immediately — this matters for `Value` breakpoints,
    /// where stepping one instruction may not change the watched byte.
    /// Cleared after one iteration (whether or not it matched).
    suppress_bp: Option<Breakpoint>,
}

impl Default for CpuDebugger {
    fn default() -> Self {
        Self::new()
    }
}

impl CpuDebugger {
    /// Construct an inactive debugger with no breakpoints.
    pub fn new() -> Self {
        Self {
            breakpoints: Vec::new(),
            paused: false,
            step_request: false,
            run_to_breakpoint: false,
            last_hit: None,
            suppress_bp: None,
        }
    }

    /// Add a breakpoint.
    pub fn add(&mut self, bp: Breakpoint) {
        if !self.breakpoints.contains(&bp) {
            self.breakpoints.push(bp);
        }
    }

    /// Remove the first breakpoint equal to `bp`. Returns `true` if a
    /// breakpoint was removed.
    pub fn remove(&mut self, bp: Breakpoint) -> bool {
        if let Some(pos) = self.breakpoints.iter().position(|b| *b == bp) {
            self.breakpoints.remove(pos);
            true
        } else {
            false
        }
    }

    /// Remove all breakpoints.
    pub fn clear(&mut self) {
        self.breakpoints.clear();
    }

    /// Borrow the active breakpoints.
    pub fn breakpoints(&self) -> &[Breakpoint] {
        &self.breakpoints
    }

    /// Toggle the paused state. Resuming after a breakpoint hit implicitly
    /// requests a single-step so the CPU advances past the breakpoint
    /// instruction before breakpoint checks resume, *and* suppresses the
    /// just-fired breakpoint for one iteration. The suppression is
    /// necessary for `Value` breakpoints, where stepping one instruction
    /// may not change the watched byte — without it the breakpoint would
    /// re-fire immediately on resume and the user could never make
    /// progress.
    pub fn toggle_pause(&mut self) {
        let was_paused = self.paused;
        self.paused = !self.paused;
        if was_paused && !self.paused {
            // Resuming.
            if let Some(bp) = self.last_hit {
                // Step past the breakpoint instruction before re-enabling
                // breakpoint checks, and suppress the just-hit BP for one
                // iteration so a Value BP whose watched byte has not yet
                // changed does not re-fire immediately.
                self.step_request = true;
                self.suppress_bp = Some(bp);
            } else {
                self.step_request = false;
                self.suppress_bp = None;
            }
            self.last_hit = None;
        } else if !was_paused && self.paused {
            // Pausing.
            self.step_request = false;
            self.suppress_bp = None;
        }
    }

    /// Explicitly set the paused state.
    pub fn set_paused(&mut self, paused: bool) {
        if self.paused != paused {
            self.toggle_pause();
        }
    }

    /// Request a single-step: run one instruction, then re-pause.
    /// Implies the paused state (the loop must be paused for stepping to
    /// make sense). Does *not* clear `last_hit` — if the user single-steps
    /// past a breakpoint and then presses F1 to resume, the resume path
    /// still needs `last_hit` to set up `suppress_bp` so the just-hit
    /// breakpoint does not re-fire.
    pub fn request_step(&mut self) {
        self.paused = true;
        self.step_request = true;
    }

    /// Consume a pending single-step request. Returns `true` if the loop
    /// should run exactly one `Cpu::step` and then re-pause.
    pub fn consume_step_request(&mut self) -> bool {
        if self.step_request {
            self.step_request = false;
            true
        } else {
            false
        }
    }

    /// Toggle run-to-breakpoint mode. When enabled, the loop runs at full
    /// speed but stops as soon as a breakpoint matches.
    pub fn toggle_run_to_breakpoint(&mut self) {
        self.run_to_breakpoint = !self.run_to_breakpoint;
        if !self.run_to_breakpoint {
            self.last_hit = None;
        }
    }

    /// Is the debugger currently paused?
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Is run-to-breakpoint mode active?
    pub fn run_to_breakpoint(&self) -> bool {
        self.run_to_breakpoint
    }

    /// The breakpoint that last fired, if any. Cleared on resume.
    pub fn last_hit(&self) -> Option<Breakpoint> {
        self.last_hit
    }

    /// Decide whether the main loop should pause *before* executing the
    /// next CPU instruction. Returns `true` if the loop should stop
    /// stepping (either because the user paused, or a breakpoint matched).
    ///
    /// When the user has requested a single-step, this returns `false`
    /// exactly once (so the loop runs one instruction) and then re-pauses.
    /// When paused without a step request, always returns `true`.
    /// When run-to-breakpoint is active, returns `true` iff a breakpoint
    /// matches.
    pub fn check_before_step(&mut self, cpu: &Cpu, bus: &Bus) -> bool {
        // A pending single-step lets exactly one instruction through.
        if self.consume_step_request() {
            return false;
        }
        if self.paused {
            return true;
        }
        if self.run_to_breakpoint {
            // Drop a one-shot suppression set by `toggle_pause` on resume.
            // It applies to exactly one `check_before_step` iteration
            // after the step-past instruction has run.
            let suppress = self.suppress_bp.take();
            for bp in &self.breakpoints {
                if bp.matches(cpu, bus) {
                    if suppress == Some(*bp) {
                        // This is the breakpoint we just hit — skip it
                        // this iteration so it does not re-fire on resume.
                        continue;
                    }
                    self.last_hit = Some(*bp);
                    self.paused = true;
                    return true;
                }
            }
        } else {
            // Not in run-to-breakpoint mode — clear any stale suppression.
            self.suppress_bp = None;
        }
        false
    }
}
