/*
 * namco163.c - Mapper 19 (Namco 163 with wavetable expansion audio).
 * Port of src/mappers/namco163.rs to C (M4.4).
 *
 * Flexible PRG/CHR banking, switchable mirroring, optional PRG-RAM, and an
 * on-chip 8-channel wavetable synthesiser. Wave RAM (128 bytes) at
 * $4800-$4FFF, address-latched via $F800 (bit 7 = auto-inc). Channel params
 * at $5000-$57FF (8 ch x 8 bytes). Bank registers via `addr & 0xF801`.
 *
 * See: https://www.nesdev.org/wiki/Namco_163
 */
#include "mapper.h"
#include "cartridge.h"
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#define PRG_8K_SIZE   8192u
#define CHR_1K_SIZE   1024u
#define PRG_RAM_SIZE  8192u
#define WAVE_RAM_SIZE 0x80u
#define WAVE_CHANNELS 8u

typedef struct WaveChan {
    uint16_t freq;
    uint8_t  length;
    uint8_t  volume;
    uint8_t  offset;
    uint32_t phase;
    bool     enabled;
} WaveChan;

typedef struct Namco163State {
    uint8_t* prg_rom;
    uint32_t prg_size;
    uint8_t* chr;
    uint32_t chr_size;
    bool     chr_is_ram;
    uint8_t  prg_ram[PRG_RAM_SIZE];
    bool     has_battery;
    uint8_t  prg_banks[4];
    uint8_t  chr_banks[8];
    Mirroring mirror;
    bool     prg_ram_enable;
    bool     prg_ram_write_protect;
    uint16_t irq_latch;
    uint16_t irq_counter;
    bool     irq_enable;
    bool     irq_pending;
    bool     irq_latch_high;
    uint8_t  wave_ram[WAVE_RAM_SIZE];
    uint8_t  wave_addr;
    WaveChan chans[WAVE_CHANNELS];
} Namco163State;

static uint32_t namco163_prg_8k_count(const Namco163State* s) {
    uint32_t n = s->prg_size / PRG_8K_SIZE;
    return n > 0u ? n : 1u;
}
static uint32_t namco163_chr_1k_count(const Namco163State* s) {
    uint32_t n = s->chr_size / CHR_1K_SIZE;
    return n > 0u ? n : 1u;
}

static uint8_t namco163_prg_read_bank(const Namco163State* s, uint32_t bank, uint32_t offset) {
    uint32_t count = namco163_prg_8k_count(s);
    bank = bank % count;
    uint32_t idx = bank * PRG_8K_SIZE + offset;
    if (idx >= s->prg_size) return 0x00u;
    return s->prg_rom[idx];
}

static uint8_t namco163_wave_read(const Namco163State* s) {
    return s->wave_ram[s->wave_addr & 0x7Fu];
}

static void namco163_wave_write(Namco163State* s, uint8_t value) {
    uint8_t a = (uint8_t)(s->wave_addr & 0x7Fu);
    s->wave_ram[a] = value;
    if (s->wave_addr & 0x80u) {
        s->wave_addr = (uint8_t)((s->wave_addr & 0x80u) | ((s->wave_addr + 1u) & 0x7Fu));
    }
}

static uint8_t namco163_chan_param_read(const Namco163State* s, uint16_t addr) {
    uint32_t off = (uint32_t)(addr - 0x5000u);
    uint32_t ch = off / 8u;
    uint32_t sub = off & 0x07u;
    if (ch >= WAVE_CHANNELS) return 0u;
    const WaveChan* c = &s->chans[ch];
    switch (sub) {
        case 0u: return (uint8_t)(c->freq & 0xFFu);
        case 1u: return (uint8_t)(((c->freq >> 8) & 0x0Fu) | ((c->length & 0x0Fu) << 4));
        case 2u: return (uint8_t)(c->volume & 0x0Fu);
        case 3u: return (uint8_t)((c->offset & 0x60u) | (c->enabled ? 0x80u : 0x00u));
        default: return 0u;
    }
}

static void namco163_chan_param_write(Namco163State* s, uint16_t addr, uint8_t value) {
    uint32_t off = (uint32_t)(addr - 0x5000u);
    uint32_t ch = off / 8u;
    uint32_t sub = off & 0x07u;
    if (ch >= WAVE_CHANNELS) return;
    WaveChan* c = &s->chans[ch];
    switch (sub) {
        case 0u: c->freq = (uint16_t)((c->freq & 0x0F00u) | value); break;
        case 1u:
            c->freq = (uint16_t)((c->freq & 0x00FFu) | ((uint16_t)(value & 0x0Fu) << 8));
            c->length = (uint8_t)((value >> 4) & 0x0Fu);
            break;
        case 2u: c->volume = (uint8_t)(value & 0x0Fu); break;
        case 3u:
            c->offset = (uint8_t)(value & 0x60u);
            c->enabled = (value & 0x80u) != 0u;
            break;
        default: break;
    }
}

static uint8_t namco163_chan_sample(const Namco163State* s, uint8_t ch) {
    const WaveChan* c = &s->chans[ch];
    if (!c->enabled) return 0u;
    uint32_t length_nibbles = ((uint32_t)c->length + 1u) * 8u;
    uint32_t phase_nibble = (c->phase >> 16) % length_nibbles;
    if (length_nibbles == 0u) length_nibbles = 1u;
    uint32_t byte_idx = ((uint32_t)c->offset + phase_nibble / 2u) & 0x7Fu;
    uint8_t byte = s->wave_ram[byte_idx];
    uint8_t nibble = (phase_nibble & 1u) ? (uint8_t)(byte >> 4) : (uint8_t)(byte & 0x0Fu);
    return (uint8_t)((nibble * c->volume) / 15u);
}

static uint8_t namco163_read_prg(const void* state, uint16_t addr) {
    const Namco163State* s = (const Namco163State*)state;
    if (addr >= 0x6000u && addr < 0x8000u) {
        if (s->prg_ram_enable) {
            uint32_t idx = (uint32_t)(addr - 0x6000u) & (PRG_RAM_SIZE - 1u);
            return s->prg_ram[idx];
        }
        return 0x00u;
    }
    if (addr >= 0x4800u && addr < 0x5000u) {
        return namco163_wave_read(s);
    }
    if (addr >= 0x5000u && addr < 0x5800u) {
        return namco163_chan_param_read(s, addr);
    }
    uint32_t local = (uint32_t)(addr - 0x8000u);
    uint32_t slot = local / PRG_8K_SIZE;
    uint32_t offset = local & (PRG_8K_SIZE - 1u);
    uint32_t bank = (uint32_t)s->prg_banks[slot] % namco163_prg_8k_count(s);
    return namco163_prg_read_bank(s, bank, offset);
}

static void namco163_write_prg(void* state, uint16_t addr, uint8_t value) {
    Namco163State* s = (Namco163State*)state;
    if (addr >= 0x6000u && addr < 0x8000u) {
        if (s->prg_ram_enable && !s->prg_ram_write_protect) {
            uint32_t idx = (uint32_t)(addr - 0x6000u) & (PRG_RAM_SIZE - 1u);
            s->prg_ram[idx] = value;
        }
        return;
    }
    if (addr >= 0x4800u && addr < 0x5000u) {
        namco163_wave_write(s, value);
        return;
    }
    if (addr >= 0x5000u && addr < 0x5800u) {
        namco163_chan_param_write(s, addr, value);
        return;
    }
    uint16_t reg = (uint16_t)(addr & 0xF801u);
    switch (reg) {
        case 0x8000u: s->chr_banks[0] = value; break;
        case 0x8800u: s->chr_banks[1] = value; break;
        case 0x9000u: s->chr_banks[2] = value; break;
        case 0x9800u: s->chr_banks[3] = value; break;
        case 0xA000u: s->chr_banks[4] = value; break;
        case 0xA800u: s->chr_banks[5] = value; break;
        case 0xB000u: s->chr_banks[6] = value; break;
        case 0xB800u: s->chr_banks[7] = value; break;
        case 0xC000u: s->prg_banks[0] = (uint8_t)(value & 0x3Fu); break;
        case 0xC800u: s->prg_banks[1] = (uint8_t)(value & 0x3Fu); break;
        case 0xD000u: s->prg_banks[2] = (uint8_t)(value & 0x3Fu); break;
        case 0xD800u: s->prg_banks[3] = (uint8_t)(value & 0x3Fu); break;
        case 0xE000u:
            switch (value & 0x03u) {
                case 0u: s->mirror = MIRROR_VERTICAL; break;
                case 1u: s->mirror = MIRROR_HORIZONTAL; break;
                case 2u: s->mirror = MIRROR_SINGLE_SCREEN_0; break;
                default: s->mirror = MIRROR_SINGLE_SCREEN_1; break;
            }
            break;
        case 0xE800u:
            s->prg_ram_enable = (value & 0x80u) != 0u;
            s->prg_ram_write_protect = (value & 0x40u) != 0u;
            break;
        case 0xF000u:
            if (!s->irq_latch_high) {
                s->irq_latch = (uint16_t)((s->irq_latch & 0xFF00u) | value);
                s->irq_latch_high = true;
            } else {
                s->irq_latch = (uint16_t)((s->irq_latch & 0x00FFu) | ((uint16_t)value << 8));
                s->irq_latch_high = false;
            }
            break;
        case 0xF800u: s->wave_addr = value; break;
        default: break;
    }
}

static uint8_t namco163_read_chr(const void* state, uint16_t addr) {
    const Namco163State* s = (const Namco163State*)state;
    uint32_t slot = (uint32_t)addr / CHR_1K_SIZE;
    uint32_t offset = (uint32_t)addr & (CHR_1K_SIZE - 1u);
    uint32_t count = namco163_chr_1k_count(s);
    uint32_t bank = (uint32_t)s->chr_banks[slot] % count;
    uint32_t idx = bank * CHR_1K_SIZE + offset;
    if (idx >= s->chr_size) return 0x00u;
    return s->chr[idx];
}

static void namco163_write_chr(void* state, uint16_t addr, uint8_t value) {
    Namco163State* s = (Namco163State*)state;
    if (!s->chr_is_ram) return;
    uint32_t slot = (uint32_t)addr / CHR_1K_SIZE;
    uint32_t offset = (uint32_t)addr & (CHR_1K_SIZE - 1u);
    uint32_t count = namco163_chr_1k_count(s);
    uint32_t bank = (uint32_t)s->chr_banks[slot] % count;
    uint32_t idx = bank * CHR_1K_SIZE + offset;
    if (idx < s->chr_size) s->chr[idx] = value;
}

static Mirroring namco163_mirror_mode(const void* state) {
    return ((const Namco163State*)state)->mirror;
}

static bool namco163_chr_is_ram(const void* state) {
    return ((const Namco163State*)state)->chr_is_ram;
}

static bool namco163_has_battery(const void* state) {
    return ((const Namco163State*)state)->has_battery;
}

static bool namco163_irq_pending(const void* state) {
    return ((const Namco163State*)state)->irq_pending;
}

static void namco163_clock_cpu(void* state, uint32_t cpu_cycles) {
    Namco163State* s = (Namco163State*)state;
    for (uint32_t i = 0u; i < cpu_cycles; ++i) {
        if (s->irq_counter == 0u) {
            s->irq_counter = s->irq_latch;
            if (s->irq_enable) s->irq_pending = true;
        } else {
            s->irq_counter = (uint16_t)(s->irq_counter - 1u);
        }
    }
    uint32_t active = 0u;
    for (uint8_t ch = 0u; ch < WAVE_CHANNELS; ++ch) {
        if (s->chans[ch].enabled) ++active;
    }
    if (active == 0u) active = 1u;
    for (uint32_t i = 0u; i < cpu_cycles; ++i) {
        for (uint8_t ch = 0u; ch < WAVE_CHANNELS; ++ch) {
            WaveChan* c = &s->chans[ch];
            if (!c->enabled) continue;
            uint32_t inc = (uint32_t)c->freq << 8;
            c->phase = c->phase + (inc / active);
        }
    }
}

static float namco163_expansion_audio_sample(const void* state) {
    const Namco163State* s = (const Namco163State*)state;
    int32_t sum = 0;
    for (uint8_t ch = 0u; ch < WAVE_CHANNELS; ++ch) {
        sum += namco163_chan_sample(s, ch);
    }
    const float N163_GAIN = 0.4f;
    float v = (float)sum / 120.0f;
    if (v < -1.0f) v = -1.0f;
    if (v > 1.0f) v = 1.0f;
    return v * N163_GAIN;
}

static void namco163_destroy(void* state) {
    Namco163State* s = (Namco163State*)state;
    if (s) {
        free(s->prg_rom);
        free(s->chr);
        free(s);
    }
}

/* ---- Save / load state (M4.5) ---------------------------------------- */

/* Layout: [sizeof(Namco163State)] struct (includes embedded prg_ram[8192],
 * wave_ram[128], and the WaveChan[8] audio state) + [chr_size] CHR (only
 * if chr_is_ram). */
static size_t namco163_save_state(const void* state, uint8_t* buf) {
    const Namco163State* s = (const Namco163State*)state;
    size_t total = sizeof(Namco163State);
    if (s->chr_is_ram) {
        total += s->chr_size;
    }
    if (buf) {
        memcpy(buf, s, sizeof(Namco163State));
        if (s->chr_is_ram && s->chr_size > 0u) {
            memcpy(buf + sizeof(Namco163State), s->chr, s->chr_size);
        }
    }
    return total;
}

static bool namco163_load_state(void* state, const uint8_t* buf, size_t len) {
    Namco163State* s = (Namco163State*)state;
    size_t need = sizeof(Namco163State);
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
    memcpy(s, buf, sizeof(Namco163State));
    s->prg_rom = valid_prg;
    s->prg_size = valid_prg_size;
    s->chr = valid_chr;
    s->chr_size = valid_chr_size;
    s->chr_is_ram = valid_chr_is_ram;
    if (s->chr_is_ram && s->chr_size > 0u) {
        memcpy(s->chr, buf + sizeof(Namco163State), s->chr_size);
    }
    return true;
}

static const MapperVTable NAMCO163_VTABLE = {
    namco163_read_prg, NULL, namco163_write_prg,
    namco163_read_chr, NULL, namco163_write_chr,
    namco163_mirror_mode, namco163_chr_is_ram, namco163_has_battery,
    namco163_irq_pending, NULL, NULL, namco163_clock_cpu,
    namco163_expansion_audio_sample, namco163_destroy,
    namco163_save_state, namco163_load_state
};

int namco163_create(const uint8_t* prg, uint32_t prg_size,
                    const uint8_t* chr, uint32_t chr_size,
                    Mirroring mirroring, bool has_battery, Mapper* out) {
    Namco163State* s = (Namco163State*)malloc(sizeof(Namco163State));
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
    s->mirror = mirroring;
    s->prg_ram_enable = false;
    s->prg_ram_write_protect = false;
    s->irq_latch = 0u;
    s->irq_counter = 0u;
    s->irq_enable = false;
    s->irq_pending = false;
    s->irq_latch_high = false;
    memset(s->wave_ram, 0, WAVE_RAM_SIZE);
    s->wave_addr = 0u;
    memset(s->chans, 0, sizeof(s->chans));
    out->vt = &NAMCO163_VTABLE;
    out->state = s;
    out->mapper_num = 19u;
    return 0;
}
