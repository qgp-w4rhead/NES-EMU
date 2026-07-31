/*
 * fme7.c - Mapper 69 (FME-7 / Sunsoft 5B). Port of src/mappers/fme7.rs to C
 * (M4.4).
 *
 * Sunsoft's mapper: flexible PRG/CHR banking, PRG-RAM, switchable mirroring,
 * 16-bit CPU-clocked IRQ timer, and on-chip YM2149 PSG (Sunsoft 5B audio).
 * Uses a command/data latch pair at $8000/$8001.
 *
 * See: https://www.nesdev.org/wiki/FME-7
 */
#include "mapper.h"
#include "cartridge.h"
#include "ym2149.h"
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#define PRG_BANK_SIZE 8192u
#define CHR_1K_SIZE   1024u
#define PRG_RAM_SIZE  8192u

typedef struct Fme7State {
    uint8_t* prg_rom;
    uint32_t prg_size;
    uint8_t* chr;
    uint32_t chr_size;
    bool     chr_is_ram;
    uint8_t  prg_ram[PRG_RAM_SIZE];
    bool     has_battery;
    uint8_t  command;
    uint8_t  prg_banks[4];
    uint8_t  chr_banks[8];
    bool     mirror_horizontal;
    bool     prg_ram_enable;
    bool     prg_ram_write_protect;
    uint16_t irq_latch;
    uint16_t irq_counter;
    bool     irq_latch_high;
    bool     irq_enable;
    bool     irq_pending;
    Ym2149   ym2149;
} Fme7State;

static uint32_t fme7_prg_bank_count(const Fme7State* s) {
    uint32_t n = s->prg_size / PRG_BANK_SIZE;
    return n > 0u ? n : 1u;
}
static uint32_t fme7_chr_1k_count(const Fme7State* s) {
    uint32_t n = s->chr_size / CHR_1K_SIZE;
    return n > 0u ? n : 1u;
}

static uint8_t fme7_prg_read_bank(const Fme7State* s, uint32_t bank, uint32_t offset) {
    uint32_t count = fme7_prg_bank_count(s);
    bank = bank % count;
    uint32_t idx = bank * PRG_BANK_SIZE + offset;
    if (idx >= s->prg_size) return 0x00u;
    return s->prg_rom[idx];
}

static void fme7_write_data(Fme7State* s, uint8_t value) {
    uint8_t cmd = (uint8_t)(s->command & 0x0Fu);
    if (cmd <= 7u) {
        s->chr_banks[cmd & 0x07u] = value;
    } else {
        switch (cmd) {
            case 8u:  s->prg_banks[0] = (uint8_t)(value & 0x3Fu); break;
            case 9u:  s->prg_banks[1] = (uint8_t)(value & 0x3Fu); break;
            case 10u: s->prg_banks[2] = (uint8_t)(value & 0x3Fu); break;
            case 11u: s->prg_banks[3] = (uint8_t)(value & 0x3Fu); break;
            case 12u: s->mirror_horizontal = (value & 0x01u) != 0u; break;
            case 13u:
                s->prg_ram_enable = (value & 0x80u) != 0u;
                s->prg_ram_write_protect = (value & 0x40u) != 0u;
                break;
            case 14u:
                if (!s->irq_latch_high) {
                    s->irq_latch = (uint16_t)((s->irq_latch & 0xFF00u) | value);
                    s->irq_latch_high = true;
                } else {
                    s->irq_latch = (uint16_t)((s->irq_latch & 0x00FFu) | ((uint16_t)value << 8));
                    s->irq_latch_high = false;
                }
                break;
            case 15u:
                if (value & 0x02u) {
                    s->irq_enable = true;
                    s->irq_pending = false;
                    s->irq_counter = s->irq_latch;
                } else if (value & 0x01u) {
                    s->irq_enable = true;
                    s->irq_counter = s->irq_latch;
                } else {
                    s->irq_enable = false;
                }
                break;
            default: break;
        }
    }
}

static uint8_t fme7_read_prg(const void* state, uint16_t addr) {
    const Fme7State* s = (const Fme7State*)state;
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
    uint32_t bank = (uint32_t)s->prg_banks[slot] % fme7_prg_bank_count(s);
    return fme7_prg_read_bank(s, bank, offset);
}

static void fme7_write_prg(void* state, uint16_t addr, uint8_t value) {
    Fme7State* s = (Fme7State*)state;
    if (addr >= 0x6000u && addr < 0x8000u) {
        if (s->prg_ram_enable && !s->prg_ram_write_protect) {
            uint32_t idx = (uint32_t)(addr - 0x6000u) & (PRG_RAM_SIZE - 1u);
            s->prg_ram[idx] = value;
        }
        return;
    }
    switch (addr) {
        case 0x8000u: s->command = value; break;
        case 0x8001u: fme7_write_data(s, value); break;
        case 0xC000u: ym2149_write_addr(&s->ym2149, value); break;
        case 0xE000u: ym2149_write_data(&s->ym2149, value); break;
        default: break;
    }
}

static uint8_t fme7_read_chr(const void* state, uint16_t addr) {
    const Fme7State* s = (const Fme7State*)state;
    uint32_t slot = (uint32_t)addr / CHR_1K_SIZE;
    uint32_t offset = (uint32_t)addr & (CHR_1K_SIZE - 1u);
    uint32_t count = fme7_chr_1k_count(s);
    uint32_t bank = (uint32_t)s->chr_banks[slot] % count;
    uint32_t idx = bank * CHR_1K_SIZE + offset;
    if (idx >= s->chr_size) return 0x00u;
    return s->chr[idx];
}

static void fme7_write_chr(void* state, uint16_t addr, uint8_t value) {
    Fme7State* s = (Fme7State*)state;
    if (!s->chr_is_ram) return;
    uint32_t slot = (uint32_t)addr / CHR_1K_SIZE;
    uint32_t offset = (uint32_t)addr & (CHR_1K_SIZE - 1u);
    uint32_t count = fme7_chr_1k_count(s);
    uint32_t bank = (uint32_t)s->chr_banks[slot] % count;
    uint32_t idx = bank * CHR_1K_SIZE + offset;
    if (idx < s->chr_size) s->chr[idx] = value;
}

static Mirroring fme7_mirror_mode(const void* state) {
    const Fme7State* s = (const Fme7State*)state;
    return s->mirror_horizontal ? MIRROR_HORIZONTAL : MIRROR_VERTICAL;
}

static bool fme7_chr_is_ram(const void* state) {
    return ((const Fme7State*)state)->chr_is_ram;
}

static bool fme7_has_battery(const void* state) {
    return ((const Fme7State*)state)->has_battery;
}

static bool fme7_irq_pending(const void* state) {
    return ((const Fme7State*)state)->irq_pending;
}

static void fme7_clock_cpu(void* state, uint32_t cpu_cycles) {
    Fme7State* s = (Fme7State*)state;
    for (uint32_t i = 0u; i < cpu_cycles; ++i) {
        if (s->irq_counter == 0u) {
            s->irq_counter = s->irq_latch;
            if (s->irq_enable) {
                s->irq_pending = true;
            }
        } else {
            s->irq_counter = (uint16_t)(s->irq_counter - 1u);
        }
    }
    ym2149_clock(&s->ym2149, cpu_cycles / 2u);
}

static float fme7_expansion_audio_sample(const void* state) {
    const Fme7State* s = (const Fme7State*)state;
    const float S5B_GAIN = 0.5f;
    return ym2149_sample(&s->ym2149) * S5B_GAIN;
}

static void fme7_destroy(void* state) {
    Fme7State* s = (Fme7State*)state;
    if (s) {
        free(s->prg_rom);
        free(s->chr);
        free(s);
    }
}

static const MapperVTable FME7_VTABLE = {
    fme7_read_prg, NULL, fme7_write_prg,
    fme7_read_chr, NULL, fme7_write_chr,
    fme7_mirror_mode, fme7_chr_is_ram, fme7_has_battery,
    fme7_irq_pending, NULL, NULL, fme7_clock_cpu,
    fme7_expansion_audio_sample, fme7_destroy
};

int fme7_create(const uint8_t* prg, uint32_t prg_size,
                const uint8_t* chr, uint32_t chr_size,
                Mirroring mirroring, bool has_battery, Mapper* out) {
    Fme7State* s = (Fme7State*)malloc(sizeof(Fme7State));
    if (!s) return 1;
    s->prg_size = prg_size;
    s->prg_rom = NULL;
    if (prg_size > 0u) {
        s->prg_rom = (uint8_t*)malloc(prg_size);
        if (!s->prg_rom) { free(s); return 1; }
        memcpy(s->prg_rom, prg, prg_size);
    }
    if (chr_size > 0u) {
        s->chr_size = chr_size;
        s->chr = (uint8_t*)malloc(chr_size);
        if (!s->chr) { free(s->prg_rom); free(s); return 1; }
        memcpy(s->chr, chr, chr_size);
        s->chr_is_ram = false;
    } else {
        s->chr_size = 8192u;
        s->chr = (uint8_t*)malloc(s->chr_size);
        if (!s->chr) { free(s->prg_rom); free(s); return 1; }
        memset(s->chr, 0, s->chr_size);
        s->chr_is_ram = true;
    }
    memset(s->prg_ram, 0, PRG_RAM_SIZE);
    s->has_battery = has_battery;
    s->command = 0u;
    memset(s->prg_banks, 0, sizeof(s->prg_banks));
    memset(s->chr_banks, 0, sizeof(s->chr_banks));
    s->mirror_horizontal = (mirroring == MIRROR_HORIZONTAL);
    s->prg_ram_enable = false;
    s->prg_ram_write_protect = false;
    s->irq_latch = 0u;
    s->irq_counter = 0u;
    s->irq_latch_high = false;
    s->irq_enable = false;
    s->irq_pending = false;
    ym2149_init(&s->ym2149);
    out->vt = &FME7_VTABLE;
    out->state = s;
    out->mapper_num = 69u;
    return 0;
}
