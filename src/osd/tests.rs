//! Inline unit tests for the OSD overlay (M30).

use super::*;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[test]
fn default_disabled() {
    let o = Osd::default();
    assert!(!o.enabled());
}

#[test]
fn toggle_flips_state() {
    let mut o = Osd::default();
    assert!(o.toggle());
    assert!(!o.toggle());
}

#[test]
fn fps_zero_before_any_frame() {
    let o = Osd::default();
    assert_eq!(o.fps(), 0.0);
}

#[test]
fn fps_after_uniform_intervals() {
    let mut o = Osd::default();
    let mut t = Instant::now();
    for _ in 0..10 {
        o.record_frame(t);
        t += Duration::from_millis(16);
    }
    // ~62.5 fps (1000/16). Allow tolerance for float rounding.
    let fps = o.fps();
    assert!(fps > 60.0 && fps < 65.0, "fps was {fps}");
}

#[test]
fn fps_ignores_large_gap() {
    let mut o = Osd::default();
    let mut t = Instant::now();
    for _ in 0..5 {
        o.record_frame(t);
        t += Duration::from_millis(16);
    }
    // Simulate a 2-second pause (e.g. debugger stop). The gap should
    // be ignored, not recorded as a 2-second interval.
    t += Duration::from_secs(2);
    o.record_frame(t);
    // The 5 good intervals are still in the buffer; fps should still
    // be ~62.5, not tanked by the 2-second gap.
    let fps = o.fps();
    assert!(fps > 60.0 && fps < 65.0, "fps was {fps}");
}

#[test]
fn render_no_op_when_disabled() {
    let mut fb = vec![0u32; 256 * 240];
    let o = Osd::default();
    o.render(&mut fb, 256, 240, &["HELLO"]);
    // No pixels should have changed.
    assert!(fb.iter().all(|&p| p == 0));
}

#[test]
fn render_writes_pixels_when_enabled() {
    let mut fb = vec![0u32; 256 * 240];
    let mut o = Osd::default();
    o.set_enabled(true);
    o.render(&mut fb, 256, 240, &["A"]);
    // The 'A' glyph has set bits in its first row (0x18) — at least
    // one pixel in the top-left 8x8 cell should be the FG colour.
    let fg_pixels = fb.iter().filter(|&&p| p == OSD_FG_ARGB).count();
    assert!(
        fg_pixels > 0,
        "expected non-zero FG pixels, got {fg_pixels}"
    );
}

#[test]
fn render_writes_bg_for_blank_cells() {
    let mut fb = vec![0x12345678u32; 256 * 240];
    let mut o = Osd::default();
    o.set_enabled(true);
    o.render(&mut fb, 256, 240, &["A"]);
    // The 8x8 cell for 'A' should be entirely overwritten with either
    // FG or BG — no original pixels should remain in that cell.
    for y in 0..8 {
        for x in 0..8 {
            let p = fb[(y * 256 + x) as usize];
            assert!(
                p == OSD_FG_ARGB || p == OSD_BG_ARGB,
                "pixel ({x},{y}) = {p:#x} should be FG or BG"
            );
        }
    }
}

#[test]
fn render_clips_long_lines() {
    let mut fb = vec![0u32; 256 * 240];
    let mut o = Osd::default();
    o.set_enabled(true);
    // 40 chars * 8 px = 320 px > 256 — should not panic, just clip.
    let long = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    o.render(&mut fb, 256, 240, &[long]);
    // The first 32 glyphs fit (32 * 8 = 256 px); glyphs 33..40 are
    // clipped. Verify no panic occurred and that the first glyph was
    // drawn (fb[0] is the top-left pixel of 'A' row 0 = 0x18, bit 7
    // = 0 → BG pixel).
    assert_eq!(
        fb[0], OSD_BG_ARGB,
        "first pixel of first glyph should be BG"
    );
    assert!(
        fb.iter().any(|&p| p == OSD_FG_ARGB),
        "some FG pixels should be drawn"
    );
}

#[test]
fn render_clips_too_many_lines() {
    let mut fb = vec![0u32; 256 * 240];
    let mut o = Osd::default();
    o.set_enabled(true);
    // 240 / 8 = 30 lines fit; 50 lines should clip without panic.
    let lines: Vec<&str> = (0..50).map(|_| "A").collect();
    o.render(&mut fb, 256, 240, &lines);
    // The 31st line (y=240) would start past the framebuffer; verify
    // no panic occurred and some pixels were drawn.
    assert!(fb.iter().any(|&p| p == OSD_FG_ARGB));
}

#[test]
fn render_no_op_on_size_mismatch() {
    let mut fb = vec![0u32; 100];
    let mut o = Osd::default();
    o.set_enabled(true);
    o.render(&mut fb, 256, 240, &["A"]);
    // fb.len() != 256*240 → no writes.
    assert!(fb.iter().all(|&p| p == 0));
}

#[test]
fn build_lines_format() {
    let lines = build_lines(60.1, 4, "Megaman2", 2, true, 42);
    assert_eq!(lines.len(), 4);
    assert_eq!(lines[0], "FPS:60.1 MAP:4");
    assert_eq!(lines[1], "GAME:Megaman2");
    assert_eq!(lines[2], "SLOT:3/10 OCCUPIED");
    assert_eq!(lines[3], "REWIND:42");
}

#[test]
fn build_lines_empty_slot() {
    let lines = build_lines(0.0, 0, "?", 0, false, 0);
    assert_eq!(lines[2], "SLOT:1/10 EMPTY");
}

#[test]
fn game_name_from_path_strips_dir_and_ext() {
    let p = PathBuf::from("/home/user/roms/Super Mario Bros.nes");
    assert_eq!(game_name_from_path(&p), "Super Mario Bros");
}

#[test]
fn game_name_from_path_no_ext() {
    let p = PathBuf::from("rom");
    assert_eq!(game_name_from_path(&p), "rom");
}

#[test]
fn game_name_from_path_missing() {
    let p = PathBuf::from("/");
    assert_eq!(game_name_from_path(&p), "?");
}
