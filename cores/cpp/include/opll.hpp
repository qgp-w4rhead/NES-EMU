/*
 * opll.h - YM2413 OPLL FM synthesiser for VRC7 (mapper 85 audio).
 * Port of src/mappers/opll.rs to C (M4.4).
 */
#ifndef NES_CORE_C_OPLL_H
#define NES_CORE_C_OPLL_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

#define OPLL_CHANNELS 6
#define OPLL_SINE_LEN 256
#define OPLL_SINE_QUARTER (OPLL_SINE_LEN / 4 + 1)

typedef enum {
    OPLL_ENV_OFF = 0,
    OPLL_ENV_ATTACK,
    OPLL_ENV_DECAY,
    OPLL_ENV_SUSTAIN,
    OPLL_ENV_RELEASE
} OpllEnvPhase;

typedef struct OpllOperator {
    uint32_t phase;
    int32_t  level;
    OpllEnvPhase env_phase;
    int32_t  env_amp;
} OpllOperator;

typedef struct OpllChanReg {
    uint16_t fnum;
    uint8_t  block;
    bool     key_on;
    uint8_t  volume;
    uint8_t  instrument;
} OpllChanReg;

typedef struct OpllPatch {
    uint8_t mult, tl, fb, ar, dr, sl, rr, kl, am, wf;
} OpllPatch;

typedef struct Opll {
    uint8_t  regs[0x80];
    uint8_t  addr_latch;
    OpllChanReg chan[OPLL_CHANNELS];
    OpllOperator ops[OPLL_CHANNELS][2];
    OpllPatch patches[16];
    uint8_t  user_inst_buf[8];
    uint16_t sine[OPLL_SINE_QUARTER];
} Opll;

void opll_init(Opll* o);
void opll_write_addr(Opll* o, uint8_t addr);
void opll_write_data(Opll* o, uint8_t value);
void opll_clock(Opll* o, uint32_t apu_cycles);
float opll_sample(const Opll* o);

#ifdef __cplusplus
}
#endif
#endif /* NES_CORE_C_OPLL_H */
