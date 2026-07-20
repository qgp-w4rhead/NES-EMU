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

pub mod axrom;
pub mod cnrom;
pub mod mmc1;
pub mod mmc3;
pub mod nrom;
pub mod uxrom;

use crate::cartridge::CartridgeError;

/// Serialised snapshot of a mapper's full internal state, used by the save
/// state system (M20). Each variant holds a complete mapper struct (PRG-ROM,
/// CHR, bank registers, PRG-RAM, IRQ state, etc.). The variant discriminant
/// records which mapper type produced the snapshot so that
/// [`Mapper::restore_state`] can downcast correctly.
///
/// All variants derive `Serialize`/`Deserialize` via the per-mapper struct
/// derives; the enum itself derives them too so the entire snapshot can be
/// serialised with `bincode` as part of a `SaveState`.
#[derive(serde::Serialize, serde::Deserialize)]
pub enum MapperState {
    /// Mapper 0 — NROM.
    Nrom(nrom::Nrom),
    /// Mapper 1 — MMC1.
    Mmc1(mmc1::Mmc1),
    /// Mapper 2 — UxROM.
    Uxrom(uxrom::Uxrom),
    /// Mapper 3 — CNROM.
    Cnrom(cnrom::Cnrom),
    /// Mapper 4 — MMC3.
    Mmc3(mmc3::Mmc3),
    /// Mapper 7 — AxROM.
    Axrom(axrom::Axrom),
}

impl MapperState {
    /// Convert a [`MapperState`] snapshot back into a boxed `dyn Mapper`,
    /// used by the save state system (M20) to reconstruct a cartridge from
    /// a serialised snapshot.
    pub fn into_boxed_mapper(self) -> Box<dyn Mapper> {
        match self {
            MapperState::Nrom(m) => Box::new(m),
            MapperState::Mmc1(m) => Box::new(m),
            MapperState::Uxrom(m) => Box::new(m),
            MapperState::Cnrom(m) => Box::new(m),
            MapperState::Mmc3(m) => Box::new(m),
            MapperState::Axrom(m) => Box::new(m),
        }
    }
}

/// Nametable mirroring mode configured by the cartridge.
///
/// See: https://www.nesdev.org/wiki/Mirroring
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Mirroring {
    /// Horizontal mirroring — NT0/NT1 share the top half, NT2/NT3 the bottom.
    Horizontal,
    /// Vertical mirroring — NT0/NT2 share the left half, NT1/NT3 the right.
    Vertical,
    /// Four-screen VRAM — all four nametables are independently addressable
    /// (requires extra VRAM on the cartridge).
    FourScreen,
    /// Single-screen mirroring — only one nametable is visible at all four
    /// slots. The `u8` selects which nametable (0-3) is visible. Used by
    /// MMC1 (1ScA=0, 1ScB=1) and AxROM (0-3). See:
    /// https://www.nesdev.org/wiki/Mirroring
    SingleScreen(u8),
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

    /// Whether the mapper is currently asserting a CPU IRQ. The emulator
    /// main loop polls this after each CPU step and raises
    /// `Cpu::irq_pending` when it returns `true`. Default is `false`
    /// (mappers without IRQ sources need not override this).
    fn irq_pending(&self) -> bool {
        false
    }

    /// Clock the mapper's IRQ counter by one step. Called by the bus when
    /// the PPU A12 line rises during rendering (approximately once per
    /// scanline). Used by MMC3 and similar mappers for raster-effect IRQs.
    /// Default is a no-op.
    fn clock_irq(&mut self) {}

    /// Return the current contents of battery-backed PRG-RAM (`$6000-$7FFF`),
    /// or `None` if this cartridge has no battery-backed SRAM.
    ///
    /// Used by the battery-SRAM persistence layer (M21) to dump PRG-RAM to a
    /// `.nessram` file on exit. Only mappers that both *have* PRG-RAM and
    /// *are* battery-backed (per the iNES header's battery flag) should
    /// return `Some`. The default implementation returns `None`.
    ///
    /// See: https://www.nesdev.org/wiki/INES#Flags_6
    fn battery_sram(&self) -> Option<Vec<u8>> {
        None
    }

    /// Load battery-backed PRG-RAM contents from a previously-saved
    /// `.nessram` file. Called on boot when a `.nessram` file exists
    /// alongside the ROM.
    ///
    /// Implementations should copy `data` into their PRG-RAM buffer, handling
    /// length mismatches gracefully (truncating or zero-padding to the
    /// mapper's PRG-RAM size). The default implementation is a no-op
    /// (non-battery mappers ignore the call).
    ///
    /// See: https://www.nesdev.org/wiki/INES#Flags_6
    fn load_battery_sram(&mut self, _data: &[u8]) {}

    /// Capture the mapper's full internal state as a [`MapperState`]
    /// snapshot. Used by the save state system (M20) to serialise the
    /// cartridge. Every mapper must implement this.
    fn save_state(&self) -> MapperState;

    /// Restore the mapper's full internal state from a [`MapperState`]
    /// snapshot. Used by the save state system (M20) to deserialise the
    /// cartridge. Every mapper must implement this.
    ///
    /// Implementations should match the `MapperState` variant to `self`'s
    /// type and copy/clone the snapshot's fields into `self`. A variant
    /// mismatch is a programming error (the save state was produced by a
    /// different mapper type) and should panic.
    fn restore_state(&mut self, state: MapperState);
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
        1 => Ok(Box::new(mmc1::Mmc1::new(
            prg_rom,
            chr_rom,
            mirroring,
            has_battery,
        ))),
        2 => Ok(Box::new(uxrom::Uxrom::new(
            prg_rom,
            chr_rom,
            mirroring,
            has_battery,
        ))),
        3 => Ok(Box::new(cnrom::Cnrom::new(
            prg_rom,
            chr_rom,
            mirroring,
            has_battery,
        ))),
        4 => Ok(Box::new(mmc3::Mmc3::new(
            prg_rom,
            chr_rom,
            mirroring,
            has_battery,
        ))),
        7 => Ok(Box::new(axrom::Axrom::new(
            prg_rom,
            chr_rom,
            mirroring,
            has_battery,
        ))),
        n => Err(CartridgeError::UnsupportedMapper(n)),
    }
}
