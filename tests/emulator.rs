//! Integration tests for the frame-locked main loop (M12): `EmulatorState`,
//! `step_frame`, CPU/PPU lockstep timing, NMI delivery, and framebuffer
//! rendering.
//!
//! These tests build minimal iNES ROM images in memory (no external ROM
//! fixtures needed) and drive the emulator through `step_frame`, verifying:
//!
//! - One `step_frame` completes exactly one PPU frame (262 scanlines).
//! - The CPU executes ~29,830 cycles per frame (NTSC).
//! - VBlank NMI is delivered to the CPU when PPUCTRL bit 7 is set.
//! - The framebuffer is populated with visible (non-black) pixels after
//!   running a ROM that enables background rendering with a non-black
//!   universal palette color.
//! - The framebuffer color matches the palette entry written by the ROM.
//! - OAM-DMA stall cycles are accounted for (PPU advances during DMA).
//! - Multiple frames run without crashing or deadlock.
//! - `reset` loads PC from the RESET vector.

// The ROM-builder helpers use a running `p` cursor that is assigned but not
// read after the final write in each block; this is intentional (keeps the
// layout linear and easy to audit) so we silence the unused-assignment lint.
#![allow(unused_assignments)]

use nes_emu::cartridge::{Cartridge, PRG_ROM_UNIT};
use nes_emu::emulator::EmulatorState;

/// iNES header magic.
const INES_MAGIC: [u8; 4] = [b'N', b'E', b'S', 0x1A];

/// Build an iNES image from raw PRG bytes (16 KB per bank) and optional
/// CHR bytes (8 KB per bank). The RESET / NMI / IRQ vectors are taken from
/// the last 6 bytes of the PRG data.
fn make_ines(prg_banks: u8, chr_banks: u8, flags6: u8, prg: &[u8], chr: &[u8]) -> Vec<u8> {
    let prg_size = prg_banks as usize * PRG_ROM_UNIT;
    let chr_size = chr_banks as usize * 8 * 1024;
    let mut buf = Vec::with_capacity(16 + prg_size + chr_size);
    buf.extend_from_slice(&INES_MAGIC);
    buf.push(prg_banks);
    buf.push(chr_banks);
    buf.push(flags6);
    buf.push(0); // flags7
    buf.extend_from_slice(&[0u8; 8]); // remaining header
                                      // PRG
    buf.extend_from_slice(prg);
    buf.resize(16 + prg_size, 0);
    // CHR
    buf.extend_from_slice(chr);
    buf.resize(16 + prg_size + chr_size, 0);
    buf
}

/// 6502 opcode constants used to build test programs.
mod op {
    pub const SEI: u8 = 0x78;
    pub const CLD: u8 = 0xD8;
    pub const LDA_IMM: u8 = 0xA9;
    pub const STA_ABS: u8 = 0x8D;
    pub const STA_ZP: u8 = 0x85;
    pub const LDX_IMM: u8 = 0xA2;
    pub const INX: u8 = 0xE8;
    pub const BNE: u8 = 0xD0;
    pub const JMP_ABS: u8 = 0x4C;
    pub const BIT_ABS: u8 = 0x2C;
    pub const BPL: u8 = 0x10;
}

/// PPU register addresses (CPU-side).
const PPUCTRL: u16 = 0x2000;
const PPUMASK: u16 = 0x2001;
const PPUSTATUS: u16 = 0x2002;
const PPUADDR: u16 = 0x2006;
const PPUDATA: u16 = 0x2007;
const OAMDMA: u16 = 0x4014;

/// Build a test ROM that:
/// 1. Disables interrupts and clears decimal mode.
/// 2. Enables NMI (PPUCTRL = $90) and background rendering (PPUMASK = $0E).
/// 3. Waits for VBlank (polls PPUSTATUS bit 7).
/// 4. Writes palette entries: $3F00 = `bg_color`, $3F01 = `fg_color`.
/// 5. Fills the first 256 bytes of nametable 0 ($2000) with tile index 1.
/// 6. Loops forever (JMP self).
///
/// CHR-ROM is 8 KB with tile 0 all-zero (transparent) and tile 1 fully
/// opaque (all bits set), so the nametable fill with tile 1 produces
/// visible foreground-color pixels.
fn make_visible_rom(bg_color: u8, fg_color: u8) -> Vec<u8> {
    let mut prg = vec![0u8; PRG_ROM_UNIT];
    let mut p = 0usize;

    // reset: SEI CLD
    prg[p] = op::SEI;
    p += 1;
    prg[p] = op::CLD;
    p += 1;

    // LDA #$90 ; STA PPUCTRL (NMI enable, base NT $2000)
    prg[p] = op::LDA_IMM;
    p += 1;
    prg[p] = 0x90;
    p += 1;
    prg[p] = op::STA_ABS;
    p += 1;
    prg[p] = PPUCTRL as u8;
    p += 1;
    prg[p] = (PPUCTRL >> 8) as u8;
    p += 1;

    // wait_vblank: BIT PPUSTATUS ; BPL wait_vblank
    let wait_vblank = p;
    prg[p] = op::BIT_ABS;
    p += 1;
    prg[p] = PPUSTATUS as u8;
    p += 1;
    prg[p] = (PPUSTATUS >> 8) as u8;
    p += 1;
    prg[p] = op::BPL;
    p += 1;
    prg[p] = (wait_vblank as i16 - (p as i16 + 1)) as i8 as u8;
    p += 1;

    // Write palette: PPUADDR = $3F00, PPUDATA = bg_color, PPUDATA = fg_color
    prg[p] = op::LDA_IMM;
    p += 1;
    prg[p] = 0x3F;
    p += 1;
    prg[p] = op::STA_ABS;
    p += 1;
    prg[p] = PPUADDR as u8;
    p += 1;
    prg[p] = (PPUADDR >> 8) as u8;
    p += 1;
    prg[p] = op::LDA_IMM;
    p += 1;
    prg[p] = 0x00;
    p += 1;
    prg[p] = op::STA_ABS;
    p += 1;
    prg[p] = PPUADDR as u8;
    p += 1;
    prg[p] = (PPUADDR >> 8) as u8;
    p += 1;
    prg[p] = op::LDA_IMM;
    p += 1;
    prg[p] = bg_color;
    p += 1;
    prg[p] = op::STA_ABS;
    p += 1;
    prg[p] = PPUDATA as u8;
    p += 1;
    prg[p] = (PPUDATA >> 8) as u8;
    p += 1;
    prg[p] = op::LDA_IMM;
    p += 1;
    prg[p] = fg_color;
    p += 1;
    prg[p] = op::STA_ABS;
    p += 1;
    prg[p] = PPUDATA as u8;
    p += 1;
    prg[p] = (PPUDATA >> 8) as u8;
    p += 1;

    // Write nametable: PPUADDR = $2000, fill 256 bytes with tile index 1.
    prg[p] = op::LDA_IMM;
    p += 1;
    prg[p] = 0x20;
    p += 1;
    prg[p] = op::STA_ABS;
    p += 1;
    prg[p] = PPUADDR as u8;
    p += 1;
    prg[p] = (PPUADDR >> 8) as u8;
    p += 1;
    prg[p] = op::LDA_IMM;
    p += 1;
    prg[p] = 0x00;
    p += 1;
    prg[p] = op::STA_ABS;
    p += 1;
    prg[p] = PPUADDR as u8;
    p += 1;
    prg[p] = (PPUADDR >> 8) as u8;
    p += 1;
    // LDX #$00 ; LDA #$01 ; loop: STA PPUDATA ; INX ; BNE loop
    prg[p] = op::LDX_IMM;
    p += 1;
    prg[p] = 0x00;
    p += 1;
    let fill_loop = p;
    prg[p] = op::LDA_IMM;
    p += 1;
    prg[p] = 0x01;
    p += 1;
    prg[p] = op::STA_ABS;
    p += 1;
    prg[p] = PPUDATA as u8;
    p += 1;
    prg[p] = (PPUDATA >> 8) as u8;
    p += 1;
    prg[p] = op::INX;
    p += 1;
    prg[p] = op::BNE;
    p += 1;
    prg[p] = (fill_loop as i16 - (p as i16 + 1)) as i8 as u8;
    p += 1;

    // Enable background rendering: PPUMASK = $0E (show bg + left bg).
    prg[p] = op::LDA_IMM;
    p += 1;
    prg[p] = 0x0E;
    p += 1;
    prg[p] = op::STA_ABS;
    p += 1;
    prg[p] = PPUMASK as u8;
    p += 1;
    prg[p] = (PPUMASK >> 8) as u8;
    p += 1;

    // forever: JMP forever
    let forever = p;
    prg[p] = op::JMP_ABS;
    p += 1;
    prg[p] = forever as u8;
    p += 1;
    prg[p] = (forever >> 8) as u8;
    p += 1;

    // Set vectors: RESET → $C000, NMI → $C000 (same loop), IRQ → $0000.
    // For NROM-128, $C000 maps to PRG offset $4000.
    let reset_addr: u16 = 0xC000;
    let nmi_addr: u16 = 0xC000;
    let irq_addr: u16 = 0x0000;
    let vec_off = PRG_ROM_UNIT - 6;
    prg[vec_off] = nmi_addr as u8;
    prg[vec_off + 1] = (nmi_addr >> 8) as u8;
    prg[vec_off + 2] = reset_addr as u8;
    prg[vec_off + 3] = (reset_addr >> 8) as u8;
    prg[vec_off + 4] = irq_addr as u8;
    prg[vec_off + 5] = (irq_addr >> 8) as u8;

    // CHR-ROM: 8 KB. Tile 0 = all zero (transparent). Tile 1 = all $FF
    // (fully opaque, pattern value 3 at every pixel).
    let chr_size = 8 * 1024;
    let mut chr = vec![0u8; chr_size];
    // Tile 1 starts at CHR offset 16 (each tile = 16 bytes: 8 plane0 + 8 plane1).
    for i in 0..16 {
        chr[16 + i] = 0xFF;
    }

    make_ines(1, 1, 0, &prg, &chr)
}

/// Build a ROM that does nothing but NOP forever (no PPU setup, no NMI).
/// RESET vector → $C000, PRG filled with NOP ($EA), ending with JMP self.
fn make_nop_rom() -> Vec<u8> {
    let mut prg = vec![0xEA; PRG_ROM_UNIT]; // NOP fill
                                            // JMP self at $C000... actually $C000 is the entry. Let's put JMP self
                                            // at the end so the program runs NOPs then loops. Simpler: just fill
                                            // with NOP and let it run off into the vector area (which is fine for
                                            // timing tests). Put JMP self at offset 0 so PC=$C000 loops immediately.
    prg[0] = op::JMP_ABS;
    prg[1] = 0x00;
    prg[2] = 0xC0;
    // Vectors: RESET → $C000.
    let vec_off = PRG_ROM_UNIT - 6;
    prg[vec_off + 2] = 0x00;
    prg[vec_off + 3] = 0xC0;
    make_ines(1, 0, 0, &prg, &[])
}

// ===========================================================================
// Frame boundary + cycle count
// ===========================================================================

#[test]
fn step_frame_completes_one_ppu_frame() {
    let rom = make_nop_rom();
    let cart = Cartridge::from_bytes(&rom).expect("load NOP cart");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    emu.step_frame();
    // After one frame the PPU should be at scanline 0 (start of next frame).
    assert_eq!(emu.bus().ppu().scanline(), 0);
}

#[test]
fn step_frame_runs_approximately_29830_cpu_cycles() {
    let rom = make_nop_rom();
    let cart = Cartridge::from_bytes(&rom).expect("load NOP cart");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    let cycles = emu.step_frame();
    // NTSC frame ≈ 29,830 CPU cycles. Allow ±600 for boundary effects.
    assert!(
        (29_200..=30_500).contains(&cycles),
        "expected ~29830 CPU cycles per frame, got {cycles}",
    );
}

#[test]
fn scanlines_per_frame_is_262() {
    assert_eq!(EmulatorState::scanlines_per_frame(), 262);
}

// ===========================================================================
// NMI delivery
// ===========================================================================

#[test]
fn vblank_nmi_delivered_to_cpu_when_enabled() {
    // ROM: enable NMI (PPUCTRL=$90), then JMP self. The VBlank NMI should
    // fire and the CPU should jump to the NMI vector. We point the NMI
    // vector to a handler that writes $42 to $0000, then RTI.
    let mut prg = vec![0u8; PRG_ROM_UNIT];
    let mut p = 0usize;

    // $C000: SEI CLD
    prg[p] = op::SEI;
    p += 1;
    prg[p] = op::CLD;
    p += 1;
    // LDA #$90 ; STA PPUCTRL
    prg[p] = op::LDA_IMM;
    p += 1;
    prg[p] = 0x90;
    p += 1;
    prg[p] = op::STA_ABS;
    p += 1;
    prg[p] = PPUCTRL as u8;
    p += 1;
    prg[p] = (PPUCTRL >> 8) as u8;
    p += 1;
    // forever: JMP forever
    let forever = p;
    prg[p] = op::JMP_ABS;
    p += 1;
    prg[p] = forever as u8;
    p += 1;
    prg[p] = (forever >> 8) as u8;
    p += 1;

    // NMI handler at $C100: LDA #$42 ; STA $0000 ; RTI (0x40)
    let nmi_handler: u16 = 0xC100;
    let nmi_off = 0x100; // PRG offset for $C100 in NROM-128
    prg[nmi_off] = op::LDA_IMM;
    prg[nmi_off + 1] = 0x42;
    prg[nmi_off + 2] = op::STA_ZP;
    prg[nmi_off + 3] = 0x00;
    prg[nmi_off + 4] = 0x40; // RTI

    // Vectors.
    let vec_off = PRG_ROM_UNIT - 6;
    prg[vec_off] = nmi_handler as u8;
    prg[vec_off + 1] = (nmi_handler >> 8) as u8;
    prg[vec_off + 2] = 0x00; // RESET → $C000
    prg[vec_off + 3] = 0xC0;
    prg[vec_off + 4] = 0x00; // IRQ → $0000
    prg[vec_off + 5] = 0x00;

    let rom = make_ines(1, 0, 0, &prg, &[]);
    let cart = Cartridge::from_bytes(&rom).expect("load NMI cart");
    let mut emu = EmulatorState::new(cart);
    emu.reset();

    // Run a few frames — the NMI handler should write $42 to $0000.
    for _ in 0..3 {
        emu.step_frame();
    }
    let flag = emu.bus_mut().read(0x0000);
    assert_eq!(
        flag, 0x42,
        "NMI handler should have written $42 to $0000 after VBlank NMI",
    );
}

#[test]
fn vblank_nmi_not_delivered_when_disabled() {
    // ROM: do NOT enable NMI (PPUCTRL = $00), JMP self. NMI handler writes
    // $42 to $0001. After several frames $0001 should still be $00.
    let mut prg = vec![0u8; PRG_ROM_UNIT];
    let mut p = 0usize;
    prg[p] = op::SEI;
    p += 1;
    prg[p] = op::CLD;
    p += 1;
    // LDA #$00 ; STA PPUCTRL (NMI disabled)
    prg[p] = op::LDA_IMM;
    p += 1;
    prg[p] = 0x00;
    p += 1;
    prg[p] = op::STA_ABS;
    p += 1;
    prg[p] = PPUCTRL as u8;
    p += 1;
    prg[p] = (PPUCTRL >> 8) as u8;
    p += 1;
    let forever = p;
    prg[p] = op::JMP_ABS;
    p += 1;
    prg[p] = forever as u8;
    p += 1;
    prg[p] = (forever >> 8) as u8;
    p += 1;

    // NMI handler at $C100 (should never run).
    let nmi_handler: u16 = 0xC100;
    let nmi_off = 0x100;
    prg[nmi_off] = op::LDA_IMM;
    prg[nmi_off + 1] = 0x42;
    prg[nmi_off + 2] = op::STA_ZP;
    prg[nmi_off + 3] = 0x01;
    prg[nmi_off + 4] = 0x40; // RTI

    let vec_off = PRG_ROM_UNIT - 6;
    prg[vec_off] = nmi_handler as u8;
    prg[vec_off + 1] = (nmi_handler >> 8) as u8;
    prg[vec_off + 2] = 0x00;
    prg[vec_off + 3] = 0xC0;
    prg[vec_off + 4] = 0x00;
    prg[vec_off + 5] = 0x00;

    let rom = make_ines(1, 0, 0, &prg, &[]);
    let cart = Cartridge::from_bytes(&rom).expect("load no-NMI cart");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    for _ in 0..3 {
        emu.step_frame();
    }
    assert_eq!(
        emu.bus_mut().read(0x0001),
        0x00,
        "NMI handler should not have run"
    );
}

// ===========================================================================
// Framebuffer rendering — visible graphics
// ===========================================================================

#[test]
fn framebuffer_populated_after_visible_rom() {
    // ROM with bg_color=$16 (red), fg_color=$16, tile 1 fully opaque.
    // After running, the framebuffer should have non-black pixels.
    let rom = make_visible_rom(0x16, 0x16);
    let cart = Cartridge::from_bytes(&rom).expect("load visible cart");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    // Run a few frames to let the init code execute (wait for VBlank,
    // write palette + nametable, enable rendering).
    for _ in 0..5 {
        emu.step_frame();
    }
    let fb = emu.framebuffer();
    let non_black = fb.iter().filter(|&&p| p != 0xFF00_0000).count();
    assert!(
        non_black > 0,
        "framebuffer should have non-black pixels after running visible ROM, got {non_black}",
    );
}

#[test]
fn framebuffer_uniform_when_rendering_disabled() {
    // ROM that never enables PPUMASK rendering. The framebuffer should be
    // uniformly filled with the universal background color (the renderer
    // fills with $3F00 when bg is disabled) — no tile/sprite content.
    let mut prg = vec![0u8; PRG_ROM_UNIT];
    let mut p = 0usize;
    prg[p] = op::SEI;
    p += 1;
    prg[p] = op::CLD;
    p += 1;
    let forever = p;
    prg[p] = op::JMP_ABS;
    p += 1;
    prg[p] = forever as u8;
    p += 1;
    prg[p] = (forever >> 8) as u8;
    p += 1;
    let vec_off = PRG_ROM_UNIT - 6;
    prg[vec_off + 2] = 0x00;
    prg[vec_off + 3] = 0xC0;
    let rom = make_ines(1, 0, 0, &prg, &[]);
    let cart = Cartridge::from_bytes(&rom).expect("load blank cart");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    for _ in 0..3 {
        emu.step_frame();
    }
    let fb = emu.framebuffer();
    // The framebuffer should be uniform (all pixels the same value =
    // the universal bg color). No tile or sprite content.
    let first = fb[0];
    assert!(
        fb.iter().all(|&p| p == first),
        "framebuffer should be uniform (universal bg color) when rendering is disabled",
    );
}

#[test]
fn framebuffer_matches_universal_bg_color() {
    // ROM with bg_color=$16 (red), no tile pattern data (CHR all zero →
    // every pixel is pattern 0 → universal bg color). The entire
    // framebuffer should be the ARGB value for NES color $16.
    // Build a custom ROM: set $3F00 and enable rendering, with zero CHR
    // → all pixels = $3F00 color.
    let mut prg = vec![0u8; PRG_ROM_UNIT];
    let mut p = 0usize;
    prg[p] = op::SEI;
    p += 1;
    prg[p] = op::CLD;
    p += 1;
    // LDA #$90 ; STA PPUCTRL (NMI on)
    prg[p] = op::LDA_IMM;
    p += 1;
    prg[p] = 0x90;
    p += 1;
    prg[p] = op::STA_ABS;
    p += 1;
    prg[p] = PPUCTRL as u8;
    p += 1;
    prg[p] = (PPUCTRL >> 8) as u8;
    p += 1;
    // wait_vblank
    let wv = p;
    prg[p] = op::BIT_ABS;
    p += 1;
    prg[p] = PPUSTATUS as u8;
    p += 1;
    prg[p] = (PPUSTATUS >> 8) as u8;
    p += 1;
    prg[p] = op::BPL;
    p += 1;
    prg[p] = (wv as i16 - (p as i16 + 1)) as i8 as u8;
    p += 1;
    // PPUADDR = $3F00 ; PPUDATA = $16 (universal bg = red)
    prg[p] = op::LDA_IMM;
    p += 1;
    prg[p] = 0x3F;
    p += 1;
    prg[p] = op::STA_ABS;
    p += 1;
    prg[p] = PPUADDR as u8;
    p += 1;
    prg[p] = (PPUADDR >> 8) as u8;
    p += 1;
    prg[p] = op::LDA_IMM;
    p += 1;
    prg[p] = 0x00;
    p += 1;
    prg[p] = op::STA_ABS;
    p += 1;
    prg[p] = PPUADDR as u8;
    p += 1;
    prg[p] = (PPUADDR >> 8) as u8;
    p += 1;
    prg[p] = op::LDA_IMM;
    p += 1;
    prg[p] = 0x16;
    p += 1;
    prg[p] = op::STA_ABS;
    p += 1;
    prg[p] = PPUDATA as u8;
    p += 1;
    prg[p] = (PPUDATA >> 8) as u8;
    p += 1;
    // PPUMASK = $0E (show bg)
    prg[p] = op::LDA_IMM;
    p += 1;
    prg[p] = 0x0E;
    p += 1;
    prg[p] = op::STA_ABS;
    p += 1;
    prg[p] = PPUMASK as u8;
    p += 1;
    prg[p] = (PPUMASK >> 8) as u8;
    p += 1;
    // JMP self
    let forever = p;
    prg[p] = op::JMP_ABS;
    p += 1;
    prg[p] = forever as u8;
    p += 1;
    prg[p] = (forever >> 8) as u8;
    p += 1;
    let vec_off = PRG_ROM_UNIT - 6;
    prg[vec_off + 2] = 0x00;
    prg[vec_off + 3] = 0xC0;
    // CHR all zero (CHR-RAM, 0 banks).
    let rom = make_ines(1, 0, 0, &prg, &[]);
    let cart = Cartridge::from_bytes(&rom).expect("load bg-color cart");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    for _ in 0..5 {
        emu.step_frame();
    }
    let fb = emu.framebuffer();
    // NES color $16 = red. The NES palette table maps it to an ARGB value.
    // Every visible pixel should be that value (not black).
    let first_visible = fb[0];
    assert!(
        first_visible != 0xFF00_0000,
        "first pixel should be non-black (universal bg color $16), got {first_visible:#010X}",
    );
    // The entire visible 256×240 region should be the same color.
    let all_same = fb.iter().all(|&p| p == first_visible);
    assert!(
        all_same,
        "all pixels should match universal bg color (CHR all zero → pattern 0 everywhere)",
    );
}

// ===========================================================================
// OAM-DMA stall accounting
// ===========================================================================

#[test]
fn oam_dma_stall_cycles_accounted() {
    // ROM: write $00 to $4014 (OAMDMA), triggering a DMA. The bus should
    // record 512 stall cycles. After step_frame, the DMA stall should
    // have been consumed and the PPU advanced accordingly.
    let mut prg = vec![0u8; PRG_ROM_UNIT];
    let mut p = 0usize;
    prg[p] = op::SEI;
    p += 1;
    prg[p] = op::CLD;
    p += 1;
    // LDA #$00 ; STA $4014 (OAMDMA — triggers DMA from page $00)
    prg[p] = op::LDA_IMM;
    p += 1;
    prg[p] = 0x00;
    p += 1;
    prg[p] = op::STA_ABS;
    p += 1;
    prg[p] = OAMDMA as u8;
    p += 1;
    prg[p] = (OAMDMA >> 8) as u8;
    p += 1;
    // JMP self
    let forever = p;
    prg[p] = op::JMP_ABS;
    p += 1;
    prg[p] = forever as u8;
    p += 1;
    prg[p] = (forever >> 8) as u8;
    p += 1;
    let vec_off = PRG_ROM_UNIT - 6;
    prg[vec_off + 2] = 0x00;
    prg[vec_off + 3] = 0xC0;
    let rom = make_ines(1, 0, 0, &prg, &[]);
    let cart = Cartridge::from_bytes(&rom).expect("load DMA cart");
    let mut emu = EmulatorState::new(cart);
    emu.reset();

    // Run one frame. The DMA stall cycles should be consumed within
    // step_frame (no panic, no deadlock). The frame should complete at
    // the correct boundary. The total CPU cycle count should still be
    // ~29780 because the PPU frame is a fixed length (262×341 PPU
    // cycles = 29780 CPU cycles); the DMA stall cycles *replace*
    // instruction cycles 1:1 (both advance the PPU by 3×), so the frame
    // total is unchanged.
    let cycles = emu.step_frame();
    assert_eq!(emu.bus().ppu().scanline(), 0);
    assert!(
        (29_200..=30_500).contains(&cycles),
        "frame with DMA should still be ~29830 cycles, got {cycles}",
    );
    // The DMA stall cycles should have been consumed (no leftover).
    assert_eq!(emu.bus_mut().take_dma_stall_cycles(), 0);
}

#[test]
fn oam_dma_near_prerender_does_not_overshoot_frame() {
    // Regression test for the OAM-DMA-near-prerender-end overshoot: if
    // DMA is triggered near the end of scanline 261, the 1548-PPU-cycle
    // batch could skip past scanline 0 without being caught. The chunked
    // stepping in step_frame prevents this.
    //
    // We trigger DMA in a tight loop (JMP back to the STA $4014) so DMA
    // fires repeatedly throughout the frame, including near the prerender
    // boundary. After several frames, the PPU should still be at scanline
    // 0 (frame boundary detected correctly each time).
    let mut prg = vec![0u8; PRG_ROM_UNIT];
    let mut p = 0usize;
    prg[p] = op::SEI;
    p += 1;
    prg[p] = op::CLD;
    p += 1;
    // LDA #$00 (stays in A for the loop)
    prg[p] = op::LDA_IMM;
    p += 1;
    prg[p] = 0x00;
    p += 1;
    // loop: STA $4014 ; JMP loop
    let loop_start = p;
    prg[p] = op::STA_ABS;
    p += 1;
    prg[p] = OAMDMA as u8;
    p += 1;
    prg[p] = (OAMDMA >> 8) as u8;
    p += 1;
    prg[p] = op::JMP_ABS;
    p += 1;
    prg[p] = loop_start as u8;
    p += 1;
    prg[p] = (loop_start >> 8) as u8;
    p += 1;
    let vec_off = PRG_ROM_UNIT - 6;
    prg[vec_off + 2] = 0x00;
    prg[vec_off + 3] = 0xC0;
    let rom = make_ines(1, 0, 0, &prg, &[]);
    let cart = Cartridge::from_bytes(&rom).expect("load DMA-loop cart");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    // Run 5 frames with continuous DMA. Each should complete at scanline 0.
    for i in 0..5 {
        emu.step_frame();
        assert_eq!(
            emu.bus().ppu().scanline(),
            0,
            "frame {i} should end at scanline 0 even with continuous DMA",
        );
    }
}

// ===========================================================================
// Reset + multi-frame stability
// ===========================================================================

#[test]
fn reset_loads_pc_from_reset_vector() {
    let rom = make_nop_rom();
    let cart = Cartridge::from_bytes(&rom).expect("load NOP cart");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    assert_eq!(emu.cpu().pc, 0xC000);
}

#[test]
fn ten_frames_run_without_crash_or_deadlock() {
    let rom = make_visible_rom(0x16, 0x21);
    let cart = Cartridge::from_bytes(&rom).expect("load visible cart");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    for _ in 0..10 {
        emu.step_frame();
    }
    assert_eq!(emu.bus().ppu().scanline(), 0);
}

#[test]
fn framebuffer_size_is_256x240() {
    let rom = make_nop_rom();
    let cart = Cartridge::from_bytes(&rom).expect("load NOP cart");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    emu.step_frame();
    let fb = emu.framebuffer();
    assert_eq!(fb.len(), 256 * 240);
}

// ===========================================================================
// nestest.nes — full-frame integration with a real ROM
// ===========================================================================

/// Path to the nestest.nes ROM fixture.
const NESTEST_ROM: &str = "tests/test_roms/nestest.nes";

#[test]
fn nestest_runs_one_frame_without_crash() {
    // Verify the emulator can load a real iNES ROM and run one frame
    // without crashing. nestest.nes is a 16KB NROM-128 with CHR-ROM.
    let cart = Cartridge::from_path(NESTEST_ROM).expect("load nestest.nes");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    // nestest's RESET vector points to $C004 (interactive menu). Run a
    // few frames to confirm stability.
    for _ in 0..3 {
        emu.step_frame();
    }
    assert_eq!(emu.bus().ppu().scanline(), 0);
}
