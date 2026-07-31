/*
 * axrom.c - Mapper 7 (AxROM). Port of src/mappers/axrom.rs to C (M4.4).
 *
 * Switches PRG-ROM in 32 KB windows: the entire $8000-$FFFF range is mapped
 * to a single selectable bank. The bank register also controls single-screen
 * nametable mirroring (bit 4 selects which of two physical NT pages is
 * visible at all four NT slots). CHR is a single 8 KB window of ROM or RAM.
 * No PRG-RAM, no IRQ.
 *
 * See: https://www.nesdev.org/wiki/AxROM
 */
#include "mapper.hpp"
#include "cartridge.hpp"
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#define PRG_BANK_SIZE 32768u
#define CHR_SIZE      8192u

typedef struct AxromState {
    uint8_t* prg_rom;
    uint32_t prg_size;
    uint8_t* chr;
    uint32_t chr_size;
    bool     chr_is_ram;
    bool     has_battery;
    uint8_t  prg_bank;  /* bits 0-2: 32 KB PRG bank. */
    uint8_t  mirror_nt; /* bit 4: single-screen NT page (0 or 1). */
} AxromState;

static uint32_t axrom_prg_bank_count(const AxromState* s) {
    uint32_t n = s->prg_size / PRG_BANK_SIZE;
    return n > 0u ? n : 1u;
}

static uint8_t axrom_read_prg(const void* state, uint16_t addr) {
    const AxromState* s = (const AxromState*)state;
    if (addr < 0x8000u) {
        return 0x00u;
    }
    uint32_t local = (uint32_t)(addr - 0x8000u);
    uint32_t count = axrom_prg_bank_count(s);
    uint32_t bank = (uint32_t)s->prg_bank % count;
    uint32_t idx = bank * PRG_BANK_SIZE + local;
    if (idx >= s->prg_size) {
        return 0x00u;
    }
    return s->prg_rom[idx];
}

static void axrom_write_prg(void* state, uint16_t addr, uint8_t value) {
    AxromState* s = (AxromState*)state;
    if (addr < 0x8000u) {
        return;
    }
    s->mirror_nt = (uint8_t)((value >> 4) & 1u);
    s->prg_bank = (uint8_t)(value & 0x07u);
}

static uint8_t axrom_read_chr(const void* state, uint16_t addr) {
    const AxromState* s = (const AxromState*)state;
    uint32_t sz = s->chr_size;
    if (sz == 0u) {
        return 0x00u;
    }
    return s->chr[(uint32_t)addr % sz];
}

static void axrom_write_chr(void* state, uint16_t addr, uint8_t value) {
    AxromState* s = (AxromState*)state;
    if (!s->chr_is_ram) {
        return;
    }
    uint32_t sz = s->chr_size;
    if (sz == 0u) {
        return;
    }
    s->chr[(uint32_t)addr % sz] = value;
}

static Mirroring axrom_mirror_mode(const void* state) {
    const AxromState* s = (const AxromState*)state;
    /* AxROM is always single-screen; bit 4 selects NT 0 or 1. */
    return s->mirror_nt == 0u ? MIRROR_SINGLE_SCREEN_0 : MIRROR_SINGLE_SCREEN_1;
}

static bool axrom_chr_is_ram(const void* state) {
    return ((const AxromState*)state)->chr_is_ram;
}

static bool axrom_has_battery(const void* state) {
    return ((const AxromState*)state)->has_battery;
}

static void axrom_destroy(void* state) {
    AxromState* s = (AxromState*)state;
    if (s) {
        free(s->prg_rom);
        free(s->chr);
        free(s);
    }
}

/* ---- Save / load state (M4.5) ---------------------------------------- */

/* Layout: [sizeof(AxromState)] struct + [chr_size] CHR (only if chr_is_ram). */
static size_t axrom_save_state(const void* state, uint8_t* buf) {
    const AxromState* s = (const AxromState*)state;
    size_t total = sizeof(AxromState);
    if (s->chr_is_ram) {
        total += s->chr_size;
    }
    if (buf) {
        memcpy(buf, s, sizeof(AxromState));
        if (s->chr_is_ram && s->chr_size > 0u) {
            memcpy(buf + sizeof(AxromState), s->chr, s->chr_size);
        }
    }
    return total;
}

static bool axrom_load_state(void* state, const uint8_t* buf, size_t len) {
    AxromState* s = (AxromState*)state;
    size_t need = sizeof(AxromState);
    if (s->chr_is_ram) {
        need += s->chr_size;
    }
    if (len < need) {
        return false;
    }
    uint8_t* valid_prg = s->prg_rom;
    uint8_t* valid_chr = s->chr;
    uint32_t valid_prg_size = s->prg_size;
    uint32_t valid_chr_size = s->chr_size;
    bool valid_chr_is_ram = s->chr_is_ram;
    memcpy(s, buf, sizeof(AxromState));
    s->prg_rom = valid_prg;
    s->prg_size = valid_prg_size;
    s->chr = valid_chr;
    s->chr_size = valid_chr_size;
    s->chr_is_ram = valid_chr_is_ram;
    if (s->chr_is_ram && s->chr_size > 0u) {
        memcpy(s->chr, buf + sizeof(AxromState), s->chr_size);
    }
    return true;
}

static const MapperVTable AXROM_VTABLE = {
    axrom_read_prg, NULL, axrom_write_prg,
    axrom_read_chr, NULL, axrom_write_chr,
    axrom_mirror_mode, axrom_chr_is_ram, axrom_has_battery,
    NULL, NULL, NULL, NULL, NULL, axrom_destroy,
    axrom_save_state, axrom_load_state
};

int axrom_create(const uint8_t* prg, uint32_t prg_size,
                 const uint8_t* chr, uint32_t chr_size,
                 Mirroring mirroring, bool has_battery, Mapper* out) {
    AxromState* s = (AxromState*)malloc(sizeof(AxromState));
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
        s->chr_size = CHR_SIZE;
        s->chr = (uint8_t*)malloc(CHR_SIZE);
        if (!s->chr) {
            free(s->prg_rom);
            free(s);
            return 1;
        }
        memset(s->chr, 0, CHR_SIZE);
        s->chr_is_ram = true;
    }
    s->has_battery = has_battery;
    s->prg_bank = 0u;
    s->mirror_nt = 0u;
    (void)mirroring; /* AxROM ignores header mirroring (always single-screen). */
    out->vt = &AXROM_VTABLE;
    out->state = s;
    out->mapper_num = 7u;
    return 0;
}
