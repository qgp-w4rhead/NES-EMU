//! CPU memory bus — address-space routing and mirroring.
//!
//! See: https://www.nesdev.org/wiki/CPU_memory_map

#![allow(dead_code)]

use crate::apu::Apu;
use crate::cartridge::Cartridge;
use crate::joypad::Joypad;
use crate::ppu::{Ppu, ChrReader, SCREEN_HEIGHT};
use crate::debug::PpuWriteLogger;

/// Size of the CPU internal RAM in bytes (2 KB).
pub const RAM_SIZE: usize = 0x0800;

/// Mask applied to addresses in `$0000-$1FFF` to de-mirror RAM.
const RAM_MASK: u16 = RAM_SIZE as u16 - 1; // 0x07FF

/// Number of distinct PPU registers ($2000-$2007).
const PPU_REG_COUNT: usize = 8;

/// Mask applied to addresses in `$2000-$3FFF` to find the PPU register index.
const PPU_REG_MASK: u16 = (PPU_REG_COUNT as u16) - 1; // 0x0007

/// Base address of PPU register space.
const PPU_REG_BASE: u16 = 0x2000;

/// Base address of APU / I/O register space.
const APU_IO_BASE: u16 = 0x4000;

/// Number of bytes in the APU / I/O register window ($4000-$4017).
pub const APU_IO_REG_COUNT: usize = 0x18;

/// First address of cartridge space.
const CART_BASE: u16 = 0x4020;

/// PPU cycle at which the MMC3 IRQ counter is clocked (A12 rising edge approx). See: https://www.nesdev.org/wiki/MMC3#IRQ
const MMC3_IRQ_CLOCK_CYCLE: u16 = 260;

impl ChrReader for Option<&mut Cartridge> {
    #[inline]
    fn read_chr(&mut self, addr: u16) -> u8 {
        match self {
            Some(c) => c.read_chr_latched(addr),
            None => 0,
        }
    }
}

/// Last address of the APU / I/O test region (disabled on retail units).
const APU_IO_TEST_END: u16 = 0x401F;

/// The CPU memory bus — owns RAM, PPU, APU, joypad, and optional cartridge.
pub struct Bus {
    /// 2 KB internal CPU RAM (`$0000-$07FF`).
    ram: [u8; RAM_SIZE],

    /// The PPU, routed for `$2000-$3FFF` and `$4014` (OAMDMA).
    ppu: Ppu,

    /// Open-bus latch for APU/IO register window (`$4000-$4017`).
    apu_open_bus: [u8; APU_IO_REG_COUNT],

    /// The APU (Audio Processing Unit).
    apu: Apu,

    /// Two NES controllers, polled via `$4016`/`$4017`.
    joypad: Joypad,

    /// Loaded cartridge, if any.
    cartridge: Option<Cartridge>,

    /// Pending OAM-DMA stall cycles (512 per DMA, 513 if aligned to odd
    /// CPU cycle). See: https://www.nesdev.org/wiki/PPU_registers#OAMDMA
    dma_stall_cycles: u32,

    /// Total CPU cycles elapsed since power-on. Used to determine even/odd
    /// cycle alignment for OAM-DMA (an odd-cycle write to $4014 adds 1
    /// extra stall cycle for alignment to the next even cycle).
    cpu_cycle_count: u64,

    /// Optional PPU register write logger (toggled via debug hotkey).
    /// When `Some` and enabled, every PPU register write and OAM DMA is
    /// logged with the PPU scanline/cycle for timing-race analysis.
    ppu_write_logger: Option<PpuWriteLogger>,
}

impl Bus {
    /// Construct an empty bus with no cartridge and zeroed RAM.
    pub fn new() -> Self {
        Self {
            ram: [0u8; RAM_SIZE],
            ppu: Ppu::new(),
            apu_open_bus: [0u8; APU_IO_REG_COUNT],
            apu: Apu::new(),
            joypad: Joypad::new(),
            cartridge: None,
            dma_stall_cycles: 0,
            cpu_cycle_count: 0,
            ppu_write_logger: None,
        }
    }

    /// Construct a bus with a loaded cartridge. The PPU's nametable mirroring
    /// is configured from the cartridge header.
    pub fn with_cartridge(cartridge: Cartridge) -> Self {
        let mirroring = cartridge.mirror_mode();
        let mut bus = Self {
            cartridge: Some(cartridge),
            ..Self::new()
        };
        bus.ppu.set_mirroring(mirroring);
        bus
    }

    /// Replace the loaded cartridge. Returns the previous one, if any.
    /// Updates the PPU mirroring from the new cartridge.
    pub fn insert_cartridge(&mut self, cartridge: Cartridge) -> Option<Cartridge> {
        let mirroring = cartridge.mirror_mode();
        let prev = std::mem::replace(&mut self.cartridge, Some(cartridge));
        self.ppu.set_mirroring(mirroring);
        prev
    }

    /// Remove and return the loaded cartridge, leaving the bus empty.
    pub fn remove_cartridge(&mut self) -> Option<Cartridge> {
        self.cartridge.take()
    }

    /// Borrow the loaded cartridge, if any.
    pub fn cartridge(&self) -> Option<&Cartridge> {
        self.cartridge.as_ref()
    }

    /// Mutably borrow the loaded cartridge, if any.
    pub fn cartridge_mut(&mut self) -> Option<&mut Cartridge> {
        self.cartridge.as_mut()
    }

    /// Borrow the PPU.
    pub fn ppu(&self) -> &Ppu {
        &self.ppu
    }

    /// Mutably borrow the PPU.
    pub fn ppu_mut(&mut self) -> &mut Ppu {
        &mut self.ppu
    }

    /// Borrow the joypad (controllers 1 and 2).
    pub fn joypad(&self) -> &Joypad {
        &self.joypad
    }

    /// Mutably borrow the joypad. The host input layer (M13 `src/input.rs`)
    /// uses this to feed SDL2 keyboard events into the controller state.
    pub fn joypad_mut(&mut self) -> &mut Joypad {
        &mut self.joypad
    }

    /// Borrow the APU.
    pub fn apu(&self) -> &Apu {
        &self.apu
    }

    /// Mutably borrow the APU.
    pub fn apu_mut(&mut self) -> &mut Apu {
        &mut self.apu
    }

    /// Advance APU by `cpu_cycles` (DMC DMA reads from RAM/cartridge).
    pub fn step_apu(&mut self, cpu_cycles: u32) {
        let apu = &mut self.apu;
        let ram = &self.ram;
        let cart = self.cartridge.as_ref();
        apu.step(cpu_cycles, |addr| match addr {
            0x0000..=0x1FFF => ram[(addr & RAM_MASK) as usize],
            0x8000..=0xFFFF => match cart {
                Some(c) => c.read_prg(addr),
                None => 0,
            },
            _ => 0,
        });
    }

    /// Whether the APU has a pending IRQ (frame counter or DMC).
    pub fn apu_irq_pending(&self) -> bool {
        self.apu.irq_pending()
    }

    /// Whether the cartridge mapper is asserting a CPU IRQ.
    pub fn cart_irq_pending(&self) -> bool {
        self.cartridge
            .as_ref()
            .map(|c| c.irq_pending())
            .unwrap_or(false)
    }

    /// Advance cartridge mapper's CPU-clocked logic by `cpu_cycles`.
    pub fn clock_cart_cpu(&mut self, cpu_cycles: u32) {
        if let Some(cart) = self.cartridge.as_mut() {
            cart.clock_cpu(cpu_cycles);
        }
    }

    /// Current expansion-audio sample `[-1.0, 1.0]` from cartridge audio chip.
    pub fn expansion_audio_sample(&self) -> f32 {
        self.cartridge
            .as_ref()
            .map(|c| c.expansion_audio_sample())
            .unwrap_or(0.0)
    }

    /// Render background layer into PPU framebuffer (CHR via cartridge).
    pub fn render_background(&mut self) {
        let ppu = &mut self.ppu;
        let mut cart = self.cartridge.as_mut();
        ppu.render_background(|addr| match cart.as_mut() {
            Some(c) => c.read_chr_latched(addr),
            None => 0,
        });
    }

    /// Render sprite layer on top of existing framebuffer (CHR via cartridge).
    pub fn render_sprites(&mut self) {
        let ppu = &mut self.ppu;
        let mut cart = self.cartridge.as_mut();
        ppu.render_sprites(|addr| match cart.as_mut() {
            Some(c) => c.read_chr_latched(addr),
            None => 0,
        });
    }

    /// Render full frame: background then sprites.
    pub fn render_frame(&mut self) {
        let ppu = &mut self.ppu;
        let mut cart = self.cartridge.as_mut();
        ppu.render_frame(|addr| match cart.as_mut() {
            Some(c) => c.read_chr_latched(addr),
            None => 0,
        });
    }

    /// Advance PPU by `cycles` PPU cycles; returns `true` if NMI requested. See: https://www.nesdev.org/wiki/PPU_rendering#Timing
    pub fn step_ppu(&mut self, cycles: u32) -> bool {
        let mut nmi = false;
        let prerender = self.ppu.region().scanline_prerender();
        let rendering = self.ppu.is_rendering();
        let mut cart = self.cartridge.as_mut();
        for _ in 0..cycles {
            if self.ppu.step_rendered(&mut cart) {
                nmi = true;
            }
            let ppu_cycle = self.ppu.cycle();
            let scanline = self.ppu.scanline();
            if rendering
                && ppu_cycle == MMC3_IRQ_CLOCK_CYCLE
                && (scanline < SCREEN_HEIGHT as u16 || scanline == prerender)
            {
                if let Some(c) = cart.as_mut() {
                    c.clock_irq();
                }
            }
            if scanline == prerender && ppu_cycle == 1 {
                if let Some(c) = cart.as_mut() {
                    c.reset_scanline_counter();
                }
            }
        }
        nmi
    }

    /// Consume and return pending PPU NMI request flag.
    pub fn take_nmi_request(&mut self) -> bool {
        self.ppu.take_nmi_request()
    }

    /// Read a byte from CPU address space (side-effectful: PPU/mapper reads).
    #[inline] pub fn read(&mut self, addr: u16) -> u8 {
        match addr {
            // $0000-$1FFF: 2 KB RAM (mirrored 3 times).
            0x0000..=0x1FFF => self.ram[(addr & RAM_MASK) as usize],

            // $2000-$3FFF: PPU registers (mirrored every 8 bytes).
            0x2000..=0x3FFF => self.ppu_read(addr & PPU_REG_MASK),

            // $4000-$4007: pulse channel registers (M14). Write-only on real
            // hardware; reads return the open-bus latch (last written value).
            0x4000..=0x4007 => self.apu_read(addr - APU_IO_BASE),

            // $4008-$400B: triangle channel registers (M15). Write-only;
            // reads return the open-bus latch.
            0x4008..=0x400B => self.apu_read(addr - APU_IO_BASE),

            // $400C-$400F: noise channel registers (M15). Write-only;
            // reads return the open-bus latch.
            0x400C..=0x400F => self.apu_read(addr - APU_IO_BASE),

            // $4010-$4013: DMC channel registers (M16). Write-only on real
            // hardware; reads return the open-bus latch (last written value).
            0x4010..=0x4013 => self.apu_read(addr - APU_IO_BASE),

            // $4014: OAMDMA — write-only; reads return open bus.
            0x4014 => self.apu_read(0x14),

            // $4015: APU status — bits 0-4 reflect channel length/bytes
            // remaining; bit 5 is open bus; bits 6,7 are the frame counter
            // and DMC IRQ flags. Reading $4015 clears both IRQ flags.
            0x4015 => self.apu_status_read(),

            // $4016: controller 1 + open-bus bits 1-7.
            0x4016 => self.joypad_read(0, 0x16),

            // $4017: controller 2 + open-bus bits 1-7 (APU frame-counter
            // IRQ flag at bit 6 lands in M14/M16; until then the open-bus
            // latch preserves the last write for the existing round-trip
            // test).
            0x4017 => self.joypad_read(1, 0x17),

            // $4018-$401F: APU / I/O test mode — disabled, reads as open bus.
            0x4018..=0x401F => 0x00,

            // $4020-$FFFF: cartridge space (PRG-RAM / PRG-ROM / mapper regs).
            0x4020..=0xFFFF => self.cart_read(addr),
        }
    }

    /// Write a byte to the CPU address space.
    #[inline] pub fn write(&mut self, addr: u16, value: u8) {
        match addr {
            0x0000..=0x1FFF => self.ram[(addr & RAM_MASK) as usize] = value,

            0x2000..=0x3FFF => self.ppu_write(addr & PPU_REG_MASK, value),

            // $4000-$4007: pulse channel registers (M14). Routed to the APU
            // pulse channels; also latched on the open bus so reads return
            // the last written value (matching real open-bus behavior).
            0x4000..=0x4007 => {
                let offset = addr - APU_IO_BASE;
                self.apu_pulse_write(offset, value);
                self.apu_open_bus[offset as usize] = value;
            }

            // $4008-$400B: triangle channel registers (M15).
            0x4008..=0x400B => {
                let offset = addr - APU_IO_BASE;
                self.apu_triangle_write(offset, value);
                self.apu_open_bus[offset as usize] = value;
            }

            // $400C-$400F: noise channel registers (M15).
            0x400C..=0x400F => {
                let offset = addr - APU_IO_BASE;
                self.apu_noise_write(offset, value);
                self.apu_open_bus[offset as usize] = value;
            }

            // $4010-$4013: DMC channel registers (M16). Routed to the APU
            // DMC channel; also latched on the open bus.
            0x4010..=0x4013 => {
                let offset = addr - APU_IO_BASE;
                self.apu_dmc_write(offset, value);
                self.apu_open_bus[offset as usize] = value;
            }

            // $4014: OAMDMA — triggers 256-byte DMA from CPU page to OAM.
            0x4014 => self.oam_dma(value),

            // $4015: APU status — enables/disables all 5 channels (bits
            // 0-4). Also latched on the open bus for bit 5 preservation.
            0x4015 => {
                self.apu_open_bus[0x15] = value;
                self.apu.write_status(value);
            }

            // $4016: controller strobe (bit 0). Also latched on the open
            // bus so subsequent reads see the last written value in bits
            // 1-7.
            0x4016 => {
                self.apu_write(0x16, value);
                self.joypad.write_strobe(value);
            }

            // $4017: APU frame counter control (M16). Routed to the APU
            // frame counter; also latched on the open bus.
            0x4017 => {
                self.apu_open_bus[0x17] = value;
                self.apu.write_frame_counter(value);
            }

            // $4018-$401F: disabled test region — ignore writes.
            0x4018..=0x401F => {}

            0x4020..=0xFFFF => self.cart_write(addr, value),
        }
    }

    /// Read from the PPU register file. `reg` is the de-mirrored register
    /// index in `0..=7` corresponding to `$2000..=$2007`.
    fn ppu_read(&mut self, reg: u16) -> u8 {
        // PPUDATA (reg 7) needs CHR routing through the cartridge, so it is
        // handled here rather than inside the PPU.
        if reg & 0x07 == 7 {
            return self.ppu_read_ppudata();
        }
        self.ppu.read_register(reg)
    }

    /// Write to the PPU register file.
    fn ppu_write(&mut self, reg: u16, value: u8) {
        if let Some(logger) = &mut self.ppu_write_logger {
            if logger.is_enabled() {
                logger.log_write(reg, value, self.ppu.scanline(), self.ppu.cycle(), self.ppu.vram_addr(), None);
            }
        }
        if reg & 0x07 == 7 {
            self.ppu_write_ppudata(value);
            return;
        }
        self.ppu.write_register(reg, value);
    }

    /// Read PPUDATA ($2007) with buffered-read semantics and CHR routing. See: https://www.nesdev.org/wiki/PPU_registers#PPUDATA
    fn ppu_read_ppudata(&mut self) -> u8 {
        let addr = self.ppu.vram_addr();

        if addr >= 0x3F00 {
            // Palette: the returned value comes from palette RAM directly
            // (bypassing the stale buffer), but the buffer is still loaded
            // with the nametable byte at `v & 0x2FFF` per NESdev. This
            // matters for a subsequent non-palette read. The upper two bits
            // of the value placed on the CPU bus come from the PPU open-bus
            // latch (the last byte written to any PPU register) — palette
            // RAM only stores 6 meaningful bits.
            // See: https://www.nesdev.org/wiki/PPU_registers#PPUDATA
            let pal = self.ppu.read_palette(addr);
            let val = (pal & 0x3F) | (self.ppu.open_bus() & 0xC0);
            let nt_addr = addr & 0x2FFF;
            let buffered_fill = if nt_addr < 0x2000 {
                match &self.cartridge {
                    Some(cart) => cart.read_chr(nt_addr),
                    None => 0,
                }
            } else {
                self.ppu.read_nametable(nt_addr)
            };
            self.ppu.set_ppudata_buffer(buffered_fill);
            self.ppu.advance_vram_addr();
            // The value placed on the CPU bus becomes the new PPU open bus.
            self.ppu.set_open_bus(val);
            return val;
        }

        // Buffered read: return the stale buffer, store the fresh value.
        let buffered = self.ppu.ppudata_buffer();
        let raw = if addr < 0x2000 {
            // CHR pattern data — routed through the cartridge.
            match &self.cartridge {
                Some(cart) => cart.read_chr(addr),
                None => 0,
            }
        } else {
            // Nametable ($2000-$3EFF, with $3000-$3EFF mirror).
            self.ppu.read_nametable(addr)
        };
        self.ppu.set_ppudata_buffer(raw);
        self.ppu.advance_vram_addr();
        // The buffered value placed on the CPU bus becomes the new PPU open bus.
        self.ppu.set_open_bus(buffered);
        buffered
    }

    /// Write PPUDATA ($2007) with CHR routing and auto-increment.
    fn ppu_write_ppudata(&mut self, value: u8) {
        let addr = self.ppu.vram_addr();

        if addr >= 0x3F00 {
            self.ppu.write_palette(addr, value);
        } else if addr < 0x2000 {
            // CHR write — routed through the cartridge (CHR-RAM carts).
            if let Some(cart) = self.cartridge.as_mut() {
                cart.write_chr(addr, value);
            }
        } else {
            self.ppu.write_nametable(addr, value);
        }
        // Update open-bus latch (consistent with other PPU register writes).
        self.ppu.write_register(7, value); // latches open_bus; PPUDATA handler is a no-op
        self.ppu.advance_vram_addr();
    }

    /// Perform an OAM DMA: copy 256 bytes from CPU page `value << 8` into
    /// OAM. On real hardware this stalls the CPU for 512 cycles (514 if
    /// the write lands on an odd CPU cycle); we model the common 512-cycle
    /// stall here. The emulator main loop (M12) consumes
    /// [`Bus::take_dma_stall_cycles`] to advance the PPU by the stall time
    /// while the CPU is paused.
    ///
    /// See: https://www.nesdev.org/wiki/PPU_registers#OAMDMA
    fn oam_dma(&mut self, page: u8) {
        if let Some(logger) = &mut self.ppu_write_logger {
            if logger.is_enabled() {
                logger.log_oam_dma(page, self.ppu.scanline(), self.ppu.cycle(), self.ppu.vram_addr(), None);
            }
        }
        let base = (page as u16) << 8;
        // Collect the source bytes first to avoid borrowing self.read while
        // mutating self.ppu.
        let mut data = [0u8; 256];
        for (i, slot) in data.iter_mut().enumerate() {
            *slot = self.read(base + i as u16);
        }
        self.ppu.oam_dma(&data);
        // Latch the DMA page on the APU/IO open bus for $4014 reads.
        self.apu_open_bus[0x14] = page;
        // Any CPU bus write updates the shared open bus latch. Since the
        // PPU open bus is used for PPU register read bits, writing $4014
        // (which is outside PPU register space) still updates the PPU open
        // bus on real hardware.
        // See: https://www.nesdev.org/wiki/PPU_registers#PPU_open_bus
        self.ppu.set_open_bus(page);
        // OAM-DMA stalls the CPU for 512 cycles. If the write to $4014
        // occurs on an odd CPU cycle, an extra cycle is added for
        // alignment to the next even cycle (513 total).
        // See: https://www.nesdev.org/wiki/PPU_registers#OAMDMA
        let stall = if self.cpu_cycle_count & 1 != 0 { 513 } else { 512 };
        self.dma_stall_cycles = self.dma_stall_cycles.saturating_add(stall);
    }

    /// Consume and return pending OAM-DMA stall cycles.
    pub fn take_dma_stall_cycles(&mut self) -> u32 {
        let c = self.dma_stall_cycles;
        self.dma_stall_cycles = 0;
        c
    }

    /// Side-effect-free read for debug tools (disassembler/memory viewer).
    pub fn peek(&self, addr: u16) -> u8 {
        match addr {
            // RAM + mirrors: pure read, no side-effects.
            0x0000..=0x1FFF => self.ram[(addr & RAM_MASK) as usize],
            // PPU / APU / I/O registers: return a safe snapshot without
            // touching device state. PPU open-bus / status bits are not
            // safely readable without &mut, so report 0 here — the
            // disassembler never decodes from this range in practice.
            0x2000..=0x3FFF => 0,
            0x4000..=0x401F => 0,
            // Cartridge space: PRG-ROM / PRG-RAM reads have no read
            // side-effects on any supported mapper (MMC2 latching lives on
            // the CHR side, routed via `read_chr_latched`, not `read_prg`).
            0x4020..=0xFFFF => match &self.cartridge {
                Some(cart) => cart.read_prg(addr),
                None => 0,
            },
        }
    }

    /// Borrow the internal CPU RAM (2 KB). Used by the save state system
    /// (M20) to serialise RAM contents.
    pub fn ram(&self) -> &[u8; RAM_SIZE] {
        &self.ram
    }

    /// Mutably borrow the internal CPU RAM (2 KB). Used by the save state
    /// system (M20) to restore RAM contents.
    pub fn ram_mut(&mut self) -> &mut [u8; RAM_SIZE] {
        &mut self.ram
    }

    /// Borrow the APU / I/O open-bus latch array. Used by the save state
    /// system (M20) to serialise the open-bus state.
    pub fn apu_open_bus(&self) -> &[u8; APU_IO_REG_COUNT] {
        &self.apu_open_bus
    }

    /// Mutably borrow the APU / I/O open-bus latch array. Used by the save
    /// state system (M20) to restore the open-bus state.
    pub fn apu_open_bus_mut(&mut self) -> &mut [u8; APU_IO_REG_COUNT] {
        &mut self.apu_open_bus
    }

    /// Set the pending OAM-DMA stall cycle count. Used by the save state
    /// system (M20) to restore the DMA stall state.
    pub fn set_dma_stall_cycles(&mut self, cycles: u32) {
        self.dma_stall_cycles = cycles;
    }

    /// Current pending OAM-DMA stall cycles (without consuming). Exposed
    /// for the save state system (M20) to serialise the DMA stall state.
    pub fn dma_stall_cycles(&self) -> u32 {
        self.dma_stall_cycles
    }

    /// Advance the total CPU cycle counter. Called by the emulator loop
    /// after each instruction (and DMA stall) to keep the even/odd parity
    /// tracking accurate for OAM-DMA alignment.
    pub fn advance_cpu_cycles(&mut self, cycles: u32) {
        self.cpu_cycle_count = self.cpu_cycle_count.wrapping_add(cycles as u64);
    }

    /// Current total CPU cycle count (for save state serialisation).
    pub fn cpu_cycle_count(&self) -> u64 {
        self.cpu_cycle_count
    }

    /// Set the total CPU cycle count (for save state restoration).
    pub fn set_cpu_cycle_count(&mut self, count: u64) {
        self.cpu_cycle_count = count;
    }

    /// Install a PPU write logger on the bus. When the logger is enabled,
    /// every PPU register write and OAM DMA is logged with the PPU
    /// scanline/cycle for timing-race analysis.
    pub fn set_ppu_write_logger(&mut self, logger: PpuWriteLogger) {
        self.ppu_write_logger = Some(logger);
    }

    /// Mutably borrow the PPU write logger, if installed.
    pub fn ppu_write_logger_mut(&mut self) -> Option<&mut PpuWriteLogger> {
        self.ppu_write_logger.as_mut()
    }

    /// Read from the APU / I/O register file (open-bus latch until M14/M16).
    ///
    /// `offset` is `addr - 0x4000`, in `0..=0x17`.
    fn apu_read(&self, offset: u16) -> u8 {
        self.apu_open_bus[offset as usize]
    }

    /// Write to a pulse channel register. `offset` is `addr - 0x4000` in
    /// `0..=7`: offsets 0-3 → pulse 1, offsets 4-7 → pulse 2.
    fn apu_pulse_write(&mut self, offset: u16, value: u8) {
        if offset < 4 {
            self.apu.pulse1_mut().write_register(offset as u8, value);
        } else {
            self.apu
                .pulse2_mut()
                .write_register((offset - 4) as u8, value);
        }
    }

    /// Write to a triangle channel register. `offset` is `addr - 0x4000`
    /// in `8..=0xB`: reg = offset - 8 (0..=3).
    fn apu_triangle_write(&mut self, offset: u16, value: u8) {
        self.apu
            .triangle_mut()
            .write_register((offset - 0x08) as u8, value);
    }

    /// Write to a noise channel register. `offset` is `addr - 0x4000` in
    /// `0xC..=0xF`: reg = offset - 0xC (0..=3).
    fn apu_noise_write(&mut self, offset: u16, value: u8) {
        self.apu
            .noise_mut()
            .write_register((offset - 0x0C) as u8, value);
    }

    /// Write to a DMC channel register. `offset` is `addr - 0x4000` in
    /// `0x10..=0x13`: reg = offset - 0x10 (0..=3).
    fn apu_dmc_write(&mut self, offset: u16, value: u8) {
        self.apu
            .dmc_mut()
            .write_register((offset - 0x10) as u8, value);
    }

    /// Read `$4015` APU status (channel bits + IRQ flags; clears IRQs).
    fn apu_status_read(&mut self) -> u8 {
        let status = self.apu.read_status();
        (status & 0xDF) | (self.apu_open_bus[0x15] & 0x20)
    }

    /// Read controller register (`$4016`/`$4017`): bit 0 from joypad, bits 1-7 open-bus. See: https://www.nesdev.org/wiki/Controller_port#Reading
    fn joypad_read(&mut self, controller: usize, latch_offset: u16) -> u8 {
        let button_bit = self.joypad.read(controller) & 1;
        let open_bus = self.apu_open_bus[latch_offset as usize];
        // Bit 0 from the joypad; bits 1-7 from the open-bus latch.
        button_bit | (open_bus & 0xFE)
    }

    /// Write to the APU / I/O register file (open-bus latch until M14/M16).
    fn apu_write(&mut self, offset: u16, value: u8) {
        self.apu_open_bus[offset as usize] = value;
    }

    /// Read from cartridge space (`$4020-$FFFF`, via `read_prg_mut` for side-effects).
    fn cart_read(&mut self, addr: u16) -> u8 {
        match &mut self.cartridge {
            Some(cart) => cart.read_prg_mut(addr),
            None => 0x00,
        }
    }

    /// Write to cartridge space; re-syncs PPU mirroring from cartridge.
    fn cart_write(&mut self, addr: u16, value: u8) {
        if let Some(cart) = self.cartridge.as_mut() {
            cart.write_prg(addr, value);
        }
        if let Some(m) = self.cartridge.as_ref().map(|c| c.mirror_mode()) {
            self.ppu.set_mirroring(m);
        }
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::InesHeader;
    use crate::mappers::Mirroring;

    /// Build a tiny NROM-128 cartridge (16 KB PRG filled with `fill`, no CHR)
    /// entirely in memory — used to exercise cartridge-space routing.
    fn make_test_cartridge(fill: u8) -> Cartridge {
        // Construct via the public `from_bytes` path so the mapper wiring is
        // identical to a real load. 16 KB PRG, 0 CHR (CHR-RAM), mapper 0.
        let mut bytes = Vec::with_capacity(16 + 16 * 1024);
        bytes.extend_from_slice(&[b'N', b'E', b'S', 0x1A]);
        bytes.push(1); // 1 x 16KB PRG bank
        bytes.push(0); // 0 x 8KB CHR (CHR-RAM)
        bytes.push(0); // flags6: mapper 0, horizontal mirroring
        bytes.push(0); // flags7
        bytes.extend_from_slice(&[0u8; 8]);
        bytes.resize(16 + 16 * 1024, fill);
        Cartridge::from_bytes(&bytes).expect("build test cartridge")
    }

    // ---- RAM routing + mirroring --------------------------------------

    #[test]
    fn ram_read_write_round_trip() {
        let mut bus = Bus::new();
        bus.write(0x0000, 0x42);
        assert_eq!(bus.read(0x0000), 0x42);
        bus.write(0x07FF, 0xAB);
        assert_eq!(bus.read(0x07FF), 0xAB);
    }

    #[test]
    fn ram_mirror_0800_aliases_0000() {
        let mut bus = Bus::new();
        bus.write(0x0800, 0x11);
        // $0800 mirrors $0000.
        assert_eq!(bus.read(0x0000), 0x11);
        assert_eq!(bus.read(0x0800), 0x11);
    }

    #[test]
    fn ram_mirror_1000_aliases_0000() {
        let mut bus = Bus::new();
        bus.write(0x1000, 0x22);
        assert_eq!(bus.read(0x0000), 0x22);
        assert_eq!(bus.read(0x1000), 0x22);
    }

    #[test]
    fn ram_mirror_1800_aliases_0000() {
        let mut bus = Bus::new();
        bus.write(0x1800, 0x33);
        assert_eq!(bus.read(0x0000), 0x33);
        assert_eq!(bus.read(0x1800), 0x33);
    }

    #[test]
    fn ram_mirror_wraps_within_2kb() {
        // $1FFF mirrors $07FF (top of physical RAM).
        let mut bus = Bus::new();
        bus.write(0x07FF, 0xCD);
        assert_eq!(bus.read(0x1FFF), 0xCD);
        // $0800 mirrors $0000, not $0800+0.
        bus.write(0x1FFF, 0xEE);
        assert_eq!(bus.read(0x07FF), 0xEE);
    }

    // ---- PPU register mirroring ---------------------------------------

    #[test]
    fn ppu_write_only_register_read_returns_open_bus() {
        let mut bus = Bus::new();
        // PPUCTRL ($2000) is write-only; reads return the open-bus latch
        // (the last value written to any PPU register).
        bus.write(0x2000, 0xA5);
        assert_eq!(bus.read(0x2000), 0xA5);
    }

    #[test]
    fn ppu_oamdata_round_trip() {
        let mut bus = Bus::new();
        // OAMDATA ($2004) is genuinely readable. Set OAMADDR, write, reset,
        // read back.
        bus.write(0x2003, 0x00); // OAMADDR = 0
        bus.write(0x2004, 0x5A); // write OAM[0], OAMADDR → 1
        bus.write(0x2003, 0x00); // reset OAMADDR
        assert_eq!(bus.read(0x2004), 0x5A);
    }

    #[test]
    fn ppu_registers_mirror_every_8_bytes() {
        let mut bus = Bus::new();
        // Write through the mirror at $2008 (maps to $2000 = PPUCTRL).
        // PPUCTRL is write-only, so reading $2000 returns the open-bus
        // latch which holds the last written value.
        bus.write(0x2008, 0x77);
        assert_eq!(bus.read(0x2000), 0x77);
        assert_eq!(bus.read(0x2008), 0x77);

        // $2010 mirrors $2000.
        bus.write(0x2010, 0x10);
        assert_eq!(bus.read(0x2000), 0x10);
    }

    #[test]
    fn ppu_register_window_does_not_leak_into_ram_or_cart() {
        let mut bus = Bus::new();
        bus.write(0x2000, 0xF0);
        // RAM must be untouched.
        assert_eq!(bus.read(0x0000), 0x00);
        // And the cartridge space below $4020 is not affected.
        assert_eq!(bus.read(0x4020), 0x00);
    }

    // ---- APU / I/O register window ------------------------------------

    #[test]
    fn apu_io_register_round_trip() {
        let mut bus = Bus::new();
        // $4000 is a pulse register (M14): writes latch the open bus, and
        // reads return that latch (the register itself is write-only).
        bus.write(0x4000, 0x12);
        assert_eq!(bus.read(0x4000), 0x12);
        // $4015 is the APU status register (M16): bits 0-4 reflect channel
        // length/bytes-remaining; bit 5 comes from the open-bus latch;
        // bits 6,7 are IRQ flags. With no length loaded and no IRQs, the
        // read returns just the open-bus bit 5 (0x0F & 0x20 = 0).
        bus.write(0x4015, 0x0F); // enable pulse 1+2 (bits 0,1); latch 0x0F.
        assert_eq!(bus.read(0x4015), 0x00); // no length loaded → bits 0-4 = 0
                                            // Loading a length into pulse 1 sets bit 0.
        bus.write(0x4003, 0x00); // length index 0 → length = 10
        assert_eq!(bus.read(0x4015) & 0x01, 0x01);
        // $4017 is the frame counter control (M16): writes latch the open
        // bus and configure the frame counter. Reads return the open-bus
        // latch (via the joypad read path, bit 0 from controller 2).
        bus.write(0x4017, 0xC0);
        assert_eq!(bus.read(0x4017), 0xC0);
    }

    #[test]
    fn apu_io_test_region_4018_to_401f_reads_zero_ignores_writes() {
        let mut bus = Bus::new();
        bus.write(0x4018, 0xFF);
        bus.write(0x401F, 0xFF);
        assert_eq!(bus.read(0x4018), 0x00);
        assert_eq!(bus.read(0x401F), 0x00);
    }

    // ---- Cartridge space ----------------------------------------------

    #[test]
    fn cart_space_reads_return_zero_with_no_cartridge() {
        let mut bus = Bus::new();
        assert_eq!(bus.read(0x4020), 0x00);
        assert_eq!(bus.read(0x8000), 0x00);
        assert_eq!(bus.read(0xFFFF), 0x00);
    }

    #[test]
    fn cart_space_writes_ignored_with_no_cartridge() {
        let mut bus = Bus::new();
        // Should not panic; reads still return 0.
        bus.write(0x8000, 0xAB);
        bus.write(0xFFFF, 0xCD);
        assert_eq!(bus.read(0x8000), 0x00);
        assert_eq!(bus.read(0xFFFF), 0x00);
    }

    #[test]
    fn cart_space_routes_to_loaded_cartridge_prg() {
        let cart = make_test_cartridge(0x99);
        let mut bus = Bus::with_cartridge(cart);
        // $8000 is the first PRG-ROM byte (NROM 16K mirrors high half too).
        assert_eq!(bus.read(0x8000), 0x99);
        assert_eq!(bus.read(0xC000), 0x99); // mirror of $8000 for 16K PRG
        assert_eq!(bus.read(0xFFFF), 0x99);
    }

    #[test]
    fn cart_space_prg_ram_region_routes_to_cartridge() {
        // NROM has no PRG-RAM at $6000-$7FFF, so reads return 0 — but the
        // important thing is that the bus *delegates* to the cartridge
        // rather than returning its own open-bus 0. We verify by checking
        // that a 16K PRG ROM's $6000 reads 0 (cartridge's PRG-RAM region
        // answer) and $8000 reads the ROM fill (cartridge's PRG-ROM answer).
        let cart = make_test_cartridge(0x77);
        let mut bus = Bus::with_cartridge(cart);
        assert_eq!(bus.read(0x6000), 0x00); // PRG-RAM region: NROM returns 0
        assert_eq!(bus.read(0x8000), 0x77); // PRG-ROM: fill byte
    }

    #[test]
    fn insert_and_remove_cartridge() {
        let mut bus = Bus::new();
        assert!(bus.cartridge().is_none());

        let cart = make_test_cartridge(0x55);
        assert!(bus.insert_cartridge(cart).is_none());
        assert!(bus.cartridge().is_some());
        assert_eq!(bus.read(0x8000), 0x55);

        let taken = bus.remove_cartridge();
        assert!(taken.is_some());
        assert!(bus.cartridge().is_none());
        // After removal, cartridge space reverts to open bus.
        assert_eq!(bus.read(0x8000), 0x00);
    }

    // ---- Boundary / edge cases ----------------------------------------

    #[test]
    fn full_address_space_routing_smoke() {
        // Walk one representative address per region and confirm routing.
        let cart = make_test_cartridge(0xCC);
        let mut bus = Bus::with_cartridge(cart);

        bus.write(0x0000, 0x01); // RAM
        bus.write(0x2000, 0x02); // PPU reg
        bus.write(0x4000, 0x03); // APU reg

        assert_eq!(bus.read(0x0000), 0x01);
        assert_eq!(bus.read(0x2000), 0x02);
        assert_eq!(bus.read(0x4000), 0x03);
        assert_eq!(bus.read(0x4018), 0x00); // disabled test region
        assert_eq!(bus.read(0x4020), 0x00); // cart PRG-RAM region (NROM: 0)
        assert_eq!(bus.read(0x8000), 0xCC); // cartridge PRG-ROM
    }

    #[test]
    fn default_is_empty_bus() {
        let mut bus = Bus::default();
        assert!(bus.cartridge().is_none());
        assert_eq!(bus.read(0x0000), 0x00);
    }

    // ---- M-BUS-06: OAMDMA open bus + alignment -------------------------

    #[test]
    fn oam_dma_updates_ppu_open_bus() {
        let mut bus = Bus::new();
        // Write 0x12 to PPUCTRL to set the PPU open bus to 0x12.
        bus.write(0x2000, 0x12);
        assert_eq!(bus.read(0x2000), 0x12);
        // Write to OAMDMA ($4014) with page 0x42.
        bus.write(0x4014, 0x42);
        // The PPU open bus should now be 0x42 (the DMA page value).
        // Reading a write-only PPU register should return 0x42.
        assert_eq!(bus.read(0x2000), 0x42);
    }

    #[test]
    fn oam_dma_stall_512_on_even_cycle() {
        let mut bus = Bus::new();
        // cpu_cycle_count starts at 0 (even).
        assert_eq!(bus.cpu_cycle_count() & 1, 0);
        bus.write(0x4014, 0x00);
        // Even cycle → 512 stall cycles.
        assert_eq!(bus.take_dma_stall_cycles(), 512);
    }

    #[test]
    fn oam_dma_stall_513_on_odd_cycle() {
        let mut bus = Bus::new();
        // Advance 3 cycles → odd parity.
        bus.advance_cpu_cycles(3);
        assert_eq!(bus.cpu_cycle_count() & 1, 1);
        bus.write(0x4014, 0x00);
        // Odd cycle → 513 stall cycles (512 + 1 alignment).
        assert_eq!(bus.take_dma_stall_cycles(), 513);
    }

    #[test]
    fn oam_dma_stall_alignment_alternates() {
        let mut bus = Bus::new();
        // First DMA: even (0 cycles) → 512
        bus.write(0x4014, 0x00);
        let s1 = bus.take_dma_stall_cycles();
        assert_eq!(s1, 512);
        // Advance by the stall cycles (even) → still even.
        bus.advance_cpu_cycles(s1);
        // Second DMA: still even → 512
        bus.write(0x4014, 0x00);
        let s2 = bus.take_dma_stall_cycles();
        assert_eq!(s2, 512);
        // Now advance by an odd amount.
        bus.advance_cpu_cycles(3);
        // Third DMA: odd → 513
        bus.write(0x4014, 0x00);
        let s3 = bus.take_dma_stall_cycles();
        assert_eq!(s3, 513);
    }

    #[test]
    fn cpu_cycle_count_round_trips() {
        let mut bus = Bus::new();
        bus.advance_cpu_cycles(12345);
        assert_eq!(bus.cpu_cycle_count(), 12345);
        bus.set_cpu_cycle_count(999);
        assert_eq!(bus.cpu_cycle_count(), 999);
    }
}
