/*
 * ym2149.c - YM2149 / AY-3-8910 PSG for Sunsoft 5B (mapper 69 audio).
 * Port of src/mappers/ym2149.rs to C (M4.4).
 *
 * 3-channel tone + noise synthesiser. Runs at APU clock = CPU clock / 2.
 * See: https://www.nesdev.org/wiki/Sunsoft_5B_audio
 */
#include "ym2149.hpp"
#include <stdint.h>

void ym2149_init(Ym2149* y) {
    uint8_t i;
    for (i = 0u; i < 16u; ++i) y->regs[i] = 0u;
    y->addr_latch = 0u;
    for (i = 0u; i < 3u; ++i) {
        y->tone_timer[i] = 0u;
        y->tone_out[i] = 0u;
    }
    y->noise_timer = 0u;
    y->noise_lfsr = 0x10000u;
    y->noise_out = 0u;
    y->env_timer = 0u;
    y->env_pos = 0u;
    y->env_holding = false;
}

void ym2149_write_addr(Ym2149* y, uint8_t addr) {
    y->addr_latch = (uint8_t)(addr & 0x0Fu);
}

void ym2149_write_data(Ym2149* y, uint8_t value) {
    uint8_t a = y->addr_latch;
    if (a >= 16u) return;
    y->regs[a] = value;
}

uint8_t ym2149_read_data(const Ym2149* y) {
    uint8_t a = y->addr_latch;
    if (a >= 16u) return 0u;
    return y->regs[a];
}

static uint16_t ym2149_tone_period(const Ym2149* y, uint8_t ch) {
    uint16_t lo = (uint16_t)y->regs[ch * 2u];
    uint16_t hi = (uint16_t)(y->regs[ch * 2u + 1u] & 0x0Fu);
    uint16_t p = (uint16_t)(lo | (hi << 8));
    return p > 0u ? p : 1u;
}

static uint16_t ym2149_noise_period(const Ym2149* y) {
    uint16_t p = (uint16_t)(y->regs[6] & 0x1Fu);
    return p > 0u ? p : 1u;
}

static uint16_t ym2149_env_period(const Ym2149* y) {
    uint16_t lo = (uint16_t)y->regs[0x0Bu];
    uint16_t hi = (uint16_t)y->regs[0x0Cu];
    uint16_t p = (uint16_t)(lo | (hi << 8));
    return p > 0u ? p : 1u;
}

static uint8_t ym2149_env_shape(const Ym2149* y) {
    return (uint8_t)(y->regs[0x0Du] & 0x0Fu);
}

static void ym2149_apply_shape_end(Ym2149* y) {
    uint8_t shape = ym2149_env_shape(y);
    bool hold = (shape & 0x08u) != 0u;
    bool alternate = (shape & 0x04u) != 0u;
    if (hold) {
        y->env_holding = true;
        if (alternate) {
            y->env_pos = (uint8_t)(31u - y->env_pos);
        }
    } else {
        y->env_pos = 0u;
    }
}

void ym2149_clock(Ym2149* y, uint32_t apu_cycles) {
    for (uint32_t c = 0u; c < apu_cycles; ++c) {
        for (uint8_t ch = 0u; ch < 3u; ++ch) {
            if (y->tone_timer[ch] == 0u) {
                y->tone_timer[ch] = ym2149_tone_period(y, ch);
                y->tone_out[ch] = (uint8_t)(y->tone_out[ch] ^ 1u);
            } else {
                y->tone_timer[ch] = (uint16_t)(y->tone_timer[ch] - 1u);
            }
        }
        if (y->noise_timer == 0u) {
            y->noise_timer = ym2149_noise_period(y);
            uint32_t bit = (y->noise_lfsr ^ (y->noise_lfsr >> 3)) & 1u;
            y->noise_lfsr = (y->noise_lfsr >> 1) | (bit << 16);
            y->noise_out = (uint8_t)(y->noise_lfsr & 1u);
        } else {
            y->noise_timer = (uint16_t)(y->noise_timer - 1u);
        }
        if (!y->env_holding) {
            if (y->env_timer == 0u) {
                y->env_timer = ym2149_env_period(y);
                if (y->env_pos < 31u) {
                    y->env_pos = (uint8_t)(y->env_pos + 1u);
                } else {
                    ym2149_apply_shape_end(y);
                }
            } else {
                y->env_timer = (uint16_t)(y->env_timer - 1u);
            }
        }
    }
}

static uint8_t ym2149_env_amplitude(const Ym2149* y) {
    uint8_t shape = ym2149_env_shape(y);
    bool attack = (shape & 0x02u) != 0u;
    bool alternate = (shape & 0x04u) != 0u;
    uint8_t pos = y->env_pos;
    uint8_t half = (uint8_t)(pos / 2u);
    if (half > 15u) half = 15u;
    uint8_t amp = attack ? half : (uint8_t)(15u - half);
    if (pos >= 16u) {
        if (alternate) {
            uint8_t second = (uint8_t)((pos - 16u) / 2u);
            if (second > 15u) second = 15u;
            return attack ? (uint8_t)(15u - second) : second;
        } else if (attack) {
            return 15u;
        } else {
            return 0u;
        }
    }
    return amp;
}

static uint8_t ym2149_chan_out(const Ym2149* y, uint8_t ch) {
    uint8_t disable = y->regs[7];
    bool tone_en = ((disable >> ch) & 1u) == 0u;
    bool noise_en = ((disable >> (ch + 3u)) & 1u) == 0u;
    uint8_t v = y->regs[8u + ch];
    uint8_t vol = (uint8_t)(v & 0x0Fu);
    bool env_mode = (v & 0x10u) != 0u;
    uint8_t amp = env_mode ? ym2149_env_amplitude(y) : vol;
    uint8_t tone = tone_en ? y->tone_out[ch] : 0u;
    uint8_t noise = noise_en ? y->noise_out : 0u;
    return ((tone | noise) != 0u) ? amp : 0u;
}

float ym2149_sample(const Ym2149* y) {
    int32_t a = ym2149_chan_out(y, 0);
    int32_t b = ym2149_chan_out(y, 1);
    int32_t c = ym2149_chan_out(y, 2);
    int32_t sum = a + b + c;
    float s = (float)sum / 45.0f;
    if (s < -1.0f) s = -1.0f;
    if (s > 1.0f) s = 1.0f;
    return s;
}
