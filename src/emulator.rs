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

    /// Soft reset (M29 UI hotkey: Ctrl+R). Identical to [`reset`] at the
    /// CPU level — the 6502 RESET sequence is the same whether the
    /// machine is powering on or being reset mid-run. The PPU, APU, and
    /// mapper keep their current state (real hardware partially resets
    /// these too, but for emulator purposes a CPU-only reset matches
    /// what most games expect from the reset button).
    ///
    /// See: https://www.nesdev.org/wiki/CPU_interrupts#RESET
    pub fn soft_reset(&mut self) {
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

        // M25: the PPU's per-pixel (cycle-accurate) renderer fills the
        // framebuffer incrementally during `step_ppu`. We no longer call
        // `render_frame` at the end of the frame — the framebuffer is
        // already complete (visible scanlines are filled pixel-by-pixel;
        // when rendering is disabled, each pixel is the universal bg
        // color). `rendered_this_frame` is reset here so we can detect
        // whether any per-pixel output happened.
        //
        // Clear the framebuffer to the universal background color first so
        // that any pixels not covered by the per-pixel path (e.g. when the
        // PPU starts mid-scanline after a save-state restore and the first
        // few pixels of scanline 0 are skipped) still have a valid color
        // instead of stale data.
        let universal_bg = self.bus.ppu().universal_bg_argb();
        self.bus.ppu_mut().clear_framebuffer(universal_bg);
        self.bus.ppu_mut().reset_rendered_flag();

        loop {
            let prev_scanline = self.bus.ppu().scanline();
            let (tick_cycles, frame_done) = self.step_one_cpu_tick(prev_scanline);
            cpu_cycles = cpu_cycles.saturating_add(tick_cycles);
            if frame_done {
                break;
            }
        }

        // M25: the per-pixel (cycle-accurate) renderer fills the
        // framebuffer incrementally during `step_ppu` above. If for some
        // reason no per-pixel output happened (e.g. the frame ended before
        // any visible scanline was reached), fall back to the whole-frame
        // scanline renderer so the framebuffer is never left stale.
        if !self.bus.ppu().rendered_this_frame() {
            self.bus.render_frame();
        }

        cpu_cycles
    }

    /// Run one CPU instruction plus its PPU/APU/mapper side-effects,
    /// returning the CPU cycles consumed this tick and whether the frame
    /// just completed (the PPU scanline wrapped from the prerender
    /// scanline 261 to scanline 0).
    ///
    /// This is the body of the [`step_frame`] loop, extracted so that the
    /// debugger (M27) can run a single instruction at a time and so that
    /// [`step_frame_debug`] can check breakpoints between ticks without
    /// duplicating the per-tick logic.
    ///
    /// `prev_scanline` is the PPU scanline observed *before* this tick's
    /// CPU step; it is used to detect the 261→0 frame-boundary wrap.
    fn step_one_cpu_tick(&mut self, prev_scanline: u16) -> (u32, bool) {
        let mut cpu_cycles: u32 = 0;

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

        // Advance CPU-clocked mapper logic (FME-7 / VRC6 IRQ timers,
        // VRC6 expansion audio). Mappers without CPU-clocked logic
        // ignore this.
        self.bus.clock_cart_cpu(apu_cycles);

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
        let mut frame_done = false;
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

        (cpu_cycles, frame_done)
    }

    /// Run one CPU instruction (with PPU / APU / mapper side-effects) and
    /// return the CPU cycles consumed. Intended for the debugger's
    /// single-step (F2) — this does *not* loop until a frame completes and
    /// does *not* clear the framebuffer or reset the rendered flag.
    pub fn step_instruction(&mut self) -> u32 {
        let prev_scanline = self.bus.ppu().scanline();
        let (cycles, _frame_done) = self.step_one_cpu_tick(prev_scanline);
        cycles
    }

    /// Run the emulator forward, stopping when the frame completes *or*
    /// when `debugger.check_before_step` reports that the debugger wants
    /// to pause (user paused, single-step consumed, or a breakpoint
    /// matched). Returns the CPU cycles consumed this call.
    ///
    /// When the debugger pauses mid-frame, the framebuffer may be
    /// partially rendered — the caller should still present it (the
    /// per-pixel renderer leaves a valid image for every pixel visited so
    /// far, and the universal-bg clear at the top of this method covers
    /// the rest).
    ///
    /// See: [`crate::debug::CpuDebugger`]
    pub fn step_frame_debug(&mut self, debugger: &mut crate::debug::CpuDebugger) -> u32 {
        // No per-step action in plain debug mode — the debugger's
        // `check_before_step` is consulted by the helper.
        self.step_frame_with(debugger, |_pre_cpu, _bus, _cycles| {})
    }

    /// Run the emulator forward with per-instruction trace logging. This
    /// is [`step_frame_debug`] plus a [`crate::debug::TraceLogger`] call
    /// after each CPU step. The trace logger captures the disassembly and
    /// register state at the *pre-step* PC (matching the FCEUX convention:
    /// the line shows the instruction that ran and the registers as they
    /// were when it started). The cycle count returned by
    /// `step_one_cpu_tick` is passed to `log_instruction` so the trace's
    /// `CYC` column advances correctly.
    ///
    /// The pre-step CPU is captured by cloning `Cpu` (which derives
    /// `Clone`); the disassembly reads via `Bus::peek` (no side-effects),
    /// so capturing cannot perturb the step we are about to run.
    ///
    /// See: [`crate::debug::TraceLogger`]
    pub fn step_frame_traced(
        &mut self,
        debugger: &mut crate::debug::CpuDebugger,
        logger: &mut crate::debug::TraceLogger,
    ) -> u32 {
        self.step_frame_with(debugger, |pre_cpu, bus, cycles| {
            // Log the instruction that just executed. The disassembly
            // reads via `Bus::peek` (no side-effects), so this cannot
            // perturb the next iteration's step. On the first write
            // error (e.g. disk full), stop the trace and report once
            // to stderr so the user knows tracing halted.
            if logger.is_enabled() {
                if let Err(e) = logger.log_instruction(pre_cpu, bus, cycles) {
                    let lines = logger.stop();
                    eprintln!("nes-emu: trace log write failed ({e}); stopped after {lines} lines");
                }
            }
        })
    }

    /// Shared frame loop for `step_frame_debug` and `step_frame_traced`.
    /// Clears the framebuffer, runs CPU/PPU ticks in lockstep until the
    /// PPU completes a frame (or the debugger pauses), invoking `step`
    /// after each CPU tick with the pre-step CPU snapshot, the bus, and
    /// the cycle count for the tick. Finally renders the framebuffer if
    /// the PPU did not already render it inline.
    fn step_frame_with(
        &mut self,
        debugger: &mut crate::debug::CpuDebugger,
        mut step: impl FnMut(&Cpu, &Bus, u32),
    ) -> u32 {
        let mut cpu_cycles: u32 = 0;

        let universal_bg = self.bus.ppu().universal_bg_argb();
        self.bus.ppu_mut().clear_framebuffer(universal_bg);
        self.bus.ppu_mut().reset_rendered_flag();

        loop {
            // Check the debugger before each CPU step. The borrow of
            // `self.cpu` / `self.bus` here is immutable and ends before
            // the mutable `step_one_cpu_tick` call below.
            if debugger.check_before_step(&self.cpu, &self.bus) {
                break;
            }
            // Capture the pre-step CPU so the trace line / step callback
            // sees the instruction at its entry point. `Cpu` derives
            // `Clone` and is small (8 bytes of registers + 2 bools), so
            // this is cheap and allocation-free.
            let pre_cpu = self.cpu.clone();

            let prev_scanline = self.bus.ppu().scanline();
            let (tick_cycles, frame_done) = self.step_one_cpu_tick(prev_scanline);
            cpu_cycles = cpu_cycles.saturating_add(tick_cycles);

            step(&pre_cpu, &self.bus, tick_cycles);

            if frame_done {
                break;
            }
        }

        if !self.bus.ppu().rendered_this_frame() {
            self.bus.render_frame();
        }

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
