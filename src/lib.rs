//! NES emulator library — hand-written 6502 CPU, PPU, APU, memory bus,
//! cartridge mappers, and SDL2 video/audio/input.
//!
//! This crate exposes the emulation core as a library so that integration
//! tests (under `tests/`) and the binary entry point (`src/main.rs`) can
//! share the same modules.

pub mod apu;
pub mod audio;
pub mod battery;
pub mod bus;
pub mod cartridge;
pub mod config;
pub mod cpu;
pub mod debug;
pub mod emulator;
pub mod input;
pub mod joypad;
pub mod mappers;
pub mod ppu;
pub mod save_state;
pub mod video;
