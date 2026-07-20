//! Integration tests for cartridge mappers — MMC1 (mapper 1).
//!
//! These tests build minimal iNES ROM images in memory and exercise the
//! mapper through the `Cartridge` and `Bus` APIs, verifying:
//!
//! - MMC1 is selected for iNES mapper number 1.
//! - The 5-bit serial shift register loads registers correctly via bus writes.
//! - PRG bank switching works in all 3 modes (32 KB, fix-first, fix-last).
//! - CHR bank switching works in 8 KB and 4 KB modes.
//! - PRG-RAM at `$6000-$7FFF` is readable and writable through the bus.
//! - Mirroring changes via register writes propagate to the PPU.
//! - CHR-RAM writes persist through the bus.

use nes_emu::bus::Bus;
use nes_emu::cartridge::{Cartridge, PRG_ROM_UNIT};
use nes_emu::mappers::Mirroring;

/// iNES header magic.
const INES_MAGIC: [u8; 4] = [b'N', b'E', b'S', 0x1A];
const HEADER_SIZE: usize = 16;

/// Build an iNES image with the given mapper number (low nibble in flags 6),
/// PRG/CHR bank counts, and fill bytes for PRG and CHR.
///
/// PRG is filled so each 16 KB bank has a unique byte (bank index), and CHR
/// is filled so each 4 KB bank has a unique byte (bank index).
fn make_ines_mmc1(prg_banks: u8, chr_banks: u8, flags6: u8) -> Vec<u8> {
    let prg_size = prg_banks as usize * PRG_ROM_UNIT;
    let chr_size = chr_banks as usize * 8 * 1024;
    let mut buf = Vec::with_capacity(HEADER_SIZE + prg_size + chr_size);
    buf.extend_from_slice(&INES_MAGIC);
    buf.push(prg_banks);
    buf.push(chr_banks);
    buf.push(flags6);
    buf.push(0); // flags7 — mapper high nibble = 0 (mapper 1 = 0x01 in flags6 high nibble)
    buf.extend_from_slice(&[0u8; 8]);
    buf.resize(HEADER_SIZE + prg_size + chr_size, 0);
    // Fill PRG: each 16 KB bank with its bank index.
    for i in 0..prg_size {
        buf[HEADER_SIZE + i] = (i / PRG_ROM_UNIT) as u8;
    }
    // Fill CHR: each 4 KB bank with its bank index.
    for i in 0..chr_size {
        buf[HEADER_SIZE + prg_size + i] = (i / (4 * 1024)) as u8;
    }
    buf
}

/// Serially write a 5-bit value to an MMC1 register at `addr` via the bus.
/// Each write shifts bit 0 (LSB first) into the shift register.
fn write_reg_via_bus(bus: &mut Bus, addr: u16, value: u8) {
    for i in 0..5 {
        bus.write(addr, (value >> i) & 1);
    }
}

/// Map a nametable address (`$2000-$2FFF`) to a physical VRAM index using
/// the PPU's current mirroring mode. Mirrors the logic in `Ppu::map_nametable`.
fn nt_phys_addr(mirroring: Mirroring, addr: u16) -> usize {
    let addr = addr & 0x2FFF;
    let local = (addr - 0x2000) as usize;
    let nt = local >> 10;
    let offset = local & 0x3FF;
    let phys = match mirroring {
        Mirroring::Horizontal => nt >> 1,
        Mirroring::Vertical => nt & 1,
        Mirroring::FourScreen => nt,
        Mirroring::SingleScreen(n) => (n & 3) as usize,
    };
    phys * 0x400 + offset
}

/// Write a byte to PPU VRAM at `addr` via the $2006/$2007 register pair.
fn ppu_vram_write(bus: &mut Bus, addr: u16, value: u8) {
    bus.write(0x2006, ((addr >> 8) & 0xFF) as u8);
    bus.write(0x2006, (addr & 0xFF) as u8);
    bus.write(0x2007, value);
}

// ---- Cartridge / mapper selection -------------------------------------

#[test]
fn mmc1_selected_for_mapper_1() {
    // flags6 high nibble = 1 → mapper 1. Horizontal mirroring.
    let bytes = make_ines_mmc1(4, 2, 0b0001_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load MMC1 cart");
    assert_eq!(cart.header.mapper_number, 1);
    // MMC1 should report horizontal mirroring from the header initially.
    assert_eq!(cart.mirror_mode(), Mirroring::Horizontal);
}

#[test]
fn mmc1_battery_flag_propagates() {
    // flags6 = 0b0001_0010 → mapper 1, battery flag set.
    let bytes = make_ines_mmc1(4, 0, 0b0001_0010);
    let cart = Cartridge::from_bytes(&bytes).expect("load MMC1 cart");
    assert!(cart.has_battery());
}

// ---- PRG banking via bus ----------------------------------------------

#[test]
fn mmc1_prg_mode_3_fixes_last_bank_via_bus() {
    // 4 × 16 KB PRG banks. Default mode 3: fix last at $C000, switch $8000.
    let bytes = make_ines_mmc1(4, 0, 0b0001_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    // Without any bank write, default prg_bank=0 → bank 0 at $8000.
    assert_eq!(bus.read(0x8000), 0x00);
    assert_eq!(bus.read(0xBFFF), 0x00);
    // $C000 = fixed last bank (bank 3).
    assert_eq!(bus.read(0xC000), 0x03);
    assert_eq!(bus.read(0xFFFF), 0x03);
}

#[test]
fn mmc1_prg_bank_switch_via_bus() {
    let bytes = make_ines_mmc1(4, 0, 0b0001_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    // Set prg_bank = 2 via $E000 register (mode 3 default).
    write_reg_via_bus(&mut bus, 0xE000, 0x02);
    // $8000 = bank 2, $C000 = fixed last bank (3).
    assert_eq!(bus.read(0x8000), 0x02);
    assert_eq!(bus.read(0xBFFF), 0x02);
    assert_eq!(bus.read(0xC000), 0x03);
    assert_eq!(bus.read(0xFFFF), 0x03);
}

#[test]
fn mmc1_prg_mode_2_fixes_first_bank_via_bus() {
    let bytes = make_ines_mmc1(4, 0, 0b0001_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    // Set PRG mode 2: control = 0b0_10_11 = 0x0B (horizontal mirroring).
    write_reg_via_bus(&mut bus, 0x8000, 0x0B);
    // Set switchable bank = 1 at $C000.
    write_reg_via_bus(&mut bus, 0xE000, 0x01);
    // $8000 = fixed bank 0, $C000 = bank 1.
    assert_eq!(bus.read(0x8000), 0x00);
    assert_eq!(bus.read(0xBFFF), 0x00);
    assert_eq!(bus.read(0xC000), 0x01);
    assert_eq!(bus.read(0xFFFF), 0x01);
}

#[test]
fn mmc1_prg_mode_0_32kb_swap_via_bus() {
    let bytes = make_ines_mmc1(4, 0, 0b0001_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    // Set PRG mode 0: control = 0b0_00_11 = 0x03.
    write_reg_via_bus(&mut bus, 0x8000, 0x03);
    // prg_bank = 2 → 32 KB window = banks 2&3.
    write_reg_via_bus(&mut bus, 0xE000, 0x02);
    assert_eq!(bus.read(0x8000), 0x02);
    assert_eq!(bus.read(0xBFFF), 0x02);
    assert_eq!(bus.read(0xC000), 0x03);
    assert_eq!(bus.read(0xFFFF), 0x03);
}

// ---- PRG-RAM via bus --------------------------------------------------

#[test]
fn mmc1_prg_ram_read_write_via_bus() {
    let bytes = make_ines_mmc1(4, 0, 0b0001_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    bus.write(0x6000, 0x42);
    bus.write(0x7FFF, 0x99);
    assert_eq!(bus.read(0x6000), 0x42);
    assert_eq!(bus.read(0x7FFF), 0x99);
}

#[test]
fn mmc1_prg_ram_write_does_not_shift_serial_register() {
    let bytes = make_ines_mmc1(4, 0, 0b0001_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    // Write to $6000 (PRG-RAM) — should NOT affect the shift register.
    bus.write(0x6000, 0x01);
    // Now write 5 bits to $E000 — should still commit correctly.
    write_reg_via_bus(&mut bus, 0xE000, 0x01);
    // prg_bank = 1 → bank 1 at $8000 (mode 3).
    assert_eq!(bus.read(0x8000), 0x01);
}

// ---- CHR banking via bus ----------------------------------------------

#[test]
fn mmc1_chr_8kb_mode_via_bus() {
    // 2 × 8 KB CHR = 4 × 4 KB banks.
    let bytes = make_ines_mmc1(4, 2, 0b0001_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    // Default CHR mode 0 (8 KB). Set chr_bank_0 = 2 → window = banks 2&3.
    write_reg_via_bus(&mut bus, 0xA000, 0x02);
    let cart = bus.cartridge().expect("cart");
    assert_eq!(cart.read_chr(0x0000), 0x02);
    assert_eq!(cart.read_chr(0x0FFF), 0x02);
    assert_eq!(cart.read_chr(0x1000), 0x03);
    assert_eq!(cart.read_chr(0x1FFF), 0x03);
}

#[test]
fn mmc1_chr_4kb_mode_via_bus() {
    let bytes = make_ines_mmc1(4, 2, 0b0001_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    // Set CHR mode 1 (4 KB): control = 0b1_11_11 = 0x1F.
    write_reg_via_bus(&mut bus, 0x8000, 0x1F);
    // chr_bank_0 = 1 → $0000-$0FFF = bank 1.
    write_reg_via_bus(&mut bus, 0xA000, 0x01);
    // chr_bank_1 = 3 → $1000-$1FFF = bank 3.
    write_reg_via_bus(&mut bus, 0xC000, 0x03);
    let cart = bus.cartridge().expect("cart");
    assert_eq!(cart.read_chr(0x0000), 0x01);
    assert_eq!(cart.read_chr(0x0FFF), 0x01);
    assert_eq!(cart.read_chr(0x1000), 0x03);
    assert_eq!(cart.read_chr(0x1FFF), 0x03);
}

#[test]
fn mmc1_chr_ram_writes_persist_via_bus() {
    // chr_rom_banks = 0 → CHR-RAM.
    let bytes = make_ines_mmc1(4, 0, 0b0001_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    // Enable 4 KB mode and select different banks for each half.
    write_reg_via_bus(&mut bus, 0x8000, 0x1F);
    write_reg_via_bus(&mut bus, 0xA000, 0x00);
    write_reg_via_bus(&mut bus, 0xC000, 0x01);
    {
        let cart = bus.cartridge_mut().expect("cart");
        cart.write_chr(0x0000, 0xAA);
        cart.write_chr(0x1000, 0xBB);
    }
    let cart = bus.cartridge().expect("cart");
    assert_eq!(cart.read_chr(0x0000), 0xAA);
    assert_eq!(cart.read_chr(0x1000), 0xBB);
}

// ---- Mirroring propagation to PPU -------------------------------------

#[test]
fn mmc1_mirroring_change_propagates_to_ppu() {
    // Start with horizontal mirroring (header).
    let bytes = make_ines_mmc1(4, 0, 0b0001_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    // Initial mirroring from header = horizontal.
    assert_eq!(bus.ppu().mirroring(), Mirroring::Horizontal);

    // Change to vertical via control register: control = 0b0_11_10 = 0x0E.
    write_reg_via_bus(&mut bus, 0x8000, 0x0E);
    assert_eq!(bus.ppu().mirroring(), Mirroring::Vertical);

    // Change to single-screen A: control = 0b0_11_00 = 0x0C.
    write_reg_via_bus(&mut bus, 0x8000, 0x0C);
    assert_eq!(bus.ppu().mirroring(), Mirroring::SingleScreen(0));

    // Change to single-screen B: control = 0b0_11_01 = 0x0D.
    write_reg_via_bus(&mut bus, 0x8000, 0x0D);
    assert_eq!(bus.ppu().mirroring(), Mirroring::SingleScreen(1));
}

#[test]
fn mmc1_mirroring_affects_ppu_nametable_mapping() {
    let bytes = make_ines_mmc1(4, 0, 0b0001_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);

    // Write distinct values to the two physical nametable pages via $2006/$2007.
    // Under horizontal mirroring: NT0 ($2000) and NT1 ($2400) → page 0;
    // NT2 ($2800) and NT3 ($2C00) → page 1.
    ppu_vram_write(&mut bus, 0x2000, 0xAA); // page 0
    ppu_vram_write(&mut bus, 0x2800, 0xBB); // page 1

    let vram = bus.ppu().vram();
    let m = bus.ppu().mirroring();
    // Under horizontal: $2000 and $2400 both → page 0 (0xAA).
    assert_eq!(vram[nt_phys_addr(m, 0x2000)], 0xAA);
    assert_eq!(vram[nt_phys_addr(m, 0x2400)], 0xAA);
    assert_eq!(vram[nt_phys_addr(m, 0x2800)], 0xBB);
    assert_eq!(vram[nt_phys_addr(m, 0x2C00)], 0xBB);

    // Switch to vertical mirroring via MMC1 control register.
    write_reg_via_bus(&mut bus, 0x8000, 0x0E);
    assert_eq!(bus.ppu().mirroring(), Mirroring::Vertical);
    let vram = bus.ppu().vram();
    let m = bus.ppu().mirroring();
    // Under vertical: $2000 and $2800 both → page 0 (0xAA).
    assert_eq!(vram[nt_phys_addr(m, 0x2000)], 0xAA);
    assert_eq!(vram[nt_phys_addr(m, 0x2800)], 0xAA);
    assert_eq!(vram[nt_phys_addr(m, 0x2400)], 0xBB);
    assert_eq!(vram[nt_phys_addr(m, 0x2C00)], 0xBB);
}

// ---- Shift register reset via bus -------------------------------------

#[test]
fn mmc1_bit7_reset_via_bus() {
    let bytes = make_ines_mmc1(4, 0, 0b0001_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);

    // Switch to PRG mode 0: control = 0x03.
    write_reg_via_bus(&mut bus, 0x8000, 0x03);
    // Write 3 bits to $E000 (incomplete shift).
    bus.write(0xE000, 1);
    bus.write(0xE000, 1);
    bus.write(0xE000, 1);
    // Reset via bit 7 — should clear shift register and force PRG mode 3.
    bus.write(0xE000, 0x80);
    // prg_bank should still be 0 (shift was reset), and mode = 3.
    // $8000 = bank 0, $C000 = last bank (3).
    assert_eq!(bus.read(0x8000), 0x00);
    assert_eq!(bus.read(0xC000), 0x03);
}

// ---- Emulator integration ---------------------------------------------

#[test]
fn mmc1_cart_loads_in_emulator() {
    use nes_emu::emulator::EmulatorState;
    let bytes = make_ines_mmc1(4, 0, 0b0001_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let emu = EmulatorState::new(cart);
    // The emulator should have the cart loaded and not crash on construction.
    assert_eq!(emu.bus().cartridge().unwrap().header.mapper_number, 1);
}

// =====================================================================
// MMC3 (mapper 4) integration tests
// =====================================================================

/// Build an iNES image for MMC3 (mapper 4). PRG is filled so each 8 KB
/// bank has a unique byte (bank index); CHR is filled so each 1 KB bank
/// has a unique byte (bank index).
fn make_ines_mmc3(prg_8k_banks: u8, chr_1k_banks: u8, flags6: u8) -> Vec<u8> {
    let prg_size = prg_8k_banks as usize * 8 * 1024;
    let chr_size = chr_1k_banks as usize * 1024;
    let mut buf = Vec::with_capacity(HEADER_SIZE + prg_size + chr_size);
    buf.extend_from_slice(&INES_MAGIC);
    // PRG banks in 16KB units: prg_8k_banks / 2 (round up).
    buf.push((prg_8k_banks + 1) / 2);
    // CHR banks in 8KB units: chr_1k_banks / 8 (round up).
    buf.push((chr_1k_banks + 7) / 8);
    buf.push(flags6); // mapper low nibble in high nibble
    buf.push(0); // flags7 — mapper high nibble = 0
    buf.extend_from_slice(&[0u8; 8]);
    buf.resize(HEADER_SIZE + prg_size + chr_size, 0);
    // Fill PRG: each 8 KB bank with its bank index.
    for i in 0..prg_size {
        buf[HEADER_SIZE + i] = (i / (8 * 1024)) as u8;
    }
    // Fill CHR: each 1 KB bank with its bank index.
    for i in 0..chr_size {
        buf[HEADER_SIZE + prg_size + i] = (i / 1024) as u8;
    }
    buf
}

/// Write to an MMC3 bank register via the bus. `bank_select` is the full
/// value written to `$8000` (mode bits in 6-7, register index in 0-2);
/// `value` is written to `$8001`. Callers that need specific PRG/CHR mode
/// bits should OR them into `bank_select`.
fn mmc3_write_bank_full(bus: &mut Bus, bank_select: u8, value: u8) {
    bus.write(0x8000, bank_select);
    bus.write(0x8001, value);
}

/// Convenience wrapper for `mmc3_write_bank_full` that selects register
/// `reg` (0-7) with mode bits = 0 (PRG mode 0, CHR mode 0). Tests that
/// need mode 1 should use `mmc3_write_bank_full` directly.
fn mmc3_write_bank(bus: &mut Bus, reg: u8, value: u8) {
    mmc3_write_bank_full(bus, reg & 0x07, value);
}

// ---- Mapper selection --------------------------------------------------

#[test]
fn mmc3_selected_for_mapper_4() {
    // flags6 high nibble = 4 → mapper 4. Horizontal mirroring.
    let bytes = make_ines_mmc3(4, 8, 0b0100_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load MMC3 cart");
    assert_eq!(cart.header.mapper_number, 4);
    assert_eq!(cart.mirror_mode(), Mirroring::Horizontal);
}

#[test]
fn mmc3_battery_flag_propagates() {
    // flags6 = 0b0100_0010 → mapper 4, battery flag.
    let bytes = make_ines_mmc3(4, 0, 0b0100_0010);
    let cart = Cartridge::from_bytes(&bytes).expect("load MMC3 cart");
    assert!(cart.has_battery());
}

// ---- PRG banking via bus -----------------------------------------------

#[test]
fn mmc3_prg_mode_0_default_via_bus() {
    // 4 × 8KB PRG (32 KB). Default mode 0: R6@$8000, R7@$A000, fixed@$C000.
    let bytes = make_ines_mmc3(4, 8, 0b0100_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    // Default R6=R7=0 → bank 0 at $8000 and $A000.
    assert_eq!(bus.read(0x8000), 0x00);
    assert_eq!(bus.read(0xA000), 0x00);
    // Fixed last 16KB at $C000: banks 2 and 3.
    assert_eq!(bus.read(0xC000), 0x02);
    assert_eq!(bus.read(0xE000), 0x03);
}

#[test]
fn mmc3_prg_bank_switch_mode_0_via_bus() {
    let bytes = make_ines_mmc3(4, 8, 0b0100_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    // R6 = 1 → bank 1 at $8000. R7 = 2 → bank 2 at $A000.
    mmc3_write_bank(&mut bus, 6, 0x01);
    mmc3_write_bank(&mut bus, 7, 0x02);
    assert_eq!(bus.read(0x8000), 0x01);
    assert_eq!(bus.read(0x9FFF), 0x01);
    assert_eq!(bus.read(0xA000), 0x02);
    assert_eq!(bus.read(0xBFFF), 0x02);
    // Fixed last 16KB unchanged.
    assert_eq!(bus.read(0xC000), 0x02);
    assert_eq!(bus.read(0xE000), 0x03);
}

#[test]
fn mmc3_prg_mode_1_via_bus() {
    let bytes = make_ines_mmc3(4, 8, 0b0100_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    // PRG mode 1: bit 6 of $8000 = 1. R6 = 1 → bank 1 at $C000.
    mmc3_write_bank_full(&mut bus, 0b0100_0000 | 6, 0x01);
    // R7 = 2 → bank 2 at $A000 (preserve PRG mode 1).
    mmc3_write_bank_full(&mut bus, 0b0100_0000 | 7, 0x02);
    // $8000 = fixed 2nd-last (bank 2).
    assert_eq!(bus.read(0x8000), 0x02);
    // $A000 = R7 = bank 2.
    assert_eq!(bus.read(0xA000), 0x02);
    // $C000 = R6 = bank 1.
    assert_eq!(bus.read(0xC000), 0x01);
    // $E000 = fixed last (bank 3).
    assert_eq!(bus.read(0xE000), 0x03);
}

// ---- PRG-RAM via bus ---------------------------------------------------

#[test]
fn mmc3_prg_ram_enabled_via_bus() {
    let bytes = make_ines_mmc3(4, 0, 0b0100_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    // PRG-RAM disabled by default → writes ignored, reads return 0.
    bus.write(0x6000, 0x42);
    assert_eq!(bus.read(0x6000), 0x00);
    // Enable PRG-RAM via $A001 (bit 7 = 1).
    bus.write(0xA001, 0x80);
    bus.write(0x6000, 0x42);
    bus.write(0x7FFF, 0x99);
    assert_eq!(bus.read(0x6000), 0x42);
    assert_eq!(bus.read(0x7FFF), 0x99);
}

#[test]
fn mmc3_prg_ram_read_returns_zero_before_enable_via_bus() {
    // Bus-level assertion that $6000 reads 0 before $A001 enable (the
    // mapper returns 0 for disabled PRG-RAM, not open-bus filler).
    let bytes = make_ines_mmc3(4, 0, 0b0100_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    assert_eq!(bus.read(0x6000), 0x00);
    assert_eq!(bus.read(0x7FFF), 0x00);
}

#[test]
fn mmc3_prg_ram_write_protect_via_bus() {
    let bytes = make_ines_mmc3(4, 0, 0b0100_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    // Enable + write protect (bits 7 and 6).
    bus.write(0xA001, 0xC0);
    bus.write(0x6000, 0x42);
    assert_eq!(bus.read(0x6000), 0x00); // write blocked
}

// ---- CHR banking via bus -----------------------------------------------

#[test]
fn mmc3_chr_mode_0_via_bus() {
    // 8 × 1KB CHR banks.
    let bytes = make_ines_mmc3(4, 8, 0b0100_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    // R0 = 2 → 2KB at $0000 (1KB banks 2&3).
    mmc3_write_bank(&mut bus, 0, 0x02);
    // R2 = 6 → 1KB at $1000.
    mmc3_write_bank(&mut bus, 2, 0x06);
    let cart = bus.cartridge().expect("cart");
    assert_eq!(cart.read_chr(0x0000), 0x02);
    assert_eq!(cart.read_chr(0x0400), 0x03);
    assert_eq!(cart.read_chr(0x1000), 0x06);
}

#[test]
fn mmc3_chr_mode_1_via_bus() {
    let bytes = make_ines_mmc3(4, 8, 0b0100_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    // CHR mode 1: bit 7 of $8000 = 1. R0 = 2 → 2KB at $1000 (1KB banks 2&3).
    mmc3_write_bank_full(&mut bus, 0b1000_0000, 0x02);
    // R2 = 6 → 1KB at $0000 (preserve CHR mode 1).
    mmc3_write_bank_full(&mut bus, 0b1000_0000 | 2, 0x06);
    let cart = bus.cartridge().expect("cart");
    assert_eq!(cart.read_chr(0x0000), 0x06);
    assert_eq!(cart.read_chr(0x1000), 0x02);
    assert_eq!(cart.read_chr(0x1400), 0x03);
}

#[test]
fn mmc3_chr_ram_writes_persist_via_bus() {
    // chr_1k_banks = 0 → CHR-RAM (8 KB = 8 × 1KB banks).
    let bytes = make_ines_mmc3(4, 0, 0b0100_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    // With default bank registers (all 0), $0000 and $1000 both map to
    // CHR bank 0 (aliasing). Set R1 = 2 so $0800 maps to banks 2&3,
    // distinct from $0000 → banks 0&1.
    mmc3_write_bank(&mut bus, 1, 0x02);
    {
        let cart = bus.cartridge_mut().expect("cart");
        cart.write_chr(0x0000, 0xAA);
        cart.write_chr(0x0800, 0xBB);
    }
    let cart = bus.cartridge().expect("cart");
    assert_eq!(cart.read_chr(0x0000), 0xAA);
    assert_eq!(cart.read_chr(0x0800), 0xBB);
}

// ---- Mirroring via bus -------------------------------------------------

#[test]
fn mmc3_mirroring_change_propagates_to_ppu() {
    // Start with horizontal mirroring (header).
    let bytes = make_ines_mmc3(4, 0, 0b0100_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    assert_eq!(bus.ppu().mirroring(), Mirroring::Horizontal);
    // Change to vertical via $A000 (bit 0 = 0).
    bus.write(0xA000, 0x00);
    assert_eq!(bus.ppu().mirroring(), Mirroring::Vertical);
    // Change back to horizontal (bit 0 = 1).
    bus.write(0xA000, 0x01);
    assert_eq!(bus.ppu().mirroring(), Mirroring::Horizontal);
}

// ---- IRQ counter via PPU step ------------------------------------------

#[test]
fn mmc3_irq_fires_after_correct_scanline_count() {
    use nes_emu::ppu::CYCLES_PER_SCANLINE;
    let bytes = make_ines_mmc3(4, 8, 0b0100_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);

    // Enable PPU rendering so A12 rises (MMC3 IRQ clocking is gated).
    bus.write(0x2001, 0x08); // PPUMASK show background

    // Set IRQ latch = 5, reload, and enable IRQ.
    bus.write(0xC000, 0x05); // IRQ latch = 5
    bus.write(0xC001, 0x00); // force reload
    bus.write(0xE001, 0x00); // enable IRQ

    // The IRQ counter reloads to 5 on the first clock, then decrements
    // once per scanline. It fires when it wraps past 0:
    //   clock 1: reload → 5
    //   clock 2: 5→4, clock 3: 4→3, clock 4: 3→2, clock 5: 2→1,
    //   clock 6: 1→0, clock 7: 0→reload + IRQ
    // So the IRQ fires after 7 scanlines (6 decrements + 1 wrap clock).
    // Step 7 scanlines worth of PPU cycles (7 × 341 = 2387).
    assert!(!bus.cart_irq_pending());
    for _ in 0..7 {
        bus.step_ppu(CYCLES_PER_SCANLINE as u32);
    }
    assert!(bus.cart_irq_pending(), "IRQ should fire after 7 scanlines");
}

#[test]
fn mmc3_irq_disabled_does_not_fire() {
    use nes_emu::ppu::CYCLES_PER_SCANLINE;
    let bytes = make_ines_mmc3(4, 8, 0b0100_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);

    bus.write(0x2001, 0x08); // enable rendering
    bus.write(0xC000, 0x02);
    bus.write(0xC001, 0x00); // reload
                             // IRQ NOT enabled (default disabled).

    for _ in 0..10 {
        bus.step_ppu(CYCLES_PER_SCANLINE as u32);
    }
    assert!(!bus.cart_irq_pending());
}

#[test]
fn mmc3_irq_cleared_by_e000_write() {
    use nes_emu::ppu::CYCLES_PER_SCANLINE;
    let bytes = make_ines_mmc3(4, 8, 0b0100_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);

    bus.write(0x2001, 0x08);
    bus.write(0xC000, 0x01);
    bus.write(0xC001, 0x00);
    bus.write(0xE001, 0x00); // enable

    // Fire the IRQ (latch=1 → 3 clocks: reload, 1→0, 0→IRQ).
    for _ in 0..3 {
        bus.step_ppu(CYCLES_PER_SCANLINE as u32);
    }
    assert!(bus.cart_irq_pending());
    // Clear via $E000.
    bus.write(0xE000, 0x00);
    assert!(!bus.cart_irq_pending());
}

#[test]
fn mmc3_irq_not_clocked_when_rendering_disabled() {
    use nes_emu::ppu::CYCLES_PER_SCANLINE;
    let bytes = make_ines_mmc3(4, 8, 0b0100_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);

    // Rendering OFF (PPUMASK = 0) — A12 should not rise, so the IRQ
    // counter is not clocked.
    bus.write(0xC000, 0x01);
    bus.write(0xC001, 0x00);
    bus.write(0xE001, 0x00);

    for _ in 0..20 {
        bus.step_ppu(CYCLES_PER_SCANLINE as u32);
    }
    assert!(!bus.cart_irq_pending());
}

// ---- Emulator integration ----------------------------------------------

#[test]
fn mmc3_cart_loads_in_emulator() {
    use nes_emu::emulator::EmulatorState;
    let bytes = make_ines_mmc3(4, 0, 0b0100_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let emu = EmulatorState::new(cart);
    assert_eq!(emu.bus().cartridge().unwrap().header.mapper_number, 4);
}

#[test]
fn mmc3_emulator_runs_frames_without_crash() {
    use nes_emu::emulator::EmulatorState;
    // Build a ROM with RESET vector pointing to $C000 (NOP loop).
    let mut bytes = make_ines_mmc3(4, 0, 0b0100_0000);
    // Fill PRG with NOP (0xEA) first.
    for i in 0..(4 * 8 * 1024) {
        bytes[HEADER_SIZE + i] = 0xEA;
    }
    // Set the RESET vector once. $FFFC in CPU space maps to the fixed last
    // 16KB bank at $C000 = bank 3 (offset 3*8192 in PRG). $FFFC = offset
    // 0x3FFC in the $C000 window = bank 3 offset 0x1FFC.
    let fixed_off = HEADER_SIZE + 3 * 8 * 1024 + 0x1FFC;
    bytes[fixed_off] = 0x00;
    bytes[fixed_off + 1] = 0xC0;

    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut emu = EmulatorState::new(cart);
    emu.reset();
    assert_eq!(emu.cpu().pc, 0xC000);
    for _ in 0..3 {
        emu.step_frame();
    }
    assert_eq!(emu.bus().ppu().scanline(), 0);
}

// =====================================================================
// UxROM (mapper 2) integration tests
// =====================================================================

/// Build an iNES image for UxROM (mapper 2). PRG is filled so each 16 KB
/// bank has a unique byte (bank index). CHR is filled so each 8 KB bank
/// has a unique byte (bank index).
fn make_ines_uxrom(prg_16k_banks: u8, chr_8k_banks: u8, flags6: u8) -> Vec<u8> {
    let prg_size = prg_16k_banks as usize * PRG_ROM_UNIT;
    let chr_size = chr_8k_banks as usize * 8 * 1024;
    let mut buf = Vec::with_capacity(HEADER_SIZE + prg_size + chr_size);
    buf.extend_from_slice(&INES_MAGIC);
    buf.push(prg_16k_banks);
    buf.push(chr_8k_banks);
    buf.push(flags6); // mapper low nibble in high nibble (2 → 0b0010_0000)
    buf.push(0); // flags7 — mapper high nibble = 0
    buf.extend_from_slice(&[0u8; 8]);
    buf.resize(HEADER_SIZE + prg_size + chr_size, 0);
    // Fill PRG: each 16 KB bank with its bank index.
    for i in 0..prg_size {
        buf[HEADER_SIZE + i] = (i / PRG_ROM_UNIT) as u8;
    }
    // Fill CHR: each 8 KB bank with its bank index.
    for i in 0..chr_size {
        buf[HEADER_SIZE + prg_size + i] = (i / (8 * 1024)) as u8;
    }
    buf
}

#[test]
fn uxrom_selected_for_mapper_2() {
    // flags6 high nibble = 2 → mapper 2. Horizontal mirroring.
    let bytes = make_ines_uxrom(4, 1, 0b0010_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load UxROM cart");
    assert_eq!(cart.header.mapper_number, 2);
    assert_eq!(cart.mirror_mode(), Mirroring::Horizontal);
}

#[test]
fn uxrom_default_bank_0_with_fixed_last_bank_via_bus() {
    // 4 × 16 KB PRG banks. Default: bank 0 at $8000, fixed last (3) at $C000.
    let bytes = make_ines_uxrom(4, 0, 0b0010_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    assert_eq!(bus.read(0x8000), 0x00);
    assert_eq!(bus.read(0xBFFF), 0x00);
    assert_eq!(bus.read(0xC000), 0x03);
    assert_eq!(bus.read(0xFFFF), 0x03);
}

#[test]
fn uxrom_bank_switch_via_bus() {
    let bytes = make_ines_uxrom(4, 0, 0b0010_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    // Select bank 2.
    bus.write(0x8000, 0x02);
    assert_eq!(bus.read(0x8000), 0x02);
    assert_eq!(bus.read(0xBFFF), 0x02);
    // $C000 stays fixed at last bank (3).
    assert_eq!(bus.read(0xC000), 0x03);
    assert_eq!(bus.read(0xFFFF), 0x03);
}

#[test]
fn uxrom_chr_ram_writes_persist_via_bus() {
    // chr_8k_banks = 0 → CHR-RAM.
    let bytes = make_ines_uxrom(4, 0, 0b0010_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    {
        let cart = bus.cartridge_mut().expect("cart");
        cart.write_chr(0x0000, 0xAA);
        cart.write_chr(0x1FFF, 0xBB);
    }
    let cart = bus.cartridge().expect("cart");
    assert_eq!(cart.read_chr(0x0000), 0xAA);
    assert_eq!(cart.read_chr(0x1FFF), 0xBB);
}

#[test]
fn uxrom_mirroring_fixed_from_header_via_bus() {
    // Vertical mirroring from header (flags6 bit 0 = 1).
    let bytes = make_ines_uxrom(4, 0, 0b0010_0001);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    assert_eq!(bus.ppu().mirroring(), Mirroring::Vertical);
    // Bank writes must not change mirroring.
    bus.write(0x8000, 0x02);
    assert_eq!(bus.ppu().mirroring(), Mirroring::Vertical);
}

#[test]
fn uxrom_prg_ram_region_ignored_via_bus() {
    let bytes = make_ines_uxrom(4, 0, 0b0010_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    // Writes to $6000-$7FFF should be ignored (no PRG-RAM).
    bus.write(0x6000, 0xFF);
    assert_eq!(bus.read(0x6000), 0x00);
    // Bank selection should be unaffected.
    assert_eq!(bus.read(0x8000), 0x00);
}

#[test]
fn uxrom_cart_loads_in_emulator() {
    use nes_emu::emulator::EmulatorState;
    let bytes = make_ines_uxrom(4, 0, 0b0010_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let emu = EmulatorState::new(cart);
    assert_eq!(emu.bus().cartridge().unwrap().header.mapper_number, 2);
}

// =====================================================================
// CNROM (mapper 3) integration tests
// =====================================================================

/// Build an iNES image for CNROM (mapper 3). PRG is filled so each 16 KB
/// bank has a unique byte (bank index). CHR is filled so each 8 KB bank
/// has a unique byte (bank index).
fn make_ines_cnrom(prg_16k_banks: u8, chr_8k_banks: u8, flags6: u8) -> Vec<u8> {
    let prg_size = prg_16k_banks as usize * PRG_ROM_UNIT;
    let chr_size = chr_8k_banks as usize * 8 * 1024;
    let mut buf = Vec::with_capacity(HEADER_SIZE + prg_size + chr_size);
    buf.extend_from_slice(&INES_MAGIC);
    buf.push(prg_16k_banks);
    buf.push(chr_8k_banks);
    buf.push(flags6); // mapper low nibble in high nibble (3 → 0b0011_0000)
    buf.push(0); // flags7 — mapper high nibble = 0
    buf.extend_from_slice(&[0u8; 8]);
    buf.resize(HEADER_SIZE + prg_size + chr_size, 0);
    // Fill PRG: each 16 KB bank with its bank index.
    for i in 0..prg_size {
        buf[HEADER_SIZE + i] = (i / PRG_ROM_UNIT) as u8;
    }
    // Fill CHR: each 8 KB bank with its bank index.
    for i in 0..chr_size {
        buf[HEADER_SIZE + prg_size + i] = (i / (8 * 1024)) as u8;
    }
    buf
}

#[test]
fn cnrom_selected_for_mapper_3() {
    // flags6 high nibble = 3 → mapper 3. Horizontal mirroring.
    let bytes = make_ines_cnrom(2, 4, 0b0011_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load CNROM cart");
    assert_eq!(cart.header.mapper_number, 3);
    assert_eq!(cart.mirror_mode(), Mirroring::Horizontal);
}

#[test]
fn cnrom_prg_32k_reads_linearly_via_bus() {
    // 2 × 16 KB PRG banks (32 KB total). No PRG banking.
    let bytes = make_ines_cnrom(2, 4, 0b0011_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    assert_eq!(bus.read(0x8000), 0x00);
    assert_eq!(bus.read(0xBFFF), 0x00);
    assert_eq!(bus.read(0xC000), 0x01);
    assert_eq!(bus.read(0xFFFF), 0x01);
}

#[test]
fn cnrom_prg_16k_mirrors_high_half_via_bus() {
    // 1 × 16 KB PRG bank — mirrors into $C000-$FFFF.
    let bytes = make_ines_cnrom(1, 4, 0b0011_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    assert_eq!(bus.read(0x8000), 0x00);
    assert_eq!(bus.read(0xC000), 0x00); // mirror
    assert_eq!(bus.read(0xFFFF), 0x00);
}

#[test]
fn cnrom_chr_bank_switch_via_bus() {
    // 4 × 8 KB CHR banks.
    let bytes = make_ines_cnrom(2, 4, 0b0011_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    // Default chr_bank = 0.
    {
        let cart = bus.cartridge().expect("cart");
        assert_eq!(cart.read_chr(0x0000), 0x00);
        assert_eq!(cart.read_chr(0x1FFF), 0x00);
    }
    // Select CHR bank 2.
    bus.write(0x8000, 0x02);
    {
        let cart = bus.cartridge().expect("cart");
        assert_eq!(cart.read_chr(0x0000), 0x02);
        assert_eq!(cart.read_chr(0x1FFF), 0x02);
    }
    // Select CHR bank 3.
    bus.write(0x8000, 0x03);
    {
        let cart = bus.cartridge().expect("cart");
        assert_eq!(cart.read_chr(0x0000), 0x03);
    }
}

#[test]
fn cnrom_chr_bank_uses_low_2_bits_via_bus() {
    // 4 × 8 KB CHR banks. Writing 0xFF → bank 3 (bits 0-1).
    let bytes = make_ines_cnrom(2, 4, 0b0011_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    bus.write(0x8000, 0xFF);
    let cart = bus.cartridge().expect("cart");
    assert_eq!(cart.read_chr(0x0000), 0x03);
}

#[test]
fn cnrom_mirroring_fixed_from_header_via_bus() {
    // Vertical mirroring from header.
    let bytes = make_ines_cnrom(2, 4, 0b0011_0001);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    assert_eq!(bus.ppu().mirroring(), Mirroring::Vertical);
    // CHR bank writes must not change mirroring.
    bus.write(0x8000, 0x02);
    assert_eq!(bus.ppu().mirroring(), Mirroring::Vertical);
}

#[test]
fn cnrom_cart_loads_in_emulator() {
    use nes_emu::emulator::EmulatorState;
    let bytes = make_ines_cnrom(2, 4, 0b0011_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let emu = EmulatorState::new(cart);
    assert_eq!(emu.bus().cartridge().unwrap().header.mapper_number, 3);
}

// =====================================================================
// AxROM (mapper 7) integration tests
// =====================================================================

/// Build an iNES image for AxROM (mapper 7). PRG is filled so each 32 KB
/// bank has a unique byte (bank index). CHR is filled so each 8 KB bank
/// has a unique byte (bank index).
fn make_ines_axrom(prg_32k_banks: u8, chr_8k_banks: u8, flags6: u8) -> Vec<u8> {
    let prg_size = prg_32k_banks as usize * 32 * 1024;
    let chr_size = chr_8k_banks as usize * 8 * 1024;
    let mut buf = Vec::with_capacity(HEADER_SIZE + prg_size + chr_size);
    buf.extend_from_slice(&INES_MAGIC);
    // PRG banks in 16KB units: prg_32k_banks * 2.
    buf.push(prg_32k_banks.saturating_mul(2).max(1));
    buf.push(chr_8k_banks);
    buf.push(flags6); // mapper low nibble in high nibble (7 → 0b0111_0000)
    buf.push(0); // flags7 — mapper high nibble = 0
    buf.extend_from_slice(&[0u8; 8]);
    buf.resize(HEADER_SIZE + prg_size + chr_size, 0);
    // Fill PRG: each 32 KB bank with its bank index.
    for i in 0..prg_size {
        buf[HEADER_SIZE + i] = (i / (32 * 1024)) as u8;
    }
    // Fill CHR: each 8 KB bank with its bank index.
    for i in 0..chr_size {
        buf[HEADER_SIZE + prg_size + i] = (i / (8 * 1024)) as u8;
    }
    buf
}

#[test]
fn axrom_selected_for_mapper_7() {
    // flags6 high nibble = 7 → mapper 7. Header mirroring is horizontal but
    // AxROM always reports single-screen.
    let bytes = make_ines_axrom(4, 1, 0b0111_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load AxROM cart");
    assert_eq!(cart.header.mapper_number, 7);
    assert_eq!(cart.mirror_mode(), Mirroring::SingleScreen(0));
}

#[test]
fn axrom_default_bank_0_across_whole_window_via_bus() {
    // 4 × 32 KB PRG banks. Default: bank 0 across $8000-$FFFF.
    let bytes = make_ines_axrom(4, 0, 0b0111_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    assert_eq!(bus.read(0x8000), 0x00);
    assert_eq!(bus.read(0xBFFF), 0x00);
    assert_eq!(bus.read(0xC000), 0x00);
    assert_eq!(bus.read(0xFFFF), 0x00);
}

#[test]
fn axrom_bank_switch_swaps_whole_32k_window_via_bus() {
    let bytes = make_ines_axrom(4, 0, 0b0111_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    bus.write(0x8000, 0x02);
    assert_eq!(bus.read(0x8000), 0x02);
    assert_eq!(bus.read(0xBFFF), 0x02);
    assert_eq!(bus.read(0xC000), 0x02);
    assert_eq!(bus.read(0xFFFF), 0x02);
}

#[test]
fn axrom_mirroring_switches_via_bit4_via_bus() {
    let bytes = make_ines_axrom(4, 0, 0b0111_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    // Default: single-screen NT 0.
    assert_eq!(bus.ppu().mirroring(), Mirroring::SingleScreen(0));
    // Bit 4 set → single-screen NT 1.
    bus.write(0x8000, 0x10);
    assert_eq!(bus.ppu().mirroring(), Mirroring::SingleScreen(1));
    // Bit 4 clear → back to NT 0.
    bus.write(0x8000, 0x00);
    assert_eq!(bus.ppu().mirroring(), Mirroring::SingleScreen(0));
}

#[test]
fn axrom_mirroring_affects_ppu_nametable_mapping() {
    let bytes = make_ines_axrom(4, 0, 0b0111_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);

    // Under SingleScreen(0) (default), all four NT slots map to page 0.
    // Write 0xAA to $2000 → page 0 offset 0.
    assert_eq!(bus.ppu().mirroring(), Mirroring::SingleScreen(0));
    ppu_vram_write(&mut bus, 0x2000, 0xAA);

    // Switch to SingleScreen(1) via bit 4 — now all NT slots map to page 1.
    // Write 0xBB to $2000 → page 1 offset 0.
    bus.write(0x8000, 0x10);
    assert_eq!(bus.ppu().mirroring(), Mirroring::SingleScreen(1));
    ppu_vram_write(&mut bus, 0x2000, 0xBB);

    // Under SingleScreen(1): all four NT slots → page 1 (0xBB).
    let vram = bus.ppu().vram();
    let m = bus.ppu().mirroring();
    assert_eq!(vram[nt_phys_addr(m, 0x2000)], 0xBB);
    assert_eq!(vram[nt_phys_addr(m, 0x2400)], 0xBB);
    assert_eq!(vram[nt_phys_addr(m, 0x2800)], 0xBB);
    assert_eq!(vram[nt_phys_addr(m, 0x2C00)], 0xBB);

    // Switch back to SingleScreen(0): all four NT slots → page 0 (0xAA).
    bus.write(0x8000, 0x00);
    assert_eq!(bus.ppu().mirroring(), Mirroring::SingleScreen(0));
    let vram = bus.ppu().vram();
    let m = bus.ppu().mirroring();
    assert_eq!(vram[nt_phys_addr(m, 0x2000)], 0xAA);
    assert_eq!(vram[nt_phys_addr(m, 0x2400)], 0xAA);
    assert_eq!(vram[nt_phys_addr(m, 0x2800)], 0xAA);
    assert_eq!(vram[nt_phys_addr(m, 0x2C00)], 0xAA);
}

#[test]
fn axrom_chr_ram_writes_persist_via_bus() {
    // chr_8k_banks = 0 → CHR-RAM.
    let bytes = make_ines_axrom(4, 0, 0b0111_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    {
        let cart = bus.cartridge_mut().expect("cart");
        cart.write_chr(0x0000, 0xAA);
        cart.write_chr(0x1FFF, 0xBB);
    }
    let cart = bus.cartridge().expect("cart");
    assert_eq!(cart.read_chr(0x0000), 0xAA);
    assert_eq!(cart.read_chr(0x1FFF), 0xBB);
}

#[test]
fn axrom_prg_ram_region_ignored_via_bus() {
    let bytes = make_ines_axrom(4, 0, 0b0111_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let mut bus = Bus::with_cartridge(cart);
    bus.write(0x6000, 0xFF);
    assert_eq!(bus.read(0x6000), 0x00);
    // Bank selection unaffected.
    assert_eq!(bus.read(0x8000), 0x00);
}

#[test]
fn axrom_cart_loads_in_emulator() {
    use nes_emu::emulator::EmulatorState;
    let bytes = make_ines_axrom(4, 0, 0b0111_0000);
    let cart = Cartridge::from_bytes(&bytes).expect("load");
    let emu = EmulatorState::new(cart);
    assert_eq!(emu.bus().cartridge().unwrap().header.mapper_number, 7);
}
