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

use crate::cartridge::Cartridge;

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
/// Owns the 2 KB internal RAM and an optional loaded cartridge. PPU and APU
/// register windows are backed by open-bus latches until those devices are
/// implemented in later milestones.
pub struct Bus {
    /// 2 KB internal CPU RAM (`$0000-$07FF`). Mirrors at `$0800-$1FFF` are
    /// handled by masking in `read` / `write`.
    ram: [u8; RAM_SIZE],

    /// Open-bus latch for the 8 PPU registers (`$2000-$2007`), indexed by
    /// `addr & 0x0007`. Replaced by real PPU routing in M7.
    ppu_open_bus: [u8; PPU_REG_COUNT],

    /// Open-bus latch for the APU / I/O register window (`$4000-$4017`),
    /// indexed by `addr - 0x4000`. Replaced by real APU routing in M14/M16.
    apu_open_bus: [u8; APU_IO_REG_COUNT],

    /// Loaded cartridge, if any. When `None`, cartridge space reads return
    /// open bus (`0x00`) and writes are ignored.
    cartridge: Option<Cartridge>,
}

impl Bus {
    /// Construct an empty bus with no cartridge and zeroed RAM.
    pub fn new() -> Self {
        Self {
            ram: [0u8; RAM_SIZE],
            ppu_open_bus: [0u8; PPU_REG_COUNT],
            apu_open_bus: [0u8; APU_IO_REG_COUNT],
            cartridge: None,
        }
    }

    /// Construct a bus with a loaded cartridge.
    pub fn with_cartridge(cartridge: Cartridge) -> Self {
        Self {
            cartridge: Some(cartridge),
            ..Self::new()
        }
    }

    /// Replace the loaded cartridge. Returns the previous one, if any.
    pub fn insert_cartridge(&mut self, cartridge: Cartridge) -> Option<Cartridge> {
        std::mem::replace(&mut self.cartridge, Some(cartridge))
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

    /// Read a byte from the CPU address space.
    ///
    /// Routing follows the standard NES CPU memory map; see the module docs
    /// and <https://www.nesdev.org/wiki/CPU_memory_map>.
    pub fn read(&self, addr: u16) -> u8 {
        match addr {
            // $0000-$1FFF: 2 KB RAM (mirrored 3 times).
            0x0000..=0x1FFF => self.ram[(addr & RAM_MASK) as usize],

            // $2000-$3FFF: PPU registers (mirrored every 8 bytes).
            0x2000..=0x3FFF => self.ppu_read(addr & PPU_REG_MASK),

            // $4000-$4017: APU and I/O registers.
            0x4000..=0x4017 => self.apu_read(addr - APU_IO_BASE),

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

            0x4000..=0x4017 => self.apu_write(addr - APU_IO_BASE, value),

            // $4018-$401F: disabled test region — ignore writes.
            0x4018..=0x401F => {}

            0x4020..=0xFFFF => self.cart_write(addr, value),
        }
    }

    /// Read from the PPU register file (open-bus latch until M7).
    ///
    /// `reg` is the de-mirrored register index in `0..=7` corresponding to
    /// `$2000..=$2007`.
    fn ppu_read(&self, reg: u16) -> u8 {
        self.ppu_open_bus[reg as usize]
    }

    /// Write to the PPU register file (open-bus latch until M7).
    fn ppu_write(&mut self, reg: u16, value: u8) {
        self.ppu_open_bus[reg as usize] = value;
    }

    /// Read from the APU / I/O register file (open-bus latch until M14/M16).
    ///
    /// `offset` is `addr - 0x4000`, in `0..=0x17`.
    fn apu_read(&self, offset: u16) -> u8 {
        self.apu_open_bus[offset as usize]
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
    fn cart_write(&mut self, addr: u16, value: u8) {
        if let Some(cart) = self.cartridge.as_mut() {
            cart.write_prg(addr, value);
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
    fn ppu_register_write_read_round_trip() {
        let mut bus = Bus::new();
        // $2000 = PPUCTRL.
        bus.write(0x2000, 0xA5);
        assert_eq!(bus.read(0x2000), 0xA5);
        // $2007 = PPUDATA.
        bus.write(0x2007, 0x5A);
        assert_eq!(bus.read(0x2007), 0x5A);
        // Distinct registers stay distinct.
        assert_eq!(bus.read(0x2000), 0xA5);
    }

    #[test]
    fn ppu_registers_mirror_every_8_bytes() {
        let mut bus = Bus::new();
        // Write through the mirror at $2008 and read back via $2000.
        bus.write(0x2008, 0x77);
        assert_eq!(bus.read(0x2000), 0x77);
        assert_eq!(bus.read(0x2008), 0x77);

        // $3FFF is the last mirror slot — it maps to register $2007.
        bus.write(0x3FFF, 0x3C);
        assert_eq!(bus.read(0x2007), 0x3C);
        assert_eq!(bus.read(0x3FFF), 0x3C);

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
        bus.write(0x4000, 0x12);
        assert_eq!(bus.read(0x4000), 0x12);
        bus.write(0x4015, 0x0F); // $4015 = APU status.
        assert_eq!(bus.read(0x4015), 0x0F);
        bus.write(0x4017, 0xC0); // $4017 = frame counter.
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
        let bus = Bus::new();
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
        let bus = Bus::with_cartridge(cart);
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
        let bus = Bus::with_cartridge(cart);
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
        let bus = Bus::default();
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
