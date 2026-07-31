/*
 * mmc5.c - Mapper 5 (MMC5). Port of src/mappers/mmc5.rs to C (M4.4).
 *
 * Nintendo's largest NES mapper. PRG-ROM banking (8 KB mode: three
 * switchable 8 KB banks at $8000-$DFFF + fixed last 8 KB at $E000), up to
 * 64 KB PRG-RAM banked in 8 KB windows at $6000-$7FFF (via $5113), 1 KB CHR
 * banking (eight 1 KB banks via $5120-$5127), hardware multiplier ($5205/
 * $5206), switchable per-slot nametable mirroring ($5105), and a scanline
 * IRQ ($5203/$5204) clocked once per scanline via clock_irq.
 *
 * See: https://www.nesdev.org/wiki/MMC5
 */
#include "mapper.h"
#include "cartridge.h"
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#define PRG_BANK_SIZE  8192u
#define CHR_1K_SIZE    1024u
#define PRG_RAM_SIZE   65536u
#define PRG_RAM_WINDOW 8192u

typedef struct Mmc5State {
    uint8_t* prg_rom;
    uint32_t prg_size;
    uint8_t* chr;
    uint32_t chr_size;
    bool     chr_is_ram;
    uint8_t* prg_ram;       /* 64 KB. */
    bool     has_battery;
    uint8_t  prg_mode;
    uint8_t  chr_mode;
    uint8_t  prg_ram_protect1;
    uint8_t  prg_ram_protect2;
    uint8_t  nt_mirroring;
    uint8_t  fill_tile;
    uint8_t  fill_attr;
    uint8_t  prg_ram_bank;
    uint8_t  prg_banks[4];
    uint8_t  chr_banks[8];
    uint8_t  chr_banks_ex[4];
    uint8_t  chr_upper;
    uint8_t  split_control;
    uint8_t  split_y_scroll;
    uint8_t  split_bank;
    uint8_t  irq_scanline;
    uint8_t  irq_control;
    uint8_t  scanline_counter;
    bool     irq_pending;
    bool     in_vblank;
    uint8_t  mult_a;
    uint8_t  mult_b;
} Mmc5State;

static uint32_t mmc5_prg_bank_count(const Mmc5State* s) {
    uint32_t n = s->prg_size / PRG_BANK_SIZE;
    return n > 0u ? n : 1u;
}

static uint32_t mmc5_chr_1k_count(const Mmc5State* s) {
    uint32_t n = s->chr_size / CHR_1K_SIZE;
    return n > 0u ? n : 1u;
}

static uint8_t mmc5_prg_read_bank(const Mmc5State* s, uint32_t bank, uint32_t offset) {
    uint32_t count = mmc5_prg_bank_count(s);
    bank = bank % count;
    uint32_t idx = bank * PRG_BANK_SIZE + offset;
    if (idx >= s->prg_size) {
        return 0x00u;
    }
    return s->prg_rom[idx];
}

static uint16_t mmc5_product(const Mmc5State* s) {
    return (uint16_t)((uint16_t)s->mult_a * (uint16_t)s->mult_b);
}

static bool mmc5_prg_ram_writes_allowed(const Mmc5State* s) {
    return s->prg_ram_protect1 == 0x02u && s->prg_ram_protect2 == 0x01u;
}

static Mirroring mmc5_decode_mirroring(const Mmc5State* s) {
    uint8_t s0 = (uint8_t)(s->nt_mirroring & 0x03u);
    uint8_t s1 = (uint8_t)((s->nt_mirroring >> 2) & 0x03u);
    uint8_t s2 = (uint8_t)((s->nt_mirroring >> 4) & 0x03u);
    uint8_t s3 = (uint8_t)((s->nt_mirroring >> 6) & 0x03u);
    if (s0 == s1 && s1 == s2 && s2 == s3) {
        switch (s0) {
            case 0: return MIRROR_SINGLE_SCREEN_0;
            case 1: return MIRROR_SINGLE_SCREEN_1;
            case 2: return MIRROR_SINGLE_SCREEN_2;
            default: return MIRROR_SINGLE_SCREEN_3;
        }
    }
    if (s0 == 0u && s1 == 1u && s2 == 0u && s3 == 1u) {
        return MIRROR_HORIZONTAL;
    }
    if (s0 == 0u && s1 == 0u && s2 == 1u && s3 == 1u) {
        return MIRROR_VERTICAL;
    }
    if (s0 == 0u && s1 == 1u && s2 == 2u && s3 == 3u) {
        return MIRROR_FOUR_SCREEN;
    }
    return MIRROR_HORIZONTAL;
}

static uint8_t mmc5_read_prg(const void* state, uint16_t addr) {
    const Mmc5State* s = (const Mmc5State*)state;
    if (addr == 0x5205u) {
        return (uint8_t)(mmc5_product(s) & 0xFFu);
    }
    if (addr == 0x5206u) {
        return (uint8_t)((mmc5_product(s) >> 8) & 0xFFu);
    }
    if (addr == 0x5204u) {
        uint8_t status = (uint8_t)(s->irq_control & 0x80u);
        if (s->in_vblank) {
            status = (uint8_t)(status | 0x40u);
        }
        return status;
    }
    if (addr >= 0x6000u && addr < 0x8000u) {
        uint32_t bank = (uint32_t)s->prg_ram_bank % (PRG_RAM_SIZE / PRG_RAM_WINDOW);
        uint32_t idx = bank * PRG_RAM_WINDOW + ((uint32_t)(addr - 0x6000u) & (PRG_RAM_WINDOW - 1u));
        if (idx >= PRG_RAM_SIZE) {
            return 0x00u;
        }
        return s->prg_ram[idx];
    }
    uint32_t local = (uint32_t)(addr - 0x8000u);
    uint32_t slot = local / PRG_BANK_SIZE;
    uint32_t offset = local & (PRG_BANK_SIZE - 1u);
    if (slot == 3u) {
        uint32_t last = mmc5_prg_bank_count(s) - 1u;
        return mmc5_prg_read_bank(s, last, offset);
    }
    uint8_t reg = s->prg_banks[slot];
    if (reg & 0x80u) {
        uint32_t ram_bank = (uint32_t)(reg & 0x7Fu) % (PRG_RAM_SIZE / PRG_BANK_SIZE);
        uint32_t idx = ram_bank * PRG_BANK_SIZE + offset;
        if (idx >= PRG_RAM_SIZE) {
            return 0x00u;
        }
        return s->prg_ram[idx];
    }
    uint32_t count = mmc5_prg_bank_count(s);
    uint32_t bank = (uint32_t)reg % count;
    return mmc5_prg_read_bank(s, bank, offset);
}

static void mmc5_write_prg(void* state, uint16_t addr, uint8_t value) {
    Mmc5State* s = (Mmc5State*)state;
    if (addr == 0x5205u) { s->mult_a = value; return; }
    if (addr == 0x5206u) { s->mult_b = value; return; }
    if (addr == 0x5204u) {
        s->irq_control = (uint8_t)(value & 0x80u);
        if ((value & 0x80u) == 0u) {
            s->irq_pending = false;
        }
        return;
    }
    if (addr == 0x5203u) { s->irq_scanline = value; return; }
    if (addr == 0x5200u) { s->split_control = value; return; }
    if (addr == 0x5201u) { s->split_y_scroll = value; return; }
    if (addr == 0x5202u) { s->split_bank = value; return; }
    switch (addr) {
        case 0x5100u: s->prg_mode = (uint8_t)(value & 0x03u); return;
        case 0x5101u: s->chr_mode = (uint8_t)(value & 0x03u); return;
        case 0x5102u: s->prg_ram_protect1 = (uint8_t)(value & 0x03u); return;
        case 0x5103u: s->prg_ram_protect2 = (uint8_t)(value & 0x03u); return;
        case 0x5104u: return; /* extended NT mode - stored but not wired. */
        case 0x5105u: s->nt_mirroring = value; return;
        case 0x5106u: s->fill_tile = value; return;
        case 0x5107u: s->fill_attr = value; return;
        case 0x5113u: s->prg_ram_bank = value; return;
        case 0x5114u: s->prg_banks[0] = value; return;
        case 0x5115u: s->prg_banks[1] = value; return;
        case 0x5116u: s->prg_banks[2] = value; return;
        case 0x5117u: s->prg_banks[3] = value; return;
        case 0x5120u: s->chr_banks[0] = value; return;
        case 0x5121u: s->chr_banks[1] = value; return;
        case 0x5122u: s->chr_banks[2] = value; return;
        case 0x5123u: s->chr_banks[3] = value; return;
        case 0x5124u: s->chr_banks[4] = value; return;
        case 0x5125u: s->chr_banks[5] = value; return;
        case 0x5126u: s->chr_banks[6] = value; return;
        case 0x5127u: s->chr_banks[7] = value; return;
        case 0x5128u: s->chr_banks_ex[0] = value; return;
        case 0x5129u: s->chr_banks_ex[1] = value; return;
        case 0x512Au: s->chr_banks_ex[2] = value; return;
        case 0x512Bu: s->chr_banks_ex[3] = value; return;
        case 0x5130u: s->chr_upper = (uint8_t)(value & 0x01u); return;
        default: break;
    }
    if (addr >= 0x6000u && addr < 0x8000u && mmc5_prg_ram_writes_allowed(s)) {
        uint32_t bank = (uint32_t)s->prg_ram_bank % (PRG_RAM_SIZE / PRG_RAM_WINDOW);
        uint32_t idx = bank * PRG_RAM_WINDOW + ((uint32_t)(addr - 0x6000u) & (PRG_RAM_WINDOW - 1u));
        if (idx < PRG_RAM_SIZE) {
            s->prg_ram[idx] = value;
        }
        return;
    }
    if (addr >= 0x8000u && addr < 0xE000u && mmc5_prg_ram_writes_allowed(s)) {
        uint32_t local = (uint32_t)(addr - 0x8000u);
        uint32_t slot = local / PRG_BANK_SIZE;
        uint32_t offset = local & (PRG_BANK_SIZE - 1u);
        if (slot < 3u) {
            uint8_t reg = s->prg_banks[slot];
            if (reg & 0x80u) {
                uint32_t ram_bank = (uint32_t)(reg & 0x7Fu) % (PRG_RAM_SIZE / PRG_BANK_SIZE);
                uint32_t idx = ram_bank * PRG_BANK_SIZE + offset;
                if (idx < PRG_RAM_SIZE) {
                    s->prg_ram[idx] = value;
                }
            }
        }
    }
}

static uint8_t mmc5_read_chr(const void* state, uint16_t addr) {
    const Mmc5State* s = (const Mmc5State*)state;
    uint32_t slot = (uint32_t)addr / CHR_1K_SIZE;
    uint32_t offset = (uint32_t)addr & (CHR_1K_SIZE - 1u);
    uint32_t count = mmc5_chr_1k_count(s);
    uint8_t bank_reg = s->chr_banks[slot];
    uint32_t bank = (((uint32_t)s->chr_upper << 8) | (uint32_t)bank_reg) % count;
    uint32_t idx = bank * CHR_1K_SIZE + offset;
    if (idx >= s->chr_size) {
        return 0x00u;
    }
    return s->chr[idx];
}

static void mmc5_write_chr(void* state, uint16_t addr, uint8_t value) {
    Mmc5State* s = (Mmc5State*)state;
    if (!s->chr_is_ram) {
        return;
    }
    uint32_t slot = (uint32_t)addr / CHR_1K_SIZE;
    uint32_t offset = (uint32_t)addr & (CHR_1K_SIZE - 1u);
    uint32_t count = mmc5_chr_1k_count(s);
    uint8_t bank_reg = s->chr_banks[slot];
    uint32_t bank = (((uint32_t)s->chr_upper << 8) | (uint32_t)bank_reg) % count;
    uint32_t idx = bank * CHR_1K_SIZE + offset;
    if (idx < s->chr_size) {
        s->chr[idx] = value;
    }
}

static Mirroring mmc5_mirror_mode(const void* state) {
    return mmc5_decode_mirroring((const Mmc5State*)state);
}

static bool mmc5_chr_is_ram(const void* state) {
    return ((const Mmc5State*)state)->chr_is_ram;
}

static bool mmc5_has_battery(const void* state) {
    return ((const Mmc5State*)state)->has_battery;
}

static bool mmc5_irq_pending(const void* state) {
    return ((const Mmc5State*)state)->irq_pending;
}

static void mmc5_clock_irq(void* state) {
    Mmc5State* s = (Mmc5State*)state;
    s->scanline_counter = (uint8_t)(s->scanline_counter + 1u);
    if (s->scanline_counter >= 240u) {
        s->in_vblank = true;
    }
    if (s->scanline_counter == s->irq_scanline && (s->irq_control & 0x80u) != 0u) {
        s->irq_pending = true;
    }
}

static void mmc5_reset_scanline_counter(void* state) {
    Mmc5State* s = (Mmc5State*)state;
    s->scanline_counter = 0u;
    s->in_vblank = false;
}

static void mmc5_destroy(void* state) {
    Mmc5State* s = (Mmc5State*)state;
    if (s) {
        free(s->prg_rom);
        free(s->chr);
        free(s->prg_ram);
        free(s);
    }
}

/* ---- Save / load state (M4.5) ---------------------------------------- */

/* Layout: [sizeof(Mmc5State)] struct + [PRG_RAM_SIZE] prg_ram +
 * [chr_size] CHR (only if chr_is_ram). The struct memcpy captures
 * prg_rom/chr/prg_ram pointers (stale after load); the load path restores
 * the valid pointers before copying buffer contents. */
static size_t mmc5_save_state(const void* state, uint8_t* buf) {
    const Mmc5State* s = (const Mmc5State*)state;
    size_t total = sizeof(Mmc5State) + PRG_RAM_SIZE;
    if (s->chr_is_ram) {
        total += s->chr_size;
    }
    if (buf) {
        memcpy(buf, s, sizeof(Mmc5State));
        memcpy(buf + sizeof(Mmc5State), s->prg_ram, PRG_RAM_SIZE);
        size_t off = sizeof(Mmc5State) + PRG_RAM_SIZE;
        if (s->chr_is_ram && s->chr_size > 0u) {
            memcpy(buf + off, s->chr, s->chr_size);
        }
    }
    return total;
}

static bool mmc5_load_state(void* state, const uint8_t* buf, size_t len) {
    Mmc5State* s = (Mmc5State*)state;
    size_t need = sizeof(Mmc5State) + PRG_RAM_SIZE;
    if (s->chr_is_ram) {
        need += s->chr_size;
    }
    if (len < need) {
        return false;
    }
    uint8_t* valid_prg = s->prg_rom;
    uint8_t* valid_chr = s->chr;
    uint8_t* valid_prg_ram = s->prg_ram;
    uint32_t valid_prg_size = s->prg_size;
    uint32_t valid_chr_size = s->chr_size;
    bool valid_chr_is_ram = s->chr_is_ram;
    memcpy(s, buf, sizeof(Mmc5State));
    s->prg_rom = valid_prg;
    s->prg_size = valid_prg_size;
    s->chr = valid_chr;
    s->chr_size = valid_chr_size;
    s->chr_is_ram = valid_chr_is_ram;
    s->prg_ram = valid_prg_ram;
    memcpy(s->prg_ram, buf + sizeof(Mmc5State), PRG_RAM_SIZE);
    if (s->chr_is_ram && s->chr_size > 0u) {
        memcpy(s->chr, buf + sizeof(Mmc5State) + PRG_RAM_SIZE, s->chr_size);
    }
    return true;
}

static const MapperVTable MMC5_VTABLE = {
    mmc5_read_prg, NULL, mmc5_write_prg,
    mmc5_read_chr, NULL, mmc5_write_chr,
    mmc5_mirror_mode, mmc5_chr_is_ram, mmc5_has_battery,
    mmc5_irq_pending, mmc5_clock_irq, mmc5_reset_scanline_counter,
    NULL, NULL, mmc5_destroy,
    mmc5_save_state, mmc5_load_state
};

int mmc5_create(const uint8_t* prg, uint32_t prg_size,
                const uint8_t* chr, uint32_t chr_size,
                Mirroring mirroring, bool has_battery, Mapper* out) {
    Mmc5State* s = (Mmc5State*)malloc(sizeof(Mmc5State));
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
    s->prg_ram = (uint8_t*)malloc(PRG_RAM_SIZE);
    if (!s->prg_ram) {
        free(s->prg_rom);
        free(s->chr);
        free(s);
        return 1;
    }
    memset(s->prg_ram, 0, PRG_RAM_SIZE);
    s->has_battery = has_battery;
    s->prg_mode = 3u;
    s->chr_mode = 3u;
    s->prg_ram_protect1 = 0u;
    s->prg_ram_protect2 = 0u;
    /* Encode header mirroring into $5105. (mmc5.rs.) */
    uint8_t nt_mirroring;
    switch (mirroring) {
        case MIRROR_HORIZONTAL:      nt_mirroring = 0x44u; break;
        case MIRROR_VERTICAL:        nt_mirroring = 0x50u; break;
        case MIRROR_FOUR_SCREEN:     nt_mirroring = 0xE4u; break;
        case MIRROR_SINGLE_SCREEN_0: nt_mirroring = 0x00u; break;
        case MIRROR_SINGLE_SCREEN_1: nt_mirroring = 0x55u; break;
        case MIRROR_SINGLE_SCREEN_2: nt_mirroring = 0xAAu; break;
        case MIRROR_SINGLE_SCREEN_3: nt_mirroring = 0xFFu; break;
        default:                     nt_mirroring = 0x44u; break;
    }
    s->nt_mirroring = nt_mirroring;
    s->fill_tile = 0u;
    s->fill_attr = 0u;
    s->prg_ram_bank = 0u;
    memset(s->prg_banks, 0, sizeof(s->prg_banks));
    memset(s->chr_banks, 0, sizeof(s->chr_banks));
    memset(s->chr_banks_ex, 0, sizeof(s->chr_banks_ex));
    s->chr_upper = 0u;
    s->split_control = 0u;
    s->split_y_scroll = 0u;
    s->split_bank = 0u;
    s->irq_scanline = 0u;
    s->irq_control = 0u;
    s->scanline_counter = 0u;
    s->irq_pending = false;
    s->in_vblank = false;
    s->mult_a = 0u;
    s->mult_b = 0u;
    out->vt = &MMC5_VTABLE;
    out->state = s;
    out->mapper_num = 5u;
    return 0;
}
