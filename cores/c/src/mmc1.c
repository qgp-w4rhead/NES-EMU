/*
 * mmc1.c - Mapper 1 (MMC1). Port of src/mappers/mmc1.rs to C (M4.4).
 *
 * Nintendo's first bank-switching mapper. Uses a serial 5-bit shift register
 * to load four internal registers that control PRG banking (16 KB or 32 KB
 * mode), CHR banking (4 KB or 8 KB mode), and nametable mirroring. Provides
 * 8 KB of PRG-RAM at $6000-$7FFF (optionally battery-backed).
 *
 * See: https://www.nesdev.org/wiki/MMC1
 */
#include "mapper.h"
#include "cartridge.h"
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#define PRG_BANK_SIZE 16384u
#define CHR_4K_SIZE   4096u
#define CHR_8K_SIZE   8192u
#define PRG_RAM_SIZE  8192u

/* Reset value for the control register's PRG mode bits (mode 3 = fix last
 * bank at $C000, switch 16 KB at $8000). (mmc1.rs `CONTROL_PRG_MODE_3`.) */
#define CONTROL_PRG_MODE_3 0x0Cu

typedef struct Mmc1State {
    uint8_t* prg_rom;
    uint32_t prg_size;
    uint8_t* chr;
    uint32_t chr_size;
    bool     chr_is_ram;
    uint8_t  prg_ram[PRG_RAM_SIZE];
    bool     has_battery;
    uint8_t  shift_reg;
    uint8_t  shift_count;
    uint8_t  control;     /* register 0 ($8000). */
    uint8_t  chr_bank_0;  /* register 1 ($A000). */
    uint8_t  chr_bank_1;  /* register 2 ($C000). */
    uint8_t  prg_bank;    /* register 3 ($E000). */
} Mmc1State;

static uint32_t mmc1_prg_bank_count(const Mmc1State* s) {
    uint32_t n = s->prg_size / PRG_BANK_SIZE;
    return n > 0u ? n : 1u;
}

static uint32_t mmc1_chr_bank_count(const Mmc1State* s) {
    uint32_t n = s->chr_size / CHR_4K_SIZE;
    return n > 0u ? n : 1u;
}

static uint8_t mmc1_prg_mode(const Mmc1State* s) {
    return (uint8_t)((s->control >> 2) & 0x03u);
}

static uint8_t mmc1_chr_mode(const Mmc1State* s) {
    return (uint8_t)((s->control >> 4) & 1u);
}

static uint8_t mmc1_prg_read_bank(const Mmc1State* s, uint32_t bank, uint32_t offset) {
    uint32_t count = mmc1_prg_bank_count(s);
    bank = bank % count;
    uint32_t idx = bank * PRG_BANK_SIZE + offset;
    if (idx >= s->prg_size) {
        return 0x00u;
    }
    return s->prg_rom[idx];
}

/* Write to the serial port, handling shift-register accumulation and
 * register commit. (mmc1.rs `serial_write`.) */
static void mmc1_serial_write(Mmc1State* s, uint16_t addr, uint8_t value) {
    /* Bit 7 set: reset shift register and force PRG mode 3. */
    if (value & 0x80u) {
        s->shift_reg = 0u;
        s->shift_count = 0u;
        /* Preserve mirroring (bits 0-1) and CHR mode (bit 4); set PRG mode
         * bits (2-3) to 11 (mode 3). */
        s->control = (uint8_t)((s->control & 0x13u) | CONTROL_PRG_MODE_3);
        return;
    }
    /* Shift bit 0 into the register: first write -> bit 0, fifth -> bit 4. */
    s->shift_reg = (uint8_t)((s->shift_reg >> 1) | ((value & 1u) << 4));
    s->shift_count = (uint8_t)(s->shift_count + 1u);
    if (s->shift_count == 5u) {
        uint8_t reg = (uint8_t)((addr >> 13) & 0x03u);
        switch (reg) {
            case 0: s->control = s->shift_reg; break;
            case 1: s->chr_bank_0 = s->shift_reg; break;
            case 2: s->chr_bank_1 = s->shift_reg; break;
            case 3: s->prg_bank = s->shift_reg; break;
            default: break;
        }
        s->shift_reg = 0u;
        s->shift_count = 0u;
    }
}

static uint8_t mmc1_read_prg(const void* state, uint16_t addr) {
    const Mmc1State* s = (const Mmc1State*)state;
    /* PRG-RAM at $6000-$7FFF. */
    if (addr >= 0x6000u && addr < 0x8000u) {
        uint32_t idx = (uint32_t)(addr - 0x6000u) & (PRG_RAM_SIZE - 1u);
        return s->prg_ram[idx];
    }
    uint32_t local = (uint32_t)(addr - 0x8000u);
    bool in_low = local < PRG_BANK_SIZE;
    uint32_t offset = local & (PRG_BANK_SIZE - 1u);
    switch (mmc1_prg_mode(s)) {
        case 0: case 1: { /* 32 KB mode. */
            uint32_t bank = (uint32_t)(s->prg_bank & 0x0Eu);
            return mmc1_prg_read_bank(s, bank, local);
        }
        case 2: { /* Fix first bank at $8000, switch 16 KB at $C000. */
            if (in_low) {
                return mmc1_prg_read_bank(s, 0u, offset);
            }
            uint32_t bank = (uint32_t)(s->prg_bank & 0x0Fu);
            return mmc1_prg_read_bank(s, bank, offset);
        }
        default: { /* Mode 3 (default): fix last bank at $C000, switch at $8000. */
            if (in_low) {
                uint32_t bank = (uint32_t)(s->prg_bank & 0x0Fu);
                return mmc1_prg_read_bank(s, bank, offset);
            }
            uint32_t last = mmc1_prg_bank_count(s) - 1u;
            return mmc1_prg_read_bank(s, last, offset);
        }
    }
}

static void mmc1_write_prg(void* state, uint16_t addr, uint8_t value) {
    Mmc1State* s = (Mmc1State*)state;
    if (addr >= 0x6000u && addr < 0x8000u) {
        uint32_t idx = (uint32_t)(addr - 0x6000u) & (PRG_RAM_SIZE - 1u);
        s->prg_ram[idx] = value;
        return;
    }
    mmc1_serial_write(s, addr, value);
}

static uint8_t mmc1_read_chr(const void* state, uint16_t addr) {
    const Mmc1State* s = (const Mmc1State*)state;
    uint32_t a = (uint32_t)addr;
    uint32_t count = mmc1_chr_bank_count(s);
    uint32_t idx;
    if (mmc1_chr_mode(s) == 0u) {
        /* 8 KB mode: chr_bank_0 & 0x1E selects the 8 KB bank. */
        uint32_t bank = ((uint32_t)(s->chr_bank_0 & 0x1Eu)) % count;
        idx = bank * CHR_4K_SIZE + (a & (CHR_8K_SIZE - 1u));
    } else if (a < CHR_4K_SIZE) {
        uint32_t bank = ((uint32_t)(s->chr_bank_0 & 0x1Fu)) % count;
        idx = bank * CHR_4K_SIZE + a;
    } else {
        uint32_t bank = ((uint32_t)(s->chr_bank_1 & 0x1Fu)) % count;
        idx = bank * CHR_4K_SIZE + (a - CHR_4K_SIZE);
    }
    if (idx >= s->chr_size) {
        return 0x00u;
    }
    return s->chr[idx];
}

static void mmc1_write_chr(void* state, uint16_t addr, uint8_t value) {
    Mmc1State* s = (Mmc1State*)state;
    if (!s->chr_is_ram) {
        return;
    }
    uint32_t a = (uint32_t)addr;
    uint32_t count = mmc1_chr_bank_count(s);
    uint32_t idx;
    if (mmc1_chr_mode(s) == 0u) {
        uint32_t bank = ((uint32_t)(s->chr_bank_0 & 0x1Eu)) % count;
        idx = bank * CHR_4K_SIZE + (a & (CHR_8K_SIZE - 1u));
    } else if (a < CHR_4K_SIZE) {
        uint32_t bank = ((uint32_t)(s->chr_bank_0 & 0x1Fu)) % count;
        idx = bank * CHR_4K_SIZE + a;
    } else {
        uint32_t bank = ((uint32_t)(s->chr_bank_1 & 0x1Fu)) % count;
        idx = bank * CHR_4K_SIZE + (a - CHR_4K_SIZE);
    }
    if (idx < s->chr_size) {
        s->chr[idx] = value;
    }
}

static Mirroring mmc1_mirror_mode(const void* state) {
    const Mmc1State* s = (const Mmc1State*)state;
    switch (s->control & 0x03u) {
        case 0: return MIRROR_SINGLE_SCREEN_0;
        case 1: return MIRROR_SINGLE_SCREEN_1;
        case 2: return MIRROR_VERTICAL;
        default: return MIRROR_HORIZONTAL;
    }
}

static bool mmc1_chr_is_ram(const void* state) {
    return ((const Mmc1State*)state)->chr_is_ram;
}

static bool mmc1_has_battery(const void* state) {
    return ((const Mmc1State*)state)->has_battery;
}

static void mmc1_destroy(void* state) {
    Mmc1State* s = (Mmc1State*)state;
    if (s) {
        free(s->prg_rom);
        free(s->chr);
        free(s);
    }
}

/* ---- Save / load state (M4.5) ---------------------------------------- */

/* Layout: [sizeof(Mmc1State)] struct (includes embedded prg_ram[8192]) +
 * [chr_size] CHR (only if chr_is_ram). */
static size_t mmc1_save_state(const void* state, uint8_t* buf) {
    const Mmc1State* s = (const Mmc1State*)state;
    size_t total = sizeof(Mmc1State);
    if (s->chr_is_ram) {
        total += s->chr_size;
    }
    if (buf) {
        memcpy(buf, s, sizeof(Mmc1State));
        if (s->chr_is_ram && s->chr_size > 0u) {
            memcpy(buf + sizeof(Mmc1State), s->chr, s->chr_size);
        }
    }
    return total;
}

static bool mmc1_load_state(void* state, const uint8_t* buf, size_t len) {
    Mmc1State* s = (Mmc1State*)state;
    size_t need = sizeof(Mmc1State);
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
    memcpy(s, buf, sizeof(Mmc1State));
    s->prg_rom = valid_prg;
    s->prg_size = valid_prg_size;
    s->chr = valid_chr;
    s->chr_size = valid_chr_size;
    s->chr_is_ram = valid_chr_is_ram;
    if (s->chr_is_ram && s->chr_size > 0u) {
        memcpy(s->chr, buf + sizeof(Mmc1State), s->chr_size);
    }
    return true;
}

static const MapperVTable MMC1_VTABLE = {
    mmc1_read_prg, NULL, mmc1_write_prg,
    mmc1_read_chr, NULL, mmc1_write_chr,
    mmc1_mirror_mode, mmc1_chr_is_ram, mmc1_has_battery,
    NULL, NULL, NULL, NULL, NULL, mmc1_destroy,
    mmc1_save_state, mmc1_load_state
};

int mmc1_create(const uint8_t* prg, uint32_t prg_size,
                const uint8_t* chr, uint32_t chr_size,
                Mirroring mirroring, bool has_battery, Mapper* out) {
    Mmc1State* s = (Mmc1State*)malloc(sizeof(Mmc1State));
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
        s->chr_size = CHR_8K_SIZE;
        s->chr = (uint8_t*)malloc(CHR_8K_SIZE);
        if (!s->chr) {
            free(s->prg_rom);
            free(s);
            return 1;
        }
        memset(s->chr, 0, CHR_8K_SIZE);
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
    s->shift_reg = 0u;
    s->shift_count = 0u;
    /* Encode the header mirroring into the control register's bits 0-1. */
    uint8_t mirr_bits;
    switch (mirroring) {
        case MIRROR_SINGLE_SCREEN_0: mirr_bits = 0x00u; break;
        case MIRROR_SINGLE_SCREEN_1: mirr_bits = 0x01u; break;
        case MIRROR_SINGLE_SCREEN_2: mirr_bits = 0x01u; break; /* 1ScB for non-zero NT. */
        case MIRROR_SINGLE_SCREEN_3: mirr_bits = 0x01u; break;
        case MIRROR_VERTICAL:        mirr_bits = 0x02u; break;
        default:                     mirr_bits = 0x03u; break; /* Horizontal / FourScreen. */
    }
    s->control = (uint8_t)(CONTROL_PRG_MODE_3 | mirr_bits);
    s->chr_bank_0 = 0u;
    s->chr_bank_1 = 0u;
    s->prg_bank = 0u;
    out->vt = &MMC1_VTABLE;
    out->state = s;
    out->mapper_num = 1u;
    return 0;
}
