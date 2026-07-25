//! Visual rewind timeline — graphical bar overlay showing rewind buffer fill and position.

use crate::osd::{glyph_bits, GLYPH_H, GLYPH_W, OSD_BG_ARGB, OSD_FG_ARGB};

/// Bar colours (ARGB, alpha always 0xFF).
pub const TIMELINE_FILLED: u32 = 0xFF00_CC66;
pub const TIMELINE_EMPTY: u32 = 0xFF33_3333;
pub const TIMELINE_BORDER: u32 = 0xFF88_8888;
pub const TIMELINE_MARKER: u32 = 0xFFFF_FFFF;
pub const TIMELINE_MARKER_DIM: u32 = 0xFF88_8888;
pub const TIMELINE_BG: u32 = 0xFF00_0000;
/// Forward buffer fill colour (light blue-white).
pub const TIMELINE_FORWARD: u32 = 0xFFAA_CCDD;
/// Branch bar colour (orange).
pub const TIMELINE_BRANCH: u32 = 0xFFCC_8833;
/// Branch bar forward fill colour (light orange).
pub const TIMELINE_BRANCH_FORWARD: u32 = 0xFFEE_BB88;
/// Branch diverge marker colour (bright yellow).
pub const TIMELINE_BRANCH_MARKER: u32 = 0xFFFF_DD00;

/// Bar layout constants (in NES framebuffer pixels).
pub const BAR_X: u32 = 4;
pub const BAR_Y: u32 = 228;
pub const BAR_W: u32 = 248;
pub const BAR_H: u32 = 6;
pub const BORDER_THICKNESS: u32 = 1;
pub const TEXT_Y: u32 = 218;

/// Maximum opacity value (fully visible). Fade ramps 0..=MAX_OPACITY.
pub const MAX_OPACITY: u8 = 4;

/// Timeline display mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimelineMode {
    /// Actively rewinding (Backspace held).
    Rewind,
    /// Actively forwarding (Shift+Backspace held).
    Forward,
    /// Buffer indicator (not actively rewinding/forwarding).
    Buffer,
}

/// Configuration for a single timeline render.
#[derive(Debug, Clone)]
pub struct TimelineConfig {
    /// Number of snapshots currently in the rewind buffer.
    pub filled: usize,
    /// Maximum number of snapshots the buffer can hold.
    pub capacity: usize,
    /// Display mode (controls label text and marker placement).
    pub mode: TimelineMode,
    /// Snapshots are taken every N frames; displayed counts are scaled by this.
    pub rewind_interval: usize,
    /// Opacity level (0 = invisible, `MAX_OPACITY` = fully visible).
    /// Used for fade in/out animation.
    pub opacity: u8,
    /// Animation frame counter (drives the marker pulse effect).
    pub anim_frame: u32,
    /// Rewind speed multiplier (1, 2, 4, 8). Shown in the text label.
    pub speed: u32,
    /// Number of snapshots in the forward buffer (shown as a lighter
    /// segment after the rewind fill).
    pub forward_filled: usize,
    /// Alternate timeline branches to render above the main bar.
    /// Each entry is (diverge_pos, rewind_len, forward_len, depth)
    /// where diverge_pos is the position on the parent timeline where
    /// the branch starts, and depth is the nesting level (0 = child of
    /// root, 1 = grandchild, etc.).
    pub branches: Vec<(usize, usize, usize, u32)>,
}

impl Default for TimelineConfig {
    fn default() -> Self {
        Self {
            filled: 0,
            capacity: 300,
            mode: TimelineMode::Buffer,
            rewind_interval: 2,
            opacity: MAX_OPACITY,
            anim_frame: 0,
            speed: 1,
            forward_filled: 0,
            branches: Vec::new(),
        }
    }
}

/// Blend two ARGB colours by a factor of `opacity / MAX_OPACITY`.
/// `opacity=0` → fully `bg`, `opacity=MAX_OPACITY` → fully `fg`.
fn blend(fg: u32, bg: u32, opacity: u8) -> u32 {
    if opacity >= MAX_OPACITY {
        return fg;
    }
    if opacity == 0 {
        return bg;
    }
    let a = opacity as u32;
    let b = (MAX_OPACITY - opacity) as u32;
    let fr = (fg >> 16) & 0xFF;
    let fg_ = (fg >> 8) & 0xFF;
    let fb = fg & 0xFF;
    let br = (bg >> 16) & 0xFF;
    let bg_ = (bg >> 8) & 0xFF;
    let bb = bg & 0xFF;
    let r = (fr * a + br * b) / MAX_OPACITY as u32;
    let g = (fg_ * a + bg_ * b) / MAX_OPACITY as u32;
    let bl = (fb * a + bb * b) / MAX_OPACITY as u32;
    0xFF00_0000 | (r << 16) | (g << 8) | bl
}

/// Draw the visual rewind timeline into the framebuffer.
///
/// See [`TimelineConfig`] for parameter details. When `opacity` is 0,
/// this is a no-op (no pixels are written).
pub fn render_timeline(fb: &mut [u32], width: u32, height: u32, cfg: &TimelineConfig) {
    if fb.len() != (width * height) as usize {
        return;
    }
    if cfg.capacity == 0 || cfg.opacity == 0 {
        return;
    }

    let op = cfg.opacity;

    // Compute blended colours for this opacity level.
    let bg_color = blend(TIMELINE_BG, 0, op);
    let empty_color = blend(TIMELINE_EMPTY, 0, op);
    let filled_color = blend(TIMELINE_FILLED, 0, op);
    let border_color = blend(TIMELINE_BORDER, 0, op);
    let fg_text = blend(OSD_FG_ARGB, 0, op);
    let bg_text = blend(OSD_BG_ARGB, 0, op);

    // Marker pulse: alternate between bright and dim every 8 anim frames.
    let marker_color = if cfg.anim_frame % 16 < 8 {
        blend(TIMELINE_MARKER, 0, op)
    } else {
        blend(TIMELINE_MARKER_DIM, 0, op)
    };

    // Draw a black background strip behind the bar + text area.
    let bg_top = TEXT_Y.saturating_sub(1);
    let bg_bottom = BAR_Y + BAR_H + BORDER_THICKNESS;
    fill_rect_blended(
        fb,
        width,
        height,
        0,
        bg_top,
        width,
        bg_bottom - bg_top,
        bg_color,
        op,
    );

    // Build the text label.
    let frames_filled = cfg.filled * cfg.rewind_interval;
    let frames_total = cfg.capacity * cfg.rewind_interval;
    let label = match cfg.mode {
        TimelineMode::Rewind => {
            if cfg.speed > 1 {
                format!("REWIND x{} F:{}/{}", cfg.speed, frames_filled, frames_total)
            } else {
                format!("REWIND  F:{}/{}", frames_filled, frames_total)
            }
        }
        TimelineMode::Forward => {
            if cfg.speed > 1 {
                format!(
                    "FORWARD x{} F:{}/{}",
                    cfg.speed, frames_filled, frames_total
                )
            } else {
                format!("FORWARD  F:{}/{}", frames_filled, frames_total)
            }
        }
        TimelineMode::Buffer => {
            format!("BUFFER  F:{}/{}", frames_filled, frames_total)
        }
    };
    draw_text_blended(
        fb, width, height, &label, BAR_X, TEXT_Y, fg_text, bg_text, op,
    );

    // Draw the bar border.
    let bar_outer_x = BAR_X.saturating_sub(BORDER_THICKNESS);
    let bar_outer_y = BAR_Y.saturating_sub(BORDER_THICKNESS);
    let bar_outer_w = BAR_W + 2 * BORDER_THICKNESS;
    let bar_outer_h = BAR_H + 2 * BORDER_THICKNESS;
    draw_rect_outline_blended(
        fb,
        width,
        height,
        bar_outer_x,
        bar_outer_y,
        bar_outer_w,
        bar_outer_h,
        border_color,
        op,
    );

    // Draw the empty (background) bar.
    fill_rect_blended(
        fb,
        width,
        height,
        BAR_X,
        BAR_Y,
        BAR_W,
        BAR_H,
        empty_color,
        op,
    );

    // Draw the filled portion (rewind buffer — green).
    let filled_w = if cfg.filled >= cfg.capacity {
        BAR_W
    } else {
        (BAR_W as usize * cfg.filled / cfg.capacity) as u32
    };
    if filled_w > 0 {
        fill_rect_blended(
            fb,
            width,
            height,
            BAR_X,
            BAR_Y,
            filled_w,
            BAR_H,
            filled_color,
            op,
        );
    }

    // Draw the forward buffer portion (light blue-white) starting
    // right after the rewind fill. Shows how many snapshots are
    // available for fast-forwarding. The forward buffer is an
    // independent count — its width is proportional to forward_filled,
    // not cumulative with the rewind fill.
    let forward_color = blend(TIMELINE_FORWARD, 0, op);
    let forward_w = if cfg.forward_filled == 0 || cfg.capacity == 0 {
        0
    } else {
        let fwd = (BAR_W as usize * cfg.forward_filled / cfg.capacity) as u32;
        // Clamp so forward segment doesn't extend past the bar.
        fwd.min(BAR_W - filled_w)
    };
    if forward_w > 0 && filled_w < BAR_W {
        fill_rect_blended(
            fb,
            width,
            height,
            BAR_X + filled_w,
            BAR_Y,
            forward_w,
            BAR_H,
            forward_color,
            op,
        );
    }

    // Draw tick marks every 25% of the bar for visual reference,
    for i in 1..4 {
        let tick_x = BAR_X + (BAR_W * i / 4);
        draw_vertical_line_blended(fb, width, height, tick_x, BAR_Y, BAR_H, border_color, op);
    }

    // Draw the position marker (after tick marks so it's on top).
    // In Rewind mode: marker at the right edge of the filled portion.
    // In Forward mode: marker at the right edge of the filled portion
    //   (moves right as snapshots are pushed back into the rewind buffer).
    // In Buffer mode: marker at the far right (most recent frame).
    let marker_x = match cfg.mode {
        TimelineMode::Rewind | TimelineMode::Forward => BAR_X + filled_w,
        TimelineMode::Buffer => BAR_X + BAR_W,
    };
    let marker_x = marker_x.min(BAR_X + BAR_W);
    draw_vertical_line_blended(fb, width, height, marker_x, BAR_Y, BAR_H, marker_color, op);

    // Draw alternate timeline branches as small bars above the main bar.
    // Each branch is a horizontal bar starting at its diverge_pos (mapped
    // to x on the parent timeline) and extending right by its rewind_len.
    // Forward buffer portion is shown in a lighter colour after the rewind
    // fill. Depth determines vertical offset (deeper = higher above bar).
    let branch_h: u32 = 3;
    let branch_gap: u32 = 1;
    for &(diverge_pos, rewind_len, forward_len, depth) in &cfg.branches {
        if cfg.capacity == 0 {
            continue;
        }
        // Map diverge_pos to x on the main bar.
        let div_x = if diverge_pos >= cfg.capacity {
            BAR_X + BAR_W
        } else {
            BAR_X + (BAR_W as usize * diverge_pos / cfg.capacity) as u32
        };
        // Map rewind_len to bar width (scaled by capacity).
        let rew_w = if cfg.capacity == 0 {
            0
        } else {
            (BAR_W as usize * rewind_len / cfg.capacity) as u32
        };
        let fwd_w = if cfg.capacity == 0 {
            0
        } else {
            (BAR_W as usize * forward_len / cfg.capacity) as u32
        };

        // Vertical position: each depth level is (branch_h + branch_gap)
        // pixels above the previous. Depth 0 is just above the main bar.
        let offset = (branch_h + branch_gap) * (depth + 1);
        let by = BAR_Y.saturating_sub(offset);

        // Draw diverge marker (vertical yellow line from main bar up).
        draw_vertical_line_blended(
            fb,
            width,
            height,
            div_x,
            by,
            branch_h + offset - branch_h,
            blend(TIMELINE_BRANCH_MARKER, 0, op),
            op,
        );

        // Draw rewind fill (orange).
        if rew_w > 0 {
            fill_rect_blended(
                fb,
                width,
                height,
                div_x,
                by,
                rew_w,
                branch_h,
                blend(TIMELINE_BRANCH, 0, op),
                op,
            );
        }
        // Draw forward fill (light orange) after rewind fill.
        if fwd_w > 0 {
            fill_rect_blended(
                fb,
                width,
                height,
                div_x + rew_w,
                by,
                fwd_w,
                branch_h,
                blend(TIMELINE_BRANCH_FORWARD, 0, op),
                op,
            );
        }
        // Draw border around branch bar.
        let total_w = rew_w + fwd_w;
        if total_w > 0 {
            draw_rect_outline_blended(
                fb,
                width,
                height,
                div_x,
                by,
                total_w,
                branch_h,
                blend(TIMELINE_BORDER, 0, op),
                op,
            );
        }
    }
}

/// Fill a solid rectangle of blended `color` at `(x, y)` with size `(w, h)`.
/// Reads existing framebuffer pixels for alpha blending. Clips to bounds.
#[allow(clippy::too_many_arguments)]
fn fill_rect_blended(
    fb: &mut [u32],
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    color: u32,
    opacity: u8,
) {
    if opacity >= MAX_OPACITY {
        let max_y = (y + h).min(height);
        let max_x = (x + w).min(width);
        for py in y..max_y {
            for px in x..max_x {
                fb[(py * width + px) as usize] = color;
            }
        }
        return;
    }
    let max_y = (y + h).min(height);
    let max_x = (x + w).min(width);
    for py in y..max_y {
        for px in x..max_x {
            let idx = (py * width + px) as usize;
            fb[idx] = blend(color, fb[idx], opacity);
        }
    }
}

/// Draw a 1-pixel rectangle outline with blended `color`.
#[allow(clippy::too_many_arguments)]
fn draw_rect_outline_blended(
    fb: &mut [u32],
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    color: u32,
    opacity: u8,
) {
    if w == 0 || h == 0 {
        return;
    }
    fill_rect_blended(fb, width, height, x, y, w, 1, color, opacity);
    fill_rect_blended(fb, width, height, x, y + h - 1, w, 1, color, opacity);
    fill_rect_blended(fb, width, height, x, y, 1, h, color, opacity);
    fill_rect_blended(fb, width, height, x + w - 1, y, 1, h, color, opacity);
}

/// Draw a blended vertical line at `x` from `y` down `h` pixels.
#[allow(clippy::too_many_arguments)]
fn draw_vertical_line_blended(
    fb: &mut [u32],
    width: u32,
    height: u32,
    x: u32,
    y: u32,
    h: u32,
    color: u32,
    opacity: u8,
) {
    if x >= width {
        return;
    }
    let max_y = (y + h).min(height);
    if opacity >= MAX_OPACITY {
        for py in y..max_y {
            fb[(py * width + x) as usize] = color;
        }
    } else {
        for py in y..max_y {
            let idx = (py * width + x) as usize;
            fb[idx] = blend(color, fb[idx], opacity);
        }
    }
}

/// Draw a line of blended text at `(x, y)` using the OSD 8x8 bitmap font.
#[allow(clippy::too_many_arguments)]
fn draw_text_blended(
    fb: &mut [u32],
    width: u32,
    height: u32,
    text: &str,
    x: u32,
    y: u32,
    fg: u32,
    bg: u32,
    opacity: u8,
) {
    let mut cx = x;
    for ch in text.chars() {
        if cx + GLYPH_W > width {
            break;
        }
        draw_glyph_blended(fb, width, height, ch, cx, y, fg, bg, opacity);
        cx += GLYPH_W;
    }
}

/// Draw one 8x8 glyph at `(x, y)` with blended background.
#[allow(clippy::too_many_arguments)]
fn draw_glyph_blended(
    fb: &mut [u32],
    width: u32,
    height: u32,
    ch: char,
    x: u32,
    y: u32,
    fg: u32,
    bg: u32,
    opacity: u8,
) {
    let glyph = glyph_bits(ch);
    for gy in 0..GLYPH_H {
        let py = y + gy;
        if py >= height {
            break;
        }
        let row = glyph[gy as usize];
        for gx in 0..GLYPH_W {
            let px = x + gx;
            if px >= width {
                break;
            }
            let bit = (row >> (7 - gx)) & 1;
            let color = if bit != 0 { fg } else { bg };
            let idx = (py * width + px) as usize;
            if opacity >= MAX_OPACITY {
                fb[idx] = color;
            } else {
                fb[idx] = blend(color, fb[idx], opacity);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn render_full_buffer() {
        let mut fb = vec![0u32; (W * H) as usize];
        render_timeline(&mut fb, W, H, &cfg(300, 300, TimelineMode::Buffer));
        let green_count = fb.iter().filter(|&&p| p == TIMELINE_FILLED).count();
        assert!(green_count > 0, "expected green pixels for full buffer");
        let marker_x = BAR_X + BAR_W;
        let has_marker =
            (BAR_Y..BAR_Y + BAR_H).any(|y| fb[(y * W + marker_x) as usize] == TIMELINE_MARKER);
        assert!(has_marker, "expected marker at right edge");
    }

    #[test]
    fn render_half_buffer_rewind() {
        let mut fb = vec![0u32; (W * H) as usize];
        render_timeline(&mut fb, W, H, &cfg(150, 300, TimelineMode::Rewind));
        let filled_w = (BAR_W as usize * 150 / 300) as u32;
        let mid_filled = BAR_X + filled_w / 2 + 1;
        assert_eq!(
            fb[((BAR_Y + BAR_H / 2) * W + mid_filled) as usize],
            TIMELINE_FILLED,
            "expected green in filled portion"
        );
        let empty_x = BAR_X + filled_w + 10;
        assert_eq!(
            fb[((BAR_Y + BAR_H / 2) * W + empty_x) as usize],
            TIMELINE_EMPTY,
            "expected dark gray in empty portion"
        );
        let marker_x = BAR_X + filled_w;
        let has_marker =
            (BAR_Y..BAR_Y + BAR_H).any(|y| fb[(y * W + marker_x) as usize] == TIMELINE_MARKER);
        assert!(has_marker, "expected marker at fill boundary");
    }

    #[test]
    fn render_empty_buffer() {
        let mut fb = vec![0u32; (W * H) as usize];
        render_timeline(&mut fb, W, H, &cfg(0, 300, TimelineMode::Rewind));
        let green_count = fb.iter().filter(|&&p| p == TIMELINE_FILLED).count();
        assert_eq!(green_count, 0, "expected no green for empty buffer");
        let has_marker =
            (BAR_Y..BAR_Y + BAR_H).any(|y| fb[(y * W + BAR_X) as usize] == TIMELINE_MARKER);
        assert!(has_marker, "expected marker at left edge for empty buffer");
    }

    #[test]
    fn render_zero_capacity_no_panic() {
        let mut fb = vec![0u32; (W * H) as usize];
        let c = TimelineConfig {
            capacity: 0,
            ..cfg(0, 0, TimelineMode::Rewind)
        };
        render_timeline(&mut fb, W, H, &c);
    }

    #[test]
    fn render_wrong_fb_size_no_panic() {
        let mut fb = vec![0u32; 100];
        render_timeline(&mut fb, W, H, &cfg(10, 300, TimelineMode::Rewind));
    }

    #[test]
    fn border_pixels_present() {
        let mut fb = vec![0u32; (W * H) as usize];
        render_timeline(&mut fb, W, H, &cfg(100, 300, TimelineMode::Buffer));
        let bx = BAR_X.saturating_sub(BORDER_THICKNESS);
        let by = BAR_Y.saturating_sub(BORDER_THICKNESS);
        assert_eq!(fb[(by * W + bx) as usize], TIMELINE_BORDER);
    }

    #[test]
    fn text_pixels_present() {
        let mut fb = vec![0u32; (W * H) as usize];
        render_timeline(&mut fb, W, H, &cfg(100, 300, TimelineMode::Buffer));
        let fg_count = fb.iter().filter(|&&p| p == OSD_FG_ARGB).count();
        assert!(fg_count > 0, "expected white text pixels");
    }

    #[test]
    fn tick_marks_present() {
        let mut fb = vec![0u32; (W * H) as usize];
        render_timeline(&mut fb, W, H, &cfg(300, 300, TimelineMode::Buffer));
        let tick_x = BAR_X + BAR_W / 4;
        let has_tick =
            (BAR_Y..BAR_Y + BAR_H).any(|y| fb[(y * W + tick_x) as usize] == TIMELINE_BORDER);
        assert!(has_tick, "expected 25% tick mark");
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
    fn opacity_partial_blends() {
        let mut fb = vec![0xFF0000FFu32; (W * H) as usize];
        let c = TimelineConfig {
            opacity: 2,
            anim_frame: 0,
            ..cfg(300, 300, TimelineMode::Buffer)
        };
        render_timeline(&mut fb, W, H, &c);
        let mid_x = BAR_X + BAR_W / 2;
        let mid_y = BAR_Y + BAR_H / 2;
        let pixel = fb[(mid_y * W + mid_x) as usize];
        assert_ne!(
            pixel, TIMELINE_FILLED,
            "should not be pure green at partial opacity"
        );
        assert_ne!(
            pixel, 0xFF0000FF,
            "should not be pure blue at partial opacity"
        );
    }

    #[test]
    fn pulse_alternates_marker_color() {
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
        assert_eq!(marker2, TIMELINE_MARKER_DIM, "frame 8 should be dim marker");
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
    fn speed_shown_in_label() {
        let mut fb = vec![0u32; (W * H) as usize];
        let c = TimelineConfig {
            speed: 4,
            ..cfg(100, 300, TimelineMode::Rewind)
        };
        render_timeline(&mut fb, W, H, &c);
        let fg_count = fb.iter().filter(|&&p| p == OSD_FG_ARGB).count();
        assert!(fg_count > 0, "expected text pixels with speed label");
    }

    #[test]
    fn forward_label_present() {
        let mut fb = vec![0u32; (W * H) as usize];
        render_timeline(&mut fb, W, H, &cfg(100, 300, TimelineMode::Forward));
        let f_x = BAR_X + 1;
        let f_y = TEXT_Y;
        assert_eq!(
            fb[(f_y * W + f_x) as usize],
            OSD_FG_ARGB,
            "expected 'F' glyph pixel for FORWARD label"
        );
    }

    #[test]
    fn blend_full_opacity_returns_fg() {
        assert_eq!(blend(0xFF00FF00, 0xFF0000FF, MAX_OPACITY), 0xFF00FF00);
    }

    #[test]
    fn blend_zero_opacity_returns_bg() {
        assert_eq!(blend(0xFF00FF00, 0xFF0000FF, 0), 0xFF0000FF);
    }

    #[test]
    fn blend_half_opacity_averages() {
        let result = blend(0xFFFFFFFF, 0xFF000000, 2);
        let r = (result >> 16) & 0xFF;
        assert_eq!(r, 0x7F, "half opacity should average to 0x7F");
    }
}
