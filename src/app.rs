//! Application-level helpers — ROM loading and `EmulatorState` construction.

use std::path::{Path, PathBuf};

use crate::battery;
use crate::cartridge::Cartridge;
use crate::config::Config;
use crate::emulator::EmulatorState;
use crate::fds;
use crate::rom_manager::{build_rom_info, file_name_from_path, RomInfo};

/// The product of a successful ROM load: a fresh emulator ready to run,
/// a [`RomInfo`] snapshot for the OSD, and the canonical on-disk path of
/// the loaded ROM (for battery-SRAM sidecar lookup and recent-list
/// recording).
pub struct LoadedRom {
    pub emulator: EmulatorState,
    pub info: RomInfo,
    pub rom_path: PathBuf,
    pub file_size: u64,
}

/// Load a ROM from `rom_path`, applying the standard boot sequence:
///
/// 1. Read + (if needed) decompress the file.
/// 2. Load battery SRAM sidecar if present.
/// 3. Resolve the region (config override → header hint → NTSC).
/// 4. Build the emulator and apply per-channel APU volumes from config.
///
/// Returns a [`LoadedRom`] on success. Errors are surfaced as a
/// human-readable `String` for the main loop to print to stderr.
pub fn load_rom(rom_path: &Path, config: &Config) -> Result<LoadedRom, String> {
    let file_size = std::fs::metadata(rom_path).map(|m| m.len()).unwrap_or(0);

    // M36: FDS disk images use a separate loading path (not iNES).
    let mut cartridge = if fds::is_fds_file(rom_path) {
        Cartridge::from_fds_path(rom_path)
            .map_err(|e| format!("failed to load FDS disk '{}': {e}", rom_path.display()))?
    } else {
        Cartridge::from_path(rom_path)
            .map_err(|e| format!("failed to load ROM '{}': {e}", rom_path.display()))?
    };

    // Battery-backed PRG-RAM persistence (M21): load `.nessram` sidecar
    // if present. Missing file / errors are non-fatal.
    if cartridge.has_battery() {
        match battery::load_for_rom(rom_path) {
            Ok(Some(data)) => cartridge.load_battery_sram(&data),
            Ok(None) => {}
            Err(e) => eprintln!("nes-emu: warning: could not read battery SRAM: {e}"),
        }
    }

    // M32: resolve region — config override wins, else header hint, else NTSC.
    let region = config.resolve_region(cartridge.header.region_hint());
    eprintln!("nes-emu: region = {}", region.short_name());
    let mut emulator = EmulatorState::new_with_region(cartridge, region);
    emulator.reset();
    // M31: apply per-channel APU volumes from config.
    emulator
        .bus_mut()
        .apu_mut()
        .apply_channel_volumes(&config.audio_channels.as_array());

    // Apply debug rendering settings from config.
    if config.debug.slant_corruption {
        eprintln!("nes-emu: debug slant_corruption enabled — rendering with fine-X scroll bug");
        emulator.bus_mut().ppu_mut().set_slant_corruption(true);
    }

    // Build the ROM-info snapshot for the OSD overlay.
    let header = emulator.bus().cartridge().map(|c| &c.header).cloned();
    let info = if let Some(h) = header {
        build_rom_info(&h, &file_name_from_path(rom_path), file_size, region)
    } else {
        RomInfo::default()
    };

    Ok(LoadedRom {
        emulator,
        info,
        rom_path: rom_path.to_path_buf(),
        file_size,
    })
}

/// Save the battery SRAM of the currently-loaded cartridge to its
/// `.nessram` sidecar. Errors are non-fatal (the emulator is shutting
/// down or swapping cartridges). Returns `true` if a save was attempted.
pub fn save_battery_sram(emulator: &EmulatorState, rom_path: &Path) -> bool {
    if !emulator.has_battery() {
        return false;
    }
    if let Some(sram) = emulator.battery_sram() {
        if let Err(e) = battery::save_for_rom(rom_path, &sram) {
            eprintln!("nes-emu: warning: could not save battery SRAM: {e}");
        }
        return true;
    }
    false
}

/// Helper for tests: build a minimal `Config` with default settings.
#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn default_config() -> Config {
    Config::default()
}

/// Helper for tests: resolve the region for a cartridge header using the
/// default config (no override).
#[cfg(test)]
#[allow(dead_code)]
pub(crate) fn default_region_for(hint: Option<crate::region::Region>) -> crate::region::Region {
    Config::default().resolve_region(hint)
}
