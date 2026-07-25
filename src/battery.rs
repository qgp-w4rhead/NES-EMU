//! Battery-backed PRG-RAM persistence — saves/loads `.nessram` sidecar files.
//!
//! See: https://www.nesdev.org/wiki/INES#Flags_6

#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// File extension used for battery-backed PRG-RAM sidecar files.
pub const SRAM_EXTENSION: &str = "nessram";

/// Compute the `.nessram` sidecar path for a given ROM path.
///
/// The sidecar file lives in the same directory as the ROM and shares its
/// stem, with the extension replaced by `nessram`. If the ROM has no
/// extension, `.nessram` is appended.
///
/// ```
/// use nes_emu::battery::sram_path_for_rom;
/// use std::path::PathBuf;
///
/// assert_eq!(
///     sram_path_for_rom(&PathBuf::from("/games/zelda.nes")),
///     PathBuf::from("/games/zelda.nessram"),
/// );
/// assert_eq!(
///     sram_path_for_rom(&PathBuf::from("roms/smb3")),
///     PathBuf::from("roms/smb3.nessram"),
/// );
/// ```
pub fn sram_path_for_rom(rom_path: &Path) -> PathBuf {
    let stem = rom_path.file_stem().map(|s| s.to_os_string());
    let dir = rom_path.parent().unwrap_or_else(|| Path::new(""));
    match stem {
        Some(stem) => {
            // Build `<dir>/<stem>.nessram` manually rather than via
            // `with_extension`, so multi-dot stems like `foo.bar.nes` →
            // `foo.bar.nessram` (preserving the inner `.bar`).
            let mut name = stem;
            name.push(".");
            name.push(SRAM_EXTENSION);
            dir.join(name)
        }
        None => {
            // No stem (e.g. path is empty or "/") — fall back to a
            // `.nessram` file in the same directory with an empty stem.
            dir.join(format!(".{SRAM_EXTENSION}"))
        }
    }
}

/// Load battery-backed PRG-RAM for the given ROM, if a `.nessram` sidecar
/// file exists.
///
/// Returns `Ok(Some(data))` if the sidecar file exists and was read
/// successfully, `Ok(None)` if no sidecar file exists (the common case for
/// a first run), or `Err` if the file exists but could not be read. A
/// missing sidecar is **not** an error — the cartridge should simply start
/// with zeroed PRG-RAM in that case.
pub fn load_for_rom(rom_path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    let sram_path = sram_path_for_rom(rom_path);
    match std::fs::read(&sram_path) {
        Ok(data) => Ok(Some(data)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Save battery-backed PRG-RAM contents to the `.nessram` sidecar file next
/// to the given ROM.
///
/// The sidecar file is created or overwritten atomically: the data is first
/// written to a temporary file in the same directory and then renamed over
/// the target path. This avoids leaving a truncated/corrupt save file if the
/// write is interrupted (e.g. by a power loss or signal).
///
/// Returns the path the data was written to on success, or an IO error on
/// failure.
pub fn save_for_rom(rom_path: &Path, data: &[u8]) -> std::io::Result<PathBuf> {
    let sram_path = sram_path_for_rom(rom_path);
    write_atomic(&sram_path, data)?;
    Ok(sram_path)
}

/// Write `data` to `path` via a same-directory temp file + rename, so that
/// an interrupted write cannot leave a truncated save file.
fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new(""));
    let tmp = dir.join(format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("nessram")
    ));
    std::fs::write(&tmp, data)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        // Clean up the temp file if the rename failed (e.g. cross-filesystem
        // move, permission error) so we don't leave stray `.tmp` files in
        // the ROM directory. The original save file, if any, is left
        // untouched — the rename never overwrote it.
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Global counter so each test gets a unique subdirectory under the
    /// system temp dir — avoids parallel tests stomping on each other's
    /// sidecar files without pulling in a `tempfile` dev-dependency.
    static TEST_DIR_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// Create a unique temp directory for a single test. The directory is
    /// created fresh; the caller is responsible for cleaning up (though
    /// leftover dirs under the system temp are harmless).
    fn temp_dir() -> PathBuf {
        let n = TEST_DIR_COUNTER.fetch_add(1, Ordering::SeqCst);
        let pid = std::process::id();
        let path = std::env::temp_dir().join(format!("nes-emu-battery-{pid}-{n}"));
        std::fs::create_dir_all(&path).expect("create temp dir");
        path
    }

    #[test]
    fn sram_path_replaces_nes_extension() {
        let p = sram_path_for_rom(&PathBuf::from("/games/zelda.nes"));
        assert_eq!(p, PathBuf::from("/games/zelda.nessram"));
    }

    #[test]
    fn sram_path_appends_extension_when_missing() {
        let p = sram_path_for_rom(&PathBuf::from("roms/smb3"));
        assert_eq!(p, PathBuf::from("roms/smb3.nessram"));
    }

    #[test]
    fn sram_path_preserves_directory() {
        let p = sram_path_for_rom(&PathBuf::from("/home/user/games/foo.nes"));
        assert_eq!(p, PathBuf::from("/home/user/games/foo.nessram"));
    }

    #[test]
    fn sram_path_handles_uppercase_extension() {
        let p = sram_path_for_rom(&PathBuf::from("/games/ROM.NES"));
        assert_eq!(p, PathBuf::from("/games/ROM.nessram"));
    }

    #[test]
    fn sram_path_handles_double_extension() {
        // `with_extension` only strips the final extension, so a file like
        // `foo.bar.nes` becomes `foo.bar.nessram` — the `.bar` part is kept
        // as part of the stem.
        let p = sram_path_for_rom(&PathBuf::from("/games/foo.bar.nes"));
        assert_eq!(p, PathBuf::from("/games/foo.bar.nessram"));
    }

    #[test]
    fn load_returns_none_when_no_sidecar() {
        let dir = temp_dir();
        let rom = dir.join("game.nes");
        std::fs::write(&rom, b"NES\x1A").unwrap();
        let loaded = load_for_rom(&rom).expect("load should not error");
        assert!(loaded.is_none(), "no sidecar → None");
    }

    #[test]
    fn load_returns_data_when_sidecar_exists() {
        let dir = temp_dir();
        let rom = dir.join("game.nes");
        std::fs::write(&rom, b"NES\x1A").unwrap();
        let sram = sram_path_for_rom(&rom);
        std::fs::write(&sram, [0x11, 0x22, 0x33, 0x44]).unwrap();

        let loaded = load_for_rom(&rom).expect("load should not error");
        assert_eq!(loaded, Some(vec![0x11, 0x22, 0x33, 0x44]));
    }

    #[test]
    fn save_writes_sidecar_file() {
        let dir = temp_dir();
        let rom = dir.join("game.nes");
        std::fs::write(&rom, b"NES\x1A").unwrap();

        let written_to = save_for_rom(&rom, &[0xAA, 0xBB, 0xCC]).expect("save");
        assert_eq!(written_to, sram_path_for_rom(&rom));

        let on_disk = std::fs::read(&written_to).expect("read back");
        assert_eq!(on_disk, vec![0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn save_overwrites_existing_sidecar() {
        let dir = temp_dir();
        let rom = dir.join("game.nes");
        std::fs::write(&rom, b"NES\x1A").unwrap();
        let sram = sram_path_for_rom(&rom);
        std::fs::write(&sram, [0xFF; 16]).unwrap();

        save_for_rom(&rom, &[0x01, 0x02]).expect("save");
        let on_disk = std::fs::read(&sram).expect("read back");
        assert_eq!(on_disk, vec![0x01, 0x02]);
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = temp_dir();
        let rom = dir.join("game.nes");
        std::fs::write(&rom, b"NES\x1A").unwrap();

        let payload: Vec<u8> = (0..8192u32).map(|i| (i & 0xFF) as u8).collect();
        save_for_rom(&rom, &payload).expect("save");
        let loaded = load_for_rom(&rom).expect("load");
        assert_eq!(loaded, Some(payload));
    }

    #[test]
    fn save_does_not_leave_temp_file() {
        let dir = temp_dir();
        let rom = dir.join("game.nes");
        std::fs::write(&rom, b"NES\x1A").unwrap();

        save_for_rom(&rom, &[0x42]).expect("save");
        let entries = std::fs::read_dir(&dir)
            .expect("read dir")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        // Only the ROM and the sidecar should be present — no leftover .tmp.
        assert!(
            entries.iter().all(|n| !n.ends_with(".tmp")),
            "leftover tmp: {entries:?}"
        );
    }

    #[test]
    fn save_empty_payload_is_valid() {
        let dir = temp_dir();
        let rom = dir.join("game.nes");
        std::fs::write(&rom, b"NES\x1A").unwrap();

        save_for_rom(&rom, &[]).expect("save empty");
        let loaded = load_for_rom(&rom).expect("load");
        assert_eq!(loaded, Some(Vec::new()));
    }

    #[test]
    fn save_to_missing_directory_errors() {
        let rom = PathBuf::from("/this/does/not/exist/game.nes");
        let result = save_for_rom(&rom, &[0x01]);
        assert!(result.is_err(), "writing to a missing dir should error");
    }

    /// Verify the atomic-write temp file is created in the same directory as
    /// the target (so the rename is a same-filesystem atomic move). We can't
    /// observe the temp file directly (it's renamed away), but we can verify
    /// the rename happens by writing to a file whose target dir we control.
    #[test]
    fn atomic_write_replaces_existing_file() {
        let dir = temp_dir();
        let target = dir.join("existing.nessram");
        std::fs::write(&target, "OLD-CONTENT").unwrap();

        write_atomic(&target, b"NEW-CONTENT").expect("atomic write");
        let on_disk = std::fs::read_to_string(&target).unwrap();
        assert_eq!(on_disk, "NEW-CONTENT");
    }
}
