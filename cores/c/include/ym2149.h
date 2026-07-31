/*
 * ym2149.h - YM2149 / AY-3-8910 PSG for Sunsoft 5B (mapper 69 audio).
 * Port of src/mappers/ym2149.rs to C (M4.4).
 */
#ifndef NES_CORE_C_YM2149_H
#define NES_CORE_C_YM2149_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct Ym2149 {
    uint8_t  regs[16];
    uint8_t  addr_latch;
    uint16_t tone_timer[3];
    uint8_t  tone_out[3];
    uint16_t noise_timer;
    uint32_t noise_lfsr;
    uint8_t  noise_out;
    uint16_t env_timer;
    uint8_t  env_pos;
    bool     env_holding;
} Ym2149;

void ym2149_init(Ym2149* y);
void ym2149_write_addr(Ym2149* y, uint8_t addr);
void ym2149_write_data(Ym2149* y, uint8_t value);
uint8_t ym2149_read_data(const Ym2149* y);
void ym2149_clock(Ym2149* y, uint32_t apu_cycles);
float ym2149_sample(const Ym2149* y);

#ifdef __cplusplus
}
#endif
#endif /* NES_CORE_C_YM2149_H */
