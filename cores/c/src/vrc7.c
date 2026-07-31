/*
 * vrc7.c - Mapper 85 (VRC7 with YM2413 OPLL audio). Port of
 * src/mappers/vrc7.rs to C (M4.4).
 *
 * Konami VRC7 ASIC: flexible PRG/CHR banking, CPU-clocked IRQ timer, and
 * on-chip YM2413 OPLL FM synthesiser (9 voices, 6 exposed). Register decode
 * uses A0,A2,A3,A4,A5,A12-A15 ? effective index `addr & 0xF03D`.
 *
 * See: https://www.nesdev.org/wiki/VRC7
 */
#include "mapper.h"
#include "cartridge.h"
#include "opll.h"
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#define PRG_8K_SIZE   8192u
#define CHR_1K_SIZE   1024u
#define PRG_RAM_SIZE  8192u

typedef struct Vrc7State {
    uint8_t* prg_rom;
    uint32_t prg_size;
    uint8_t* chr;
    uint32_t chr_size;
    bool     chr_is_ram;
    uint8_t  prg_ram[PRG_RAM_SIZE];
    bool     has_battery;
    uint8_t  prg_banks[3];
    uint8_t  chr_banks[8];
    bool     mirror_horizontal;
    uint16_t irq_latch;
    uint16_t irq_counter;
    bool     irq_latch_high;
    bool     irq_enable;
    bool     irq_pending;
    Opll     opll;
} Vrc7State;

static uint32_t vrc7_prg_8k_count(const Vrc7State* s) {
    uint32_t n = s->prg_size / PRG_8K_SIZE;
    return n > 0u ? n : 1u;
}
static uint32_t vrc7_chr_1k_count(const Vrc7State* s) {
    uint32_t n = s->chr_size / CHR_1K_SIZE;
    return n > 0u ? n : 1u;
}

static uint8_t vrc7_prg_read_bank(const Vrc7State* s, uint32_t bank, uint32_t offset) {
    uint32_t count = vrc7_prg_8k_count(s);
    bank = bank % count;
    uint32_t idx = bank * PRG_8K_SIZE + offset;
    if (idx >= s->prg_size) return 0x00u;
    return s->prg_rom[idx];
}

static uint8_t vrc7_read_prg(const void* state, uint16_t addr) {
    const Vrc7State* s = (const Vrc7State*)state;
    if (addr >= 0x6000u && addr < 0x8000u) {
        uint32_t idx = (uint32_t)(addr - 0x6000u) & (PRG_RAM_SIZE - 1u);
        return s->prg_ram[idx];
    }
    uint32_t local = (uint32_t)(addr - 0x8000u);
    uint32_t slot = local / PRG_8K_SIZE;
    uint32_t offset = local & (PRG_8K_SIZE - 1u);
    if (slot < 3u) {
        uint32_t bank = (uint32_t)s->prg_banks[slot] % vrc7_prg_8k_count(s);
        return vrc7_prg_read_bank(s, bank, offset);
    }
    uint32_t last = vrc7_prg_8k_count(s) - 1u;
    return vrc7_prg_read_bank(s, last, offset);
}

static void vrc7_write_prg(void* state, uint16_t addr, uint8_t value) {
    Vrc7State* s = (Vrc7State*)state;
    if (addr >= 0x6000u && addr < 0x8000u) {
        uint32_t idx = (uint32_t)(addr - 0x6000u) & (PRG_RAM_SIZE - 1u);
        s->prg_ram[idx] = value;
        return;
    }
    uint16_t reg = (uint16_t)(addr & 0xF03Du);
    switch (reg) {
        case 0x8000u: s->prg_banks[0] = (uint8_t)(value & 0x3Fu); break;
        case 0x8008u: s->prg_banks[1] = (uint8_t)(value & 0x3Fu); break;
        case 0x9000u: s->prg_banks[2] = (uint8_t)(value & 0x3Fu); break;
        case 0x9010u: opll_write_addr(&s->opll, value); break;
        case 0x9030u: opll_write_data(&s->opll, value); break;
        case 0xB000u: s->mirror_horizontal = (value & 0x01u) != 0u; break;
        case 0xC000u: s->chr_banks[0] = value; break;
        case 0xC004u: s->chr_banks[1] = value; break;
        case 0xC008u: s->chr_banks[2] = value; break;
        case 0xC00Cu: s->chr_banks[3] = value; break;
        case 0xD000u: s->chr_banks[4] = value; break;
        case 0xD004u: s->chr_banks[5] = value; break;
        case 0xD008u: s->chr_banks[6] = value; break;
        case 0xD00Cu: s->chr_banks[7] = value; break;
        case 0xE000u:
            if (!s->irq_latch_high) {
                s->irq_latch = (uint16_t)((s->irq_latch & 0xFF00u) | value);
                s->irq_latch_high = true;
            } else {
                s->irq_latch = (uint16_t)((s->irq_latch & 0x00FFu) | ((uint16_t)value << 8));
                s->irq_latch_high = false;
            }
            break;
        case 0xE008u:
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
        case 0xE010u: s->irq_pending = false; break;
        default: break;
    }
}

static uint8_t vrc7_read_chr(const void* state, uint16_t addr) {
    const Vrc7State* s = (const Vrc7State*)state;
    uint32_t slot = (uint32_t)addr / CHR_1K_SIZE;
    uint32_t offset = (uint32_t)addr & (CHR_1K_SIZE - 1u);
    uint32_t count = vrc7_chr_1k_count(s);
    uint32_t bank = (uint32_t)s->chr_banks[slot] % count;
    uint32_t idx = bank * CHR_1K_SIZE + offset;
    if (idx >= s->chr_size) return 0x00u;
    return s->chr[idx];
}

static void vrc7_write_chr(void* state, uint16_t addr, uint8_t value) {
    Vrc7State* s = (Vrc7State*)state;
    if (!s->chr_is_ram) return;
    uint32_t slot = (uint32_t)addr / CHR_1K_SIZE;
    uint32_t offset = (uint32_t)addr & (CHR_1K_SIZE - 1u);
    uint32_t count = vrc7_chr_1k_count(s);
    uint32_t bank = (uint32_t)s->chr_banks[slot] % count;
    uint32_t idx = bank * CHR_1K_SIZE + offset;
    if (idx < s->chr_size) s->chr[idx] = value;
}

static Mirroring vrc7_mirror_mode(const void* state) {
    const Vrc7State* s = (const Vrc7State*)state;
    return s->mirror_horizontal ? MIRROR_HORIZONTAL : MIRROR_VERTICAL;
}

static bool vrc7_chr_is_ram(const void* state) {
    return ((const Vrc7State*)state)->chr_is_ram;
}

static bool vrc7_has_battery(const void* state) {
    return ((const Vrc7State*)state)->has_battery;
}

static bool vrc7_irq_pending(const void* state) {
    return ((const Vrc7State*)state)->irq_pending;
}

static void vrc7_clock_cpu(void* state, uint32_t cpu_cycles) {
    Vrc7State* s = (Vrc7State*)state;
    for (uint32_t i = 0u; i < cpu_cycles; ++i) {
        if (s->irq_counter == 0u) {
            s->irq_counter = s->irq_latch;
            if (s->irq_enable) s->irq_pending = true;
        } else {
            s->irq_counter = (uint16_t)(s->irq_counter - 1u);
        }
    }
    opll_clock(&s->opll, cpu_cycles / 2u);
}

static float vrc7_expansion_audio_sample(const void* state) {
    const Vrc7State* s = (const Vrc7State*)state;
    const float VRC7_GAIN = 0.6f;
    return opll_sample(&s->opll) * VRC7_GAIN;
}

static void vrc7_destroy(void* state) {
    Vrc7State* s = (Vrc7State*)state;
    if (s) {
        free(s->prg_rom);
        free(s->chr);
        free(s);
    }
}

/* ---- Save / load state (M4.5) ---------------------------------------- */

/* Layout: [sizeof(Vrc7State)] struct (includes embedded prg_ram[8192] and
 * the Opll audio state) + [chr_size] CHR (only if chr_is_ram). */
static size_t vrc7_save_state(const void* state, uint8_t* buf) {
    const Vrc7State* s = (const Vrc7State*)state;
    size_t total = sizeof(Vrc7State);
    if (s->chr_is_ram) {
        total += s->chr_size;
    }
    if (buf) {
        memcpy(buf, s, sizeof(Vrc7State));
        if (s->chr_is_ram && s->chr_size > 0u) {
            memcpy(buf + sizeof(Vrc7State), s->chr, s->chr_size);
        }
    }
    return total;
}

static bool vrc7_load_state(void* state, const uint8_t* buf, size_t len) {
    Vrc7State* s = (Vrc7State*)state;
    size_t need = sizeof(Vrc7State);
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
    memcpy(s, buf, sizeof(Vrc7State));
    s->prg_rom = valid_prg;
    s->prg_size = valid_prg_size;
    s->chr = valid_chr;
    s->chr_size = valid_chr_size;
    s->chr_is_ram = valid_chr_is_ram;
    if (s->chr_is_ram && s->chr_size > 0u) {
        memcpy(s->chr, buf + sizeof(Vrc7State), s->chr_size);
    }
    return true;
}

static const MapperVTable VRC7_VTABLE = {
    vrc7_read_prg, NULL, vrc7_write_prg,
    vrc7_read_chr, NULL, vrc7_write_chr,
    vrc7_mirror_mode, vrc7_chr_is_ram, vrc7_has_battery,
    vrc7_irq_pending, NULL, NULL, vrc7_clock_cpu,
    vrc7_expansion_audio_sample, vrc7_destroy,
    vrc7_save_state, vrc7_load_state
};

int vrc7_create(const uint8_t* prg, uint32_t prg_size,
                const uint8_t* chr, uint32_t chr_size,
                Mirroring mirroring, bool has_battery, Mapper* out) {
    Vrc7State* s = (Vrc7State*)malloc(sizeof(Vrc7State));
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
    memset(s->prg_banks, 0, sizeof(s->prg_banks));
    memset(s->chr_banks, 0, sizeof(s->chr_banks));
    s->mirror_horizontal = (mirroring == MIRROR_HORIZONTAL);
    s->irq_latch = 0u;
    s->irq_counter = 0u;
    s->irq_latch_high = false;
    s->irq_enable = false;
    s->irq_pending = false;
    opll_init(&s->opll);
    out->vt = &VRC7_VTABLE;
    out->state = s;
    out->mapper_num = 85u;
    return 0;
}
