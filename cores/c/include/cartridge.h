/*
 * cartridge.h — MINIMAL iNES cartridge for M4.1 (NROM only).
 *
 * Port of the minimal subset of src/cartridge.rs + src/mappers/nrom.rs needed
 * for M4.1. This is a SCOPED but fully-functional implementation: it parses the
 * 16-byte iNES header, supports NROM (mapper 0) with 16 KB (mirrored) or 32 KB
 * (linear) PRG-ROM, and provides PRG read with the 16 KB mirror / 32 KB linear
 * mapping that NROM requires. CHR is held as an 8 KB buffer (RAM when
 * chr_rom_banks == 0, ROM otherwise). PRG-RAM region ($6000-$7FFF) reads 0 and
 * writes are ignored, matching NROM.
 *
 * Full iNES support (all 13 mappers, trainer, NES 2.0, PRG-RAM, battery) is
 * M4.4 — NOT this milestone. Everything here works correctly for NROM-128
 * (the only cartridge the M4.1 NOP ROM test uses).
 *
 * See: https://www.nesdev.org/wiki/INES
 * See: https://www.nesdev.org/wiki/NROM
 */
#ifndef NES_CORE_C_CARTRIDGE_H
#define NES_CORE_C_CARTRIDGE_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* iNES header is always 16 bytes. (cartridge.rs `HEADER_SIZE`.) */
#define CART_HEADER_SIZE 16u
/* PRG-ROM unit size as reported by the iNES header (16 KB blocks). (cartridge.rs `PRG_ROM_UNIT`.) */
#define CART_PRG_ROM_UNIT 16384u
/* CHR-ROM unit size as reported by the iNES header (8 KB blocks). (cartridge.rs `CHR_ROM_UNIT`.) */
#define CART_CHR_ROM_UNIT 8192u
/* Maximum PRG-ROM we support in M4.1 (32 KB — NROM-256). */
#define CART_PRG_ROM_MAX  32768u
/* CHR buffer size (always 8 KB for NROM). */
#define CART_CHR_SIZE     8192u

/* Nametable mirroring mode (mappers `Mirroring`). */
typedef enum {
    MIRROR_HORIZONTAL,
    MIRROR_VERTICAL,
    MIRROR_FOUR_SCREEN
} Mirroring;

/* Parsed iNES header fields relevant to emulation (cartridge.rs `InesHeader`). */
typedef struct InesHeader {
    uint8_t  prg_rom_banks;  /* Number of 16 KB PRG-ROM banks. */
    uint8_t  chr_rom_banks;  /* Number of 8 KB CHR-ROM banks (0 = CHR-RAM). */
    uint16_t mapper_number;  /* Mapper number (low nibble flags6 hi, high nibble flags7 hi). */
    Mirroring mirroring;     /* Nametable mirroring from flags 6. */
    bool     has_trainer;    /* 512-byte trainer present between header and PRG-ROM. */
    bool     has_battery;    /* Battery-backed PRG-RAM present. */
    uint8_t  tv_system;      /* TV system hint from byte 9 (0=NTSC,1=PAL,2=Dendy,3=dual). */
} InesHeader;

/* MINIMAL cartridge for M4.1: holds PRG-ROM bytes + size, an 8 KB CHR buffer,
 * a chr_is_ram flag, mirroring, and the parsed header. This is enough for
 * NROM-128 / NROM-256. (Combines src/cartridge.rs `Cartridge` + src/mappers/nrom.rs `Nrom`.) */
typedef struct Cartridge {
    InesHeader header;
    uint8_t  prg_rom[CART_PRG_ROM_MAX]; /* PRG-ROM bytes (16 KB or 32 KB). */
    uint32_t prg_size;                  /* Actual PRG-ROM size in bytes (16384 or 32768). */
    uint8_t  chr[CART_CHR_SIZE];        /* CHR data (8 KB ROM or RAM). */
    bool     chr_is_ram;                /* True when chr_rom_banks == 0 (CHR-RAM). */
} Cartridge;

/* Parse the 16-byte iNES header. Returns 0 on success, non-zero on bad magic
 * or insufficient length. (cartridge.rs `InesHeader::parse`.) */
int cartridge_parse_header(const uint8_t* bytes, size_t len, InesHeader* out);

/* Load and parse an iNES ROM image from a byte buffer (no file I/O). Builds the
 * minimal NROM cartridge. Returns 0 on success, non-zero on error. Only mapper
 * 0 (NROM) is supported in M4.1; other mappers return non-zero.
 * (cartridge.rs `from_bytes` + mappers `from_ines` for NROM.) */
int cartridge_from_bytes(const uint8_t* bytes, size_t len, Cartridge* out);

/* ---- PRG / CHR access (mappers/nrom.rs `Mapper` impl) ----------------- */

/* Read a PRG byte. $6000-$7FFF returns 0 (no PRG-RAM on NROM). $8000-$FFFF
 * reads PRG-ROM with 16 KB mirror / 32 KB linear. (nrom.rs `read_prg`.) */
uint8_t cartridge_read_prg(const Cartridge* cart, uint16_t addr);

/* Write a PRG byte. NROM has no writable PRG-RAM / bank regs — ignored. (nrom.rs `write_prg`.) */
void cartridge_write_prg(Cartridge* cart, uint16_t addr, uint8_t value);

/* Read a CHR byte. (nrom.rs `read_chr`.) */
uint8_t cartridge_read_chr(const Cartridge* cart, uint16_t addr);

/* Write a CHR byte. Ignored for CHR-ROM; persisted for CHR-RAM. (nrom.rs `write_chr`.) */
void cartridge_write_chr(Cartridge* cart, uint16_t addr, uint8_t value);

/* Nametable mirroring mode. (nrom.rs `mirror_mode`.) */
Mirroring cartridge_mirror_mode(const Cartridge* cart);

#ifdef __cplusplus
}
#endif
#endif /* NES_CORE_C_CARTRIDGE_H */

