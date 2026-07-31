/*
 * cnrom.c - Mapper 3 (CNROM). Port of src/mappers/cnrom.rs to C (M4.4).
 *
 * CHR counterpart to UxROM: no PRG bank switching (PRG-ROM mapped linearly
 * across $8000-$FFFF, with 16 KB mirroring for single-bank carts), but CHR
 * is bank-switched in 8 KB windows. The bank register is written anywhere
 * in $8000-$FFFF (low 2 bits select 1 of 4 CHR banks). No PRG-RAM, no IRQ.
 *
 * See: https://www.nesdev.org/wiki/CNROM
 */
#include "mapper.h"
#include "cartridge.h"
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#define PRG_BANK_SIZE 16384u
#define CHR_BANK_SIZE 8192u

typedef struct CnromState {
    uint8_t* prg_rom;
    uint32_t prg_size;
    uint8_t* chr;
    uint32_t chr_size;
    bool     chr_is_ram;
    Mirroring mirroring;
    bool     has_battery;
    uint8_t  chr_bank; /* currently selected 8 KB CHR bank. */
} CnromState;

static uint32_t cnrom_chr_bank_count(const CnromState* s) {
    uint32_t n = s->chr_size / CHR_BANK_SIZE;
    return n > 0u ? n : 1u;
}

static uint32_t cnrom_prg_index(const CnromState* s, uint16_t addr) {
    uint32_t local = (uint32_t)(addr - 0x8000u);
    uint32_t bank = s->prg_size;
    if (bank == 0u) {
        return 0u;
    }
    return local % bank;
}

static uint8_t cnrom_read_prg(const void* state, uint16_t addr) {
    const CnromState* s = (const CnromState*)state;
    if (addr < 0x8000u) {
        return 0x00u;
    }
    uint32_t idx = cnrom_prg_index(s, addr);
    if (idx >= s->prg_size) {
        return 0x00u;
    }
    return s->prg_rom[idx];
}

static void cnrom_write_prg(void* state, uint16_t addr, uint8_t value) {
    CnromState* s = (CnromState*)state;
    if (addr < 0x8000u) {
        return;
    }
    s->chr_bank = value;
}

static uint8_t cnrom_read_chr(const void* state, uint16_t addr) {
    const CnromState* s = (const CnromState*)state;
    uint32_t count = cnrom_chr_bank_count(s);
    uint32_t bank = (uint32_t)s->chr_bank % count;
    uint32_t idx = bank * CHR_BANK_SIZE + ((uint32_t)addr & (CHR_BANK_SIZE - 1u));
    if (idx >= s->chr_size) {
        return 0x00u;
    }
    return s->chr[idx];
}

static void cnrom_write_chr(void* state, uint16_t addr, uint8_t value) {
    CnromState* s = (CnromState*)state;
    if (!s->chr_is_ram) {
        return;
    }
    uint32_t count = cnrom_chr_bank_count(s);
    uint32_t bank = (uint32_t)s->chr_bank % count;
    uint32_t idx = bank * CHR_BANK_SIZE + ((uint32_t)addr & (CHR_BANK_SIZE - 1u));
    if (idx < s->chr_size) {
        s->chr[idx] = value;
    }
}

static Mirroring cnrom_mirror_mode(const void* state) {
    return ((const CnromState*)state)->mirroring;
}

static bool cnrom_chr_is_ram(const void* state) {
    return ((const CnromState*)state)->chr_is_ram;
}

static bool cnrom_has_battery(const void* state) {
    return ((const CnromState*)state)->has_battery;
}

static void cnrom_destroy(void* state) {
    CnromState* s = (CnromState*)state;
    if (s) {
        free(s->prg_rom);
        free(s->chr);
        free(s);
    }
}

static const MapperVTable CNROM_VTABLE = {
    cnrom_read_prg, NULL, cnrom_write_prg,
    cnrom_read_chr, NULL, cnrom_write_chr,
    cnrom_mirror_mode, cnrom_chr_is_ram, cnrom_has_battery,
    NULL, NULL, NULL, NULL, NULL, cnrom_destroy
};

int cnrom_create(const uint8_t* prg, uint32_t prg_size,
                 const uint8_t* chr, uint32_t chr_size,
                 Mirroring mirroring, bool has_battery, Mapper* out) {
    CnromState* s = (CnromState*)malloc(sizeof(CnromState));
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
    /* CHR: 8 KB RAM if no CHR-ROM, else copy CHR-ROM (may be > 8 KB). */
    if (chr_size == 0u) {
        s->chr_size = CHR_BANK_SIZE;
        s->chr = (uint8_t*)malloc(CHR_BANK_SIZE);
        if (!s->chr) {
            free(s->prg_rom);
            free(s);
            return 1;
        }
        memset(s->chr, 0, CHR_BANK_SIZE);
        s->chr_is_ram = true;
    } else {
        s->chr_size = chr_size;
        s->chr = (uint8_t*)malloc(chr_size);
        if (!s->chr) {
            free(s->prg_rom);
            free(s);
            return 1;
        }
        memcpy(s->chr, chr, chr_size);
        s->chr_is_ram = false;
    }
    s->mirroring = mirroring;
    s->has_battery = has_battery;
    s->chr_bank = 0u;
    out->vt = &CNROM_VTABLE;
    out->state = s;
    out->mapper_num = 3u;
    return 0;
}
