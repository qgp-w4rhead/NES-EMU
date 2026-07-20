//! YM2413 OPLL — FM synthesis chip used by the VRC7 mapper (iNES 85).
//!
//! Compact, deterministic 2-operator FM synthesiser with 6 melodic
//! channels (the VRC7 exposes 6 of the YM2413's 9 voices). Not a
//! cycle-exact YM2413 clone — the goal is correct register semantics and a
//! reasonable approximation of the FM timbre so VRC7 games (e.g. *Lagrange
//! Point*) are audible and recognisable.
//!
//! Register map (addressed via the `$9010`/`$9030` latch pair):
//!
//! | Reg       | Function                                           |
//! |-----------|----------------------------------------------------|
//! | `$10-$17` | User instrument (8 bytes)                          |
//! | `$20-$25` | Channel 0-5 F-Number low 8 bits                    |
//! | `$30-$35` | Channel 0-5 volume (low 4) + instrument (high 4)   |
//! | `$40-$45` | Channel 0-5 F-Number high (bit 0) + Block (1-3) +  |
//! |           | Key-on (bit 4) + Sustain (bit 5)                   |
//!
//! See: https://www.nesdev.org/wiki/VRC7_audio

/// Number of melodic channels (VRC7 exposes 6).
pub const CHANNELS: usize = 6;
const SINE_LEN: usize = 256;
/// Phase accumulator fractional bits. The top 10 bits of the
/// `(2^PHASE_BITS)`-wide accumulator index the 1024-entry sine.
const PHASE_BITS: u32 = 20;

/// Build a 256-entry quarter-sine table (linear sine * 4095).
fn sine_table() -> [u16; SINE_LEN / 4 + 1] {
    let mut t = [0u16; SINE_LEN / 4 + 1];
    for (i, slot) in t.iter_mut().enumerate() {
        let s = ((i as f64) / (SINE_LEN as f64 / 4.0) * std::f64::consts::FRAC_PI_2).sin();
        *slot = (s * 4095.0).round() as u16;
    }
    t
}

/// One FM operator (modulator or carrier).
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default)]
struct Operator {
    phase: u32,
    level: i32,
    env_phase: EnvPhase,
    env_amp: i32,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default, PartialEq, Eq)]
enum EnvPhase {
    #[default]
    Off,
    Attack,
    Decay,
    Sustain,
    Release,
}

/// Per-channel register state (decoded from OPLL registers).
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default)]
struct ChanReg {
    fnum: u16,
    block: u8,
    key_on: bool,
    volume: u8,
    instrument: u8,
}

/// Compact OPLL patch (one instrument). All fields are 0..=15 unless noted.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Default)]
struct Patch {
    mult: u8,
    tl: u8,
    fb: u8,
    ar: u8,
    dr: u8,
    sl: u8,
    rr: u8,
    kl: u8,
    am: u8,
    wf: u8,
}

impl Patch {
    /// Decode a patch from the 8-byte user-instrument register layout.
    fn from_regs(b: &[u8; 8]) -> Self {
        Self {
            mult: b[0] & 0x0F,
            tl: ((b[0] >> 4) & 0x0F) | ((b[2] & 0x03) << 4),
            fb: b[1] & 0x07,
            ar: (b[2] >> 2) & 0x0F,
            dr: b[3] & 0x0F,
            sl: (b[3] >> 4) & 0x0F,
            rr: b[4] & 0x0F,
            kl: (b[4] >> 4) & 0x03,
            am: b[5] & 0x07,
            wf: b[7] & 0x03,
        }
    }
}

/// Default VRC7-style patch set (16 instruments). Hand-tuned to produce
/// distinct timbres; not the exact VRC7 ROM (undocumented, varies between
/// chip revisions) but covers the standard instrument families.
fn default_patches() -> [Patch; 16] {
    #[allow(clippy::too_many_arguments)]
    const fn p(
        mult: u8,
        tl: u8,
        fb: u8,
        ar: u8,
        dr: u8,
        sl: u8,
        rr: u8,
        kl: u8,
        am: u8,
        wf: u8,
    ) -> Patch {
        Patch {
            mult,
            tl,
            fb,
            ar,
            dr,
            sl,
            rr,
            kl,
            am,
            wf,
        }
    }
    [
        p(1, 24, 0, 15, 7, 3, 10, 0, 0, 0),
        p(1, 16, 2, 15, 6, 5, 8, 1, 0, 0),
        p(1, 8, 5, 15, 8, 5, 8, 0, 0, 0),
        p(1, 12, 0, 15, 7, 5, 8, 1, 0, 0),
        p(1, 8, 0, 15, 5, 3, 8, 1, 0, 0),
        p(1, 16, 3, 15, 7, 5, 8, 1, 0, 0),
        p(2, 16, 3, 15, 8, 5, 8, 1, 0, 0),
        p(1, 24, 0, 15, 7, 3, 10, 0, 0, 0),
        p(1, 12, 1, 15, 5, 4, 8, 1, 0, 0),
        p(1, 8, 4, 15, 8, 5, 8, 0, 0, 0),
        p(1, 16, 0, 15, 5, 3, 8, 1, 0, 0),
        p(1, 8, 3, 15, 7, 5, 8, 1, 0, 0),
        p(1, 16, 0, 15, 5, 3, 8, 1, 0, 0),
        p(1, 16, 0, 15, 5, 3, 8, 1, 0, 0),
        p(1, 16, 0, 15, 5, 3, 8, 1, 0, 0),
        Patch::default(),
    ]
}

/// The YM2413 OPLL chip state.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Opll {
    #[serde(with = "crate::save_state::array_ser")]
    regs: [u8; 0x80],
    addr_latch: u8,
    chan: [ChanReg; CHANNELS],
    ops: [[Operator; 2]; CHANNELS],
    patches: [Patch; 16],
    user_inst_buf: [u8; 8],
    #[serde(skip, default = "sine_table")]
    sine: [u16; SINE_LEN / 4 + 1],
}

impl Default for Opll {
    fn default() -> Self {
        Self {
            regs: [0; 0x80],
            addr_latch: 0,
            chan: [ChanReg::default(); CHANNELS],
            ops: [[Operator::default(); 2]; CHANNELS],
            patches: default_patches(),
            user_inst_buf: [0; 8],
            sine: sine_table(),
        }
    }
}

impl Opll {
    pub fn new() -> Self {
        Self::default()
    }

    /// Write to the address latch (`$9010` on VRC7). The VRC7 uses 7-bit
    /// register addressing (0x00-0x7F) to access the F-Number-high /
    /// block / key-on registers at 0x40-0x45.
    pub fn write_addr(&mut self, addr: u8) {
        self.addr_latch = addr & 0x7F;
    }

    /// Write to the data port (`$9030` on VRC7).
    pub fn write_data(&mut self, value: u8) {
        let addr = self.addr_latch as usize;
        if addr >= 0x80 {
            return;
        }
        self.regs[addr] = value;
        self.apply_reg(addr, value);
    }

    fn apply_reg(&mut self, addr: usize, value: u8) {
        match addr {
            // User instrument definition bytes 0..7.
            0x10..=0x17 => {
                let i = addr - 0x10;
                self.user_inst_buf[i] = value;
                if i == 7 {
                    self.patches[15] = Patch::from_regs(&self.user_inst_buf);
                }
            }
            // Channel volume + instrument select.
            0x30..=0x35 => {
                let ch = addr - 0x30;
                self.chan[ch].volume = value & 0x0F;
                self.chan[ch].instrument = (value >> 4) & 0x0F;
            }
            // Channel Fnum-low (8 bits).
            0x20..=0x25 => {
                let ch = addr - 0x20;
                self.chan[ch].fnum = (self.chan[ch].fnum & 0x0100) | value as u16;
            }
            // Channel Fnum-high (bit 0) + Block (bits 1-3) + Key-on (bit 4).
            0x40..=0x45 => {
                let ch = addr - 0x40;
                let was_on = self.chan[ch].key_on;
                self.chan[ch].fnum = (self.chan[ch].fnum & 0x00FF) | ((value as u16 & 0x01) << 8);
                self.chan[ch].block = (value >> 1) & 0x07;
                self.chan[ch].key_on = (value & 0x10) != 0;
                let now_on = self.chan[ch].key_on;
                if now_on && !was_on {
                    self.key_on(ch);
                } else if !now_on && was_on {
                    self.key_off(ch);
                }
            }
            _ => {}
        }
    }

    fn key_on(&mut self, ch: usize) {
        for op in &mut self.ops[ch] {
            op.env_phase = EnvPhase::Attack;
            op.env_amp = 0;
            op.phase = 0;
        }
    }
    fn key_off(&mut self, ch: usize) {
        for op in &mut self.ops[ch] {
            op.env_phase = EnvPhase::Release;
        }
    }

    /// Phase increment per APU cycle for an operator. The increment is
    /// `fnum * mult * 2^block` in the 20-bit phase accumulator's units;
    /// the top 10 bits index the sine table, so a full sine cycle takes
    /// `2^10 / inc` APU cycles → output frequency `apu_clock * inc / 1024`.
    fn phase_inc(ch: &ChanReg, mult: u8) -> u32 {
        let m = if mult == 0 { 1 } else { mult as u32 };
        ((ch.fnum as u32) * m) << ch.block
    }

    fn env_step(rate: u8) -> i32 {
        if rate == 0 {
            0
        } else {
            (rate as i32) << 3
        }
    }

    /// Advance one operator's envelope by one APU cycle. Both modulator
    /// and carrier compute their output `level` from the envelope
    /// amplitude; the carrier additionally applies the channel volume.
    fn clock_env(op: &mut Operator, patch: &Patch, is_carrier: bool, ch_vol: u8) {
        let sl = (patch.sl as i32) << 7;
        const MAX: i32 = 4095;
        match op.env_phase {
            EnvPhase::Off => op.env_amp = MAX,
            EnvPhase::Attack => {
                op.env_amp -= Self::env_step(patch.ar) * 4;
                if op.env_amp <= 0 {
                    op.env_amp = 0;
                    op.env_phase = EnvPhase::Decay;
                }
            }
            EnvPhase::Decay => {
                op.env_amp += Self::env_step(patch.dr);
                if op.env_amp >= sl {
                    op.env_amp = sl;
                    op.env_phase = EnvPhase::Sustain;
                }
            }
            EnvPhase::Sustain => {}
            EnvPhase::Release => {
                op.env_amp += Self::env_step(patch.rr) * 2;
                if op.env_amp >= MAX {
                    op.env_amp = MAX;
                    op.env_phase = EnvPhase::Off;
                }
            }
        }
        // Base output level from envelope amplitude (0 = loud, MAX = silent).
        // `level` is the 12-bit sine scale factor the sample path multiplies
        // by, so loud → high level.
        let mut level = (MAX - op.env_amp.clamp(0, MAX)) >> 2;
        if is_carrier {
            // Carrier: apply channel volume + patch total-level attenuation.
            let tl = (ch_vol as i32 + ((patch.tl as i32) >> 2)).min(63);
            level = (level - tl * 16).max(0);
        } else {
            // Modulator: apply patch total-level attenuation only (no ch vol).
            level = (level - (patch.tl as i32) * 16).max(0);
        }
        op.level = level;
    }

    /// Sine value for a 10-bit phase index (0..=1023), in [-4095, 4095].
    /// The top 10 bits of the 20-bit phase accumulator index the sine.
    /// The 1024-entry sine is reconstructed by quarter-wave symmetry from
    /// the 65-entry quarter-sine table: each quadrant spans 256 entries,
    /// so the 8-bit intra-quadrant index is scaled down to 0..=64.
    fn sin(&self, phase: u32) -> i32 {
        let idx = ((phase >> 10) & 0x3FF) as usize;
        let (quad, intra) = match idx >> 8 {
            0 => (0u8, idx),
            1 => (1, 0x3FF - idx),
            2 => (2, idx - 0x200),
            _ => (3, 0x3FF - (idx - 0x200)),
        };
        // Map 0..=255 down to 0..=64 (the table has 65 entries).
        let i = (intra >> 2).min(SINE_LEN / 4);
        let v = self.sine[i] as i32;
        if quad == 1 || quad == 3 {
            -v
        } else {
            v
        }
    }

    /// Advance the chip state by `apu_cycles` APU cycles (≈ CPU/2).
    pub fn clock(&mut self, apu_cycles: u32) {
        for _ in 0..apu_cycles {
            for ch in 0..CHANNELS {
                let patch = self.patches[self.chan[ch].instrument as usize];
                let ch_vol = self.chan[ch].volume;
                let pinc_mod = Self::phase_inc(&self.chan[ch], patch.mult);
                let pinc_car = Self::phase_inc(&self.chan[ch], 1);
                self.ops[ch][0].phase = self.ops[ch][0].phase.wrapping_add(pinc_mod);
                Self::clock_env(&mut self.ops[ch][0], &patch, false, ch_vol);
                self.ops[ch][1].phase = self.ops[ch][1].phase.wrapping_add(pinc_car);
                Self::clock_env(&mut self.ops[ch][1], &patch, true, ch_vol);
            }
        }
    }

    /// Mixed output sample in `[-1.0, 1.0]`.
    pub fn sample(&self) -> f32 {
        let mut sum: i32 = 0;
        for ch in 0..CHANNELS {
            if !self.chan[ch].key_on && self.ops[ch][1].env_phase == EnvPhase::Off {
                continue;
            }
            let patch = self.patches[self.chan[ch].instrument as usize];
            let fb = if patch.fb != 0 {
                self.ops[ch][0].level >> (7 - patch.fb)
            } else {
                0
            };
            let mod_out = self.sin(self.ops[ch][0].phase.wrapping_add(fb as u32 * 64))
                * self.ops[ch][0].level
                / 4095;
            // Phase modulation: mod_out can be negative (sine quadrants 1/3).
            // Multiply in i32 first (no overflow: |mod_out| ≤ 1023, * 16 = 16368),
            // then reinterpret as u32 for wrapping phase addition.
            let car_phase = self.ops[ch][1].phase.wrapping_add((mod_out * 16) as u32);
            let car = self.sin(car_phase) * self.ops[ch][1].level / 4095;
            sum += car;
        }
        (sum as f32 / (CHANNELS as f32 * 4095.0)).clamp(-1.0, 1.0)
    }
}
