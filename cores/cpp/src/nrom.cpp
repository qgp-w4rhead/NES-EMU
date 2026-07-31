/*
 * nrom.c - Mapper 0 (NROM). Port of src/mappers/nrom.rs to C (M4.4).
 *
 * The simplest NES board: no bank switching, no extra registers, no IRQ.
 * PRG-ROM is either 16 KB (mirrored into both halves of $8000-$FFFF) or
 * 32 KB (mapped linearly). CHR is 8 KB of ROM (chr_rom_banks >= 1) or RAM
 * (chr_rom_banks == 0). PRG-RAM at $6000-$7FFF is not present on stock NROM.
 *
 * See: https://www.nesdev.org/wiki/NROM
 */
#include "mapper.hpp"
#include "cartridge.hpp"
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

/* NROM cartridge state. (nrom.rs `struct Nrom`.) */
typedef struct NromState {
    uint8_t* prg_rom;    /* heap PRG-ROM (prg_size bytes). */
    uint32_t prg_size;   /* PRG-ROM size in bytes (16384 or 32768). */
    uint8_t* chr;        /* heap CHR (8 KB ROM or RAM). */
    uint32_t chr_size;   /* CHR buffer size (8192). */
    bool     chr_is_ram; /* true when chr_rom_banks == 0 (CHR-RAM). */
    Mirroring mirroring; /* nametable mirroring (fixed for NROM). */
    bool     has_battery;/* battery-backed PRG-RAM flag (rare on NROM). */
} NromState;

/* Mask a CPU PRG address into the PRG-ROM index range. (nrom.rs `prg_index`.) */
static uint32_t nrom_prg_index(const NromState* s, uint16_t addr) {
    /* Addresses are $6000..=$FFFF; PRG-ROM lives at $8000..=$FFFF. */
    uint32_t local = (uint32_t)(addr - 0x8000u);
    uint32_t bank = s->prg_size;
    if (bank == 0u) {
        return 0u;
    }
    return local % bank;
}

static uint8_t nrom_read_prg(const void* state, uint16_t addr) {
    const NromState* s = (const NromState*)state;
    /* PRG-RAM region ($6000-$7FFF) - not present on stock NROM. */
    if (addr < 0x8000u) {
        return 0x00u;
    }
    uint32_t idx = nrom_prg_index(s, addr);
    if (idx >= s->prg_size) {
        return 0x00u;
    }
    return s->prg_rom[idx];
}

static void nrom_write_prg(void* state, uint16_t addr, uint8_t value) {
    /* NROM has no writable PRG-RAM and no bank-switch registers. Silently
     * ignore writes (including the $6000-$7FFF range). (nrom.rs.) */
    (void)state;
    (void)addr;
    (void)value;
}

static uint8_t nrom_read_chr(const void* state, uint16_t addr) {
    const NromState* s = (const NromState*)state;
    uint32_t sz = s->chr_size;
    if (sz == 0u) {
        return 0x00u;
    }
    uint32_t idx = (uint32_t)addr % sz;
    return s->chr[idx];
}

static void nrom_write_chr(void* state, uint16_t addr, uint8_t value) {
    NromState* s = (NromState*)state;
    if (!s->chr_is_ram) {
        return; /* CHR-ROM writes are ignored. */
    }
    uint32_t sz = s->chr_size;
    if (sz == 0u) {
        return;
    }
    uint32_t idx = (uint32_t)addr % sz;
    s->chr[idx] = value;
}

static Mirroring nrom_mirror_mode(const void* state) {
    const NromState* s = (const NromState*)state;
    return s->mirroring;
}

static bool nrom_chr_is_ram(const void* state) {
    const NromState* s = (const NromState*)state;
    return s->chr_is_ram;
}

static bool nrom_has_battery(const void* state) {
    const NromState* s = (const NromState*)state;
    return s->has_battery;
}

static void nrom_destroy(void* state) {
    NromState* s = (NromState*)state;
    if (s) {
        free(s->prg_rom);
        free(s->chr);
        free(s);
    }
}

/* ---- Save / load state (M4.5) ---------------------------------------- */

/* Layout: [sizeof(NromState)] struct + [chr_size] CHR (only if chr_is_ram).
 * The struct memcpy captures prg_rom/chr pointers (stale after load); the
 * load path restores the valid pointers before copying buffer contents. */
static size_t nrom_save_state(const void* state, uint8_t* buf) {
    const NromState* s = (const NromState*)state;
    size_t total = sizeof(NromState);
    if (s->chr_is_ram) {
        total += s->chr_size;
    }
    if (buf) {
        memcpy(buf, s, sizeof(NromState));
        if (s->chr_is_ram && s->chr_size > 0u) {
            memcpy(buf + sizeof(NromState), s->chr, s->chr_size);
        }
    }
    return total;
}

static bool nrom_load_state(void* state, const uint8_t* buf, size_t len) {
    NromState* s = (NromState*)state;
    size_t need = sizeof(NromState);
    if (s->chr_is_ram) {
        need += s->chr_size;
    }
    if (len < need) {
        return false;
    }
    /* Save valid pointers, memcpy the struct (overwrites pointers), then
     * restore the valid pointers before copying buffer contents. */
    uint8_t* valid_prg = s->prg_rom;
    uint8_t* valid_chr = s->chr;
    uint32_t valid_prg_size = s->prg_size;
    uint32_t valid_chr_size = s->chr_size;
    bool valid_chr_is_ram = s->chr_is_ram;
    memcpy(s, buf, sizeof(NromState));
    s->prg_rom = valid_prg;
    s->prg_size = valid_prg_size;
    s->chr = valid_chr;
    s->chr_size = valid_chr_size;
    s->chr_is_ram = valid_chr_is_ram;
    if (s->chr_is_ram && s->chr_size > 0u) {
        memcpy(s->chr, buf + sizeof(NromState), s->chr_size);
    }
    return true;
}

static const MapperVTable NROM_VTABLE = {
    nrom_read_prg,
    NULL,                  /* read_prg_mut */
    nrom_write_prg,
    nrom_read_chr,
    NULL,                  /* read_chr_latched */
    nrom_write_chr,
    nrom_mirror_mode,
    nrom_chr_is_ram,
    nrom_has_battery,
    NULL,                  /* irq_pending */
    NULL,                  /* clock_irq */
    NULL,                  /* reset_scanline_counter */
    NULL,                  /* clock_cpu */
    NULL,                  /* expansion_audio_sample */
    nrom_destroy,
    nrom_save_state,
    nrom_load_state
};

int nrom_create(const uint8_t* prg, uint32_t prg_size,
                const uint8_t* chr, uint32_t chr_size,
                Mirroring mirroring, bool has_battery, Mapper* out) {
    NromState* s = (NromState*)malloc(sizeof(NromState));
    if (!s) {
        return 1;
    }
    s->prg_size = prg_size;
    s->prg_rom = NULL;
    if (prg_size > 0u) {
        s->prg_rom = (uint8_t*)malloc(prg_size);
        if (!s->prg_rom) {
            free(s);
            return 1;
        }
        memcpy(s->prg_rom, prg, prg_size);
    }
    /* CHR: ROM (full size) when chr_size > 0, 8 KB RAM (zeroed) when 0. */
    if (chr_size > 0u) {
        s->chr_size = chr_size;
        s->chr = (uint8_t*)malloc(chr_size);
        if (!s->chr) {
            free(s->prg_rom);
            free(s);
            return 1;
        }
        memcpy(s->chr, chr, chr_size);
        s->chr_is_ram = false;
    } else {
        s->chr_size = 8192u;
        s->chr = (uint8_t*)malloc(s->chr_size);
        if (!s->chr) {
            free(s->prg_rom);
            free(s);
            return 1;
        }
        memset(s->chr, 0, s->chr_size);
        s->chr_is_ram = true;
    }
    s->mirroring = mirroring;
    s->has_battery = has_battery;
    out->vt = &NROM_VTABLE;
    out->state = s;
    out->mapper_num = 0u;
    return 0;
}
