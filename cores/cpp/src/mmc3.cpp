/*
 * mmc3.c - Mapper 4 (MMC3). Port of src/mappers/mmc3.rs to C (M4.4).
 *
 * Nintendo's most popular mapper. PRG-ROM banking (8 KB switchable + fixed
 * 16 KB top window), CHR banking (1 KB and 2 KB banks with mode swap), 8 KB
 * PRG-RAM at $6000-$7FFF (gated by $A001), switchable H/V mirroring via
 * $A000, and an IRQ counter clocked by the PPU A12 rising edge for raster
 * effects.
 *
 * See: https://www.nesdev.org/wiki/MMC3
 */
#include "mapper.hpp"
#include "cartridge.hpp"
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#define PRG_BANK_SIZE 8192u
#define CHR_1K_SIZE   1024u
#define CHR_2K_SIZE   2048u
#define PRG_RAM_SIZE  8192u

typedef struct Mmc3State {
    uint8_t* prg_rom;
    uint32_t prg_size;
    uint8_t* chr;
    uint32_t chr_size;
    bool     chr_is_ram;
    uint8_t  prg_ram[PRG_RAM_SIZE];
    bool     has_battery;
    uint8_t  bank_select;
    uint8_t  bank_values[8];
    Mirroring mirroring;
    bool     prg_ram_enable;
    bool     prg_ram_write_protect;
    uint8_t  irq_latch;
    uint8_t  irq_counter;
    bool     irq_reload_flag;
    bool     irq_enable;
    bool     irq_pending;
} Mmc3State;

static uint32_t mmc3_prg_bank_count(const Mmc3State* s) {
    uint32_t n = s->prg_size / PRG_BANK_SIZE;
    return n > 0u ? n : 1u;
}

static uint32_t mmc3_chr_1k_count(const Mmc3State* s) {
    uint32_t n = s->chr_size / CHR_1K_SIZE;
    return n > 0u ? n : 1u;
}

static uint8_t mmc3_prg_mode(const Mmc3State* s) {
    return (uint8_t)((s->bank_select >> 6) & 1u);
}

static uint8_t mmc3_chr_mode(const Mmc3State* s) {
    return (uint8_t)((s->bank_select >> 7) & 1u);
}

static uint8_t mmc3_prg_read_bank(const Mmc3State* s, uint32_t bank, uint32_t offset) {
    uint32_t count = mmc3_prg_bank_count(s);
    bank = bank % count;
    uint32_t idx = bank * PRG_BANK_SIZE + offset;
    if (idx >= s->prg_size) {
        return 0x00u;
    }
    return s->prg_rom[idx];
}

static uint32_t mmc3_chr_bank_for_slot(const Mmc3State* s, uint32_t slot) {
    const uint8_t* r = s->bank_values;
    uint8_t cm = mmc3_chr_mode(s);
    if (cm == 0u) {
        switch (slot) {
            case 0: return (uint32_t)(r[0] & 0xFEu);
            case 1: return (uint32_t)(r[0] & 0xFEu) + 1u;
            case 2: return (uint32_t)(r[1] & 0xFEu);
            case 3: return (uint32_t)(r[1] & 0xFEu) + 1u;
            case 4: return (uint32_t)r[2];
            case 5: return (uint32_t)r[3];
            case 6: return (uint32_t)r[4];
            case 7: return (uint32_t)r[5];
            default: return 0u;
        }
    } else {
        switch (slot) {
            case 0: return (uint32_t)r[2];
            case 1: return (uint32_t)r[3];
            case 2: return (uint32_t)r[4];
            case 3: return (uint32_t)r[5];
            case 4: return (uint32_t)(r[0] & 0xFEu);
            case 5: return (uint32_t)(r[0] & 0xFEu) + 1u;
            case 6: return (uint32_t)(r[1] & 0xFEu);
            case 7: return (uint32_t)(r[1] & 0xFEu) + 1u;
            default: return 0u;
        }
    }
}

static uint32_t mmc3_chr_index(const Mmc3State* s, uint16_t addr) {
    uint32_t a = (uint32_t)addr;
    uint32_t slot = a / CHR_1K_SIZE;
    uint32_t offset = a & (CHR_1K_SIZE - 1u);
    uint32_t bank = mmc3_chr_bank_for_slot(s, slot);
    uint32_t count = mmc3_chr_1k_count(s);
    return (bank % count) * CHR_1K_SIZE + offset;
}

static uint8_t mmc3_read_prg(const void* state, uint16_t addr) {
    const Mmc3State* s = (const Mmc3State*)state;
    if (addr >= 0x6000u && addr < 0x8000u) {
        if (s->prg_ram_enable) {
            uint32_t idx = (uint32_t)(addr - 0x6000u) & (PRG_RAM_SIZE - 1u);
            return s->prg_ram[idx];
        }
        return 0x00u;
    }
    uint32_t local = (uint32_t)(addr - 0x8000u);
    uint32_t slot = local / PRG_BANK_SIZE;
    uint32_t offset = local & (PRG_BANK_SIZE - 1u);
    uint32_t count = mmc3_prg_bank_count(s);
    uint32_t last = count - 1u;
    uint32_t second_last = count >= 2u ? count - 2u : 0u;
    uint32_t bank;
    uint8_t pm = mmc3_prg_mode(s);
    if (slot == 0u) {
        bank = (pm == 0u) ? ((uint32_t)s->bank_values[6] % count) : second_last;
    } else if (slot == 1u) {
        bank = (uint32_t)s->bank_values[7] % count;
    } else if (slot == 2u) {
        bank = (pm == 0u) ? second_last : ((uint32_t)s->bank_values[6] % count);
    } else { /* slot == 3 */
        bank = last;
    }
    return mmc3_prg_read_bank(s, bank, offset);
}

static void mmc3_write_prg(void* state, uint16_t addr, uint8_t value) {
    Mmc3State* s = (Mmc3State*)state;
    if (addr >= 0x6000u && addr < 0x8000u) {
        if (s->prg_ram_enable && !s->prg_ram_write_protect) {
            uint32_t idx = (uint32_t)(addr - 0x6000u) & (PRG_RAM_SIZE - 1u);
            s->prg_ram[idx] = value;
        }
        return;
    }
    if (addr >= 0x8000u && addr <= 0x9FFFu) {
        if ((addr & 1u) == 0u) {
            s->bank_select = value;
        } else {
            uint32_t reg = (uint32_t)(s->bank_select & 0x07u);
            s->bank_values[reg] = value;
        }
    } else if (addr >= 0xA000u && addr <= 0xBFFFu) {
        if ((addr & 1u) == 0u) {
            s->mirroring = (value & 1u) == 0u ? MIRROR_VERTICAL : MIRROR_HORIZONTAL;
        } else {
            s->prg_ram_enable = (value & 0x80u) != 0u;
            s->prg_ram_write_protect = (value & 0x40u) != 0u;
        }
    } else if (addr >= 0xC000u && addr <= 0xDFFFu) {
        if ((addr & 1u) == 0u) {
            s->irq_latch = value;
        } else {
            s->irq_reload_flag = true;
        }
    } else if (addr >= 0xE000u && addr <= 0xFFFFu) {
        if ((addr & 1u) == 0u) {
            s->irq_enable = false;
            s->irq_pending = false;
        } else {
            s->irq_enable = true;
        }
    }
}

static uint8_t mmc3_read_chr(const void* state, uint16_t addr) {
    const Mmc3State* s = (const Mmc3State*)state;
    uint32_t idx = mmc3_chr_index(s, addr);
    if (idx >= s->chr_size) {
        return 0x00u;
    }
    return s->chr[idx];
}

static void mmc3_write_chr(void* state, uint16_t addr, uint8_t value) {
    Mmc3State* s = (Mmc3State*)state;
    if (!s->chr_is_ram) {
        return;
    }
    uint32_t idx = mmc3_chr_index(s, addr);
    if (idx < s->chr_size) {
        s->chr[idx] = value;
    }
}

static Mirroring mmc3_mirror_mode(const void* state) {
    return ((const Mmc3State*)state)->mirroring;
}

static bool mmc3_chr_is_ram(const void* state) {
    return ((const Mmc3State*)state)->chr_is_ram;
}

static bool mmc3_has_battery(const void* state) {
    return ((const Mmc3State*)state)->has_battery;
}

static bool mmc3_irq_pending(const void* state) {
    return ((const Mmc3State*)state)->irq_pending;
}

static void mmc3_clock_irq(void* state) {
    Mmc3State* s = (Mmc3State*)state;
    if (s->irq_reload_flag) {
        s->irq_counter = s->irq_latch;
        s->irq_reload_flag = false;
    } else if (s->irq_counter == 0u) {
        s->irq_counter = s->irq_latch;
        if (s->irq_enable) {
            s->irq_pending = true;
        }
    } else {
        s->irq_counter = (uint8_t)(s->irq_counter - 1u);
    }
}

static void mmc3_destroy(void* state) {
    Mmc3State* s = (Mmc3State*)state;
    if (s) {
        free(s->prg_rom);
        free(s->chr);
        free(s);
    }
}

/* ---- Save / load state (M4.5) ---------------------------------------- */

/* Layout: [sizeof(Mmc3State)] struct (includes embedded prg_ram[8192]) +
 * [chr_size] CHR (only if chr_is_ram). */
static size_t mmc3_save_state(const void* state, uint8_t* buf) {
    const Mmc3State* s = (const Mmc3State*)state;
    size_t total = sizeof(Mmc3State);
    if (s->chr_is_ram) {
        total += s->chr_size;
    }
    if (buf) {
        memcpy(buf, s, sizeof(Mmc3State));
        if (s->chr_is_ram && s->chr_size > 0u) {
            memcpy(buf + sizeof(Mmc3State), s->chr, s->chr_size);
        }
    }
    return total;
}

static bool mmc3_load_state(void* state, const uint8_t* buf, size_t len) {
    Mmc3State* s = (Mmc3State*)state;
    size_t need = sizeof(Mmc3State);
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
    memcpy(s, buf, sizeof(Mmc3State));
    s->prg_rom = valid_prg;
    s->prg_size = valid_prg_size;
    s->chr = valid_chr;
    s->chr_size = valid_chr_size;
    s->chr_is_ram = valid_chr_is_ram;
    if (s->chr_is_ram && s->chr_size > 0u) {
        memcpy(s->chr, buf + sizeof(Mmc3State), s->chr_size);
    }
    return true;
}

static const MapperVTable MMC3_VTABLE = {
    mmc3_read_prg, NULL, mmc3_write_prg,
    mmc3_read_chr, NULL, mmc3_write_chr,
    mmc3_mirror_mode, mmc3_chr_is_ram, mmc3_has_battery,
    mmc3_irq_pending, mmc3_clock_irq, NULL, NULL, NULL, mmc3_destroy,
    mmc3_save_state, mmc3_load_state
};

int mmc3_create(const uint8_t* prg, uint32_t prg_size,
                const uint8_t* chr, uint32_t chr_size,
                Mirroring mirroring, bool has_battery, Mapper* out) {
    Mmc3State* s = (Mmc3State*)malloc(sizeof(Mmc3State));
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
    if (chr_size == 0u) {
        s->chr_size = CHR_2K_SIZE * 4u; /* 8 KB CHR-RAM default. */
        s->chr = (uint8_t*)malloc(s->chr_size);
        if (!s->chr) {
            free(s->prg_rom);
            free(s);
            return 1;
        }
        memset(s->chr, 0, s->chr_size);
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
    memset(s->prg_ram, 0, PRG_RAM_SIZE);
    s->has_battery = has_battery;
    s->bank_select = 0u;
    memset(s->bank_values, 0, sizeof(s->bank_values));
    s->mirroring = mirroring;
    s->prg_ram_enable = false;
    s->prg_ram_write_protect = false;
    s->irq_latch = 0u;
    s->irq_counter = 0u;
    s->irq_reload_flag = false;
    s->irq_enable = false;
    s->irq_pending = false;
    out->vt = &MMC3_VTABLE;
    out->state = s;
    out->mapper_num = 4u;
    return 0;
}
