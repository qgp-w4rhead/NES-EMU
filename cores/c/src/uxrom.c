/*
 * uxrom.c - Mapper 2 (UxROM). Port of src/mappers/uxrom.rs to C (M4.4).
 *
 * Nintendo's simplest PRG bank-switching board. $8000-$BFFF is a switchable
 * 16 KB PRG-ROM bank; $C000-$FFFF is hard-wired to the last 16 KB bank. CHR
 * is a single 8 KB window of ROM or RAM. No PRG-RAM, no IRQ. The bank
 * register is written anywhere in $8000-$FFFF (low bits select the bank).
 * Mirroring is fixed by the cartridge solder pads.
 *
 * See: https://www.nesdev.org/wiki/UxROM
 */
#include "mapper.h"
#include "cartridge.h"
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#define PRG_BANK_SIZE 16384u
#define CHR_SIZE      8192u

typedef struct UxromState {
    uint8_t* prg_rom;
    uint32_t prg_size;
    uint8_t* chr;
    uint32_t chr_size;
    bool     chr_is_ram;
    Mirroring mirroring;
    bool     has_battery;
    uint8_t  bank;  /* currently selected 16 KB PRG bank at $8000-$BFFF. */
} UxromState;

static uint32_t uxrom_prg_bank_count(const UxromState* s) {
    uint32_t n = s->prg_size / PRG_BANK_SIZE;
    return n > 0u ? n : 1u;
}

static uint8_t uxrom_read_prg(const void* state, uint16_t addr) {
    const UxromState* s = (const UxromState*)state;
    if (addr < 0x8000u) {
        return 0x00u; /* no PRG-RAM on UxROM. */
    }
    uint32_t local = (uint32_t)(addr - 0x8000u); /* 0..=0x7FFF */
    uint32_t offset = local & (PRG_BANK_SIZE - 1u);
    uint32_t count = uxrom_prg_bank_count(s);
    uint32_t idx;
    if (local < PRG_BANK_SIZE) {
        /* $8000-$BFFF: switchable bank. */
        uint32_t bank = (uint32_t)s->bank % count;
        idx = bank * PRG_BANK_SIZE + offset;
    } else {
        /* $C000-$FFFF: fixed last bank. */
        uint32_t last = count - 1u;
        idx = last * PRG_BANK_SIZE + offset;
    }
    if (idx >= s->prg_size) {
        return 0x00u;
    }
    return s->prg_rom[idx];
}

static void uxrom_write_prg(void* state, uint16_t addr, uint8_t value) {
    UxromState* s = (UxromState*)state;
    if (addr < 0x8000u) {
        return; /* no PRG-RAM. */
    }
    /* Any write to $8000-$FFFF latches the low bits as the bank number. */
    s->bank = value;
}

static uint8_t uxrom_read_chr(const void* state, uint16_t addr) {
    const UxromState* s = (const UxromState*)state;
    uint32_t sz = s->chr_size;
    if (sz == 0u) {
        return 0x00u;
    }
    return s->chr[(uint32_t)addr % sz];
}

static void uxrom_write_chr(void* state, uint16_t addr, uint8_t value) {
    UxromState* s = (UxromState*)state;
    if (!s->chr_is_ram) {
        return;
    }
    uint32_t sz = s->chr_size;
    if (sz == 0u) {
        return;
    }
    s->chr[(uint32_t)addr % sz] = value;
}

static Mirroring uxrom_mirror_mode(const void* state) {
    return ((const UxromState*)state)->mirroring;
}

static bool uxrom_chr_is_ram(const void* state) {
    return ((const UxromState*)state)->chr_is_ram;
}

static bool uxrom_has_battery(const void* state) {
    return ((const UxromState*)state)->has_battery;
}

static void uxrom_destroy(void* state) {
    UxromState* s = (UxromState*)state;
    if (s) {
        free(s->prg_rom);
        free(s->chr);
        free(s);
    }
}

/* ---- Save / load state (M4.5) ---------------------------------------- */

/* Layout: [sizeof(UxromState)] struct + [chr_size] CHR (only if chr_is_ram). */
static size_t uxrom_save_state(const void* state, uint8_t* buf) {
    const UxromState* s = (const UxromState*)state;
    size_t total = sizeof(UxromState);
    if (s->chr_is_ram) {
        total += s->chr_size;
    }
    if (buf) {
        memcpy(buf, s, sizeof(UxromState));
        if (s->chr_is_ram && s->chr_size > 0u) {
            memcpy(buf + sizeof(UxromState), s->chr, s->chr_size);
        }
    }
    return total;
}

static bool uxrom_load_state(void* state, const uint8_t* buf, size_t len) {
    UxromState* s = (UxromState*)state;
    size_t need = sizeof(UxromState);
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
    memcpy(s, buf, sizeof(UxromState));
    s->prg_rom = valid_prg;
    s->prg_size = valid_prg_size;
    s->chr = valid_chr;
    s->chr_size = valid_chr_size;
    s->chr_is_ram = valid_chr_is_ram;
    if (s->chr_is_ram && s->chr_size > 0u) {
        memcpy(s->chr, buf + sizeof(UxromState), s->chr_size);
    }
    return true;
}

static const MapperVTable UXROM_VTABLE = {
    uxrom_read_prg, NULL, uxrom_write_prg,
    uxrom_read_chr, NULL, uxrom_write_chr,
    uxrom_mirror_mode, uxrom_chr_is_ram, uxrom_has_battery,
    NULL, NULL, NULL, NULL, NULL, uxrom_destroy,
    uxrom_save_state, uxrom_load_state
};

int uxrom_create(const uint8_t* prg, uint32_t prg_size,
                 const uint8_t* chr, uint32_t chr_size,
                 Mirroring mirroring, bool has_battery, Mapper* out) {
    UxromState* s = (UxromState*)malloc(sizeof(UxromState));
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
    s->mirroring = mirroring;
    s->has_battery = has_battery;
    s->bank = 0u;
    out->vt = &UXROM_VTABLE;
    out->state = s;
    out->mapper_num = 2u;
    return 0;
}
