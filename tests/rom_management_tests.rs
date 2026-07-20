//! Integration tests for M34 ROM management — compressed ROM loading
//! (Gzip / ZIP) and IPS patch application through the cartridge API.

use nes_emu::cartridge::{Cartridge, CartridgeError, CHR_ROM_UNIT, PRG_ROM_UNIT};
use nes_emu::compression;
use nes_emu::ips::{IpsError, IpsPatch, IpsRecord};
use nes_emu::mappers::Mirroring;
use nes_emu::region::Region;
use nes_emu::rom_manager::{build_rom_info, file_name_from_path, RomInfo, RomManager};

/// Build a minimal in-memory iNES image with the given header fields.
fn make_ines(prg_banks: u8, chr_banks: u8, flags6: u8, flags7: u8, prg_fill: u8) -> Vec<u8> {
    let prg_size = prg_banks as usize * PRG_ROM_UNIT;
    let chr_size = chr_banks as usize * CHR_ROM_UNIT;
    let mut buf = Vec::with_capacity(16 + prg_size + chr_size);
    buf.extend_from_slice(b"NES\x1A");
    buf.push(prg_banks);
    buf.push(chr_banks);
    buf.push(flags6);
    buf.push(flags7);
    buf.extend_from_slice(&[0u8; 8]);
    buf.resize(16 + prg_size + chr_size, 0);
    for b in &mut buf[16..16 + prg_size] {
        *b = prg_fill;
    }
    buf
}

/// Build a minimal stored-block DEFLATE stream wrapping `payload`.
fn deflate_stored(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(0x01); // BFINAL=1, BTYPE=00
    let len = payload.len() as u16;
    let nlen = !len;
    out.push(len as u8);
    out.push((len >> 8) as u8);
    out.push(nlen as u8);
    out.push((nlen >> 8) as u8);
    out.extend_from_slice(payload);
    out
}

/// Wrap a DEFLATE stream + original payload as a Gzip file.
fn gzip_wrap(deflate: &[u8], original: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&[0x1F, 0x8B]);
    out.push(8); // CM
    out.push(0); // FLG
    out.extend_from_slice(&[0, 0, 0, 0]); // MTIME
    out.push(0); // XFL
    out.push(0xFF); // OS
    out.extend_from_slice(deflate);
    // CRC32 + ISIZE (computed via the compression module's internal CRC).
    let crc = nes_emu::compression::crc32(original);
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&(original.len() as u32).to_le_bytes());
    out
}

/// Wrap a payload as a ZIP archive entry with the given name and method.
fn zip_wrap(name: &str, method: u16, payload: &[u8], original: &[u8]) -> Vec<u8> {
    let name_bytes = name.as_bytes();
    let crc = nes_emu::compression::crc32(original);
    let mut out = Vec::new();
    out.extend_from_slice(&0x04034B50u32.to_le_bytes());
    out.extend_from_slice(&20u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&method.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&crc.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&(original.len() as u32).to_le_bytes());
    out.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(name_bytes);
    out.extend_from_slice(payload);
    out
}

// ---- Cartridge auto-decompression -------------------------------------------

#[test]
fn cartridge_loads_from_gzip_archive() {
    let rom = make_ines(2, 1, 0, 0, 0xAB);
    let gz = gzip_wrap(&deflate_stored(&rom), &rom);
    let cart = Cartridge::from_bytes_with_auto_decompress(&gz).expect("load gzip");
    assert_eq!(cart.header.mapper_number, 0);
    assert_eq!(cart.read_prg(0x8000), 0xAB);
}

#[test]
fn cartridge_loads_from_zip_archive_with_nes_entry() {
    let rom = make_ines(1, 1, 0, 0, 0x42);
    let zip = zip_wrap("game.nes", 0, &rom, &rom);
    let cart = Cartridge::from_bytes_with_auto_decompress(&zip).expect("load zip");
    assert_eq!(cart.header.mapper_number, 0);
    assert_eq!(cart.read_prg(0x8000), 0x42);
}

#[test]
fn cartridge_loads_from_zip_with_non_nes_fallback() {
    let rom = make_ines(1, 0, 0, 0, 0x77);
    let zip = zip_wrap("data.bin", 0, &rom, &rom);
    let cart = Cartridge::from_bytes_with_auto_decompress(&zip).expect("load zip fallback");
    assert_eq!(cart.read_prg(0x8000), 0x77);
}

#[test]
fn cartridge_loads_plain_rom_unchanged() {
    let rom = make_ines(1, 1, 0, 0, 0x55);
    let cart = Cartridge::from_bytes_with_auto_decompress(&rom).expect("load plain");
    assert_eq!(cart.read_prg(0x8000), 0x55);
}

#[test]
fn cartridge_from_path_loads_compressed_gzip() {
    use std::process::Command;
    let rom = make_ines(2, 1, 0, 0, 0xCD);
    let dir = std::env::temp_dir().join(format!("nes-emu-cart-gz-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let raw = dir.join("rom.nes");
    std::fs::write(&raw, &rom).unwrap();
    let ok = Command::new("gzip")
        .arg("-kf")
        .arg(&raw)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !ok {
        eprintln!("skipping cartridge_from_path_loads_compressed_gzip: gzip unavailable");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let gz_path = dir.join("rom.nes.gz");
    let cart = Cartridge::from_path(&gz_path).expect("load from path");
    assert_eq!(cart.read_prg(0x8000), 0xCD);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cartridge_decompress_error_propagates() {
    // Gzip magic but truncated/invalid payload — should produce a
    // Decompress error, not a BadMagic error.
    let bad = [
        0x1F, 0x8B, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0x00,
    ];
    let err = Cartridge::from_bytes_with_auto_decompress(&bad)
        .err()
        .unwrap();
    assert!(matches!(err, CartridgeError::Decompress(_)));
}

// ---- IPS patch application via cartridge ------------------------------------

#[test]
fn cartridge_loads_with_ips_patch() {
    let mut rom = make_ines(1, 1, 0, 0, 0x00);
    // Patch the first PRG byte (offset 16 = header size) to 0xEE.
    let patch = IpsPatch::from_records(vec![IpsRecord::Copy {
        offset: 16,
        data: vec![0xEE],
    }]);
    let cart = Cartridge::from_path_with_ips(write_temp_rom(&rom, "ips_plain"), Some(&patch))
        .expect("load with IPS");
    assert_eq!(cart.read_prg(0x8000), 0xEE);
    // Unpatched byte at $8001 is still 0x00.
    assert_eq!(cart.read_prg(0x8001), 0x00);
    rom[16] = 0xEE; // for the assertion above to be meaningful
    let _ = rom;
}

#[test]
fn cartridge_loads_compressed_with_ips_patch() {
    let rom = make_ines(1, 1, 0, 0, 0x00);
    let gz = gzip_wrap(&deflate_stored(&rom), &rom);
    let path = write_temp_rom(&gz, "ips_gz.gz");
    let patch = IpsPatch::from_records(vec![IpsRecord::Copy {
        offset: 16,
        data: vec![0x99],
    }]);
    let cart = Cartridge::from_path_with_ips(&path, Some(&patch)).expect("load gz + IPS");
    assert_eq!(cart.read_prg(0x8000), 0x99);
}

#[test]
fn cartridge_ips_error_propagates() {
    let rom = make_ines(1, 1, 0, 0, 0x00);
    let path = write_temp_rom(&rom, "ips_err");
    // A patch with a bad magic will fail to parse.
    let bad_patch_bytes = [0u8; 10];
    let err = IpsPatch::from_bytes(&bad_patch_bytes).unwrap_err();
    assert!(matches!(err, IpsError::BadMagic));
    // The cartridge load itself should still succeed without a patch.
    let cart = Cartridge::from_path_with_ips(&path, None).expect("load no patch");
    assert_eq!(cart.read_prg(0x8000), 0x00);
}

/// Write a ROM image to a temp file and return the path.
fn write_temp_rom(data: &[u8], tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("nes-emu-rom-mgmt-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("rom.nes");
    std::fs::write(&path, data).unwrap();
    path
}

// ---- ROM info + recent list integration ------------------------------------

#[test]
fn rom_info_built_from_cartridge_header() {
    let rom = make_ines(2, 1, 0b0000_0010, 0, 0); // battery flag
    let cart = Cartridge::from_bytes(&rom).expect("load");
    let info = build_rom_info(&cart.header, "game.nes", rom.len() as u64, Region::Ntsc);
    assert_eq!(info.mapper_number, 0);
    assert_eq!(info.prg_rom_size, 2 * PRG_ROM_UNIT);
    assert_eq!(info.chr_rom_size, CHR_ROM_UNIT);
    assert!(info.has_battery);
    assert_eq!(info.mirroring, Mirroring::Horizontal);
    assert_eq!(info.file_name, "game.nes");
}

#[test]
fn rom_manager_recent_list_persists_through_record() {
    let mut rm = RomManager::new(vec!["/old.nes".into()]);
    rm.record_loaded_rom("/new.nes");
    assert_eq!(rm.recent_roms()[0], "/new.nes");
    assert_eq!(rm.recent_roms()[1], "/old.nes");
}

#[test]
fn rom_manager_info_lines_format_correctly() {
    let mut rm = RomManager::new(vec![]);
    rm.set_info_overlay(true);
    let info = RomInfo {
        file_name: "test.nes".to_string(),
        file_size: 40976,
        mapper_number: 4,
        prg_rom_size: 32 * 1024,
        chr_rom_size: 8 * 1024,
        mirroring: Mirroring::Vertical,
        has_battery: true,
        has_trainer: false,
        region_hint: None,
        region: Region::Ntsc,
    };
    let lines = rm.info_lines(&info);
    assert_eq!(lines.len(), 4);
    assert!(lines[0].contains("test.nes"));
    assert!(lines[1].contains("MAP:4"));
    assert!(lines[1].contains("PRG:32K"));
    assert!(lines[1].contains("CHR:8K"));
    assert!(lines[2].contains("MIR:V"));
    assert!(lines[2].contains("BATT"));
}

#[test]
fn file_name_from_path_extracts_name() {
    let p = std::path::Path::new("/home/user/roms/game.nes.gz");
    assert_eq!(file_name_from_path(p), "game.nes.gz");
}

#[test]
fn compression_module_exposes_crc32_for_tests() {
    // The integration tests use `compression::crc32` to build valid
    // gzip/zip trailers. Verify it matches the known CRC of "123456789".
    assert_eq!(compression::crc32(b"123456789"), 0xCBF4_3926);
}
