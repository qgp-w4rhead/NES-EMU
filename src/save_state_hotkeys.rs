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

/// An alternate timeline branch. Created when the user releases Backspace
/// after rewinding/forwarding. Each branch has its own rewind and forward
/// buffers, and records a divergence point on its parent timeline.
struct TimelineBranch {
    /// Position on the parent timeline (rewind buffer length) where this
    /// branch diverges.
    diverge_pos: usize,
    /// Rewind buffer for this branch (frames recorded after divergence).
    rewind: RewindBuffer,
    /// Forward buffer for this branch (frames popped during rewind).
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
    /// Alternate timeline branches. Forms a chain: branches[0] is a
    /// child of the root, branches[1] is a child of branches[0], etc.
    /// When a new sub-branch is created, non-ancestor branches are
    /// cleared. Limited to `max_branches`.
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
    /// True when paused at the beginning of a branch (rewind buffer
    /// empty on a non-root timeline). Accepting returns to the parent
    /// timeline.
    paused_at_start: bool,
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
            paused_at_start: false,
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
        // If we resumed normal emulation after rewinding/forwarding,
        // the forward buffer snapshots are now stale. Clear them before
        // pushing a new rewind snapshot.
        if self.forward_dirty {
            if let Some(idx) = self.active_branch {
                self.branches[idx].forward.clear();
            } else {
                self.forward.clear();
            }
            self.forward_dirty = false;
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
        if self.rewinding {
            self.create_branch_from_active();
        }
        self.rewinding = false;
        self.rewind_speed = 1;
        self.paused_at_branch = None;
        self.paused_at_start = false;
        // Mark forward buffer dirty: it will be cleared on the next
        // maybe_push_rewind (when normal emulation resumes). This gives
        // the user a window to press Shift+Backspace to forward first.
        if self.active_forward_len() > 0 {
            self.forward_dirty = true;
        }
    }

    /// Stop forwarding (called on Shift release or Backspace key-up).
    /// Resets speed and marks the forward buffer as dirty for later
    /// clearing.
    pub fn stop_forward(&mut self) {
        if self.forwarding {
            self.create_branch_from_active();
        }
        self.forwarding = false;
        self.rewind_speed = 1;
        self.paused_at_branch = None;
        self.paused_at_start = false;
        if self.active_forward_len() > 0 {
            self.forward_dirty = true;
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
            self.paused_at_start = false;
            eprintln!(
                "nes-emu: forward started ({} snapshots)",
                self.active_forward_len()
            );
        }
    }

    /// Called when Shift is pressed while Backspace is held. If we're
    /// rewinding and the forward buffer has snapshots, switch to
    /// forwarding seamlessly without releasing Backspace.
    pub fn shift_pressed(&mut self) {
        if self.rewinding && self.active_forward_len() > 0 {
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
        let speed = self.rewind_speed as usize;
        let mut restored = false;
        for _ in 0..speed {
            // Push current state to the active forward buffer.
            if let Err(e) = self.active_forward_push(emulator) {
                eprintln!("nes-emu: forward push failed: {e}");
            }
            if self.active_rewind_pop(emulator) {
                emulator.bus_mut().render_frame();
                restored = true;
                // Check if we hit a branch point: any branch whose parent
                // is the current active timeline and whose diverge_pos
                // matches the current rewind buffer length.
                if let Some(bp_idx) = self.find_branch_point() {
                    self.paused_at_branch = Some(bp_idx);
                    eprintln!("nes-emu: paused at branch point {}", bp_idx);
                    break;
                }
                // Check if we reached the beginning of a branch (rewind
                // buffer empty on a non-root timeline).
                if self.active_branch.is_some() && self.active_rewind_len() == 0 {
                    self.paused_at_start = true;
                    eprintln!(
                        "nes-emu: paused at branch start (beginning of branch {:?})",
                        self.active_branch
                    );
                    break;
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
        let speed = self.rewind_speed as usize;
        let mut restored = false;
        for _ in 0..speed {
            if self.active_forward_pop(emulator) {
                // Push the restored state back into the active rewind buffer.
                if let Err(e) = self.active_rewind_push(emulator) {
                    eprintln!("nes-emu: rewind push failed: {e}");
                }
                restored = true;
                // Check if we hit a branch point.
                if let Some(bp_idx) = self.find_branch_point() {
                    self.paused_at_branch = Some(bp_idx);
                    eprintln!("nes-emu: paused at branch point {}", bp_idx);
                    break;
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

    /// Is the emulator paused at a branch point (diverge) or at the
    /// beginning of a branch? When true, the main loop should not step
    /// the emulator.
    pub fn is_paused_at_branch(&self) -> bool {
        self.paused_at_branch.is_some() || self.paused_at_start
    }

    /// Is the emulator paused specifically at a branch diverge point?
    /// (Not at the beginning of a branch.) Used to differentiate
    /// Backspace release behavior.
    pub fn is_paused_at_branch_point(&self) -> bool {
        self.paused_at_branch.is_some()
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
            self.rewinding = false;
            self.forwarding = false;
            self.rewind_speed = 1;
        } else if self.paused_at_start {
            // Return to parent timeline.
            let parent = self
                .active_branch
                .and_then(|idx| self.branches.get(idx))
                .and_then(|b| b.parent);
            eprintln!(
                "nes-emu: branch start accepted, returning to parent {:?}",
                parent
            );
            self.active_branch = parent;
            self.paused_at_start = false;
            self.rewinding = false;
            self.forwarding = false;
            self.rewind_speed = 1;
        }
    }

    /// Deny the current branch point: continue rewinding/forwarding
    /// past it. The branch point marker remains for future encounters.
    pub fn deny_branch(&mut self) {
        if self.paused_at_branch.is_some() {
            eprintln!("nes-emu: branch point denied");
            self.paused_at_branch = None;
        }
        if self.paused_at_start {
            eprintln!("nes-emu: branch start denied, staying at beginning");
            self.paused_at_start = false;
        }
    }

    // ── Active timeline accessors ──────────────────────────────────

    /// Current rewind buffer length on the active timeline.
    fn active_rewind_len(&self) -> usize {
        match self.active_branch {
            Some(idx) => self.branches.get(idx).map(|b| b.rewind.len()).unwrap_or(0),
            None => self.rewind.len(),
        }
    }

    /// Current forward buffer length on the active timeline.
    fn active_forward_len(&self) -> usize {
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
            .position(|b| b.parent == self.active_branch && b.diverge_pos == current_len)
    }

    /// Create a new branch from the current active timeline position.
    /// The branch's `diverge_pos` is the current rewind buffer length on
    /// the active timeline. Non-ancestor branches are cleared first.
    /// If we're already at `max_branches`, the oldest branch is dropped.
    fn create_branch_from_active(&mut self) {
        let diverge_pos = self.active_rewind_len();
        let parent = self.active_branch;

        // Copy the active forward buffer blobs (oldest-first) into the
        // branch's rewind buffer. These are the frames that were popped
        // during rewind — they represent the original timeline before
        // the rewind. We COPY (not drain) so the user can still
        // Shift+Backspace to forward on the current timeline.
        let fwd_blobs: Vec<Vec<u8>> = self.active_forward_iter_blobs().cloned().collect();

        // Keep only the ancestor chain of the new branch's parent.
        // Non-ancestor branches are discarded.
        let ancestors: Vec<usize> = self.ancestor_chain(parent);
        let mut kept: Vec<TimelineBranch> = Vec::new();
        let mut old_to_new: Vec<Option<usize>> = vec![None; self.branches.len()];
        for &anc in &ancestors {
            let new_idx = kept.len();
            old_to_new[anc] = Some(new_idx);
            let dummy = TimelineBranch {
                diverge_pos: 0,
                rewind: RewindBuffer::with_capacity(1),
                forward: RewindBuffer::with_capacity(1),
                parent: None,
            };
            kept.push(std::mem::replace(&mut self.branches[anc], dummy));
        }
        for b in &mut kept {
            if let Some(old_p) = b.parent {
                b.parent = old_to_new[old_p];
            }
        }
        self.branches = kept;

        // Enforce max_branches: if at limit, drop oldest (index 0) and
        // shift all parent indices down by 1.
        let mut removed_oldest = false;
        if self.branches.len() >= self.max_branches {
            self.branches.remove(0);
            removed_oldest = true;
            for b in &mut self.branches {
                if let Some(p) = b.parent.as_mut() {
                    if *p > 0 {
                        *p -= 1;
                    } else {
                        b.parent = None;
                    }
                }
            }
        }

        // Remap active_branch after reindexing and max_branches enforcement.
        // old_to_new maps old indices to pre-enforcement indices. If the
        // oldest was removed, shift down by 1 (index 0 → None = root).
        self.active_branch = parent.and_then(|p| {
            let mapped = old_to_new.get(p).copied().flatten();
            if removed_oldest {
                mapped.and_then(|idx| if idx > 0 { Some(idx - 1) } else { None })
            } else {
                mapped
            }
        });

        // The new branch's parent is the remapped active_branch.
        let new_parent = self.active_branch;

        // Build the branch's rewind buffer from the copied forward blobs.
        let mut branch_rewind = RewindBuffer::with_capacity(crate::save_state::MAX_REWIND_CAPACITY);
        for blob in &fwd_blobs {
            branch_rewind.push_blob(blob.clone());
        }

        let new_idx = self.branches.len();
        self.branches.push(TimelineBranch {
            diverge_pos,
            rewind: branch_rewind,
            forward: RewindBuffer::with_capacity(crate::save_state::MAX_REWIND_CAPACITY),
            parent: new_parent,
        });
        // DON'T switch active_branch — stay on the current timeline.
        // The branch is now a "side timeline" that can be accessed by
        // rewinding to the diverge_pos and accepting the branch point.
        eprintln!(
            "nes-emu: created branch {} at diverge_pos {} (parent {:?}), {} frames copied from forward buffer",
            new_idx,
            diverge_pos,
            new_parent,
            self.branches[new_idx].rewind.len()
        );
    }

    /// Get the ancestor chain (including self) of a branch index, from
    /// root to the branch itself.
    fn ancestor_chain(&self, idx: Option<usize>) -> Vec<usize> {
        let mut chain = Vec::new();
        let mut current = idx;
        while let Some(i) = current {
            if i >= self.branches.len() {
                break;
            }
            chain.push(i);
            current = self.branches[i].parent;
        }
        chain.reverse();
        chain
    }

    /// Get render info for all branches: (diverge_pos, rewind_len,
    /// forward_len, depth) for each branch, where depth is the nesting
    /// level (0 = child of root, 1 = grandchild, etc.).
    pub fn branch_render_info(&self) -> Vec<(usize, usize, usize, u32)> {
        self.branches
            .iter()
            .map(|b| {
                let d = self.branch_depth(b);
                (b.diverge_pos, b.rewind.len(), b.forward.len(), d)
            })
            .collect()
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

    /// Advance the timeline animation: increment the frame counter and
    /// ramp opacity toward the target (full when rewinding/forwarding,
    /// zero otherwise). Called once per frame by the main loop.
    pub fn tick_animation(&mut self) {
        if self.rewinding || self.forwarding {
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
        self.paused_at_start = false;
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
        let cfg = crate::rewind_timeline::TimelineConfig {
            filled: self.active_rewind_len(),
            capacity: self.rewind.capacity(),
            mode,
            rewind_interval: REWIND_INTERVAL as usize,
            opacity: self.timeline_opacity,
            anim_frame: self.anim_frame,
            speed: self.rewind_speed,
            forward_filled: self.active_forward_len(),
            branches: self.branch_render_info(),
        };
        crate::rewind_timeline::render_timeline(fb, width, height, &cfg);
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
            Keycode::O if self.paused_at_branch.is_some() => {
                self.accept_branch();
                true
            }
            Keycode::P if self.paused_at_branch.is_some() => {
                self.deny_branch();
                true
            }
            // Backspace: start rewinding (no Shift) or forwarding (Shift held).
            // While held, the main loop pops one snapshot per vsync tick
            // instead of stepping forward. Shift+Backspace starts the forward
            // buffer playback to return to where the user started rewinding.
            Keycode::Backspace => {
                let shift_held = keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD);
                if shift_held {
                    // Shift+Backspace: start forwarding through the forward buffer.
                    if !self.forwarding {
                        self.start_forward();
                    }
                } else if !self.rewinding {
                    self.rewinding = true;
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
}
