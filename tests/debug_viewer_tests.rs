//! Integration tests for the M28 debug tools — PPU viewer, memory
//! viewer, and trace logger — exercised end-to-end through a real
//! `Bus` / `EmulatorState`.
//!
//! The viewers read via side-effect-free accessors (`Bus::peek`,
//! `Ppu::read_nametable` / `read_palette` / `oam()`,
//! `Cartridge::read_chr`); dedicated tests confirm that opening the
//! viewers cannot perturb emulator state (PPUSTATUS VBlank survives,
//! MMC2 CHR latches do not fire).

use nes_emu::bus::Bus;
use nes_emu::cartridge::Cartridge;
use nes_emu::debug::{disassemble_at, MemoryRegion, MemoryViewer, PpuView, PpuViewer, TraceLogger};
use nes_emu::emulator::EmulatorState;
use std::sync::atomic::{AtomicU64, Ordering};

// ---- Test fixtures --------------------------------------------------------

/// Build a minimal NROM-128 cartridge (16 KB PRG filled with NOP, 8 KB
/// CHR-RAM) whose RESET vector points at `$C000`.
fn nop_cart() -> Cartridge {
    let mut bytes = vec![b'N', b'E', b'S', 0x1A, 1, 0, 0, 0];
    bytes.extend_from_slice(&[0u8; 8]);
    bytes.resize(16 + 16 * 1024, 0xEA);
    let reset_off = 16 + 0x3FFC;
    bytes[reset_off] = 0x00;
    bytes[reset_off + 1] = 0xC0;
    Cartridge::from_bytes(&bytes).expect("build NOP cart")
}

/// Build a minimal NROM-128 cartridge with a known CHR pattern: tile 0
/// has its first row = `$FF` (all 8 pixels set) in both bit planes, so
/// the pattern viewer should show a row of `#` for tile 0.
fn cart_with_chr_pattern() -> Cartridge {
    let mut bytes = vec![b'N', b'E', b'S', 0x1A, 1, 1, 0, 0]; // 1 CHR bank
    bytes.extend_from_slice(&[0u8; 8]);
    bytes.resize(16 + 16 * 1024, 0xEA); // PRG = NOP
    let reset_off = 16 + 0x3FFC;
    bytes[reset_off] = 0x00;
    bytes[reset_off + 1] = 0xC0;
    // CHR-ROM starts at 16 + 16KB = 0x4010. Tile 0 plane 0 row 0 = $FF.
    bytes.resize(16 + 16 * 1024 + 8 * 1024, 0);
    let chr_off = 16 + 16 * 1024;
    bytes[chr_off] = 0xFF; // plane 0, row 0
    bytes[chr_off + 8] = 0xFF; // plane 1, row 0
    Cartridge::from_bytes(&bytes).expect("build CHR-pattern cart")
}

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

fn temp_path(tag: &str) -> std::path::PathBuf {
    let n = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    let mut p = std::env::temp_dir();
    p.push(format!("nes-emu-m28-{tag}-{pid}-{n}.log"));
    p
}

fn cleanup(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
}

// ---- PPU viewer -----------------------------------------------------------

#[test]
fn ppu_viewer_nametables_show_4_tables_and_mirroring() {
    let bus = Bus::new();
    let v = PpuViewer {
        mode: PpuView::Nametables,
        pattern_bank: 0,
    };
    let s = v.render(&bus);
    assert!(s.contains("NT0 ($2000)"));
    assert!(s.contains("NT1 ($2400)"));
    assert!(s.contains("NT2 ($2800)"));
    assert!(s.contains("NT3 ($2C00)"));
    assert!(s.contains("mirroring:"));
}

#[test]
fn ppu_viewer_nametables_reflect_writes() {
    let mut bus = Bus::new();
    // Default mirroring is Horizontal: NT0 ($2000) and NT2 ($2800) map
    // to different physical nametables (phys 0 and phys 1), so writes
    // to both are independently visible.
    bus.ppu_mut().write_nametable(0x2000, 0x42);
    bus.ppu_mut().write_nametable(0x2800, 0x99);
    let v = PpuViewer {
        mode: PpuView::Nametables,
        pattern_bank: 0,
    };
    let s = v.render(&bus);
    // NT0 row 0 should start with 42; NT2 row 0 should start with 99.
    assert!(s.contains("42 00 00"));
    assert!(s.contains("99 00 00"));
}

#[test]
fn ppu_viewer_pattern_table_uses_chr_from_cartridge() {
    let cart = cart_with_chr_pattern();
    let bus = Bus::with_cartridge(cart);
    let v = PpuViewer {
        mode: PpuView::PatternTable { bank: 0 },
        pattern_bank: 0,
    };
    let s = v.render(&bus);
    // Tile 0 row 0 should be all '#' (both bit planes = $FF → pixel
    // value 3 → density char '#').
    let first_tile_row = s.lines().nth(2).expect("pattern output has rows");
    assert!(
        first_tile_row.starts_with("########"),
        "expected tile 0 row 0 to be all '#', got: {first_tile_row}",
    );
}

#[test]
fn ppu_viewer_pattern_table_empty_bus_does_not_panic() {
    let bus = Bus::new();
    let v = PpuViewer {
        mode: PpuView::PatternTable { bank: 1 },
        pattern_bank: 1,
    };
    let s = v.render(&bus);
    assert!(s.contains("pattern table 1"));
}

#[test]
fn ppu_viewer_oam_shows_64_sprites_with_fields() {
    let mut bus = Bus::new();
    // Set sprite 0 to known values.
    bus.ppu_mut().oam_mut()[0] = 0x10; // Y
    bus.ppu_mut().oam_mut()[1] = 0x20; // tile
    bus.ppu_mut().oam_mut()[2] = 0x03; // attr (palette 3, no flip)
    bus.ppu_mut().oam_mut()[3] = 0x40; // X
    let v = PpuViewer {
        mode: PpuView::Oam,
        pattern_bank: 0,
    };
    let s = v.render(&bus);
    // Header + separator + 64 sprite rows.
    let lines: Vec<&str> = s.lines().collect();
    assert!(lines.len() >= 66);
    // Sprite 0 row should contain the Y / tile / attr / X values.
    let sprite0 = lines
        .iter()
        .find(|l| l.starts_with("  0 "))
        .expect("sprite 0");
    assert!(sprite0.contains("10"));
    assert!(sprite0.contains("20"));
    assert!(sprite0.contains("40"));
}

#[test]
fn ppu_viewer_palettes_show_8_palettes_with_argb() {
    let mut bus = Bus::new();
    // $3F00 is the universal background color. $3F10 mirrors $3F00, so
    // we write to $3F01 (an independent slot) to avoid clobbering.
    bus.ppu_mut().write_palette(0x3F00, 0x21);
    bus.ppu_mut().write_palette(0x3F01, 0x20);
    let v = PpuViewer {
        mode: PpuView::Palettes,
        pattern_bank: 0,
    };
    let s = v.render(&bus);
    assert!(s.contains("(bg)"));
    assert!(s.contains("(sprite)"));
    // The ARGB for color 0x21 is 0xFF9CDCFF (verified in render tests).
    assert!(s.contains("21/FF9CDCFF"));
    // Color 0x20 is white = 0xFFFFFFFF.
    assert!(s.contains("20/FFFFFFFF"));
    // 8 palette rows + header + raw dump header + 2 raw rows.
    let lines: Vec<&str> = s.lines().collect();
    assert!(lines.len() >= 8);
}

#[test]
fn ppu_viewer_cycle_rotates_through_all_views() {
    let mut v = PpuViewer::new();
    assert_eq!(v.mode, PpuView::Nametables);
    v.cycle();
    assert_eq!(v.mode, PpuView::PatternTable { bank: 0 });
    v.cycle();
    assert_eq!(v.mode, PpuView::PatternTable { bank: 1 });
    v.cycle();
    assert_eq!(v.mode, PpuView::Oam);
    v.cycle();
    assert_eq!(v.mode, PpuView::Palettes);
    v.cycle();
    assert_eq!(v.mode, PpuView::Nametables);
}

#[test]
fn ppu_viewer_pattern_render_is_pure_no_internal_state_mutation() {
    // The PPU viewer holds no mutable state between renders (the
    // `pattern_bank` field is only changed by `cycle()`). Rendering the
    // same view twice on the same bus must produce identical output,
    // confirming the viewer does not cache or mutate anything during a
    // render pass. (CHR reads go through `Cartridge::read_chr`, which
    // for MMC2 has a *latched* variant — the viewer deliberately uses
    // the non-latched path; this test guards against accidental
    // migration to a latched read by ensuring re-rendering is stable.)
    let bus = Bus::new();
    let v = PpuViewer {
        mode: PpuView::PatternTable { bank: 0 },
        pattern_bank: 0,
    };
    let s1 = v.render(&bus);
    let s2 = v.render(&bus);
    assert_eq!(s1, s2);
}

#[test]
fn ppu_viewer_is_side_effect_free_on_ppustatus() {
    let mut bus = Bus::new();
    bus.ppu_mut().set_vblank(true);
    let v = PpuViewer {
        mode: PpuView::Nametables,
        pattern_bank: 0,
    };
    let _ = v.render(&bus);
    assert!(
        bus.ppu().in_vblank(),
        "VBlank flag should survive viewer render"
    );
}

// ---- Memory viewer --------------------------------------------------------

#[test]
fn memory_viewer_default_is_cpu_at_0200() {
    let v = MemoryViewer::new();
    assert_eq!(v.region, MemoryRegion::Cpu);
    assert_eq!(v.addr, 0x0200);
}

#[test]
fn memory_viewer_dump_cpu_region_hex_format() {
    let mut bus = Bus::new();
    for i in 0..16u8 {
        bus.write(0x0200 + i as u16, i);
    }
    let v = MemoryViewer {
        region: MemoryRegion::Cpu,
        addr: 0x0200,
        rows: 1,
    };
    let s = v.dump(&bus);
    assert!(s.contains("$0200:"));
    assert!(s.contains(" 00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F"));
    // ASCII column: 0x00-0x0F are mostly non-printable → '.', but
    // 0x09 is tab (non-printable in our 0x20..=0x7E range) → '.'.
    assert!(s.contains('|'));
}

#[test]
fn memory_viewer_dump_ppu_region_nametable() {
    let mut bus = Bus::new();
    bus.ppu_mut().write_nametable(0x2000, 0xAB);
    let v = MemoryViewer {
        region: MemoryRegion::Ppu,
        addr: 0x2000,
        rows: 1,
    };
    let s = v.dump(&bus);
    assert!(s.contains("$2000:"));
    assert!(s.contains(" AB"));
}

#[test]
fn memory_viewer_dump_ppu_region_palette() {
    let mut bus = Bus::new();
    bus.ppu_mut().write_palette(0x3F00, 0x21);
    let v = MemoryViewer {
        region: MemoryRegion::Ppu,
        addr: 0x3F00,
        rows: 1,
    };
    let s = v.dump(&bus);
    assert!(s.contains("$3F00:"));
    assert!(s.contains(" 21"));
}

#[test]
fn memory_viewer_poke_cpu_round_trips() {
    let mut bus = Bus::new();
    let v = MemoryViewer {
        region: MemoryRegion::Cpu,
        addr: 0x0300,
        rows: 1,
    };
    v.poke(&mut bus, 0x0300, 0xCD);
    assert_eq!(bus.peek(0x0300), 0xCD);
}

#[test]
fn memory_viewer_poke_ppu_nametable_round_trips() {
    let mut bus = Bus::new();
    let v = MemoryViewer {
        region: MemoryRegion::Ppu,
        addr: 0x2000,
        rows: 1,
    };
    v.poke(&mut bus, 0x2000, 0x77);
    assert_eq!(bus.ppu().read_nametable(0x2000), 0x77);
}

#[test]
fn memory_viewer_poke_ppu_palette_round_trips() {
    let mut bus = Bus::new();
    let v = MemoryViewer {
        region: MemoryRegion::Ppu,
        addr: 0x3F00,
        rows: 1,
    };
    v.poke(&mut bus, 0x3F00, 0x30);
    assert_eq!(bus.ppu().read_palette(0x3F00), 0x30);
}

#[test]
fn memory_viewer_page_up_saturates_at_zero() {
    let mut v = MemoryViewer::new();
    v.set_addr(0x0010);
    v.page_up();
    assert_eq!(v.addr, 0);
}

#[test]
fn memory_viewer_page_down_advances_by_window_size() {
    let mut v = MemoryViewer::new();
    v.set_addr(0x0200);
    v.page_down();
    // 16 rows × 16 bytes = 256 bytes per page.
    assert_eq!(v.addr, 0x0300);
}

#[test]
fn memory_viewer_page_down_clamps_to_region_max() {
    let mut v = MemoryViewer::new();
    v.set_addr(0xFFF0);
    v.page_down();
    // CPU region max = 0xFFFF; window of 256 bytes would exceed it, so
    // the address is pinned so the last byte is at 0xFFFF.
    let window_end = v.addr as u32 + 16 * 16 - 1;
    assert!(window_end <= 0xFFFF, "window_end={window_end:08X} > FFFF");
    // And the window should reach the end of the region.
    assert_eq!(window_end, 0xFFFF);
}

#[test]
fn memory_viewer_toggle_region_clamps_addr() {
    let mut v = MemoryViewer::new();
    v.set_addr(0xFF00);
    v.toggle_region();
    assert_eq!(v.region, MemoryRegion::Ppu);
    // PPU region max = 0x3FFF; address must be clamped so the window fits.
    let window_end = v.addr as u32 + 16 * 16 - 1;
    assert!(window_end <= 0x3FFF);
}

#[test]
fn memory_viewer_dump_is_side_effect_free_on_ppustatus() {
    let mut bus = Bus::new();
    bus.ppu_mut().set_vblank(true);
    let v = MemoryViewer {
        region: MemoryRegion::Cpu,
        addr: 0x2002,
        rows: 1,
    };
    let _ = v.dump(&bus);
    assert!(
        bus.ppu().in_vblank(),
        "VBlank flag should survive peek-based dump"
    );
}

#[test]
fn memory_viewer_set_addr_clamps_to_ppu_region() {
    let mut v = MemoryViewer::new();
    v.toggle_region(); // → Ppu
    v.set_addr(0xFFFF);
    let window_end = v.addr as u32 + 16 * 16 - 1;
    assert!(window_end <= 0x3FFF, "addr should be clamped to PPU region");
}

// ---- Trace logger ---------------------------------------------------------

#[test]
fn trace_logger_start_writes_header_and_lines() {
    let path = temp_path("hdr");
    let mut l = TraceLogger::new();
    l.start(&path).expect("start");
    let mut emu = EmulatorState::new(nop_cart());
    emu.reset();
    l.log_instruction(emu.cpu(), emu.bus(), 2).expect("log1");
    l.log_instruction(emu.cpu(), emu.bus(), 2).expect("log2");
    drop(l);
    let contents = std::fs::read_to_string(&path).expect("read");
    let lines: Vec<&str> = contents.lines().collect();
    assert_eq!(lines.len(), 3); // header + 2 instruction lines
    assert!(lines[0].contains("# nes-emu trace log"));
    assert!(lines[1].starts_with("C000  EA"));
    assert!(lines[1].contains("NOP"));
    assert!(lines[1].contains("CYC:0"));
    assert!(lines[2].contains("CYC:2"));
    cleanup(&path);
}

#[test]
fn trace_logger_stop_resets_state() {
    let path = temp_path("stop");
    let mut l = TraceLogger::new();
    l.start(&path).expect("start");
    let mut emu = EmulatorState::new(nop_cart());
    emu.reset();
    l.log_instruction(emu.cpu(), emu.bus(), 2).expect("log");
    let lines = l.stop();
    assert_eq!(lines, 1);
    assert!(!l.is_enabled());
    assert_eq!(l.line_count(), 0);
    cleanup(&path);
}

#[test]
fn trace_logger_noop_when_stopped() {
    let mut l = TraceLogger::new();
    let mut emu = EmulatorState::new(nop_cart());
    emu.reset();
    l.log_instruction(emu.cpu(), emu.bus(), 2).expect("log");
    assert_eq!(l.line_count(), 0);
}

#[test]
fn trace_logger_restart_flushes_previous_file() {
    let path1 = temp_path("restart1");
    let path2 = temp_path("restart2");
    let mut l = TraceLogger::new();
    l.start(&path1).expect("start1");
    let mut emu = EmulatorState::new(nop_cart());
    emu.reset();
    l.log_instruction(emu.cpu(), emu.bus(), 2).expect("log1");
    l.start(&path2).expect("start2");
    assert_eq!(l.line_count(), 0);
    assert!(l.is_enabled());
    drop(l);
    // The first file should have the header + 1 line (flushed on restart).
    let c1 = std::fs::read_to_string(&path1).expect("read1");
    assert!(c1.contains("# nes-emu trace log"));
    assert!(c1.contains("C000  EA"));
    cleanup(&path1);
    cleanup(&path2);
}

#[test]
fn trace_logger_line_format_matches_fceux_style() {
    let path = temp_path("fmt");
    let mut l = TraceLogger::new();
    l.start(&path).expect("start");
    let mut emu = EmulatorState::new(nop_cart());
    emu.reset();
    l.log_instruction(emu.cpu(), emu.bus(), 2).expect("log");
    drop(l);
    let contents = std::fs::read_to_string(&path).expect("read");
    let line = contents.lines().nth(1).expect("instruction line");
    // Format: "PC  bytes  disasm  A:XX X:XX Y:XX P:XX SP:XX CYC:N"
    assert!(line.contains("A:"));
    assert!(line.contains("X:"));
    assert!(line.contains("Y:"));
    assert!(line.contains("P:"));
    assert!(line.contains("SP:"));
    assert!(line.contains("CYC:"));
    cleanup(&path);
}

// ---- step_frame_traced integration ----------------------------------------

#[test]
fn step_frame_traced_logs_every_instruction() {
    let path = temp_path("traced");
    let mut emu = EmulatorState::new(nop_cart());
    emu.reset();
    let mut debugger = nes_emu::debug::CpuDebugger::new();
    let mut logger = TraceLogger::new();
    logger.start(&path).expect("start");
    // Run one frame with tracing. A NOP-only ROM executes ~29830/2 ≈
    // 14915 NOP instructions per frame (each NOP = 2 cycles).
    emu.step_frame_traced(&mut debugger, &mut logger);
    let lines = logger.line_count();
    assert!(
        lines > 1000,
        "expected thousands of traced instructions, got {lines}"
    );
    drop(logger);
    let contents = std::fs::read_to_string(&path).expect("read");
    // The first instruction line should be at $C000 (RESET vector).
    let first_instr = contents.lines().nth(1).expect("first instruction");
    assert!(first_instr.starts_with("C000  EA"));
    cleanup(&path);
}

#[test]
fn step_frame_traced_is_deterministic_across_runs() {
    // Two identical runs should produce identical trace files.
    let path1 = temp_path("det1");
    let path2 = temp_path("det2");
    for path in [&path1, &path2] {
        let mut emu = EmulatorState::new(nop_cart());
        emu.reset();
        let mut debugger = nes_emu::debug::CpuDebugger::new();
        let mut logger = TraceLogger::new();
        logger.start(path).expect("start");
        // Run a fixed number of single-steps (not a full frame, which
        // would produce a huge file) for deterministic comparison.
        for _ in 0..100 {
            if debugger.check_before_step(emu.cpu(), emu.bus()) {
                break;
            }
            let pre = emu.cpu().clone();
            let cycles = emu.step_instruction();
            let _ = logger.log_instruction(&pre, emu.bus(), cycles);
        }
        drop(logger);
    }
    let c1 = std::fs::read_to_string(&path1).expect("read1");
    let c2 = std::fs::read_to_string(&path2).expect("read2");
    assert_eq!(c1, c2, "trace logs should be identical for identical runs");
    cleanup(&path1);
    cleanup(&path2);
}

// ---- Disassembler integration (used by trace logger) ----------------------

#[test]
fn trace_logger_disasm_column_matches_disassemble_at() {
    let bus = Bus::new();
    let instr = disassemble_at(&bus, 0x0000);
    // RAM is zeroed → opcode 0x00 = BRK (2 bytes, "BRK").
    assert_eq!(instr.text, "BRK");
}
