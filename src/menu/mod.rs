//! In-emulator OSD menu system — navigable menu rendered into the
//! framebuffer using the existing 8x8 OSD font.
//!
//! Opened with **F12**. Navigation:
//!
//! | Key            | Action                                    |
//! |----------------|-------------------------------------------|
//! | `Up`/`Down`    | Move selection                            |
//! | `Return`       | Activate item / enter sub-menu             |
//! | `Backspace`    | Go back to parent page                    |
//! | `Escape`       | Close menu                                |
//!
//! # Pages
//!
//! - **Main**: lists sub-menus (Debug, Controller 1, Controller 2, Close)
//! - **Debug**: toggle `slant_corruption` and other debug flags at runtime
//! - **Controller N**: view current key bindings; press Enter on a button
//!   to rebind it — the next key press becomes the new binding
//!
//! The menu renders a semi-transparent dark background and highlights the
//! current selection with an inverted colour scheme. When the menu is open,
//! all key events are consumed by the menu (the emulator is effectively
//! paused from an input perspective, though emulation continues).

use sdl2::keyboard::Keycode;

use crate::config::{Config, NES_BUTTON_NAMES};
use crate::osd::{glyph_bits, GLYPH_H, GLYPH_W, OSD_BG_ARGB, OSD_FG_ARGB};

/// Menu highlight colour (bright yellow) for the selected row.
const MENU_HL_ARGB: u32 = 0xFFFF_FF00;
/// Menu highlight background (dark blue) for the selected row.
const MENU_HL_BG_ARGB: u32 = 0xFF00_0010;
/// Semi-transparent menu border colour.
const MENU_BORDER_ARGB: u32 = 0xFF40_40C0;
/// Menu title colour (bright cyan).
const MENU_TITLE_ARGB: u32 = 0xFF00_FFFF;
/// Left padding in pixels for menu text.
const MENU_PAD_X: u32 = 8;
/// Top padding in pixels for menu text.
const MENU_PAD_Y: u32 = 8;

/// Which menu page is currently active.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuPage {
    /// Root menu — lists sub-menus.
    Main,
    /// Debug options — toggle bug injection flags.
    Debug,
    /// Rewind settings — adjust buffer capacity.
    Rewind,
    /// Turbo speed settings — select Spacebar speed multiplier.
    Turbo,
    /// Controller 1 key bindings — view + rebind.
    Controller1,
    /// Controller 2 key bindings — view + rebind.
    Controller2,
}

/// State for the in-emulator menu system.
pub struct Menu {
    /// Whether the menu is currently open.
    open: bool,
    /// Current page.
    page: MenuPage,
    /// Current selection index within the page.
    selection: usize,
    /// When in rebind mode, this is the button index being rebound.
    /// `None` means not in rebind mode.
    rebind_button: Option<u8>,
    /// Which controller is being rebound (0 or 1).
    rebind_controller: usize,
    /// Live copy of debug flags (mirrors PPU state).
    slant_corruption: bool,
    /// Live copy of use_inaccurate_palette flag (mirrors PPU state).
    inaccurate_palette: bool,
    /// Live copy of nmi_retrigger flag (mirrors PPU state).
    nmi_retrigger: bool,
    /// Live copy of ring_buffer_trace flag (mirrors config).
    ring_buffer_trace: bool,
    /// Pending ring_buffer_trace change (applied on menu close).
    pending_ring_buffer_trace: Option<bool>,
    /// Pending rewind capacity (applied on menu close).
    pending_rewind_capacity: Option<usize>,
    /// Live display value of rewind capacity (set when menu opens).
    rewind_capacity_display: usize,
    /// Live display value of rewind branching (set when menu opens).
    branching_display: bool,
    /// Pending rewind branching change (applied on menu close).
    pending_branching: Option<bool>,
    /// Live display value of timeline enabled (set when menu opens).
    timeline_display: bool,
    /// Pending timeline enabled change (applied on menu close).
    pending_timeline: Option<bool>,
    /// Live display value of countdown delay in ms (set when menu opens).
    countdown_delay_display: u32,
    /// Pending countdown delay change in ms (applied on menu close).
    pending_countdown_delay: Option<u32>,
    /// Live display value of countdown start number (set when menu opens).
    countdown_start_display: u32,
    /// Pending countdown start number change (applied on menu close).
    pending_countdown_start: Option<u32>,
    /// Pending turbo speed index (applied on menu close).
    pending_turbo_index: Option<usize>,
    /// Live display value of turbo speed index (set when menu opens).
    turbo_index_display: usize,
    /// Pending key bindings being edited (applied on menu close).
    /// Stored as (controller, button, Keycode).
    pending_bindings: Vec<(usize, u8, Keycode)>,
}

impl Default for Menu {
    fn default() -> Self {
        Self {
            open: false,
            page: MenuPage::Main,
            selection: 0,
            rebind_button: None,
            rebind_controller: 0,
            slant_corruption: false,
            inaccurate_palette: false,
            nmi_retrigger: false,
            ring_buffer_trace: false,
            pending_ring_buffer_trace: None,
            pending_rewind_capacity: None,
            rewind_capacity_display: crate::save_state::DEFAULT_REWIND_CAPACITY,
            branching_display: true,
            pending_branching: None,
            timeline_display: true,
            pending_timeline: None,
            countdown_delay_display: 500,
            pending_countdown_delay: None,
            countdown_start_display: 3,
            pending_countdown_start: None,
            pending_turbo_index: None,
            turbo_index_display: crate::ui_hotkeys::DEFAULT_TURBO_SPEED_INDEX,
            pending_bindings: Vec::new(),
        }
    }
}

impl Menu {
    /// Create a new menu, initialized from the current config.
    pub fn new() -> Self {
        Self::default()
    }

    /// Is the menu currently open?
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Open the menu.
    pub fn open(&mut self) {
        self.open = true;
        self.page = MenuPage::Main;
        self.selection = 0;
        self.rebind_button = None;
    }

    /// Close the menu and discard pending rebind state.
    pub fn close(&mut self) {
        self.open = false;
        self.rebind_button = None;
    }

    /// Current debug slant_corruption flag.
    pub fn slant_corruption(&self) -> bool {
        self.slant_corruption
    }

    /// Set the slant_corruption flag (called when the PPU state changes
    /// externally, e.g. from config at startup).
    pub fn set_slant_corruption(&mut self, v: bool) {
        self.slant_corruption = v;
    }

    /// Current debug inaccurate_palette flag.
    pub fn inaccurate_palette(&self) -> bool {
        self.inaccurate_palette
    }

    /// Set the inaccurate_palette flag (called when the PPU state changes
    /// externally, e.g. from config at startup).
    pub fn set_inaccurate_palette(&mut self, v: bool) {
        self.inaccurate_palette = v;
    }

    /// Current debug nmi_retrigger flag.
    pub fn nmi_retrigger(&self) -> bool {
        self.nmi_retrigger
    }

    /// Set the nmi_retrigger flag (called when the PPU state changes
    /// externally, e.g. from config at startup).
    pub fn set_nmi_retrigger(&mut self, v: bool) {
        self.nmi_retrigger = v;
    }

    /// Set the ring_buffer_trace flag (called from main.rs when the menu
    /// opens, so the menu shows the live config value).
    pub fn set_ring_buffer_trace_display(&mut self, v: bool) {
        self.ring_buffer_trace = v;
    }

    /// Pending ring_buffer_trace change accumulated while the menu was
    /// open. Returns `Some(v)` if the user changed it, `None` otherwise.
    pub fn take_pending_ring_buffer_trace(&mut self) -> Option<bool> {
        self.pending_ring_buffer_trace.take()
    }

    /// Pending key binding changes accumulated while the menu was open.
    /// Returns `(controller, button, Keycode)` tuples. The caller
    /// should apply these to the `Config` and `InputMapper`.
    pub fn take_pending_bindings(&mut self) -> Vec<(usize, u8, Keycode)> {
        std::mem::take(&mut self.pending_bindings)
    }

    /// Pending rewind capacity change accumulated while the menu was
    /// open. Returns `Some(capacity)` if the user changed it, `None`
    /// otherwise. The caller should apply it to `SaveStateHotkeys` and
    /// `Config`.
    pub fn take_pending_rewind_capacity(&mut self) -> Option<usize> {
        self.pending_rewind_capacity.take()
    }

    /// Set the current rewind capacity display value (called from
    /// `main.rs` when the menu opens, so the menu shows the live value).
    pub fn set_rewind_capacity_display(&mut self, cap: usize) {
        self.rewind_capacity_display = cap;
    }

    /// Set the current rewind branching display value (called from
    /// `main.rs` when the menu opens, so the menu shows the live value).
    pub fn set_branching_display(&mut self, enabled: bool) {
        self.branching_display = enabled;
    }

    /// Set the current timeline enabled display value (called from
    /// `main.rs` when the menu opens, so the menu shows the live value).
    pub fn set_timeline_display(&mut self, enabled: bool) {
        self.timeline_display = enabled;
    }

    /// Set the current countdown delay display value in ms (called from
    /// `main.rs` when the menu opens, so the menu shows the live value).
    pub fn set_countdown_delay_display(&mut self, ms: u32) {
        self.countdown_delay_display = ms;
    }

    /// Set the current countdown start number display value (called from
    /// `main.rs` when the menu opens, so the menu shows the live value).
    pub fn set_countdown_start_display(&mut self, n: u32) {
        self.countdown_start_display = n;
    }

    /// Pending timeline enabled change accumulated while the menu was
    /// open. Returns `Some(enabled)` if the user changed it, `None`
    /// otherwise.
    pub fn take_pending_timeline(&mut self) -> Option<bool> {
        self.pending_timeline.take()
    }

    /// Pending countdown delay change accumulated while the menu was
    /// open. Returns `Some(ms)` if the user changed it, `None` otherwise.
    pub fn take_pending_countdown_delay(&mut self) -> Option<u32> {
        self.pending_countdown_delay.take()
    }

    /// Pending countdown start number change accumulated while the menu
    /// was open. Returns `Some(n)` if the user changed it, `None` otherwise.
    pub fn take_pending_countdown_start(&mut self) -> Option<u32> {
        self.pending_countdown_start.take()
    }

    /// Pending rewind branching change accumulated while the menu was
    /// open. Returns `Some(enabled)` if the user changed it, `None`
    /// otherwise.
    pub fn take_pending_branching(&mut self) -> Option<bool> {
        self.pending_branching.take()
    }

    /// Pending turbo speed index change accumulated while the menu was
    /// open. Returns `Some(index)` if the user changed it, `None`
    /// otherwise. The caller should apply it to `UiHotkeys` and `Config`.
    pub fn take_pending_turbo_index(&mut self) -> Option<usize> {
        self.pending_turbo_index.take()
    }

    /// Set the current turbo speed index display value (called from
    /// `main.rs` when the menu opens, so the menu shows the live value).
    pub fn set_turbo_index_display(&mut self, idx: usize) {
        self.turbo_index_display = idx;
    }

    /// Number of items on the current page.
    fn page_item_count(&self) -> usize {
        match self.page {
            MenuPage::Main => 6,   // Debug, Rewind, Turbo, Ctrl1, Ctrl2, Close
            MenuPage::Debug => 5, // slant_corruption, inaccurate_palette, nmi_retrigger, ring_buffer_trace, Back
            MenuPage::Rewind => 6, // capacity, branching, timeline, countdown delay, countdown start, Back
            MenuPage::Turbo => 2, // speed, Back
            MenuPage::Controller1 | MenuPage::Controller2 => 9, // 8 buttons + Back
        }
    }

    /// Handle a key-down event. Returns `true` if the key was consumed by
    /// the menu (always true when the menu is open). The caller should
    /// check `is_open()` before routing keys to joypad/debug dispatchers.
    ///
    /// When in rebind mode, the next key press (except Escape) is captured
    /// as the new binding for the selected button.
    pub fn handle_key(&mut self, key: Keycode) -> bool {
        if !self.open {
            return false;
        }

        // If in rebind mode, capture the next key.
        if let Some(btn) = self.rebind_button {
            match key {
                Keycode::Escape => {
                    self.rebind_button = None;
                }
                _ => {
                    self.pending_bindings
                        .push((self.rebind_controller, btn, key));
                    self.rebind_button = None;
                }
            }
            return true;
        }

        match key {
            Keycode::Escape => {
                if self.page == MenuPage::Main {
                    self.close();
                } else {
                    self.page = MenuPage::Main;
                    self.selection = 0;
                }
            }
            Keycode::Up => {
                if self.selection > 0 {
                    self.selection -= 1;
                } else {
                    self.selection = self.page_item_count().saturating_sub(1);
                }
            }
            Keycode::Down => {
                let max = self.page_item_count().saturating_sub(1);
                if self.selection < max {
                    self.selection += 1;
                } else {
                    self.selection = 0;
                }
            }
            Keycode::Return => {
                self.activate();
            }
            Keycode::Backspace => {
                if self.page != MenuPage::Main {
                    self.page = MenuPage::Main;
                    self.selection = 0;
                }
            }
            Keycode::Left if self.page == MenuPage::Rewind && self.selection == 0 => {
                let cap = self.effective_rewind_capacity();
                let new_cap = if cap <= 10 {
                    1
                } else if cap <= 100 {
                    cap - 10
                } else if cap <= 1000 {
                    cap - 100
                } else {
                    cap - 1000
                };
                self.pending_rewind_capacity = Some(new_cap);
            }
            Keycode::Right if self.page == MenuPage::Rewind && self.selection == 0 => {
                let cap = self.effective_rewind_capacity();
                let max = crate::save_state::MAX_REWIND_CAPACITY;
                let new_cap = if cap >= 1000 {
                    (cap + 1000).min(max)
                } else if cap >= 100 {
                    (cap + 100).min(max)
                } else if cap >= 10 {
                    (cap + 10).min(max)
                } else {
                    (cap + 1).min(max)
                };
                self.pending_rewind_capacity = Some(new_cap);
            }
            Keycode::Left if self.page == MenuPage::Rewind && self.selection == 1 => {
                self.pending_branching = Some(!self.effective_branching());
            }
            Keycode::Right if self.page == MenuPage::Rewind && self.selection == 1 => {
                self.pending_branching = Some(!self.effective_branching());
            }
            Keycode::Left if self.page == MenuPage::Rewind && self.selection == 2 => {
                self.pending_timeline = Some(!self.effective_timeline());
            }
            Keycode::Right if self.page == MenuPage::Rewind && self.selection == 2 => {
                self.pending_timeline = Some(!self.effective_timeline());
            }
            Keycode::Left if self.page == MenuPage::Rewind && self.selection == 3 => {
                let cd = self.effective_countdown_delay();
                let new_cd = if cd == 0 {
                    crate::save_state_hotkeys::MAX_COUNTDOWN_DELAY_MS
                } else {
                    cd.saturating_sub(100)
                };
                self.pending_countdown_delay = Some(new_cd);
            }
            Keycode::Right if self.page == MenuPage::Rewind && self.selection == 3 => {
                let cd = self.effective_countdown_delay();
                let max = crate::save_state_hotkeys::MAX_COUNTDOWN_DELAY_MS;
                let new_cd = if cd >= max {
                    0
                } else {
                    (cd + 100).min(max)
                };
                self.pending_countdown_delay = Some(new_cd);
            }
            Keycode::Left if self.page == MenuPage::Rewind && self.selection == 4 => {
                let n = self.effective_countdown_start();
                let new_n = if n <= 1 {
                    crate::save_state_hotkeys::MAX_COUNTDOWN_START_NUMBER
                } else {
                    n - 1
                };
                self.pending_countdown_start = Some(new_n);
            }
            Keycode::Right if self.page == MenuPage::Rewind && self.selection == 4 => {
                let n = self.effective_countdown_start();
                let max = crate::save_state_hotkeys::MAX_COUNTDOWN_START_NUMBER;
                let new_n = if n >= max {
                    1
                } else {
                    n + 1
                };
                self.pending_countdown_start = Some(new_n);
            }
            Keycode::Left if self.page == MenuPage::Turbo && self.selection == 0 => {
                let idx = self.effective_turbo_index();
                let new_idx = if idx == 0 {
                    crate::ui_hotkeys::TURBO_SPEEDS.len() - 1
                } else {
                    idx - 1
                };
                self.pending_turbo_index = Some(new_idx);
            }
            Keycode::Right if self.page == MenuPage::Turbo && self.selection == 0 => {
                let idx = self.effective_turbo_index();
                let new_idx = if idx + 1 >= crate::ui_hotkeys::TURBO_SPEEDS.len() {
                    0
                } else {
                    idx + 1
                };
                self.pending_turbo_index = Some(new_idx);
            }
            _ => {}
        }
        true
    }

    /// Activate the currently selected item.
    fn activate(&mut self) {
        match self.page {
            MenuPage::Main => match self.selection {
                0 => {
                    self.page = MenuPage::Debug;
                    self.selection = 0;
                }
                1 => {
                    self.page = MenuPage::Rewind;
                    self.selection = 0;
                }
                2 => {
                    self.page = MenuPage::Turbo;
                    self.selection = 0;
                }
                3 => {
                    self.page = MenuPage::Controller1;
                    self.selection = 0;
                }
                4 => {
                    self.page = MenuPage::Controller2;
                    self.selection = 0;
                }
                5 => self.close(),
                _ => {}
            },
            MenuPage::Debug => match self.selection {
                0 => {
                    self.slant_corruption = !self.slant_corruption;
                }
                1 => {
                    self.inaccurate_palette = !self.inaccurate_palette;
                }
                2 => {
                    self.nmi_retrigger = !self.nmi_retrigger;
                }
                3 => {
                    let new_val = !self.effective_ring_buffer_trace();
                    self.pending_ring_buffer_trace = Some(new_val);
                }
                4 => {
                    self.page = MenuPage::Main;
                    self.selection = 0;
                }
                _ => {}
            },
            MenuPage::Rewind => match self.selection {
                0 => {
                    // Left/Right adjusts the value; Enter is a no-op here.
                }
                1 => {
                    // Left/Right toggles; Enter also toggles.
                    self.pending_branching = Some(!self.effective_branching());
                }
                2 => {
                    self.pending_timeline = Some(!self.effective_timeline());
                }
                3 => {
                    // Left/Right adjusts the countdown delay; Enter is a no-op.
                }
                4 => {
                    // Left/Right adjusts the countdown start number; Enter is a no-op.
                }
                5 => {
                    self.page = MenuPage::Main;
                    self.selection = 0;
                }
                _ => {}
            },
            MenuPage::Turbo => match self.selection {
                0 => {
                    // Left/Right adjusts the value; Enter is a no-op here.
                }
                1 => {
                    self.page = MenuPage::Main;
                    self.selection = 0;
                }
                _ => {}
            },
            MenuPage::Controller1 => {
                if self.selection < 8 {
                    self.rebind_controller = 0;
                    self.rebind_button = Some(self.selection as u8);
                } else {
                    self.page = MenuPage::Main;
                    self.selection = 0;
                }
            }
            MenuPage::Controller2 => {
                if self.selection < 8 {
                    self.rebind_controller = 1;
                    self.rebind_button = Some(self.selection as u8);
                } else {
                    self.page = MenuPage::Main;
                    self.selection = 0;
                }
            }
        }
    }

    /// Return the effective rewind capacity: the pending value if the
    /// user has adjusted it, otherwise the display value set when the
    /// menu opened.
    fn effective_rewind_capacity(&self) -> usize {
        self.pending_rewind_capacity
            .unwrap_or(self.rewind_capacity_display)
    }

    /// Return the effective turbo speed index: the pending value if the
    /// user has adjusted it, otherwise the display value set when the
    /// menu opened.
    fn effective_turbo_index(&self) -> usize {
        self.pending_turbo_index.unwrap_or(self.turbo_index_display)
    }

    /// Return the effective ring_buffer_trace: the pending value if
    /// the user has toggled it, otherwise the display value.
    fn effective_ring_buffer_trace(&self) -> bool {
        self.pending_ring_buffer_trace
            .unwrap_or(self.ring_buffer_trace)
    }

    /// Return the effective rewind branching: the pending value if
    /// the user has toggled it, otherwise the display value.
    fn effective_branching(&self) -> bool {
        self.pending_branching.unwrap_or(self.branching_display)
    }

    /// Return the effective timeline enabled: the pending value if
    /// the user has toggled it, otherwise the display value.
    fn effective_timeline(&self) -> bool {
        self.pending_timeline.unwrap_or(self.timeline_display)
    }

    /// Return the effective countdown delay in ms: the pending value if
    /// the user has adjusted it, otherwise the display value.
    fn effective_countdown_delay(&self) -> u32 {
        self.pending_countdown_delay
            .unwrap_or(self.countdown_delay_display)
    }

    /// Return the effective countdown start number: the pending value if
    /// the user has adjusted it, otherwise the display value.
    fn effective_countdown_start(&self) -> u32 {
        self.pending_countdown_start
            .unwrap_or(self.countdown_start_display)
    }

    /// Build the lines to display for the current page state.
    fn build_lines(&self, config: &Config) -> Vec<String> {
        let mut lines = Vec::new();

        match self.page {
            MenuPage::Main => {
                lines.push("== NES-EMU MENU ==".to_string());
                lines.push(self.fmt_item(0, "Debug Options"));
                lines.push(self.fmt_item(1, "Rewind Settings"));
                lines.push(self.fmt_item(2, "Turbo Speed"));
                lines.push(self.fmt_item(3, "Controller 1 Keys"));
                lines.push(self.fmt_item(4, "Controller 2 Keys"));
                lines.push(self.fmt_item(5, "Close Menu"));
            }
            MenuPage::Debug => {
                lines.push("== DEBUG OPTIONS ==".to_string());
                let slant = if self.slant_corruption { "ON" } else { "OFF" };
                lines.push(self.fmt_item(0, &format!("Slant Corruption: {slant}")));
                let pal = if self.inaccurate_palette { "ON" } else { "OFF" };
                lines.push(self.fmt_item(1, &format!("Inaccurate Palette: {pal}")));
                let nmi = if self.nmi_retrigger { "ON" } else { "OFF" };
                lines.push(self.fmt_item(2, &format!("NMI Retrigger: {nmi}")));
                let rbt = if self.effective_ring_buffer_trace() {
                    "ON"
                } else {
                    "OFF"
                };
                lines.push(self.fmt_item(3, &format!("Ring Buffer Trace: {rbt}")));
                lines.push(self.fmt_item(4, "Back"));
            }
            MenuPage::Rewind => {
                lines.push("== REWIND SETTINGS ==".to_string());
                let cap = self.effective_rewind_capacity();
                let secs = cap * crate::save_state_hotkeys::REWIND_INTERVAL as usize / 60;
                lines.push(self.fmt_item(0, &format!("Capacity: {cap} (~{secs}s) < >")));
                let br = if self.effective_branching() {
                    "ON"
                } else {
                    "OFF"
                };
                lines.push(self.fmt_item(1, &format!("Branching: {br} < >")));
                let tl = if self.effective_timeline() {
                    "ON"
                } else {
                    "OFF"
                };
                lines.push(self.fmt_item(2, &format!("Timeline: {tl} < >")));
                let cd = self.effective_countdown_delay();
                let cd_str = if cd == 0 {
                    "OFF".to_string()
                } else {
                    format!("{}ms", cd)
                };
                lines.push(self.fmt_item(3, &format!("Countdown Delay: {cd_str} < >")));
                let cs = self.effective_countdown_start();
                lines.push(self.fmt_item(4, &format!("Countdown Start: {cs} < >")));
                lines.push(self.fmt_item(5, "Back"));
            }
            MenuPage::Turbo => {
                lines.push("== TURBO SPEED ==".to_string());
                let idx = self.effective_turbo_index();
                let speed = crate::ui_hotkeys::TURBO_SPEEDS[idx];
                lines.push(self.fmt_item(0, &format!("Speed: {:.2}x < >", speed)));
                lines.push(self.fmt_item(1, "Back"));
            }
            MenuPage::Controller1 | MenuPage::Controller2 => {
                let ctrl = if self.page == MenuPage::Controller1 {
                    0
                } else {
                    1
                };
                let title = if ctrl == 0 {
                    "== CONTROLLER 1 =="
                } else {
                    "== CONTROLLER 2 =="
                };
                lines.push(title.to_string());

                for btn in 0..8u8 {
                    let btn_name = NES_BUTTON_NAMES[btn as usize];
                    let key_name = config
                        .keycode_for(ctrl, btn)
                        .map(|kc| keycode_to_name(kc).to_string())
                        .unwrap_or_else(|| "---".to_string());

                    // Check for pending rebind override.
                    let display_name = self
                        .pending_bindings
                        .iter()
                        .rev()
                        .find(|(c, b, _)| *c == ctrl && *b == btn)
                        .map(|(_, _, kc)| keycode_to_name(*kc).to_string())
                        .unwrap_or(key_name);

                    let label = if self.rebind_button == Some(btn) && self.rebind_controller == ctrl
                    {
                        format!("{btn_name}: PRESS KEY...")
                    } else {
                        format!("{btn_name}: {display_name}")
                    };
                    lines.push(self.fmt_item(btn as usize, &label));
                }
                lines.push(self.fmt_item(8, "Back"));
            }
        }

        lines
    }

    /// Format a menu item with a selection indicator.
    fn fmt_item(&self, idx: usize, text: &str) -> String {
        if idx == self.selection {
            format!("> {text}")
        } else {
            format!("  {text}")
        }
    }

    /// Render the menu into the framebuffer. No-op when the menu is closed.
    /// Draws a bordered background, then the menu lines with the OSD font.
    pub fn render(&self, fb: &mut [u32], width: u32, height: u32, config: &Config) {
        if !self.open {
            return;
        }

        let lines = self.build_lines(config);
        let line_count = lines.len();
        let menu_h = (line_count as u32 * GLYPH_H) + 2 * MENU_PAD_Y;

        // Draw semi-transparent background.
        for y in 0..height.min(menu_h) {
            for x in 0..width {
                let idx = (y * width + x) as usize;
                if idx < fb.len() {
                    // Blend with dark background (50% alpha approximation).
                    fb[idx] = blend_dark(fb[idx]);
                }
            }
        }

        // Draw top border line.
        for x in 0..width {
            let idx = x as usize;
            if idx < fb.len() {
                fb[idx] = MENU_BORDER_ARGB;
            }
        }

        // Render lines.
        for (i, line) in lines.iter().enumerate() {
            let y = MENU_PAD_Y + i as u32 * GLYPH_H;
            if y + GLYPH_H > height {
                break;
            }

            let is_title = i == 0;
            let is_selected = match self.page {
                MenuPage::Main => i >= 1 && i <= 6 && (i - 1) == self.selection,
                MenuPage::Debug => i >= 1 && i <= 4 && (i - 1) == self.selection,
                MenuPage::Rewind => i >= 1 && i <= 3 && (i - 1) == self.selection,
                MenuPage::Turbo => i >= 1 && i <= 2 && (i - 1) == self.selection,
                MenuPage::Controller1 | MenuPage::Controller2 => {
                    i >= 1 && i <= 9 && (i - 1) == self.selection
                }
            };

            let fg = if is_title {
                MENU_TITLE_ARGB
            } else if is_selected {
                MENU_HL_ARGB
            } else {
                OSD_FG_ARGB
            };
            let bg = if is_selected {
                MENU_HL_BG_ARGB
            } else {
                OSD_BG_ARGB
            };

            draw_text_line(fb, width, height, line, MENU_PAD_X, y, fg, bg);
        }
    }
}

/// Draw a single line of text at pixel (x, y) with the given fg/bg colours.
fn draw_text_line(
    fb: &mut [u32],
    width: u32,
    height: u32,
    text: &str,
    x: u32,
    y: u32,
    fg: u32,
    bg: u32,
) {
    if y >= height {
        return;
    }
    let mut cx = x;
    for ch in text.chars() {
        if cx + GLYPH_W > width {
            break;
        }
        draw_glyph(fb, width, height, ch, cx, y, fg, bg);
        cx = cx.saturating_add(GLYPH_W);
    }
}

/// Draw one glyph at pixel (x, y) with the given fg/bg colours.
fn draw_glyph(fb: &mut [u32], width: u32, height: u32, ch: char, x: u32, y: u32, fg: u32, bg: u32) {
    let glyph = glyph_bits(ch);
    for gy in 0..GLYPH_H {
        let row = glyph[gy as usize];
        let py = y + gy;
        if py >= height {
            break;
        }
        for gx in 0..GLYPH_W {
            let px = x + gx;
            if px >= width {
                break;
            }
            let bit = (row >> (7 - gx)) & 1;
            let color = if bit != 0 { fg } else { bg };
            let idx = (py * width + px) as usize;
            fb[idx] = color;
        }
    }
}

/// Blend a pixel with a dark overlay (approximate 60% darkening).
fn blend_dark(orig: u32) -> u32 {
    let r = (orig >> 16) & 0xFF;
    let g = (orig >> 8) & 0xFF;
    let b = orig & 0xFF;
    let nr = r * 40 / 100;
    let ng = g * 40 / 100;
    let nb = b * 40 / 100;
    0xFF00_0000 | (nr << 16) | (ng << 8) | nb
}

/// Convert an SDL2 `Keycode` to a human-readable name string for display.
pub fn keycode_to_name(key: Keycode) -> &'static str {
    match key {
        Keycode::A => "A",
        Keycode::B => "B",
        Keycode::C => "C",
        Keycode::D => "D",
        Keycode::E => "E",
        Keycode::F => "F",
        Keycode::G => "G",
        Keycode::H => "H",
        Keycode::I => "I",
        Keycode::J => "J",
        Keycode::K => "K",
        Keycode::L => "L",
        Keycode::M => "M",
        Keycode::N => "N",
        Keycode::O => "O",
        Keycode::P => "P",
        Keycode::Q => "Q",
        Keycode::R => "R",
        Keycode::S => "S",
        Keycode::T => "T",
        Keycode::U => "U",
        Keycode::V => "V",
        Keycode::W => "W",
        Keycode::X => "X",
        Keycode::Y => "Y",
        Keycode::Z => "Z",
        Keycode::Num0 => "0",
        Keycode::Num1 => "1",
        Keycode::Num2 => "2",
        Keycode::Num3 => "3",
        Keycode::Num4 => "4",
        Keycode::Num5 => "5",
        Keycode::Num6 => "6",
        Keycode::Num7 => "7",
        Keycode::Num8 => "8",
        Keycode::Num9 => "9",
        Keycode::Return => "Return",
        Keycode::Escape => "Escape",
        Keycode::Backspace => "Backspace",
        Keycode::Tab => "Tab",
        Keycode::Space => "Space",
        Keycode::Up => "Up",
        Keycode::Down => "Down",
        Keycode::Left => "Left",
        Keycode::Right => "Right",
        Keycode::LeftBracket => "[",
        Keycode::RightBracket => "]",
        Keycode::Semicolon => ";",
        Keycode::Quote => "'",
        Keycode::Comma => ",",
        Keycode::Period => ".",
        Keycode::Slash => "/",
        Keycode::Minus => "-",
        Keycode::Equals => "=",
        Keycode::Backslash => "\\",
        Keycode::Backquote => "`",
        Keycode::CapsLock => "CapsLk",
        Keycode::LCTRL => "LCtrl",
        Keycode::RCTRL => "RCtrl",
        Keycode::LSHIFT => "LShift",
        Keycode::RSHIFT => "RShift",
        Keycode::LALT => "LAlt",
        Keycode::RALT => "RAlt",
        Keycode::F1 => "F1",
        Keycode::F2 => "F2",
        Keycode::F3 => "F3",
        Keycode::F4 => "F4",
        Keycode::F5 => "F5",
        Keycode::F6 => "F6",
        Keycode::F7 => "F7",
        Keycode::F8 => "F8",
        Keycode::F9 => "F9",
        Keycode::F10 => "F10",
        Keycode::F11 => "F11",
        Keycode::F12 => "F12",
        Keycode::Home => "Home",
        Keycode::End => "End",
        Keycode::PageUp => "PgUp",
        Keycode::PageDown => "PgDn",
        Keycode::Insert => "Ins",
        Keycode::Delete => "Del",
        _ => "???",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_starts_closed() {
        let m = Menu::new();
        assert!(!m.is_open());
    }

    #[test]
    fn open_sets_main_page() {
        let mut m = Menu::new();
        m.open();
        assert!(m.is_open());
        assert_eq!(m.page, MenuPage::Main);
        assert_eq!(m.selection, 0);
    }

    #[test]
    fn close_resets_state() {
        let mut m = Menu::new();
        m.open();
        m.close();
        assert!(!m.is_open());
    }

    #[test]
    fn navigate_down_wraps() {
        let mut m = Menu::new();
        m.open();
        // Main page has 6 items (0..5).
        m.handle_key(Keycode::Down); // 0 -> 1
        assert_eq!(m.selection, 1);
        m.handle_key(Keycode::Down); // 1 -> 2
        assert_eq!(m.selection, 2);
        m.handle_key(Keycode::Down); // 2 -> 3
        assert_eq!(m.selection, 3);
        m.handle_key(Keycode::Down); // 3 -> 4
        assert_eq!(m.selection, 4);
        m.handle_key(Keycode::Down); // 4 -> 5
        assert_eq!(m.selection, 5);
        m.handle_key(Keycode::Down); // 5 -> 0 (wrap)
        assert_eq!(m.selection, 0);
    }

    #[test]
    fn navigate_up_wraps() {
        let mut m = Menu::new();
        m.open();
        m.handle_key(Keycode::Up); // 0 -> 5 (wrap)
        assert_eq!(m.selection, 5);
    }

    #[test]
    fn enter_debug_page() {
        let mut m = Menu::new();
        m.open();
        m.handle_key(Keycode::Return); // select item 0 = Debug
        assert_eq!(m.page, MenuPage::Debug);
        assert_eq!(m.selection, 0);
    }

    #[test]
    fn toggle_slant_corruption() {
        let mut m = Menu::new();
        m.open();
        m.handle_key(Keycode::Return); // enter Debug
        assert!(!m.slant_corruption());
        m.handle_key(Keycode::Return); // toggle slant_corruption
        assert!(m.slant_corruption());
        m.handle_key(Keycode::Return); // toggle again
        assert!(!m.slant_corruption());
    }

    #[test]
    fn toggle_inaccurate_palette() {
        let mut m = Menu::new();
        m.open();
        m.handle_key(Keycode::Return); // enter Debug
        m.handle_key(Keycode::Down); // move to index 1 (Inaccurate Palette)
        assert!(!m.inaccurate_palette());
        m.handle_key(Keycode::Return); // toggle inaccurate_palette
        assert!(m.inaccurate_palette());
        m.handle_key(Keycode::Return); // toggle again
        assert!(!m.inaccurate_palette());
    }

    #[test]
    fn enter_controller1_page() {
        let mut m = Menu::new();
        m.open();
        m.handle_key(Keycode::Down); // 0 -> 1 (Rewind)
        m.handle_key(Keycode::Down); // 1 -> 2 (Turbo)
        m.handle_key(Keycode::Down); // 2 -> 3 (Controller 1)
        m.handle_key(Keycode::Return); // enter
        assert_eq!(m.page, MenuPage::Controller1);
        assert_eq!(m.selection, 0);
    }

    #[test]
    fn rebind_mode_captures_key() {
        let mut m = Menu::new();
        m.open();
        m.handle_key(Keycode::Down); // -> 1 (Rewind)
        m.handle_key(Keycode::Down); // -> 2 (Turbo)
        m.handle_key(Keycode::Down); // -> 3 (Ctrl1)
        m.handle_key(Keycode::Return); // enter Ctrl1 page
        m.handle_key(Keycode::Return); // rebind button 0 (A)
        assert_eq!(m.rebind_button, Some(0));
        m.handle_key(Keycode::Z); // press Z as new binding
        assert_eq!(m.rebind_button, None);
        let bindings = m.take_pending_bindings();
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0], (0, 0, Keycode::Z));
    }

    #[test]
    fn rebind_mode_escape_cancels() {
        let mut m = Menu::new();
        m.open();
        m.handle_key(Keycode::Down); // -> 1 (Rewind)
        m.handle_key(Keycode::Down); // -> 2 (Turbo)
        m.handle_key(Keycode::Down); // -> 3 (Ctrl1)
        m.handle_key(Keycode::Return); // Ctrl1
        m.handle_key(Keycode::Return); // rebind A
        assert_eq!(m.rebind_button, Some(0));
        m.handle_key(Keycode::Escape); // cancel rebind
        assert_eq!(m.rebind_button, None);
        assert!(m.take_pending_bindings().is_empty());
    }

    #[test]
    fn escape_from_main_closes_menu() {
        let mut m = Menu::new();
        m.open();
        m.handle_key(Keycode::Escape);
        assert!(!m.is_open());
    }

    #[test]
    fn escape_from_subpage_goes_back() {
        let mut m = Menu::new();
        m.open();
        m.handle_key(Keycode::Return); // -> Debug
        m.handle_key(Keycode::Escape); // back to Main
        assert_eq!(m.page, MenuPage::Main);
        assert!(m.is_open());
    }

    #[test]
    fn backspace_goes_back() {
        let mut m = Menu::new();
        m.open();
        m.handle_key(Keycode::Down);
        m.handle_key(Keycode::Down);
        m.handle_key(Keycode::Down); // -> 3 (Ctrl1)
        m.handle_key(Keycode::Return); // -> Ctrl1
        m.handle_key(Keycode::Backspace); // back to Main
        assert_eq!(m.page, MenuPage::Main);
    }

    #[test]
    fn menu_consumes_all_keys_when_open() {
        let mut m = Menu::new();
        m.open();
        // Even unmapped keys should be consumed.
        assert!(m.handle_key(Keycode::F1));
        assert!(m.handle_key(Keycode::Space));
        assert!(m.handle_key(Keycode::A));
    }

    #[test]
    fn menu_does_not_consume_when_closed() {
        let mut m = Menu::new();
        assert!(!m.handle_key(Keycode::F1));
        assert!(!m.handle_key(Keycode::A));
    }

    #[test]
    fn close_menu_item_works() {
        let mut m = Menu::new();
        m.open();
        // Navigate to "Close Menu" (item 5).
        for _ in 0..5 {
            m.handle_key(Keycode::Down);
        }
        assert_eq!(m.selection, 5);
        m.handle_key(Keycode::Return);
        assert!(!m.is_open());
    }

    #[test]
    fn keycode_to_name_covers_common_keys() {
        assert_eq!(keycode_to_name(Keycode::A), "A");
        assert_eq!(keycode_to_name(Keycode::Return), "Return");
        assert_eq!(keycode_to_name(Keycode::Up), "Up");
        assert_eq!(keycode_to_name(Keycode::Space), "Space");
        assert_eq!(keycode_to_name(Keycode::F12), "F12");
    }
}
