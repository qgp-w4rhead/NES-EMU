//! Emulator state — ties the CPU, PPU, and bus together and drives them
//! in a frame-locked main loop.
//!
//! The NES runs three independent clocks:
//!
//! - **CPU** at ~1.79 MHz (NTSC)
//! - **PPU** at ~5.37 MHz (3× CPU clock)
//! - **APU** at ~894.9 kHz (CPU clock / 2 — not yet modelled)
//!
//! One NTSC frame is 262 PPU scanlines × 341 PPU cycles = 89,342 PPU
//! cycles = 29,780.67 CPU cycles. With VBlank NMI overhead and OAM-DMA
//! stalls the effective per-frame CPU cycle count is ~29,830.
//!
//! `EmulatorState::step_frame` runs the CPU and PPU in lockstep (1 CPU
//! cycle = 3 PPU cycles) until the PPU completes a full frame (scanline
//! wraps from the prerender scanline 261 back to scanline 0), then
//! renders the framebuffer. The video layer (M12 `src/video.rs`) uploads
//! it to an SDL2 texture.
//!
//! See: https://www.nesdev.org/wiki/Cycle_reference
//! See: https://www.nesdev.org/wiki/PPU_rendering#Timing

#![allow(dead_code)]

use crate::audio::CPU_CYCLES_PER_SAMPLE;
use crate::bus::Bus;
use crate::cartridge::Cartridge;
use crate::cpu::Cpu;
use crate::ppu::{CYCLES_PER_SCANLINE, SCANLINES_PER_FRAME, SCANLINE_PRERENDER};

/// The complete NES emulator state — CPU + bus (which owns the PPU, APU,
/// and cartridge) + audio sample accumulation.
///
/// Created once at startup and driven by the main loop's
/// [`EmulatorState::step_frame`]. No allocation happens inside
/// `step_frame` after construction (the PPU framebuffer, bg-pattern
/// buffer, and audio buffer are pre-allocated).
pub struct EmulatorState {
    cpu: Cpu,
    bus: Bus,
    /// Audio sample accumulator — tracks fractional CPU cycles toward
    /// the next audio sample.
    sample_accumulator: f32,
    /// Pre-allocated buffer of audio samples produced during the current
    /// frame. Drained by the main loop via [`take_audio_samples`].
    audio_buffer: Vec<f32>,
}

impl EmulatorState {
    /// Build an emulator with a loaded cartridge. The CPU is left in its
    /// `Cpu::new` power-on state; call [`EmulatorState::reset`] to load
    /// PC from the cartridge's RESET vector.
    pub fn new(cartridge: Cartridge) -> Self {
        Self {
            cpu: Cpu::new(),
            bus: Bus::with_cartridge(cartridge),
            sample_accumulator: 0.0,
            // Pre-allocate for ~735 samples/frame (44100/60) + headroom.
            audio_buffer: Vec::with_capacity(800),
        }
    }

    /// Perform the 6502 RESET sequence: load PC from `$FFFC/$FFFD`, set
    /// SP = `$FD`, set the I flag. This is the normal boot path after
    /// constructing the emulator.
    ///
    /// See: https://www.nesdev.org/wiki/CPU_interrupts#RESET
    pub fn reset(&mut self) {
        self.cpu.reset(&mut self.bus);
    }

    /// Borrow the CPU.
    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    /// Mutably borrow the CPU.
    pub fn cpu_mut(&mut self) -> &mut Cpu {
        &mut self.cpu
    }

    /// Borrow the bus.
    pub fn bus(&self) -> &Bus {
        &self.bus
    }

    /// Mutably borrow the bus.
    pub fn bus_mut(&mut self) -> &mut Bus {
        &mut self.bus
    }

    /// Whether the loaded cartridge has battery-backed PRG-RAM. Used by the
    /// battery-SRAM persistence layer (M21) to decide whether to load/save a
    /// `.nessram` file alongside the ROM.
    pub fn has_battery(&self) -> bool {
        self.bus
            .cartridge()
            .map(|c| c.has_battery())
            .unwrap_or(false)
    }

    /// Return the current contents of battery-backed PRG-RAM, or `None` if
    /// the cartridge has no battery-backed SRAM. Used by the battery-SRAM
    /// persistence layer (M21) to dump PRG-RAM to a `.nessram` file on exit.
    pub fn battery_sram(&self) -> Option<Vec<u8>> {
        self.bus.cartridge().and_then(|c| c.battery_sram())
    }

    /// Load battery-backed PRG-RAM contents from a previously-saved
    /// `.nessram` file. Called on boot when a `.nessram` file exists
    /// alongside the ROM. Non-battery cartridges silently ignore the call.
    pub fn load_battery_sram(&mut self, data: &[u8]) {
        if let Some(cart) = self.bus.cartridge_mut() {
            cart.load_battery_sram(data);
        }
    }

    /// Current audio sample accumulator (fractional CPU cycles toward the
    /// next audio sample). Exposed for the save state system (M20).
    pub fn sample_accumulator(&self) -> f32 {
        self.sample_accumulator
    }

    /// Set the audio sample accumulator. Used by the save state system
    /// (M20) to restore the accumulator.
    pub fn set_sample_accumulator(&mut self, acc: f32) {
        self.sample_accumulator = acc;
    }

    /// Borrow the current audio sample buffer (samples produced during the
    /// current frame, not yet drained). Exposed for the save state system
    /// (M20).
    pub fn audio_buffer(&self) -> &[f32] {
        &self.audio_buffer
    }

    /// Replace the audio sample buffer. Used by the save state system
    /// (M20) to restore the buffer contents.
    pub fn set_audio_buffer(&mut self, buffer: Vec<f32>) {
        self.audio_buffer = buffer;
    }

    /// Borrow the current framebuffer (256×240 ARGB). The video layer
    /// uploads this to an SDL2 texture each frame.
    pub fn framebuffer(&self) -> &[u32] {
        self.bus.ppu().framebuffer()
    }

    /// Run one full NTSC frame: steps the CPU and PPU in lockstep (1 CPU
    /// cycle = 3 PPU cycles) until the PPU scanline wraps from the
    /// prerender scanline (261) back to scanline 0, then renders the
    /// framebuffer.
    ///
    /// # NMI handling
    ///
    /// When the PPU asserts VBlank NMI (scanline 241, cycle 1, with
    /// PPUCTRL bit 7 set) — or when the game enables NMI mid-VBlank via
    /// a PPUCTRL write — the pending request is latched in the PPU and
    /// consumed here to set `Cpu::nmi_pending`. The CPU services the NMI
    /// at the start of its next `step`.
    ///
    /// # OAM-DMA stall
    ///
    /// A write to `$4014` (OAMDMA) stalls the CPU for 512 cycles while
    /// the 256-byte transfer runs. The bus records the stall; this loop
    /// advances the PPU by 3×512 cycles without running the CPU, keeping
    /// the clocks in sync.
    ///
    /// # Frame boundary detection
    ///
    /// The loop detects a completed frame by observing the PPU scanline
    /// wrapping from 261 (prerender) to 0 (start of next frame). The PPU
    /// is stepped in chunks of at most one scanline (341 cycles) so that
    /// even a large OAM-DMA stall batch (up to 3×516 = 1548 PPU cycles)
    /// cannot skip over scanline 0 without being caught.
    ///
    /// Returns the number of CPU cycles executed during this frame
    /// (including DMA stall cycles), which is approximately 29,830 for
    /// NTSC.
    pub fn step_frame(&mut self) -> u32 {
        let mut cpu_cycles: u32 = 0;
        let mut frame_done = false;

        while !frame_done {
            let prev_scanline = self.bus.ppu().scanline();

            // Execute one CPU instruction.
            let step_cycles = self.cpu.step(&mut self.bus) as u32;
            cpu_cycles = cpu_cycles.saturating_add(step_cycles);

            // Account for OAM-DMA stall: the bus records 512 cycles when
            // $4014 is written (inside the CPU step above). Advance the
            // PPU by the stall time without running more CPU instructions.
            let dma_cycles = self.bus.take_dma_stall_cycles();
            cpu_cycles = cpu_cycles.saturating_add(dma_cycles);

            // Advance the APU by the total CPU cycles this iteration
            // (instruction + DMA stall). The APU runs at CPU clock / 2;
            // the frame counter advances at the CPU clock rate and clocks
            // quarter/half-frame signals (M16).
            let apu_cycles = step_cycles + dma_cycles;
            self.bus.step_apu(apu_cycles);

            // Poll the APU IRQ line (frame counter or DMC). The 6502 IRQ
            // is level-triggered, so we set `irq_pending` whenever the APU
            // flag is set; the CPU services it at the next instruction
            // boundary if the I flag is clear.
            if self.bus.apu_irq_pending() {
                self.cpu.irq_pending = true;
            }

            // Poll the cartridge mapper IRQ line (e.g. MMC3 IRQ counter).
            // Like the APU IRQ, the 6502 IRQ is level-triggered, so we set
            // `irq_pending` whenever the mapper flag is set; the CPU
            // services it at the next instruction boundary if the I flag
            // is clear. The game's IRQ handler clears the flag by writing
            // to the mapper's IRQ-disable register (e.g. $E000 for MMC3).
            if self.bus.cart_irq_pending() {
                self.cpu.irq_pending = true;
            }

            // Generate audio samples at 44.1 kHz from the APU output.
            // One sample every ~40.585 CPU cycles.
            self.sample_accumulator += apu_cycles as f32;
            while self.sample_accumulator >= CPU_CYCLES_PER_SAMPLE {
                self.sample_accumulator -= CPU_CYCLES_PER_SAMPLE;
                self.audio_buffer.push(self.bus.apu().output());
            }

            // Advance the PPU by 3× the total CPU cycles this iteration
            // (instruction + DMA stall), maintaining the 1:3 CPU:PPU clock
            // ratio. Step in chunks of at most one scanline (341 cycles)
            // so the 261→0 wrap can be detected even when a DMA stall
            // pushes the batch past multiple scanlines.
            let ppu_cycles = 3 * apu_cycles;
            let mut remaining = ppu_cycles;
            while remaining > 0 {
                let chunk = remaining.min(CYCLES_PER_SCANLINE as u32);
                self.bus.step_ppu(chunk);
                remaining -= chunk;

                // Consume any latched NMI request after each chunk.
                if self.bus.take_nmi_request() {
                    self.cpu.nmi_pending = true;
                }

                // Detect frame completion: the PPU scanline wrapped from
                // the prerender scanline (261) to scanline 0 (start of a
                // new frame). `prev_scanline == SCANLINE_PRERENDER`
                // guards against breaking on the very first iteration
                // when the PPU starts at scanline 0.
                let curr_scanline = self.bus.ppu().scanline();
                if curr_scanline == 0 && prev_scanline == SCANLINE_PRERENDER {
                    // Any remaining PPU cycles belong to the next frame;
                    // they are discarded here and the PPU resumes a few
                    // cycles into scanline 0 on the next `step_frame`.
                    frame_done = true;
                    break;
                }
            }
        }

        // Render the completed frame into the PPU framebuffer. The PPU
        // state (VRAM, OAM, palette, scroll) reflects the VBlank handler's
        // setup for this new frame — which is exactly what should be
        // displayed.
        self.bus.render_frame();

        cpu_cycles
    }

    /// Drain and return the audio samples produced during the current
    /// frame. The main loop calls this after `step_frame` and pushes the
    /// samples to the SDL2 audio device. The internal buffer is cleared
    /// (capacity preserved) so no allocation happens on the next frame.
    pub fn take_audio_samples(&mut self) -> Vec<f32> {
        let buf = std::mem::take(&mut self.audio_buffer);
        // Re-allocate the buffer with the same capacity for the next frame.
        self.audio_buffer = Vec::with_capacity(buf.capacity().max(800));
        buf
    }

    /// Run `step_frame` and assert the PPU is at a valid frame boundary.
    /// Intended for tests; production code uses `step_frame` directly.
    #[cfg(test)]
    pub fn step_frame_checked(&mut self) -> u32 {
        let cycles = self.step_frame();
        let sl = self.bus.ppu().scanline();
        debug_assert!(
            sl == 0,
            "after step_frame PPU scanline should be 0, got {sl}"
        );
        cycles
    }
}

impl EmulatorState {
    /// Total scanlines per NTSC frame (262). Exposed for tests.
    pub fn scanlines_per_frame() -> u16 {
        SCANLINES_PER_FRAME
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal NROM-128 cartridge (16 KB PRG, 8 KB CHR-RAM) whose
    /// PRG is filled with `0xEA` (NOP) and whose RESET vector points to
    /// `$C000`. Used to exercise the frame loop without a real game.
    fn make_nop_cart() -> Cartridge {
        let mut bytes = vec![b'N', b'E', b'S', 0x1A, 1, 0, 0, 0];
        bytes.extend_from_slice(&[0u8; 8]); // remaining header
        bytes.resize(16 + 16 * 1024, 0xEA); // PRG filled with NOP
                                            // RESET vector → $C000 (mirrors $8000 in NROM-128).
        let reset_off = 16 + 0x3FFC;
        bytes[reset_off] = 0x00;
        bytes[reset_off + 1] = 0xC0;
        // NMI and IRQ vectors → $0000 (unused).
        Cartridge::from_bytes(&bytes).expect("build NOP cart")
    }

    #[test]
    fn reset_loads_pc_from_reset_vector() {
        let mut emu = EmulatorState::new(make_nop_cart());
        emu.reset();
        assert_eq!(emu.cpu().pc, 0xC000);
    }

    #[test]
    fn step_frame_completes_one_ppu_frame() {
        let mut emu = EmulatorState::new(make_nop_cart());
        emu.reset();
        emu.step_frame();
        // After a frame the PPU should be at scanline 0 (start of next frame).
        assert_eq!(emu.bus().ppu().scanline(), 0);
    }

    #[test]
    fn step_frame_runs_approximately_29830_cpu_cycles() {
        let mut emu = EmulatorState::new(make_nop_cart());
        emu.reset();
        let cycles = emu.step_frame();
        // NTSC frame ≈ 29,830 CPU cycles. Allow ±500 for boundary effects
        // and DMA/NMI overhead.
        assert!(
            (29_300..=30_400).contains(&cycles),
            "expected ~29830 CPU cycles per frame, got {cycles}",
        );
    }

    #[test]
    fn multiple_frames_run_without_crash() {
        let mut emu = EmulatorState::new(make_nop_cart());
        emu.reset();
        for _ in 0..10 {
            emu.step_frame();
        }
        assert_eq!(emu.bus().ppu().scanline(), 0);
    }
}
