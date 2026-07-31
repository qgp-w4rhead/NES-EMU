/*
 * vrc6.c - Mappers 24 (VRC6a) and 26 (VRC6b). Port of src/mappers/vrc6.rs
 * to C (M4.4).
 *
 * Konami's VRC6 ASIC: three expansion audio channels (two pulses + sawtooth),
 * PRG/CHR banking, and a CPU-clocked IRQ timer. Mapper 26 swaps A0/A1
 * register decode. $E000-$FFFF is fixed to the last 8 KB PRG bank; $6000-$7FFF
 * is 8 KB PRG-RAM (optionally battery-backed).
 *
 * See: https://www.nesdev.org/wiki/VRC6 and VRC6_audio
 */
#include "mapper.hpp"
#include "cartridge.hpp"
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#define PRG_16K_SIZE   16384u
#define PRG_8K_SIZE    8192u
#define CHR_1K_SIZE    1024u
#define PRG_RAM_SIZE   8192u

typedef struct PulseChannel {
    uint8_t  control;  /* $x000: duty/volume. */
    uint16_t period;   /* 12-bit. */
    uint16_t timer;
    uint8_t  step;     /* 0-15. */
    bool     enabled;
    uint8_t  scale;    /* $9003 frequency scaling (channel 1 only). */
} PulseChannel;

typedef struct SawChannel {
    uint8_t  rate;     /* $B000: accumulator rate. */
    uint16_t period;   /* 12-bit. */
    uint16_t timer;
    uint8_t  accum;    /* 7-bit phase accumulator. */
    bool     enabled;
} SawChannel;

typedef struct Vrc6State {
    uint8_t* prg_rom;
    uint32_t prg_size;
    uint8_t* chr;
    uint32_t chr_size;
    bool     chr_is_ram;
    uint8_t  prg_ram[PRG_RAM_SIZE];
    bool     has_battery;
    uint8_t  prg_bank_16k;
    uint8_t  prg_bank_8k;
    uint8_t  chr_banks[8];
    bool     mirror_horizontal;
    uint8_t  irq_latch;
    uint8_t  irq_counter;
    bool     irq_enable;
    bool     irq_enable_after_ack;
    bool     irq_pending;
    PulseChannel pulse1;
    PulseChannel pulse2;
    SawChannel   saw;
    bool     swap_addr;
} Vrc6State;

static uint32_t vrc6_prg_16k_count(const Vrc6State* s) {
    uint32_t n = s->prg_size / PRG_16K_SIZE;
    return n > 0u ? n : 1u;
}
static uint32_t vrc6_prg_8k_count(const Vrc6State* s) {
    uint32_t n = s->prg_size / PRG_8K_SIZE;
    return n > 0u ? n : 1u;
}
static uint32_t vrc6_chr_1k_count(const Vrc6State* s) {
    uint32_t n = s->chr_size / CHR_1K_SIZE;
    return n > 0u ? n : 1u;
}

static uint16_t vrc6_decode(const Vrc6State* s, uint16_t addr) {
    uint16_t masked = (uint16_t)(addr & 0xF003u);
    if (!s->swap_addr) {
        return masked;
    }
    uint16_t a0 = (uint16_t)((masked & 0x001u) << 1);
    uint16_t a1 = (uint16_t)((masked & 0x002u) >> 1);
    return (uint16_t)((masked & 0xFFFCu) | a0 | a1);
}

static uint8_t vrc6_prg_read_8k(const Vrc6State* s, uint32_t bank, uint32_t offset) {
    uint32_t count = vrc6_prg_8k_count(s);
    bank = bank % count;
    uint32_t idx = bank * PRG_8K_SIZE + offset;
    if (idx >= s->prg_size) return 0x00u;
    return s->prg_rom[idx];
}

static uint8_t vrc6_prg_read_16k(const Vrc6State* s, uint32_t bank, uint32_t offset) {
    uint32_t count = vrc6_prg_16k_count(s);
    bank = bank % count;
    uint32_t idx = bank * PRG_16K_SIZE + offset;
    if (idx >= s->prg_size) return 0x00u;
    return s->prg_rom[idx];
}

static uint8_t vrc6_pulse_duty(const PulseChannel* p) {
    return (uint8_t)(((p->control >> 4) & 0x07u) + 1u);
}
static uint8_t vrc6_pulse_volume(const PulseChannel* p) {
    return (uint8_t)(p->control & 0x0Fu);
}

static void vrc6_pulse_clock(PulseChannel* p, uint32_t cpu_cycles) {
    if (!p->enabled) return;
    uint16_t period = p->period;
    if (p->scale & 1u) {
        period = (uint16_t)(period * 2u);
        if (period == 0u) period = 1u;
    }
    if (period == 0u) period = 1u;
    for (uint32_t i = 0u; i < cpu_cycles; ++i) {
        if (p->timer == 0u) {
            p->timer = period;
            p->step = (uint8_t)((p->step + 1u) & 0x0Fu);
        } else {
            p->timer = (uint16_t)(p->timer - 1u);
        }
    }
}

static void vrc6_saw_clock(SawChannel* saw, uint32_t cpu_cycles) {
    if (!saw->enabled) return;
    uint16_t period = saw->period;
    if (period == 0u) period = 1u;
    for (uint32_t i = 0u; i < cpu_cycles; ++i) {
        if (saw->timer == 0u) {
            saw->timer = period;
            saw->accum = (uint8_t)(saw->accum + (saw->rate & 0x3Fu));
            if (saw->accum >= 0x80u) saw->accum = 0u;
        } else {
            saw->timer = (uint16_t)(saw->timer - 1u);
        }
    }
}

static uint8_t vrc6_pulse1_sample(const Vrc6State* s) {
    if (s->pulse1.enabled && s->pulse1.step < vrc6_pulse_duty(&s->pulse1)) {
        return vrc6_pulse_volume(&s->pulse1);
    }
    return 0u;
}
static uint8_t vrc6_pulse2_sample(const Vrc6State* s) {
    if (s->pulse2.enabled && s->pulse2.step < vrc6_pulse_duty(&s->pulse2)) {
        return vrc6_pulse_volume(&s->pulse2);
    }
    return 0u;
}
static uint8_t vrc6_saw_sample(const Vrc6State* s) {
    return s->saw.enabled ? (uint8_t)(s->saw.accum >> 2) : 0u;
}

static uint8_t vrc6_read_prg(const void* state, uint16_t addr) {
    const Vrc6State* s = (const Vrc6State*)state;
    if (addr >= 0x6000u && addr < 0x8000u) {
        uint32_t idx = (uint32_t)(addr - 0x6000u) & (PRG_RAM_SIZE - 1u);
        return s->prg_ram[idx];
    }
    uint32_t local = (uint32_t)(addr - 0x8000u);
    if (local < PRG_16K_SIZE) {
        uint32_t bank = (uint32_t)s->prg_bank_16k % vrc6_prg_16k_count(s);
        return vrc6_prg_read_16k(s, bank, local);
    }
    if (local < PRG_16K_SIZE + PRG_8K_SIZE) {
        uint32_t off = local - PRG_16K_SIZE;
        uint32_t bank = (uint32_t)s->prg_bank_8k % vrc6_prg_8k_count(s);
        return vrc6_prg_read_8k(s, bank, off);
    }
    uint32_t off = local - PRG_16K_SIZE - PRG_8K_SIZE;
    uint32_t last = vrc6_prg_8k_count(s) - 1u;
    return vrc6_prg_read_8k(s, last, off);
}

static void vrc6_write_prg(void* state, uint16_t addr, uint8_t value) {
    Vrc6State* s = (Vrc6State*)state;
    if (addr >= 0x6000u && addr < 0x8000u) {
        uint32_t idx = (uint32_t)(addr - 0x6000u) & (PRG_RAM_SIZE - 1u);
        s->prg_ram[idx] = value;
        return;
    }
    uint16_t reg = vrc6_decode(s, addr);
    switch (reg) {
        case 0x8000u: s->prg_bank_16k = (uint8_t)(value & 0x3Fu); break;
        case 0x9000u: s->pulse1.control = value; break;
        case 0x9001u: s->pulse1.period = (uint16_t)((s->pulse1.period & 0x0F00u) | value); break;
        case 0x9002u:
            s->pulse1.period = (uint16_t)((s->pulse1.period & 0x00FFu) | ((uint16_t)(value & 0x0Fu) << 8));
            s->pulse1.enabled = (value & 0x80u) != 0u;
            break;
        case 0x9003u: s->pulse1.scale = value; break;
        case 0xA000u: s->pulse2.control = value; break;
        case 0xA001u: s->pulse2.period = (uint16_t)((s->pulse2.period & 0x0F00u) | value); break;
        case 0xA002u:
            s->pulse2.period = (uint16_t)((s->pulse2.period & 0x00FFu) | ((uint16_t)(value & 0x0Fu) << 8));
            s->pulse2.enabled = (value & 0x80u) != 0u;
            break;
        case 0xB000u: s->saw.rate = value; break;
        case 0xB001u: s->saw.period = (uint16_t)((s->saw.period & 0x0F00u) | value); break;
        case 0xB002u:
            s->saw.period = (uint16_t)((s->saw.period & 0x00FFu) | ((uint16_t)(value & 0x0Fu) << 8));
            s->saw.enabled = (value & 0x80u) != 0u;
            break;
        case 0xB003u: s->mirror_horizontal = (value & 0x01u) != 0u; break;
        case 0xC000u: s->prg_bank_8k = (uint8_t)(value & 0x3Fu); break;
        case 0xD000u: s->chr_banks[0] = value; break;
        case 0xD001u: s->chr_banks[1] = value; break;
        case 0xD002u: s->chr_banks[2] = value; break;
        case 0xD003u: s->chr_banks[3] = value; break;
        case 0xE000u: s->chr_banks[4] = value; break;
        case 0xE001u: s->chr_banks[5] = value; break;
        case 0xE002u: s->chr_banks[6] = value; break;
        case 0xE003u: s->chr_banks[7] = value; break;
        case 0xF000u: s->irq_latch = value; break;
        case 0xF001u:
            s->irq_enable = (value & 0x01u) != 0u;
            s->irq_counter = s->irq_latch;
            break;
        case 0xF002u:
            s->irq_enable_after_ack = (value & 0x02u) != 0u;
            s->irq_enable = (value & 0x01u) != 0u;
            s->irq_pending = false;
            break;
        default: break;
    }
}

static uint8_t vrc6_read_chr(const void* state, uint16_t addr) {
    const Vrc6State* s = (const Vrc6State*)state;
    uint32_t slot = (uint32_t)addr / CHR_1K_SIZE;
    uint32_t offset = (uint32_t)addr & (CHR_1K_SIZE - 1u);
    uint32_t count = vrc6_chr_1k_count(s);
    uint32_t bank = (uint32_t)s->chr_banks[slot] % count;
    uint32_t idx = bank * CHR_1K_SIZE + offset;
    if (idx >= s->chr_size) return 0x00u;
    return s->chr[idx];
}

static void vrc6_write_chr(void* state, uint16_t addr, uint8_t value) {
    Vrc6State* s = (Vrc6State*)state;
    if (!s->chr_is_ram) return;
    uint32_t slot = (uint32_t)addr / CHR_1K_SIZE;
    uint32_t offset = (uint32_t)addr & (CHR_1K_SIZE - 1u);
    uint32_t count = vrc6_chr_1k_count(s);
    uint32_t bank = (uint32_t)s->chr_banks[slot] % count;
    uint32_t idx = bank * CHR_1K_SIZE + offset;
    if (idx < s->chr_size) s->chr[idx] = value;
}

static Mirroring vrc6_mirror_mode(const void* state) {
    const Vrc6State* s = (const Vrc6State*)state;
    return s->mirror_horizontal ? MIRROR_HORIZONTAL : MIRROR_VERTICAL;
}

static bool vrc6_chr_is_ram(const void* state) {
    return ((const Vrc6State*)state)->chr_is_ram;
}

static bool vrc6_has_battery(const void* state) {
    return ((const Vrc6State*)state)->has_battery;
}

static bool vrc6_irq_pending(const void* state) {
    return ((const Vrc6State*)state)->irq_pending;
}

static void vrc6_clock_cpu(void* state, uint32_t cpu_cycles) {
    Vrc6State* s = (Vrc6State*)state;
    for (uint32_t i = 0u; i < cpu_cycles; ++i) {
        if (s->irq_counter == 0u) {
            s->irq_counter = s->irq_latch;
            if (s->irq_enable) {
                s->irq_pending = true;
                if (!s->irq_enable_after_ack) {
                    s->irq_enable = false;
                }
            }
        } else {
            s->irq_counter = (uint8_t)(s->irq_counter - 1u);
        }
    }
    vrc6_pulse_clock(&s->pulse1, cpu_cycles);
    vrc6_pulse_clock(&s->pulse2, cpu_cycles);
    vrc6_saw_clock(&s->saw, cpu_cycles);
}

static float vrc6_expansion_audio_sample(const void* state) {
    const Vrc6State* s = (const Vrc6State*)state;
    uint32_t p1 = vrc6_pulse1_sample(s);
    uint32_t p2 = vrc6_pulse2_sample(s);
    uint32_t saw = vrc6_saw_sample(s);
    uint32_t sum = p1 + p2;
    sum = (sum > 63u) ? 63u : (sum + saw);
    if (sum > 63u) sum = 63u;
    const float VRC6_GAIN = 0.75f;
    float normalized = ((float)sum / 63.0f) * 2.0f - 1.0f;
    return normalized * VRC6_GAIN;
}

static void vrc6_destroy(void* state) {
    Vrc6State* s = (Vrc6State*)state;
    if (s) {
        free(s->prg_rom);
        free(s->chr);
        free(s);
    }
}

/* ---- Save / load state (M4.5) ---------------------------------------- */

/* Layout: [sizeof(Vrc6State)] struct (includes embedded prg_ram[8192] and
 * the PulseChannel/SawChannel audio state) + [chr_size] CHR (only if
 * chr_is_ram). */
static size_t vrc6_save_state(const void* state, uint8_t* buf) {
    const Vrc6State* s = (const Vrc6State*)state;
    size_t total = sizeof(Vrc6State);
    if (s->chr_is_ram) {
        total += s->chr_size;
    }
    if (buf) {
        memcpy(buf, s, sizeof(Vrc6State));
        if (s->chr_is_ram && s->chr_size > 0u) {
            memcpy(buf + sizeof(Vrc6State), s->chr, s->chr_size);
        }
    }
    return total;
}

static bool vrc6_load_state(void* state, const uint8_t* buf, size_t len) {
    Vrc6State* s = (Vrc6State*)state;
    size_t need = sizeof(Vrc6State);
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
    memcpy(s, buf, sizeof(Vrc6State));
    s->prg_rom = valid_prg;
    s->prg_size = valid_prg_size;
    s->chr = valid_chr;
    s->chr_size = valid_chr_size;
    s->chr_is_ram = valid_chr_is_ram;
    if (s->chr_is_ram && s->chr_size > 0u) {
        memcpy(s->chr, buf + sizeof(Vrc6State), s->chr_size);
    }
    return true;
}

static const MapperVTable VRC6_VTABLE = {
    vrc6_read_prg, NULL, vrc6_write_prg,
    vrc6_read_chr, NULL, vrc6_write_chr,
    vrc6_mirror_mode, vrc6_chr_is_ram, vrc6_has_battery,
    vrc6_irq_pending, NULL, NULL, vrc6_clock_cpu,
    vrc6_expansion_audio_sample, vrc6_destroy,
    vrc6_save_state, vrc6_load_state
};

int vrc6_create(const uint8_t* prg, uint32_t prg_size,
                const uint8_t* chr, uint32_t chr_size,
                Mirroring mirroring, bool has_battery, bool is_26, Mapper* out) {
    Vrc6State* s = (Vrc6State*)malloc(sizeof(Vrc6State));
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
    s->prg_bank_16k = 0u;
    s->prg_bank_8k = 0u;
    memset(s->chr_banks, 0, sizeof(s->chr_banks));
    s->mirror_horizontal = (mirroring == MIRROR_HORIZONTAL);
    s->irq_latch = 0u;
    s->irq_counter = 0u;
    s->irq_enable = false;
    s->irq_enable_after_ack = false;
    s->irq_pending = false;
    memset(&s->pulse1, 0, sizeof(s->pulse1));
    memset(&s->pulse2, 0, sizeof(s->pulse2));
    memset(&s->saw, 0, sizeof(s->saw));
    s->swap_addr = is_26;
    out->vt = &VRC6_VTABLE;
    out->state = s;
    out->mapper_num = is_26 ? 26u : 24u;
    return 0;
}
