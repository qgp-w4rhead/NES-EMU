//! YM2149 / AY-3-8910 PSG — 3-channel tone + noise synthesiser used by the
//! Sunsoft 5B mapper (iNES 69, the audio-equipped variant of FME-7).
//!
//! The YM2149 has three tone channels (A/B/C), a noise generator, and an
//! envelope generator. Each channel mixes its tone, noise, and envelope
//! outputs through a 4-bit digital volume control (16 levels). The chip
//! runs at the CPU clock / 2 (APU clock) on the NES.
//!
//! # Register layout
//!
//! The chip uses an address/data latch pair (FME-7 wires these at `$C000`
//! and `$E000` for the 5B variant):
//!
//! | Addr | Function                                              |
//! |------|-------------------------------------------------------|
//! | `$00`| Channel A period low (bits 0-7)                       |
//! | `$01`| Channel A period high (bits 8-11)                     |
//! | `$02`| Channel B period low                                  |
//! | `$03`| Channel B period high                                 |
//! | `$04`| Channel C period low                                  |
//! | `$05`| Channel C period high                                 |
//! | `$06`| Noise period (bits 0-4)                               |
//! | `$07`| Enable: bits 0-2 = tone A/B/C, bits 3-5 = noise A/B/C |
//! | `$08`| Channel A volume (bits 0-3) + envelope mode (bit 4)   |
//! | `$09`| Channel B volume + envelope mode                      |
//! | `$0A`| Channel C volume + envelope mode                      |
//! | `$0B`| Envelope period low                                   |
//! | `$0C`| Envelope period high                                  |
//! | `$0D`| Envelope shape (bits 0-3)                             |
//!
//! See: https://www.nesdev.org/wiki/Sunsoft_5B_audio

/// Number of tone channels.
pub const CHANNELS: usize = 3;

/// YM2149 PSG chip state.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Ym2149 {
    /// 16-byte register file.
    regs: [u8; 16],
    /// Address latch for the addr/data write pair.
    addr_latch: u8,
    /// Per-channel 12-bit tone timer.
    tone_timer: [u16; CHANNELS],
    /// Per-channel tone output toggle (0/1).
    tone_out: [u8; CHANNELS],
    /// 5-bit noise timer.
    noise_timer: u16,
    /// Noise LFSR state (17-bit).
    noise_lfsr: u32,
    /// Noise output (0/1).
    noise_out: u8,
    /// 16-bit envelope timer.
    env_timer: u16,
    /// Envelope counter position (0-31).
    env_pos: u8,
    /// Envelope hold flag.
    env_holding: bool,
}

impl Default for Ym2149 {
    fn default() -> Self {
        Self {
            regs: [0; 16],
            addr_latch: 0,
            tone_timer: [0; CHANNELS],
            tone_out: [0; CHANNELS],
            noise_timer: 0,
            noise_lfsr: 0x1_0000, // LFSR starts with bit 16 set
            noise_out: 0,
            env_timer: 0,
            env_pos: 0,
            env_holding: false,
        }
    }
}

impl Ym2149 {
    pub fn new() -> Self {
        Self::default()
    }

    /// Write to the address latch (`$C000` on the 5B).
    pub fn write_addr(&mut self, addr: u8) {
        self.addr_latch = addr & 0x0F;
    }

    /// Write to the data port (`$E000` on the 5B).
    pub fn write_data(&mut self, value: u8) {
        let a = self.addr_latch as usize;
        if a >= 16 {
            return;
        }
        self.regs[a] = value;
    }

    /// Read a register (rarely used on the 5B; provided for completeness).
    pub fn read_data(&self) -> u8 {
        let a = self.addr_latch as usize;
        if a >= 16 {
            0
        } else {
            self.regs[a]
        }
    }

    /// 12-bit tone period for a channel.
    fn tone_period(&self, ch: usize) -> u16 {
        let lo = self.regs[ch * 2] as u16;
        let hi = (self.regs[ch * 2 + 1] as u16) & 0x0F;
        (lo | (hi << 8)).max(1)
    }

    /// 5-bit noise period.
    fn noise_period(&self) -> u16 {
        ((self.regs[6] as u16) & 0x1F).max(1)
    }

    /// 16-bit envelope period.
    fn env_period(&self) -> u16 {
        let lo = self.regs[0x0B] as u16;
        let hi = self.regs[0x0C] as u16;
        (lo | (hi << 8)).max(1)
    }

    /// Channel enable mask (bit set = tone/noise DISABLED).
    fn disable_mask(&self) -> u8 {
        self.regs[7]
    }

    /// Channel volume (low 4 bits) and envelope-mode flag (bit 4).
    fn volume(&self, ch: usize) -> (u8, bool) {
        let v = self.regs[8 + ch];
        (v & 0x0F, (v & 0x10) != 0)
    }

    /// Envelope shape (bits 0-3 of register 0x0D).
    fn env_shape(&self) -> u8 {
        self.regs[0x0D] & 0x0F
    }

    /// Advance the chip state by `apu_cycles` APU cycles (CPU clock / 2).
    pub fn clock(&mut self, apu_cycles: u32) {
        for _ in 0..apu_cycles {
            // Tone channels: 12-bit down-counters; toggle output at 0.
            for ch in 0..CHANNELS {
                if self.tone_timer[ch] == 0 {
                    self.tone_timer[ch] = self.tone_period(ch);
                    self.tone_out[ch] ^= 1;
                } else {
                    self.tone_timer[ch] -= 1;
                }
            }
            // Noise: 5-bit down-counter; clock LFSR at 0.
            if self.noise_timer == 0 {
                self.noise_timer = self.noise_period();
                // YM2149 noise LFSR: bit 0 = bit 0 XOR bit 3, shift right.
                let bit = (self.noise_lfsr ^ (self.noise_lfsr >> 3)) & 1;
                self.noise_lfsr = (self.noise_lfsr >> 1) | (bit << 16);
                self.noise_out = (self.noise_lfsr & 1) as u8;
            } else {
                self.noise_timer -= 1;
            }
            // Envelope: 16-bit down-counter; advance shape position at 0.
            if !self.env_holding {
                if self.env_timer == 0 {
                    self.env_timer = self.env_period();
                    if self.env_pos < 31 {
                        self.env_pos += 1;
                    } else {
                        // End of cycle — apply shape hold/continue logic.
                        self.apply_shape_end();
                    }
                } else {
                    self.env_timer -= 1;
                }
            }
        }
    }

    /// Apply the envelope shape's end-of-cycle behaviour.
    fn apply_shape_end(&mut self) {
        let shape = self.env_shape();
        // Bit 0 = continue, bit 1 = attack, bit 2 = alternate, bit 3 = hold.
        let hold = (shape & 0x08) != 0;
        let alternate = (shape & 0x04) != 0;
        if hold {
            self.env_holding = true;
            if alternate {
                self.env_pos = 31 - self.env_pos;
            }
        } else {
            // Non-hold: restart cycle (continue) or hold at 0.
            self.env_pos = 0;
        }
    }

    /// Current envelope amplitude (0-15) given the shape and position.
    /// Shape bits (per AY-3-8910 spec): bit 0 = Continue, bit 1 = Attack,
    /// bit 2 = Alternate, bit 3 = Hold. The 32-step envelope cycle ramps
    /// from 0 to 15 (attack) or 15 to 0 (decay) over the first 16 steps,
    /// then reverses direction if Alternate is set.
    fn env_amplitude(&self) -> u8 {
        let shape = self.env_shape();
        let attack = (shape & 0x02) != 0; // bit 1 = Attack
        let alternate = (shape & 0x04) != 0; // bit 2 = Alternate
        let pos = self.env_pos;
        // First half (0..16): attack ramps 0→15, decay ramps 15→0.
        // pos is 0..=31 but each step is 2 amplitude units, so pos/2 ∈ 0..=15.
        let half = (pos / 2).min(15);
        let amp = if attack { half } else { 15 - half };
        // Second half (16..32): alternate reverses direction; non-alternate
        // holds the final value (Continue) or 0.
        if pos >= 16 {
            if alternate {
                let second = ((pos - 16) / 2).min(15);
                if attack {
                    15 - second
                } else {
                    second
                }
            } else if attack {
                15 // hold at peak
            } else {
                0 // hold at zero
            }
        } else {
            amp
        }
    }

    /// Per-channel output sample (0-15 amplitude scale).
    fn chan_out(&self, ch: usize) -> u8 {
        let disable = self.disable_mask();
        let tone_en = (disable >> ch) & 1 == 0;
        let noise_en = (disable >> (ch + 3)) & 1 == 0;
        let (vol, env_mode) = self.volume(ch);
        let amp = if env_mode { self.env_amplitude() } else { vol };
        // Gate: tone AND noise both contribute; if disabled, the source is 0.
        let tone = if tone_en { self.tone_out[ch] } else { 0 };
        let noise = if noise_en { self.noise_out } else { 0 };
        // Output is amp when (tone | noise) is non-zero, else 0.
        if (tone | noise) != 0 {
            amp
        } else {
            0
        }
    }

    /// Mixed output sample in `[-1.0, 1.0]`. Silence (all channels off)
    /// maps to 0.0; full output (3 channels at max amp 15) maps to 1.0.
    pub fn sample(&self) -> f32 {
        let a = self.chan_out(0) as i32;
        let b = self.chan_out(1) as i32;
        let c = self.chan_out(2) as i32;
        let sum = a + b + c; // 0..=45
        (sum as f32 / 45.0).clamp(-1.0, 1.0)
    }
}
