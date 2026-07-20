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

/// 32-step triangle waveform sequence. The channel outputs one value per
/// timer period; the sequence ramps 15→0 then 0→15, producing a triangle
/// wave at 1/32 the timer frequency. There is no envelope — the output
/// level is taken directly from this table.
///
/// See: https://www.nesdev.org/wiki/APU_Triangle#Sequencer
const TRIANGLE_SEQUENCE: [u8; 32] = [
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
    13, 14, 15,
];

/// Noise channel timer periods indexed by the 4-bit period select
/// (`$400E` bits 0-3). Each entry is the timer reload value in APU cycles.
///
/// See: https://www.nesdev.org/wiki/APU_Noise#Tableref
const NOISE_PERIOD_TABLE: [u16; 16] = [
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
];

/// One of the two NES pulse wave channels.
///
/// `pulse2` selects the pulse-2 sweep negate variant, which subtracts one
/// extra from the target period (a hardware quirk; see APU_Sweep).
#[derive(serde::Serialize, serde::Deserialize, Clone)]
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

/// The NES triangle wave channel.
///
/// The triangle channel produces a 32-step waveform (15→0→15) at full
/// volume — there is no envelope. Two counters gate the output:
///
/// - **Linear counter** — 7-bit, clocked at the quarter-frame rate. The
///   halt flag (`$4008` bit 7) doubles as the length-counter halt flag.
/// - **Length counter** — clocked at the half-frame rate, same table as
///   the pulse channels.
///
/// The 11-bit timer ticks at the APU clock rate (CPU/2); each time it
/// reloads the 32-step sequence advances one position.
///
/// Register map (see <https://www.nesdev.org/wiki/APU_Triangle>):
///
/// | Register  | Bits          | Function                                     |
/// |-----------|---------------|----------------------------------------------|
/// | `$4008`   | `Clll llll`   | Halt/loop + linear counter reload value      |
/// | `$4009`   | `---- ----`   | Unused                                       |
/// | `$400A`   | `TTTT TTTT`   | Timer low 8 bits                             |
/// | `$400B`   | `LLLL LTTT`   | Length load + timer high 3 bits              |
///
/// See: https://www.nesdev.org/wiki/APU_Triangle
/// See: https://www.nesdev.org/wiki/APU_Length_Counter
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct TriangleChannel {
    // ---- $4008: linear counter control ---------------------------------
    /// Halt flag — halts both the linear counter and the length counter
    /// (bit 7 of `$4008`).
    halt: bool,
    /// Linear counter reload value (bits 0-6 of `$4008`, 0..=127).
    linear_reload: u8,
    /// Current linear counter value; 0 silences the channel.
    linear_counter: u8,
    /// Start flag — set on `$400B` write; reloads the linear counter on
    /// the next quarter-frame clock.
    linear_start: bool,

    // ---- $400A + $400B: timer ------------------------------------------
    /// 11-bit timer reload value (period).
    timer_period: u16,
    /// Running timer counter.
    timer: u16,
    /// Current position in the 32-step triangle sequence (0..=31).
    sequence: u8,

    // ---- Length counter -------------------------------------------------
    /// Current length counter value; 0 silences the channel.
    length_counter: u8,
    /// Channel enable from `$4015` bit 2. When cleared the length counter
    /// is forced to 0.
    enabled: bool,
}

impl TriangleChannel {
    /// Create a new, fully-reset triangle channel.
    fn new() -> Self {
        Self {
            halt: false,
            linear_reload: 0,
            linear_counter: 0,
            linear_start: false,
            timer_period: 0,
            timer: 0,
            sequence: 0,
            length_counter: 0,
            enabled: false,
        }
    }

    /// Write to one of the triangle channel registers.
    ///
    /// `reg` is `0..=3` (the low 2 bits of the address after subtracting
    /// the channel base `$4008`).
    ///
    /// See: https://www.nesdev.org/wiki/APU_Triangle#Registers
    pub fn write_register(&mut self, reg: u8, value: u8) {
        match reg {
            0 => {
                // $4008: Clll llll — halt + linear counter reload value.
                self.halt = (value & 0x80) != 0;
                self.linear_reload = value & 0x7F;
            }
            1 => {
                // $4009: unused.
            }
            2 => {
                // $400A: timer low 8 bits.
                self.timer_period = (self.timer_period & 0xFF00) | value as u16;
            }
            3 => {
                // $400B: LLLL LTTT — length load + timer high 3 bits.
                let high = (value & 0x07) as u16;
                self.timer_period = (self.timer_period & 0x00FF) | (high << 8);
                // The running timer's high 3 bits are set immediately; the
                // low 8 bits are not affected (matches the pulse $4003
                // behavior).
                self.timer = (self.timer & 0x00FF) | (high << 8);

                // Length counter load (only if the channel is enabled via
                // $4015; otherwise the load is suppressed).
                if self.enabled {
                    let idx = (value >> 3) as usize;
                    self.length_counter = LENGTH_TABLE[idx];
                }

                // Side effects: linear counter restart + sequencer reset.
                self.linear_start = true;
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

    /// Current linear counter value (0 means the channel is silenced).
    pub fn linear_counter(&self) -> u8 {
        self.linear_counter
    }

    /// Current timer period (11-bit reload value).
    pub fn timer_period(&self) -> u16 {
        self.timer_period
    }

    /// Current running timer counter value.
    pub fn timer(&self) -> u16 {
        self.timer
    }

    /// Current sequence position (0..=31).
    pub fn sequence(&self) -> u8 {
        self.sequence
    }

    /// Advance the channel by one APU cycle (half a CPU cycle). The timer
    /// counts down; on reaching zero it reloads to the period and the
    /// 32-step waveform sequencer advances one step.
    ///
    /// See: https://www.nesdev.org/wiki/APU_Triangle#Timer
    pub fn tick(&mut self) {
        if self.timer == 0 {
            self.timer = self.timer_period;
            self.sequence = (self.sequence + 1) & 0x1F;
        } else {
            self.timer -= 1;
        }
    }

    /// Clock the linear counter (quarter-frame signal from the frame
    /// counter).
    ///
    /// Per NESdev:
    /// 1. If the start flag is set, reload the counter and clear start.
    /// 2. Else if the counter is non-zero, decrement it.
    /// 3. If the halt flag is set, set the start flag (so step 1 runs next
    ///    quarter-frame — the counter never decrements while halt is set).
    ///
    /// See: https://www.nesdev.org/wiki/APU_Triangle#Linear_counter
    fn clock_linear(&mut self) {
        if self.linear_start {
            self.linear_start = false;
            self.linear_counter = self.linear_reload;
        } else if self.linear_counter > 0 {
            self.linear_counter -= 1;
        }
        if self.halt {
            self.linear_start = true;
        }
    }

    /// Clock the length counter (half-frame signal). The counter is held
    /// (not decremented) when the halt flag is set (shared with the linear
    /// counter halt).
    ///
    /// See: https://www.nesdev.org/wiki/APU_Length_Counter#Clocking
    fn clock_length(&mut self) {
        if !self.halt && self.length_counter > 0 {
            self.length_counter -= 1;
        }
    }

    /// Quarter-frame clock: clocks the linear counter.
    pub fn clock_quarter_frame(&mut self) {
        self.clock_linear();
    }

    /// Half-frame clock: clocks the length counter (the linear counter is
    /// also clocked at the quarter-frame rate, which the frame counter
    /// calls separately).
    pub fn clock_half_frame(&mut self) {
        self.clock_length();
    }

    /// Current output sample (0..=15). Returns 0 when the channel is
    /// disabled or silenced by the length counter or the linear counter
    /// reaching zero. The triangle channel has no envelope — the output
    /// level is taken directly from the 32-step waveform sequence.
    ///
    /// See: https://www.nesdev.org/wiki/APU_Triangle#Output
    pub fn sample(&self) -> u8 {
        if !self.enabled || self.length_counter == 0 || self.linear_counter == 0 {
            return 0;
        }
        TRIANGLE_SEQUENCE[self.sequence as usize]
    }
}

/// The NES noise channel.
///
/// The noise channel produces pseudo-random audio via a 15-bit linear-
/// feedback shift register (LFSR). Two feedback taps are selectable via
/// the mode bit (`$400E` bit 7): bits 0+1 (default, periodic) or bits 0+6
/// (metallic). The channel shares the envelope + length-counter design
/// of the pulse channels but has no sweep and a fixed lookup-table period.
///
/// Register map (see <https://www.nesdev.org/wiki/APU_Noise>):
///
/// | Register  | Bits          | Function                                     |
/// |-----------|---------------|----------------------------------------------|
/// | `$400C`   | `--LC VVVV`   | Halt/loop + constant-volume + volume         |
/// | `$400D`   | `---- ----`   | Unused                                       |
/// | `$400E`   | `L--- PPPP`   | Mode + period select (index into table)      |
/// | `$400F`   | `LLLL L---`   | Length load + envelope restart               |
///
/// See: https://www.nesdev.org/wiki/APU_Noise
/// See: https://www.nesdev.org/wiki/APU_Envelope
/// See: https://www.nesdev.org/wiki/APU_Length_Counter
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct NoiseChannel {
    // ---- $400C: envelope / length control ------------------------------
    /// Length-counter halt / envelope loop flag (bit 5 of `$400C`).
    halt: bool,
    /// Constant-volume mode (bit 4 of `$400C`).
    constant_volume: bool,
    /// Volume / envelope divider period (bits 3-0 of `$400C`), 0..=15.
    volume: u8,

    // ---- $400E: mode + period ------------------------------------------
    /// LFSR mode (bit 7 of `$400E`): `false` = XOR bits 0,1; `true` = XOR
    /// bits 0,6.
    mode: bool,
    /// Period index (bits 0-3 of `$400E`) into `NOISE_PERIOD_TABLE`.
    period_index: u8,

    // ---- Timer ----------------------------------------------------------
    /// Current timer reload value (from `NOISE_PERIOD_TABLE[period_index]`).
    timer_period: u16,
    /// Running timer counter.
    timer: u16,

    // ---- LFSR -----------------------------------------------------------
    /// 15-bit linear-feedback shift register. Initialized to 1 on reset
    /// per the NESdev wiki.
    lfsr: u16,

    // ---- Length counter -------------------------------------------------
    /// Current length counter value; 0 silences the channel.
    length_counter: u8,
    /// Channel enable from `$4015` bit 3. When cleared the length counter
    /// is forced to 0.
    enabled: bool,

    // ---- Envelope -------------------------------------------------------
    /// Envelope divider count.
    envelope_divider: u8,
    /// Current envelope decay level (0..=15) — used as the output volume
    /// when not in constant-volume mode.
    envelope_decay: u8,
    /// Start flag — set on `$400F` write; restarts the envelope on the
    /// next quarter-frame clock.
    envelope_start: bool,
}

impl NoiseChannel {
    /// Create a new, fully-reset noise channel. The LFSR is initialized
    /// to 1 per the NESdev wiki and the timer period defaults to the
    /// first entry of the noise period table.
    fn new() -> Self {
        Self {
            halt: false,
            constant_volume: false,
            volume: 0,
            mode: false,
            period_index: 0,
            timer_period: NOISE_PERIOD_TABLE[0],
            timer: 0,
            lfsr: 1,
            length_counter: 0,
            enabled: false,
            envelope_divider: 0,
            envelope_decay: 0,
            envelope_start: false,
        }
    }

    /// Write to one of the noise channel registers.
    ///
    /// `reg` is `0..=3` (the low 2 bits of the address after subtracting
    /// the channel base `$400C`).
    ///
    /// See: https://www.nesdev.org/wiki/APU_Noise#Registers
    pub fn write_register(&mut self, reg: u8, value: u8) {
        match reg {
            0 => {
                // $400C: --LC VVVV
                self.halt = (value & 0x20) != 0;
                self.constant_volume = (value & 0x10) != 0;
                self.volume = value & 0x0F;
            }
            1 => {
                // $400D: unused.
            }
            2 => {
                // $400E: L--- PPPP — mode + period select.
                self.mode = (value & 0x80) != 0;
                self.period_index = value & 0x0F;
                self.timer_period = NOISE_PERIOD_TABLE[self.period_index as usize];
                // The running timer is not reset on a period change.
            }
            3 => {
                // $400F: LLLL L--- — length load + envelope restart.
                if self.enabled {
                    let idx = (value >> 3) as usize;
                    self.length_counter = LENGTH_TABLE[idx];
                }
                self.envelope_start = true;
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

    /// Current envelope decay level (the volume used in envelope mode).
    pub fn envelope_decay(&self) -> u8 {
        self.envelope_decay
    }

    /// Current timer period (from the noise period lookup table).
    pub fn timer_period(&self) -> u16 {
        self.timer_period
    }

    /// Current running timer counter value.
    pub fn timer(&self) -> u16 {
        self.timer
    }

    /// Current LFSR value (15-bit shift register).
    pub fn lfsr(&self) -> u16 {
        self.lfsr
    }

    /// Current LFSR mode (`false` = bits 0+1, `true` = bits 0+6).
    pub fn mode(&self) -> bool {
        self.mode
    }

    /// Current period index (0..=15) into `NOISE_PERIOD_TABLE`.
    pub fn period_index(&self) -> u8 {
        self.period_index
    }

    /// Advance the channel by one APU cycle (half a CPU cycle). The timer
    /// counts down; on reaching zero it reloads to the period and the LFSR
    /// is clocked (one shift).
    ///
    /// LFSR feedback: `bit 0 XOR (mode ? bit 6 : bit 1)`. The register is
    /// shifted right by 1 and bit 14 is set to the feedback bit.
    ///
    /// See: https://www.nesdev.org/wiki/APU_Noise#Shift_register
    pub fn tick(&mut self) {
        if self.timer == 0 {
            self.timer = self.timer_period;
            // Clock the LFSR.
            let bit0 = self.lfsr & 0x0001;
            let tap = if self.mode { 6 } else { 1 };
            let tap_bit = (self.lfsr >> tap) & 0x0001;
            let feedback = bit0 ^ tap_bit;
            self.lfsr >>= 1;
            if feedback != 0 {
                self.lfsr |= 0x4000; // bit 14
            }
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

    /// Quarter-frame clock: clocks the envelope.
    pub fn clock_quarter_frame(&mut self) {
        self.clock_envelope();
    }

    /// Half-frame clock: clocks the length counter.
    pub fn clock_half_frame(&mut self) {
        self.clock_length();
    }

    /// Current output sample (0..=15). Returns 0 when the channel is
    /// silenced (length counter zero) or when LFSR bit 0 is set (the
    /// hardware gates the output on the inverted bit 0). Otherwise the
    /// output is the envelope decay level (or constant volume).
    ///
    /// See: https://www.nesdev.org/wiki/APU_Noise#Output
    pub fn sample(&self) -> u8 {
        if !self.enabled || self.length_counter == 0 {
            return 0;
        }
        if self.lfsr & 1 != 0 {
            return 0;
        }
        if self.constant_volume {
            self.volume
        } else {
            self.envelope_decay
        }
    }
}

/// DMC rate table (NTSC) — timer period in APU cycles (CPU clock / 2).
/// The NESdev wiki lists these in CPU cycles: [428, 380, 340, 320, 298,
/// 276, 254, 226, 214, 190, 170, 160, 142, 126, 108, 84]. Dividing by 2
/// gives the APU-cycle period since the DMC timer is clocked at the APU
/// rate alongside the other channel timers.
///
/// See: https://www.nesdev.org/wiki/APU_DMC#Rate_table
const DMC_RATE_TABLE: [u16; 16] = [
    214, 190, 170, 160, 149, 138, 127, 113, 107, 95, 85, 80, 71, 63, 54, 42,
];

/// The NES DMC (Delta Modulation Channel) — plays DMA-fetched 1-bit delta
/// samples from CPU memory at a configurable rate. The output is a 7-bit
/// DAC counter (0..=127) that increments by 2 on a `1` bit and decrements
/// by 2 on a `0` bit (saturating at both ends).
///
/// The channel has no length counter; instead it plays a fixed-length
/// sample (1..=4081 bytes) starting at a configurable address
/// (`$C000 + ($4012 << 6)`). When the sample completes, the channel
/// optionally raises an IRQ (if `$4010` bit 7 is set) or loops.
///
/// Register map (see <https://www.nesdev.org/wiki/APU_DMC>):
///
/// | Register  | Bits          | Function                                     |
/// |-----------|---------------|----------------------------------------------|
/// | `$4010`   | `IL-- RRRR`   | IRQ enable, loop, rate index                 |
/// | `$4011`   | `-DDD DDDD`   | Direct DAC load (bits 0-6; bit 7 ignored)    |
/// | `$4012`   | `AAAA AAAA`   | Sample address (base = value << 6 + $C000)   |
/// | `$4013`   | `LLLL LLLL`   | Sample length (value << 4 + 1 bytes)         |
///
/// See: https://www.nesdev.org/wiki/APU_DMC
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct DmcChannel {
    // ---- $4010: rate + control ------------------------------------------
    /// IRQ enable (bit 7 of `$4010`). When set, the DMC raises an IRQ when
    /// a sample completes without the loop flag set.
    irq_enable: bool,
    /// Loop flag (bit 6 of `$4010`). When set, the sample restarts from
    /// the beginning when it completes.
    loop_flag: bool,
    /// Rate index (bits 0-3 of `$4010`) into `DMC_RATE_TABLE`.
    rate_index: u8,

    // ---- Timer ----------------------------------------------------------
    /// Timer reload value (from `DMC_RATE_TABLE[rate_index]`, in APU cycles).
    timer_period: u16,
    /// Running timer counter.
    timer: u16,

    // ---- Output unit ----------------------------------------------------
    /// 7-bit output counter (0..=127). This is the DAC value directly.
    output_counter: u8,
    /// Current byte being shifted out (filled from memory).
    sample_buffer: u8,
    /// Number of bits remaining in `sample_buffer` (0 = buffer empty).
    buffer_bits: u8,

    // ---- Sample pointer -------------------------------------------------
    /// Sample address base (reloaded on restart/loop). Set by `$4012`:
    /// `base = (value << 6) + $C000`.
    sample_addr_base: u16,
    /// Current fetch address (increments per byte, wraps `$FFFF` → `$8000`).
    sample_address: u16,
    /// Total sample length in bytes (reloaded on restart/loop). Set by
    /// `$4013`: `length = (value << 4) + 1`.
    sample_length: u16,
    /// Bytes remaining to fetch in the current sample.
    bytes_remaining: u16,

    // ---- Channel enable + IRQ -------------------------------------------
    /// Channel enable from `$4015` bit 4.
    enabled: bool,
    /// DMC IRQ flag (read at `$4015` bit 7, cleared by reading `$4015`).
    irq_flag: bool,
}

impl DmcChannel {
    /// Create a new, fully-reset DMC channel. The output counter starts
    /// at 0 and the timer period defaults to the first rate-table entry.
    fn new() -> Self {
        Self {
            irq_enable: false,
            loop_flag: false,
            rate_index: 0,
            timer_period: DMC_RATE_TABLE[0],
            timer: 0,
            output_counter: 0,
            sample_buffer: 0,
            buffer_bits: 0,
            sample_addr_base: 0xC000,
            sample_address: 0xC000,
            sample_length: 1,
            bytes_remaining: 0,
            enabled: false,
            irq_flag: false,
        }
    }

    /// Write to one of the DMC channel registers.
    ///
    /// `reg` is `0..=3` (the low 2 bits of the address after subtracting
    /// the channel base `$4010`).
    ///
    /// See: https://www.nesdev.org/wiki/APU_DMC#Registers
    pub fn write_register(&mut self, reg: u8, value: u8) {
        match reg {
            0 => {
                // $4010: IL-- RRRR — IRQ enable, loop, rate index.
                self.irq_enable = (value & 0x80) != 0;
                self.loop_flag = (value & 0x40) != 0;
                self.rate_index = value & 0x0F;
                self.timer_period = DMC_RATE_TABLE[self.rate_index as usize];
            }
            1 => {
                // $4011: -DDD DDDD — direct DAC load. Bits 0-6 set the
                // output counter directly; bit 7 is ignored.
                self.output_counter = value & 0x7F;
            }
            2 => {
                // $4012: sample address base = (value << 6) + $C000.
                self.sample_addr_base = ((value as u16) << 6) | 0xC000;
            }
            3 => {
                // $4013: sample length = (value << 4) + 1.
                self.sample_length = ((value as u16) << 4) | 1;
            }
            _ => {}
        }
    }

    /// Set the channel enable flag from `$4015` bit 4. When enabled and
    /// the sample is not already playing (`bytes_remaining == 0`), the
    /// sample is restarted from the base address. When disabled, bytes
    /// remaining is forced to 0 but the output counter keeps its value.
    ///
    /// See: https://www.nesdev.org/wiki/APU_Status
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        if enabled {
            // Only restart if the sample has finished (or was never
            // started). Per NESdev: "If DMC bytes remaining is 0, restart
            // DMC sample."
            if self.bytes_remaining == 0 {
                self.sample_address = self.sample_addr_base;
                self.bytes_remaining = self.sample_length;
                self.buffer_bits = 0; // empty the sample buffer
            }
        } else {
            // Stop playback; the output counter retains its value.
            self.bytes_remaining = 0;
        }
    }

    /// Whether the channel is currently enabled via `$4015`.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Whether the DMC has bytes remaining to fetch (read at `$4015` bit 4).
    pub fn bytes_remaining(&self) -> u16 {
        self.bytes_remaining
    }

    /// Current output counter value (0..=127) — the DAC output.
    pub fn output_counter(&self) -> u8 {
        self.output_counter
    }

    /// Current timer period (from the rate lookup table).
    pub fn timer_period(&self) -> u16 {
        self.timer_period
    }

    /// Current rate index (0..=15).
    pub fn rate_index(&self) -> u8 {
        self.rate_index
    }

    /// Current sample address (the next byte to fetch).
    pub fn sample_address(&self) -> u16 {
        self.sample_address
    }

    /// Current sample address base (set by `$4012`).
    pub fn sample_addr_base(&self) -> u16 {
        self.sample_addr_base
    }

    /// Current sample length (set by `$4013`).
    pub fn sample_length(&self) -> u16 {
        self.sample_length
    }

    /// Whether the loop flag is set.
    pub fn loop_flag(&self) -> bool {
        self.loop_flag
    }

    /// Whether the IRQ enable flag is set.
    pub fn irq_enable(&self) -> bool {
        self.irq_enable
    }

    /// Whether the DMC IRQ flag is set (read at `$4015` bit 7).
    pub fn irq_flag(&self) -> bool {
        self.irq_flag
    }

    /// Clear the DMC IRQ flag (done by reading `$4015`).
    pub fn clear_irq(&mut self) {
        self.irq_flag = false;
    }

    /// Advance the channel by one APU cycle (half a CPU cycle). The timer
    /// counts down; on reaching zero it reloads to the period and the
    /// output unit clocks one bit. `read` is a closure that fetches a
    /// byte from CPU memory at the given address (used for DMA sample
    /// fetches).
    ///
    /// See: https://www.nesdev.org/wiki/APU_DMC#Output_unit
    pub fn tick(&mut self, read: &mut impl FnMut(u16) -> u8) {
        if self.timer == 0 {
            self.timer = self.timer_period;
            self.clock_output_unit(read);
        } else {
            self.timer -= 1;
        }
    }

    /// Clock the output unit: optionally fetch a byte from memory, then
    /// shift one bit out and update the output counter.
    ///
    /// Per NESdev, each timer tick does the following in order:
    /// 1. If the sample buffer is empty and bytes remain, fetch a byte.
    /// 2. If the sample buffer is non-empty, shift one bit out and update
    ///    the output counter.
    ///
    /// See: https://www.nesdev.org/wiki/APU_DMC#Output_unit
    fn clock_output_unit(&mut self, read: &mut impl FnMut(u16) -> u8) {
        // Step 1: refill the sample buffer if empty and bytes remain.
        if self.buffer_bits == 0 && self.bytes_remaining > 0 {
            self.sample_buffer = read(self.sample_address);
            self.buffer_bits = 8;
            // Advance the sample address, wrapping $FFFF → $8000.
            self.sample_address = self.sample_address.wrapping_add(1);
            if self.sample_address == 0 {
                self.sample_address = 0x8000;
            }
            self.bytes_remaining -= 1;
            // If the sample is now exhausted, handle loop / IRQ.
            if self.bytes_remaining == 0 {
                if self.loop_flag {
                    self.sample_address = self.sample_addr_base;
                    self.bytes_remaining = self.sample_length;
                } else if self.irq_enable {
                    self.irq_flag = true;
                }
            }
        }

        // Step 2: output one bit from the sample buffer.
        if self.buffer_bits > 0 {
            let bit = self.sample_buffer & 1;
            if bit == 0 {
                self.output_counter = self.output_counter.saturating_sub(2);
            } else {
                self.output_counter = (self.output_counter.saturating_add(2)).min(127);
            }
            self.sample_buffer >>= 1;
            self.buffer_bits -= 1;
        }
    }

    /// Current output sample (0..=127). The DMC always outputs its DAC
    /// counter value regardless of the enabled flag (the counter retains
    /// its value after the channel is disabled).
    ///
    /// See: https://www.nesdev.org/wiki/APU_DMC#Output
    pub fn sample(&self) -> u8 {
        self.output_counter
    }
}

/// The APU — owns the two pulse channels, the triangle channel, the noise
/// channel, the DMC channel, and the frame counter.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Apu {
    pulse1: PulseChannel,
    pulse2: PulseChannel,
    triangle: TriangleChannel,
    noise: NoiseChannel,
    dmc: DmcChannel,

    /// Half-cycle accumulator: the APU runs at CPU clock / 2, so one APU
    /// cycle is two CPU cycles. This accumulates fractional APU cycles
    /// across `step` calls.
    cycle_accumulator: u32,

    // ---- Per-channel volume / mute (M31) --------------------------------
    /// Per-channel volume scalars in `[0.0, 1.0]`, indexed as
    /// `[pulse1, pulse2, triangle, noise, dmc]`. Applied to each
    /// channel's raw sample before the non-linear mix. `0.0` is
    /// equivalent to mute (but distinct from `channel_muted` so a
    /// "reset" can restore the configured volume). Defaults to `1.0`
    /// (full volume). Set at startup from `config.toml`
    /// `[audio_channels]` and adjustable at runtime via hotkeys.
    channel_volumes: [f32; 5],
    /// Per-channel mute flags. A muted channel contributes 0 to the
    /// mix regardless of its volume scalar. Toggled at runtime via
    /// `Alt+1`..`Alt+5` + `Alt+M` (M31).
    channel_muted: [bool; 5],
    /// Currently-selected channel index (0..4) for volume adjustment
    /// hotkeys (`Alt+Up`/`Alt+Down`). Advanced by `Alt+1`..`Alt+5`.
    selected_channel: u8,

    // ---- Low-pass filter (M31) ------------------------------------------
    /// Previous output sample held by the one-pole low-pass filter.
    /// The LPF smooths harsh square-wave harmonics above ~12 kHz.
    /// Updated each `output()` call; serialized so save states preserve
    /// filter state.
    lpf_prev: f32,

    // ---- Frame counter ($4017) -----------------------------------------
    /// `true` for 5-step mode (bit 7 of `$4017`), `false` for 4-step mode.
    frame_mode_5step: bool,
    /// IRQ inhibit flag (bit 6 of `$4017`). When set, the frame counter
    /// does not raise IRQs in 4-step mode.
    frame_irq_inhibit: bool,
    /// Frame counter cycle position (in CPU cycles). Resets at the end of
    /// each frame-counter period (29830 for 4-step, 37282 for 5-step).
    frame_cycle: u32,
    /// Frame counter IRQ flag (read at `$4015` bit 6, cleared by reading
    /// `$4015`). Set at the 4th step of 4-step mode if IRQ is not inhibited.
    frame_irq: bool,
    /// Pending reset delay in CPU cycles. On `$4017` write the frame
    /// counter resets after ~3-4 CPU cycles; this counts down that delay.
    /// A value of 0 means no pending reset.
    frame_reset_delay: u32,
}

impl Apu {
    /// Build a reset APU with all channels silenced.
    pub fn new() -> Self {
        Self {
            pulse1: PulseChannel::new(false),
            pulse2: PulseChannel::new(true),
            triangle: TriangleChannel::new(),
            noise: NoiseChannel::new(),
            dmc: DmcChannel::new(),
            cycle_accumulator: 0,
            channel_volumes: [1.0; 5],
            channel_muted: [false; 5],
            selected_channel: 0,
            // Initialize to the silence level (-1.0) so a cold-boot APU
            // (all channels off) produces a steady -1.0 immediately,
            // with no startup transient ramping 0 → -1.0 (audible click).
            lpf_prev: -1.0,
            frame_mode_5step: false,
            frame_irq_inhibit: false,
            frame_cycle: 0,
            frame_irq: false,
            frame_reset_delay: 0,
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

    /// Borrow the triangle channel.
    pub fn triangle(&self) -> &TriangleChannel {
        &self.triangle
    }

    /// Mutably borrow the triangle channel.
    pub fn triangle_mut(&mut self) -> &mut TriangleChannel {
        &mut self.triangle
    }

    /// Borrow the noise channel.
    pub fn noise(&self) -> &NoiseChannel {
        &self.noise
    }

    /// Mutably borrow the noise channel.
    pub fn noise_mut(&mut self) -> &mut NoiseChannel {
        &mut self.noise
    }

    /// Borrow the DMC channel.
    pub fn dmc(&self) -> &DmcChannel {
        &self.dmc
    }

    /// Mutably borrow the DMC channel.
    pub fn dmc_mut(&mut self) -> &mut DmcChannel {
        &mut self.dmc
    }

    /// Write the status register `$4015`. Bits 0-3 enable/disable the
    /// pulse 1, pulse 2, triangle, and noise channels (clearing a bit
    /// forces that channel's length counter to zero). Bit 4 enables the
    /// DMC channel (restarting the sample from its base address).
    ///
    /// See: https://www.nesdev.org/wiki/APU_Status
    pub fn write_status(&mut self, value: u8) {
        self.pulse1.set_enabled(value & 0x01 != 0);
        self.pulse2.set_enabled(value & 0x02 != 0);
        self.triangle.set_enabled(value & 0x04 != 0);
        self.noise.set_enabled(value & 0x08 != 0);
        self.dmc.set_enabled(value & 0x10 != 0);
    }

    /// Read the status register `$4015`. Bits 0-4 reflect whether each
    /// channel's length counter / bytes-remaining is non-zero. Bit 6 is
    /// the frame counter IRQ flag; bit 7 is the DMC IRQ flag. Reading
    /// `$4015` clears both IRQ flags (matching real hardware).
    ///
    /// See: https://www.nesdev.org/wiki/APU_Status
    pub fn read_status(&mut self) -> u8 {
        let mut v = 0u8;
        if self.pulse1.length_counter > 0 {
            v |= 0x01;
        }
        if self.pulse2.length_counter > 0 {
            v |= 0x02;
        }
        if self.triangle.length_counter > 0 {
            v |= 0x04;
        }
        if self.noise.length_counter > 0 {
            v |= 0x08;
        }
        if self.dmc.bytes_remaining > 0 {
            v |= 0x10;
        }
        if self.frame_irq {
            v |= 0x40;
        }
        if self.dmc.irq_flag {
            v |= 0x80;
        }
        // Reading $4015 clears both IRQ flags.
        self.frame_irq = false;
        self.dmc.irq_flag = false;
        v
    }

    /// Write the frame counter control register `$4017`.
    ///
    /// - Bit 7: mode (`0` = 4-step, `1` = 5-step).
    /// - Bit 6: IRQ inhibit (`1` = suppress frame counter IRQ).
    ///
    /// On write, the frame counter is reset after a short delay (~3-4 CPU
    /// cycles on real hardware; modeled here as a 4-cycle delay). In 5-step
    /// mode, an immediate quarter+half-frame clock is also performed.
    ///
    /// See: https://www.nesdev.org/wiki/APU_Frame_Counter
    pub fn write_frame_counter(&mut self, value: u8) {
        let new_mode_5step = (value & 0x80) != 0;
        self.frame_irq_inhibit = (value & 0x40) != 0;

        // If the IRQ inhibit flag is set, clear any pending frame IRQ.
        if self.frame_irq_inhibit {
            self.frame_irq = false;
        }

        // 5-step mode: immediately clock quarter + half frame.
        if new_mode_5step {
            self.clock_quarter_frame();
            self.clock_half_frame();
        }

        // Schedule a frame counter reset after ~4 CPU cycles.
        self.frame_reset_delay = 4;
        // Update the mode immediately (the reset will zero the cycle counter).
        self.frame_mode_5step = new_mode_5step;
    }

    /// Whether the APU has a pending IRQ (frame counter or DMC). The
    /// emulator main loop polls this after each CPU step and raises
    /// `Cpu::irq_pending` when it returns `true`.
    pub fn irq_pending(&self) -> bool {
        self.frame_irq || self.dmc.irq_flag
    }

    /// Advance the APU by `cpu_cycles` CPU cycles. All channel timers
    /// (including the DMC) tick once per APU cycle (every 2 CPU cycles).
    /// The frame counter advances at the CPU clock rate and triggers
    /// quarter/half-frame clocks at the appropriate cycle counts. `read`
    /// is a closure that fetches a byte from CPU memory at the given
    /// address (used by the DMC for DMA sample fetches).
    ///
    /// See: https://www.nesdev.org/wiki/APU_Frame_Counter
    pub fn step(&mut self, cpu_cycles: u32, mut read: impl FnMut(u16) -> u8) {
        // ---- Frame counter (CPU clock rate) ----
        self.step_frame_counter(cpu_cycles);

        // ---- Channel timers (APU clock rate = CPU / 2) ----
        self.cycle_accumulator = self.cycle_accumulator.saturating_add(cpu_cycles);
        while self.cycle_accumulator >= 2 {
            self.cycle_accumulator -= 2;
            self.pulse1.tick();
            self.pulse2.tick();
            self.triangle.tick();
            self.noise.tick();
            self.dmc.tick(&mut read);
        }
    }

    /// Advance the frame counter by `cpu_cycles` CPU cycles, firing
    /// quarter/half-frame clocks and IRQ at the appropriate thresholds.
    ///
    /// 4-step mode thresholds (NTSC, CPU cycles):
    /// - 7457: quarter-frame
    /// - 14913: quarter + half-frame
    /// - 22371: quarter-frame
    /// - 29828: quarter + half-frame + IRQ (if not inhibited)
    /// - 29830: counter resets
    ///
    /// 5-step mode thresholds:
    /// - 7457: quarter-frame
    /// - 14913: quarter + half-frame
    /// - 22371: quarter-frame
    /// - 37281: quarter + half-frame (no IRQ)
    /// - 37282: counter resets
    ///
    /// See: https://www.nesdev.org/wiki/APU_Frame_Counter
    fn step_frame_counter(&mut self, cpu_cycles: u32) {
        // Handle the pending reset delay from a $4017 write.
        if self.frame_reset_delay > 0 {
            let advance = cpu_cycles.min(self.frame_reset_delay);
            self.frame_reset_delay -= advance;
            if self.frame_reset_delay == 0 {
                // The reset takes effect: zero the cycle counter.
                self.frame_cycle = 0;
            }
            // During the reset delay, the frame counter does not advance.
            // The remaining cycles (if any) are applied after the reset.
            let remaining = cpu_cycles - advance;
            if remaining == 0 {
                return;
            }
            self.frame_cycle = self.frame_cycle.saturating_add(remaining);
        } else {
            self.frame_cycle = self.frame_cycle.saturating_add(cpu_cycles);
        }

        // Threshold crossings. We check each threshold against the
        // pre/post cycle count so that a large batch (e.g. DMA stall)
        // doesn't skip a threshold.
        let prev = self.frame_cycle.saturating_sub(cpu_cycles);
        let thresholds: [(u32, bool, bool); 4] = if self.frame_mode_5step {
            [
                (7457, true, false),
                (14913, true, true),
                (22371, true, false),
                (37281, true, true),
            ]
        } else {
            [
                (7457, true, false),
                (14913, true, true),
                (22371, true, false),
                (29828, true, true),
            ]
        };

        for &(threshold, quarter, half) in thresholds.iter() {
            if prev < threshold && self.frame_cycle >= threshold {
                if quarter {
                    self.clock_quarter_frame();
                }
                if half {
                    self.clock_half_frame();
                }
                // IRQ only in 4-step mode, at the 4th step, if not inhibited.
                if !self.frame_mode_5step && threshold == 29828 && !self.frame_irq_inhibit {
                    self.frame_irq = true;
                }
            }
        }

        // Reset the counter at the end of the period.
        let reset_at: u32 = if self.frame_mode_5step { 37282 } else { 29830 };
        if self.frame_cycle >= reset_at {
            self.frame_cycle -= reset_at;
        }
    }

    /// Quarter-frame signal — clocks the pulse/noise envelopes and the
    /// triangle linear counter. Called by the frame counter (M16) at
    /// ≈240 Hz NTSC.
    pub fn clock_quarter_frame(&mut self) {
        self.pulse1.clock_quarter_frame();
        self.pulse2.clock_quarter_frame();
        self.triangle.clock_quarter_frame();
        self.noise.clock_quarter_frame();
    }

    /// Half-frame signal — clocks the length counters (all four channels)
    /// and the pulse sweep units. The triangle linear counter is clocked
    /// separately at the quarter-frame rate by `clock_quarter_frame`.
    /// Called by the frame counter (M16) at ≈120 Hz NTSC.
    pub fn clock_half_frame(&mut self) {
        self.pulse1.clock_half_frame();
        self.pulse2.clock_half_frame();
        self.triangle.clock_half_frame();
        self.noise.clock_half_frame();
    }

    /// Mix the current channel samples into a single 0..=15 value (linear
    /// sum, clamped). This is the simple linear mixer used for debug
    /// inspection; the actual audio path uses the hardware-accurate
    /// non-linear mixer in [`Apu::output`] (M31). The DMC is not included
    /// here; see [`Apu::output`] for the full mix including DMC.
    pub fn mix(&self) -> u8 {
        let s = self.pulse1.sample() as u16
            + self.pulse2.sample() as u16
            + self.triangle.sample() as u16
            + self.noise.sample() as u16;
        if s > 15 {
            15
        } else {
            s as u8
        }
    }

    /// Full audio output sample as an `f32` in `[-1.0, 1.0]`, including
    /// all five channels (M31: non-linear hardware-accurate mixing).
    ///
    /// The NES APU mixer is non-linear: the two pulse channels share one
    /// mixing junction and the triangle/noise/DMC share another. The
    /// formulas (from the NESdev wiki "APU Mixer" page) are:
    ///
    /// - `pulse_out = 95.52 / (8128.0 / (p1 + p2) + 100.0)` (0 if p1+p2 = 0)
    /// - `tnd_out = 163.67 / (1.0 / (tri/8227 + noise/12241 + dmc/22638) + 100.0)`
    ///   (0 if the inner sum is 0)
    ///
    /// Each channel's raw sample is first scaled by its per-channel
    /// volume scalar and zeroed if muted (M31). The mixed value
    /// (`pulse_out + tnd_out`, range ~0..1.017) is then mapped to
    /// `[-1.0, 1.0]` via `out = mixed * 2.0 - 1.0` (so silence → -1.0,
    /// matching the pre-M31 centered mapping for backward compatibility
    /// with the DC-offset acceptance documented in M16) and run through
    /// a one-pole low-pass filter (~12 kHz cutoff) to smooth harsh
    /// square-wave harmonics.
    ///
    /// See: https://www.nesdev.org/wiki/APU_Mixer
    pub fn output(&mut self) -> f32 {
        // Per-channel raw samples, scaled by volume and muted.
        let p1 = self.scaled_sample(0, self.pulse1.sample());
        let p2 = self.scaled_sample(1, self.pulse2.sample());
        let tri = self.scaled_sample(2, self.triangle.sample());
        let noise = self.scaled_sample(3, self.noise.sample());
        let dmc = self.scaled_sample(4, self.dmc.sample());

        // Non-linear pulse junction.
        // https://www.nesdev.org/wiki/APU_Mixer
        let pulse_sum = p1 + p2;
        let pulse_out = if pulse_sum > 0.0 {
            95.52 / (8128.0 / pulse_sum + 100.0)
        } else {
            0.0
        };

        // Non-linear triangle/noise/DMC junction.
        let tnd_inner = tri / 8227.0 + noise / 12241.0 + dmc / 22638.0;
        let tnd_out = if tnd_inner > 0.0 {
            163.67 / (1.0 / tnd_inner + 100.0)
        } else {
            0.0
        };

        // Map [0, ~1.017] → [-1.0, ~1.034] and clamp to [-1.0, 1.0].
        let mixed = (pulse_out + tnd_out) * 2.0 - 1.0;
        let clamped = mixed.clamp(-1.0, 1.0);

        // One-pole low-pass filter (cutoff ≈ 12 kHz at 44.1 kHz).
        // y[n] = a * x[n] + (1 - a) * y[n-1], where
        // a = 1 - exp(-2*pi*fc/fs) ≈ 0.8192 for fc=12kHz, fs=44.1kHz.
        const LPF_ALPHA: f32 = 0.8192;
        let filtered = LPF_ALPHA * clamped + (1.0 - LPF_ALPHA) * self.lpf_prev;
        self.lpf_prev = filtered;
        filtered
    }

    /// Return a channel's raw sample scaled by its per-channel volume
    /// scalar and zeroed if muted. `idx` is 0=pulse1, 1=pulse2,
    /// 2=triangle, 3=noise, 4=dmc. Out-of-range indices return 0.0.
    fn scaled_sample(&self, idx: usize, raw: u8) -> f32 {
        if idx >= self.channel_volumes.len() || self.channel_muted[idx] {
            return 0.0;
        }
        raw as f32 * self.channel_volumes[idx]
    }

    // ---- Per-channel volume / mute accessors (M31) ---------------------

    /// Per-channel volume scalars in `[0.0, 1.0]`, indexed as
    /// `[pulse1, pulse2, triangle, noise, dmc]`.
    pub fn channel_volumes(&self) -> [f32; 5] {
        self.channel_volumes
    }

    /// Set the per-channel volume scalar for channel `idx` (0..4).
    /// Out-of-range indices are ignored. Values are clamped to
    /// `[0.0, 1.0]`.
    pub fn set_channel_volume(&mut self, idx: usize, vol: f32) {
        if idx < self.channel_volumes.len() {
            self.channel_volumes[idx] = vol.clamp(0.0, 1.0);
        }
    }

    /// Get the volume scalar for channel `idx` (0..4). Out-of-range
    /// indices return 0.0.
    pub fn channel_volume(&self, idx: usize) -> f32 {
        self.channel_volumes.get(idx).copied().unwrap_or(0.0)
    }

    /// Per-channel mute flags, indexed as `[pulse1, pulse2, triangle,
    /// noise, dmc]`.
    pub fn channel_muted(&self) -> [bool; 5] {
        self.channel_muted
    }

    /// Whether channel `idx` (0..4) is muted. Out-of-range → true.
    pub fn channel_muted_at(&self, idx: usize) -> bool {
        self.channel_muted.get(idx).copied().unwrap_or(true)
    }

    /// Toggle the mute flag for channel `idx` (0..4). Out-of-range
    /// indices are ignored. Returns the new muted state (or `None` if
    /// the index was invalid).
    pub fn toggle_channel_mute(&mut self, idx: usize) -> Option<bool> {
        let m = self.channel_muted.get_mut(idx)?;
        *m = !*m;
        Some(*m)
    }

    /// Explicitly set the mute flag for channel `idx` (0..4).
    pub fn set_channel_muted(&mut self, idx: usize, muted: bool) {
        if let Some(m) = self.channel_muted.get_mut(idx) {
            *m = muted;
        }
    }

    /// Currently-selected channel index (0..4) for volume hotkeys.
    pub fn selected_channel(&self) -> u8 {
        self.selected_channel
    }

    /// Set the selected channel index. Clamped to 0..4.
    pub fn set_selected_channel(&mut self, idx: u8) {
        self.selected_channel = idx.min(4);
    }

    /// Reset all channels to unmuted with volume `1.0` (full). Used by
    /// the `Alt+0` "reset all" hotkey (M31).
    pub fn reset_channel_mix(&mut self) {
        self.channel_volumes = [1.0; 5];
        self.channel_muted = [false; 5];
    }

    /// Apply a set of per-channel volumes from config (M31). Each
    /// entry is clamped to `[0.0, 1.0]`. Missing entries (if the
    /// caller passes a shorter slice) leave the existing value.
    pub fn apply_channel_volumes(&mut self, vols: &[f32]) {
        for (i, &v) in vols.iter().enumerate() {
            if i < self.channel_volumes.len() {
                self.channel_volumes[i] = v.clamp(0.0, 1.0);
            }
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
        apu.step(2, |_| 0); // one APU cycle
        assert_eq!(apu.pulse1().sequence(), (s0 + 1) & 7);
        apu.step(1, |_| 0); // not enough for a full APU cycle
        assert_eq!(apu.pulse1().sequence(), (s0 + 1) & 7);
        apu.step(1, |_| 0); // now completes the second APU cycle
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

    // ============================================================
    // Triangle channel (M15)
    // ============================================================

    /// Build a triangle channel with a loaded length + linear counter,
    /// enabled via `$4015`. The linear counter is started by the `$400B`
    /// write's start flag, then clocked once via `clock_quarter_frame` to
    /// reload it to the configured reload value.
    fn triangle_audible(period: u16, linear_reload: u8) -> TriangleChannel {
        let mut c = TriangleChannel::new();
        c.set_enabled(true);
        // $4008: halt clear, linear reload value.
        c.write_register(0, linear_reload & 0x7F);
        // $400A: timer low.
        c.write_register(2, (period & 0xFF) as u8);
        // $400B: length index 0 (length = 10) + timer high.
        c.write_register(3, ((period >> 8) as u8) & 0x07);
        // Start the linear counter (the $400B write set the start flag;
        // clock it once to reload).
        c.clock_quarter_frame();
        c
    }

    #[test]
    fn triangle_sequence_is_correct() {
        // 32-step ramp: 15→0 (with a doubled 0) then 0→15.
        assert_eq!(TRIANGLE_SEQUENCE[0], 15);
        assert_eq!(TRIANGLE_SEQUENCE[15], 0);
        assert_eq!(TRIANGLE_SEQUENCE[16], 0);
        assert_eq!(TRIANGLE_SEQUENCE[31], 15);
        assert_eq!(TRIANGLE_SEQUENCE.len(), 32);
    }

    #[test]
    fn triangle_timer_reload_advances_sequence_one_step() {
        // period = 0 → every tick reloads + advances.
        let mut c = triangle_audible(0, 127);
        let s0 = c.sequence();
        c.tick();
        assert_eq!(c.sequence(), (s0 + 1) & 0x1F);
        c.tick();
        assert_eq!(c.sequence(), (s0 + 2) & 0x1F);
    }

    #[test]
    fn triangle_timer_counts_down_then_reloads() {
        // period = 3. After $400B the running timer's high 3 bits are set
        // but the low 8 bits are 0, so timer starts at 0. The first tick
        // reloads to 3 and advances the sequence.
        let mut c = triangle_audible(3, 127);
        assert_eq!(c.timer(), 0);
        c.tick(); // 0 → reload(3) + advance
        assert_eq!(c.timer(), 3);
        c.tick(); // 3 → 2
        c.tick(); // 2 → 1
        c.tick(); // 1 → 0
        c.tick(); // 0 → reload(3) + advance
        assert_eq!(c.timer(), 3);
    }

    #[test]
    fn triangle_400b_write_preserves_low_8_bits_of_running_timer() {
        let mut c = triangle_audible(0x1FF, 127);
        // After $400B: timer = (0 & 0xFF) | (1 << 8) = 0x100.
        assert_eq!(c.timer(), 0x100);
        c.tick(); // 0x100 → 0x0FF
        assert_eq!(c.timer(), 0x0FF);
        // Write $400B with high = 0x3. Low 8 bits (0xFF) preserved.
        c.write_register(3, 0x03);
        assert_eq!(c.timer(), 0x3FF);
        assert_eq!(c.timer_period(), 0x3FF);
    }

    #[test]
    fn triangle_length_load_gated_by_4015_enable() {
        let mut c = TriangleChannel::new();
        // Not enabled — length load suppressed.
        c.write_register(3, 0 << 3);
        assert_eq!(c.length_counter(), 0);
        // Enable + load.
        c.set_enabled(true);
        c.write_register(3, 0 << 3);
        assert_eq!(c.length_counter(), LENGTH_TABLE[0]);
    }

    #[test]
    fn triangle_length_decrements_on_half_frame() {
        let mut c = triangle_audible(8, 127);
        let start = c.length_counter();
        c.clock_half_frame();
        assert_eq!(c.length_counter(), start - 1);
    }

    #[test]
    fn triangle_length_halt_freezes_count() {
        let mut c = TriangleChannel::new();
        c.set_enabled(true);
        // $4008 bit 7 = halt (halts both linear and length counters).
        c.write_register(0, 0x80);
        c.write_register(3, 0 << 3); // length = 10
        let start = c.length_counter();
        c.clock_half_frame();
        assert_eq!(c.length_counter(), start);
    }

    #[test]
    fn triangle_linear_counter_reloads_on_start() {
        let mut c = TriangleChannel::new();
        c.set_enabled(true);
        c.write_register(0, 0x2A); // linear reload = 0x2A
        c.write_register(3, 0); // sets linear_start
        assert_eq!(c.linear_counter(), 0);
        c.clock_quarter_frame(); // start → reload to 0x2A
        assert_eq!(c.linear_counter(), 0x2A);
    }

    #[test]
    fn triangle_linear_counter_decrements_when_not_halted() {
        let mut c = triangle_audible(8, 5);
        assert_eq!(c.linear_counter(), 5);
        c.clock_quarter_frame();
        assert_eq!(c.linear_counter(), 4);
        c.clock_quarter_frame();
        assert_eq!(c.linear_counter(), 3);
    }

    #[test]
    fn triangle_linear_counter_halt_keeps_it_reloaded() {
        // With halt set, the linear counter is reloaded every quarter-frame
        // (never decrements).
        let mut c = TriangleChannel::new();
        c.set_enabled(true);
        c.write_register(0, 0x80 | 5); // halt + linear reload 5
        c.write_register(3, 0); // start flag
        c.clock_quarter_frame(); // start → reload to 5
        assert_eq!(c.linear_counter(), 5);
        c.clock_quarter_frame(); // halt → reload again
        assert_eq!(c.linear_counter(), 5);
        c.clock_quarter_frame();
        assert_eq!(c.linear_counter(), 5);
    }

    #[test]
    fn triangle_linear_counter_stops_at_zero() {
        let mut c = triangle_audible(8, 1);
        assert_eq!(c.linear_counter(), 1);
        c.clock_quarter_frame(); // 1 → 0
        assert_eq!(c.linear_counter(), 0);
        c.clock_quarter_frame(); // stays 0
        assert_eq!(c.linear_counter(), 0);
    }

    #[test]
    fn triangle_sample_zero_when_length_zero() {
        let mut c = triangle_audible(8, 127);
        // Burn down length to 0 (length 10 → 10 half-frames).
        for _ in 0..10 {
            c.clock_half_frame();
        }
        assert_eq!(c.length_counter(), 0);
        assert_eq!(c.sample(), 0);
    }

    #[test]
    fn triangle_sample_zero_when_linear_zero() {
        let mut c = triangle_audible(8, 1);
        // Burn down linear counter to 0 (1 quarter-frame).
        c.clock_quarter_frame();
        assert_eq!(c.linear_counter(), 0);
        assert_eq!(c.sample(), 0);
    }

    #[test]
    fn triangle_sample_follows_sequence() {
        // With length + linear loaded, the sample tracks the 32-step
        // sequence as the timer advances.
        let mut c = triangle_audible(0, 127); // period 0 → advance every tick
        for &expected in TRIANGLE_SEQUENCE.iter() {
            assert_eq!(c.sample(), expected);
            c.tick(); // advance to next sequence position
        }
        // After 32 ticks the sequence wraps back to position 0.
        assert_eq!(c.sequence(), 0);
        assert_eq!(c.sample(), TRIANGLE_SEQUENCE[0]);
    }

    #[test]
    fn triangle_disabling_via_4015_clears_length() {
        let mut c = triangle_audible(8, 127);
        assert!(c.length_counter() > 0);
        c.set_enabled(false);
        assert_eq!(c.length_counter(), 0);
        assert_eq!(c.sample(), 0);
    }

    #[test]
    fn triangle_4009_write_is_ignored() {
        let mut c = TriangleChannel::new();
        // $4009 is unused — writing it must not change any state.
        let before_period = c.timer_period();
        c.write_register(1, 0xFF);
        assert_eq!(c.timer_period(), before_period);
    }

    // ============================================================
    // Noise channel (M15)
    // ============================================================

    /// Build a noise channel with a loaded length + envelope started,
    /// enabled via `$4015`. The envelope is started by the `$400F` write
    /// and clocked once to set decay = 15.
    fn noise_audible(period_index: u8, volume: u8) -> NoiseChannel {
        let mut c = NoiseChannel::new();
        c.set_enabled(true);
        // $400C: halt clear, constant volume, volume.
        c.write_register(0, 0b0001_0000 | (volume & 0x0F));
        // $400E: mode 0, period index.
        c.write_register(2, period_index & 0x0F);
        // $400F: length index 0 (length = 10) + envelope restart.
        c.write_register(3, 0 << 3);
        c
    }

    #[test]
    fn noise_period_table_loads_correct_values() {
        let mut c = NoiseChannel::new();
        for (idx, &expected) in NOISE_PERIOD_TABLE.iter().enumerate() {
            c.write_register(2, idx as u8);
            assert_eq!(c.timer_period(), expected);
            assert_eq!(c.period_index(), idx as u8);
        }
    }

    #[test]
    fn noise_mode_bit_selects_tap() {
        let mut c = NoiseChannel::new();
        c.write_register(2, 0x00); // mode 0
        assert!(!c.mode());
        c.write_register(2, 0x80); // mode 1
        assert!(c.mode());
    }

    #[test]
    fn noise_lfsr_shifts_on_timer_reload() {
        // The LFSR only shifts when the timer reloads. With period index 0
        // → period 4, the constructor leaves timer = 0, so the first tick
        // reloads + shifts. Thereafter each shift takes period+1 ticks
        // (period ticks to count down + 1 to trigger the reload).
        let mut c = NoiseChannel::new();
        c.write_register(2, 0); // period index 0 → period 4
        c.set_enabled(true);
        let lfsr_before = c.lfsr();
        // First tick: timer 0 → reload(4) + shift.
        c.tick();
        // feedback = bit0 XOR bit1 (mode 0). LFSR=1: bit0=1, bit1=0 → fb=1.
        // Shift right → 0, set bit 14 → 0x4000.
        assert_eq!(c.lfsr(), 0x4000);
        assert_ne!(c.lfsr(), lfsr_before);
        // 5 ticks (period+1) to the next shift.
        for _ in 0..5 {
            c.tick();
        }
        // LFSR=0x4000: bit0=0, bit1=0 → fb=0. Shift right → 0x2000.
        assert_eq!(c.lfsr(), 0x2000);
    }

    #[test]
    fn noise_lfsr_mode0_xor_bits_0_and_1() {
        let mut c = NoiseChannel::new();
        c.set_enabled(true);
        c.write_register(2, 0x00); // mode 0, period index 0 → period 4
                                   // First shift (tick 1): LFSR 1 → 0x4000 (bit0=1,bit1=0 → fb=1).
        c.tick();
        assert_eq!(c.lfsr(), 0x4000);
        // 5 ticks (period+1) to the next shift: 0x4000 → 0x2000 (fb=0).
        for _ in 0..5 {
            c.tick();
        }
        assert_eq!(c.lfsr(), 0x2000);
        // Continue until the bit reaches position 1 (LFSR=0x0002). From
        // position 13 (after shift 2) that's 12 more shifts (13→12→…→1).
        // Total shifts: 2 + 12 = 14.
        for _ in 0..12 {
            for _ in 0..5 {
                c.tick();
            }
        }
        assert_eq!(c.lfsr(), 0x0002);
        // One more shift: bit0=0, bit1=1 → fb=1 → 0x4001.
        for _ in 0..5 {
            c.tick();
        }
        assert_eq!(c.lfsr(), 0x4001);
    }

    #[test]
    fn noise_lfsr_mode1_xor_bits_0_and_6() {
        let mut c = NoiseChannel::new();
        c.set_enabled(true);
        c.write_register(2, 0x80); // mode 1, period index 0 → period 4
                                   // First shift: LFSR 1 → 0x4000 (bit0=1, bit6=0 → fb=1).
        c.tick();
        assert_eq!(c.lfsr(), 0x4000);
        // Same as mode 0 until the bit reaches position 1 or 6. Drive the
        // LFSR until the bit is at position 6 (LFSR=0x0040). From position
        // 14 that's 8 shifts (14→13→…→6). 1 shift already done, so 8 more.
        for _ in 0..8 {
            for _ in 0..5 {
                c.tick();
            }
        }
        assert_eq!(c.lfsr(), 0x0040);
        // At LFSR=0x0040: bit0=0, bit6=1 → mode 1 fb = 0 XOR 1 = 1.
        // Mode 0 would give bit0=0, bit1=0 → fb=0. So the next shift
        // differs between modes. Mode 1: fb=1 → shift right (0x0020) +
        // set bit 14 → 0x4020.
        for _ in 0..5 {
            c.tick();
        }
        assert_eq!(c.lfsr(), 0x4020);
    }

    #[test]
    fn noise_lfsr_mode1_distinguishes_from_mode0() {
        // Run both modes forward enough that the feedback tap difference
        // (bit 1 vs bit 6) produces divergent LFSR states. Starting from
        // LFSR=1, the single bit walks down from position 14; the modes
        // diverge once the bit reaches position 6 (where mode 1 taps it
        // but mode 0 doesn't). That's ~8 shifts = ~41 ticks (period 4).
        let mut c0 = NoiseChannel::new();
        c0.set_enabled(true);
        c0.write_register(2, 0x00); // mode 0
        let mut c1 = NoiseChannel::new();
        c1.set_enabled(true);
        c1.write_register(2, 0x80); // mode 1
        for _ in 0..50 {
            c0.tick();
            c1.tick();
        }
        // After 50 ticks (10 shifts with period 4) the bit has walked
        // past position 6, where mode 1 produced different feedback than
        // mode 0. The LFSRs must have diverged.
        assert_ne!(c0.lfsr(), c1.lfsr());
    }

    #[test]
    fn noise_length_load_gated_by_4015_enable() {
        let mut c = NoiseChannel::new();
        c.write_register(3, 0 << 3);
        assert_eq!(c.length_counter(), 0);
        c.set_enabled(true);
        c.write_register(3, 0 << 3);
        assert_eq!(c.length_counter(), LENGTH_TABLE[0]);
    }

    #[test]
    fn noise_length_decrements_on_half_frame() {
        let mut c = noise_audible(0, 15);
        let start = c.length_counter();
        c.clock_half_frame();
        assert_eq!(c.length_counter(), start - 1);
    }

    #[test]
    fn noise_length_halt_freezes_count() {
        let mut c = NoiseChannel::new();
        c.set_enabled(true);
        // $400C bit 5 = halt.
        c.write_register(0, 0x20);
        c.write_register(3, 0 << 3);
        let start = c.length_counter();
        c.clock_half_frame();
        assert_eq!(c.length_counter(), start);
    }

    #[test]
    fn noise_envelope_restarts_on_400f_write() {
        let mut c = NoiseChannel::new();
        c.write_register(0, 0x00); // envelope mode, volume 0
        c.write_register(3, 0); // start
        c.clock_envelope(); // start → decay 15
        assert_eq!(c.envelope_decay(), 15);
        for _ in 0..3 {
            c.clock_envelope();
        }
        assert!(c.envelope_decay() < 15);
        c.write_register(3, 0); // restart
        c.clock_envelope();
        assert_eq!(c.envelope_decay(), 15);
    }

    #[test]
    fn noise_envelope_decays_one_step_per_volume_plus_one_clocks() {
        let mut c = NoiseChannel::new();
        c.write_register(0, 0x00); // volume 0 → divider period 1
        c.write_register(3, 0); // start
        c.clock_envelope(); // start → decay 15
        c.clock_envelope(); // divider 0 → reload, decay 15 → 14
        assert_eq!(c.envelope_decay(), 14);
        c.clock_envelope();
        assert_eq!(c.envelope_decay(), 13);
    }

    #[test]
    fn noise_constant_volume_bypasses_decay() {
        let mut c = noise_audible(0, 7);
        // LFSR starts at 1 (bit 0 set) → sample 0 until the LFSR shifts.
        assert_eq!(c.lfsr() & 1, 1);
        assert_eq!(c.sample(), 0);
        // Tick once: timer 0 → reload(4) + shift. LFSR 1 → 0x4000 (bit 0 = 0).
        c.tick();
        assert_eq!(c.lfsr() & 1, 0);
        assert_eq!(c.sample(), 7);
        // Clocking the envelope should not change the output volume.
        for _ in 0..20 {
            c.clock_envelope();
            assert_eq!(c.sample(), 7);
        }
    }

    #[test]
    fn noise_sample_zero_when_disabled() {
        let mut c = NoiseChannel::new();
        // Not enabled — length load suppressed, sample 0.
        c.write_register(0, 0b0001_1111); // const vol 15
        c.write_register(3, 0);
        assert_eq!(c.sample(), 0);
    }

    #[test]
    fn noise_sample_zero_when_length_zero() {
        let mut c = noise_audible(0, 15);
        // Burn down length to 0 (length 10 → 10 half-frames).
        for _ in 0..10 {
            c.clock_half_frame();
        }
        assert_eq!(c.length_counter(), 0);
        assert_eq!(c.sample(), 0);
    }

    #[test]
    fn noise_sample_zero_when_lfsr_bit0_set() {
        // LFSR starts at 1 (bit 0 set) → sample 0 regardless of volume.
        let mut c = noise_audible(0, 15);
        assert_eq!(c.lfsr() & 1, 1);
        assert_eq!(c.sample(), 0);
        // After one tick the LFSR shifts to 0x4000 (bit 0 clear) → audible.
        c.tick();
        assert_eq!(c.lfsr() & 1, 0);
        assert_eq!(c.sample(), 15);
    }

    #[test]
    fn noise_400d_write_is_ignored() {
        let mut c = NoiseChannel::new();
        let before_period = c.timer_period();
        c.write_register(1, 0xFF);
        assert_eq!(c.timer_period(), before_period);
    }

    #[test]
    fn noise_400e_write_does_not_reset_running_timer() {
        // Writing $400E changes the period but the running timer keeps
        // counting down from its current value (the new period applies on
        // the next reload).
        let mut c = NoiseChannel::new();
        c.set_enabled(true);
        c.write_register(2, 0x00); // period index 0 → period 4
        c.tick(); // timer 0 → reload(4) + shift; timer now 4
        c.tick(); // 4 → 3
        assert_eq!(c.timer(), 3);
        // Change to period index 1 (period 8). Running timer stays at 3.
        c.write_register(2, 0x01);
        assert_eq!(c.timer_period(), 8);
        assert_eq!(c.timer(), 3);
        c.tick(); // 3 → 2 (still counting down, not reloaded)
        assert_eq!(c.timer(), 2);
    }

    // ---- Apu top-level with triangle + noise ---------------------------

    #[test]
    fn apu_status_reflects_triangle_and_noise_length() {
        let mut apu = Apu::new();
        apu.write_status(0x0C); // enable triangle (bit 2) + noise (bit 3)
        apu.triangle_mut().write_register(3, 0 << 3); // length = 10
        apu.noise_mut().write_register(3, 1 << 3); // length = 254
        let s = apu.read_status();
        assert_eq!(s & 0x04, 0x04);
        assert_eq!(s & 0x08, 0x08);
    }

    #[test]
    fn apu_step_ticks_triangle_and_noise_timers() {
        let mut apu = Apu::new();
        apu.write_status(0x04); // enable triangle
        apu.triangle_mut().write_register(2, 0x00);
        apu.triangle_mut().write_register(3, 0x00); // period 0
        let s0 = apu.triangle().sequence();
        apu.step(2, |_| 0); // one APU cycle
        assert_eq!(apu.triangle().sequence(), (s0 + 1) & 0x1F);
    }

    #[test]
    fn apu_mix_includes_triangle_and_noise() {
        let mut apu = Apu::new();
        apu.write_status(0x0C); // triangle + noise
                                // Triangle: period 0, length 10, linear counter reloaded.
        apu.triangle_mut().write_register(0, 127); // linear reload 127
        apu.triangle_mut().write_register(2, 0x00);
        apu.triangle_mut().write_register(3, 0x00); // length 10, start
        apu.clock_quarter_frame(); // start linear counter → 127
                                   // Noise: const vol 5, length 10. LFSR bit 0 = 1 initially → silent.
        apu.noise_mut().write_register(0, 0b0001_0101); // const vol 5
        apu.noise_mut().write_register(3, 0x00); // length 10
                                                 // Triangle at sequence 0 outputs 15; noise is silent (LFSR bit 0 = 1).
        assert_eq!(apu.triangle().sample(), 15);
        assert_eq!(apu.noise().sample(), 0);
        assert_eq!(apu.mix(), 15);
        // Shift the noise LFSR so bit 0 = 0 → noise contributes 5.
        apu.noise_mut().tick();
        assert_eq!(apu.noise().sample(), 5);
        // Mix = 15 (triangle) + 5 (noise) = 20 → clamped to 15.
        assert_eq!(apu.mix(), 15);
    }

    // ---- DMC channel (M16) ----------------------------------------------

    #[test]
    fn dmc_rate_table_all_entries() {
        // NTSC rate table (APU cycles = CPU/2). Derived from the NESdev
        // CPU-cycle values: [428, 380, 340, 320, 298, 276, 254, 226, 214,
        // 190, 170, 160, 142, 126, 108, 84].
        assert_eq!(
            DMC_RATE_TABLE,
            [214, 190, 170, 160, 149, 138, 127, 113, 107, 95, 85, 80, 71, 63, 54, 42]
        );
    }

    #[test]
    fn dmc_write_4010_sets_irq_loop_and_rate_index() {
        let mut d = DmcChannel::new();
        // 0xFF = IRQ enable + loop + rate index 15.
        d.write_register(0, 0xFF);
        assert!(d.irq_enable());
        assert!(d.loop_flag());
        assert_eq!(d.rate_index(), 0x0F);
        assert_eq!(d.timer_period(), DMC_RATE_TABLE[15]);
    }

    #[test]
    fn dmc_write_4011_loads_output_counter_7_bits() {
        let mut d = DmcChannel::new();
        d.write_register(1, 0x7F);
        assert_eq!(d.output_counter(), 127);
        // Bit 7 is ignored.
        d.write_register(1, 0x80);
        assert_eq!(d.output_counter(), 0);
    }

    #[test]
    fn dmc_write_4012_sets_sample_address_base() {
        let mut d = DmcChannel::new();
        d.write_register(2, 0x00);
        assert_eq!(d.sample_addr_base(), 0xC000);
        d.write_register(2, 0x40);
        assert_eq!(d.sample_addr_base(), 0xD000);
        d.write_register(2, 0xFF);
        assert_eq!(d.sample_addr_base(), 0xFFC0);
    }

    #[test]
    fn dmc_write_4013_sets_sample_length() {
        let mut d = DmcChannel::new();
        d.write_register(3, 0x00);
        assert_eq!(d.sample_length(), 1);
        d.write_register(3, 0x01);
        assert_eq!(d.sample_length(), 17);
        d.write_register(3, 0xFF);
        assert_eq!(d.sample_length(), 0xFF1);
    }

    #[test]
    fn dmc_enable_restarts_sample_from_base() {
        let mut d = DmcChannel::new();
        d.write_register(2, 0x10); // base = $C400
        d.write_register(3, 0x01); // length = 17
        d.set_enabled(true);
        assert!(d.enabled());
        assert_eq!(d.sample_address(), 0xC400);
        assert_eq!(d.bytes_remaining(), 17);
    }

    #[test]
    fn dmc_disable_stops_fetch_but_keeps_output() {
        let mut d = DmcChannel::new();
        d.write_register(1, 0x40); // output = 64
        d.set_enabled(true);
        d.set_enabled(false);
        assert_eq!(d.bytes_remaining(), 0);
        assert_eq!(d.output_counter(), 64);
    }

    #[test]
    fn dmc_tick_increments_output_on_one_bit() {
        let mut d = DmcChannel::new();
        d.write_register(1, 0x00); // output = 0
                                   // Set rate index 0 (period 214 APU cycles) and prime the buffer
                                   // with 0xFF (all 1 bits → 8 increments of 2 = +16, saturating at 127).
        d.write_register(0, 0x0F); // rate index 15, period 42
        d.set_enabled(true);
        // Manually fill the buffer to avoid needing a memory read.
        // We tick the timer; on each reload the output unit clocks a bit.
        // With period 42, we need 27+1 ticks to shift one bit (timer starts
        // at 0 → first tick reloads + clocks).
        // Instead, directly test the output unit by ticking enough times
        // to shift all 8 bits of a fetched byte.
        // Use a read closure that returns 0xFF.
        for _ in 0..(42 * 8 + 1) {
            d.tick(&mut |_| 0xFF);
        }
        // 8 bits of 1 → +16, but saturating at 127.
        assert_eq!(d.output_counter(), 16);
    }

    #[test]
    fn dmc_tick_decrements_output_on_zero_bit() {
        let mut d = DmcChannel::new();
        d.write_register(1, 0x7F); // output = 127
        d.write_register(0, 0x0F); // rate index 15, period 42
        d.set_enabled(true);
        // Fetch 0x00 (all 0 bits → 8 decrements of 2 = -16).
        for _ in 0..(42 * 8 + 1) {
            d.tick(&mut |_| 0x00);
        }
        assert_eq!(d.output_counter(), 127 - 16);
    }

    #[test]
    fn dmc_output_saturates_at_127() {
        let mut d = DmcChannel::new();
        d.write_register(1, 0x7E); // output = 126
        d.write_register(0, 0x0F); // period 42
        d.set_enabled(true);
        // Fetch 0xFF → 8 bits of 1 → +16 → saturate at 127.
        for _ in 0..(42 * 8 + 1) {
            d.tick(&mut |_| 0xFF);
        }
        assert_eq!(d.output_counter(), 127);
    }

    #[test]
    fn dmc_output_saturates_at_0() {
        let mut d = DmcChannel::new();
        d.write_register(1, 0x02); // output = 2
        d.write_register(0, 0x0F); // period 42
        d.set_enabled(true);
        // Fetch 0x00 → 8 bits of 0 → -16 → saturate at 0.
        for _ in 0..(42 * 8 + 1) {
            d.tick(&mut |_| 0x00);
        }
        assert_eq!(d.output_counter(), 0);
    }

    #[test]
    fn dmc_dma_fetch_advances_address_and_decrements_bytes() {
        let mut d = DmcChannel::new();
        d.write_register(2, 0x00); // base = $C000
        d.write_register(3, 0x01); // length = 17
        d.write_register(0, 0x0F); // period 42
        d.set_enabled(true);
        // Tick enough to fetch one byte (8 bits): the first timer reload
        // triggers a fetch + shifts 8 bits.
        for _ in 0..(42 * 8 + 1) {
            d.tick(&mut |_| 0xAA);
        }
        // One byte fetched: address advanced, bytes remaining decremented.
        assert_eq!(d.sample_address(), 0xC001);
        assert_eq!(d.bytes_remaining(), 16);
    }

    #[test]
    fn dmc_address_wraps_ffff_to_8000() {
        let mut d = DmcChannel::new();
        d.write_register(2, 0xFF); // base = $FFC0
                                   // length = (0x04 << 4) + 1 = 65 bytes → enough to wrap past $FFFF.
        d.write_register(3, 0x04); // length = 65
        d.write_register(0, 0x0F); // period 42 (rate index 15)
        d.set_enabled(true);
        // The timer counts from period to 0 (period+1 ticks per clock).
        // Period 42 → 43 ticks per bit. 8 bits per byte → 344 ticks/byte.
        // 65 fetches → 65 * 344 = 22360 ticks + headroom.
        for _ in 0..23000 {
            d.tick(&mut |_| 0x00);
        }
        // After 65 fetches from $FFC0, address should have wrapped to $8001.
        assert!(d.sample_address() >= 0x8000 && d.sample_address() < 0xC000);
    }

    #[test]
    fn dmc_loop_restart_resets_address_and_bytes() {
        let mut d = DmcChannel::new();
        d.write_register(0, 0x4F); // loop + rate index 15
        d.write_register(2, 0x10); // base = $C400
        d.write_register(3, 0x00); // length = 1
        d.set_enabled(true);
        // Fetch the single byte (8 bits), then the loop should restart.
        for _ in 0..(42 * 8 + 1) {
            d.tick(&mut |_| 0x00);
        }
        // After loop restart, bytes_remaining should be back to 1.
        assert_eq!(d.bytes_remaining(), 1);
        assert_eq!(d.sample_address(), 0xC400);
    }

    #[test]
    fn dmc_irq_raised_on_completion_without_loop() {
        let mut d = DmcChannel::new();
        d.write_register(0, 0x8F); // IRQ enable + rate index 15 (no loop)
        d.write_register(3, 0x00); // length = 1
        d.set_enabled(true);
        for _ in 0..(42 * 8 + 1) {
            d.tick(&mut |_| 0x00);
        }
        assert!(d.irq_flag());
    }

    #[test]
    fn dmc_no_irq_when_loop_set() {
        let mut d = DmcChannel::new();
        d.write_register(0, 0xCF); // IRQ enable + loop + rate index 15
        d.write_register(3, 0x00); // length = 1
        d.set_enabled(true);
        for _ in 0..(42 * 8 + 1) {
            d.tick(&mut |_| 0x00);
        }
        assert!(!d.irq_flag());
    }

    #[test]
    fn dmc_clear_irq() {
        let mut d = DmcChannel::new();
        d.irq_flag = true;
        d.clear_irq();
        assert!(!d.irq_flag());
    }

    #[test]
    fn dmc_sample_returns_output_counter() {
        let mut d = DmcChannel::new();
        d.write_register(1, 0x5A);
        assert_eq!(d.sample(), 0x5A & 0x7F);
    }

    // ---- Frame counter (M16) --------------------------------------------

    #[test]
    fn apu_write_frame_counter_4_step_mode() {
        let mut apu = Apu::new();
        apu.write_frame_counter(0x00); // 4-step, IRQ not inhibited
        assert!(!apu.frame_mode_5step);
        assert!(!apu.frame_irq_inhibit);
    }

    #[test]
    fn apu_write_frame_counter_5_step_mode_clocks_immediately() {
        let mut apu = Apu::new();
        apu.write_status(0x01);
        apu.pulse1_mut().write_register(3, 0x00); // length 10
        let len0 = apu.pulse1().length_counter();
        apu.write_frame_counter(0x80); // 5-step → immediate Q+H clock
                                       // Half-frame clocks the length counter → 10 → 9.
        assert_eq!(apu.pulse1().length_counter(), len0 - 1);
    }

    #[test]
    fn apu_write_frame_counter_irq_inhibit_clears_pending_irq() {
        let mut apu = Apu::new();
        apu.frame_irq = true;
        apu.write_frame_counter(0x40); // IRQ inhibit
        assert!(!apu.frame_irq);
        assert!(apu.frame_irq_inhibit);
    }

    #[test]
    fn apu_frame_counter_4_step_raises_irq_at_29828() {
        let mut apu = Apu::new();
        apu.write_frame_counter(0x00); // 4-step, IRQ enabled
                                       // The $4017 write imposes a 4-cycle reset delay, so we need
                                       // 29828 + 4 + a few extra cycles to reach the IRQ threshold.
        apu.step(29833, |_| 0);
        assert!(apu.irq_pending());
    }

    #[test]
    fn apu_frame_counter_4_step_irq_inhibited() {
        let mut apu = Apu::new();
        apu.write_frame_counter(0x40); // 4-step, IRQ inhibited
        apu.step(29833, |_| 0);
        assert!(!apu.irq_pending());
    }

    #[test]
    fn apu_frame_counter_5_step_no_irq() {
        let mut apu = Apu::new();
        apu.write_frame_counter(0x80); // 5-step
        apu.step(37288, |_| 0);
        assert!(!apu.irq_pending());
    }

    #[test]
    fn apu_frame_counter_4_step_quarter_frame_clocks_envelope() {
        let mut apu = Apu::new();
        apu.write_status(0x01);
        // Envelope mode (bit 4 = 0), volume 0 → divider starts at 0, so
        // the second quarter-frame immediately decrements decay.
        apu.pulse1_mut().write_register(0, 0x00); // envelope mode, volume 0
        apu.pulse1_mut().write_register(3, 0x00); // length 10, sets envelope_start
                                                  // The first quarter-frame (at 7457) loads decay=15, divider=0, and
                                                  // clears envelope_start. The second quarter-frame (at 14913) sees
                                                  // divider==0 → reloads divider, decrements decay to 14.
                                                  // Account for the 4-cycle reset delay from $4017 write.
        apu.write_frame_counter(0x00);
        apu.step(14918, |_| 0); // past the second quarter-frame
        assert_eq!(apu.pulse1().envelope_decay(), 14);
    }

    #[test]
    fn apu_frame_counter_4_step_half_frame_clocks_length() {
        let mut apu = Apu::new();
        apu.write_status(0x01);
        apu.pulse1_mut().write_register(3, 0x00); // length 10
        apu.write_frame_counter(0x00);
        apu.step(14918, |_| 0); // past first half-frame at 14913 (+4 reset delay)
        assert_eq!(apu.pulse1().length_counter(), 9);
    }

    #[test]
    fn apu_read_status_clears_frame_and_dmc_irq() {
        let mut apu = Apu::new();
        apu.frame_irq = true;
        apu.dmc.irq_flag = true;
        let s = apu.read_status();
        assert_eq!(s & 0x40, 0x40); // frame IRQ
        assert_eq!(s & 0x80, 0x80); // DMC IRQ
                                    // Reading clears both.
        let s2 = apu.read_status();
        assert_eq!(s2 & 0xC0, 0x00);
    }

    #[test]
    fn apu_read_status_reflects_dmc_bytes_remaining() {
        let mut apu = Apu::new();
        apu.dmc_mut().write_register(3, 0x01); // length 17
        apu.dmc_mut().set_enabled(true);
        let s = apu.read_status();
        assert_eq!(s & 0x10, 0x10);
    }

    #[test]
    fn apu_output_includes_dmc() {
        let mut apu = Apu::new();
        // DMC output = 127, all other channels silent.
        apu.dmc_mut().write_register(1, 0x7F);
        let out = apu.output();
        // M31 non-linear mixer: pulse_out = 0 (no pulse), tnd_out with
        // only DMC = 127 → 163.67 / (1/(127/22638) + 100) ≈ 0.5883.
        // mixed = 0.5883 * 2 - 1 = 0.1766. LPF starts at -1.0 (silence)
        // so first sample = 0.8192*0.1766 + 0.1808*(-1.0) ≈ -0.0359.
        // Allow tolerance for the LPF transient from the silence init.
        assert!(
            (out - (-0.0359)).abs() < 2e-2,
            "expected ~-0.0359 (non-linear DMC + LPF from silence), got {out}"
        );
    }

    #[test]
    fn apu_output_with_both_ptn_and_dmc() {
        let mut apu = Apu::new();
        apu.write_status(0x01);
        // Duty 3 (bits 6-7 = 11) → sequence [1,0,0,0,1,1,1,1], seq[0] = 1.
        // Const vol 15 (bit 4 + bits 0-3 = 0x1F). So $4000 = 0xDF.
        apu.pulse1_mut().write_register(0, 0b1101_1111); // duty 3, const vol 15
                                                         // Period must be >= MIN_AUDIBLE_PERIOD (8) to avoid muting.
        apu.pulse1_mut().write_register(2, 0x08); // timer low = 8
        apu.pulse1_mut().write_register(3, 0x00); // length 10, timer high = 0
        apu.dmc_mut().write_register(1, 0x7F); // DMC = 127
        let out = apu.output();
        // M31 non-linear: pulse_out = 95.52/(8128/15+100) ≈ 0.1488,
        // tnd_out ≈ 0.5883 (DMC only). mixed = (0.1488+0.5883)*2-1 ≈ 0.4742.
        // LPF from silence (-1.0): 0.8192*0.4742 + 0.1808*(-1.0) ≈ 0.2081.
        assert!(
            (out - 0.2081).abs() < 2e-2,
            "expected ~0.2081 (non-linear pulse+DMC + LPF from silence), got {out}"
        );
    }

    #[test]
    fn apu_output_silence_is_negative_one() {
        let mut apu = Apu::new();
        // All channels off, DMC = 0. Non-linear mixer: pulse_out = 0,
        // tnd_out = 0 → mixed = 0*2 - 1 = -1.0. The LPF is initialized
        // to -1.0 (the silence level) so there is no boot transient —
        // the very first sample is already -1.0.
        let out = apu.output();
        assert!(
            (out - (-1.0_f32)).abs() < 1e-6,
            "expected silence to be -1.0 from first sample, got {out}"
        );
        // Stays at -1.0.
        for _ in 0..10 {
            let out = apu.output();
            assert!(
                (out - (-1.0_f32)).abs() < 1e-6,
                "silence should stay at -1.0, got {out}"
            );
        }
    }

    #[test]
    fn apu_irq_pending_reflects_both_sources() {
        let mut apu = Apu::new();
        assert!(!apu.irq_pending());
        apu.frame_irq = true;
        assert!(apu.irq_pending());
        apu.frame_irq = false;
        apu.dmc.irq_flag = true;
        assert!(apu.irq_pending());
    }
}
