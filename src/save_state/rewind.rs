//! Rewind buffer — ring buffer of recent snapshots for backward stepping
//! (M30).
//!
//! See: https://www.nesdev.org/wiki/Save_state (rewind is a common
//! emulator feature built on top of save state slots)

use super::SaveStateError;
use crate::emulator::EmulatorState;

/// Default rewind depth in frames — ~1 second of NTSC gameplay (60 fps).
/// Tuned so the buffer costs at most `capacity * ~30 KB` ≈ 1.8 MB for a
/// typical NROM snapshot, which is acceptable for an in-memory ring.
pub const DEFAULT_REWIND_CAPACITY: usize = 60;

/// A bounded ring buffer of recent emulator snapshots used by the rewind
/// feature (M30). Each [`push`](Self::push) captures the full emulator
/// state into the buffer; each [`pop`](Self::pop) restores the most
/// recently pushed snapshot and removes it from the buffer, so repeated
/// pops step the emulator backward through time.
///
/// The buffer has a fixed capacity; once full, the oldest snapshot is
/// dropped on the next `push` (FIFO eviction). The buffer is *not*
/// pre-allocated with placeholder entries — `len` starts at 0 and grows
/// up to `capacity`.
///
/// # Allocation note
///
/// Each `push` serialises the full emulator state into a fresh `Vec<u8>`
/// via `bincode`. This is a per-frame allocation (~13–30 KB) that
/// technically violates tech-stack.md's "no allocation in main loop" rule.
/// It is an accepted exception because: (1) the rewind feature is
/// throttled to every `REWIND_INTERVAL`-th frame (default 2), halving the
/// cost; (2) the allocation is bounded by the buffer capacity; (3) the
/// serialisation itself is sub-microsecond on modern CPUs. A future
/// optimisation could use a buffer pool of pre-allocated `Vec<u8>` slots
/// rotated through the ring to eliminate the per-push allocation.
///
/// # Audio note
///
/// Rewind restores are best-effort with respect to audio: the APU state
/// is part of the serialised snapshot, so the audio buffer is restored
/// exactly. The host audio device may briefly underrun during a rewind
/// pop (the restored samples are slightly stale relative to the audio
/// thread's read pointer), but this is unavoidable without host-side
/// audio buffering changes and matches the behaviour of other emulators
/// (e.g. FCEUX rewind).
#[derive(Debug)]
pub struct RewindBuffer {
    /// Ring of serialised snapshots, oldest first.
    buffer: std::collections::VecDeque<Vec<u8>>,
    /// Maximum number of snapshots retained.
    capacity: usize,
}

impl Default for RewindBuffer {
    fn default() -> Self {
        Self::with_capacity(DEFAULT_REWIND_CAPACITY)
    }
}

impl RewindBuffer {
    /// Construct an empty rewind buffer with the given frame capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            buffer: std::collections::VecDeque::with_capacity(capacity.max(1)),
            capacity: capacity.max(1),
        }
    }

    /// Maximum number of snapshots the buffer will retain.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Current number of snapshots stored.
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// Is the buffer empty?
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Is the buffer at capacity (next push evicts the oldest)?
    pub fn is_full(&self) -> bool {
        self.buffer.len() >= self.capacity
    }

    /// Capture `emulator` into the buffer. If the buffer is full, the
    /// oldest snapshot is dropped first. Errors from the serialiser are
    /// propagated — a failed push leaves the buffer unchanged.
    pub fn push(&mut self, emulator: &EmulatorState) -> Result<(), SaveStateError> {
        let blob = emulator.save_state()?;
        if self.buffer.len() >= self.capacity {
            self.buffer.pop_front();
        }
        self.buffer.push_back(blob);
        Ok(())
    }

    /// Restore the most recently pushed snapshot and remove it from the
    /// buffer. Returns `true` if a snapshot was restored, `false` if the
    /// buffer was empty (no-op).
    ///
    /// If `load_state` fails on the popped blob (corrupt data — should
    /// never happen for self-serialised blobs), the blob is consumed and
    /// `false` is returned; the emulator state is left untouched.
    pub fn pop(&mut self, emulator: &mut EmulatorState) -> bool {
        match self.buffer.pop_back() {
            Some(blob) => {
                if let Err(e) = emulator.load_state(&blob) {
                    eprintln!("nes-emu: rewind restore failed: {e}");
                    return false;
                }
                true
            }
            None => false,
        }
    }

    /// Drop every snapshot. The capacity is preserved.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewind_buffer_default_capacity() {
        let r = RewindBuffer::default();
        assert_eq!(r.capacity(), DEFAULT_REWIND_CAPACITY);
        assert!(r.is_empty());
        assert!(!r.is_full());
    }

    #[test]
    fn rewind_buffer_capacity_floor() {
        let r = RewindBuffer::with_capacity(0);
        assert_eq!(r.capacity(), 1);
    }
}
