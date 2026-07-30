//! M30 save-state / rewind / OSD hotkey dispatcher.
//!
//! Centralizes the M30 control-feature key handling so `src/main.rs` stays
//! under the 400-line file-size limit. The dispatcher owns the
//! [`SaveStateSlots`] bank, the [`RewindBuffer`], and the [`Osd`] state;
//! it does *not* own any SDL2 state — the `EmulatorState` is borrowed per
//! call.
//!
//! # Hotkeys
//!
//! | Key            | Action                                                  |
//! |----------------|---------------------------------------------------------|
//! | `F5`           | Save state to current slot.                             |
//! | `F7`           | Load state from current slot.                           |
//! | `F10`          | Toggle on-screen display (FPS / mapper / game / slot).  |
//! | `Backspace`    | Rewind one frame (pop the rewind buffer).               |
//! | `1..=9`        | Select save state slot 0..=8.                           |
//! | `0`            | Select save state slot 9.                               |
//!
//! Modifier guards are strict: the number keys are only intercepted when
//! no `Ctrl` / `Alt` / `Shift` modifier is held, so `Ctrl+1`, `Alt+1`,
//! and `Shift+1` still route to the joypad (some users bind NES buttons
//! to number keys). `F5` / `F7` / `F10` / `Backspace` are reserved
//! unconditionally — do not bind NES buttons to them in `config.toml`.
//!
//! See: https://www.nesdev.org/wiki/Save_state (slot conventions)
//! See: [`crate::save_state`] for the slot + rewind data structures.
//! See: [`crate::osd`] for the on-screen display renderer.

use sdl2::keyboard::{Keycode, Mod};

use crate::emulator::EmulatorState;
use crate::osd::{build_lines, Osd};
use crate::save_state::{RewindBuffer, SaveStateError, SaveStateSlots};

/// Number of emulator frames between rewind buffer snapshots. 2 = every
/// other frame (halves the per-frame serialisation cost while keeping
/// ~2 s of rewind depth at the default capacity of 60). Set to 1 for
/// maximum granularity.
pub const REWIND_INTERVAL: u32 = 2;

/// Default maximum number of alternate timeline branches.
pub const DEFAULT_MAX_BRANCHES: usize = 5;

/// Default duration of the delay before the 3-2-1 countdown starts,
/// in frames at 60 FPS. 30 frames = 0.5 seconds. Configurable at
/// runtime via [`SaveStateHotkeys::set_countdown_delay_ms`].
const DEFAULT_COUNTDOWN_DELAY_FRAMES: u32 = 30;

/// Maximum countdown delay in milliseconds (3 seconds).
pub const MAX_COUNTDOWN_DELAY_MS: u32 = 3000;

/// Maximum value for the countdown start number (10 → counts 10-9-...-1).
pub const MAX_COUNTDOWN_START_NUMBER: u32 = 10;

/// Default countdown start number (3 → counts 3-2-1).
const DEFAULT_COUNTDOWN_START_NUMBER: u32 = 3;

/// FPS used for converting milliseconds to frame counts.
const COUNTDOWN_FPS: u32 = 60;

/// Duration of each countdown number (3, 2, 1) in frames at 60 FPS.
/// 60 frames = 1 second per number.
const COUNTDOWN_NUMBER_FRAMES: u32 = 60;

/// State machine for the 3-2-1 resume countdown.
///
/// After the user releases Backspace/Shift (stops rewinding/forwarding),
/// the emulator pauses and enters `Delay`. After a configurable delay
/// (default 0.5 seconds), it transitions to `Countdown` showing 3 → 2 → 1 (1 second each), then resumes.
/// If the user presses rewind/forward again during Delay or Countdown,
/// the sequence is interrupted. Tab skips the delay and countdown
/// entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CountdownState {
    /// No countdown active — emulation runs normally.
    Idle,
    /// Waiting period before the 3-2-1 countdown starts
    /// (configurable, default 0.5 seconds).
    Delay,
    /// Showing the 3-2-1 countdown. `number` is 3, 2, or 1.
    Countdown { number: u32 },
}

/// An alternate timeline branch. Created when the user releases Backspace
/// after rewinding/forwarding. Each branch has its own rewind and forward
/// buffers, and records a divergence point on its parent timeline.
struct TimelineBranch {
    /// Position on the parent timeline (rewind buffer length) where this
    /// branch diverges.
    diverge_pos: usize,
    /// Rewind buffer for this branch (past states within the branch).
    rewind: RewindBuffer,
    /// Forward buffer for this branch (future states from the diverge
    /// point — the branch's recorded history that gets replayed via
    /// forward_step when the branch is accepted).
    forward: RewindBuffer,
    /// Index of parent branch in `branches`, or `None` if the parent is
    /// the root timeline.
    parent: Option<usize>,
}

/// Bundle of M30 save-state / rewind / OSD state. Held by the main loop;
/// [`SaveStateHotkeys::handle_key`] dispatches key events to the
/// appropriate action.
pub struct SaveStateHotkeys {
    /// 10 save state slots (F5 save / F7 load).
    pub slots: SaveStateSlots,
    /// Ring buffer of recent snapshots for the rewind feature
    /// (Backspace pops one frame).
    pub rewind: RewindBuffer,
    /// Forward buffer — snapshots popped during rewind are kept here so
    /// the user can fast-forward back (Shift+Backspace). Cleared on
    /// normal forward emulation.
    pub forward: RewindBuffer,
    /// On-screen display state (F10 toggles).
    pub osd: Osd,
    /// Game name shown in the OSD (ROM file stem, set at startup).
    pub game_name: String,
    /// Frame counter used to throttle rewind snapshots to every
    /// `REWIND_INTERVAL`-th frame.
    frame_counter: u32,
    /// Whether the rewind key (Backspace) is currently held. When true,
    /// the main loop pops from the rewind buffer instead of stepping the
    /// emulator forward, and `maybe_push_rewind` skips pushing new
    /// snapshots so the buffer is not polluted while rewinding.
    rewinding: bool,
    /// Whether the forward key (Shift+Backspace) is currently held. When
    /// true, the main loop pops from the forward buffer to advance
    /// through previously-rewound states.
    forwarding: bool,
    /// Current rewind speed multiplier (1, 2, 4, 8). Cycled by pressing
    /// Space while rewinding. Controls how many snapshots are popped per
    /// vsync tick.
    rewind_speed: u32,
    /// Animation frame counter for the timeline overlay (pulse + fade).
    /// Incremented each render call while the timeline is visible.
    anim_frame: u32,
    /// Current opacity level for the timeline overlay (0..=MAX_OPACITY).
    /// Ramps up when rewind starts, ramps down when it stops.
    timeline_opacity: u8,
    /// Set to `true` when rewinding or forwarding stops. The next time
    /// `maybe_push_rewind` pushes a new snapshot (i.e. the emulator has
    /// stepped forward), the forward buffer is cleared and the flag is
    /// reset. This gives the user an unlimited window to press
    /// Shift+Backspace between releasing Backspace and resuming play.
    forward_dirty: bool,
    /// Alternate timeline branches. Each branch stores its recorded
    /// history in the `forward` buffer (future states from the diverge
    /// point). All branches are preserved so the user can switch between
    /// sibling timelines. Limited to `max_branches`.
    branches: Vec<TimelineBranch>,
    /// Index of the currently active branch, or `None` if on the root
    /// timeline. When `Some`, `maybe_push_rewind` pushes to the branch's
    /// rewind buffer instead of the root.
    active_branch: Option<usize>,
    /// Maximum number of alternate branches.
    max_branches: usize,
    /// Index into `branches` if currently paused at a branch point
    /// (a divergence point of a child branch on the current timeline).
    /// When `Some`, the main loop should not step the emulator and
    /// should wait for the user to press o (accept) or p (deny).
    paused_at_branch: Option<usize>,
    /// When paused at a branch point with multiple sibling branches,
    /// this tracks which branch is currently visually selected.
    /// Up/Down arrows cycle through sibling branches at the same
    /// diverge position. O accepts the selected branch.
    selected_branch: Option<usize>,
    /// Branch indices that have been denied at the current rewind
    /// position. Used to skip past sibling branches at the same
    /// diverge point so the user can cycle through them with P.
    /// Cleared when the rewind position changes.
    denied_branches: Vec<usize>,
    /// True when paused at the beginning of a branch (rewind buffer
    /// empty on a non-root timeline). Accepting returns to the parent
    /// timeline.
    paused_at_start: bool,
    /// True when paused at a diverge point on the parent timeline
    /// after returning from a branch via `paused_at_start`. The
    /// emulator stays paused so the user can scrub forward through
    /// the parent's future frames (Shift+Backspace) or rewind
    /// further back (Backspace). Pressing O resumes normal play.
    paused_at_return: bool,
    /// Whether timeline branching is enabled. When `true` (default),
    /// releasing Backspace after rewinding creates an alternate
    /// timeline branch, and rewinding past a branch point pauses for
    /// accept/deny. When `false`, rewind/forward is a simple linear
    /// buffer with no branching.
    branching_enabled: bool,
    /// Whether the timeline rewind feature is enabled at all. When
    /// `true` (default), Backspace rewinds, Shift+Backspace forwards,
    /// and a 3-2-1 countdown plays before resuming. When `false`,
    /// Backspace/Shift+Backspace do nothing.
    timeline_enabled: bool,
    /// Duration of the delay before the 3-2-1 countdown starts, in
    /// frames at 60 FPS. Configurable via
    /// [`set_countdown_delay_ms`](Self::set_countdown_delay_ms).
    /// Default is 30 frames (0.5 seconds). Clamped to
    /// `[0, MAX_COUNTDOWN_DELAY_MS]` equivalent in frames.
    countdown_delay_frames: u32,
    /// The number the countdown starts from (default 3 → 3-2-1).
    /// Configurable via
    /// [`set_countdown_start_number`](Self::set_countdown_start_number).
    /// Clamped to `[1, MAX_COUNTDOWN_START_NUMBER]`.
    countdown_start_number: u32,
    /// Countdown state machine for the 3-2-1 resume-after-rewind
    /// sequence. See [`CountdownState`].
    countdown: CountdownState,
    /// Frame counter for the countdown state machine. Counts up
    /// during Delay and Countdown phases.
    countdown_frame: u32,
}

impl Default for SaveStateHotkeys {
    fn default() -> Self {
        Self {
            slots: SaveStateSlots::new(),
            rewind: RewindBuffer::default(),
            forward: RewindBuffer::with_capacity(crate::save_state::MAX_REWIND_CAPACITY),
            osd: Osd::new(),
            game_name: String::new(),
            frame_counter: 0,
            rewinding: false,
            forwarding: false,
            rewind_speed: 1,
            anim_frame: 0,
            timeline_opacity: 0,
            forward_dirty: false,
            branches: Vec::new(),
            active_branch: None,
            max_branches: DEFAULT_MAX_BRANCHES,
            paused_at_branch: None,
            selected_branch: None,
            denied_branches: Vec::new(),
            paused_at_start: false,
            paused_at_return: false,
            branching_enabled: true,
            timeline_enabled: true,
            countdown_delay_frames: DEFAULT_COUNTDOWN_DELAY_FRAMES,
            countdown_start_number: DEFAULT_COUNTDOWN_START_NUMBER,
            countdown: CountdownState::Idle,
            countdown_frame: 0,
        }
    }
}

impl SaveStateHotkeys {
    /// Construct a dispatcher with the given game name (ROM file stem)
    /// for OSD display and the default rewind buffer capacity.
    pub fn new(game_name: String) -> Self {
        Self::with_capacity(game_name, crate::save_state::DEFAULT_REWIND_CAPACITY)
    }

    /// Construct a dispatcher with the given game name (ROM file stem)
    /// for OSD display and a custom rewind buffer capacity.
    pub fn with_capacity(game_name: String, rewind_capacity: usize) -> Self {
        let cap = rewind_capacity.clamp(1, crate::save_state::MAX_REWIND_CAPACITY);
        Self {
            game_name,
            rewind: RewindBuffer::with_capacity(cap),
            forward: RewindBuffer::with_capacity(crate::save_state::MAX_REWIND_CAPACITY),
            ..Self::default()
        }
    }

    /// Is the OSD currently visible on screen?
    pub fn osd_enabled(&self) -> bool {
        self.osd.enabled()
    }

    /// Current save-state slot index (`0..SAVE_STATE_SLOT_COUNT`).
    pub fn current_slot(&self) -> usize {
        self.slots.current_slot()
    }

    /// Capture a rewind snapshot if the frame counter hit the
    /// `REWIND_INTERVAL` boundary. Called once per presented frame by the
    /// main loop. Errors from the serialiser are reported to stderr and
    /// do not crash the emulator.
    pub fn maybe_push_rewind(&mut self, emulator: &EmulatorState) {
        if self.rewinding || self.forwarding {
            return;
        }
        // Don't record snapshots during the 3-2-1 countdown — the
        // emulator is paused and the frames would be duplicates.
        if self.countdown != CountdownState::Idle {
            return;
        }
        // If we resumed normal emulation after rewinding/forwarding,
        // the forward buffer snapshots are now stale. Clear them before
        // pushing a new rewind snapshot. This must run even while
        // paused at a branch point — the user may have interrupted
        // the countdown to rewind again, leaving forward_dirty set.
        if self.forward_dirty {
            // Create a branch from the forward buffer before clearing
            // it. The branch preserves the original timeline frames that
            // were rewound/forwarded through. Only create when branching
            // is enabled and there are frames to branch from.
            if self.branching_enabled && self.active_forward_len() > 0 {
                self.create_branch_from_active();
            }
            if let Some(idx) = self.active_branch {
                self.branches[idx].forward.clear();
            } else {
                self.forward.clear();
            }
            self.forward_dirty = false;
        }
        // Don't record snapshots while paused at a branch point,
        // branch start, or return point — the emulator is not
        // being stepped, so any snapshot would be a dead frame.
        if self.is_paused_at_branch() {
            return;
        }
        self.frame_counter = self.frame_counter.wrapping_add(1);
        if self.frame_counter % REWIND_INTERVAL != 0 {
            return;
        }
        // Push to the active timeline (root or branch).
        if let Some(idx) = self.active_branch {
            if let Err(e) = self.branches[idx].rewind.push(emulator) {
                eprintln!("nes-emu: branch rewind push failed: {e}");
            }
        } else if let Err(e) = self.rewind.push(emulator) {
            eprintln!("nes-emu: rewind snapshot failed: {e}");
        }
    }

    /// Is the rewind key currently held? When true, the main loop should
    /// call [`rewind_step`](Self::rewind_step) instead of stepping the
    /// emulator forward.
    pub fn is_rewinding(&self) -> bool {
        self.rewinding
    }

    /// Is the forward key currently held? When true, the main loop should
    /// call [`forward_step`](Self::forward_step) instead of stepping the
    /// emulator forward.
    pub fn is_forwarding(&self) -> bool {
        self.forwarding
    }

    /// Stop rewinding (called on Backspace key-up). Marks the forward
    /// buffer as dirty so it will be cleared when normal emulation
    /// resumes — but does NOT clear it immediately, giving the user
    /// time to press Shift+Backspace to fast-forward.
    pub fn stop_rewind(&mut self) {
        self.rewinding = false;
        self.rewind_speed = 1;
        self.paused_at_branch = None;
        self.selected_branch = None;
        self.paused_at_start = false;
        self.paused_at_return = false;
        self.denied_branches.clear();
        // Mark forward buffer dirty: it will be cleared on the next
        // maybe_push_rewind (when normal emulation resumes). This gives
        // the user a window to press Shift+Backspace to forward first.
        if self.active_forward_len() > 0 {
            self.forward_dirty = true;
        }
        // Start the 3-2-1 countdown if timeline is enabled.
        if self.timeline_enabled {
            self.start_countdown();
        }
    }

    /// Stop forwarding (called on Shift release or Backspace key-up).
    /// Resets speed and marks the forward buffer as dirty for later
    /// clearing.
    pub fn stop_forward(&mut self) {
        self.forwarding = false;
        self.rewind_speed = 1;
        self.paused_at_branch = None;
        self.selected_branch = None;
        self.paused_at_start = false;
        self.paused_at_return = false;
        self.denied_branches.clear();
        if self.active_forward_len() > 0 {
            self.forward_dirty = true;
        }
        // Start the 3-2-1 countdown if timeline is enabled.
        if self.timeline_enabled {
            self.start_countdown();
        }
    }

    /// Start forwarding (called on Shift+Backspace key-down, or when
    /// Shift is pressed while already rewinding). Stops rewinding first
    /// so the main loop forwarding branch runs instead of the rewind
    /// branch.
    pub fn start_forward(&mut self) {
        if !self.forwarding {
            self.rewinding = false;
            self.forwarding = true;
            self.forward_dirty = false;
            // Clear any pause state so forwarding can proceed.
            self.paused_at_branch = None;
            self.selected_branch = None;
            self.paused_at_start = false;
            self.paused_at_return = false;
            // Interrupt any active countdown.
            self.interrupt_countdown();
            eprintln!(
                "nes-emu: forward started ({} snapshots)",
                self.active_forward_len()
            );
        }
    }

    /// Called when Shift is pressed while Backspace is held. If we're
    /// rewinding and the forward buffer has snapshots, switch to
    /// forwarding seamlessly without releasing Backspace. Also handles
    /// the case where we're paused at a return point (e.g. just accepted
    /// a branch) — starts forwarding without needing to re-press
    /// Backspace.
    pub fn shift_pressed(&mut self) {
        if self.rewinding && self.active_forward_len() > 0 {
            self.start_forward();
        } else if self.paused_at_return && self.active_forward_len() > 0 {
            self.paused_at_return = false;
            self.start_forward();
        }
    }

    /// Called when Shift is released while Backspace is still held.
    /// If we're forwarding, switch back to rewinding.
    pub fn shift_released(&mut self) {
        if self.forwarding {
            self.forwarding = false;
            self.rewinding = true;
            eprintln!(
                "nes-emu: back to rewind ({} snapshots)",
                self.active_rewind_len()
            );
        }
    }

    /// Cycle the rewind speed multiplier: 1 → 2 → 4 → 8 → 1.
    /// Called when Space is pressed while rewinding.
    pub fn cycle_rewind_speed(&mut self) {
        const SPEEDS: [u32; 4] = [1, 2, 4, 8];
        let idx = SPEEDS
            .iter()
            .position(|&s| s == self.rewind_speed)
            .unwrap_or(0);
        let next = SPEEDS[(idx + 1) % SPEEDS.len()];
        self.rewind_speed = next;
        eprintln!("nes-emu: rewind speed {}x", next);
    }

    /// Current rewind speed multiplier (1, 2, 4, 8).
    pub fn rewind_speed(&self) -> u32 {
        self.rewind_speed
    }

    /// Pop one snapshot from the rewind buffer and restore it. The popped
    /// snapshot is pushed into the forward buffer so the user can
    /// fast-forward back. Returns `true` if a state was restored, `false`
    /// if the buffer was empty. Called once per vsync tick while
    /// `is_rewinding()` is true, multiplied by the current speed.
    pub fn rewind_step(&mut self, emulator: &mut EmulatorState) -> bool {
        if self.paused_at_branch.is_some() || self.paused_at_start {
            return false;
        }
        // Clear denied branches — we're about to move to a new position.
        self.denied_branches.clear();
        let speed = self.rewind_speed as usize;
        let mut restored = false;
        for _ in 0..speed {
            // Don't push to forward buffer if there's nothing to rewind
            // to — otherwise we'd accumulate dead frames in the forward
            // buffer while holding Backspace on an empty rewind buffer.
            if self.active_rewind_len() == 0 {
                break;
            }
            // Push current state to the active forward buffer.
            if let Err(e) = self.active_forward_push(emulator) {
                eprintln!("nes-emu: forward push failed: {e}");
            }
            if self.active_rewind_pop(emulator) {
                emulator.bus_mut().render_frame();
                restored = true;
                if self.branching_enabled {
                    // Check if we reached the beginning of a branch (rewind
                    // buffer empty on a non-root timeline). This takes
                    // priority over branch points: when at the very start
                    // of a branch, the user should be able to press O to
                    // return to the parent timeline. Sub-branches that
                    // diverge at position 0 can be accessed by re-entering
                    // the branch after returning to the parent.
                    if self.active_branch.is_some() && self.active_rewind_len() == 0 {
                        self.paused_at_start = true;
                        eprintln!(
                            "nes-emu: paused at branch start (beginning of branch {:?})",
                            self.active_branch
                        );
                        break;
                    }
                    // Check if we hit a branch point: any branch whose parent
                    // is the current active timeline and whose diverge_pos
                    // matches the current rewind buffer length.
                    if let Some(bp_idx) = self.find_branch_point() {
                        self.paused_at_branch = Some(bp_idx);
                        self.selected_branch = Some(bp_idx);
                        eprintln!("nes-emu: paused at branch point {}", bp_idx);
                        break;
                    }
                }
            } else {
                break;
            }
        }
        restored
    }

    /// Pop one snapshot from the forward buffer and restore it. The popped
    /// snapshot is pushed back into the rewind buffer. Returns `true` if a
    /// state was restored, `false` if the forward buffer was empty.
    pub fn forward_step(&mut self, emulator: &mut EmulatorState) -> bool {
        if self.paused_at_branch.is_some() || self.paused_at_start {
            return false;
        }
        // Clear denied branches — we're about to move to a new position.
        self.denied_branches.clear();
        let speed = self.rewind_speed as usize;
        let mut restored = false;
        for _ in 0..speed {
            if self.active_forward_pop(emulator) {
                // Push the restored state back into the active rewind buffer.
                if let Err(e) = self.active_rewind_push(emulator) {
                    eprintln!("nes-emu: rewind push failed: {e}");
                }
                restored = true;
                if self.branching_enabled {
                    // Check if we hit a branch point.
                    if let Some(bp_idx) = self.find_branch_point() {
                        self.paused_at_branch = Some(bp_idx);
                        self.selected_branch = Some(bp_idx);
                        eprintln!("nes-emu: paused at branch point {}", bp_idx);
                        break;
                    }
                }
            } else {
                break;
            }
        }
        if restored {
            emulator.bus_mut().render_frame();
        }
        restored
    }

    /// Is the emulator paused at a branch point (diverge), at the
    /// beginning of a branch, or at a return point on the parent?
    /// When true, the main loop should not step the emulator.
    pub fn is_paused_at_branch(&self) -> bool {
        self.paused_at_branch.is_some() || self.paused_at_start || self.paused_at_return
    }

    /// Is the emulator paused specifically at a branch diverge point?
    /// (Not at the beginning of a branch.) Used to differentiate
    /// Backspace release behavior.
    pub fn is_paused_at_branch_point(&self) -> bool {
        self.paused_at_branch.is_some()
    }

    /// Is the emulator paused at a return point on the parent timeline
    /// (after returning from a branch via paused_at_start)?
    pub fn is_paused_at_return(&self) -> bool {
        self.paused_at_return
    }

    /// Is the emulator paused at the beginning of a branch (rewind
    /// buffer empty on a non-root timeline)? When true, pressing O
    /// returns to the parent timeline.
    pub fn is_paused_at_start(&self) -> bool {
        self.paused_at_start
    }

    /// Stop rewinding but keep `paused_at_start` true. Used when the
    /// user releases Backspace at the beginning of a branch — we don't
    /// want to start the countdown or clear the pause state because the
    /// user hasn't resumed normal play; they're still at the branch
    /// start and can press O to return to the parent.
    pub fn stop_rewind_keep_paused(&mut self) {
        self.rewinding = false;
        self.rewind_speed = 1;
        // Don't clear paused_at_start — user is still at branch beginning.
        // Don't set forward_dirty — forward buffer has branch history,
        // not stale frames to branch from.
        // Don't start countdown — we haven't resumed normal play.
    }

    // ── Countdown state machine ────────────────────────────────────

    /// Start the 3-2-1 countdown sequence (Delay → Countdown → Resume).
    /// Called when rewind/forward stops and timeline is enabled.
    fn start_countdown(&mut self) {
        self.countdown = CountdownState::Delay;
        self.countdown_frame = 0;
        let ms = self.countdown_delay_frames * 1000 / COUNTDOWN_FPS;
        eprintln!("nes-emu: countdown delay started ({ms}ms)");
    }

    /// Interrupt the countdown — called when the user starts
    /// rewinding/forwarding again before the countdown completes.
    fn interrupt_countdown(&mut self) {
        if self.countdown != CountdownState::Idle {
            eprintln!("nes-emu: countdown interrupted");
        }
        self.countdown = CountdownState::Idle;
        self.countdown_frame = 0;
    }

    /// Skip the countdown and resume immediately — called when the
    /// user presses Tab during the delay or countdown.
    pub fn skip_countdown(&mut self) {
        if self.countdown != CountdownState::Idle {
            eprintln!("nes-emu: countdown skipped by user");
            self.countdown = CountdownState::Idle;
            self.countdown_frame = 0;
        }
    }

    /// Advance the countdown state machine by one frame. Called once
    /// per vsync tick from the main loop. Returns `true` while the
    /// countdown is active (Delay or Countdown), `false` when idle.
    pub fn tick_countdown(&mut self) -> bool {
        match self.countdown {
            CountdownState::Idle => false,
            CountdownState::Delay => {
                self.countdown_frame += 1;
                if self.countdown_frame >= self.countdown_delay_frames {
                    self.countdown = CountdownState::Countdown { number: self.countdown_start_number };
                    self.countdown_frame = 0;
                    eprintln!("nes-emu: countdown: {}", self.countdown_start_number);
                }
                true
            }
            CountdownState::Countdown { number } => {
                self.countdown_frame += 1;
                if self.countdown_frame >= COUNTDOWN_NUMBER_FRAMES {
                    if number <= 1 {
                        self.countdown = CountdownState::Idle;
                        self.countdown_frame = 0;
                        eprintln!("nes-emu: countdown complete, resuming");
                        return false;
                    }
                    let next = number - 1;
                    self.countdown = CountdownState::Countdown { number: next };
                    self.countdown_frame = 0;
                    eprintln!("nes-emu: countdown: {next}");
                }
                true
            }
        }
    }

    /// Is the countdown currently active (Delay or Countdown)?
    pub fn is_countdown_active(&self) -> bool {
        self.countdown != CountdownState::Idle
    }

    /// What number should be displayed on screen? Returns `Some(3)`,
    /// `Some(2)`, `Some(1)` during the Countdown phase, or `None`
    /// during Idle or Delay.
    pub fn countdown_display(&self) -> Option<u32> {
        match self.countdown {
            CountdownState::Countdown { number } => Some(number),
            _ => None,
        }
    }

    /// Is the timeline rewind feature enabled?
    pub fn timeline_enabled(&self) -> bool {
        self.timeline_enabled
    }

    /// Enable or disable the timeline rewind feature. When disabled,
    /// Backspace/Shift+Backspace do nothing and no rewind buffer is
    /// maintained.
    pub fn set_timeline_enabled(&mut self, enabled: bool) {
        self.timeline_enabled = enabled;
        if !enabled {
            self.interrupt_countdown();
            self.rewinding = false;
            self.forwarding = false;
            self.paused_at_branch = None;
            self.selected_branch = None;
            self.paused_at_start = false;
            self.paused_at_return = false;
        }
    }

    /// Current countdown delay in milliseconds.
    pub fn countdown_delay_ms(&self) -> u32 {
        self.countdown_delay_frames * 1000 / COUNTDOWN_FPS
    }

    /// Set the countdown delay in milliseconds. Clamped to
    /// `[0, MAX_COUNTDOWN_DELAY_MS]`. Converted to frames at 60 FPS
    /// internally (rounded up to at least 1 frame unless `ms` is 0,
    /// which disables the delay entirely).
    pub fn set_countdown_delay_ms(&mut self, ms: u32) {
        let clamped = ms.min(MAX_COUNTDOWN_DELAY_MS);
        self.countdown_delay_frames = if clamped == 0 {
            0
        } else {
            // Round up so a non-zero ms always yields at least 1 frame.
            let frames = clamped * COUNTDOWN_FPS / 1000;
            frames.max(1)
        };
    }

    /// Current countdown start number (e.g. 3 for 3-2-1).
    pub fn countdown_start_number(&self) -> u32 {
        self.countdown_start_number
    }

    /// Set the countdown start number. Clamped to
    /// `[1, MAX_COUNTDOWN_START_NUMBER]`.
    pub fn set_countdown_start_number(&mut self, n: u32) {
        self.countdown_start_number = n.clamp(1, MAX_COUNTDOWN_START_NUMBER);
    }

    /// Accept the current branch point: switch to the branch whose
    /// diverge point we paused at. The branch becomes the new active
    /// timeline. If paused at the start of a branch (rewind buffer
    /// empty), accept returns to the parent timeline.
    pub fn accept_branch(&mut self) {
        if let Some(branch_idx) = self.paused_at_branch {
            eprintln!(
                "nes-emu: branch point accepted, switching to branch {}",
                branch_idx
            );
            self.active_branch = Some(branch_idx);
            self.paused_at_branch = None;
            self.selected_branch = None;
            self.denied_branches.clear();
            self.rewinding = false;
            self.forwarding = false;
            self.rewind_speed = 1;
            // Don't auto-start forwarding. Instead, pause at the
            // diverge point so the user can scrub forward through
            // the branch's frames (Shift+Backspace) or rewind back
            // on the branch (Backspace). Press O to resume normal
            // play. Same behavior as returning to root from
            // paused_at_start.
            self.paused_at_return = true;
            self.forward_dirty = false;
            eprintln!(
                "nes-emu: paused at branch {} entry ({} frames to scrub forward)",
                branch_idx,
                self.active_forward_len()
            );
        } else if self.paused_at_start {
            // Return to parent timeline. Get diverge_pos before switching.
            let (diverge_pos, parent) = self
                .active_branch
                .and_then(|idx| self.branches.get(idx))
                .map(|b| (b.diverge_pos, b.parent))
                .unwrap_or((0, None));
            eprintln!(
                "nes-emu: branch start accepted, returning to parent {:?} at diverge_pos {}",
                parent, diverge_pos
            );
            self.active_branch = parent;
            self.paused_at_start = false;
            self.paused_at_return = true;
            self.rewinding = false;
            self.forwarding = false;
            self.rewind_speed = 1;
            self.selected_branch = None;
            self.denied_branches.clear();
            // Truncate the parent's rewind buffer to the diverge
            // position so it matches the current emulator state (which
            // is at the branch start = diverge point). Without this,
            // the rewind buffer would have stale frames from before the
            // branch was entered, causing a state/buffer mismatch.
            while self.active_rewind_len() > diverge_pos {
                if let Some(idx) = self.active_branch {
                    self.branches[idx].rewind.pop_blob();
                } else {
                    self.rewind.pop_blob();
                }
            }
            // Keep the parent's forward buffer intact — it contains
            // the parent's future frames (pushed during the rewind
            // that created the branch). The user can Shift+Backspace
            // to scrub forward through the parent's timeline, or
            // Backspace to rewind further back. Pressing O resumes
            // normal play (with forward_dirty to create a branch).
            // Don't auto-pause at sibling branch points. The user
            // explicitly chose to return to the parent — let them
            // stay here and scrub. They can rewind to re-enter
            // branches later.
        }
    }

    /// Deny the current branch point: continue rewinding/forwarding
    /// past it. The branch point marker remains for future encounters.
    pub fn deny_branch(&mut self) {
        if let Some(idx) = self.paused_at_branch.take() {
            self.denied_branches.push(idx);
            self.selected_branch = None;
            eprintln!("nes-emu: branch point {} denied ({} denied at this position)", idx, self.denied_branches.len());
            // If there are more sibling branches, select the next one.
            if let Some(next) = self.find_branch_point() {
                self.paused_at_branch = Some(next);
                self.selected_branch = Some(next);
                eprintln!("nes-emu: next sibling branch point {}", next);
            }
        }
        if self.paused_at_start {
            eprintln!("nes-emu: branch start denied, staying at beginning");
            self.paused_at_start = false;
            self.selected_branch = None;
        }
    }

    // ── Active timeline accessors ──────────────────────────────────

    /// Current rewind buffer length on the active timeline.
    pub fn active_rewind_len(&self) -> usize {
        match self.active_branch {
            Some(idx) => self.branches.get(idx).map(|b| b.rewind.len()).unwrap_or(0),
            None => self.rewind.len(),
        }
    }

    /// Current forward buffer length on the active timeline.
    pub fn active_forward_len(&self) -> usize {
        match self.active_branch {
            Some(idx) => self.branches.get(idx).map(|b| b.forward.len()).unwrap_or(0),
            None => self.forward.len(),
        }
    }

    /// Iterate over the active timeline's forward buffer blobs (oldest
    /// first) without removing them.
    fn active_forward_iter_blobs(&self) -> impl Iterator<Item = &Vec<u8>> {
        match self.active_branch {
            Some(idx) => self
                .branches
                .get(idx)
                .map(|b| b.forward.iter_blobs().collect::<Vec<_>>())
                .unwrap_or_default()
                .into_iter(),
            None => self.forward.iter_blobs().collect::<Vec<_>>().into_iter(),
        }
    }

    /// Push a snapshot to the active timeline's rewind buffer.
    fn active_rewind_push(&mut self, emulator: &EmulatorState) -> Result<(), SaveStateError> {
        match self.active_branch {
            Some(idx) => {
                if let Some(b) = self.branches.get_mut(idx) {
                    b.rewind.push(emulator)
                } else {
                    self.rewind.push(emulator)
                }
            }
            None => self.rewind.push(emulator),
        }
    }

    /// Pop a snapshot from the active timeline's rewind buffer.
    fn active_rewind_pop(&mut self, emulator: &mut EmulatorState) -> bool {
        match self.active_branch {
            Some(idx) => {
                if let Some(b) = self.branches.get_mut(idx) {
                    b.rewind.pop(emulator)
                } else {
                    self.rewind.pop(emulator)
                }
            }
            None => self.rewind.pop(emulator),
        }
    }

    /// Push a snapshot to the active timeline's forward buffer.
    fn active_forward_push(&mut self, emulator: &EmulatorState) -> Result<(), SaveStateError> {
        match self.active_branch {
            Some(idx) => {
                if let Some(b) = self.branches.get_mut(idx) {
                    b.forward.push(emulator)
                } else {
                    self.forward.push(emulator)
                }
            }
            None => self.forward.push(emulator),
        }
    }

    /// Pop a snapshot from the active timeline's forward buffer.
    fn active_forward_pop(&mut self, emulator: &mut EmulatorState) -> bool {
        match self.active_branch {
            Some(idx) => {
                if let Some(b) = self.branches.get_mut(idx) {
                    b.forward.pop(emulator)
                } else {
                    self.forward.pop(emulator)
                }
            }
            None => self.forward.pop(emulator),
        }
    }

    // ── Branch management ──────────────────────────────────────────

    /// Find a branch point on the current active timeline: a branch whose
    /// parent is the active timeline and whose `diverge_pos` matches the
    /// current rewind buffer length.
    fn find_branch_point(&self) -> Option<usize> {
        let current_len = self.active_rewind_len();
        self.branches
            .iter()
            .enumerate()
            .find(|(idx, b)| {
                b.parent == self.active_branch
                    && b.diverge_pos == current_len
                    && !self.denied_branches.contains(idx)
            })
            .map(|(idx, _)| idx)
    }

    /// Find all branch points at the current rewind position on the
    /// active timeline. Returns indices of all (non-denied) sibling
    /// branches that diverge here.
    fn find_all_branch_points(&self) -> Vec<usize> {
        let current_len = self.active_rewind_len();
        self.branches
            .iter()
            .enumerate()
            .filter(|(idx, b)| {
                b.parent == self.active_branch
                    && b.diverge_pos == current_len
                    && !self.denied_branches.contains(idx)
            })
            .map(|(idx, _)| idx)
            .collect()
    }

    /// Cycle branch selection up (previous sibling branch).
    pub fn select_branch_up(&mut self) {
        if self.paused_at_branch.is_none() {
            return;
        }
        let siblings = self.find_all_branch_points();
        if siblings.len() <= 1 {
            return;
        }
        let current = self.selected_branch.unwrap_or(self.paused_at_branch.unwrap());
        let pos = siblings.iter().position(|&s| s == current).unwrap_or(0);
        let new_pos = if pos == 0 { siblings.len() - 1 } else { pos - 1 };
        let new_idx = siblings[new_pos];
        self.selected_branch = Some(new_idx);
        self.paused_at_branch = Some(new_idx);
        eprintln!("nes-emu: selected branch {}", new_idx);
    }

    /// Cycle branch selection down (next sibling branch).
    pub fn select_branch_down(&mut self) {
        if self.paused_at_branch.is_none() {
            return;
        }
        let siblings = self.find_all_branch_points();
        if siblings.len() <= 1 {
            return;
        }
        let current = self.selected_branch.unwrap_or(self.paused_at_branch.unwrap());
        let pos = siblings.iter().position(|&s| s == current).unwrap_or(0);
        let new_pos = (pos + 1) % siblings.len();
        let new_idx = siblings[new_pos];
        self.selected_branch = Some(new_idx);
        self.paused_at_branch = Some(new_idx);
        eprintln!("nes-emu: selected branch {}", new_idx);
    }

    /// Get the currently selected branch index for rendering.
    pub fn selected_branch_idx(&self) -> Option<usize> {
        self.selected_branch.or(self.paused_at_branch)
    }

    /// Create a new branch from the current active timeline position.
    /// The branch's `diverge_pos` is the current rewind buffer length on
    /// the active timeline. All existing branches are preserved so the
    /// user can switch between sibling timelines. If we're already at
    /// `max_branches`, the oldest branch is dropped.
    fn create_branch_from_active(&mut self) {
        let diverge_pos = self.active_rewind_len();

        // Copy the active forward buffer blobs (oldest-first) into the
        // branch's forward buffer. These are the frames that were popped
        // during rewind — they represent the original timeline after the
        // diverge point. From the diverge point's perspective, they are
        // the branch's future. We COPY (not drain) so the user can still
        // Shift+Backspace to forward on the current timeline.
        let fwd_blobs: Vec<Vec<u8>> = self.active_forward_iter_blobs().cloned().collect();

        // Enforce max_branches: if at limit, drop oldest (index 0) and
        // shift all parent indices down by 1.
        if self.branches.len() >= self.max_branches {
            self.branches.remove(0);
            for b in &mut self.branches {
                if let Some(p) = b.parent.as_mut() {
                    if *p > 0 {
                        *p -= 1;
                    } else {
                        b.parent = None;
                    }
                }
            }
            self.active_branch = self.active_branch.and_then(|idx| {
                if idx > 0 { Some(idx - 1) } else { None }
            });
        }

        let new_parent = self.active_branch;

        // Build the branch's forward buffer from the copied forward
        // blobs. The branch's rewind starts empty — when the user
        // accepts the branch, forward_step will pop from forward and
        // push to rewind, naturally building up the branch's rewind
        // buffer as the history is replayed.
        let mut branch_forward = RewindBuffer::with_capacity(crate::save_state::MAX_REWIND_CAPACITY);
        for blob in &fwd_blobs {
            branch_forward.push_blob(blob.clone());
        }

        let new_idx = self.branches.len();
        self.branches.push(TimelineBranch {
            diverge_pos,
            rewind: RewindBuffer::with_capacity(crate::save_state::MAX_REWIND_CAPACITY),
            forward: branch_forward,
            parent: new_parent,
        });
        // DON'T switch active_branch — stay on the current timeline.
        // The branch is now a "side timeline" that can be accessed by
        // rewinding to the diverge_pos and accepting the branch point.
        eprintln!(
            "nes-emu: created branch {} at diverge_pos {} (parent {:?}), {} frames in forward buffer",
            new_idx,
            diverge_pos,
            new_parent,
            self.branches[new_idx].forward.len()
        );
    }

    /// Get render info for all branches: (diverge_pos, rewind_len,
    /// forward_len, stack_row) for each branch. The stack_row combines
    /// the branch's nesting depth with an overlap offset so that
    /// branches occupying the same time range are placed on separate
    /// Y levels instead of overlapping.
    pub fn branch_render_info(&self) -> Vec<(usize, usize, usize, u32)> {
        // Collect (diverge_pos, rewind_len, forward_len, depth, original_index)
        let mut infos: Vec<(usize, usize, usize, u32, usize)> = self
            .branches
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let d = self.branch_depth(b);
                (b.diverge_pos, b.rewind.len(), b.forward.len(), d, i)
            })
            .collect();

        // Sort by (depth, diverge_pos) for deterministic stacking.
        infos.sort_by_key(|&(div, _, _, d, _)| (d, div));

        // Assign stack rows. Each branch gets the lowest row >= its depth
        // that doesn't horizontally overlap with any branch already on
        // that row.
        let mut row_ranges: Vec<Vec<(usize, usize)>> = Vec::new();
        let mut stack_rows = vec![0u32; infos.len()];

        for (idx, &(diverge_pos, rewind_len, forward_len, depth, _)) in
            infos.iter().enumerate()
        {
            let start = diverge_pos;
            let end = diverge_pos.saturating_add(rewind_len).saturating_add(forward_len);

            let mut row = depth;
            loop {
                while row_ranges.len() <= row as usize {
                    row_ranges.push(Vec::new());
                }
                let overlaps = row_ranges[row as usize]
                    .iter()
                    .any(|&(s, e)| start < e && end > s);
                if !overlaps {
                    break;
                }
                row += 1;
            }
            row_ranges[row as usize].push((start, end));
            stack_rows[idx] = row;
        }

        // Build result in original branch order.
        let mut result = vec![(0usize, 0usize, 0usize, 0u32); self.branches.len()];
        for (idx, &(_, _, _, _, branch_idx)) in infos.iter().enumerate() {
            result[branch_idx] = (
                infos[idx].0,
                infos[idx].1,
                infos[idx].2,
                stack_rows[idx],
            );
        }
        result
    }

    /// Compute the depth of a branch (0 = child of root, 1 = grandchild…).
    fn branch_depth(&self, branch: &TimelineBranch) -> u32 {
        let mut depth = 0u32;
        let mut current = branch.parent;
        while let Some(i) = current {
            depth += 1;
            current = self.branches.get(i).and_then(|b| b.parent);
        }
        depth
    }

    /// Set the maximum number of branches.
    pub fn set_max_branches(&mut self, max: usize) {
        self.max_branches = max.max(1);
    }

    /// Is timeline branching enabled?
    pub fn branching_enabled(&self) -> bool {
        self.branching_enabled
    }

    /// Enable or disable timeline branching. When disabled, no branches
    /// are created on rewind/forward release, and branch point pausing
    /// is skipped — rewind/forward operates as a simple linear buffer.
    pub fn set_branching_enabled(&mut self, enabled: bool) {
        self.branching_enabled = enabled;
        if !enabled {
            // Clear existing branch state when disabling.
            self.branches.clear();
            self.active_branch = None;
            self.paused_at_branch = None;
            self.selected_branch = None;
            self.paused_at_start = false;
            self.paused_at_return = false;
        }
    }

    /// Advance the timeline animation: increment the frame counter and
    /// ramp opacity toward the target (full when rewinding/forwarding,
    /// zero otherwise). Called once per frame by the main loop.
    pub fn tick_animation(&mut self) {
        if self.rewinding || self.forwarding || self.is_paused_at_branch() {
            if self.timeline_opacity < crate::rewind_timeline::MAX_OPACITY {
                self.timeline_opacity += 1;
            }
        } else if self.timeline_opacity > 0 {
            self.timeline_opacity -= 1;
        }
        self.anim_frame = self.anim_frame.wrapping_add(1);
    }

    /// Current rewind buffer capacity (max snapshots).
    pub fn rewind_capacity(&self) -> usize {
        self.rewind.capacity()
    }

    /// Rebuild the rewind buffer with a new capacity. Existing snapshots
    /// are discarded (the buffer is cleared). Clamped to
    /// `[1, MAX_REWIND_CAPACITY]`.
    pub fn set_rewind_capacity(&mut self, capacity: usize) {
        let cap = capacity.clamp(1, crate::save_state::MAX_REWIND_CAPACITY);
        self.rewind = RewindBuffer::with_capacity(cap);
        self.forward.clear();
        self.branches.clear();
        self.active_branch = None;
        self.paused_at_branch = None;
        self.selected_branch = None;
        self.paused_at_start = false;
        self.paused_at_return = false;
    }

    /// Render the visual rewind/forward timeline into the framebuffer.
    /// Drawn while the rewind or forward key is held (or fading out after
    /// release). Uses the current opacity for fade in/out animation and
    /// the animation frame counter for the marker pulse effect.
    pub fn render_rewind_overlay(&self, fb: &mut [u32], width: u32, height: u32) {
        if self.timeline_opacity == 0 {
            return;
        }
        let mode = if self.forwarding {
            crate::rewind_timeline::TimelineMode::Forward
        } else {
            crate::rewind_timeline::TimelineMode::Rewind
        };
        // Always show root as the main bar (reference timeline).
        // Branches are shown as offset bars above. The active branch
        // index is passed so the renderer can place the position marker
        // on the correct bar.
        let cfg = crate::rewind_timeline::TimelineConfig {
            filled: self.rewind.len(),
            capacity: self.rewind.capacity(),
            mode,
            rewind_interval: REWIND_INTERVAL as usize,
            opacity: self.timeline_opacity,
            anim_frame: self.anim_frame,
            speed: self.rewind_speed,
            forward_filled: self.forward.len(),
            branches: self.branch_render_info(),
            active_branch: self.active_branch,
            active_filled: self.active_rewind_len(),
            active_forward_filled: self.active_forward_len(),
            selected_branch: self.selected_branch_idx(),
        };
        crate::rewind_timeline::render_timeline(fb, width, height, &cfg);
    }

    /// Render the countdown overlay. Draws a large number (4×
    /// scaled 8×8 glyphs = 32×32 pixels per digit) centered on the
    /// framebuffer with a semi-transparent dark background. Supports
    /// multi-digit numbers (e.g. 10). Only draws during the Countdown
    /// phase (not during Delay).
    pub fn render_countdown(&self, fb: &mut [u32], width: u32, height: u32) {
        if fb.len() != (width * height) as usize {
            return;
        }
        let number = match self.countdown_display() {
            Some(n) => n,
            None => return,
        };

        // Scale factor for each digit: 4× makes an 8×8 glyph into 32×32.
        const SCALE: u32 = 4;
        const GLYPH_SIZE: u32 = 8;
        const SCALED_SIZE: u32 = GLYPH_SIZE * SCALE; // 32
        const DIGIT_SPACING: u32 = 2; // pixels between digits

        // Convert number to digit characters (supports 1–10 → 1 or 2 digits).
        let digits: Vec<char> = number.to_string().chars().collect();
        let digit_count = digits.len() as u32;
        let total_w = digit_count * SCALED_SIZE + (digit_count - 1) * DIGIT_SPACING;

        // Center on screen.
        let start_x = (width.saturating_sub(total_w)) / 2;
        let start_y = (height.saturating_sub(SCALED_SIZE)) / 2;

        // Draw a semi-transparent dark background behind the digits.
        const BG_ARGB: u32 = 0x8000_0000;
        const FG_ARGB: u32 = 0xFFFF_FFFF;

        let bg_margin = 4;
        let bg_x0 = start_x.saturating_sub(bg_margin);
        let bg_y0 = start_y.saturating_sub(bg_margin);
        let bg_x1 = (start_x + total_w + bg_margin).min(width);
        let bg_y1 = (start_y + SCALED_SIZE + bg_margin).min(height);
        for py in bg_y0..bg_y1 {
            for px in bg_x0..bg_x1 {
                let idx = (py * width + px) as usize;
                if idx < fb.len() {
                    fb[idx] = BG_ARGB;
                }
            }
        }

        // Draw each digit glyph.
        for (di, &ch) in digits.iter().enumerate() {
            let glyph = crate::osd::glyph_bits(ch);
            let digit_x = start_x + (di as u32) * (SCALED_SIZE + DIGIT_SPACING);

            for gy in 0..GLYPH_SIZE {
                let row = glyph[gy as usize];
                for gx in 0..GLYPH_SIZE {
                    let bit = (row >> (7 - gx)) & 1;
                    if bit == 0 {
                        continue;
                    }
                    // Scale each pixel into a SCALE×SCALE block.
                    for dy in 0..SCALE {
                        for dx in 0..SCALE {
                            let px = digit_x + gx * SCALE + dx;
                            let py = start_y + gy * SCALE + dy;
                            if px < width && py < height {
                                let idx = (py * width + px) as usize;
                                if idx < fb.len() {
                                    fb[idx] = FG_ARGB;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Render the rewind timeline in "buffer display" mode (not actively
    /// rewinding/forwarding). Shown as part of the OSD when the OSD is
    /// enabled to give a visual indication of how much rewind history is
    /// available.
    pub fn render_timeline_indicator(&self, fb: &mut [u32], width: u32, height: u32) {
        if self.rewind.is_empty() && self.branches.is_empty() {
            return;
        }
        let cfg = crate::rewind_timeline::TimelineConfig {
            filled: self.active_rewind_len(),
            capacity: self.rewind.capacity(),
            mode: crate::rewind_timeline::TimelineMode::Buffer,
            rewind_interval: REWIND_INTERVAL as usize,
            opacity: crate::rewind_timeline::MAX_OPACITY,
            anim_frame: self.anim_frame,
            speed: 1,
            forward_filled: 0,
            branches: self.branch_render_info(),
            active_branch: self.active_branch,
            active_filled: self.active_rewind_len(),
            active_forward_filled: 0,
            selected_branch: self.selected_branch_idx(),
        };
        crate::rewind_timeline::render_timeline(fb, width, height, &cfg);
    }

    /// Render the OSD into the framebuffer. No-op when the OSD is
    /// disabled. The lines shown are built from the current FPS, the
    /// supplied `mapper_number`, game name, slot, and rewind depth.
    ///
    /// `mapper_number` is passed in by the caller (rather than borrowed
    /// from the emulator) so the caller can take the mutable framebuffer
    /// borrow without conflicting with an immutable emulator borrow.
    pub fn render_osd(
        &self,
        mapper_number: u16,
        region: crate::region::Region,
        fb: &mut [u32],
        width: u32,
        height: u32,
    ) {
        if !self.osd.enabled() {
            return;
        }
        let slot = self.slots.current_slot();
        let slot_occupied = !self.slots.is_empty(slot);
        let lines_str = build_lines(
            self.osd.fps(),
            mapper_number,
            &self.game_name,
            slot,
            slot_occupied,
            self.rewind.len(),
            region,
        );
        let lines: Vec<&str> = lines_str.iter().map(|s| s.as_str()).collect();
        self.osd.render(fb, width, height, &lines);
    }

    /// Post-frame hook: capture a rewind snapshot, then (if the OSD is
    /// enabled) record the frame timestamp, blit the OSD into the
    /// framebuffer, and re-present via the supplied `present` closure.
    /// Extracted from `src/main.rs` to keep the binary under the 400-line
    /// file-size limit.
    ///
    /// The rewind snapshot is taken *before* the OSD blit so the snapshot
    /// stays clean of OSD pixels — the OSD is a host-side overlay, not
    /// part of the emulated framebuffer.
    ///
    /// `present` is called with the post-OSD framebuffer so the overlay is
    /// actually visible this frame. It is only invoked when the OSD is
    /// enabled — the caller is responsible for presenting when the OSD is
    /// off (see `src/main.rs`). This avoids a double-present that would
    /// halve the frame rate (the renderer is vsync-locked, so each
    /// `present` blocks until the next vsync).
    pub fn post_frame(
        &mut self,
        emulator: &mut EmulatorState,
        present: impl FnOnce(&[u32]) -> Result<(), String>,
    ) -> Result<(), String> {
        self.maybe_push_rewind(emulator);
        if self.osd_enabled() {
            self.osd.record_frame(std::time::Instant::now());
            let mapper_number = emulator.mapper_number();
            let region = emulator.region();
            let fb = emulator.framebuffer_mut();
            self.render_osd(
                mapper_number,
                region,
                fb,
                crate::video::NES_WIDTH,
                crate::video::NES_HEIGHT,
            );
            // Re-borrow the framebuffer immutably for the present call.
            // The mutable borrow above has ended, so this is fine.
            present(emulator.framebuffer())?;
        }
        Ok(())
    }

    /// Handle a key-down event. Returns `true` if the key was consumed by
    /// a save-state / rewind / OSD hotkey (and should *not* be forwarded
    /// to the joypad or debug dispatchers), `false` if it should be
    /// routed as usual.
    ///
    /// `keymod` is the SDL2 keyboard modifier bitmask at the time of the
    /// event. Number-key slot selection requires no *held* modifiers
    /// (Shift / Ctrl / Alt) so `Ctrl+1` / `Alt+1` / `Shift+1` still route
    /// to the joypad. Toggle states (Num Lock, Caps Lock) are intentionally
    /// *not* checked — they appear in `keymod` whenever active and would
    /// otherwise break slot selection on systems where Num Lock is on by
    /// default.
    pub fn handle_key(&mut self, emulator: &mut EmulatorState, key: Keycode, keymod: Mod) -> bool {
        let no_mod = !keymod.intersects(
            Mod::LSHIFTMOD
                | Mod::RSHIFTMOD
                | Mod::LCTRLMOD
                | Mod::RCTRLMOD
                | Mod::LALTMOD
                | Mod::RALTMOD,
        );
        match key {
            // F5: save state to current slot.
            Keycode::F5 => {
                let slot = self.slots.current_slot();
                match self.slots.save_current(emulator) {
                    Ok(()) => eprintln!(
                        "nes-emu: state saved to slot {}/{}",
                        slot + 1,
                        crate::save_state::SAVE_STATE_SLOT_COUNT
                    ),
                    Err(e) => eprintln!("nes-emu: save state failed: {e}"),
                }
                true
            }
            // F7: load state from current slot.
            Keycode::F7 => {
                let slot = self.slots.current_slot();
                match self.slots.load_current(emulator) {
                    Ok(()) => {
                        eprintln!(
                            "nes-emu: state loaded from slot {}/{}",
                            slot + 1,
                            crate::save_state::SAVE_STATE_SLOT_COUNT
                        );
                        // Loading a save state invalidates the rewind
                        // buffer — the snapshots in it belong to a
                        // timeline that no longer matches the restored
                        // state. Clear it so a subsequent Backspace does
                        // not jump to an unrelated frame.
                        self.rewind.clear();
                        self.forward.clear();
                    }
                    Err(SaveStateError::SlotEmpty { .. }) => {
                        eprintln!(
                            "nes-emu: slot {}/{} is empty — nothing to load",
                            slot + 1,
                            crate::save_state::SAVE_STATE_SLOT_COUNT
                        );
                    }
                    Err(e) => eprintln!("nes-emu: load state failed: {e}"),
                }
                true
            }
            // F10: toggle OSD.
            Keycode::F10 => {
                let on = self.osd.toggle();
                eprintln!("nes-emu: OSD {}", if on { "ON" } else { "OFF" });
                true
            }
            // O: accept current branch point (resume from here).
            // P: deny current branch point (continue past it).
            // Up/Down: cycle through sibling branches at a branch point.
            // Only active when branching is enabled.
            Keycode::Up if self.branching_enabled
                && self.paused_at_branch.is_some() =>
            {
                self.select_branch_up();
                true
            }
            Keycode::Down if self.branching_enabled
                && self.paused_at_branch.is_some() =>
            {
                self.select_branch_down();
                true
            }
            Keycode::O if self.branching_enabled
                && (self.paused_at_branch.is_some() || self.paused_at_start) =>
            {
                self.accept_branch();
                // If Shift is already held and we're now paused at
                // return with forward frames, auto-start forwarding so
                // the user doesn't need to release and re-press
                // Backspace.
                if self.paused_at_return
                    && keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD)
                    && self.active_forward_len() > 0
                {
                    self.paused_at_return = false;
                    self.start_forward();
                }
                true
            }
            // O when paused at return point: resume normal play.
            // Set forward_dirty so maybe_push_rewind creates a branch
            // from the forward buffer before clearing it, then start
            // the countdown.
            Keycode::O if self.branching_enabled
                && self.paused_at_return =>
            {
                self.paused_at_return = false;
                if self.active_forward_len() > 0 {
                    self.forward_dirty = true;
                }
                if self.timeline_enabled {
                    self.start_countdown();
                }
                eprintln!("nes-emu: resumed normal play from return point");
                true
            }
            Keycode::P if self.branching_enabled
                && (self.paused_at_branch.is_some() || self.paused_at_start) =>
            {
                self.deny_branch();
                true
            }
            // Backspace: start rewinding (no Shift) or forwarding (Shift held).
            // While held, the main loop pops one snapshot per vsync tick
            // instead of stepping forward. Shift+Backspace starts the forward
            // buffer playback to return to where the user started rewinding.
            // Does nothing when timeline is disabled.
            Keycode::Backspace if self.timeline_enabled => {
                let shift_held = keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD);
                if shift_held {
                    // Shift+Backspace: start forwarding through the forward buffer.
                    if !self.forwarding {
                        // Clear paused_at_return so forward_step doesn't block.
                        self.paused_at_return = false;
                        self.start_forward();
                    }
                } else if !self.rewinding {
                    // Clear paused_at_return so rewind_step doesn't block.
                    self.paused_at_return = false;
                    self.rewinding = true;
                    self.interrupt_countdown();
                    eprintln!(
                        "nes-emu: rewind started ({} snapshots)",
                        self.active_rewind_len()
                    );
                }
                true
            }
            // Space: cycle speed while rewinding or forwarding (1x → 2x → 4x → 8x).
            // When not rewinding/forwarding, Space is not consumed here
            // (routes to turbo in the main loop).
            Keycode::Space if self.rewinding || self.forwarding => {
                self.cycle_rewind_speed();
                true
            }
            // Number keys 1..=9 select slots 0..=8; 0 selects slot 9.
            // Only when no modifiers are held so Ctrl+1 / Alt+1 / Shift+1
            // still route to the joypad.
            Keycode::Num1 if no_mod => {
                self.select_slot(0);
                true
            }
            Keycode::Num2 if no_mod => {
                self.select_slot(1);
                true
            }
            Keycode::Num3 if no_mod => {
                self.select_slot(2);
                true
            }
            Keycode::Num4 if no_mod => {
                self.select_slot(3);
                true
            }
            Keycode::Num5 if no_mod => {
                self.select_slot(4);
                true
            }
            Keycode::Num6 if no_mod => {
                self.select_slot(5);
                true
            }
            Keycode::Num7 if no_mod => {
                self.select_slot(6);
                true
            }
            Keycode::Num8 if no_mod => {
                self.select_slot(7);
                true
            }
            Keycode::Num9 if no_mod => {
                self.select_slot(8);
                true
            }
            Keycode::Num0 if no_mod => {
                self.select_slot(9);
                true
            }
            _ => false,
        }
    }

    /// Select a save state slot and report it to stderr.
    fn select_slot(&mut self, slot: usize) {
        self.slots.set_current_slot(slot);
        let occupied = !self.slots.is_empty(slot);
        eprintln!(
            "nes-emu: slot {}/{} selected ({})",
            slot + 1,
            crate::save_state::SAVE_STATE_SLOT_COUNT,
            if occupied { "occupied" } else { "empty" }
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_no_osd() {
        let h = SaveStateHotkeys::default();
        assert!(!h.osd_enabled());
    }

    #[test]
    fn default_slot_zero() {
        let h = SaveStateHotkeys::default();
        assert_eq!(h.current_slot(), 0);
    }

    #[test]
    fn default_rewind_empty() {
        let h = SaveStateHotkeys::default();
        assert!(h.rewind.is_empty());
    }

    #[test]
    fn new_sets_game_name() {
        let h = SaveStateHotkeys::new("TestGame".to_string());
        assert_eq!(h.game_name, "TestGame");
    }

    #[test]
    fn branch_render_info_overlapping_siblings_stack() {
        let mut h = SaveStateHotkeys::default();
        h.branching_enabled = true;
        // Branch 0: diverge at 100, rewind 50, forward 20 → spans [100, 170)
        let mut b0_rew = RewindBuffer::with_capacity(300);
        for _ in 0..50 { b0_rew.push_blob(vec![0; 10]); }
        let mut b0_fwd = RewindBuffer::with_capacity(300);
        for _ in 0..20 { b0_fwd.push_blob(vec![0; 10]); }
        h.branches.push(TimelineBranch {
            diverge_pos: 100,
            rewind: b0_rew,
            forward: b0_fwd,
            parent: None,
        });
        // Branch 1: diverge at 120, rewind 60, forward 0 → spans [120, 180)
        // Overlaps with branch 0 → should get stack_row 1
        let mut b1_rew = RewindBuffer::with_capacity(300);
        for _ in 0..60 { b1_rew.push_blob(vec![0; 10]); }
        h.branches.push(TimelineBranch {
            diverge_pos: 120,
            rewind: b1_rew,
            forward: RewindBuffer::with_capacity(300),
            parent: None,
        });
        // Branch 2: diverge at 250, rewind 10, forward 0 → spans [250, 260)
        // No overlap with branch 0 or 1 → should get stack_row 0
        let mut b2_rew = RewindBuffer::with_capacity(300);
        for _ in 0..10 { b2_rew.push_blob(vec![0; 10]); }
        h.branches.push(TimelineBranch {
            diverge_pos: 250,
            rewind: b2_rew,
            forward: RewindBuffer::with_capacity(300),
            parent: None,
        });

        let info = h.branch_render_info();
        // All at depth 0 (parent is root).
        // Branch 0: stack_row 0, Branch 1: stack_row 1, Branch 2: stack_row 0
        assert_eq!(info[0].3, 0, "branch 0 should be at stack_row 0");
        assert_eq!(info[1].3, 1, "branch 1 should be at stack_row 1 (overlaps branch 0)");
        assert_eq!(info[2].3, 0, "branch 2 should be at stack_row 0 (no overlap)");
    }
}
