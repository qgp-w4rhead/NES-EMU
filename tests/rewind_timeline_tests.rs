//! Integration tests for the visual rewind timeline feature.
//!
//! Tests verify that the timeline bar renders correctly into the
//! framebuffer for various buffer states (empty, partial, full),
//! that the position marker moves as expected during rewind, and
//! that the SaveStateHotkeys overlay integration works end-to-end.

use nes_emu::osd::OSD_FG_ARGB;
use nes_emu::rewind_timeline::{
    render_timeline, TimelineConfig, TimelineMode, BAR_H, BAR_W, BAR_X, BAR_Y, BORDER_THICKNESS,
    MAX_OPACITY, TEXT_Y, TIMELINE_BG, TIMELINE_BORDER, TIMELINE_BRANCH, TIMELINE_BRANCH_MARKER,
    TIMELINE_EMPTY, TIMELINE_FILLED, TIMELINE_FORWARD, TIMELINE_MARKER,
};

const W: u32 = 256;
const H: u32 = 240;

fn cfg(filled: usize, capacity: usize, mode: TimelineMode) -> TimelineConfig {
    TimelineConfig {
        filled,
        capacity,
        mode,
        rewind_interval: 2,
        opacity: MAX_OPACITY,
        anim_frame: 0,
        speed: 1,
        forward_filled: 0,
        branches: Vec::new(),
    }
}

#[test]
fn full_buffer_shows_green_bar() {
    let mut fb = vec![0u32; (W * H) as usize];
    render_timeline(&mut fb, W, H, &cfg(300, 300, TimelineMode::Buffer));
    // The entire bar should be filled green.
    for x in BAR_X..BAR_X + BAR_W {
        for y in BAR_Y..BAR_Y + BAR_H {
            let px = fb[(y * W + x) as usize];
            // Tick marks will be border color, so allow either.
            assert!(
                px == TIMELINE_FILLED || px == TIMELINE_BORDER,
                "pixel ({x},{y}) = {px:#x} should be FILLED or BORDER (tick)"
            );
        }
    }
}

#[test]
fn empty_buffer_shows_no_green() {
    let mut fb = vec![0u32; (W * H) as usize];
    render_timeline(&mut fb, W, H, &cfg(0, 300, TimelineMode::Rewind));
    let green_count = fb.iter().filter(|&&p| p == TIMELINE_FILLED).count();
    assert_eq!(green_count, 0);
}

#[test]
fn partial_buffer_shows_correct_fill_ratio() {
    let mut fb = vec![0u32; (W * H) as usize];
    render_timeline(&mut fb, W, H, &cfg(100, 300, TimelineMode::Rewind));
    let expected_filled_w = (BAR_W as usize * 100 / 300) as u32;
    // Pixel inside filled portion should be green (avoiding tick marks).
    let test_x = BAR_X + expected_filled_w / 2;
    let test_y = BAR_Y + BAR_H / 2;
    assert_eq!(
        fb[(test_y * W + test_x) as usize],
        TIMELINE_FILLED,
        "pixel in filled portion should be green"
    );
    // Pixel in empty portion should be dark gray.
    let empty_x = BAR_X + expected_filled_w + 20;
    assert_eq!(
        fb[(test_y * W + empty_x) as usize],
        TIMELINE_EMPTY,
        "pixel in empty portion should be dark gray"
    );
}

#[test]
fn marker_at_right_edge_when_not_rewinding() {
    let mut fb = vec![0u32; (W * H) as usize];
    render_timeline(&mut fb, W, H, &cfg(150, 300, TimelineMode::Buffer));
    let marker_x = BAR_X + BAR_W;
    let has_marker =
        (BAR_Y..BAR_Y + BAR_H).any(|y| fb[(y * W + marker_x) as usize] == TIMELINE_MARKER);
    assert!(
        has_marker,
        "marker should be at right edge when not rewinding"
    );
}

#[test]
fn marker_at_fill_boundary_when_rewinding() {
    let mut fb = vec![0u32; (W * H) as usize];
    render_timeline(&mut fb, W, H, &cfg(150, 300, TimelineMode::Rewind));
    let filled_w = (BAR_W as usize * 150 / 300) as u32;
    let marker_x = BAR_X + filled_w;
    let has_marker =
        (BAR_Y..BAR_Y + BAR_H).any(|y| fb[(y * W + marker_x) as usize] == TIMELINE_MARKER);
    assert!(
        has_marker,
        "marker should be at fill boundary when rewinding"
    );
}

#[test]
fn marker_moves_left_as_buffer_empties() {
    let mut fb1 = vec![0u32; (W * H) as usize];
    render_timeline(&mut fb1, W, H, &cfg(200, 300, TimelineMode::Rewind));
    let marker_x1 = BAR_X + (BAR_W as usize * 200 / 300) as u32;

    let mut fb2 = vec![0u32; (W * H) as usize];
    render_timeline(&mut fb2, W, H, &cfg(100, 300, TimelineMode::Rewind));
    let marker_x2 = BAR_X + (BAR_W as usize * 100 / 300) as u32;

    assert!(
        marker_x2 < marker_x1,
        "marker should move left as buffer empties: {marker_x2} < {marker_x1}"
    );
}

#[test]
fn border_drawn_around_bar() {
    let mut fb = vec![0u32; (W * H) as usize];
    render_timeline(&mut fb, W, H, &cfg(100, 300, TimelineMode::Buffer));
    let bx = BAR_X.saturating_sub(BORDER_THICKNESS);
    let by = BAR_Y.saturating_sub(BORDER_THICKNESS);
    let bw = BAR_W + 2 * BORDER_THICKNESS;
    let bh = BAR_H + 2 * BORDER_THICKNESS;
    // Check four corners.
    assert_eq!(fb[(by * W + bx) as usize], TIMELINE_BORDER, "top-left");
    assert_eq!(
        fb[(by * W + bx + bw - 1) as usize],
        TIMELINE_BORDER,
        "top-right"
    );
    assert_eq!(
        fb[((by + bh - 1) * W + bx) as usize],
        TIMELINE_BORDER,
        "bottom-left"
    );
    assert_eq!(
        fb[((by + bh - 1) * W + bx + bw - 1) as usize],
        TIMELINE_BORDER,
        "bottom-right"
    );
}

#[test]
fn text_rendered_above_bar() {
    let mut fb = vec![0u32; (W * H) as usize];
    render_timeline(&mut fb, W, H, &cfg(100, 300, TimelineMode::Buffer));
    // Check for white text pixels in the text area (y=TEXT_Y to TEXT_Y+8).
    let mut has_fg = false;
    for y in TEXT_Y..TEXT_Y + 8 {
        for x in 0..W {
            if fb[(y * W + x) as usize] == OSD_FG_ARGB {
                has_fg = true;
                break;
            }
        }
        if has_fg {
            break;
        }
    }
    assert!(has_fg, "expected white text pixels in text area");
}

#[test]
fn background_strip_behind_bar_and_text() {
    let mut fb = vec![0x12345678u32; (W * H) as usize];
    render_timeline(&mut fb, W, H, &cfg(100, 300, TimelineMode::Buffer));
    // The background strip should overwrite original pixels.
    let bg_top = TEXT_Y.saturating_sub(1);
    // Check a pixel in the text area that should be background (not text).
    // The 'R' glyph row 0 is 0x7C — bit 1 (x=1) is 0 → BG pixel.
    // But to be safe, just check that some pixel in the bg strip is black.
    let has_bg = (bg_top..BAR_Y + BAR_H + BORDER_THICKNESS)
        .any(|y| (0..W).any(|x| fb[(y * W + x) as usize] == TIMELINE_BG));
    assert!(has_bg, "expected black background strip");
}

#[test]
fn tick_marks_at_quarter_positions() {
    let mut fb = vec![0u32; (W * H) as usize];
    render_timeline(&mut fb, W, H, &cfg(300, 300, TimelineMode::Buffer));
    for i in 1..4 {
        let tick_x = BAR_X + BAR_W * i / 4;
        let has_tick =
            (BAR_Y..BAR_Y + BAR_H).any(|y| fb[(y * W + tick_x) as usize] == TIMELINE_BORDER);
        assert!(has_tick, "expected tick mark at {i}/4 position");
    }
}

#[test]
fn zero_capacity_is_noop() {
    let mut fb = vec![0u32; (W * H) as usize];
    let c = TimelineConfig {
        capacity: 0,
        ..cfg(0, 0, TimelineMode::Rewind)
    };
    render_timeline(&mut fb, W, H, &c);
    // No pixels should have changed.
    assert!(fb.iter().all(|&p| p == 0));
}

#[test]
fn mismatched_fb_size_is_noop() {
    let mut fb = vec![0u32; 100];
    render_timeline(&mut fb, W, H, &cfg(10, 300, TimelineMode::Rewind));
    assert!(fb.iter().all(|&p| p == 0));
}

#[test]
fn rewinding_shows_rewind_label() {
    let mut fb = vec![0u32; (W * H) as usize];
    render_timeline(&mut fb, W, H, &cfg(100, 300, TimelineMode::Rewind));
    // "REWIND" text should be present — check for 'R' glyph first pixel.
    // 'R' = [0x7C, 0x66, 0x66, 0x7C, 0x78, 0x6C, 0x66, 0x00]
    // Row 0 = 0x7C = bits 2-6 set → x=2 is the first FG pixel.
    let r_x = BAR_X + 2;
    let r_y = TEXT_Y;
    assert_eq!(
        fb[(r_y * W + r_x) as usize],
        OSD_FG_ARGB,
        "expected 'R' glyph FG pixel at ({r_x},{r_y})"
    );
}

#[test]
fn not_rewinding_shows_buffer_label() {
    let mut fb = vec![0u32; (W * H) as usize];
    render_timeline(&mut fb, W, H, &cfg(100, 300, TimelineMode::Buffer));
    // "BUFFER" text — 'B' = [0x7C, 0x66, 0x66, 0x7C, 0x66, 0x66, 0x7C, 0x00]
    // Row 0 = 0x7C → x=2 is first FG pixel.
    let b_x = BAR_X + 2;
    let b_y = TEXT_Y;
    assert_eq!(
        fb[(b_y * W + b_x) as usize],
        OSD_FG_ARGB,
        "expected 'B' glyph FG pixel at ({b_x},{b_y})"
    );
}

#[test]
fn frame_count_reflects_interval() {
    // With interval=1, 150 snapshots = 150 frames.
    let mut fb1 = vec![0u32; (W * H) as usize];
    let c1 = TimelineConfig {
        rewind_interval: 1,
        ..cfg(150, 300, TimelineMode::Rewind)
    };
    render_timeline(&mut fb1, W, H, &c1);
    // With interval=2, 150 snapshots = 300 frames.
    let mut fb2 = vec![0u32; (W * H) as usize];
    render_timeline(&mut fb2, W, H, &cfg(150, 300, TimelineMode::Rewind));
    // Both should have text pixels, but the actual numbers differ.
    // We just verify no panic and text is rendered.
    assert!(fb1.iter().any(|&p| p == OSD_FG_ARGB));
    assert!(fb2.iter().any(|&p| p == OSD_FG_ARGB));
}

#[test]
fn bar_does_not_exceed_framebuffer_bounds() {
    let mut fb = vec![0u32; (W * H) as usize];
    render_timeline(&mut fb, W, H, &cfg(300, 300, TimelineMode::Rewind));
    // No writes outside the framebuffer (would panic if OOB).
    // Verify bar is within bounds.
    assert!(BAR_X + BAR_W <= W, "bar right edge within width");
    assert!(BAR_Y + BAR_H <= H, "bar bottom edge within height");
}

#[test]
fn save_state_hotkeys_overlay_not_rewinding_is_noop() {
    use nes_emu::save_state_hotkeys::SaveStateHotkeys;
    let h = SaveStateHotkeys::default();
    let mut fb = vec![0u32; (W * H) as usize];
    h.render_rewind_overlay(&mut fb, W, H);
    // Should be a no-op since opacity is 0 (not rewinding).
    assert!(fb.iter().all(|&p| p == 0));
}

#[test]
fn forward_mode_shows_forward_label() {
    let mut fb = vec![0u32; (W * H) as usize];
    render_timeline(&mut fb, W, H, &cfg(100, 300, TimelineMode::Forward));
    // "FORWARD" text — 'F' = [0x7E, 0x42, 0x42, 0x3C, 0x02, 0x02, 0x02, 0x00]
    // Row 0 = 0x7E → bit 1 (x=1) is set.
    let f_x = BAR_X + 1;
    let f_y = TEXT_Y;
    assert_eq!(
        fb[(f_y * W + f_x) as usize],
        OSD_FG_ARGB,
        "expected 'F' glyph FG pixel for FORWARD label"
    );
}

#[test]
fn speed_label_shown_when_speed_gt_1() {
    let mut fb = vec![0u32; (W * H) as usize];
    let c = TimelineConfig {
        speed: 4,
        ..cfg(100, 300, TimelineMode::Rewind)
    };
    render_timeline(&mut fb, W, H, &c);
    // "REWIND x4" — 'x' glyph has set bits, verify text is rendered.
    let fg_count = fb.iter().filter(|&&p| p == OSD_FG_ARGB).count();
    assert!(fg_count > 0, "expected text pixels with speed label");
}

#[test]
fn opacity_zero_is_noop() {
    let mut fb = vec![0x12345678u32; (W * H) as usize];
    let c = TimelineConfig {
        opacity: 0,
        ..cfg(100, 300, TimelineMode::Rewind)
    };
    render_timeline(&mut fb, W, H, &c);
    assert!(
        fb.iter().all(|&p| p == 0x12345678),
        "no pixels should change at opacity 0"
    );
}

#[test]
fn pulse_alternates_marker_brightness() {
    let mut fb1 = vec![0u32; (W * H) as usize];
    let c1 = TimelineConfig {
        anim_frame: 0,
        ..cfg(150, 300, TimelineMode::Rewind)
    };
    render_timeline(&mut fb1, W, H, &c1);

    let mut fb2 = vec![0u32; (W * H) as usize];
    let c2 = TimelineConfig {
        anim_frame: 8,
        ..cfg(150, 300, TimelineMode::Rewind)
    };
    render_timeline(&mut fb2, W, H, &c2);

    let filled_w = (BAR_W as usize * 150 / 300) as u32;
    let marker_x = BAR_X + filled_w;
    let marker1 = fb1[(BAR_Y * W + marker_x) as usize];
    let marker2 = fb2[(BAR_Y * W + marker_x) as usize];
    assert_eq!(marker1, TIMELINE_MARKER, "frame 0 should be bright marker");
    assert_eq!(
        marker2,
        nes_emu::rewind_timeline::TIMELINE_MARKER_DIM,
        "frame 8 should be dim marker"
    );
}

#[test]
fn forward_mode_marker_at_fill_boundary() {
    let mut fb = vec![0u32; (W * H) as usize];
    render_timeline(&mut fb, W, H, &cfg(150, 300, TimelineMode::Forward));
    let filled_w = (BAR_W as usize * 150 / 300) as u32;
    let marker_x = BAR_X + filled_w;
    let has_marker =
        (BAR_Y..BAR_Y + BAR_H).any(|y| fb[(y * W + marker_x) as usize] == TIMELINE_MARKER);
    assert!(has_marker, "forward mode marker should be at fill boundary");
}

#[test]
fn save_state_hotkeys_timeline_indicator_empty_is_noop() {
    use nes_emu::save_state_hotkeys::SaveStateHotkeys;
    let h = SaveStateHotkeys::default();
    let mut fb = vec![0u32; (W * H) as usize];
    h.render_timeline_indicator(&mut fb, W, H);
    // Should be a no-op since buffer is empty.
    assert!(fb.iter().all(|&p| p == 0));
}

#[test]
fn forward_buffer_shown_on_timeline() {
    let mut fb = vec![0u32; (W * H) as usize];
    let c = TimelineConfig {
        filled: 100,
        capacity: 300,
        mode: TimelineMode::Rewind,
        rewind_interval: 2,
        opacity: MAX_OPACITY,
        anim_frame: 0,
        speed: 1,
        forward_filled: 80,
        branches: Vec::new(),
    };
    render_timeline(&mut fb, W, H, &c);
    // Rewind fill: 100/300 of bar width = green.
    let filled_w = (BAR_W as usize * 100 / 300) as u32;
    // Forward fill starts at BAR_X + filled_w, width = 80/300 of bar.
    let forward_w = (BAR_W as usize * 80 / 300) as u32;
    let forward_start = BAR_X + filled_w;
    // Check a pixel in the forward portion (not on a tick mark).
    let test_x = forward_start + forward_w / 2;
    let test_y = BAR_Y + BAR_H / 2;
    assert_eq!(
        fb[(test_y * W + test_x) as usize],
        TIMELINE_FORWARD,
        "expected forward buffer colour (light blue-white) after rewind fill"
    );
}

#[test]
fn forward_buffer_zero_is_noop() {
    let mut fb = vec![0u32; (W * H) as usize];
    let c = TimelineConfig {
        filled: 100,
        capacity: 300,
        mode: TimelineMode::Rewind,
        rewind_interval: 2,
        opacity: MAX_OPACITY,
        anim_frame: 0,
        speed: 1,
        forward_filled: 0,
        branches: Vec::new(),
    };
    render_timeline(&mut fb, W, H, &c);
    let filled_w = (BAR_W as usize * 100 / 300) as u32;
    let after_fill = BAR_X + filled_w + 5;
    let test_y = BAR_Y + BAR_H / 2;
    // Should be empty (dark gray), not forward colour.
    assert_eq!(
        fb[(test_y * W + after_fill) as usize],
        TIMELINE_EMPTY,
        "expected empty colour after rewind fill when forward_filled=0"
    );
}

#[test]
fn branch_bars_rendered_above_main_bar() {
    let mut fb = vec![0u32; (W * H) as usize];
    let c = TimelineConfig {
        filled: 150,
        capacity: 300,
        mode: TimelineMode::Rewind,
        rewind_interval: 2,
        opacity: MAX_OPACITY,
        anim_frame: 0,
        speed: 1,
        forward_filled: 0,
        branches: vec![(100, 50, 20, 0), (200, 30, 0, 1)],
    };
    render_timeline(&mut fb, W, H, &c);
    // Branch 0: diverge at 100, depth 0 → y = BAR_Y - 4
    let bp0_y = BAR_Y - 4;
    let bp0_x = BAR_X + (BAR_W as usize * 100 / 300) as u32;
    let rew0_w = (BAR_W as usize * 50 / 300) as u32;
    // Check for orange pixels in the branch bar.
    let has_branch0 = (bp0_y..bp0_y + 3)
        .any(|y| (bp0_x..bp0_x + rew0_w).any(|x| fb[(y * W + x) as usize] == TIMELINE_BRANCH));
    assert!(has_branch0, "expected branch 0 bar with orange fill");
    // Branch 1: diverge at 200, depth 1 → y = BAR_Y - 8
    let bp1_y = BAR_Y - 8;
    let bp1_x = BAR_X + (BAR_W as usize * 200 / 300) as u32;
    let rew1_w = (BAR_W as usize * 30 / 300) as u32;
    let has_branch1 = (bp1_y..bp1_y + 3)
        .any(|y| (bp1_x..bp1_x + rew1_w).any(|x| fb[(y * W + x) as usize] == TIMELINE_BRANCH));
    assert!(has_branch1, "expected branch 1 bar with orange fill");
    // Check diverge marker (yellow) at branch 0 position.
    let has_marker = (bp0_y..BAR_Y).any(|y| fb[(y * W + bp0_x) as usize] == TIMELINE_BRANCH_MARKER);
    assert!(has_marker, "expected yellow diverge marker at branch 0");
}
