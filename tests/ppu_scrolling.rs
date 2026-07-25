//! Integration tests for M10 — PPU scrolling + VBlank NMI timing.
//!
//! These exercise the PPU scanline/cycle stepper and VBlank NMI signalling
//! through the `Bus` (the public entry point the M12 main loop will use),
//! plus the PPUCTRL → `t` nametable-select copy and the per-scanline
//! scroll increments.

use nes_emu::bus::Bus;
use nes_emu::ppu::{
    CYCLES_PER_SCANLINE, SCANLINES_PER_FRAME, SCANLINE_PRERENDER, SCANLINE_VBLANK_START,
};

/// PPU register addresses.
const PPUCTRL: u16 = 0x2000;
const PPUMASK: u16 = 0x2001;
const PPUSTATUS: u16 = 0x2002;

/// PPUMASK value enabling background rendering (triggers scroll increments).
const MASK_SHOW_BG: u8 = 0b0000_1000;

/// Step the PPU by `cycles` cycles through the bus and return whether any
/// NMI was requested.
fn step_ppu(bus: &mut Bus, cycles: u32) -> bool {
    bus.step_ppu(cycles)
}

/// Advance the bus PPU to a specific (scanline, cycle) position.
fn advance_to(bus: &mut Bus, scanline: u16, cycle: u16) {
    let target: u32 = (scanline as u32) * (CYCLES_PER_SCANLINE as u32) + (cycle as u32);
    let mut current: u32 =
        (bus.ppu().scanline() as u32) * (CYCLES_PER_SCANLINE as u32) + (bus.ppu().cycle() as u32);
    while current < target {
        step_ppu(bus, 1);
        current += 1;
    }
}

// ---- PPUCTRL → t nametable-select copy --------------------------------

#[test]
fn ppuctrl_write_sets_t_nametable_bits() {
    let mut bus = Bus::new();
    // PPUCTRL with base NT = 0b11 → t bits 10-11 should be 0b11.
    bus.write(PPUCTRL, 0b0000_0011);
    let t = bus.ppu().temp_vram_addr();
    assert_eq!((t >> 10) & 0b11, 0b11);
}

#[test]
fn ppuctrl_write_overwrites_previous_nt_bits() {
    let mut bus = Bus::new();
    bus.write(PPUCTRL, 0b0000_0010); // nt = 0b10
    assert_eq!((bus.ppu().temp_vram_addr() >> 10) & 0b11, 0b10);
    bus.write(PPUCTRL, 0b0000_0001); // nt = 0b01
    assert_eq!((bus.ppu().temp_vram_addr() >> 10) & 0b11, 0b01);
}

// ---- VBlank NMI timing through the bus --------------------------------

#[test]
fn bus_step_ppu_advances_cycle() {
    let mut bus = Bus::new();
    assert_eq!(bus.ppu().cycle(), 0);
    step_ppu(&mut bus, 1);
    assert_eq!(bus.ppu().cycle(), 1);
    step_ppu(&mut bus, 10);
    assert_eq!(bus.ppu().cycle(), 11);
}

#[test]
fn bus_step_ppu_wraps_scanline() {
    let mut bus = Bus::new();
    step_ppu(&mut bus, CYCLES_PER_SCANLINE as u32);
    assert_eq!(bus.ppu().scanline(), 1);
    assert_eq!(bus.ppu().cycle(), 0);
}

#[test]
fn vblank_flag_set_at_scanline_241() {
    let mut bus = Bus::new();
    bus.write(PPUCTRL, 0x00); // NMI disabled
    advance_to(&mut bus, SCANLINE_VBLANK_START, 0);
    assert!(!bus.ppu().in_vblank());
    step_ppu(&mut bus, 1); // → cycle 1 → VBlank asserted
    assert!(bus.ppu().in_vblank());
}

#[test]
fn nmi_requested_when_nmi_enabled_at_vblank() {
    let mut bus = Bus::new();
    bus.write(PPUCTRL, 0x80); // NMI enabled
    advance_to(&mut bus, SCANLINE_VBLANK_START, 0);
    let nmi = step_ppu(&mut bus, 1);
    assert!(nmi, "bus.step_ppu returns true when NMI requested");
    assert!(bus.take_nmi_request(), "NMI request latched");
    assert!(!bus.take_nmi_request(), "NMI request cleared after take");
}

#[test]
fn nmi_not_requested_when_nmi_disabled() {
    let mut bus = Bus::new();
    bus.write(PPUCTRL, 0x00); // NMI disabled
    advance_to(&mut bus, SCANLINE_VBLANK_START, 0);
    let nmi = step_ppu(&mut bus, 1);
    assert!(!nmi);
    assert!(!bus.take_nmi_request());
    // VBlank flag still set.
    assert!(bus.ppu().in_vblank());
}

#[test]
fn vblank_cleared_at_prerender_cycle_1() {
    let mut bus = Bus::new();
    bus.write(PPUCTRL, 0x00);
    // Enter VBlank.
    advance_to(&mut bus, SCANLINE_VBLANK_START, 5);
    assert!(bus.ppu().in_vblank());
    // Advance to prerender scanline, cycle 0.
    advance_to(&mut bus, SCANLINE_PRERENDER, 0);
    assert!(
        bus.ppu().in_vblank(),
        "still in VBlank before prerender cycle 1"
    );
    step_ppu(&mut bus, 1);
    assert!(
        !bus.ppu().in_vblank(),
        "VBlank cleared at prerender cycle 1"
    );
}

#[test]
fn sprite_overflow_cleared_at_prerender() {
    let mut bus = Bus::new();
    bus.ppu_mut().set_sprite_overflow(true);
    advance_to(&mut bus, SCANLINE_PRERENDER, 0);
    assert_eq!(bus.ppu().ppustatus() & 0b0010_0000, 0b0010_0000);
    step_ppu(&mut bus, 1);
    assert_eq!(bus.ppu().ppustatus() & 0b0010_0000, 0);
}

#[test]
fn sprite_zero_hit_cleared_at_prerender() {
    let mut bus = Bus::new();
    bus.ppu_mut().set_sprite_zero_hit(true);
    advance_to(&mut bus, SCANLINE_PRERENDER, 0);
    assert_eq!(bus.ppu().ppustatus() & 0b0100_0000, 0b0100_0000);
    step_ppu(&mut bus, 1);
    assert_eq!(bus.ppu().ppustatus() & 0b0100_0000, 0);
}

// ---- Full-frame timing ------------------------------------------------

#[test]
fn one_full_frame_is_262_times_341_cycles() {
    let mut bus = Bus::new();
    let total: u32 = (SCANLINES_PER_FRAME as u32) * (CYCLES_PER_SCANLINE as u32);
    step_ppu(&mut bus, total);
    // After exactly one frame we should be back at scanline 0, cycle 0.
    assert_eq!(bus.ppu().scanline(), 0);
    assert_eq!(bus.ppu().cycle(), 0);
}

#[test]
fn exactly_one_nmi_per_frame_with_nmi_enabled() {
    let mut bus = Bus::new();
    bus.write(PPUCTRL, 0x80); // NMI enabled
    let total: u32 = (SCANLINES_PER_FRAME as u32) * (CYCLES_PER_SCANLINE as u32);
    let mut nmi_count = 0u32;
    for _ in 0..total {
        if step_ppu(&mut bus, 1) {
            nmi_count += 1;
        }
    }
    assert_eq!(nmi_count, 1, "exactly one NMI per frame");
}

#[test]
fn two_frames_produce_two_nmis() {
    let mut bus = Bus::new();
    bus.write(PPUCTRL, 0x80);
    let total: u32 = 2 * (SCANLINES_PER_FRAME as u32) * (CYCLES_PER_SCANLINE as u32);
    let mut nmi_count = 0u32;
    for _ in 0..total {
        if step_ppu(&mut bus, 1) {
            nmi_count += 1;
        }
    }
    assert_eq!(nmi_count, 2);
}

#[test]
fn nmi_request_persists_until_taken() {
    let mut bus = Bus::new();
    bus.write(PPUCTRL, 0x80);
    advance_to(&mut bus, SCANLINE_VBLANK_START, 0);
    step_ppu(&mut bus, 1);
    // Run 100 more cycles without taking the request.
    step_ppu(&mut bus, 100);
    // The request should still be pending.
    assert!(bus.take_nmi_request(), "NMI request persists until taken");
}

// ---- Scroll increments during rendering -------------------------------

#[test]
fn h_scroll_increment_during_rendering() {
    let mut bus = Bus::new();
    bus.write(PPUMASK, MASK_SHOW_BG);
    // After 8 cycles (dot 8), coarse X in v should have incremented from 0 to 1.
    // fine_x (PPUSCROLL) must remain unchanged.
    step_ppu(&mut bus, 8);
    assert_eq!(bus.ppu().cycle(), 8);
    assert_eq!(bus.ppu().vram_addr() & 0x1F, 1);
    assert_eq!(bus.ppu().fine_x(), 0);
}

#[test]
fn no_h_scroll_increment_when_rendering_disabled() {
    let mut bus = Bus::new();
    // PPUMASK = 0 → rendering disabled.
    step_ppu(&mut bus, 8);
    assert_eq!(bus.ppu().fine_x(), 0);
}

#[test]
fn h_scroll_increments_31_times_per_scanline() {
    let mut bus = Bus::new();
    bus.write(PPUMASK, MASK_SHOW_BG);
    // Advance through one full visible scanline (cycles 8, 16, ..., 248
    // = 31 h-scroll increments of coarse X in v). fine_x must remain at 0.
    step_ppu(&mut bus, CYCLES_PER_SCANLINE as u32);
    // After 31 coarse-X increments from 0: 0 + 31 = 31 (no wrap).
    // Then at cycle 257, copy_h_t_to_v resets coarse X from t (0).
    // fine_x must not be modified during rendering.
    assert_eq!(
        bus.ppu().fine_x(),
        0,
        "fine_x must not be modified during rendering"
    );
}

#[test]
fn v_scroll_increment_at_cycle_256() {
    let mut bus = Bus::new();
    bus.write(PPUMASK, MASK_SHOW_BG);
    // Advance to cycle 256 of scanline 0.
    step_ppu(&mut bus, 256);
    assert_eq!(bus.ppu().cycle(), 256);
    // fine Y (v bits 12-14) should have advanced from 0 to 1.
    let v = bus.ppu().vram_addr();
    assert_eq!((v >> 12) & 0x07, 1, "fine Y incremented at cycle 256");
}

#[test]
fn h_t_to_v_copy_at_cycle_257() {
    let mut bus = Bus::new();
    bus.write(PPUMASK, MASK_SHOW_BG);
    // Set t's coarse X = 10, nt = 0b11 via PPUCTRL + PPUSCROLL.
    bus.write(PPUCTRL, 0b0000_0011); // nt → 0b11
    bus.write(0x2005, 0x50); // PPUSCROLL first: coarse X = 10, fine X = 0
    bus.write(0x2005, 0x00); // PPUSCROLL second: coarse Y = 0, fine Y = 0
                             // Advance to cycle 257 of scanline 0.
    step_ppu(&mut bus, 257);
    // v should now have t's coarse X (10) and nt H bit (bit 10 only).
    let v = bus.ppu().vram_addr();
    assert_eq!(v & 0b0000_0000_0001_1111, 10, "coarse X copied from t");
    assert_eq!((v >> 10) & 0b01, 0b01, "nt H bit copied from t");
}

#[test]
fn v_t_to_v_copy_at_prerender_280_304() {
    let mut bus = Bus::new();
    bus.write(PPUMASK, MASK_SHOW_BG);
    // Set t's coarse Y = 15, fine Y = 5, nt = 0b10.
    bus.write(PPUCTRL, 0b0000_0010); // nt → 0b10
                                     // PPUSCROLL second write: coarse Y = 15, fine Y = 5 → (15<<3)|5 = 0x7D.
    bus.write(0x2005, 0x00);
    bus.write(0x2005, 0x7D);
    // Advance to prerender scanline, cycle 304 (last V-copy cycle).
    advance_to(&mut bus, SCANLINE_PRERENDER, 304);
    // v should now have t's coarse Y (15), fine Y (5), nt V bit (bit 11).
    let v = bus.ppu().vram_addr();
    assert_eq!((v >> 5) & 0b11111, 15, "coarse Y copied from t");
    assert_eq!((v >> 12) & 0x07, 5, "fine Y copied from t");
    assert_eq!((v >> 10) & 0b10, 0b10, "nt V bit copied from t");
}

// ---- PPUSTATUS read clears VBlank (M7 behaviour, re-verified) ---------

#[test]
fn ppustatus_read_clears_vblank_flag() {
    let mut bus = Bus::new();
    bus.write(PPUCTRL, 0x00);
    advance_to(&mut bus, SCANLINE_VBLANK_START, 5);
    assert!(bus.ppu().in_vblank());
    let status = bus.read(PPUSTATUS);
    assert_eq!(status & 0x80, 0x80, "VBlank bit visible in PPUSTATUS");
    assert!(!bus.ppu().in_vblank(), "VBlank cleared by PPUSTATUS read");
}

// ---- NMI-during-VBlank quirk ------------------------------------------
//
// Per NESdev: "If the PPU's NMI enable bit (PPUCTRL bit 7) is set during
// VBlank, an NMI will be generated at that moment." Games rely on this
// when enabling NMI inside the VBlank handler.

#[test]
fn nmi_enable_toggled_on_during_vblank_immediately_requests_nmi() {
    let mut bus = Bus::new();
    bus.write(PPUCTRL, 0x00); // NMI disabled at VBlank start
    advance_to(&mut bus, SCANLINE_VBLANK_START, 1);
    assert!(bus.ppu().in_vblank());
    assert!(
        !bus.take_nmi_request(),
        "no NMI request when disabled at VBlank"
    );
    // Enable NMI mid-VBlank → NMI generated immediately.
    bus.write(PPUCTRL, 0x80);
    assert!(
        bus.take_nmi_request(),
        "NMI requested when PPUCTRL bit 7 set during VBlank"
    );
}

#[test]
fn nmi_enable_toggled_on_outside_vblank_does_not_request() {
    let mut bus = Bus::new();
    // Enable NMI during a visible scanline (not VBlank) → no immediate NMI.
    advance_to(&mut bus, 100, 50);
    assert!(!bus.ppu().in_vblank());
    bus.write(PPUCTRL, 0x80);
    assert!(
        !bus.take_nmi_request(),
        "no NMI when enabled outside VBlank"
    );
}

#[test]
fn nmi_enable_toggled_off_before_vblank_start_suppresses_nmi() {
    let mut bus = Bus::new();
    bus.write(PPUCTRL, 0x80); // NMI enabled
    advance_to(&mut bus, SCANLINE_VBLANK_START - 1, 340);
    // Disable NMI just before VBlank starts.
    bus.write(PPUCTRL, 0x00);
    step_ppu(&mut bus, 1); // → scanline 241, cycle 0
    step_ppu(&mut bus, 1); // → cycle 1 → VBlank asserted, no NMI
    assert!(bus.ppu().in_vblank());
    assert!(
        !bus.take_nmi_request(),
        "NMI suppressed when disabled before VBlank"
    );
}
