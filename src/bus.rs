//! CPU memory bus — full address-space routing and mirroring.
//!
//! The NES CPU has a 16-bit address space (`$0000..=$FFFF`). The bus decodes
//! the high bits of the address and routes each access to the appropriate
//! device:
//!
//! | Range         | Device                                  |
//! |---------------|-----------------------------------------|
//! | `$0000-$07FF` | 2 KB internal RAM                       |
//! | `$0800-$1FFF` | Mirror of `$0000-$07FF` (3 copies)      |
//! | `$2000-$2007` | PPU registers                           |
//! | `$2008-$3FFF` | Mirror of `$2000-$2007` (every 8 bytes) |
//! | `$4000-$4017` | APU and I/O registers                   |
//! | `$4018-$401F` | APU / I/O test mode (disabled, ignored) |
//! | `$4020-$FFFF` | Cartridge space (PRG-RAM / PRG-ROM)     |
//!
//! See: https://www.nesdev.org/wiki/CPU_memory_map
//!
//! The PPU (M7) and APU (M14/M16) are not yet implemented. Until they land,
//! accesses to their register ranges are routed to small "open-bus" latch
//! arrays — writes record the value, reads return it. This is genuine
//! open-bus behavior (the last value written stays visible on the bus) and
//! makes register *mirroring* unit-testable without the devices present.
//! When the PPU/APU are introduced, the relevant `ppu_*` / `apu_*` helpers
//! will be changed to delegate to those structs instead of the latches.

#![allow(dead_code)]

use crate::apu::Apu;
use crate::cartridge::Cartridge;
use crate::joypad::Joypad;
use crate::ppu::Ppu;

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
const APU_IO_REG_COUNT: usize = 0x18;

/// First address of cartridge space.
const CART_BASE: u16 = 0x4020;

/// Last address of the APU / I/O test region (disabled on retail units).
const APU_IO_TEST_END: u16 = 0x401F;

/// The CPU memory bus.
///
/// Owns the 2 KB internal RAM, the PPU (with its VRAM / OAM / palette), and
/// an optional loaded cartridge. The APU register window is still backed by
/// an open-bus latch until the APU lands in M14/M16.
pub struct Bus {
    /// 2 KB internal CPU RAM (`$0000-$07FF`). Mirrors at `$0800-$1FFF` are
    /// handled by masking in `read` / `write`.
    ram: [u8; RAM_SIZE],

    /// The PPU (Picture Processing Unit). Routed for `$2000-$3FFF` (PPU
    /// registers, 8-byte mirror) and `$4014` (OAMDMA, in the APU/IO window).
    ppu: Ppu,

    /// Open-bus latch for the APU / I/O register window (`$4000-$4017`),
    /// indexed by `addr - 0x4000`. Replaced by real APU routing in M14/M16.
    apu_open_bus: [u8; APU_IO_REG_COUNT],

    /// The APU (Audio Processing Unit). Pulse channels 1 & 2 landed in M14;
    /// triangle, noise, DMC, and the frame counter follow in M15/M16.
    /// See: https://www.nesdev.org/wiki/APU
    apu: Apu,

    /// The two NES standard controllers, polled via `$4016`/`$4017`.
    /// Introduced in M13. See: https://www.nesdev.org/wiki/Controller_port
    joypad: Joypad,

    /// Loaded cartridge, if any. When `None`, cartridge space reads return
    /// open bus (`0x00`) and writes are ignored.
    cartridge: Option<Cartridge>,

    /// Pending OAM-DMA stall cycles (512 per DMA). Set when `$4014` is
    /// written; consumed by the emulator main loop (M12) so the PPU
    /// advances by the stall time while the CPU is paused.
    /// See: https://www.nesdev.org/wiki/PPU_registers#OAMDMA
    dma_stall_cycles: u32,
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

    /// Advance the APU by `cpu_cycles` CPU cycles. The APU runs at half the
    /// CPU clock, so the channel timers tick once every 2 CPU cycles. The
    /// frame counter advances at the CPU clock rate. The DMC channel's DMA
    /// fetches read from CPU memory (RAM + cartridge PRG space) via a
    /// split-borrow closure.
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

    /// Whether the APU has a pending IRQ (frame counter or DMC). The
    /// emulator main loop polls this after each CPU step and raises
    /// `Cpu::irq_pending` when it returns `true`.
    pub fn apu_irq_pending(&self) -> bool {
        self.apu.irq_pending()
    }

    /// Render the background layer into the PPU framebuffer.
    ///
    /// Delegates to [`Ppu::render_background`], supplying a CHR-read
    /// closure that routes pattern-table fetches through the loaded
    /// cartridge (CHR-ROM or CHR-RAM). With no cartridge loaded, CHR
    /// reads return 0 (blank pattern table).
    ///
    /// This is the M8 entry point for producing a visible frame; the
    /// video layer (M12) uploads the resulting framebuffer to an SDL2
    /// texture.
    pub fn render_background(&mut self) {
        let ppu = &mut self.ppu;
        let cart = self.cartridge.as_ref();
        ppu.render_background(move |addr| match cart {
            Some(c) => c.read_chr(addr),
            None => 0,
        });
    }

    /// Render the sprite layer on top of the existing framebuffer.
    ///
    /// Delegates to [`Ppu::render_sprites`], supplying a CHR-read closure
    /// that routes pattern-table fetches through the loaded cartridge.
    /// Must be called after [`Bus::render_background`] (or use
    /// [`Bus::render_frame`]).
    pub fn render_sprites(&mut self) {
        let ppu = &mut self.ppu;
        let cart = self.cartridge.as_ref();
        ppu.render_sprites(move |addr| match cart {
            Some(c) => c.read_chr(addr),
            None => 0,
        });
    }

    /// Render a full frame: background first, then sprites.
    ///
    /// This is the M9 entry point for producing a complete visible frame;
    /// the video layer (M12) uploads the resulting framebuffer to an SDL2
    /// texture each frame.
    pub fn render_frame(&mut self) {
        let ppu = &mut self.ppu;
        let cart = self.cartridge.as_ref();
        ppu.render_frame(move |addr| match cart {
            Some(c) => c.read_chr(addr),
            None => 0,
        });
    }

    /// Advance the PPU by `cycles` PPU cycles, returning `true` if an NMI
    /// was requested during any of those cycles.
    ///
    /// The main loop (M12) calls this 3× per `Cpu::step` (the PPU runs at
    /// 3× the CPU clock). When this returns `true`, the caller should set
    /// `Cpu::nmi_pending = true` (or call [`Ppu::take_nmi_request`]
    /// directly).
    ///
    /// See: https://www.nesdev.org/wiki/PPU_rendering#Timing
    pub fn step_ppu(&mut self, cycles: u32) -> bool {
        let mut nmi = false;
        for _ in 0..cycles {
            if self.ppu.step() {
                nmi = true;
            }
        }
        nmi
    }

    /// Consume and return the PPU's pending NMI request flag. The main
    /// loop (M12) polls this after stepping the PPU and raises
    /// `Cpu::nmi_pending` when it returns `true`.
    pub fn take_nmi_request(&mut self) -> bool {
        self.ppu.take_nmi_request()
    }

    /// Read a byte from the CPU address space.
    ///
    /// Takes `&mut self` because some reads have side effects: PPU
    /// register reads clear flags / increment pointers, and mapper
    /// reads may trigger bank-switch side effects.
    ///
    /// Routing follows the standard NES CPU memory map; see the module docs
    /// and <https://www.nesdev.org/wiki/CPU_memory_map>.
    pub fn read(&mut self, addr: u16) -> u8 {
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
    pub fn write(&mut self, addr: u16, value: u8) {
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
        if reg & 0x07 == 7 {
            self.ppu_write_ppudata(value);
            return;
        }
        self.ppu.write_register(reg, value);
    }

    /// Read PPUDATA ($2007) with the buffered-read semantics and CHR routing.
    ///
    /// - Palette reads (`$3F00-$3FFF`) return immediately (no buffer).
    /// - All other reads return the stale buffer; the freshly-fetched value
    ///   is stored into the buffer for the next read.
    /// - After the access, `v` advances by 1 or 32 (PPUCTRL bit 2).
    ///
    /// See: https://www.nesdev.org/wiki/PPU_registers#PPUDATA
    fn ppu_read_ppudata(&mut self) -> u8 {
        let addr = self.ppu.vram_addr();

        if addr >= 0x3F00 {
            // Palette: the returned value comes from palette RAM directly
            // (bypassing the stale buffer), but the buffer is still loaded
            // with the nametable byte at `v & 0x2FFF` per NESdev. This
            // matters for a subsequent non-palette read.
            // See: https://www.nesdev.org/wiki/PPU_registers#PPUDATA
            let val = self.ppu.read_palette(addr);
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
        // Record the CPU stall cycles for the emulator loop to consume.
        self.dma_stall_cycles = self.dma_stall_cycles.saturating_add(512);
    }

    /// Consume and return pending OAM-DMA stall cycles (set by a write to
    /// `$4014`). The emulator main loop calls this after each `Cpu::step`
    /// and advances the PPU by 3× the returned value to model the CPU
    /// being stalled for the DMA transfer.
    pub fn take_dma_stall_cycles(&mut self) -> u32 {
        let c = self.dma_stall_cycles;
        self.dma_stall_cycles = 0;
        c
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

    /// Read the `$4015` APU status register. Bits 0-4 come from the APU
    /// (channel length/bytes-remaining status); bit 5 comes from the
    /// open-bus latch (unused); bits 6,7 are the frame counter and DMC
    /// IRQ flags from the APU. Reading `$4015` clears both IRQ flags
    /// (side effect of `Apu::read_status`).
    fn apu_status_read(&mut self) -> u8 {
        let status = self.apu.read_status();
        (status & 0xDF) | (self.apu_open_bus[0x15] & 0x20)
    }

    /// Read a controller register (`$4016` for controller 1, `$4017` for
    /// controller 2). Bit 0 is the next button bit from the joypad shift
    /// register; bits 1-7 come from the open-bus latch (the last value
    /// written to the register), matching the behavior of a retail NES
    /// without expansion-port peripherals.
    ///
    /// `latch_offset` is the open-bus index (`0x16` or `0x17`).
    ///
    /// See: https://www.nesdev.org/wiki/Controller_port#Reading
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

    /// Read from cartridge space (`$4020-$FFFF`).
    fn cart_read(&self, addr: u16) -> u8 {
        match &self.cartridge {
            Some(cart) => cart.read_prg(addr),
            None => 0x00,
        }
    }

    /// Write to cartridge space (`$4020-$FFFF`).
    ///
    /// After the write, the PPU's nametable mirroring is re-synced from the
    /// cartridge — mappers like MMC1 can change mirroring at runtime via
    /// register writes, and the PPU must reflect the new mode immediately.
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

    // ---- Sanity: header parsing still wires Mirroring through the bus ---
    #[test]
    fn bus_exposes_cartridge_mirror_mode() {
        // Confirms the bus surfaces cartridge metadata (used by PPU in M7+).
        let cart = make_test_cartridge(0x00);
        let bus = Bus::with_cartridge(cart);
        let m = bus.cartridge().expect("cart present").header.mirroring;
        assert_eq!(m, Mirroring::Horizontal);
        // InesHeader fields are also reachable.
        assert_eq!(bus.cartridge().unwrap().header.mapper_number, 0);
        // Reference InesHeader to ensure the type is part of the public API.
        let _: &InesHeader = &bus.cartridge().unwrap().header;
    }
}
