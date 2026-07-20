//! On-screen display integration tests (M30).
//!
//! These tests exercise the [`Osd`] overlay renderer and the
//! [`build_lines`] / [`game_name_from_path`] helpers. They verify that:
//!
//! - The OSD is disabled by default and toggled correctly.
//! - FPS is computed from a rolling window of inter-frame intervals.
//! - `render` writes FG pixels for set glyph bits and BG pixels for
//!   unset bits, with no leftover original pixels in the glyph cell.
//! - Long lines and too-many-lines are clipped without panic.
//! - `render` is a no-op when disabled or when the framebuffer size does
//!   not match `width * height`.
//! - `build_lines` produces the expected 4-line OSD format.
//! - `game_name_from_path` strips directory and extension.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use nes_emu::osd::{
    build_lines, game_name_from_path, Osd, FPS_WINDOW, GLYPH_H, GLYPH_W, OSD_BG_ARGB, OSD_FG_ARGB,
};
use nes_emu::region::Region;

#[test]
fn osd_default_disabled() {
    let o = Osd::default();
    assert!(!o.enabled());
}

#[test]
fn osd_toggle_flips_state() {
    let mut o = Osd::default();
    assert!(o.toggle(), "first toggle → on");
    assert!(o.enabled());
    assert!(!o.toggle(), "second toggle → off");
    assert!(!o.enabled());
}

#[test]
fn osd_set_enabled_explicit() {
    let mut o = Osd::default();
    o.set_enabled(true);
    assert!(o.enabled());
    o.set_enabled(false);
    assert!(!o.enabled());
}

#[test]
fn osd_fps_zero_before_any_frame() {
    let o = Osd::default();
    assert_eq!(o.fps(), 0.0);
}

#[test]
fn osd_fps_after_uniform_16ms_intervals() {
    let mut o = Osd::default();
    let mut t = Instant::now();
    for _ in 0..10 {
        o.record_frame(t);
        t += Duration::from_millis(16);
    }
    let fps = o.fps();
    // 1000/16 = 62.5 fps. Allow a small tolerance for float rounding.
    assert!(fps > 60.0 && fps < 65.0, "fps was {fps}");
}

#[test]
fn osd_fps_after_uniform_33ms_intervals() {
    let mut o = Osd::default();
    let mut t = Instant::now();
    for _ in 0..15 {
        o.record_frame(t);
        t += Duration::from_millis(33);
    }
    let fps = o.fps();
    // 1000/33 ≈ 30.3 fps.
    assert!(fps > 29.0 && fps < 31.5, "fps was {fps}");
}

#[test]
fn osd_fps_uses_rolling_window() {
    let mut o = Osd::default();
    let mut t = Instant::now();
    // Fill the window with 60 fps intervals.
    for _ in 0..FPS_WINDOW {
        o.record_frame(t);
        t += Duration::from_millis(16);
    }
    assert!(o.fps() > 60.0 && o.fps() < 65.0);

    // Now push 30 fps intervals until they fill the window.
    for _ in 0..FPS_WINDOW {
        o.record_frame(t);
        t += Duration::from_millis(33);
    }
    // After the window fully turns over, fps should be ~30.
    let fps = o.fps();
    assert!(fps > 29.0 && fps < 31.5, "fps after turnover was {fps}");
}

#[test]
fn osd_fps_ignores_large_gap_from_pause() {
    let mut o = Osd::default();
    let mut t = Instant::now();
    for _ in 0..5 {
        o.record_frame(t);
        t += Duration::from_millis(16);
    }
    // Simulate a 2-second debugger pause. The gap should be ignored.
    t += Duration::from_secs(2);
    o.record_frame(t);
    let fps = o.fps();
    assert!(
        fps > 60.0 && fps < 65.0,
        "fps should ignore the 2s gap, got {fps}"
    );
}

#[test]
fn osd_render_no_op_when_disabled() {
    let mut fb = vec![0x12345678u32; 256 * 240];
    let o = Osd::default(); // disabled
    o.render(&mut fb, 256, 240, &["HELLO"]);
    assert!(
        fb.iter().all(|&p| p == 0x12345678),
        "no pixels should change"
    );
}

#[test]
fn osd_render_no_op_on_size_mismatch() {
    let mut fb = vec![0u32; 100]; // wrong size
    let mut o = Osd::default();
    o.set_enabled(true);
    o.render(&mut fb, 256, 240, &["A"]);
    assert!(fb.iter().all(|&p| p == 0));
}

#[test]
fn osd_render_writes_fg_pixels_for_set_bits() {
    let mut fb = vec![0u32; 256 * 240];
    let mut o = Osd::default();
    o.set_enabled(true);
    o.render(&mut fb, 256, 240, &["A"]);
    let fg_count = fb.iter().filter(|&&p| p == OSD_FG_ARGB).count();
    assert!(fg_count > 0, "expected some FG pixels for 'A'");
    // The 'A' glyph first row is 0x18 = 00011000 → 2 set bits.
    // Verify those specific pixels are FG.
    assert_eq!(fb[3], OSD_FG_ARGB, "row 0 col 3 should be FG");
    assert_eq!(fb[4], OSD_FG_ARGB, "row 0 col 4 should be FG");
}

#[test]
fn osd_render_writes_bg_for_unset_bits() {
    let mut fb = vec![0xABCDEF01u32; 256 * 240];
    let mut o = Osd::default();
    o.set_enabled(true);
    o.render(&mut fb, 256, 240, &["A"]);
    // The 8x8 cell for 'A' should be entirely overwritten with FG or BG.
    for y in 0..GLYPH_H {
        for x in 0..GLYPH_W {
            let p = fb[(y * 256 + x) as usize];
            assert!(
                p == OSD_FG_ARGB || p == OSD_BG_ARGB,
                "pixel ({x},{y}) = {p:#x} should be FG or BG"
            );
        }
    }
    // Pixels outside the glyph cell should be untouched.
    assert_eq!(fb[8], 0xABCDEF01, "row 0 col 8 should be untouched");
    assert_eq!(fb[256 * 8], 0xABCDEF01, "row 8 col 0 should be untouched");
}

#[test]
fn osd_render_multiple_lines_stack_vertically() {
    let mut fb = vec![0u32; 256 * 240];
    let mut o = Osd::default();
    o.set_enabled(true);
    o.render(&mut fb, 256, 240, &["A", "B"]);
    // Line 0 (A) occupies rows 0..8; line 1 (B) occupies rows 8..16.
    // Both glyphs have set bits in their first row.
    let a_row0 = fb[3] == OSD_FG_ARGB; // 'A' row 0 has bit at col 3
    let b_row8 = fb[(8 * 256) + 1] == OSD_FG_ARGB; // 'B' row 0 has bit at col 1
    assert!(a_row0, "line 0 'A' should be drawn at y=0");
    assert!(b_row8, "line 1 'B' should be drawn at y=8");
}

#[test]
fn osd_render_clips_long_line_without_panic() {
    let mut fb = vec![0u32; 256 * 240];
    let mut o = Osd::default();
    o.set_enabled(true);
    // 40 chars * 8 px = 320 px > 256 px width — should clip, not panic.
    let long = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    o.render(&mut fb, 256, 240, &[long]);
    // The last full glyph that fits starts at x=248 (cols 248..255).
    // Pixels at x=256+ don't exist; verify no panic occurred.
    assert!(fb.iter().any(|&p| p == OSD_FG_ARGB));
}

#[test]
fn osd_render_clips_too_many_lines_without_panic() {
    let mut fb = vec![0u32; 256 * 240];
    let mut o = Osd::default();
    o.set_enabled(true);
    // 240 / 8 = 30 lines fit; 50 lines should clip.
    let lines: Vec<&str> = (0..50).map(|_| "A").collect();
    o.render(&mut fb, 256, 240, &lines);
    assert!(fb.iter().any(|&p| p == OSD_FG_ARGB));
}

#[test]
fn osd_render_empty_lines_is_no_op_on_pixels() {
    let mut fb = vec![0x11223344u32; 256 * 240];
    let mut o = Osd::default();
    o.set_enabled(true);
    o.render(&mut fb, 256, 240, &[]);
    assert!(fb.iter().all(|&p| p == 0x11223344));
}

#[test]
fn osd_render_unknown_char_draws_blank_cell() {
    let mut fb = vec![0u32; 256 * 240];
    let mut o = Osd::default();
    o.set_enabled(true);
    // Emoji is not in the font → should draw as a blank (all-BG) cell.
    o.render(&mut fb, 256, 240, &["\u{1F600}"]);
    for y in 0..GLYPH_H {
        for x in 0..GLYPH_W {
            assert_eq!(
                fb[(y * 256 + x) as usize],
                OSD_BG_ARGB,
                "unknown char should fill cell with BG"
            );
        }
    }
}

#[test]
fn osd_render_space_draws_blank_cell() {
    let mut fb = vec![0u32; 256 * 240];
    let mut o = Osd::default();
    o.set_enabled(true);
    o.render(&mut fb, 256, 240, &[" "]);
    for y in 0..GLYPH_H {
        for x in 0..GLYPH_W {
            assert_eq!(fb[(y * 256 + x) as usize], OSD_BG_ARGB);
        }
    }
}

#[test]
fn osd_render_full_alphabet_writes_pixels() {
    let mut fb = vec![0u32; 256 * 240];
    let mut o = Osd::default();
    o.set_enabled(true);
    let text = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    o.render(&mut fb, 256, 240, &[text]);
    let fg_count = fb.iter().filter(|&&p| p == OSD_FG_ARGB).count();
    // 26 glyphs, each with at least one set bit → fg_count > 26.
    assert!(fg_count > 26, "expected >26 FG pixels, got {fg_count}");
}

#[test]
fn osd_render_digits_writes_pixels() {
    let mut fb = vec![0u32; 256 * 240];
    let mut o = Osd::default();
    o.set_enabled(true);
    o.render(&mut fb, 256, 240, &["0123456789"]);
    let fg_count = fb.iter().filter(|&&p| p == OSD_FG_ARGB).count();
    assert!(
        fg_count > 10,
        "expected >10 FG pixels for digits, got {fg_count}"
    );
}

#[test]
fn osd_render_lowercase_writes_pixels() {
    let mut fb = vec![0u32; 256 * 240];
    let mut o = Osd::default();
    o.set_enabled(true);
    o.render(&mut fb, 256, 240, &["abcdefghijklmnopqrstuvwxyz"]);
    let fg_count = fb.iter().filter(|&&p| p == OSD_FG_ARGB).count();
    assert!(
        fg_count > 26,
        "expected >26 FG pixels for lowercase, got {fg_count}"
    );
}

#[test]
fn osd_render_punctuation_writes_pixels() {
    let mut fb = vec![0u32; 256 * 240];
    let mut o = Osd::default();
    o.set_enabled(true);
    // Punctuation used by the OSD lines: `: . / - % [ ]`
    o.render(&mut fb, 256, 240, &[":./-%[]_"]);
    let fg_count = fb.iter().filter(|&&p| p == OSD_FG_ARGB).count();
    assert!(fg_count > 0, "expected some FG pixels for punctuation");
}

#[test]
fn osd_render_glyph_width_is_8px() {
    assert_eq!(GLYPH_W, 8);
    assert_eq!(GLYPH_H, 8);
}

#[test]
fn osd_render_does_not_overwrite_pixels_outside_cells() {
    let mut fb = vec![0xDEADBEEFu32; 256 * 240];
    let mut o = Osd::default();
    o.set_enabled(true);
    o.render(&mut fb, 256, 240, &["A"]); // 1 glyph → 8x8 cell at (0,0)
                                         // Pixel at (8, 0) — just past the glyph — should be untouched.
    assert_eq!(fb[8], 0xDEADBEEF);
    // Pixel at (0, 8) — just below the glyph — should be untouched.
    assert_eq!(fb[8 * 256], 0xDEADBEEF);
    // Pixel at (100, 100) — far from the glyph — should be untouched.
    assert_eq!(fb[100 * 256 + 100], 0xDEADBEEF);
}

// ---------------------------------------------------------------------------
// build_lines
// ---------------------------------------------------------------------------

#[test]
fn build_lines_returns_four_lines() {
    let lines = build_lines(60.0, 4, "Megaman2", 2, true, 42, Region::Ntsc);
    assert_eq!(lines.len(), 4);
}

#[test]
fn build_lines_fps_line_format() {
    let lines = build_lines(59.94, 0, "Test", 0, false, 0, Region::Ntsc);
    assert_eq!(lines[0], "FPS:59.9 MAP:0 NTSC");
}

#[test]
fn build_lines_fps_one_decimal_place() {
    let lines = build_lines(60.123, 0, "X", 0, false, 0, Region::Ntsc);
    assert_eq!(lines[0], "FPS:60.1 MAP:0 NTSC");
}

#[test]
fn build_lines_game_line() {
    let lines = build_lines(60.0, 0, "Super Mario Bros", 0, false, 0, Region::Ntsc);
    assert_eq!(lines[1], "GAME:Super Mario Bros");
}

#[test]
fn build_lines_slot_occupied() {
    let lines = build_lines(60.0, 0, "X", 3, true, 0, Region::Ntsc);
    assert_eq!(lines[2], "SLOT:4/10 OCCUPIED");
}

#[test]
fn build_lines_slot_empty() {
    let lines = build_lines(60.0, 0, "X", 0, false, 0, Region::Ntsc);
    assert_eq!(lines[2], "SLOT:1/10 EMPTY");
}

#[test]
fn build_lines_slot_index_is_one_based() {
    // slot 0 → "SLOT:1/10", slot 9 → "SLOT:10/10"
    let lines = build_lines(60.0, 0, "X", 9, true, 0, Region::Ntsc);
    assert_eq!(lines[2], "SLOT:10/10 OCCUPIED");
}

#[test]
fn build_lines_rewind_line() {
    let lines = build_lines(60.0, 0, "X", 0, false, 60, Region::Ntsc);
    assert_eq!(lines[3], "REWIND:60");
}

#[test]
fn build_lines_rewind_zero() {
    let lines = build_lines(60.0, 0, "X", 0, false, 0, Region::Ntsc);
    assert_eq!(lines[3], "REWIND:0");
}

#[test]
fn build_lines_mapper_number_displayed() {
    let lines = build_lines(60.0, 4, "X", 0, false, 0, Region::Ntsc);
    assert!(lines[0].contains("MAP:4"));
    let lines2 = build_lines(60.0, 255, "X", 0, false, 0, Region::Ntsc);
    assert!(lines2[0].contains("MAP:255"));
}

#[test]
fn build_lines_region_displayed() {
    // M32: the region short name is appended to the FPS/MAP line.
    let ntsc = build_lines(60.0, 0, "X", 0, false, 0, Region::Ntsc);
    assert!(ntsc[0].ends_with("NTSC"));
    let pal = build_lines(50.0, 0, "X", 0, false, 0, Region::Pal);
    assert!(pal[0].ends_with("PAL"));
    let dendy = build_lines(50.0, 0, "X", 0, false, 0, Region::Dendy);
    assert!(dendy[0].ends_with("DENDY"));
}

// ---------------------------------------------------------------------------
// game_name_from_path
// ---------------------------------------------------------------------------

#[test]
fn game_name_strips_directory_and_extension() {
    let p = PathBuf::from("/home/user/roms/Super Mario Bros.nes");
    assert_eq!(game_name_from_path(&p), "Super Mario Bros");
}

#[test]
fn game_name_strips_extension_only() {
    let p = PathBuf::from("Megaman2.nes");
    assert_eq!(game_name_from_path(&p), "Megaman2");
}

#[test]
fn game_name_no_extension() {
    let p = PathBuf::from("rom");
    assert_eq!(game_name_from_path(&p), "rom");
}

#[test]
fn game_name_relative_path() {
    let p = PathBuf::from("./roms/game.nes");
    assert_eq!(game_name_from_path(&p), "game");
}

#[test]
fn game_name_multiple_dots() {
    let p = PathBuf::from("Super.Mario.Bros.nes");
    assert_eq!(game_name_from_path(&p), "Super.Mario.Bros");
}

#[test]
fn game_name_root_returns_question_mark() {
    let p = PathBuf::from("/");
    assert_eq!(game_name_from_path(&p), "?");
}

#[test]
fn game_name_empty_returns_question_mark() {
    let p = PathBuf::from("");
    assert_eq!(game_name_from_path(&p), "?");
}
