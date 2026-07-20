//! Integration tests for the PPU register interface (M7).
//!
//! These tests exercise the PPU registers through the CPU memory bus
//! (`$2000-`$2007` + `$4014 OAMDMA) and verify the behaviors documented on
//! the NESdev wiki:
//!
//! - PPUCTRL / PPUMASK / OAMADDR / PPUSCROLL / PPUADDR are write-only
//!   (reads return open bus).
//! - PPUSTATUS read returns VBlank / sprite-0-hit / overflow flags, clears
//!   VBlank, and resets the shared write latch.
//! - OAMDATA read/write hits OAM at OAMADDR and increments OAMADDR.
//! - PPUSCROLL and PPUADDR use a shared two-write latch (`w`).
//! - PPUDATA reads are buffered (one-read delay) except for palette
//!   addresses; writes and reads auto-increment `v` by 1 or 32.
//! - OAMDMA (`$4014`) copies a 256-byte CPU page into OAM.
//!
//! See: https://www.nesdev.org/wiki/PPU_registers

use nes_emu::bus::Bus;
use nes_emu::cartridge::Cartridge;
use nes_emu::mappers::Mirroring;
use nes_emu::ppu::Ppu;

// ---- helpers -----------------------------------------------------------

/// Build a 32 KB NROM cartridge with CHR-RAM and the given mirroring.
fn make_cart(mirroring: Mirroring) -> Cartridge {
    let mut bytes = Vec::with_capacity(16 + 32 * 1024);
    bytes.extend_from_slice(&[b'N', b'E', b'S', 0x1A]);
    bytes.push(2); // 2 × 16 KB PRG
    bytes.push(0); // 0 × 8 KB CHR → CHR-RAM
    let flags6: u8 = match mirroring {
        Mirroring::Vertical => 0b0000_0001,
        Mirroring::Horizontal => 0b0000_0000,
        Mirroring::FourScreen => 0b0000_1000,
        Mirroring::SingleScreen(_) => 0b0000_0000, // not standard for NROM
    };
    bytes.push(flags6);
    bytes.push(0);
    bytes.extend_from_slice(&[0u8; 8]);
    bytes.resize(16 + 32 * 1024, 0);
    Cartridge::from_bytes(&bytes).expect("build test cartridge")
}

/// Build a bus with an NROM cartridge (CHR-RAM) using horizontal mirroring.
fn bus_with_cart() -> Bus {
    Bus::with_cartridge(make_cart(Mirroring::Horizontal))
}

/// Write `hi` then `lo` to PPUADDR ($2006) to set the VRAM address.
fn set_vram_addr(bus: &mut Bus, addr: u16) {
    bus.write(0x2006, ((addr >> 8) & 0x3F) as u8);
    bus.write(0x2006, (addr & 0xFF) as u8);
}

// =====================================================================
//  PPUCTRL ($2000)
// =====================================================================

#[test]
fn ppuctrl_write_only_reads_return_open_bus() {
    let mut bus = bus_with_cart();
    bus.write(0x2000, 0xA5);
    // PPUCTRL is write-only; reading $2000 returns the open-bus latch.
    assert_eq!(bus.read(0x2000), 0xA5);
}

#[test]
fn ppuctrl_nmi_enable_bit_reflected() {
    let mut bus = bus_with_cart();
    bus.write(0x2000, 0x80);
    assert!(bus.ppu().nmi_enabled());
    bus.write(0x2000, 0x00);
    assert!(!bus.ppu().nmi_enabled());
}

#[test]
fn ppuctrl_increment_32_bit_sets_32_step() {
    let mut bus = bus_with_cart();
    bus.write(0x2000, 0x04); // bit 2 = increment by 32
    assert_eq!(bus.ppu().vram_increment(), 32);
    bus.write(0x2000, 0x00);
    assert_eq!(bus.ppu().vram_increment(), 1);
}

#[test]
fn ppuctrl_mirrors_every_8_bytes() {
    let mut bus = bus_with_cart();
    bus.write(0x2008, 0x3C); // $2008 mirrors $2000
    assert_eq!(bus.ppu().ppuctrl(), 0x3C);
}

// =====================================================================
//  PPUMASK ($2001)
// =====================================================================

#[test]
fn ppumask_write_only_reads_return_open_bus() {
    let mut bus = bus_with_cart();
    bus.write(0x2001, 0x1E);
    assert_eq!(bus.read(0x2001), 0x1E);
    assert_eq!(bus.ppu().ppumask(), 0x1E);
}

// =====================================================================
//  PPUSTATUS ($2002)
// =====================================================================

#[test]
fn ppustatus_vblank_set_and_cleared_on_read() {
    let mut bus = bus_with_cart();
    bus.ppu_mut().set_vblank(true);
    let r = bus.read(0x2002);
    assert_eq!(r & 0x80, 0x80, "VBlank bit visible");
    // Reading PPUSTATUS clears VBlank.
    let r2 = bus.read(0x2002);
    assert_eq!(r2 & 0x80, 0x00, "VBlank cleared after first read");
}

#[test]
fn ppustatus_read_resets_write_latch() {
    let mut bus = bus_with_cart();
    // First PPUADDR write sets w = true.
    bus.write(0x2006, 0x20);
    assert!(bus.ppu().write_latch());
    // Reading PPUSTATUS resets w.
    bus.read(0x2002);
    assert!(!bus.ppu().write_latch());
}

#[test]
fn ppustatus_low_bits_from_open_bus() {
    let mut bus = bus_with_cart();
    bus.write(0x2000, 0x1F); // open bus = 0x1F
    bus.ppu_mut().set_vblank(true);
    let r = bus.read(0x2002);
    assert_eq!(r, 0x80 | 0x1F);
}

#[test]
fn ppustatus_sprite_zero_and_overflow_flags() {
    let mut bus = bus_with_cart();
    bus.ppu_mut().set_sprite_zero_hit(true);
    bus.ppu_mut().set_sprite_overflow(true);
    let r = bus.read(0x2002);
    assert_eq!(r & 0x60, 0x60, "sprite 0 hit (bit 6) + overflow (bit 5)");
}

// =====================================================================
//  OAMADDR ($2003) + OAMDATA ($2004)
// =====================================================================

#[test]
fn oamaddr_write_only() {
    let mut bus = bus_with_cart();
    bus.write(0x2003, 0x42);
    assert_eq!(bus.ppu().oamaddr(), 0x42);
    // Reading $2003 returns open bus, not OAMADDR.
    assert_eq!(bus.read(0x2003), 0x42);
}

#[test]
fn oamdata_write_and_read_back() {
    let mut bus = bus_with_cart();
    bus.write(0x2003, 0x10); // OAMADDR = 0x10
    bus.write(0x2004, 0xAA); // OAM[0x10] = 0xAA, OAMADDR → 0x11
    bus.write(0x2003, 0x10); // reset
    assert_eq!(bus.read(0x2004), 0xAA);
}

#[test]
fn oamdata_write_increments_oamaddr() {
    let mut bus = bus_with_cart();
    bus.write(0x2003, 0x00);
    bus.write(0x2004, 0x11);
    bus.write(0x2004, 0x22);
    assert_eq!(bus.ppu().oam()[0], 0x11);
    assert_eq!(bus.ppu().oam()[1], 0x22);
    assert_eq!(bus.ppu().oamaddr(), 2);
}

#[test]
fn oamdata_read_increments_oamaddr() {
    let mut bus = bus_with_cart();
    bus.write(0x2003, 0x00);
    bus.write(0x2004, 0xCC);
    bus.write(0x2003, 0x00);
    let _ = bus.read(0x2004);
    assert_eq!(bus.ppu().oamaddr(), 1);
}

#[test]
fn oamdata_wraps_oamaddr_at_256() {
    let mut bus = bus_with_cart();
    bus.write(0x2003, 0xFF);
    bus.write(0x2004, 0x01);
    assert_eq!(
        bus.ppu().oamaddr(),
        0x00,
        "wraps to 0 after writing at 0xFF"
    );
}

// =====================================================================
//  PPUSCROLL ($2005) — two-write latch
// =====================================================================

#[test]
fn ppuscroll_two_writes_set_fine_and_coarse() {
    let mut bus = bus_with_cart();
    // First write: 0x7D → coarse X = 15, fine X = 5
    bus.write(0x2005, 0x7D);
    assert_eq!(bus.ppu().fine_x(), 5);
    assert!(bus.ppu().write_latch());
    // Second write: 0x1B → coarse Y = 3, fine Y = 3
    bus.write(0x2005, 0x1B);
    assert!(!bus.ppu().write_latch());
    let t = bus.ppu().temp_vram_addr();
    assert_eq!(t & 0x1F, 15, "coarse X = 15");
    assert_eq!((t >> 5) & 0x1F, 3, "coarse Y = 3");
    assert_eq!((t >> 12) & 0x07, 3, "fine Y = 3");
}

#[test]
fn ppuscroll_latch_reset_by_status_read() {
    let mut bus = bus_with_cart();
    bus.write(0x2005, 0xFF); // first write → w = true
    assert!(bus.ppu().write_latch());
    bus.read(0x2002); // resets w
                      // Now the next PPUSCROLL write is treated as a first write again.
    bus.write(0x2005, 0x00);
    assert!(
        bus.ppu().write_latch(),
        "treated as first write after reset"
    );
}

#[test]
fn ppuscroll_second_write_preserves_nametable_select_bits() {
    // Regression: the second PPUSCROLL write must preserve the nametable
    // select bits (t[11:10]) set by a prior PPUADDR write, and must
    // overwrite coarse Y bit 3 (t[8]) even if it was previously set.
    let mut bus = bus_with_cart();
    // PPUADDR hi byte maps to t[13:8]: bit 3 → t[11] (NT select 1),
    // bit 2 → t[10] (NT select 0), bit 0 → t[8] (coarse Y bit 3).
    // hi byte 0x05 = 0b000101 → t[11:10] = 0b01 (NT1), t[8] = 1.
    bus.write(0x2006, 0x05); // t[13:8] = 0b000101, w = true
    bus.read(0x2002); // reset w to false
                      // First PPUSCROLL write: coarse X = 0, fine X = 0.
    bus.write(0x2005, 0x00); // t[4:0] = 0, fine_x = 0, w = true
                             // Second PPUSCROLL write: coarse Y = 0, fine Y = 0.
                             // This must clear t[8] and preserve t[11:10] = 0b01.
    bus.write(0x2005, 0x00); // w = false
    let t = bus.ppu().temp_vram_addr();
    assert_eq!(
        (t >> 10) & 0x03,
        0b01,
        "nametable select bits preserved by second scroll write"
    );
    assert_eq!(
        (t >> 5) & 0x1F,
        0,
        "coarse Y fully overwritten (t[8] cleared)"
    );
    assert_eq!((t >> 12) & 0x07, 0, "fine Y overwritten");
}

#[test]
fn ppuscroll_first_write_preserves_coarse_y_bits() {
    // Regression: the first PPUSCROLL write must preserve bits 6-5 of t
    // (part of coarse Y) set by a prior PPUADDR write.
    let mut bus = bus_with_cart();
    // PPUADDR lo byte maps to t[7:0]. lo byte 0x60 = 0b01100000 →
    // t[6:5] = 0b11.
    bus.write(0x2006, 0x00);
    bus.write(0x2006, 0x60); // t = 0x0060, v = 0x0060, w = false
                             // First PPUSCROLL write: coarse X = 0, fine X = 0.
    bus.write(0x2005, 0x00); // w = true
    let t = bus.ppu().temp_vram_addr();
    // t[6:5] = 0b11 preserved, t[9:7] = 0, so coarse Y = 0b00011 = 3.
    assert_eq!(
        (t >> 5) & 0x1F,
        0b00011,
        "coarse Y bits 6-5 preserved by first scroll write"
    );
}

// =====================================================================
//  PPUADDR ($2006) — two-write latch
// =====================================================================

#[test]
fn ppuaddr_two_writes_set_v() {
    let mut bus = bus_with_cart();
    bus.write(0x2006, 0x21);
    bus.write(0x2006, 0x08);
    assert_eq!(bus.ppu().vram_addr(), 0x2108);
}

#[test]
fn ppuaddr_high_byte_masked_to_14_bits() {
    let mut bus = bus_with_cart();
    bus.write(0x2006, 0xFF); // 0xFF & 0x3F = 0x3F
    bus.write(0x2006, 0x00);
    assert_eq!(bus.ppu().vram_addr(), 0x3F00);
}

#[test]
fn ppuaddr_latch_reset_by_status_read() {
    let mut bus = bus_with_cart();
    bus.write(0x2006, 0x20); // first write → w = true
    bus.read(0x2002); // resets w
    bus.write(0x2006, 0x08); // treated as first write (hi byte)
    assert!(bus.ppu().write_latch());
    // v should NOT be set yet — only the high byte of t was written.
    assert_eq!(
        bus.ppu().vram_addr(),
        0x0000,
        "v not updated on first write"
    );
}

// =====================================================================
//  PPUDATA ($2007) — nametable writes/reads, buffered read, increment
// =====================================================================

#[test]
fn ppudata_write_to_nametable_then_read_back() {
    let mut bus = bus_with_cart();
    // Write $AB to $2000 (nametable 0).
    set_vram_addr(&mut bus, 0x2000);
    bus.write(0x2007, 0xAB);
    // Read back: first read is buffered (returns stale 0), second read
    // returns the actual value.
    set_vram_addr(&mut bus, 0x2000);
    let _first = bus.read(0x2007); // buffered → stale, v advances
    let second = bus.read(0x2007); // now returns $AB
    assert_eq!(second, 0xAB);
}

#[test]
fn ppudata_write_increments_v_by_1() {
    let mut bus = bus_with_cart();
    set_vram_addr(&mut bus, 0x2000);
    bus.write(0x2007, 0x11);
    assert_eq!(bus.ppu().vram_addr(), 0x2001);
}

#[test]
fn ppudata_write_increments_v_by_32_when_ctrl_bit_set() {
    let mut bus = bus_with_cart();
    bus.write(0x2000, 0x04); // increment = 32
    set_vram_addr(&mut bus, 0x2000);
    bus.write(0x2007, 0x11);
    assert_eq!(bus.ppu().vram_addr(), 0x2020);
}

#[test]
fn ppudata_first_read_returns_buffered_garbage() {
    let mut bus = bus_with_cart();
    // Set up a known value at $2000.
    set_vram_addr(&mut bus, 0x2000);
    bus.write(0x2007, 0x42);
    // Read: first read should return the stale buffer (0), not 0x42.
    set_vram_addr(&mut bus, 0x2000);
    let first = bus.read(0x2007);
    assert_eq!(first, 0x00, "first read returns stale buffer");
    let second = bus.read(0x2007);
    assert_eq!(second, 0x42, "second read returns actual data");
}

#[test]
fn ppudata_palette_read_is_immediate() {
    let mut bus = bus_with_cart();
    // Write a palette color at $3F00.
    set_vram_addr(&mut bus, 0x3F00);
    bus.write(0x2007, 0x0F);
    // Read back — palette reads bypass the buffer, so the FIRST read
    // should return the actual value.
    set_vram_addr(&mut bus, 0x3F00);
    let val = bus.read(0x2007);
    assert_eq!(val, 0x0F, "palette read is immediate (no buffer)");
}

#[test]
fn ppudata_palette_write_and_mirror() {
    let mut bus = bus_with_cart();
    set_vram_addr(&mut bus, 0x3F00);
    bus.write(0x2007, 0x21);
    // $3F10 mirrors $3F00.
    set_vram_addr(&mut bus, 0x3F10);
    let val = bus.read(0x2007);
    assert_eq!(val, 0x21, "$3F10 mirrors $3F00");
}

#[test]
fn ppudata_nametable_horizontal_mirroring() {
    let mut bus = bus_with_cart(); // horizontal mirroring
                                   // Write to NT0 ($2000).
    set_vram_addr(&mut bus, 0x2000);
    bus.write(0x2007, 0x55);
    // Read from NT1 ($2400) — should mirror NT0.
    set_vram_addr(&mut bus, 0x2400);
    let _ = bus.read(0x2007); // buffered
    let val = bus.read(0x2007);
    assert_eq!(val, 0x55, "NT1 mirrors NT0 in horizontal mode");
}

#[test]
fn ppudata_nametable_vertical_mirroring() {
    let mut bus = Bus::with_cartridge(make_cart(Mirroring::Vertical));
    set_vram_addr(&mut bus, 0x2000);
    bus.write(0x2007, 0x66);
    // NT2 ($2800) mirrors NT0 in vertical mode.
    set_vram_addr(&mut bus, 0x2800);
    let _ = bus.read(0x2007);
    let val = bus.read(0x2007);
    assert_eq!(val, 0x66, "NT2 mirrors NT0 in vertical mode");
}

#[test]
fn ppudata_chr_write_to_chr_ram() {
    let mut bus = bus_with_cart(); // CHR-RAM cart
    set_vram_addr(&mut bus, 0x0000);
    bus.write(0x2007, 0x77);
    // Read back via CHR (buffered).
    set_vram_addr(&mut bus, 0x0000);
    let _ = bus.read(0x2007);
    let val = bus.read(0x2007);
    assert_eq!(val, 0x77, "CHR-RAM write persists");
}

#[test]
fn ppudata_3000_mirror_of_2000() {
    let mut bus = bus_with_cart();
    set_vram_addr(&mut bus, 0x2000);
    bus.write(0x2007, 0x33);
    // $3000 mirrors $2000.
    set_vram_addr(&mut bus, 0x3000);
    let _ = bus.read(0x2007);
    let val = bus.read(0x2007);
    assert_eq!(val, 0x33);
}

// =====================================================================
//  OAMDMA ($4014)
// =====================================================================

#[test]
fn oam_dma_copies_cpu_page_to_oam() {
    let mut bus = bus_with_cart();
    // Fill CPU RAM page $02 with a recognizable pattern.
    for i in 0..256u16 {
        bus.write(0x0200 + i, (i as u8).wrapping_add(0x40));
    }
    // Trigger OAMDMA from page $02.
    bus.write(0x4014, 0x02);
    // Verify OAM contents.
    for i in 0..256 {
        assert_eq!(bus.ppu().oam()[i], (i as u8).wrapping_add(0x40));
    }
    assert_eq!(bus.ppu().oamaddr(), 0, "OAMADDR reset to 0 after DMA");
}

#[test]
fn oam_dma_from_zero_page() {
    let mut bus = bus_with_cart();
    bus.write(0x0000, 0xDE);
    bus.write(0x0001, 0xAD);
    bus.write(0x0002, 0xBE);
    bus.write(0x0003, 0xEF);
    bus.write(0x4014, 0x00);
    assert_eq!(bus.ppu().oam()[0], 0xDE);
    assert_eq!(bus.ppu().oam()[1], 0xAD);
    assert_eq!(bus.ppu().oam()[2], 0xBE);
    assert_eq!(bus.ppu().oam()[3], 0xEF);
}

#[test]
fn oam_dma_overwrites_full_256_bytes() {
    let mut bus = bus_with_cart();
    // Pre-fill OAM with 0xFF via direct OAMDATA writes.
    bus.write(0x2003, 0x00);
    for _ in 0..256 {
        bus.write(0x2004, 0xFF);
    }
    // Fill CPU page $03 with 0x00.
    for i in 0..256u16 {
        bus.write(0x0300 + i, 0x00);
    }
    bus.write(0x4014, 0x03);
    for i in 0..256 {
        assert_eq!(bus.ppu().oam()[i], 0x00, "OAM[{i}] overwritten by DMA");
    }
}

// =====================================================================
//  Register mirroring within $2000-$3FFF
// =====================================================================

#[test]
fn ppu_register_mirrors_route_correctly() {
    let mut bus = bus_with_cart();
    // $2008 mirrors $2000 (PPUCTRL).
    bus.write(0x2008, 0x11);
    assert_eq!(bus.ppu().ppuctrl(), 0x11);
    // $3FF8 mirrors $2000.
    bus.write(0x3FF8, 0x22);
    assert_eq!(bus.ppu().ppuctrl(), 0x22);
    // $2014 mirrors $2004 (OAMDATA).
    bus.write(0x2003, 0x20);
    bus.write(0x2014, 0x99); // mirror of $2004
    bus.write(0x2003, 0x20);
    assert_eq!(bus.read(0x2004), 0x99);
}

// =====================================================================
//  VBlank NMI enable
// =====================================================================

#[test]
fn vblank_flag_set_and_nmi_enable_independent() {
    let mut bus = bus_with_cart();
    // VBlank can be set regardless of NMI enable.
    bus.write(0x2000, 0x00); // NMI disabled
    bus.ppu_mut().set_vblank(true);
    assert!(bus.ppu().in_vblank());
    assert!(!bus.ppu().nmi_enabled());
    // Enable NMI — VBlank still set.
    bus.write(0x2000, 0x80);
    assert!(bus.ppu().in_vblank());
    assert!(bus.ppu().nmi_enabled());
}

// =====================================================================
//  Bus exposes PPU
// =====================================================================

#[test]
fn bus_exposes_ppu_borrows() {
    let mut bus = bus_with_cart();
    bus.ppu_mut().set_vblank(true);
    assert!(bus.ppu().in_vblank());
    // Read-only borrow should work.
    let _ctrl = bus.ppu().ppuctrl();
}

#[test]
fn bus_sets_ppu_mirroring_from_cartridge() {
    let bus = Bus::with_cartridge(make_cart(Mirroring::Vertical));
    assert_eq!(bus.ppu().mirroring(), Mirroring::Vertical);

    let bus = Bus::with_cartridge(make_cart(Mirroring::Horizontal));
    assert_eq!(bus.ppu().mirroring(), Mirroring::Horizontal);
}

// =====================================================================
//  Ppu::new() defaults
// =====================================================================

#[test]
fn ppu_new_defaults() {
    let ppu = Ppu::new();
    assert_eq!(ppu.ppuctrl(), 0);
    assert_eq!(ppu.ppumask(), 0);
    assert_eq!(ppu.ppustatus(), 0);
    assert_eq!(ppu.vram_addr(), 0);
    assert!(!ppu.write_latch());
    assert!(!ppu.in_vblank());
    assert_eq!(ppu.mirroring(), Mirroring::Horizontal);
}
