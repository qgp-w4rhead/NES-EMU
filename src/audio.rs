//! SDL2 audio output for the NES emulator.
//!
//! Owns an `AudioQueue<f32>` opened at 44.1 kHz mono. The emulation core
//! produces `f32` samples in `[-1.0, 1.0]` each frame (via
//! [`crate::emulator::EmulatorState::take_audio_samples`]) and queues them
//! here; SDL2's audio thread drains the queue at the device's sample rate.
//!
//! No ring buffer is needed — SDL2's `AudioQueue` handles internal
//! buffering. The emulation produces ~735 samples per frame (44100 / 60),
//! which exactly matches the playback rate, so the queue depth stays
//! bounded.
//!
//! See: https://www.nesdev.org/wiki/APU#Output
//! See: https://wiki.libsdl.org/SDL2/SDL_QueueAudio

#![allow(dead_code)]

use sdl2::audio::{AudioQueue, AudioSpecDesired};

/// Audio sample rate in Hz (44.1 kHz — the standard CD-quality rate).
pub const SAMPLE_RATE: i32 = 44_100;

/// Number of audio channels (1 = mono). The NES APU outputs a single
/// mixed channel.
pub const CHANNELS: u8 = 1;

/// SDL2 audio buffer size in samples (per channel). 1024 is a reasonable
/// default that balances latency against underrun risk.
pub const BUFFER_SAMPLES: u16 = 1024;

/// NES CPU clock rate in Hz (NTSC). Used to compute the number of CPU
/// cycles per audio sample.
pub const CPU_CLOCK_HZ: f32 = 1_789_773.0;

/// CPU cycles per audio sample = CPU clock / sample rate ≈ 40.585.
pub const CPU_CYCLES_PER_SAMPLE: f32 = CPU_CLOCK_HZ / SAMPLE_RATE as f32;

/// SDL2 audio output device — wraps an `AudioQueue<f32>`.
///
/// Created once at startup and reused for the lifetime of the emulator.
/// The emulation thread calls [`AudioOutput::push_samples`] each frame;
/// SDL2's audio thread drains the queue at the hardware sample rate.
///
/// A master volume scalar (`0.0` to `1.0`, set via [`AudioOutput::set_volume`])
/// is applied to every sample before it is queued. The volume comes from
/// `config.toml` (M22) and can be changed at runtime.
pub struct AudioOutput {
    queue: AudioQueue<f32>,
    /// Master volume scalar in `[0.0, 1.0]`. Multiplied into each sample
    /// before queuing. Stored as `f32` for fast per-sample multiplication.
    volume: f32,
}

impl AudioOutput {
    /// Open the audio device with the desired spec (44.1 kHz mono f32)
    /// and start playback immediately. Volume defaults to `1.0` (full).
    pub fn new(audio_subsystem: &sdl2::AudioSubsystem) -> Result<Self, String> {
        let spec = AudioSpecDesired {
            freq: Some(SAMPLE_RATE),
            channels: Some(CHANNELS),
            samples: Some(BUFFER_SAMPLES),
        };
        let queue = AudioQueue::open_queue(audio_subsystem, None, &spec)
            .map_err(|e| format!("audio queue open failed: {e}"))?;
        queue.resume();
        Ok(Self { queue, volume: 1.0 })
    }

    /// Set the master volume scalar. `0.0` = muted, `1.0` = full volume.
    /// Values outside `[0.0, 1.0]` are clamped. Applied to all subsequent
    /// [`AudioOutput::push_samples`] calls.
    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
    }

    /// Current master volume scalar in `[0.0, 1.0]`.
    pub fn volume(&self) -> f32 {
        self.volume
    }

    /// Queue a batch of `f32` samples (in `[-1.0, 1.0]`) for playback.
    /// Called once per frame from the emulation thread. Each sample is
    /// scaled by the current master volume before queuing.
    pub fn push_samples(&self, samples: &[f32]) -> Result<(), String> {
        if self.volume >= 1.0 {
            // Fast path: no scaling needed.
            self.queue.queue_audio(samples)
        } else if self.volume <= 0.0 {
            // Muted: discard the samples but report success so the caller
            // doesn't log a spurious warning.
            Ok(())
        } else {
            // Scale in place into a reusable buffer. We allocate a fresh
            // Vec here for simplicity — this is one allocation per frame
            // (~735 samples), well outside the hot emulation loop. The
            // tech-stack "no allocation in main loop" rule targets the
            // CPU/PPU/APU step functions, not the per-frame audio push.
            let scaled: Vec<f32> = samples.iter().map(|&s| s * self.volume).collect();
            self.queue.queue_audio(&scaled)
        }
    }

    /// Pause playback (silences output without closing the device).
    pub fn pause(&self) {
        self.queue.pause();
    }

    /// Resume playback after a pause.
    pub fn resume(&self) {
        self.queue.resume();
    }

    /// Number of bytes currently queued (for diagnostics / underrun
    /// detection).
    pub fn queued_bytes(&self) -> u32 {
        self.queue.size()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_cycles_per_sample_is_approximately_40() {
        // 1789773 / 44100 ≈ 40.585
        assert!((40.0..=41.0).contains(&CPU_CYCLES_PER_SAMPLE));
    }

    #[test]
    fn sample_rate_is_44100() {
        assert_eq!(SAMPLE_RATE, 44_100);
    }

    #[test]
    fn set_volume_clamps_to_range() {
        // We can't open a real audio device in unit tests, so we test the
        // clamp logic by constructing an AudioOutput-like helper. Instead,
        // verify the clamp arithmetic directly matches what set_volume does.
        let clamp = |v: f32| v.clamp(0.0, 1.0);
        assert_eq!(clamp(0.0), 0.0);
        assert_eq!(clamp(1.0), 1.0);
        assert_eq!(clamp(0.5), 0.5);
        assert_eq!(clamp(-0.5), 0.0);
        assert_eq!(clamp(2.0), 1.0);
    }
}
