/*
 * cartridge.c — MINIMAL iNES cartridge for M4.1 (NROM only).
 *
 * Port of the minimal subset of src/cartridge.rs (header parse) + src/mappers/nrom.rs
 * (PRG/CHR access) to C (M4.1). Parses the 16-byte iNES header, supports NROM
 * (mapper 0) with 16 KB (mirrored) or 32 KB (linear) PRG-ROM, and provides
 * PRG read with the 16 KB mirror / 32 KB linear mapping. CHR is an 8 KB buffer
 * (RAM when chr_rom_banks == 0, ROM otherwise). PRG-RAM region ($6000-$7FFF)
 * reads 0 and writes are ignored, matching NROM.
 *
 * Full iNES support (all 13 mappers, trainer, NES 2.0, PRG-RAM, battery) is
 * M4.4 — NOT this milestone. Everything here works correctly for NROM-128.
 *
 * See: https://www.nesdev.org/wiki/INES
 * See: https://www.nesdev.org/wiki/NROM
 */
#include "cartridge.h"
#include <string.h>

/* iNES file magic: bytes 0..=3 are "NES\x1A". (cartridge.rs `INES_MAGIC`.) */
static const uint8_t INES_MAGIC[4] = { 'N', 'E', 'S', 0x1A };

/* ---- Header parse (cartridge.rs `InesHeader::parse`) ----------------- */

int cartridge_parse_header(const uint8_t* bytes, size_t len, InesHeader* out) {
    if (len < CART_HEADER_SIZE) {
        return 1; /* TooShort */
    }
    if (memcmp(bytes, INES_MAGIC, 4) != 0) {
        return 2; /* BadMagic */
    }

    uint8_t prg_rom_banks = bytes[4];
    uint8_t chr_rom_banks = bytes[5];
    uint8_t flags6 = bytes[6];
    uint8_t flags7 = bytes[7];

    bool has_trainer  = (flags6 & 0x04u) != 0u;
    bool has_battery  = (flags6 & 0x02u) != 0u;
    bool four_screen  = (flags6 & 0x08u) != 0u;
    bool vertical     = (flags6 & 0x01u) != 0u;

    Mirroring mirroring;
    if (four_screen) {
        mirroring = MIRROR_FOUR_SCREEN;
    } else if (vertical) {
        mirroring = MIRROR_VERTICAL;
    } else {
        mirroring = MIRROR_HORIZONTAL;
    }

    /* Mapper low nibble from flags6 high nibble, high nibble from flags7 high nibble. */
    uint16_t mapper_number = (uint16_t)(((uint16_t)(flags6 >> 4)) | (uint16_t)(((uint16_t)(flags7 >> 4)) << 4));

    /* TV system hint from byte 9 bits 0-1. */
    uint8_t tv_system = (uint8_t)(bytes[9] & 0x03u);

    out->prg_rom_banks = prg_rom_banks;
    out->chr_rom_banks = chr_rom_banks;
    out->mapper_number = mapper_number;
    out->mirroring = mirroring;
    out->has_trainer = has_trainer;
    out->has_battery = has_battery;
    out->tv_system = tv_system;
    return 0;
}

/* ---- from_bytes: build minimal NROM cartridge (cartridge.rs `from_bytes` + nrom) ---- */

int cartridge_from_bytes(const uint8_t* bytes, size_t len, Cartridge* out) {
    InesHeader hdr;
    int rc = cartridge_parse_header(bytes, len, &hdr);
    if (rc != 0) {
        return rc;
    }
    /* M4.1 supports only mapper 0 (NROM). */
    if (hdr.mapper_number != 0u) {
        return 3; /* UnsupportedMapper */
    }
    /* PRG size: 16 KB banks. M4.1 supports 1 or 2 banks (NROM-128 / NROM-256). */
    uint32_t prg_size = (uint32_t)hdr.prg_rom_banks * CART_PRG_ROM_UNIT;
    if (prg_size > CART_PRG_ROM_MAX) {
        return 4; /* PRG too large for M4.1 minimal cartridge. */
    }
    /* Compute the byte offset where PRG-ROM begins in the file. */
    size_t prg_off = CART_HEADER_SIZE;
    if (hdr.has_trainer) {
        prg_off += 512u; /* skip 512-byte trainer */
    }
    /* Verify the buffer is long enough for header + trainer + PRG + CHR. */
    size_t chr_size = (hdr.chr_rom_banks > 0u) ? (size_t)hdr.chr_rom_banks * CART_CHR_ROM_UNIT : 0u;
    if (len < prg_off + prg_size + chr_size) {
        return 5; /* TooShort for declared PRG/CHR. */
    }

    /* Zero the struct first. */
    memset(out, 0, sizeof(Cartridge));
    out->header = hdr;
    out->prg_size = prg_size;
    memcpy(out->prg_rom, bytes + prg_off, prg_size);

    /* CHR: 8 KB. ROM when chr_rom_banks >= 1, RAM (zeroed) when 0. */
    if (hdr.chr_rom_banks > 0u) {
        memcpy(out->chr, bytes + prg_off + prg_size, CART_CHR_SIZE);
        out->chr_is_ram = false;
    } else {
        memset(out->chr, 0, CART_CHR_SIZE);
        out->chr_is_ram = true;
    }
    return 0;
}

/* ---- PRG read (nrom.rs `read_prg`) ----------------------------------- */

uint8_t cartridge_read_prg(const Cartridge* cart, uint16_t addr) {
    /* PRG-RAM region ($6000-$7FFF) — not present on stock NROM. */
    if (addr < 0x8000u) {
        return 0x00u;
    }
    /* PRG-ROM lives at $8000..=$FFFF; subtract $8000, then mirror/modulo by
     * bank size (16 KB mirror or 32 KB linear). (nrom.rs `prg_index`.) */
    uint32_t local = (uint32_t)(addr - 0x8000u);
    uint32_t bank  = cart->prg_size;
    if (bank == 0u) {
        return 0x00u;
    }
    uint32_t idx = local % bank;
    if (idx >= CART_PRG_ROM_MAX) {
        return 0x00u;
    }
    return cart->prg_rom[idx];
}

/* ---- PRG write (nrom.rs `write_prg`) --------------------------------- */

void cartridge_write_prg(Cartridge* cart, uint16_t addr, uint8_t value) {
    /* NROM has no writable PRG-RAM and no bank-switch registers. Silently
     * ignore writes (including the $6000-$7FFF range). */
    (void)cart;
    (void)addr;
    (void)value;
}

/* ---- CHR read (nrom.rs `read_chr`) ----------------------------------- */

uint8_t cartridge_read_chr(const Cartridge* cart, uint16_t addr) {
    uint32_t idx = (uint32_t)addr % CART_CHR_SIZE;
    return cart->chr[idx];
}

/* ---- CHR write (nrom.rs `write_chr`) --------------------------------- */

void cartridge_write_chr(Cartridge* cart, uint16_t addr, uint8_t value) {
    if (cart->chr_is_ram) {
        uint32_t idx = (uint32_t)addr % CART_CHR_SIZE;
        cart->chr[idx] = value;
    }
    /* CHR-ROM writes are ignored. */
}

/* ---- mirror_mode (nrom.rs `mirror_mode`) ----------------------------- */

Mirroring cartridge_mirror_mode(const Cartridge* cart) {
    return cart->header.mirroring;
}

