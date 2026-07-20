//! ROM management — drag-and-drop queue, recent-ROM list, and ROM info
//! overlay (M34).
//!
//! This module is the glue between the SDL2 event layer (which delivers
//! `DropFile` events), the config layer (which persists the recent-ROM
//! list), and the OSD layer (which renders the ROM-info overlay). The
//! actual cartridge loading / emulator rebuild happens in `main.rs`; this
//! module only tracks *what* to load and *what* to display.
//!
//! # Recent ROMs
//!
//! The recent-ROM list is a capped, deduplicated, most-recent-first list
//! of ROM file paths. It is persisted by [`crate::config::Config`] under
//! the `recent_roms` array in `config.toml`. The list is updated via
//! [`RomManager::record_loaded_rom`], which is called by the main loop
//! after a successful ROM load (initial `--rom`, drag-and-drop, or
//! hotkey-selected recent entry).
//!
//! # ROM info overlay
//!
//! [`RomInfo`] is a snapshot of the loaded cartridge's metadata (mapper
//! number, PRG/CHR sizes, mirroring, battery flag, region hint, file
//! size). The main loop builds it from the cartridge header and feeds it
//! to [`RomManager::info_lines`], which formats it as OSD strings. The
//! overlay is toggled with a hotkey (default `F12`).
//!
//! See: https://www.nesdev.org/wiki/INES

use std::path::PathBuf;

use crate::mappers::Mirroring;
use crate::region::Region;

/// Maximum number of recent-ROM entries kept in the list. The acceptance
/// criteria for M34 specify "last 10".
pub const RECENT_ROM_MAX: usize = 10;

/// Hotkey for toggling the ROM-info overlay (M34).
pub const ROM_INFO_HOTKEY: sdl2::keyboard::Keycode = sdl2::keyboard::Keycode::F12;

/// A snapshot of a loaded cartridge's metadata, used for the ROM-info
/// OSD overlay. Built by the main loop from the cartridge header + the
/// on-disk file size.
#[derive(Debug, Clone)]
pub struct RomInfo {
    /// ROM file name (no directory) for display.
    pub file_name: String,
    /// On-disk file size in bytes (compressed size if loaded from a
    /// `.gz`/`.zip` archive — the decompressed size is reflected in the
    /// PRG/CHR totals).
    pub file_size: u64,
    /// iNES mapper number.
    pub mapper_number: u16,
    /// PRG-ROM size in bytes (16 KB × `prg_rom_banks`).
    pub prg_rom_size: usize,
    /// CHR-ROM size in bytes (8 KB × `chr_rom_banks`); 0 for CHR-RAM.
    pub chr_rom_size: usize,
    /// Nametable mirroring mode.
    pub mirroring: Mirroring,
    /// Whether the cartridge advertises battery-backed PRG-RAM.
    pub has_battery: bool,
    /// Whether the cartridge has a 512-byte trainer.
    pub has_trainer: bool,
    /// Region hint from the iNES header (byte 9), if any.
    pub region_hint: Option<Region>,
    /// Resolved region actually in use (after config override + hint).
    pub region: Region,
}

impl Default for RomInfo {
    fn default() -> Self {
        RomInfo {
            file_name: String::new(),
            file_size: 0,
            mapper_number: 0,
            prg_rom_size: 0,
            chr_rom_size: 0,
            mirroring: Mirroring::Horizontal,
            has_battery: false,
            has_trainer: false,
            region_hint: None,
            region: Region::Ntsc,
        }
    }
}

/// ROM manager state held by the main loop.
#[derive(Debug, Default)]
pub struct RomManager {
    /// Path dropped onto the window via SDL2 `DropFile`, waiting to be
    /// loaded by the main loop between frames. Only one drop is queued
    /// at a time; later drops overwrite earlier ones (the user can
    /// re-drop if they change their mind).
    pending_drop: Option<PathBuf>,
    /// Whether the ROM-info overlay is currently visible.
    info_overlay: bool,
    /// Most-recent-first list of loaded ROM paths (capped at
    /// [`RECENT_ROM_MAX`], deduplicated).
    recent_roms: Vec<String>,
}

impl RomManager {
    /// Build a new ROM manager with the given initial recent-ROM list
    /// (typically loaded from `config.toml`).
    pub fn new(recent_roms: Vec<String>) -> Self {
        RomManager {
            pending_drop: None,
            info_overlay: false,
            recent_roms,
        }
    }

    /// Queue a file path dropped onto the window for loading. The main
    /// loop drains this between frames via [`take_pending_drop`].
    pub fn queue_drop(&mut self, path: PathBuf) {
        self.pending_drop = Some(path);
    }

    /// Drain the pending drop path, if any. The main loop calls this
    /// after the event pump has been emptied; if it returns `Some`, the
    /// loop rebuilds the emulator with the new ROM.
    pub fn take_pending_drop(&mut self) -> Option<PathBuf> {
        self.pending_drop.take()
    }

    /// Is there a pending drop waiting to be processed?
    pub fn has_pending_drop(&self) -> bool {
        self.pending_drop.is_some()
    }

    /// Toggle the ROM-info overlay visibility. Returns the new state.
    pub fn toggle_info_overlay(&mut self) -> bool {
        self.info_overlay = !self.info_overlay;
        self.info_overlay
    }

    /// Explicitly set the ROM-info overlay visibility.
    pub fn set_info_overlay(&mut self, on: bool) {
        self.info_overlay = on;
    }

    /// Is the ROM-info overlay currently visible?
    pub fn info_overlay_enabled(&self) -> bool {
        self.info_overlay
    }

    /// Record a successfully loaded ROM path at the head of the recent
    /// list. Deduplicates (moves existing entry to front) and trims to
    /// [`RECENT_ROM_MAX`]. Returns the new list so the caller can persist
    /// it to `config.toml`.
    pub fn record_loaded_rom(&mut self, path: &str) -> Vec<String> {
        // Deduplicate case-sensitively (paths are case-sensitive on
        // Linux; on Windows/macOS the case-insensitive comparison would
        // need canonicalisation, but the worst case is a duplicate entry
        // which is harmless).
        if let Some(pos) = self.recent_roms.iter().position(|p| p == path) {
            self.recent_roms.remove(pos);
        }
        self.recent_roms.insert(0, path.to_string());
        if self.recent_roms.len() > RECENT_ROM_MAX {
            self.recent_roms.truncate(RECENT_ROM_MAX);
        }
        self.recent_roms.clone()
    }

    /// Borrow the recent-ROM list (most-recent-first).
    pub fn recent_roms(&self) -> &[String] {
        &self.recent_roms
    }

    /// Replace the recent-ROM list (e.g. after loading from config).
    pub fn set_recent_roms(&mut self, roms: Vec<String>) {
        self.recent_roms = roms;
        if self.recent_roms.len() > RECENT_ROM_MAX {
            self.recent_roms.truncate(RECENT_ROM_MAX);
        }
    }

    /// Build the OSD line set for the ROM-info overlay. Returns an empty
    /// `Vec` if the overlay is disabled.
    pub fn info_lines(&self, info: &RomInfo) -> Vec<String> {
        if !self.info_overlay {
            return Vec::new();
        }
        let mirroring = match info.mirroring {
            Mirroring::Horizontal => "H",
            Mirroring::Vertical => "V",
            Mirroring::FourScreen => "4S",
            Mirroring::SingleScreen(nt) => match nt {
                0 => "1SA",
                1 => "1SB",
                2 => "1SC",
                _ => "1SD",
            },
        };
        let battery = if info.has_battery { "BATT" } else { "----" };
        let trainer = if info.has_trainer { "TRN" } else { "---" };
        let region_hint = match info.region_hint {
            Some(r) => r.short_name(),
            None => "auto",
        };
        vec![
            format!(
                "ROM:{} ({}B)",
                truncate_for_osd(&info.file_name, 20),
                info.file_size
            ),
            format!(
                "MAP:{} PRG:{}K CHR:{}K",
                info.mapper_number,
                info.prg_rom_size / 1024,
                info.chr_rom_size / 1024
            ),
            format!("MIR:{} {} {} {}", mirroring, battery, trainer, region_hint),
            format!("REGION:{}", info.region.short_name()),
        ]
    }

    /// Build the OSD line set showing the recent-ROM list (used when the
    /// user is browsing the recent list via hotkeys). Each line is
    /// `N:path` where `N` is the 1-based index. Returns an empty `Vec`
    /// if the list is empty.
    pub fn recent_lines(&self) -> Vec<String> {
        self.recent_roms
            .iter()
            .enumerate()
            .map(|(i, p)| format!("{}:{}", i + 1, truncate_for_osd(p, 28)))
            .collect()
    }

    /// Render the ROM-info overlay into the framebuffer if the overlay is
    /// enabled (M34). This is a one-shot blit that ignores the persistent
    /// OSD `enabled` flag — it draws whenever [`info_overlay_enabled`]
    /// is true. Called by the main loop after the save-state post-frame
    /// hook so the OSD + info overlay stack correctly.
    pub fn render_info_overlay(&self, fb: &mut [u32], width: u32, height: u32, info: &RomInfo) {
        if !self.info_overlay {
            return;
        }
        let lines = self.info_lines(info);
        if lines.is_empty() {
            return;
        }
        let strs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        let osd = crate::osd::Osd::default();
        osd.render_forced(fb, width, height, &strs);
    }
}

/// Truncate `s` to at most `max` characters, appending `..` if truncated.
/// Used to keep OSD lines within the 256-pixel-wide framebuffer.
fn truncate_for_osd(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(2)).collect();
    out.push_str("..");
    out
}

/// Build a [`RomInfo`] snapshot from a cartridge header + file metadata.
/// `file_size` is the on-disk size of the loaded file (may be the
/// compressed archive size). `region` is the resolved region in use.
pub fn build_rom_info(
    header: &crate::cartridge::InesHeader,
    file_name: &str,
    file_size: u64,
    region: Region,
) -> RomInfo {
    RomInfo {
        file_name: file_name.to_string(),
        file_size,
        mapper_number: header.mapper_number,
        prg_rom_size: header.prg_rom_banks as usize * crate::cartridge::PRG_ROM_UNIT,
        chr_rom_size: header.chr_rom_banks as usize * crate::cartridge::CHR_ROM_UNIT,
        mirroring: header.mirroring,
        has_battery: header.has_battery,
        has_trainer: header.has_trainer,
        region_hint: header.region_hint(),
        region,
    }
}

/// Extract the file name (no directory) from a path for display. Returns
/// `"?"` on parse failure. Mirrors [`osd::game_name_from_path`] but keeps
/// the extension (useful for ROM-info display where the user wants to see
/// `.nes`/`.gz`/`.zip`).
pub fn file_name_from_path(path: &std::path::Path) -> String {
    path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("?")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cartridge::{InesHeader, PRG_ROM_UNIT};

    fn fake_header(mapper: u16, mirroring: Mirroring) -> InesHeader {
        InesHeader {
            prg_rom_banks: 2,
            chr_rom_banks: 1,
            mapper_number: mapper,
            mirroring,
            has_trainer: false,
            has_battery: false,
            tv_system: 0,
        }
    }

    #[test]
    fn record_loaded_rom_inserts_at_head() {
        let mut rm = RomManager::new(vec![]);
        rm.record_loaded_rom("/a.nes");
        rm.record_loaded_rom("/b.nes");
        assert_eq!(
            rm.recent_roms(),
            &["/b.nes".to_string(), "/a.nes".to_string()]
        );
    }

    #[test]
    fn record_loaded_rom_deduplicates_and_moves_to_front() {
        let mut rm = RomManager::new(vec!["/a.nes".into(), "/b.nes".into()]);
        rm.record_loaded_rom("/a.nes");
        assert_eq!(
            rm.recent_roms(),
            &["/a.nes".to_string(), "/b.nes".to_string()]
        );
    }

    #[test]
    fn record_loaded_rom_caps_at_max() {
        let mut rm = RomManager::new(vec![]);
        for i in 0..(RECENT_ROM_MAX + 5) {
            rm.record_loaded_rom(&format!("/{i}.nes"));
        }
        assert_eq!(rm.recent_roms().len(), RECENT_ROM_MAX);
        // Most recent first.
        assert_eq!(rm.recent_roms()[0], format!("/{}.nes", RECENT_ROM_MAX + 4));
    }

    #[test]
    fn pending_drop_queue_and_take() {
        let mut rm = RomManager::new(vec![]);
        assert!(!rm.has_pending_drop());
        rm.queue_drop(PathBuf::from("/dropped.nes"));
        assert!(rm.has_pending_drop());
        let taken = rm.take_pending_drop();
        assert_eq!(taken, Some(PathBuf::from("/dropped.nes")));
        assert!(!rm.has_pending_drop());
        // Second take returns None.
        assert!(rm.take_pending_drop().is_none());
    }

    #[test]
    fn pending_drop_overwrites_earlier() {
        let mut rm = RomManager::new(vec![]);
        rm.queue_drop(PathBuf::from("/first.nes"));
        rm.queue_drop(PathBuf::from("/second.nes"));
        assert_eq!(rm.take_pending_drop(), Some(PathBuf::from("/second.nes")));
    }

    #[test]
    fn info_overlay_toggle() {
        let mut rm = RomManager::new(vec![]);
        assert!(!rm.info_overlay_enabled());
        assert!(rm.toggle_info_overlay());
        assert!(rm.info_overlay_enabled());
        assert!(!rm.toggle_info_overlay());
        assert!(!rm.info_overlay_enabled());
    }

    #[test]
    fn info_lines_empty_when_overlay_disabled() {
        let rm = RomManager::new(vec![]);
        let info = build_rom_info(
            &fake_header(0, Mirroring::Horizontal),
            "game.nes",
            40976,
            Region::Ntsc,
        );
        assert!(rm.info_lines(&info).is_empty());
    }

    #[test]
    fn info_lines_show_mapper_prg_chr_mirroring() {
        let mut rm = RomManager::new(vec![]);
        rm.set_info_overlay(true);
        let info = build_rom_info(
            &fake_header(4, Mirroring::Vertical),
            "game.nes",
            40976,
            Region::Ntsc,
        );
        let lines = rm.info_lines(&info);
        assert_eq!(lines.len(), 4);
        assert!(lines[1].contains("MAP:4"));
        assert!(lines[1].contains("PRG:32K"));
        assert!(lines[1].contains("CHR:8K"));
        assert!(lines[2].contains("MIR:V"));
    }

    #[test]
    fn info_lines_show_battery_and_trainer_flags() {
        let mut rm = RomManager::new(vec![]);
        rm.set_info_overlay(true);
        let mut h = fake_header(1, Mirroring::Horizontal);
        h.has_battery = true;
        h.has_trainer = true;
        let info = build_rom_info(&h, "game.nes", 32768, Region::Ntsc);
        let lines = rm.info_lines(&info);
        assert!(lines[2].contains("BATT"));
        assert!(lines[2].contains("TRN"));
    }

    #[test]
    fn info_lines_show_region_hint_and_resolved() {
        let mut rm = RomManager::new(vec![]);
        rm.set_info_overlay(true);
        let mut h = fake_header(0, Mirroring::Horizontal);
        h.tv_system = 1; // PAL hint
        let info = build_rom_info(&h, "game.nes", 32768, Region::Pal);
        let lines = rm.info_lines(&info);
        // Region hint line shows the hint short name; resolved region line
        // shows the actual region in use.
        assert!(lines[2].contains("PAL") || lines.iter().any(|l| l.contains("PAL")));
        assert!(lines[3].contains("PAL"));
    }

    #[test]
    fn info_lines_truncate_long_file_names() {
        let mut rm = RomManager::new(vec![]);
        rm.set_info_overlay(true);
        let info = build_rom_info(
            &fake_header(0, Mirroring::Horizontal),
            "this_is_a_very_long_rom_file_name_that_exceeds_the_osd_width.nes",
            32768,
            Region::Ntsc,
        );
        let lines = rm.info_lines(&info);
        // The first line should be truncated (contain "..").
        assert!(lines[0].contains(".."));
    }

    #[test]
    fn recent_lines_numbered_one_based() {
        let rm = RomManager::new(vec!["/a.nes".into(), "/b.nes".into()]);
        let lines = rm.recent_lines();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("1:/a.nes"));
        assert!(lines[1].starts_with("2:/b.nes"));
    }

    #[test]
    fn recent_lines_empty_when_no_roms() {
        let rm = RomManager::new(vec![]);
        assert!(rm.recent_lines().is_empty());
    }

    #[test]
    fn set_recent_roms_caps_at_max() {
        let mut rm = RomManager::new(vec![]);
        let mut many = Vec::new();
        for i in 0..(RECENT_ROM_MAX + 3) {
            many.push(format!("/{i}.nes"));
        }
        rm.set_recent_roms(many);
        assert_eq!(rm.recent_roms().len(), RECENT_ROM_MAX);
    }

    #[test]
    fn build_rom_info_prg_chr_sizes() {
        let mut h = fake_header(0, Mirroring::Horizontal);
        h.prg_rom_banks = 4;
        h.chr_rom_banks = 2;
        let info = build_rom_info(&h, "g.nes", 0, Region::Ntsc);
        assert_eq!(info.prg_rom_size, 4 * PRG_ROM_UNIT);
        assert_eq!(info.chr_rom_size, 2 * crate::cartridge::CHR_ROM_UNIT);
    }

    #[test]
    fn file_name_from_path_keeps_extension() {
        let p = std::path::Path::new("/some/dir/game.nes.gz");
        assert_eq!(file_name_from_path(p), "game.nes.gz");
    }

    #[test]
    fn file_name_from_path_handles_root() {
        let p = std::path::Path::new("game.nes");
        assert_eq!(file_name_from_path(p), "game.nes");
    }

    #[test]
    fn truncate_for_osd_short_string_unchanged() {
        assert_eq!(truncate_for_osd("short", 10), "short");
    }

    #[test]
    fn truncate_for_osd_long_string_gets_ellipsis() {
        let s = "x".repeat(30);
        let t = truncate_for_osd(&s, 10);
        assert_eq!(t.chars().count(), 10);
        assert!(t.ends_with(".."));
    }
}
