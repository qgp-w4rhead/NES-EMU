/*
 * fds.c - Mapper 20 (Famicom Disk System RAM adapter + disk drive).
 * Port of src/mappers/fds.rs to C (M4.4).
 *
 * 32 KB PRG-RAM at $6000-$DFFF, 8 KB BIOS ROM at $E000-$FFFF, 8 KB CHR-RAM,
 * FDS expansion audio, disk drive I/O at $4020-$4033, timer IRQ.
 *
 * See: https://www.nesdev.org/wiki/Famicom_Disk_System
 */
#include "mapper.h"
#include "cartridge.h"
#include "fds_audio.h"
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#define PRG_RAM_SIZE  (32u * 1024u)
#define CHR_RAM_SIZE  (8u * 1024u)
#define BIOS_SIZE     (8u * 1024u)

typedef struct FdsState {
    uint8_t  prg_ram[PRG_RAM_SIZE];
    uint8_t* bios;
    uint8_t  chr_ram[CHR_RAM_SIZE];
    uint8_t* disk_data;
    uint32_t disk_size;
    uint32_t disk_read_pos;
    uint32_t disk_write_pos;
    bool     disk_motor_on;
    bool     disk_transfer_reset;
    bool     disk_write_mode;
    bool     disk_data_available;
    bool     disk_inserted;
    uint8_t  disk_write_latch;
    bool     io_enable_disk;
    bool     io_enable_timer;
    uint16_t timer_latch;
    uint16_t timer_counter;
    bool     timer_enable;
    bool     timer_irq_pending;
    bool     mirror_horizontal;
    FdsAudio audio;
} FdsState;

static void fds_write_disk_register(FdsState* s, uint16_t addr, uint8_t value) {
    switch (addr) {
        case 0x4020u:
            s->timer_latch = (uint16_t)((s->timer_latch & 0xFF00u) | value);
            break;
        case 0x4021u:
            s->timer_latch = (uint16_t)((s->timer_latch & 0x00FFu) | ((uint16_t)value << 8));
            break;
        case 0x4022u:
            s->timer_enable = (value & 0x01u) != 0u;
            if (!s->timer_enable) s->timer_irq_pending = false;
            if (s->timer_enable) s->timer_counter = s->timer_latch;
            break;
        case 0x4023u:
            s->io_enable_disk = (value & 0x01u) != 0u;
            s->io_enable_timer = (value & 0x02u) != 0u;
            if (!s->io_enable_timer) s->timer_irq_pending = false;
            break;
        case 0x4024u:
            s->disk_write_latch = value;
            if (s->disk_write_mode && s->io_enable_disk) {
                if (s->disk_write_pos < s->disk_size) {
                    s->disk_data[s->disk_write_pos] = value;
                    s->disk_write_pos += 1u;
                }
            }
            break;
        case 0x4025u:
            if (!s->io_enable_disk) return;
            bool reset = (value & 0x01u) != 0u;
            if (reset && !s->disk_transfer_reset) {
                s->disk_read_pos = 0u;
                s->disk_write_pos = 0u;
                s->disk_data_available = false;
            }
            s->disk_transfer_reset = reset;
            s->mirror_horizontal = (value & 0x02u) != 0u;
            s->disk_write_mode = (value & 0x04u) != 0u;
            s->disk_motor_on = (value & 0x20u) != 0u;
            if (s->disk_motor_on && !s->disk_write_mode) {
                s->disk_data_available = s->disk_read_pos < s->disk_size;
            }
            break;
        default: break;
    }
}

static uint8_t fds_read_disk_register(FdsState* s, uint16_t addr) {
    switch (addr) {
        case 0x4030u: {
            uint8_t status = 0u;
            if (s->disk_data_available) status |= 0x01u;
            if (s->disk_motor_on && s->disk_inserted) status |= 0x04u;
            if (s->disk_transfer_reset) status |= 0x10u;
            if (s->timer_irq_pending) status |= 0x80u;
            s->timer_irq_pending = false;
            return status;
        }
        case 0x4031u: {
            if (s->disk_read_pos < s->disk_size) {
                uint8_t byte = s->disk_data[s->disk_read_pos];
                s->disk_read_pos += 1u;
                s->disk_data_available = s->disk_read_pos < s->disk_size;
                return byte;
            }
            return 0x00u;
        }
        case 0x4032u:
            return s->disk_inserted ? 0x00u : 0x01u;
        case 0x4033u:
            return 0x80u;
        default:
            return 0x00u;
    }
}

static uint8_t fds_read_prg(const void* state, uint16_t addr) {
    const FdsState* s = (const FdsState*)state;
    if (addr >= 0x6000u && addr < 0xE000u) {
        uint32_t idx = (uint32_t)(addr - 0x6000u) & (PRG_RAM_SIZE - 1u);
        return s->prg_ram[idx];
    }
    if (addr >= 0xE000u) {
        uint32_t idx = (uint32_t)(addr - 0xE000u) & (BIOS_SIZE - 1u);
        return s->bios[idx];
    }
    /* $4020-$4033 and $4040-$408A have read side-effects; handled by
     * fds_read_prg_mut. Return 0 here. */
    return 0x00u;
}

static uint8_t fds_read_prg_mut(void* state, uint16_t addr) {
    FdsState* s = (FdsState*)state;
    if (addr >= 0x6000u && addr < 0xE000u) {
        uint32_t idx = (uint32_t)(addr - 0x6000u) & (PRG_RAM_SIZE - 1u);
        return s->prg_ram[idx];
    }
    if (addr >= 0xE000u) {
        uint32_t idx = (uint32_t)(addr - 0xE000u) & (BIOS_SIZE - 1u);
        return s->bios[idx];
    }
    if (addr >= 0x4020u && addr <= 0x4033u) {
        return fds_read_disk_register(s, addr);
    }
    if (addr == 0x4090u || addr == 0x4092u) {
        return fds_audio_read_register(&s->audio, addr);
    }
    return 0x00u;
}

static void fds_write_prg(void* state, uint16_t addr, uint8_t value) {
    FdsState* s = (FdsState*)state;
    if (addr >= 0x6000u && addr < 0xE000u) {
        uint32_t idx = (uint32_t)(addr - 0x6000u) & (PRG_RAM_SIZE - 1u);
        s->prg_ram[idx] = value;
        return;
    }
    if (addr >= 0x4020u && addr <= 0x4033u) {
        fds_write_disk_register(s, addr, value);
        return;
    }
    if (addr >= 0x4040u && addr <= 0x408Au) {
        fds_audio_write_register(&s->audio, addr, value);
    }
    /* BIOS ROM area ($E000-$FFFF) is read-only. */
}

static uint8_t fds_read_chr(const void* state, uint16_t addr) {
    const FdsState* s = (const FdsState*)state;
    return s->chr_ram[(uint32_t)addr & (CHR_RAM_SIZE - 1u)];
}

static void fds_write_chr(void* state, uint16_t addr, uint8_t value) {
    FdsState* s = (FdsState*)state;
    s->chr_ram[(uint32_t)addr & (CHR_RAM_SIZE - 1u)] = value;
}

static Mirroring fds_mirror_mode(const void* state) {
    const FdsState* s = (const FdsState*)state;
    return s->mirror_horizontal ? MIRROR_HORIZONTAL : MIRROR_VERTICAL;
}

static bool fds_chr_is_ram(const void* state) {
    (void)state;
    return true;
}

static bool fds_has_battery(const void* state) {
    (void)state;
    return false;
}

static bool fds_irq_pending(const void* state) {
    return ((const FdsState*)state)->timer_irq_pending;
}

static void fds_clock_cpu(void* state, uint32_t cpu_cycles) {
    FdsState* s = (FdsState*)state;
    if (s->timer_enable && s->io_enable_timer) {
        for (uint32_t i = 0u; i < cpu_cycles; ++i) {
            if (s->timer_counter == 0u) {
                s->timer_counter = s->timer_latch;
                s->timer_irq_pending = true;
            } else {
                s->timer_counter = (uint16_t)(s->timer_counter - 1u);
            }
        }
    }
    fds_audio_clock(&s->audio, cpu_cycles / 2u);
}

static float fds_expansion_audio_sample(const void* state) {
    const FdsState* s = (const FdsState*)state;
    return fds_audio_sample(&s->audio);
}

static void fds_destroy(void* state) {
    FdsState* s = (FdsState*)state;
    if (s) {
        free(s->bios);
        free(s->disk_data);
        free(s);
    }
}

static const MapperVTable FDS_VTABLE = {
    fds_read_prg, fds_read_prg_mut, fds_write_prg,
    fds_read_chr, NULL, fds_write_chr,
    fds_mirror_mode, fds_chr_is_ram, fds_has_battery,
    fds_irq_pending, NULL, NULL, fds_clock_cpu,
    fds_expansion_audio_sample, fds_destroy
};

int fds_create(const uint8_t* bios, size_t bios_len,
               const uint8_t* disk_data, size_t disk_len, Mapper* out) {
    FdsState* s = (FdsState*)malloc(sizeof(FdsState));
    if (!s) return 1;
    memset(s, 0, sizeof(*s));
    s->bios = (uint8_t*)malloc(BIOS_SIZE);
    if (!s->bios) { free(s); return 1; }
    memset(s->bios, 0, BIOS_SIZE);
    if (bios_len > BIOS_SIZE) bios_len = BIOS_SIZE;
    if (bios_len > 0u && bios) memcpy(s->bios, bios, bios_len);
    if (disk_len > 0u && disk_data) {
        s->disk_data = (uint8_t*)malloc(disk_len);
        if (!s->disk_data) { free(s->bios); free(s); return 1; }
        memcpy(s->disk_data, disk_data, disk_len);
        s->disk_size = (uint32_t)disk_len;
    } else {
        s->disk_data = NULL;
        s->disk_size = 0u;
    }
    s->disk_inserted = true;
    s->io_enable_disk = true;
    s->io_enable_timer = true;
    fds_audio_init(&s->audio);
    out->vt = &FDS_VTABLE;
    out->state = s;
    out->mapper_num = 20u;
    return 0;
}
