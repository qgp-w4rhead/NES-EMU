/*
 * cartridge.h - iNES cartridge loader + mapper dispatch (M4.4).
 *
 * Port of src/cartridge.rs (header parse + from_bytes / from_fds_bytes) and
 * src/mappers/mod.rs (Mirroring enum) to C. The Cartridge struct owns a
 * `Mapper` (vtable + opaque state) and dispatches PRG/CHR/mirroring/IRQ/
 * audio calls through it. All 13 iNES mappers from the Rust core are
 * supported (NROM, MMC1/2/3/5, UxROM, CNROM, AxROM, VRC6, VRC7, FME-7,
 * Namco 163, FDS).
 *
 * See: https://www.nesdev.org/wiki/INES
 * See: https://www.nesdev.org/wiki/Mapper
 */
#ifndef NES_CORE_C_CARTRIDGE_H
#define NES_CORE_C_CARTRIDGE_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* iNES header is always 16 bytes. (cartridge.rs `HEADER_SIZE`.) */
#define CART_HEADER_SIZE 16u
/* Optional trainer area size (skipped if present). (cartridge.rs `TRAINER_SIZE`.) */
#define CART_TRAINER_SIZE 512u
/* PRG-ROM unit size as reported by the iNES header (16 KB blocks). (cartridge.rs `PRG_ROM_UNIT`.) */
#define CART_PRG_ROM_UNIT 16384u
/* CHR-ROM unit size as reported by the iNES header (8 KB blocks). (cartridge.rs `CHR_ROM_UNIT`.) */
#define CART_CHR_ROM_UNIT 8192u

/* mapper.h defines the Mirroring enum (the lower-level type) and the full
 * Mapper struct (MapperVTable + state + mapper_num) that Cartridge holds by
 * value. Including it here gives us both; cartridge.h does not need to be
 * included by mapper.h. */
#include "mapper.h"

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

/* A loaded NES cartridge - owns the parsed header and a Mapper (vtable +
 * opaque state). (cartridge.rs `Cartridge`.) */
typedef struct Cartridge {
    InesHeader header;
    Mapper mapper;  /* vtable + state + mapper_num (full def from mapper.h). */
} Cartridge;

/* Parse the 16-byte iNES header. Returns 0 on success, non-zero on bad magic
 * or insufficient length. (cartridge.rs `InesHeader::parse`.) */
int cartridge_parse_header(const uint8_t* bytes, size_t len, InesHeader* out);

/* Load and parse an iNES ROM image from a byte buffer (no file I/O). Builds
 * the appropriate mapper via mapper_from_ines. Returns 0 on success, non-zero
 * on error (bad header, unsupported mapper, OOM). (cartridge.rs `from_bytes`
 * + mappers `from_ines`.) */
int cartridge_from_bytes(const uint8_t* bytes, size_t len, Cartridge* out);

/* Build an FDS cartridge from raw `.fds` disk image bytes and BIOS ROM data.
 * The disk image is parsed (FdsDisk::parse) to validate the format; the raw
 * disk side data (16-byte FDS file header stripped) is passed to the FDS
 * mapper. Returns 0 on success, non-zero on parse error / OOM.
 * (cartridge.rs `from_fds_bytes`.) */
int cartridge_from_fds_bytes(const uint8_t* disk_data, size_t disk_len,
                             const uint8_t* bios, size_t bios_len,
                             Cartridge* out);

/* Free heap-allocated mapper state (PRG/CHR buffers, mapper state struct).
 * Safe to call on a zeroed Cartridge. (Rust `Drop` for Cartridge.) */
void cartridge_destroy(Cartridge* cart);

/* ---- PRG / CHR access (mappers `Mapper` trait) ------------------------ */

/* Read a PRG byte. $6000-$7FFF may be PRG-RAM (mapper-dependent).
 * (Mapper::read_prg.) */
uint8_t cartridge_read_prg(const Cartridge* cart, uint16_t addr);

/* Read a PRG byte with side effects (FDS disk-data read advancing the read
 * pointer). Falls back to read_prg for mappers without read side-effects.
 * (Mapper::read_prg_mut.) */
uint8_t cartridge_read_prg_mut(Cartridge* cart, uint16_t addr);

/* Write a PRG byte (PRG-RAM write or mapper bank register write).
 * (Mapper::write_prg.) */
void cartridge_write_prg(Cartridge* cart, uint16_t addr, uint8_t value);

/* Read a CHR byte. (Mapper::read_chr.) */
uint8_t cartridge_read_chr(const Cartridge* cart, uint16_t addr);

/* Read a CHR byte with side effects (MMC2 bank latching). Falls back to
 * read_chr for mappers without latching. (Mapper::read_chr_latched.) */
uint8_t cartridge_read_chr_latched(Cartridge* cart, uint16_t addr);

/* Write a CHR byte. Ignored for CHR-ROM; persisted for CHR-RAM.
 * (Mapper::write_chr.) */
void cartridge_write_chr(Cartridge* cart, uint16_t addr, uint8_t value);

/* Nametable mirroring mode. (Mapper::mirror_mode.) */
Mirroring cartridge_mirror_mode(const Cartridge* cart);

/* Whether CHR is RAM (writable). (Mapper::chr_is_ram.) */
bool cartridge_chr_is_ram(const Cartridge* cart);

/* Whether the cartridge has battery-backed PRG-RAM. (Mapper::has_battery.) */
bool cartridge_has_battery(const Cartridge* cart);

/* Whether the mapper is asserting a CPU IRQ. (Mapper::irq_pending.) */
bool cartridge_irq_pending(const Cartridge* cart);

/* Clock the mapper's IRQ counter by one step (MMC3 A12 rising edge / MMC5
 * scanline). (Mapper::clock_irq.) */
void cartridge_clock_irq(Cartridge* cart);

/* Reset the mapper's per-frame scanline counter (MMC5). Called at the start
 * of each frame (prerender scanline). (Mapper::reset_scanline_counter.) */
void cartridge_reset_scanline_counter(Cartridge* cart);

/* Advance the mapper's CPU-clocked logic by `cpu_cycles` CPU cycles. Used by
 * mappers whose IRQ timer runs on the CPU clock (FME-7, VRC6, VRC7, FDS).
 * (Mapper::clock_cpu.) */
void cartridge_clock_cpu(Cartridge* cart, uint32_t cpu_cycles);

/* Current expansion-audio sample in [-1.0, 1.0] from the cartridge's audio
 * chip (VRC6/VRC7/Sunsoft 5B/Namco 163/FDS). Returns 0.0 for carts without
 * expansion audio. (Mapper::expansion_audio_sample.) */
float cartridge_expansion_audio_sample(const Cartridge* cart);

/* ---- Save / load state (M4.5) ---------------------------------------- */

/* Serialize the mapper state into buf. If buf is NULL, return the required
 * size only. If buf is non-NULL, write state and return bytes written.
 * Returns 0 if the mapper has no save_state vtable entry. */
size_t cartridge_save_state(const Cartridge* cart, uint8_t* buf);

/* Restore mapper state from buf. Returns true on success, false on failure
 * (no load_state vtable entry, or buffer too small). */
bool cartridge_load_state(Cartridge* cart, const uint8_t* buf, size_t len);

#ifdef __cplusplus
}
#endif
#endif /* NES_CORE_C_CARTRIDGE_H */
