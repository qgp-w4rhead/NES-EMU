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
