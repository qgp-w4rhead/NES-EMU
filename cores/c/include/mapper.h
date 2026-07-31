/*
 * mapper.h - Mapper vtable abstraction for the NES cartridge system.
 *
 * Port of src/mappers/mod.rs (the `Mapper` trait + `from_ines` dispatch) to C
 * (M4.4). Each mapper board (NROM, MMC1, MMC3, VRC6, FDS, ...) implements a
 * `MapperVTable` and exposes a heap-allocated state struct. The `Mapper`
 * struct holds a vtable pointer + opaque state pointer + the iNES mapper
 * number, exactly mirroring Rust's `Box<dyn Mapper>`.
 *
 * PRG addresses are $6000..=$FFFF (de-mirrored by the bus); CHR are
 * $0000..=$1FFF. Optional hooks (IRQ, scanline counter, CPU clocking,
 * expansion audio) default to no-op/false/0.0 when the vtable entry is NULL.
 *
 * See: https://www.nesdev.org/wiki/Mapper
 */
#ifndef NES_CORE_C_MAPPER_H
#define NES_CORE_C_MAPPER_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Nametable mirroring mode (mappers `Mirroring`). Single-screen variants
 * encode the visible nametable index (0-3) in the enum value, matching
 * Rust's `Mirroring::SingleScreen(u8)`. (mappers/mod.rs `Mirroring`.)
 *
 * Defined here (in the lower-level mapper.h) rather than cartridge.h so
 * that cartridge.h can include mapper.h for the full Mapper struct without
 * a circular dependency. */
typedef enum {
    MIRROR_HORIZONTAL,
    MIRROR_VERTICAL,
    MIRROR_FOUR_SCREEN,
    MIRROR_SINGLE_SCREEN_0,  /* SingleScreen(0) - AxROM / MMC1 1ScA. */
    MIRROR_SINGLE_SCREEN_1,  /* SingleScreen(1) - AxROM / MMC1 1ScB. */
    MIRROR_SINGLE_SCREEN_2,  /* SingleScreen(2) - AxROM. */
    MIRROR_SINGLE_SCREEN_3   /* SingleScreen(3) - AxROM. */
} Mirroring;

/* ---- Mapper vtable (mirrors src/mappers/mod.rs `trait Mapper`) ------- */

typedef struct MapperVTable {
    /* Read a byte from CPU-side PRG ($6000..=$FFFF). (Mapper::read_prg.) */
    uint8_t  (*read_prg)(const void* state, uint16_t addr);
    /* Read PRG with side effects (FDS disk I/O). NULL = fall back to read_prg. */
    uint8_t  (*read_prg_mut)(void* state, uint16_t addr);
    /* Write a byte to CPU-side PRG ($6000..=$FFFF). (Mapper::write_prg.) */
    void     (*write_prg)(void* state, uint16_t addr, uint8_t val);
    /* Read a byte from PPU-side CHR ($0000..=$1FFF). (Mapper::read_chr.) */
    uint8_t  (*read_chr)(const void* state, uint16_t addr);
    /* Read CHR with side effects (MMC2 latching). NULL = fall back to read_chr. */
    uint8_t  (*read_chr_latched)(void* state, uint16_t addr);
    /* Write a byte to PPU-side CHR ($0000..=$1FFF). (Mapper::write_chr.) */
    void     (*write_chr)(void* state, uint16_t addr, uint8_t val);
    /* Nametable mirroring mode. (Mapper::mirror_mode.) */
    Mirroring (*mirror_mode)(const void* state);
    /* Whether CHR is RAM (writable). Added for cartridge_chr_is_ram. */
    bool     (*chr_is_ram)(const void* state);
    /* Whether the cartridge has battery-backed PRG-RAM. NULL = false. */
    bool     (*has_battery)(const void* state);
    /* Whether the mapper is asserting a CPU IRQ. NULL = false. */
    bool     (*irq_pending)(const void* state);
    /* Clock IRQ counter (MMC3 A12 rising edge / MMC5 scanline). NULL = no-op. */
    void     (*clock_irq)(void* state);
    /* Reset per-frame scanline counter (MMC5). NULL = no-op. */
    void     (*reset_scanline_counter)(void* state);
    /* Advance CPU-clocked logic (FME-7/VRC6/VRC7/FDS IRQ + audio). NULL = no-op. */
    void     (*clock_cpu)(void* state, uint32_t cpu_cycles);
    /* Expansion-audio sample [-1.0, 1.0]. NULL = 0.0. */
    float    (*expansion_audio_sample)(const void* state);
    /* Free heap-allocated PRG/CHR/state. NULL = no-op (no allocation). */
    void     (*destroy)(void* state);
    /* Serialize mapper state into buf. If buf is NULL, return required size
     * only. If buf is non-NULL, write state and return bytes written. NULL =
     * no writable state (size 0). (M4.5 save state.) */
    size_t   (*save_state)(const void* state, uint8_t* buf);
    /* Restore mapper state from buf. Returns true on success. The mapper
     * state already exists (created from the same ROM); buffers are valid.
     * NULL = no writable state (no-op, returns true). (M4.5 save state.) */
    bool     (*load_state)(void* state, const uint8_t* buf, size_t len);
} MapperVTable;

/* A loaded mapper: vtable + opaque state + iNES mapper number.
 * (Rust `Box<dyn Mapper>`.) */
typedef struct Mapper {
    const MapperVTable* vt;
    void* state;
    uint16_t mapper_num;
} Mapper;

/* Construct the appropriate mapper for an iNES mapper number. Returns 0 on
 * success, non-zero on unsupported mapper / OOM. Each constructor mallocs its
 * state struct and copies PRG/CHR data into heap buffers. (mappers/mod.rs
 * `from_ines`.) FDS (mapper 20) is NOT handled here - it uses
 * cartridge_from_fds_bytes (BIOS + disk data, not PRG/CHR). For VRC6,
 * mapper 24 passes is_26=false, mapper 26 passes is_26=true. */
int mapper_from_ines(uint16_t mapper_number,
                     const uint8_t* prg_rom, uint32_t prg_size,
                     const uint8_t* chr_rom, uint32_t chr_size,
                     Mirroring mirroring, bool has_battery,
                     Mapper* out);

/* ---- Per-mapper constructors (each returns 0 on success) ------------- */
/* Defined in src/<mapper>.c. Each mallocs the state, copies PRG/CHR, and
 * sets out->vt / out->state / out->mapper_num. */

int nrom_create(const uint8_t* prg, uint32_t prg_size,
                const uint8_t* chr, uint32_t chr_size,
                Mirroring mirroring, bool has_battery, Mapper* out);
int uxrom_create(const uint8_t* prg, uint32_t prg_size,
                 const uint8_t* chr, uint32_t chr_size,
                 Mirroring mirroring, bool has_battery, Mapper* out);
int cnrom_create(const uint8_t* prg, uint32_t prg_size,
                 const uint8_t* chr, uint32_t chr_size,
                 Mirroring mirroring, bool has_battery, Mapper* out);
int axrom_create(const uint8_t* prg, uint32_t prg_size,
                 const uint8_t* chr, uint32_t chr_size,
                 Mirroring mirroring, bool has_battery, Mapper* out);
int mmc1_create(const uint8_t* prg, uint32_t prg_size,
                const uint8_t* chr, uint32_t chr_size,
                Mirroring mirroring, bool has_battery, Mapper* out);
int mmc3_create(const uint8_t* prg, uint32_t prg_size,
                const uint8_t* chr, uint32_t chr_size,
                Mirroring mirroring, bool has_battery, Mapper* out);
int mmc5_create(const uint8_t* prg, uint32_t prg_size,
                const uint8_t* chr, uint32_t chr_size,
                Mirroring mirroring, bool has_battery, Mapper* out);
int mmc2_create(const uint8_t* prg, uint32_t prg_size,
                const uint8_t* chr, uint32_t chr_size,
                Mirroring mirroring, bool has_battery, Mapper* out);
int vrc6_create(const uint8_t* prg, uint32_t prg_size,
                const uint8_t* chr, uint32_t chr_size,
                Mirroring mirroring, bool has_battery, bool is_26, Mapper* out);
int fme7_create(const uint8_t* prg, uint32_t prg_size,
                const uint8_t* chr, uint32_t chr_size,
                Mirroring mirroring, bool has_battery, Mapper* out);
int vrc7_create(const uint8_t* prg, uint32_t prg_size,
                const uint8_t* chr, uint32_t chr_size,
                Mirroring mirroring, bool has_battery, Mapper* out);
int namco163_create(const uint8_t* prg, uint32_t prg_size,
                    const uint8_t* chr, uint32_t chr_size,
                    Mirroring mirroring, bool has_battery, Mapper* out);
/* FDS constructor: takes BIOS + raw disk data (NOT PRG/CHR). */
int fds_create(const uint8_t* bios, size_t bios_len,
               const uint8_t* disk_data, size_t disk_len, Mapper* out);

#ifdef __cplusplus
}
#endif
#endif /* NES_CORE_C_MAPPER_H */
