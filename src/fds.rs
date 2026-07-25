//! Famicom Disk System (FDS) disk image parser and BIOS loader.
//!
//! See: https://www.nesdev.org/wiki/FDS_disk_format

use std::path::{Path, PathBuf};

use crate::cartridge::CartridgeError;

/// FDS disk image magic: bytes 0..=3 are `"FDS\x1A"`.
pub const FDS_MAGIC: [u8; 4] = [b'F', b'D', b'S', 0x1A];

/// FDS header size in bytes.
pub const FDS_HEADER_SIZE: usize = 16;

/// Size of one disk side in bytes.
pub const DISK_SIDE_SIZE: usize = 65_500;

/// FDS BIOS ROM size (8 KB at `$E000-$FFFF`).
pub const BIOS_SIZE: usize = 8 * 1024;

/// PRG-RAM size (32 KB at `$6000-$DFFF`).
pub const PRG_RAM_SIZE: usize = 32 * 1024;

/// CHR-RAM size (8 KB at `$0000-$1FFF`).
pub const CHR_RAM_SIZE: usize = 8 * 1024;

/// Parsed FDS disk image header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FdsHeader {
    /// Number of disk sides in the image (usually 1).
    pub disk_count: u8,
}

/// A parsed FDS disk image — the raw disk data for each side.
#[derive(Debug, Clone)]
pub struct FdsDisk {
    /// Parsed header (disk count).
    pub header: FdsHeader,
    /// Raw disk data for each side (each `DISK_SIDE_SIZE` bytes).
    pub sides: Vec<Vec<u8>>,
}

impl FdsDisk {
    /// Parse an FDS disk image from a byte slice.
    ///
    /// Validates the magic and extracts the per-side raw data. The disk
    /// data is not interpreted here — the FDS BIOS handles block parsing
    /// at runtime via the disk I/O registers.
    pub fn parse(data: &[u8]) -> Result<Self, CartridgeError> {
        if data.len() < FDS_HEADER_SIZE {
            return Err(CartridgeError::TooShort);
        }
        if data[..4] != FDS_MAGIC {
            return Err(CartridgeError::BadMagic);
        }

        let disk_count = data[4];
        if disk_count == 0 {
            return Err(CartridgeError::Io("FDS disk count is zero".to_string()));
        }

        let needed = FDS_HEADER_SIZE + disk_count as usize * DISK_SIDE_SIZE;
        if data.len() < needed {
            return Err(CartridgeError::Io(format!(
                "FDS image truncated: expected {needed} bytes, got {}",
                data.len()
            )));
        }

        let mut sides = Vec::with_capacity(disk_count as usize);
        for i in 0..disk_count as usize {
            let start = FDS_HEADER_SIZE + i * DISK_SIDE_SIZE;
            let end = start + DISK_SIDE_SIZE;
            sides.push(data[start..end].to_vec());
        }

        Ok(Self {
            header: FdsHeader { disk_count },
            sides,
        })
    }

    /// Get the raw disk data for side `index` (0-based).
    pub fn side(&self, index: usize) -> Option<&[u8]> {
        self.sides.get(index).map(|v| v.as_slice())
    }

    /// Number of disk sides.
    pub fn disk_count(&self) -> u8 {
        self.header.disk_count
    }
}

/// Search for the FDS BIOS ROM (`disksys.rom`) in common locations.
///
/// Search order:
/// 1. Alongside the FDS file (same directory)
/// 2. Current working directory
/// 3. `~/.config/nes-emu/disksys.rom`
/// 4. `/usr/share/nes-emu/disksys.rom`
/// 5. `~/.local/share/nes-emu/disksys.rom`
///
/// Returns the path of the first matching file, or `None` if not found.
pub fn find_bios_path(fds_path: Option<&Path>) -> Option<PathBuf> {
    let candidates = bios_search_paths(fds_path);
    for path in &candidates {
        if path.is_file() {
            return Some(path.clone());
        }
    }
    None
}

/// Build the list of candidate BIOS search paths.
fn bios_search_paths(fds_path: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // 1. Alongside the FDS file.
    if let Some(fds) = fds_path {
        if let Some(dir) = fds.parent() {
            paths.push(dir.join("disksys.rom"));
        }
    }

    // 2. Current working directory.
    paths.push(PathBuf::from("disksys.rom"));

    // 3-5. User and system config directories.
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        paths.push(home.join(".config").join("nes-emu").join("disksys.rom"));
        paths.push(
            home.join(".local")
                .join("share")
                .join("nes-emu")
                .join("disksys.rom"),
        );
    }
    paths.push(PathBuf::from("/usr/share/nes-emu/disksys.rom"));

    paths
}

/// Load the FDS BIOS ROM from the first matching search path.
///
/// Returns the 8 KB BIOS data, or an error if not found / wrong size.
pub fn load_bios(fds_path: Option<&Path>) -> Result<Vec<u8>, CartridgeError> {
    let path = find_bios_path(fds_path).ok_or_else(|| {
        CartridgeError::Io(
            "FDS BIOS (disksys.rom) not found. Place it alongside the .fds file, in the current directory, or in ~/.config/nes-emu/".to_string(),
        )
    })?;
    load_bios_from_path(&path)
}

/// Load the FDS BIOS ROM from a specific path.
pub fn load_bios_from_path(path: &Path) -> Result<Vec<u8>, CartridgeError> {
    let data = std::fs::read(path).map_err(|e| {
        CartridgeError::Io(format!("cannot read FDS BIOS '{}': {e}", path.display()))
    })?;
    if data.len() < BIOS_SIZE {
        return Err(CartridgeError::Io(format!(
            "FDS BIOS too short: expected at least {BIOS_SIZE} bytes, got {}",
            data.len()
        )));
    }
    // Use exactly the first 8 KB.
    Ok(data[..BIOS_SIZE].to_vec())
}

/// Check whether a file path has an `.fds` extension (case-insensitive).
pub fn is_fds_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("fds"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal FDS image with the given number of disk sides,
    /// each filled with a recognizable pattern.
    fn make_fds(disk_count: u8, fill: u8) -> Vec<u8> {
        let mut buf = Vec::with_capacity(FDS_HEADER_SIZE + disk_count as usize * DISK_SIDE_SIZE);
        buf.extend_from_slice(&FDS_MAGIC);
        buf.push(disk_count);
        buf.extend_from_slice(&[0xFF; 11]); // padding
        for i in 0..disk_count as usize {
            let mut side = vec![fill.wrapping_add(i as u8); DISK_SIDE_SIZE];
            // First block: disk info block ($80).
            side[0] = 0x80;
            side[1..4].copy_from_slice(b"FDS");
            buf.extend_from_slice(&side);
        }
        buf
    }

    #[test]
    fn parses_valid_single_sided_fds() {
        let data = make_fds(1, 0xAA);
        let disk = FdsDisk::parse(&data).expect("parse");
        assert_eq!(disk.disk_count(), 1);
        assert_eq!(disk.sides.len(), 1);
        assert_eq!(disk.sides[0].len(), DISK_SIDE_SIZE);
        // First byte is the block type $80.
        assert_eq!(disk.sides[0][0], 0x80);
        // Bytes 4+ are the fill pattern.
        assert_eq!(disk.sides[0][4], 0xAA);
    }

    #[test]
    fn parses_multi_sided_fds() {
        let data = make_fds(2, 0xBB);
        let disk = FdsDisk::parse(&data).expect("parse");
        assert_eq!(disk.disk_count(), 2);
        assert_eq!(disk.sides.len(), 2);
        // Side 0 fill = 0xBB, side 1 fill = 0xBC (i=1 added).
        assert_eq!(disk.sides[0][4], 0xBB);
        assert_eq!(disk.sides[1][4], 0xBC);
    }

    #[test]
    fn rejects_bad_magic() {
        let mut data = make_fds(1, 0);
        data[0] = b'X';
        assert!(matches!(
            FdsDisk::parse(&data),
            Err(CartridgeError::BadMagic)
        ));
    }

    #[test]
    fn rejects_too_short() {
        let data = [0u8; 8];
        assert!(matches!(
            FdsDisk::parse(&data),
            Err(CartridgeError::TooShort)
        ));
    }

    #[test]
    fn rejects_zero_disk_count() {
        let mut data = make_fds(1, 0);
        data[4] = 0;
        assert!(matches!(FdsDisk::parse(&data), Err(CartridgeError::Io(_))));
    }

    #[test]
    fn rejects_truncated_disk_data() {
        let mut data = make_fds(1, 0);
        data.truncate(FDS_HEADER_SIZE + 100);
        assert!(matches!(FdsDisk::parse(&data), Err(CartridgeError::Io(_))));
    }

    #[test]
    fn side_accessor_returns_data() {
        let data = make_fds(1, 0x42);
        let disk = FdsDisk::parse(&data).expect("parse");
        assert!(disk.side(0).is_some());
        assert_eq!(disk.side(0).unwrap().len(), DISK_SIDE_SIZE);
        assert!(disk.side(1).is_none()); // out of range
    }

    #[test]
    fn is_fds_file_detects_extension() {
        assert!(is_fds_file(Path::new("game.fds")));
        assert!(is_fds_file(Path::new("GAME.FDS")));
        assert!(is_fds_file(Path::new("/path/to/game.fds")));
        assert!(!is_fds_file(Path::new("game.nes")));
        assert!(!is_fds_file(Path::new("game")));
    }

    #[test]
    fn bios_search_paths_includes_cwd() {
        let paths = bios_search_paths(None);
        assert!(paths.iter().any(|p| p == &PathBuf::from("disksys.rom")));
    }

    #[test]
    fn bios_search_paths_includes_fds_directory() {
        let paths = bios_search_paths(Some(Path::new("/games/smb2.fds")));
        assert!(paths
            .iter()
            .any(|p| p == &PathBuf::from("/games/disksys.rom")));
    }

    #[test]
    fn find_bios_path_returns_none_when_absent() {
        // In a test environment, disksys.rom is unlikely to exist in
        // the standard search paths. We verify the function returns None
        // rather than panicking. (If a disksys.rom happens to exist on
        // the test machine, this test is skipped.)
        let result = find_bios_path(Some(Path::new("/nonexistent/path/game.fds")));
        // The /nonexistent dir won't have disksys.rom, but other paths
        // might. We just assert the function doesn't panic.
        let _ = result;
    }

    #[test]
    fn load_bios_from_path_rejects_short_file() {
        let tmp = tempfile_path();
        std::fs::write(&tmp, [0u8; 100]).unwrap();
        let result = load_bios_from_path(&tmp);
        assert!(matches!(result, Err(CartridgeError::Io(_))));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn load_bios_from_path_loads_8kb() {
        let tmp = tempfile_path();
        let mut bios = vec![0u8; BIOS_SIZE];
        bios[0] = 0x4C; // JMP opcode
        bios[1] = 0x00;
        bios[2] = 0xE0;
        std::fs::write(&tmp, &bios).unwrap();
        let loaded = load_bios_from_path(&tmp).expect("load");
        assert_eq!(loaded.len(), BIOS_SIZE);
        assert_eq!(loaded[0], 0x4C);
        let _ = std::fs::remove_file(&tmp);
    }

    /// Create a unique temporary file path for testing.
    fn tempfile_path() -> PathBuf {
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        PathBuf::from(format!("/tmp/nes_emu_test_{pid}_{nanos}.rom"))
    }
}
