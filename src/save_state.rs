//! Save state — serialisation and deserialisation of the full emulator state.
//!
//! The entire `EmulatorState` (CPU registers + RAM, PPU state + VRAM + OAM +
//! palette, APU state, Bus state, Mapper state, cycle counters) is captured
//! into a [`SaveState`] struct, which derives `Serialize`/`Deserialize` and
//! is encoded with [`bincode`] for a compact binary `.nessav` file.
//!
//! A [`SAVE_STATE_VERSION`] constant is embedded in every save state. On
//! load, the version is checked — a mismatch returns an error so that
//! forward-incompatible save states are rejected cleanly rather than
//! deserialising into a corrupt state.
//!
//! # What is serialised
//!
//! | Component          | Fields                                                  |
//! |--------------------|---------------------------------------------------------|
//! | CPU                | A, X, Y, SP, PC, status, nmi_pending, irq_pending       |
//! | Bus RAM            | 2 KB internal RAM                                       |
//! | PPU                | All registers, VRAM, OAM, palette, scroll, scanline/cycle |
//! | APU open bus       | 24-byte register latch array                            |
//! | APU                | All 5 channels + frame counter state                    |
//! | Joypad             | Strobe, shift registers, read counters (live buttons skipped) |
//! | Cartridge          | iNES header + full mapper state (PRG/CHR, banks, PRG-RAM, IRQ) |
//! | Bus DMA stall      | Pending OAM-DMA stall cycles                            |
//! | Emulator           | Audio sample accumulator + audio buffer                 |
//!
//! # What is NOT serialised
//!
//! - **PPU framebuffer** (`256×240` ARGB) — derived data, recomputed on the
//!   next `render_frame` call after a state restore.
//! - **PPU bg_pattern buffer** — derived data, recomputed during rendering.
//! - **Joypad live button state** — host input, re-poled each frame by the
//!   input layer; restored to "no buttons pressed" on load.
//!
//! See: https://www.nesdev.org/wiki/Save_state

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// Serde helper for serialising large `u8` arrays (length > 32, which serde
/// does not support out of the box). Serialises the array as a `Vec<u8>` and
/// deserialises it back, checking the length matches `N`.
///
/// Used via `#[serde(with = "array_ser")]` on fields like `vram: [u8; 4096]`.
pub mod array_ser {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    /// Serialise a `[u8; N]` as a `Vec<u8>`.
    pub fn serialize<S, const N: usize>(arr: &[u8; N], s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        arr.to_vec().serialize(s)
    }

    /// Deserialise a `Vec<u8>` and convert it back to a `[u8; N]`, verifying
    /// the length.
    pub fn deserialize<'de, D, const N: usize>(d: D) -> Result<[u8; N], D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = Vec::<u8>::deserialize(d)?;
        v.try_into()
            .map_err(|_| serde::de::Error::custom(format!("expected array of length {N}")))
    }
}

use crate::apu::Apu;
use crate::bus::{APU_IO_REG_COUNT, RAM_SIZE};
use crate::cartridge::{Cartridge, InesHeader};
use crate::cpu::Cpu;
use crate::emulator::EmulatorState;
use crate::joypad::Joypad;
use crate::mappers::MapperState;
use crate::ppu::Ppu;

/// Save state format version. Increment this when the `SaveState` struct
/// layout changes in a backward-incompatible way. On load, a version
/// mismatch returns [`SaveStateError::VersionMismatch`] rather than
/// attempting to deserialise into a potentially incompatible layout.
pub const SAVE_STATE_VERSION: u32 = 1;

/// Errors that can occur during save state serialisation or deserialisation.
#[derive(Debug)]
pub enum SaveStateError {
    /// Serialisation failed (bincode encode error).
    Encode(String),
    /// Deserialisation failed (bincode decode error — corrupt or truncated
    /// save state data).
    Decode(String),
    /// The save state was produced by a different, incompatible version of
    /// the emulator. The stored version and the current
    /// [`SAVE_STATE_VERSION`] are included.
    VersionMismatch { expected: u32, found: u32 },
}

impl std::fmt::Display for SaveStateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveStateError::Encode(msg) => write!(f, "save state encode error: {msg}"),
            SaveStateError::Decode(msg) => write!(f, "save state decode error: {msg}"),
            SaveStateError::VersionMismatch { expected, found } => {
                write!(
                    f,
                    "save state version mismatch: expected {expected}, found {found}"
                )
            }
        }
    }
}

impl std::error::Error for SaveStateError {}

/// Serialised snapshot of a cartridge — the iNES header (metadata) plus the
/// full mapper state (PRG/CHR data, bank registers, PRG-RAM, IRQ state).
#[derive(Serialize, Deserialize)]
pub struct CartridgeSnapshot {
    /// Parsed iNES header (mapper number, mirroring, battery flag, etc.).
    pub header: InesHeader,
    /// Full mapper state — one variant per implemented mapper type.
    pub mapper_state: MapperState,
}

/// The complete serialisable emulator state.
///
/// Every field that constitutes the emulated machine's "current execution
/// state" is captured here. Derived data (framebuffer, bg_pattern, live
/// joypad button state) is excluded — see the [module docs](self) for
/// details.
#[derive(Serialize, Deserialize)]
pub struct SaveState {
    /// Format version — must match [`SAVE_STATE_VERSION`] on load.
    pub version: u32,
    /// 6502 CPU registers + interrupt pending flags.
    pub cpu: Cpu,
    /// 2 KB internal CPU RAM (`$0000-$07FF`).
    pub ram: Vec<u8>,
    /// PPU state (registers, VRAM, OAM, palette, scroll, scanline/cycle).
    pub ppu: Ppu,
    /// APU / I/O open-bus latch array (24 bytes).
    pub apu_open_bus: [u8; APU_IO_REG_COUNT],
    /// APU state (5 channels + frame counter).
    pub apu: Apu,
    /// Joypad state (strobe, shift registers, read counters).
    pub joypad: Joypad,
    /// Loaded cartridge, if any (header + full mapper state).
    pub cartridge: Option<CartridgeSnapshot>,
    /// Pending OAM-DMA stall cycles.
    pub dma_stall_cycles: u32,
    /// Audio sample accumulator (fractional CPU cycles toward next sample).
    pub sample_accumulator: f32,
    /// Audio samples produced during the current frame (not yet drained).
    pub audio_buffer: Vec<f32>,
}

impl EmulatorState {
    /// Serialise the full emulator state into a compact binary `.nessav`
    /// blob using [`bincode`].
    ///
    /// The returned `Vec<u8>` can be written to a file, stored in memory,
    /// or passed to [`EmulatorState::load_state`] to restore the exact
    /// execution state.
    pub fn save_state(&self) -> Result<Vec<u8>, SaveStateError> {
        let bus = self.bus();
        let cartridge = bus.cartridge().map(|cart| CartridgeSnapshot {
            header: cart.header.clone(),
            mapper_state: cart.save_mapper_state(),
        });

        let state = SaveState {
            version: SAVE_STATE_VERSION,
            cpu: self.cpu().clone(),
            ram: bus.ram().to_vec(),
            ppu: bus.ppu().clone(),
            apu_open_bus: *bus.apu_open_bus(),
            apu: bus.apu().clone(),
            joypad: bus.joypad().clone(),
            cartridge,
            // DMA stall cycles are normally 0 outside step_frame's inner
            // loop (set and consumed within the same iteration), but we
            // serialise the actual bus value for completeness — a
            // mid-frame save could theoretically capture a non-zero value.
            dma_stall_cycles: self.bus().dma_stall_cycles(),
            sample_accumulator: self.sample_accumulator(),
            audio_buffer: self.audio_buffer().to_vec(),
        };

        bincode::serialize(&state).map_err(|e| SaveStateError::Encode(e.to_string()))
    }

    /// Deserialise a `.nessav` blob and restore the full emulator state.
    ///
    /// The save state's version is checked against [`SAVE_STATE_VERSION`];
    /// a mismatch returns [`SaveStateError::VersionMismatch`]. After
    /// deserialisation, every component (CPU, RAM, PPU, APU, joypad,
    /// cartridge, DMA stall, audio accumulator/buffer) is restored in
    /// place — the emulator resumes exactly where it left off.
    ///
    /// The PPU framebuffer and bg_pattern buffer are re-allocated to their
    /// full size (they are skipped during serialisation as derived data);
    /// the next `render_frame` call will fill them.
    pub fn load_state(&mut self, data: &[u8]) -> Result<(), SaveStateError> {
        let state: SaveState =
            bincode::deserialize(data).map_err(|e| SaveStateError::Decode(e.to_string()))?;

        if state.version != SAVE_STATE_VERSION {
            return Err(SaveStateError::VersionMismatch {
                expected: SAVE_STATE_VERSION,
                found: state.version,
            });
        }

        // ---- CPU ----
        *self.cpu_mut() = state.cpu;

        // ---- Bus components ----
        // Restore RAM from the Vec, verifying the length matches.
        let ram_len = state.ram.len();
        if ram_len != RAM_SIZE {
            return Err(SaveStateError::Decode(format!(
                "RAM length mismatch: expected {RAM_SIZE}, got {ram_len}"
            )));
        }
        self.bus_mut().ram_mut().copy_from_slice(&state.ram);
        *self.bus_mut().ppu_mut() = state.ppu;
        *self.bus_mut().apu_open_bus_mut() = state.apu_open_bus;
        *self.bus_mut().apu_mut() = state.apu;
        *self.bus_mut().joypad_mut() = state.joypad;
        self.bus_mut().set_dma_stall_cycles(state.dma_stall_cycles);

        // ---- Cartridge ----
        // Rebuild the cartridge from the snapshot (header + mapper state)
        // and insert it into the bus. The PPU mirroring is re-synced from
        // the restored cartridge.
        if let Some(cart_snap) = state.cartridge {
            let cart = Cartridge::from_header_and_mapper(cart_snap.header, cart_snap.mapper_state);
            self.bus_mut().insert_cartridge(cart);
        } else {
            // No cartridge in the save state — remove any loaded cartridge.
            self.bus_mut().remove_cartridge();
        }

        // ---- Emulator-level state ----
        self.set_sample_accumulator(state.sample_accumulator);
        self.set_audio_buffer(state.audio_buffer);

        Ok(())
    }
}
