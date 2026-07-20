//! APU (Audio Processing Unit) — pulse wave channels 1 & 2.
//!
//! This module implements the two pulse channels of the NES APU, the first
//! piece of the audio subsystem (M14). Each pulse channel has four
//! sub-components driven by the channel's 11-bit timer:
//!
//! - **Waveform generator** — 4 selectable duty-cycle patterns (8-step
//!   sequence) clocked each time the timer reloads.
//! - **Envelope** — downward sawtooth volume envelope with configurable
//!   divider, loop/halt, and constant-volume mode.
//! - **Sweep** — periodically shifts the timer period up or down; can mute
//!   the channel when the result goes out of range.
//! - **Length counter** — automatic note duration; silences the channel
//!   when it reaches zero (unless halted).
//!
//! The frame counter (M16) clocks the envelope at the quarter-frame rate
//! (≈240 Hz NTSC) and the length counter + sweep at the half-frame rate
//! (≈120 Hz). Until M16 lands, [`Apu::clock_quarter_frame`] and
//! [`Apu::clock_half_frame`] are exposed so the sub-components can be
//! unit-tested and so the frame counter can drive them later.
//!
//! Register map (see <https://www.nesdev.org/wiki/APU_Pulse>):
//!
//! | Register        | Bits          | Function                                     |
//! |-----------------|---------------|----------------------------------------------|
//! | `$4000`/`$4004` | `DDLC VVVV`   | Duty, halt/loop, constant-volume, volume     |
//! | `$4001`/`$4005` | `EPPP NSSS`   | Sweep enable/period/negate/shift             |
//! | `$4002`/`$4006` | `TTTT TTTT`   | Timer low 8 bits                             |
//! | `$4003`/`$4007` | `LLLL LTTT`   | Length load + timer high 3 bits              |
//!
//! See: https://www.nesdev.org/wiki/APU
//! See: https://www.nesdev.org/wiki/APU_Pulse
//! See: https://www.nesdev.org/wiki/APU_Envelope
//! See: https://www.nesdev.org/wiki/APU_Sweep
//! See: https://www.nesdev.org/wiki/APU_Length_Counter

#![allow(dead_code)]

/// 4 duty-cycle patterns, each an 8-step sequence clocked by the timer.
/// The bit at the current sequence position selects whether the channel
/// outputs its volume or is silent for that timer period.
///
/// See: https://www.nesdev.org/wiki/APU_Pulse#Sequencer
const DUTY_PATTERNS: [[u8; 8]; 4] = [
    [0, 1, 0, 0, 0, 0, 0, 0], // 0: 12.5%
    [0, 1, 1, 0, 0, 0, 0, 0], // 1: 25%
    [0, 1, 1, 1, 1, 0, 0, 0], // 2: 50%
    [1, 0, 0, 0, 1, 1, 1, 1], // 3: 25% negated
];

/// Length-counter lookup table indexed by the 5-bit length index (bits 7-3
/// of `$4003`/`$4007`/`$400B`/`$400F`). Each entry is the loaded length
/// value (in half-frame clocks).
///
/// See: https://www.nesdev.org/wiki/APU_Length_Counter#Length_table
const LENGTH_TABLE: [u8; 32] = [
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14, 12, 16, 24, 18, 48, 20, 96, 22,
    192, 24, 72, 26, 16, 28, 32, 30,
];

/// Maximum 11-bit timer period value (the period is 11 bits, `$000-$7FF`).
const MAX_PERIOD: u16 = 0x7FF;

/// A pulse channel's period below this value silences the channel (sweep
/// unit mute). See: https://www.nesdev.org/wiki/APU_Sweep
const MIN_AUDIBLE_PERIOD: u16 = 8;

/// One of the two NES pulse wave channels.
///
/// `pulse2` selects the pulse-2 sweep negate variant, which subtracts one
/// extra from the target period (a hardware quirk; see APU_Sweep).
pub struct PulseChannel {
    /// `true` for pulse channel 2 (affects sweep negate target).
    pulse2: bool,

    // ---- $4000/$4004: duty + envelope/length control -------------------
    /// Duty cycle selection (2 bits → 0..=3 index into `DUTY_PATTERNS`).
    duty: u8,
    /// Length-counter halt / envelope loop flag (bit 5 of $4000).
    halt: bool,
    /// Constant-volume mode (bit 4 of $4000). When set the envelope is
    /// bypassed and `volume` is used directly.
    constant_volume: bool,
    /// Volume / envelope divider period (bits 3-0 of $4000), 0..=15.
    volume: u8,

    // ---- $4001/$4005: sweep unit ----------------------------------------
    /// Sweep enabled (bit 7).
    sweep_enabled: bool,
    /// Sweep divider period (bits 6-4, raw 3-bit value). The divider
    /// counts `period + 1` half-frames per sweep.
    sweep_period: u8,
    /// Sweep negate flag (bit 3) — subtract instead of add.
    sweep_negate: bool,
    /// Sweep shift count (bits 2-0). 0 disables the sweep.
    sweep_shift: u8,
    /// Current sweep divider count.
    sweep_divider: u8,
    /// Reload flag — set on $4001 write; reloads the divider on the next
    /// half-frame clock.
    sweep_reload: bool,

    // ---- $4002/$4006 + $4003/$4007: timer -------------------------------
    /// 11-bit timer reload value (period).
    timer_period: u16,
    /// Running timer counter.
    timer: u16,
    /// Current position in the 8-step duty sequence (0..=7).
    sequence: u8,

    // ---- Length counter -------------------------------------------------
    /// Current length counter value; 0 silences the channel.
    length_counter: u8,
    /// Channel enable from `$4015` bit 0 (pulse 1) / bit 1 (pulse 2).
    /// When cleared the length counter is forced to 0.
    enabled: bool,

    // ---- Envelope -------------------------------------------------------
    /// Envelope divider count.
    envelope_divider: u8,
    /// Current envelope decay level (0..=15) — used as the output volume
    /// when not in constant-volume mode.
    envelope_decay: u8,
    /// Start flag — set on $4003 write; restarts the envelope on the next
    /// quarter-frame clock.
    envelope_start: bool,
}

impl PulseChannel {
    /// Create a new, fully-reset pulse channel. `pulse2` selects the
    /// pulse-2 sweep negate quirk.
    fn new(pulse2: bool) -> Self {
        Self {
            pulse2,
            duty: 0,
            halt: false,
            constant_volume: false,
            volume: 0,
            sweep_enabled: false,
            sweep_period: 0,
            sweep_negate: false,
            sweep_shift: 0,
            sweep_divider: 0,
            sweep_reload: false,
            timer_period: 0,
            timer: 0,
            sequence: 0,
            length_counter: 0,
            enabled: false,
            envelope_divider: 0,
            envelope_decay: 0,
            envelope_start: false,
        }
    }

    /// Write to one of the four pulse channel registers.
    ///
    /// `reg` is `0..=3` (the low 2 bits of the address after subtracting
    /// the channel base `$4000` or `$4004`).
    ///
    /// See: https://www.nesdev.org/wiki/APU_Pulse#Registers
    pub fn write_register(&mut self, reg: u8, value: u8) {
        match reg {
            0 => {
                // $4000/$4004: DDLC VVVV
                self.duty = (value >> 6) & 0x03;
                self.halt = (value & 0x20) != 0;
                self.constant_volume = (value & 0x10) != 0;
                self.volume = value & 0x0F;
            }
            1 => {
                // $4001/$4005: EPPP NSSS — sets sweep params + reload flag.
                self.sweep_enabled = (value & 0x80) != 0;
                self.sweep_period = (value >> 4) & 0x07;
                self.sweep_negate = (value & 0x08) != 0;
                self.sweep_shift = value & 0x07;
                self.sweep_reload = true;
            }
            2 => {
                // $4002/$4006: timer low 8 bits.
                self.timer_period = (self.timer_period & 0xFF00) | value as u16;
            }
            3 => {
                // $4003/$4007: LLLL LTTT — length load + timer high 3 bits.
                let high = (value & 0x07) as u16;
                self.timer_period = (self.timer_period & 0x00FF) | (high << 8);
                // The running timer's high 3 bits are set immediately; the
                // low 8 bits are not affected until the next timer tick.
                self.timer = (self.timer & 0x00FF) | (high << 8);

                // Length counter load (only if the channel is enabled via
                // $4015; otherwise the load is suppressed and the counter
                // stays at 0).
                if self.enabled {
                    let idx = (value >> 3) as usize;
                    self.length_counter = LENGTH_TABLE[idx];
                }

                // Side effects: envelope restart + sequencer reset.
                self.envelope_start = true;
                self.sequence = 0;
            }
            _ => {}
        }
    }

    /// Set the channel enable flag from `$4015`. When cleared, the length
    /// counter is immediately forced to 0 (silencing the channel).
    ///
    /// See: https://www.nesdev.org/wiki/APU_Status
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if !enabled {
            self.length_counter = 0;
        }
    }

    /// Whether the channel is currently enabled via `$4015`.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Current length counter value (0 means the channel is silenced).
    pub fn length_counter(&self) -> u8 {
        self.length_counter
    }

    /// Current timer period (11-bit reload value).
    pub fn timer_period(&self) -> u16 {
        self.timer_period
    }

    /// Current running timer counter value.
    pub fn timer(&self) -> u16 {
        self.timer
    }

    /// Current envelope decay level (the volume used in envelope mode).
    pub fn envelope_decay(&self) -> u8 {
        self.envelope_decay
    }

    /// Current duty sequence position (0..=7).
    pub fn sequence(&self) -> u8 {
        self.sequence
    }

    /// Compute the sweep target period for the current settings.
    ///
    /// When the sweep is disabled or the shift count is zero, the target
    /// equals the current period (no change). Otherwise:
    ///
    /// - negate clear: `target = period + (period >> shift)`
    /// - negate set, pulse 1: `target = period - (period >> shift)`
    /// - negate set, pulse 2: `target = period - (period >> shift) - 1`
    ///
    /// Subtraction uses wrapping arithmetic; an underflow produces a value
    /// greater than `MAX_PERIOD`, which the mute check catches.
    ///
    /// See: https://www.nesdev.org/wiki/APU_Sweep#Calculating_the_target_period
    fn sweep_target(&self) -> u16 {
        if !self.sweep_enabled || self.sweep_shift == 0 {
            return self.timer_period;
        }
        let shifted = self.timer_period >> self.sweep_shift;
        if self.sweep_negate {
            if self.pulse2 {
                self.timer_period.wrapping_sub(shifted).wrapping_sub(1)
            } else {
                self.timer_period.wrapping_sub(shifted)
            }
        } else {
            self.timer_period.wrapping_add(shifted)
        }
    }

    /// Whether the sweep unit is currently muting the channel. This happens
    /// when the current period is below 8 or the sweep target exceeds the
    /// 11-bit range. The mute applies regardless of whether the sweep is
    /// enabled.
    ///
    /// See: https://www.nesdev.org/wiki/APU_Sweep#Muting
    fn is_muted(&self) -> bool {
        self.timer_period < MIN_AUDIBLE_PERIOD || self.sweep_target() > MAX_PERIOD
    }

    /// Advance the channel by one APU cycle (half a CPU cycle). The timer
    /// counts down; on reaching zero it reloads to the period and the
    /// waveform sequencer advances one step.
    ///
    /// See: https://www.nesdev.org/wiki/APU_Pulse#Timer
    pub fn tick(&mut self) {
        if self.timer == 0 {
            self.timer = self.timer_period;
            self.sequence = (self.sequence + 1) & 0x07;
        } else {
            self.timer -= 1;
        }
    }

    /// Clock the envelope (quarter-frame signal from the frame counter).
    ///
    /// See: https://www.nesdev.org/wiki/APU_Envelope#Clocking
    pub fn clock_envelope(&mut self) {
        if self.envelope_start {
            self.envelope_start = false;
            self.envelope_decay = 15;
            self.envelope_divider = self.volume;
        } else if self.envelope_divider == 0 {
            self.envelope_divider = self.volume;
            if self.envelope_decay > 0 {
                self.envelope_decay -= 1;
            } else if self.halt {
                // Loop: envelope restarts from 15.
                self.envelope_decay = 15;
            }
        } else {
            self.envelope_divider -= 1;
        }
    }

    /// Clock the length counter (half-frame signal). The counter is held
    /// (not decremented) when the halt flag is set.
    ///
    /// See: https://www.nesdev.org/wiki/APU_Length_Counter#Clocking
    fn clock_length(&mut self) {
        if !self.halt && self.length_counter > 0 {
            self.length_counter -= 1;
        }
    }

    /// Clock the sweep unit (half-frame signal).
    ///
    /// Per NESdev, two things happen on each half-frame clock:
    /// 1. If the divider is zero (and the sweep is enabled, shift nonzero,
    ///    and not muting), the target period is applied. The reload flag
    ///    does **not** trigger target application — only a zero divider does.
    /// 2. If the divider is zero **or** the reload flag is set, the divider
    ///    is reloaded to `period` and the reload flag cleared; otherwise the
    ///    divider is decremented.
    ///
    /// See: https://www.nesdev.org/wiki/APU_Sweep#Clocking
    fn clock_sweep(&mut self) {
        let divider_zero = self.sweep_divider == 0;
        // Step 1: apply the target only when the divider reached zero.
        if divider_zero && self.sweep_enabled && self.sweep_shift != 0 {
            let target = self.sweep_target();
            if target <= MAX_PERIOD && self.timer_period >= MIN_AUDIBLE_PERIOD {
                self.timer_period = target;
            }
        }
        // Step 2: reload or decrement the divider.
        if divider_zero || self.sweep_reload {
            self.sweep_divider = self.sweep_period;
            self.sweep_reload = false;
        } else {
            self.sweep_divider -= 1;
        }
    }

    /// Quarter-frame clock: clocks the envelope.
    pub fn clock_quarter_frame(&mut self) {
        self.clock_envelope();
    }

    /// Half-frame clock: clocks the length counter and sweep unit.
    pub fn clock_half_frame(&mut self) {
        self.clock_length();
        self.clock_sweep();
    }

    /// Current output sample (0..=15). Returns 0 when the channel is
    /// silenced (disabled, length counter zero, sweep mute, or the duty bit
    /// at the current sequence position is 0).
    ///
    /// See: https://www.nesdev.org/wiki/APU_Pulse#Output
    pub fn sample(&self) -> u8 {
        if !self.enabled || self.length_counter == 0 || self.is_muted() {
            return 0;
        }
        let duty_bit = DUTY_PATTERNS[self.duty as usize][self.sequence as usize];
        if duty_bit == 0 {
            return 0;
        }
        if self.constant_volume {
            self.volume
        } else {
            self.envelope_decay
        }
    }
}

/// The APU — currently owns the two pulse channels. Triangle, noise, DMC,
/// and the frame counter are added in M15/M16.
pub struct Apu {
    pulse1: PulseChannel,
    pulse2: PulseChannel,
    /// Half-cycle accumulator: the APU runs at CPU clock / 2, so one APU
    /// cycle is two CPU cycles. This accumulates fractional APU cycles
    /// across `step` calls.
    cycle_accumulator: u32,
}

impl Apu {
    /// Build a reset APU with both pulse channels silenced.
    pub fn new() -> Self {
        Self {
            pulse1: PulseChannel::new(false),
            pulse2: PulseChannel::new(true),
            cycle_accumulator: 0,
        }
    }

    /// Borrow pulse channel 1.
    pub fn pulse1(&self) -> &PulseChannel {
        &self.pulse1
    }

    /// Mutably borrow pulse channel 1.
    pub fn pulse1_mut(&mut self) -> &mut PulseChannel {
        &mut self.pulse1
    }

    /// Borrow pulse channel 2.
    pub fn pulse2(&self) -> &PulseChannel {
        &self.pulse2
    }

    /// Mutably borrow pulse channel 2.
    pub fn pulse2_mut(&mut self) -> &mut PulseChannel {
        &mut self.pulse2
    }

    /// Write the status register `$4015`. Bits 0 and 1 enable/disable the
    /// two pulse channels (clearing the bit forces the channel's length
    /// counter to zero). Other bits are handled in M15/M16.
    ///
    /// See: https://www.nesdev.org/wiki/APU_Status
    pub fn write_status(&mut self, value: u8) {
        self.pulse1.set_enabled(value & 0x01 != 0);
        self.pulse2.set_enabled(value & 0x02 != 0);
    }

    /// Read the status register `$4015`. Bits 0 and 1 reflect whether each
    /// pulse channel's length counter is non-zero. Other bits (triangle,
    /// noise, DMC, IRQ flags) are 0 until M15/M16.
    ///
    /// See: https://www.nesdev.org/wiki/APU_Status
    pub fn read_status(&self) -> u8 {
        let mut v = 0u8;
        if self.pulse1.length_counter > 0 {
            v |= 0x01;
        }
        if self.pulse2.length_counter > 0 {
            v |= 0x02;
        }
        v
    }

    /// Advance the APU by `cpu_cycles` CPU cycles. The pulse channel timers
    /// tick once per APU cycle (every 2 CPU cycles). Frame-counter clocking
    /// (quarter/half-frame) is added in M16.
    pub fn step(&mut self, cpu_cycles: u32) {
        self.cycle_accumulator = self.cycle_accumulator.saturating_add(cpu_cycles);
        while self.cycle_accumulator >= 2 {
            self.cycle_accumulator -= 2;
            self.pulse1.tick();
            self.pulse2.tick();
        }
    }

    /// Quarter-frame signal — clocks the pulse envelopes. Called by the
    /// frame counter (M16) at ≈240 Hz NTSC.
    pub fn clock_quarter_frame(&mut self) {
        self.pulse1.clock_quarter_frame();
        self.pulse2.clock_quarter_frame();
    }

    /// Half-frame signal — clocks the pulse length counters and sweep
    /// units. Called by the frame counter (M16) at ≈120 Hz NTSC.
    pub fn clock_half_frame(&mut self) {
        self.pulse1.clock_half_frame();
        self.pulse2.clock_half_frame();
    }

    /// Mix the current pulse channel samples into a single 0..=15 value.
    /// For M14 this is a simple sum (clamped); the non-linear mixer and
    /// remaining channels land in M15/M16.
    pub fn mix(&self) -> u8 {
        let s = self.pulse1.sample() as u16 + self.pulse2.sample() as u16;
        if s > 15 {
            15
        } else {
            s as u8
        }
    }
}

impl Default for Apu {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a pulse channel with constant volume `v` and a given timer
    /// period, enabled via $4015, with a loaded length counter. Duty 2
    /// (50%); halt clear; sequence left at 0 (duty 2 bit 0 = 0, so the
    /// channel is silent until the sequencer advances to position 1).
    fn pulse_audible(pulse2: bool, period: u16, volume: u8) -> PulseChannel {
        let mut c = PulseChannel::new(pulse2);
        c.set_enabled(true);
        // $4000: duty 2 (50%), halt clear, constant volume, volume.
        c.write_register(0, 0b1001_0000 | (volume & 0x0F));
        // $4002: timer low.
        c.write_register(2, (period & 0xFF) as u8);
        // $4003: timer high + length index 0 (length = 10).
        c.write_register(3, ((period >> 8) as u8) & 0x07);
        c
    }

    // ---- Duty patterns --------------------------------------------------

    #[test]
    fn duty_patterns_are_correct() {
        assert_eq!(DUTY_PATTERNS[0], [0, 1, 0, 0, 0, 0, 0, 0]);
        assert_eq!(DUTY_PATTERNS[1], [0, 1, 1, 0, 0, 0, 0, 0]);
        assert_eq!(DUTY_PATTERNS[2], [0, 1, 1, 1, 1, 0, 0, 0]);
        assert_eq!(DUTY_PATTERNS[3], [1, 0, 0, 0, 1, 1, 1, 1]);
    }

    #[test]
    fn timer_reload_advances_sequence_one_step() {
        // period = 0 → timer stays 0; every tick advances the sequence.
        let mut c = pulse_audible(false, 0, 15);
        let s0 = c.sequence();
        c.tick();
        assert_eq!(c.sequence(), (s0 + 1) & 7);
        c.tick();
        assert_eq!(c.sequence(), (s0 + 2) & 7);
    }

    #[test]
    fn timer_counts_down_then_reloads() {
        // period = 3. After $4003 the running timer's high 3 bits are set
        // but the low 8 bits are unchanged (0 here), so timer starts at 0.
        // The first tick reloads to the period and advances the sequencer.
        let mut c = pulse_audible(false, 3, 15);
        assert_eq!(c.timer(), 0);
        c.tick(); // 0 → reload(3) + advance
        assert_eq!(c.timer(), 3);
        c.tick(); // 3 → 2
        assert_eq!(c.timer(), 2);
        c.tick(); // 2 → 1
        c.tick(); // 1 → 0
        c.tick(); // 0 → reload(3) + advance
        assert_eq!(c.timer(), 3);
    }

    #[test]
    fn timer_4003_write_preserves_low_8_bits_of_running_timer() {
        // $4003 sets the running timer's high 3 bits but leaves the low 8
        // bits untouched. Set the low 8 bits by counting down partway, then
        // write $4003 and confirm only the high bits change.
        let mut c = pulse_audible(false, 0x1FF, 15); // period = 0x1FF
                                                     // After $4003: timer = (0 & 0xFF) | (1 << 8) = 0x100.
        assert_eq!(c.timer(), 0x100);
        // Tick once to change the low 8 bits (0x100 → 0x0FF).
        c.tick();
        assert_eq!(c.timer(), 0x0FF);
        // Now write $4003 with high = 0x3. Low 8 bits (0xFF) preserved;
        // high 3 bits become 0x3 → timer = 0x3FF.
        c.write_register(3, 0x03);
        assert_eq!(c.timer(), 0x3FF);
        assert_eq!(c.timer_period(), 0x3FF);
    }

    // ---- Length counter -------------------------------------------------

    #[test]
    fn length_table_loads_correct_values() {
        let mut c = PulseChannel::new(false);
        c.set_enabled(true);
        // Index 0 → 10, index 1 → 254.
        c.write_register(3, 0 << 3);
        assert_eq!(c.length_counter(), LENGTH_TABLE[0]);
        assert_eq!(c.length_counter(), 10);
        c.write_register(3, 1 << 3);
        assert_eq!(c.length_counter(), LENGTH_TABLE[1]);
        assert_eq!(c.length_counter(), 254);
    }

    #[test]
    fn length_counter_load_suppressed_when_disabled() {
        let mut c = PulseChannel::new(false);
        // Not enabled — length load is suppressed.
        c.write_register(3, 0 << 3);
        assert_eq!(c.length_counter(), 0);
    }

    #[test]
    fn length_counter_decrements_on_half_frame() {
        let mut c = PulseChannel::new(false);
        c.set_enabled(true);
        c.write_register(3, 0 << 3); // length = 10
        let start = c.length_counter();
        c.clock_half_frame();
        assert_eq!(c.length_counter(), start - 1);
    }

    #[test]
    fn length_counter_halt_freezes_count() {
        let mut c = PulseChannel::new(false);
        c.set_enabled(true);
        // $4000 bit 5 = halt.
        c.write_register(0, 0x20);
        c.write_register(3, 0 << 3); // length = 10
        let start = c.length_counter();
        c.clock_half_frame();
        assert_eq!(c.length_counter(), start);
    }

    #[test]
    fn length_counter_stops_at_zero() {
        let mut c = PulseChannel::new(false);
        c.set_enabled(true);
        // Load length 2 (index 3 → length 2).
        c.write_register(3, 3 << 3);
        assert_eq!(c.length_counter(), 2);
        c.clock_half_frame();
        assert_eq!(c.length_counter(), 1);
        c.clock_half_frame();
        assert_eq!(c.length_counter(), 0);
        c.clock_half_frame();
        assert_eq!(c.length_counter(), 0);
    }

    #[test]
    fn disabling_channel_via_4015_clears_length() {
        let mut c = PulseChannel::new(false);
        c.set_enabled(true);
        c.write_register(3, 0 << 3);
        assert!(c.length_counter() > 0);
        c.set_enabled(false);
        assert_eq!(c.length_counter(), 0);
    }

    // ---- Envelope -------------------------------------------------------

    #[test]
    fn envelope_restarts_on_4003_write() {
        let mut c = PulseChannel::new(false);
        // Run envelope down a bit first.
        c.write_register(0, 0x00); // volume 0, envelope mode, no loop
        c.write_register(3, 0); // triggers envelope_start
        c.clock_envelope(); // start → decay=15, divider=0
        assert_eq!(c.envelope_decay(), 15);
        // Decay a few times.
        for _ in 0..3 {
            c.clock_envelope();
        }
        assert!(c.envelope_decay() < 15);
        // Restart.
        c.write_register(3, 0);
        c.clock_envelope();
        assert_eq!(c.envelope_decay(), 15);
    }

    #[test]
    fn envelope_decays_one_step_per_volume_plus_one_clocks() {
        let mut c = PulseChannel::new(false);
        c.write_register(0, 0x00); // volume = 0 → divider period 1
        c.write_register(3, 0); // start
        c.clock_envelope(); // start → decay=15, divider=0
        assert_eq!(c.envelope_decay(), 15);
        // divider=0 → reload to 0, decay→14.
        c.clock_envelope();
        assert_eq!(c.envelope_decay(), 14);
        c.clock_envelope();
        assert_eq!(c.envelope_decay(), 13);
    }

    #[test]
    fn envelope_loops_when_halt_set() {
        // volume 0 → divider period 1 → one decay step per clock. halt/loop
        // makes the envelope restart from 15 after reaching 0.
        let mut c = PulseChannel::new(false);
        c.write_register(0, 0x20); // halt/loop, volume 0
        c.write_register(3, 0); // start
        c.clock_envelope(); // start → decay=15, divider=0
                            // 15 clocks: decay 15 → 0.
        for _ in 0..15 {
            c.clock_envelope();
        }
        assert_eq!(c.envelope_decay(), 0);
        // 16th clock: decay 0 + halt → loop back to 15.
        c.clock_envelope();
        assert_eq!(c.envelope_decay(), 15);
        // And it continues decaying again.
        c.clock_envelope();
        assert_eq!(c.envelope_decay(), 14);
    }

    #[test]
    fn envelope_constant_volume_bypasses_decay() {
        // duty 3 (seq 0 bit = 1) so the channel is audible at sequence 0.
        let mut c = PulseChannel::new(false);
        c.set_enabled(true);
        c.write_register(0, 0b1101_0111); // duty 3, const vol 7
        c.write_register(2, 0x10);
        c.write_register(3, 0x00); // period = 16, length = 10
        assert_eq!(c.sample(), 7);
        // Clocking the envelope should not change the output volume.
        for _ in 0..20 {
            c.clock_envelope();
            assert_eq!(c.sample(), 7);
        }
    }

    #[test]
    fn envelope_decay_used_as_volume_in_envelope_mode() {
        let mut c = PulseChannel::new(false);
        c.set_enabled(true);
        // duty 2, envelope mode (const vol clear), volume 15, no halt.
        c.write_register(0, 0b1000_1111);
        c.write_register(2, 0x08); // period low = 8
        c.write_register(3, 0x00); // period = 8, length = 10, restart env, seq 0
        c.clock_envelope(); // start → decay = 15
                            // duty 2 seq 0 = 0 → silent; advance to seq 1 (timer 0 → reload + advance).
        c.tick();
        assert_eq!(c.sample(), 15);
        // Decay the envelope: volume 15 → divider period 16 → 16 clocks per
        // decay step. After 16 clocks decay should be 14.
        for _ in 0..16 {
            c.clock_envelope();
        }
        assert_eq!(c.envelope_decay(), 14);
        assert_eq!(c.sample(), 14);
    }

    // ---- Sweep ----------------------------------------------------------

    #[test]
    fn sweep_target_add_when_not_negate() {
        let mut c = PulseChannel::new(false);
        c.write_register(2, 0x00); // timer low = 0
        c.write_register(3, 0x04); // timer high = 4 → period = 0x400
        c.write_register(1, 0b1000_0001); // enabled, period 0, no negate, shift 1
        assert_eq!(c.sweep_target(), 0x400 + (0x400 >> 1));
    }

    #[test]
    fn sweep_target_subtract_pulse1() {
        let mut c = PulseChannel::new(false);
        c.write_register(2, 0x00);
        c.write_register(3, 0x04); // period = 0x400
        c.write_register(1, 0b1000_1001); // enabled, negate, shift 1
        assert_eq!(c.sweep_target(), 0x400 - (0x400 >> 1));
    }

    #[test]
    fn sweep_target_subtract_pulse2_has_minus_one_quirk() {
        let mut c = PulseChannel::new(true);
        c.write_register(2, 0x00);
        c.write_register(3, 0x04); // period = 0x400
        c.write_register(1, 0b1000_1001); // enabled, negate, shift 1
        assert_eq!(c.sweep_target(), 0x400 - (0x400 >> 1) - 1);
    }

    #[test]
    fn sweep_disabled_returns_current_period() {
        let mut c = PulseChannel::new(false);
        c.write_register(2, 0x00);
        c.write_register(3, 0x04); // period = 0x400
        c.write_register(1, 0b0000_0001); // disabled, shift 1
        assert_eq!(c.sweep_target(), 0x400);
    }

    #[test]
    fn sweep_shift_zero_returns_current_period() {
        let mut c = PulseChannel::new(false);
        c.write_register(2, 0x00);
        c.write_register(3, 0x04);
        c.write_register(1, 0b1000_0000); // enabled, shift 0
        assert_eq!(c.sweep_target(), 0x400);
    }

    #[test]
    fn sweep_applies_after_divider_period() {
        let mut c = PulseChannel::new(false);
        c.set_enabled(true);
        c.write_register(2, 0x00);
        c.write_register(3, 0x01); // period = 0x100
                                   // sweep: enabled, period 2 (divider counts 3 half-frames), shift 1, add.
        c.write_register(1, 0b1010_0001);
        // First half-frame: divider is 0 (fresh) → apply + reload to 2.
        c.clock_half_frame();
        let after_first = c.timer_period();
        assert_eq!(after_first, 0x100 + (0x100 >> 1)); // 0x180
                                                       // Now divider = 2. Two half-frames decrement without applying.
        c.clock_half_frame(); // divider 2 → 1
        assert_eq!(c.timer_period(), after_first);
        c.clock_half_frame(); // divider 1 → 0
        assert_eq!(c.timer_period(), after_first);
        // Third half-frame: divider 0 → apply + reload.
        c.clock_half_frame();
        assert_eq!(c.timer_period(), after_first + (after_first >> 1)); // 0x240
    }

    #[test]
    fn sweep_mutes_when_target_exceeds_range() {
        let mut c = PulseChannel::new(false);
        c.set_enabled(true);
        // period near top of range, shift 1, add → target > 0x7FF.
        c.write_register(2, 0xFF);
        c.write_register(3, 0x07); // period = 0x7FF
        c.write_register(0, 0b1011_1111); // duty 2, constant vol 15
        c.write_register(1, 0b1000_0001); // enabled, shift 1, add
        assert!(c.is_muted());
        assert_eq!(c.sample(), 0);
    }

    #[test]
    fn sweep_mutes_when_period_below_8() {
        let mut c = PulseChannel::new(false);
        c.set_enabled(true);
        c.write_register(2, 0x04);
        c.write_register(3, 0x00); // period = 4 (< 8)
        c.write_register(0, 0b1011_1111);
        assert!(c.is_muted());
        assert_eq!(c.sample(), 0);
    }

    #[test]
    fn sweep_negate_target_stays_in_range() {
        // Negate subtracts `period >> shift` (plus 1 for pulse 2), which is
        // always <= period, so the target never exceeds 0x7FF — the only
        // mute path for negate is `period < 8`. Verify a negate sweep with
        // an in-range period is not muted and computes the right target.
        let mut c = PulseChannel::new(false);
        c.set_enabled(true);
        c.write_register(2, 0x00);
        c.write_register(3, 0x01); // period = 0x100
        c.write_register(1, 0b1000_1100); // enabled, negate, shift 4
        assert_eq!(c.sweep_target(), 0x100 - (0x100 >> 4)); // 0x100 - 0x10
        assert!(!c.is_muted());
    }

    #[test]
    fn sweep_add_overflow_mutes_via_target() {
        // Add sweep with a near-max period + shift pushes the target past
        // 0x7FF, muting the channel even though the period itself is in
        // range. This is the only target-overflow mute path.
        let mut c = PulseChannel::new(false);
        c.set_enabled(true);
        c.write_register(2, 0xFF);
        c.write_register(3, 0x07); // period = 0x7FF
        c.write_register(1, 0b1000_0001); // enabled, shift 1, add
        assert!(c.sweep_target() > MAX_PERIOD);
        assert!(c.is_muted());
    }

    // ---- Sample output --------------------------------------------------

    #[test]
    fn sample_zero_when_disabled() {
        let mut c = PulseChannel::new(false);
        c.write_register(0, 0b1011_1111); // duty 2, const vol 15
        c.write_register(2, 0x00);
        c.write_register(3, 0x04); // length loaded but enabled=false → suppressed
        assert_eq!(c.sample(), 0);
    }

    #[test]
    fn sample_zero_when_length_zero() {
        let mut c = PulseChannel::new(true);
        c.set_enabled(true);
        c.write_register(0, 0b1101_1111); // duty 3, const vol 15
        c.write_register(2, 0x10);
        c.write_register(3, 0x00); // period = 16, length = 10
                                   // Burn down length counter to 0 (length 10 → 10 half-frames).
        for _ in 0..10 {
            c.clock_half_frame();
        }
        assert_eq!(c.length_counter(), 0);
        assert_eq!(c.sample(), 0);
    }

    #[test]
    fn sample_follows_duty_pattern() {
        // duty 2 (50%), const vol 15, period 8 (>= 8 so not muted).
        let mut c = pulse_audible(false, 8, 15);
        let pattern = [0u8, 15, 15, 15, 15, 0, 0, 0];
        for &expected in pattern.iter() {
            assert_eq!(c.sample(), expected);
            // Advance the sequencer one step: tick until the sequence
            // position changes (handles the post-$4003 timer=0 case and
            // the period-countdown thereafter).
            let prev = c.sequence();
            while c.sequence() == prev {
                c.tick();
            }
        }
    }

    // ---- Apu top-level --------------------------------------------------

    #[test]
    fn apu_status_reflects_length_counters() {
        let mut apu = Apu::new();
        apu.write_status(0x03); // enable both
        apu.pulse1_mut().write_register(3, 0 << 3); // length = 10
        apu.pulse2_mut().write_register(3, 1 << 3); // length = 254
        let s = apu.read_status();
        assert_eq!(s & 0x01, 0x01);
        assert_eq!(s & 0x02, 0x02);
    }

    #[test]
    fn apu_status_clears_when_channel_disabled() {
        let mut apu = Apu::new();
        apu.write_status(0x01);
        apu.pulse1_mut().write_register(3, 0 << 3);
        assert_eq!(apu.read_status() & 0x01, 0x01);
        apu.write_status(0x00);
        assert_eq!(apu.read_status() & 0x01, 0x00);
    }

    #[test]
    fn apu_step_ticks_timers_at_half_cpu_rate() {
        let mut apu = Apu::new();
        apu.write_status(0x01);
        // period 0 → sequence advances every APU cycle (every 2 CPU cycles).
        apu.pulse1_mut().write_register(2, 0x00);
        apu.pulse1_mut().write_register(3, 0x00);
        let s0 = apu.pulse1().sequence();
        apu.step(2); // one APU cycle
        assert_eq!(apu.pulse1().sequence(), (s0 + 1) & 7);
        apu.step(1); // not enough for a full APU cycle
        assert_eq!(apu.pulse1().sequence(), (s0 + 1) & 7);
        apu.step(1); // now completes the second APU cycle
        assert_eq!(apu.pulse1().sequence(), (s0 + 2) & 7);
    }

    #[test]
    fn apu_mix_sums_both_channels_clamped() {
        let mut apu = Apu::new();
        apu.write_status(0x03);
        // Both channels: duty 3 (seq 0 bit = 1), const vol 15, period 16.
        {
            let p = apu.pulse1_mut();
            p.write_register(0, 0b1101_1111);
            p.write_register(2, 0x10);
            p.write_register(3, 0x00);
        }
        {
            let p = apu.pulse2_mut();
            p.write_register(0, 0b1101_1111);
            p.write_register(2, 0x10);
            p.write_register(3, 0x00);
        }
        // Each channel outputs 15 at sequence 0; mix clamps 15+15 to 15.
        assert_eq!(apu.mix(), 15);
    }
}
