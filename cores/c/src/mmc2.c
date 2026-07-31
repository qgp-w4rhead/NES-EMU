/*
 * mmc2.c - Mapper 9 (MMC2). Port of src/mappers/mmc2.rs to C (M4.4).
 *
 * Nintendo's mapper used by Punch-Out!!. Defining feature is CHR bank
 * latching: four 4 KB CHR bank registers, only two active at a time. PPU
 * reads from $0FD8/$0FE8 (left) and $1FD8/$1FE8 (right) toggle the active
 * bank. PRG: 8 KB switchable at $8000, fixed last 24 KB at $A000. 1 KB
 * PRG-RAM (mirrored across $6000-$7FFF). Fixed mirroring from header.
 *
 * See: https://www.nesdev.org/wiki/MMC2
 */
#include "mapper.h"
#include "cartridge.h"
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#define PRG_BANK_SIZE 8192u
#define CHR_4K_SIZE   4096u
#define PRG_RAM_SIZE  1024u

/* CHR latch trigger address ranges. (mmc2.rs.) */
#define LATCH_LEFT_BANK0_LO   0x0FD8u
#define LATCH_LEFT_BANK0_HI   0x0FDFu
#define LATCH_LEFT_BANK1_LO   0x0FE8u
#define LATCH_LEFT_BANK1_HI   0x0FEFu
#define LATCH_RIGHT_BANK0_LO  0x1FD8u
#define LATCH_RIGHT_BANK0_HI  0x1FDFu
#define LATCH_RIGHT_BANK1_LO  0x1FE8u
#define LATCH_RIGHT_BANK1_HI  0x1FEFu

typedef struct Mmc2State {
    uint8_t* prg_rom;
    uint32_t prg_size;
    uint8_t* chr;
    uint32_t chr_size;
    bool     chr_is_ram;
    uint8_t  prg_ram[PRG_RAM_SIZE];
    bool     has_battery;
    uint8_t  prg_bank;
    uint8_t  chr_banks[4];
    uint8_t  latch_left;
    uint8_t  latch_right;
    Mirroring mirroring;
} Mmc2State;

static uint32_t mmc2_prg_bank_count(const Mmc2State* s) {
    uint32_t n = s->prg_size / PRG_BANK_SIZE;
    return n > 0u ? n : 1u;
}

static uint32_t mmc2_chr_bank_count(const Mmc2State* s) {
    uint32_t n = s->chr_size / CHR_4K_SIZE;
    return n > 0u ? n : 1u;
}

static uint8_t mmc2_prg_read_bank(const Mmc2State* s, uint32_t bank, uint32_t offset) {
    uint32_t count = mmc2_prg_bank_count(s);
    bank = bank % count;
    uint32_t idx = bank * PRG_BANK_SIZE + offset;
    if (idx >= s->prg_size) {
        return 0x00u;
    }
    return s->prg_rom[idx];
}

static uint8_t mmc2_left_bank(const Mmc2State* s) {
    return s->latch_left == 0u ? s->chr_banks[0] : s->chr_banks[1];
}

static uint8_t mmc2_right_bank(const Mmc2State* s) {
    return s->latch_right == 0u ? s->chr_banks[2] : s->chr_banks[3];
}

static void mmc2_update_latches(Mmc2State* s, uint16_t addr) {
    if (addr >= LATCH_LEFT_BANK0_LO && addr <= LATCH_LEFT_BANK0_HI) {
        s->latch_left = 0u;
    } else if (addr >= LATCH_LEFT_BANK1_LO && addr <= LATCH_LEFT_BANK1_HI) {
        s->latch_left = 1u;
    } else if (addr >= LATCH_RIGHT_BANK0_LO && addr <= LATCH_RIGHT_BANK0_HI) {
        s->latch_right = 0u;
    } else if (addr >= LATCH_RIGHT_BANK1_LO && addr <= LATCH_RIGHT_BANK1_HI) {
        s->latch_right = 1u;
    }
}

static uint8_t mmc2_read_prg(const void* state, uint16_t addr) {
    const Mmc2State* s = (const Mmc2State*)state;
    if (addr >= 0x6000u && addr < 0x8000u) {
        uint32_t idx = (uint32_t)(addr - 0x6000u) & (PRG_RAM_SIZE - 1u);
        return s->prg_ram[idx];
    }
    uint32_t local = (uint32_t)(addr - 0x8000u);
    uint32_t count = mmc2_prg_bank_count(s);
    if (local < PRG_BANK_SIZE) {
        uint32_t bank = (uint32_t)s->prg_bank % count;
        return mmc2_prg_read_bank(s, bank, local);
    }
    /* $A000-$FFFF: fixed last 24 KB = last 3 x 8 KB banks. */
    uint32_t fixed_offset = local - PRG_BANK_SIZE;
    uint32_t fixed_bank_base = count >= 3u ? count - 3u : 0u;
    uint32_t bank = fixed_bank_base + (fixed_offset / PRG_BANK_SIZE);
    uint32_t offset = fixed_offset & (PRG_BANK_SIZE - 1u);
    return mmc2_prg_read_bank(s, bank, offset);
}

static void mmc2_write_prg(void* state, uint16_t addr, uint8_t value) {
    Mmc2State* s = (Mmc2State*)state;
    if (addr >= 0x6000u && addr < 0x8000u) {
        uint32_t idx = (uint32_t)(addr - 0x6000u) & (PRG_RAM_SIZE - 1u);
        s->prg_ram[idx] = value;
        return;
    }
    switch (addr) {
        case 0xA000u: s->prg_bank = (uint8_t)(value & 0x0Fu); break;
        case 0xB000u: s->chr_banks[0] = (uint8_t)(value & 0x3Fu); break;
        case 0xB001u: s->chr_banks[1] = (uint8_t)(value & 0x3Fu); break;
        case 0xB002u: s->chr_banks[2] = (uint8_t)(value & 0x3Fu); break;
        case 0xB003u: s->chr_banks[3] = (uint8_t)(value & 0x3Fu); break;
        default: break;
    }
}

static uint8_t mmc2_read_chr(const void* state, uint16_t addr) {
    const Mmc2State* s = (const Mmc2State*)state;
    uint8_t bank = ((uint32_t)addr < CHR_4K_SIZE) ? mmc2_left_bank(s) : mmc2_right_bank(s);
    uint32_t count = mmc2_chr_bank_count(s);
    uint32_t b = (uint32_t)bank % count;
    uint32_t offset = (uint32_t)addr & (CHR_4K_SIZE - 1u);
    uint32_t idx = b * CHR_4K_SIZE + offset;
    if (idx >= s->chr_size) {
        return 0x00u;
    }
    return s->chr[idx];
}

static uint8_t mmc2_read_chr_latched(void* state, uint16_t addr) {
    Mmc2State* s = (Mmc2State*)state;
    mmc2_update_latches(s, addr);
    return mmc2_read_chr(s, addr);
}

static void mmc2_write_chr(void* state, uint16_t addr, uint8_t value) {
    Mmc2State* s = (Mmc2State*)state;
    if (!s->chr_is_ram) {
        return;
    }
    uint8_t bank = ((uint32_t)addr < CHR_4K_SIZE) ? mmc2_left_bank(s) : mmc2_right_bank(s);
    uint32_t count = mmc2_chr_bank_count(s);
    uint32_t b = (uint32_t)bank % count;
    uint32_t offset = (uint32_t)addr & (CHR_4K_SIZE - 1u);
    uint32_t idx = b * CHR_4K_SIZE + offset;
    if (idx < s->chr_size) {
        s->chr[idx] = value;
    }
}

static Mirroring mmc2_mirror_mode(const void* state) {
    return ((const Mmc2State*)state)->mirroring;
}

static bool mmc2_chr_is_ram(const void* state) {
    return ((const Mmc2State*)state)->chr_is_ram;
}

static bool mmc2_has_battery(const void* state) {
    return ((const Mmc2State*)state)->has_battery;
}

static void mmc2_destroy(void* state) {
    Mmc2State* s = (Mmc2State*)state;
    if (s) {
        free(s->prg_rom);
        free(s->chr);
        free(s);
    }
}

/* ---- Save / load state (M4.5) ---------------------------------------- */

/* Layout: [sizeof(Mmc2State)] struct (includes embedded prg_ram[1024]) +
 * [chr_size] CHR (only if chr_is_ram). */
static size_t mmc2_save_state(const void* state, uint8_t* buf) {
    const Mmc2State* s = (const Mmc2State*)state;
    size_t total = sizeof(Mmc2State);
    if (s->chr_is_ram) {
        total += s->chr_size;
    }
    if (buf) {
        memcpy(buf, s, sizeof(Mmc2State));
        if (s->chr_is_ram && s->chr_size > 0u) {
            memcpy(buf + sizeof(Mmc2State), s->chr, s->chr_size);
        }
    }
    return total;
}

static bool mmc2_load_state(void* state, const uint8_t* buf, size_t len) {
    Mmc2State* s = (Mmc2State*)state;
    size_t need = sizeof(Mmc2State);
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
    memcpy(s, buf, sizeof(Mmc2State));
    s->prg_rom = valid_prg;
    s->prg_size = valid_prg_size;
    s->chr = valid_chr;
    s->chr_size = valid_chr_size;
    s->chr_is_ram = valid_chr_is_ram;
    if (s->chr_is_ram && s->chr_size > 0u) {
        memcpy(s->chr, buf + sizeof(Mmc2State), s->chr_size);
    }
    return true;
}

static const MapperVTable MMC2_VTABLE = {
    mmc2_read_prg, NULL, mmc2_write_prg,
    mmc2_read_chr, mmc2_read_chr_latched, mmc2_write_chr,
    mmc2_mirror_mode, mmc2_chr_is_ram, mmc2_has_battery,
    NULL, NULL, NULL, NULL, NULL, mmc2_destroy,
    mmc2_save_state, mmc2_load_state
};

int mmc2_create(const uint8_t* prg, uint32_t prg_size,
                const uint8_t* chr, uint32_t chr_size,
                Mirroring mirroring, bool has_battery, Mapper* out) {
    Mmc2State* s = (Mmc2State*)malloc(sizeof(Mmc2State));
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
    memset(s->prg_ram, 0, PRG_RAM_SIZE);
    s->has_battery = has_battery;
    s->prg_bank = 0u;
    memset(s->chr_banks, 0, sizeof(s->chr_banks));
    s->latch_left = 0u;
    s->latch_right = 0u;
    s->mirroring = mirroring;
    out->vt = &MMC2_VTABLE;
    out->state = s;
    out->mapper_num = 9u;
    return 0;
}
