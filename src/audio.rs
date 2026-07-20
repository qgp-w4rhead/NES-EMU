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
pub struct AudioOutput {
    queue: AudioQueue<f32>,
}

impl AudioOutput {
    /// Open the audio device with the desired spec (44.1 kHz mono f32)
    /// and start playback immediately.
    pub fn new(audio_subsystem: &sdl2::AudioSubsystem) -> Result<Self, String> {
        let spec = AudioSpecDesired {
            freq: Some(SAMPLE_RATE),
            channels: Some(CHANNELS),
            samples: Some(BUFFER_SAMPLES),
        };
        let queue = AudioQueue::open_queue(audio_subsystem, None, &spec)
            .map_err(|e| format!("audio queue open failed: {e}"))?;
        queue.resume();
        Ok(Self { queue })
    }

    /// Queue a batch of `f32` samples (in `[-1.0, 1.0]`) for playback.
    /// Called once per frame from the emulation thread.
    pub fn push_samples(&self, samples: &[f32]) -> Result<(), String> {
        self.queue.queue_audio(samples)
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
}
