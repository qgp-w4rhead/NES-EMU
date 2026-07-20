//! iNES cartridge loader.
//!
//! Parses the 16-byte iNES header, skips the optional 512-byte trainer, and
//! loads PRG-ROM and CHR-ROM into owned buffers. The actual bank-switching
//! behavior is delegated to a `Mapper` implementation selected by the mapper
//! number extracted from the header.
//!
//! See: https://www.nesdev.org/wiki/INES
//!
//! The cartridge API is wired into the memory bus at M3; until then some
//! accessors are unused at runtime, so we silence dead-code warnings here.
#![allow(dead_code)]

use std::fmt;
use std::path::Path;

use crate::mappers::{from_ines, Mapper, Mirroring};

/// iNES file magic: bytes 0..=3 are `"NES\x1A"`.
const INES_MAGIC: [u8; 4] = [b'N', b'E', b'S', 0x1A];

/// iNES header is always 16 bytes.
const HEADER_SIZE: usize = 16;

/// Optional trainer area size (skipped if present).
const TRAINER_SIZE: usize = 512;

/// PRG-ROM unit size as reported by the iNES header (16 KB blocks).
pub const PRG_ROM_UNIT: usize = 16 * 1024;

/// CHR-ROM unit size as reported by the iNES header (8 KB blocks).
pub const CHR_ROM_UNIT: usize = 8 * 1024;

/// Errors that can occur while loading or constructing a cartridge.
#[derive(Debug)]
pub enum CartridgeError {
    /// File could not be read (path / IO error).
    Io(String),
    /// File is shorter than the 16-byte iNES header.
    TooShort,
    /// First four bytes are not `"NES\x1A"`.
    BadMagic,
    /// Mapper number is not yet implemented.
    UnsupportedMapper(u16),
}

impl fmt::Display for CartridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CartridgeError::Io(msg) => write!(f, "cartridge IO error: {msg}"),
            CartridgeError::TooShort => write!(f, "cartridge too short for iNES header"),
            CartridgeError::BadMagic => write!(f, "iNES magic 'NES\\x1A' not found"),
            CartridgeError::UnsupportedMapper(n) => {
                write!(f, "mapper {n} not yet implemented")
            }
        }
    }
}

impl std::error::Error for CartridgeError {}

impl From<std::io::Error> for CartridgeError {
    fn from(e: std::io::Error) -> Self {
        CartridgeError::Io(e.to_string())
    }
}

/// Parsed iNES header fields relevant to emulation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InesHeader {
    /// Number of 16 KB PRG-ROM banks.
    pub prg_rom_banks: u8,
    /// Number of 8 KB CHR-ROM banks (0 means CHR-RAM).
    pub chr_rom_banks: u8,
    /// Mapper number (low nibble from flags 6, high nibble from flags 7).
    pub mapper_number: u16,
    /// Nametable mirroring mode derived from flags 6.
    pub mirroring: Mirroring,
    /// True if a 512-byte trainer is present between header and PRG-ROM.
    pub has_trainer: bool,
    /// True if the cartridge has battery-backed PRG-RAM.
    pub has_battery: bool,
}

impl InesHeader {
    /// Parse the 16-byte iNES header. Returns `Err` on bad magic or
    /// insufficient length.
    ///
    /// Only the iNES 1.0 fields needed for emulation are extracted; NES 2.0
    /// extensions (header bytes 8-15) are accepted but not interpreted —
    /// later milestones can extend this struct.
    pub fn parse(bytes: &[u8; HEADER_SIZE]) -> Result<Self, CartridgeError> {
        if bytes[..4] != INES_MAGIC {
            return Err(CartridgeError::BadMagic);
        }

        let prg_rom_banks = bytes[4];
        let chr_rom_banks = bytes[5];

        let flags6 = bytes[6];
        let flags7 = bytes[7];

        let has_trainer = (flags6 & 0b0000_0100) != 0;
        let has_battery = (flags6 & 0b0000_0010) != 0;
        let four_screen = (flags6 & 0b0000_1000) != 0;
        let vertical = (flags6 & 0b0000_0001) != 0;

        let mirroring = if four_screen {
            Mirroring::FourScreen
        } else if vertical {
            Mirroring::Vertical
        } else {
            Mirroring::Horizontal
        };

        // Mapper low nibble from flags 6 high nibble, high nibble from
        // flags 7 high nibble. (iNES 1.0 form.)
        let mapper_number = ((flags6 >> 4) as u16) | (((flags7 >> 4) as u16) << 4);

        Ok(Self {
            prg_rom_banks,
            chr_rom_banks,
            mapper_number,
            mirroring,
            has_trainer,
            has_battery,
        })
    }
}

/// A loaded NES cartridge — owns PRG/CHR data and dispatches reads/writes
/// to its `Mapper`.
pub struct Cartridge {
    pub header: InesHeader,
    mapper: Box<dyn Mapper>,
}

impl Cartridge {
    /// Load and parse an iNES file from disk, then build the mapper.
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self, CartridgeError> {
        let bytes = std::fs::read(path.as_ref())?;
        Self::from_bytes(&bytes)
    }

    /// Build a cartridge from a raw iNES file image already in memory.
    pub fn from_bytes(data: &[u8]) -> Result<Self, CartridgeError> {
        if data.len() < HEADER_SIZE {
            return Err(CartridgeError::TooShort);
        }
        let mut header_bytes = [0u8; HEADER_SIZE];
        header_bytes.copy_from_slice(&data[..HEADER_SIZE]);
        let header = InesHeader::parse(&header_bytes)?;

        let prg_size = header.prg_rom_banks as usize * PRG_ROM_UNIT;
        let chr_size = header.chr_rom_banks as usize * CHR_ROM_UNIT;

        let needed =
            HEADER_SIZE + if header.has_trainer { TRAINER_SIZE } else { 0 } + prg_size + chr_size;
        if data.len() < needed {
            return Err(CartridgeError::Io(format!(
                "file truncated: expected {needed} bytes, got {}",
                data.len()
            )));
        }

        let mut offset = HEADER_SIZE;
        if header.has_trainer {
            offset += TRAINER_SIZE;
        }

        let prg_rom = data[offset..offset + prg_size].to_vec();
        offset += prg_size;
        let chr_rom = data[offset..offset + chr_size].to_vec();

        let mapper = from_ines(
            header.mapper_number,
            prg_rom,
            chr_rom,
            header.mirroring,
            header.has_battery,
        )?;

        Ok(Self { header, mapper })
    }

    /// Read a byte from the CPU-side PRG address space (`$6000..=$FFFF`).
    pub fn read_prg(&self, addr: u16) -> u8 {
        self.mapper.read_prg(addr)
    }

    /// Write a byte to the CPU-side PRG address space (`$6000..=$FFFF`).
    pub fn write_prg(&mut self, addr: u16, value: u8) {
        self.mapper.write_prg(addr, value);
    }

    /// Read a byte from the PPU-side CHR address space (`$0000..=$1FFF`).
    pub fn read_chr(&self, addr: u16) -> u8 {
        self.mapper.read_chr(addr)
    }

    /// Write a byte to the PPU-side CHR address space (`$0000..=$1FFF`).
    pub fn write_chr(&mut self, addr: u16, value: u8) {
        self.mapper.write_chr(addr, value);
    }

    /// Current nametable mirroring mode advertised by the cartridge.
    pub fn mirror_mode(&self) -> Mirroring {
        self.mapper.mirror_mode()
    }

    /// Whether the cartridge advertises battery-backed PRG-RAM.
    pub fn has_battery(&self) -> bool {
        self.mapper.has_battery()
    }

    /// Whether the mapper is currently asserting a CPU IRQ. The emulator
    /// main loop polls this to raise `Cpu::irq_pending`.
    pub fn irq_pending(&self) -> bool {
        self.mapper.irq_pending()
    }

    /// Clock the mapper's IRQ counter by one step. Called by the bus when
    /// the PPU A12 line rises during rendering.
    pub fn clock_irq(&mut self) {
        self.mapper.clock_irq();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal in-memory iNES image with the given header fields.
    fn make_ines(prg_banks: u8, chr_banks: u8, flags6: u8, flags7: u8, prg_fill: u8) -> Vec<u8> {
        let prg_size = prg_banks as usize * PRG_ROM_UNIT;
        let chr_size = chr_banks as usize * CHR_ROM_UNIT;
        let mut buf = Vec::with_capacity(HEADER_SIZE + prg_size + chr_size);
        buf.extend_from_slice(&INES_MAGIC);
        buf.push(prg_banks);
        buf.push(chr_banks);
        buf.push(flags6);
        buf.push(flags7);
        // bytes 8..16 — zero padding (iNES 1.0).
        buf.extend_from_slice(&[0u8; 8]);
        buf.resize(HEADER_SIZE + prg_size + chr_size, 0);
        for b in &mut buf[HEADER_SIZE..HEADER_SIZE + prg_size] {
            *b = prg_fill;
        }
        // CHR stays zero-filled.
        buf
    }

    #[test]
    fn parses_valid_header() {
        // NROM-256: 32KB PRG, 8KB CHR, mapper 0, horizontal mirroring.
        let bytes = make_ines(2, 1, 0b0000_0000, 0b0000_0000, 0xAB);
        let cart = Cartridge::from_bytes(&bytes).expect("load");
        assert_eq!(cart.header.mapper_number, 0);
        assert_eq!(cart.header.prg_rom_banks, 2);
        assert_eq!(cart.header.chr_rom_banks, 1);
        assert_eq!(cart.header.mirroring, Mirroring::Horizontal);
        assert!(!cart.header.has_battery);
        assert!(!cart.header.has_trainer);
    }

    #[test]
    fn rejects_bad_magic() {
        let mut bytes = make_ines(1, 1, 0, 0, 0);
        bytes[0] = b'X';
        assert!(matches!(
            Cartridge::from_bytes(&bytes),
            Err(CartridgeError::BadMagic)
        ));
    }

    #[test]
    fn rejects_too_short() {
        let bytes = [0u8; 8];
        assert!(matches!(
            Cartridge::from_bytes(&bytes),
            Err(CartridgeError::TooShort)
        ));
    }

    #[test]
    fn extracts_vertical_mirroring() {
        let bytes = make_ines(1, 1, 0b0000_0001, 0, 0);
        let cart = Cartridge::from_bytes(&bytes).expect("load");
        assert_eq!(cart.header.mirroring, Mirroring::Vertical);
    }

    #[test]
    fn extracts_four_screen_mirroring() {
        let bytes = make_ines(1, 1, 0b0000_1000, 0, 0);
        let cart = Cartridge::from_bytes(&bytes).expect("load");
        assert_eq!(cart.header.mirroring, Mirroring::FourScreen);
    }

    #[test]
    fn extracts_battery_and_trainer_flags() {
        let mut bytes = make_ines(1, 1, 0b0000_0110, 0, 0);
        // Insert trainer between header and PRG.
        let prg = bytes[HEADER_SIZE..].to_vec();
        bytes.truncate(HEADER_SIZE);
        bytes.extend_from_slice(&[0u8; TRAINER_SIZE]);
        bytes.extend_from_slice(&prg);
        let cart = Cartridge::from_bytes(&bytes).expect("load");
        assert!(cart.header.has_battery);
        assert!(cart.header.has_trainer);
    }

    #[test]
    fn extracts_mapper_number_from_high_nibbles() {
        // mapper 0x71: flags6 high nibble = 1, flags7 high nibble = 7.
        // Parse the header directly (the full cartridge load would reject
        // the unimplemented mapper).
        let mut header = [0u8; HEADER_SIZE];
        header[..4].copy_from_slice(&INES_MAGIC);
        header[4] = 1; // prg banks
        header[5] = 1; // chr banks
        header[6] = 0b0001_0000; // mapper low nibble = 1
        header[7] = 0b0111_0000; // mapper high nibble = 7
        let parsed = InesHeader::parse(&header).expect("parse");
        assert_eq!(parsed.mapper_number, 0x71);
    }

    #[test]
    fn unsupported_mapper_returns_error() {
        // mapper 5 (MMC5) — not yet implemented.
        let bytes = make_ines(1, 1, 0b0101_0000, 0, 0);
        assert!(matches!(
            Cartridge::from_bytes(&bytes),
            Err(CartridgeError::UnsupportedMapper(5))
        ));
    }

    #[test]
    fn nrom_32k_prg_reads_correct_data() {
        // 32KB PRG filled with 0xAB, 8KB CHR zero-filled, mapper 0.
        let bytes = make_ines(2, 1, 0, 0, 0xAB);
        let cart = Cartridge::from_bytes(&bytes).expect("load");
        // $8000 and $C000 should both read 0xAB (no mirroring needed for 32K).
        assert_eq!(cart.read_prg(0x8000), 0xAB);
        assert_eq!(cart.read_prg(0xC000), 0xAB);
        assert_eq!(cart.read_prg(0xFFFF), 0xAB);
    }

    #[test]
    fn nrom_16k_prg_mirrors_high_half() {
        // 16KB PRG: fill first half with 0x11, second half (which is the
        // only bank) with 0x22 by writing distinct bytes per 4KB chunk.
        let mut bytes = make_ines(1, 0, 0, 0, 0);
        // PRG is one 16KB bank starting at HEADER_SIZE.
        let prg_off = HEADER_SIZE;
        for i in 0..PRG_ROM_UNIT {
            bytes[prg_off + i] = (i & 0xFF) as u8;
        }
        let cart = Cartridge::from_bytes(&bytes).expect("load");
        // $8000 maps to PRG offset 0; $C000 mirrors the same 16KB bank.
        assert_eq!(cart.read_prg(0x8000), 0x00);
        assert_eq!(cart.read_prg(0x8001), 0x01);
        assert_eq!(cart.read_prg(0xC000), 0x00);
        assert_eq!(cart.read_prg(0xC001), 0x01);
        assert_eq!(cart.read_prg(0xFFFF), 0xFF);
    }

    #[test]
    fn nrom_chr_reads_return_chr_rom_data() {
        // 16KB PRG, 8KB CHR with a recognizable pattern.
        let mut bytes = make_ines(1, 1, 0, 0, 0);
        let chr_off = HEADER_SIZE + PRG_ROM_UNIT;
        for i in 0..CHR_ROM_UNIT {
            bytes[chr_off + i] = (i as u8).wrapping_add(0x40);
        }
        let cart = Cartridge::from_bytes(&bytes).expect("load");
        assert_eq!(cart.read_chr(0x0000), 0x40);
        assert_eq!(cart.read_chr(0x0001), 0x41);
        assert_eq!(
            cart.read_chr(0x1FFF),
            ((0x1FFFu16).wrapping_add(0x40) & 0xFF) as u8
        );
    }

    #[test]
    fn nrom_with_zero_chr_banks_uses_chr_ram() {
        // chr_rom_banks = 0 → CHR-RAM; writes should stick and reads return
        // the written value.
        let bytes = make_ines(1, 0, 0, 0, 0);
        let mut cart = Cartridge::from_bytes(&bytes).expect("load");
        assert_eq!(cart.read_chr(0x0000), 0);
        cart.write_chr(0x0000, 0x77);
        assert_eq!(cart.read_chr(0x0000), 0x77);
    }

    #[test]
    fn nrom_prg_writes_are_ignored_for_rom() {
        // NROM has no PRG-RAM at $6000-$7FFF on stock boards; writes should
        // be silently dropped and reads return 0 (open bus filler) or ROM
        // data depending on address. We only assert it does not panic.
        let bytes = make_ines(1, 1, 0, 0, 0);
        let mut cart = Cartridge::from_bytes(&bytes).expect("load");
        cart.write_prg(0x8000, 0xFF);
        // Read should still return ROM data, not the written value.
        assert_ne!(cart.read_prg(0x8000), 0xFF);
    }
}
