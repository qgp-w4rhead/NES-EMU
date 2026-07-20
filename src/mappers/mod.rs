//! Cartridge mapper abstraction.
//!
//! Each NES cartridge wires its PRG-ROM / CHR-ROM / PRG-RAM differently and
//! may implement bank switching, IRQ timers, or extra audio. We model this
//! with a `Mapper` trait — every board type is a struct implementing it.
//!
//! The trait is intentionally minimal: the memory bus asks the mapper how to
//! route a CPU-side PRG address ($6000-$FFFF) and a PPU-side CHR address
//! ($0000-$1FFF), and the mapper is responsible for any bank switching,
//! mirroring, or register side effects.
//!
//! See: https://www.nesdev.org/wiki/Mapper
//!
//! The mapper API is exercised by `cartridge` and the memory bus (M3); until
//! the bus lands, individual methods may be unused at runtime, so we silence
//! dead-code warnings at the module level.
#![allow(dead_code)]

pub mod nrom;

use crate::cartridge::CartridgeError;

/// Nametable mirroring mode configured by the cartridge.
///
/// See: https://www.nesdev.org/wiki/Mirroring
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mirroring {
    /// Horizontal mirroring — NT0/NT1 share the top half, NT2/NT3 the bottom.
    Horizontal,
    /// Vertical mirroring — NT0/NT2 share the left half, NT1/NT3 the right.
    Vertical,
    /// Four-screen VRAM — all four nametables are independently addressable
    /// (requires extra VRAM on the cartridge).
    FourScreen,
    /// Single-screen mirroring — only one nametable is visible at all four
    /// slots (used by e.g. AxROM).
    SingleScreen,
}

/// The mapper trait — every board type implements this.
///
/// Addresses passed in are *cart-local*: PRG addresses are in the
/// `$6000..=$FFFF` CPU range (already de-mirrored by the bus), and CHR
/// addresses are in the `$0000..=$1FFF` PPU range.
pub trait Mapper: Send {
    /// Read a byte from the CPU-side PRG address space (`$6000..=$FFFF`).
    fn read_prg(&self, addr: u16) -> u8;

    /// Write a byte to the CPU-side PRG address space (`$6000..=$FFFF`).
    /// For read-only ROM mappers this is usually a no-op (or a register
    /// write for bank-switching boards).
    fn write_prg(&mut self, addr: u16, value: u8);

    /// Read a byte from the PPU-side CHR address space (`$0000..=$1FFF`).
    fn read_chr(&self, addr: u16) -> u8;

    /// Write a byte to the PPU-side CHR address space (`$0000..=$1FFF`).
    /// Only meaningful for CHR-RAM carts; CHR-ROM carts ignore writes.
    fn write_chr(&mut self, addr: u16, value: u8);

    /// Nametable mirroring mode advertised by this cartridge.
    fn mirror_mode(&self) -> Mirroring;

    /// Whether the cartridge has battery-backed PRG-RAM at `$6000-$7FFF`.
    /// Default is `false`; mappers with battery SRAM override this.
    fn has_battery(&self) -> bool {
        false
    }
}

/// Construct the appropriate mapper for an iNES mapper number.
///
/// Returns `CartridgeError::UnsupportedMapper` for any mapper not yet
/// implemented. Only NROM (mapper 0) is supported at M2; later milestones
/// add MMC1, MMC3, UxROM, etc.
pub fn from_ines(
    mapper_number: u16,
    prg_rom: Vec<u8>,
    chr_rom: Vec<u8>,
    mirroring: Mirroring,
    has_battery: bool,
) -> Result<Box<dyn Mapper>, CartridgeError> {
    match mapper_number {
        0 => Ok(Box::new(nrom::Nrom::new(
            prg_rom,
            chr_rom,
            mirroring,
            has_battery,
        ))),
        n => Err(CartridgeError::UnsupportedMapper(n)),
    }
}
