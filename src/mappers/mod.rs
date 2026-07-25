//! Cartridge mapper abstraction — `Mapper` trait for PRG/CHR routing and bank switching.
//!
//! See: https://www.nesdev.org/wiki/Mapper
#![allow(dead_code)]

pub mod axrom;
pub mod cnrom;
pub mod fds;
pub mod fds_audio;
pub mod fme7;
pub mod mmc1;
pub mod mmc2;
pub mod mmc3;
pub mod mmc5;
pub mod namco163;
pub mod nrom;
pub mod opll;
pub mod uxrom;
pub mod vrc6;
pub mod vrc7;
pub mod ym2149;

use crate::cartridge::CartridgeError;

/// Serialised snapshot of a mapper's internal state for save state.
#[derive(serde::Serialize, serde::Deserialize)]
#[allow(clippy::large_enum_variant)]
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
    /// Mapper 5 — MMC5.
    Mmc5(mmc5::Mmc5),
    /// Mapper 9 — MMC2.
    Mmc2(mmc2::Mmc2),
    /// Mapper 24 / 26 — VRC6.
    Vrc6(vrc6::Vrc6),
    /// Mapper 69 — FME-7 / Sunsoft 5B.
    Fme7(fme7::Fme7),
    /// Mapper 85 — VRC7.
    Vrc7(vrc7::Vrc7),
    /// Mapper 19 — Namco 163.
    Namco163(namco163::Namco163),
    /// Mapper 20 — Famicom Disk System (FDS).
    Fds(fds::Fds),
}

impl MapperState {
    /// Convert snapshot back into a boxed `dyn Mapper`.
    pub fn into_boxed_mapper(self) -> Box<dyn Mapper> {
        match self {
            MapperState::Nrom(m) => Box::new(m),
            MapperState::Mmc1(m) => Box::new(m),
            MapperState::Uxrom(m) => Box::new(m),
            MapperState::Cnrom(m) => Box::new(m),
            MapperState::Mmc3(m) => Box::new(m),
            MapperState::Axrom(m) => Box::new(m),
            MapperState::Mmc5(m) => Box::new(m),
            MapperState::Mmc2(m) => Box::new(m),
            MapperState::Vrc6(m) => Box::new(m),
            MapperState::Fme7(m) => Box::new(m),
            MapperState::Vrc7(m) => Box::new(m),
            MapperState::Namco163(m) => Box::new(m),
            MapperState::Fds(m) => Box::new(m),
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
/// PRG addresses are `$6000..=$FFFF` (de-mirrored by bus); CHR are `$0000..=$1FFF`.
pub trait Mapper: Send {
    /// Read a byte from the CPU-side PRG address space (`$6000..=$FFFF`).
    fn read_prg(&self, addr: u16) -> u8;

    /// Read PRG with side effects (FDS etc.); default delegates to `read_prg`.
    fn read_prg_mut(&mut self, addr: u16) -> u8 {
        self.read_prg(addr)
    }

    /// Write a byte to the CPU-side PRG address space (`$6000..=$FFFF`).
    /// For read-only ROM mappers this is usually a no-op (or a register
    /// write for bank-switching boards).
    fn write_prg(&mut self, addr: u16, value: u8);

    /// Read a byte from the PPU-side CHR address space (`$0000..=$1FFF`).
    fn read_chr(&self, addr: u16) -> u8;

    /// Read CHR with side effects (MMC2 latching); default delegates to `read_chr`.
    fn read_chr_latched(&mut self, addr: u16) -> u8 {
        self.read_chr(addr)
    }

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

    /// Whether the mapper is asserting a CPU IRQ. Default: `false`.
    fn irq_pending(&self) -> bool {
        false
    }

    /// Clock IRQ counter (MMC3 A12 rising edge). Default: no-op.
    fn clock_irq(&mut self) {}

    /// Reset per-frame scanline counter (MMC5). Default: no-op.
    fn reset_scanline_counter(&mut self) {}

    /// Advance CPU-clocked logic (FME-7 IRQ etc.). Default: no-op.
    fn clock_cpu(&mut self, _cpu_cycles: u32) {}

    /// Expansion-audio sample `[-1.0, 1.0]` (VRC6/VRC7/etc.). Default: 0.0. See: https://www.nesdev.org/wiki/APU#Expansion_audio
    fn expansion_audio_sample(&self) -> f32 {
        0.0
    }

    /// Return battery-backed PRG-RAM contents, or `None`. See: https://www.nesdev.org/wiki/INES#Flags_6
    fn battery_sram(&self) -> Option<Vec<u8>> {
        None
    }

    /// Load battery-backed PRG-RAM from `.nessram` file. Default: no-op.
    fn load_battery_sram(&mut self, _data: &[u8]) {}

    /// Capture mapper state as a `MapperState` snapshot.
    fn save_state(&self) -> MapperState;

    /// Restore mapper state from a `MapperState` snapshot (panics on mismatch).
    fn restore_state(&mut self, state: MapperState);
}

/// Construct the appropriate mapper for an iNES mapper number.
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
        5 => Ok(Box::new(mmc5::Mmc5::new(
            prg_rom,
            chr_rom,
            mirroring,
            has_battery,
        ))),
        9 => Ok(Box::new(mmc2::Mmc2::new(
            prg_rom,
            chr_rom,
            mirroring,
            has_battery,
        ))),
        24 => Ok(Box::new(vrc6::Vrc6::new(
            prg_rom,
            chr_rom,
            mirroring,
            has_battery,
            false,
        ))),
        26 => Ok(Box::new(vrc6::Vrc6::new(
            prg_rom,
            chr_rom,
            mirroring,
            has_battery,
            true,
        ))),
        69 => Ok(Box::new(fme7::Fme7::new(
            prg_rom,
            chr_rom,
            mirroring,
            has_battery,
        ))),
        85 => Ok(Box::new(vrc7::Vrc7::new(
            prg_rom,
            chr_rom,
            mirroring,
            has_battery,
        ))),
        19 => Ok(Box::new(namco163::Namco163::new(
            prg_rom,
            chr_rom,
            mirroring,
            has_battery,
        ))),
        n => Err(CartridgeError::UnsupportedMapper(n)),
    }
}
