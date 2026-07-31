/*
 * fds_audio.c - Famicom Disk System expansion audio (wavetable + modulator).
 * Port of src/mappers/fds_audio.rs to C (M4.4).
 *
 * Single wavetable channel with frequency modulation. 64-entry wave table
 * RAM (6-bit), 12-bit frequency, 6-bit volume with envelope, modulator with
 * its own 64-entry wave table (3-bit), modulator gain and sweep.
 *
 * See: https://www.nesdev.org/wiki/FDS_audio
 */
#include "fds_audio.h"
#include <string.h>

static int8_t sat_sub_i8(int8_t a, int8_t b) {
    int32_t r = (int32_t)a - (int32_t)b;
    if (r < -128) return -128;
    if (r > 127) return 127;
    return (int8_t)r;
}

static int8_t sat_add_i8(int8_t a, int8_t b) {
    int32_t r = (int32_t)a + (int32_t)b;
    if (r < -128) return -128;
    if (r > 127) return 127;
    return (int8_t)r;
}

void fds_audio_init(FdsAudio* a) {
    memset(a, 0, sizeof(*a));
    a->mod_disabled = true;
}

void fds_audio_write_register(FdsAudio* a, uint16_t addr, uint8_t value) {
    if (addr >= 0x4040u && addr <= 0x407Fu) {
        a->wave_ram[a->wave_addr % FDS_WAVE_TABLE_SIZE] = (uint8_t)(value & 0x3Fu);
        a->wave_addr = (uint8_t)((a->wave_addr + 1u) & 0x3Fu);
        return;
    }
    switch (addr) {
        case 0x4080u:
            a->master_volume = (uint8_t)(value & 0x3Fu);
            a->env_disabled = (value & 0x40u) != 0u;
            a->env_increase = (value & 0x80u) != 0u;
            if (a->env_disabled) {
                a->volume_gain = a->master_volume;
            } else {
                a->volume_gain = a->env_increase ? 0u : 0x3Fu;
            }
            break;
        case 0x4081u:
            a->freq = (uint16_t)((a->freq & 0x00FFu) | ((uint16_t)(value & 0x0Fu) << 8));
            break;
        case 0x4082u:
            a->freq = (uint16_t)((a->freq & 0x0F00u) | value);
            break;
        case 0x4083u:
            a->mod_freq = (uint16_t)((a->mod_freq & 0x00FFu) | ((uint16_t)(value & 0x0Fu) << 8));
            if (value & 0x80u) {
                a->mod_disabled = true;
                a->mod_phase_acc = 0u;
            }
            break;
        case 0x4084u:
            a->mod_sweep_neg = (uint8_t)(value & 0x0Fu);
            break;
        case 0x4085u:
            a->mod_sweep_pos = (uint8_t)(value & 0x0Fu);
            break;
        case 0x4086u: {
            int8_t neg = (int8_t)(value & 0x3Fu);
            a->mod_gain = sat_sub_i8(a->mod_gain, neg);
            break;
        }
        case 0x4087u: {
            int8_t pos = (int8_t)(value & 0x3Fu);
            a->mod_gain = sat_add_i8(a->mod_gain, pos);
            if (value & 0x80u) a->mod_disabled = true;
            break;
        }
        case 0x4088u:
            a->mod_wave[a->mod_wave_addr % FDS_MOD_TABLE_SIZE] = (uint8_t)(value & 0x07u);
            a->mod_wave_addr = (uint8_t)((a->mod_wave_addr + 1u) & 0x3Fu);
            break;
        case 0x4089u:
            a->master_volume = (uint8_t)((a->master_volume & 0x3Cu) | (value & 0x03u));
            break;
        case 0x408Au:
            if (value & 0x80u) a->mod_disabled = true;
            break;
        default: break;
    }
}

uint8_t fds_audio_read_register(const FdsAudio* a, uint16_t addr) {
    switch (addr) {
        case 0x4090u: return (uint8_t)(a->volume_gain & 0x3Fu);
        case 0x4092u: return (uint8_t)(a->mod_gain_output & 0x3Fu);
        default: return 0u;
    }
}

void fds_audio_clock(FdsAudio* a, uint32_t apu_cycles) {
    for (uint32_t c = 0u; c < apu_cycles; ++c) {
        a->phase_acc = a->phase_acc + (uint32_t)a->freq;
        if (!a->mod_disabled) {
            a->mod_phase_acc = a->mod_phase_acc + (uint32_t)a->mod_freq;
            a->mod_sweep_counter = (uint8_t)(a->mod_sweep_counter + 1u);
            if (a->mod_sweep_counter >= 8u) {
                a->mod_sweep_counter = 0u;
                if (a->mod_sweep_neg > 0u) {
                    a->mod_gain = sat_sub_i8(a->mod_gain, (int8_t)a->mod_sweep_neg);
                }
                if (a->mod_sweep_pos > 0u) {
                    a->mod_gain = sat_add_i8(a->mod_gain, (int8_t)a->mod_sweep_pos);
                }
            }
        }
        if (!a->env_disabled && a->mod_sweep_counter == 0u) {
            if (a->env_increase && a->volume_gain < 0x3Fu) {
                a->volume_gain = (uint8_t)(a->volume_gain + 1u);
            } else if (!a->env_increase && a->volume_gain > 0u) {
                a->volume_gain = (uint8_t)(a->volume_gain - 1u);
            }
        }
    }
}

float fds_audio_sample(const FdsAudio* a) {
    if (a->freq == 0u) return 0.0f;
    uint32_t wave_idx = (a->phase_acc >> 12) & 0x3Fu;
    int16_t mod_offset = 0;
    if (!a->mod_disabled) {
        uint32_t mod_idx = (a->mod_phase_acc >> 12) & 0x3Fu;
        int16_t mod_val = (int16_t)a->mod_wave[mod_idx];
        mod_offset = (int16_t)(((int16_t)a->mod_gain * mod_val) >> 3);
    }
    int16_t eff_idx = (int16_t)(((int16_t)wave_idx + mod_offset) & 0x3F);
    int16_t wave_val = (int16_t)a->wave_ram[eff_idx];
    int16_t vol = (int16_t)a->volume_gain;
    int16_t sample = (wave_val - 32) * vol;
    float normalized = (float)sample / 2016.0f;
    if (normalized < -1.0f) normalized = -1.0f;
    if (normalized > 1.0f) normalized = 1.0f;
    return normalized;
}
