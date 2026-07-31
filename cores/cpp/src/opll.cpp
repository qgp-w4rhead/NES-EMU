/*
 * opll.c - YM2413 OPLL FM synthesiser for VRC7 (mapper 85 audio).
 * Port of src/mappers/opll.rs to C (M4.4).
 *
 * Compact, deterministic 2-operator FM synthesiser. Not a cycle-exact
 * YM2413 clone; goal is correct register semantics and a reasonable
 * approximation of the FM timbre. See: https://www.nesdev.org/wiki/VRC7_audio
 */
#include "opll.hpp"
#include <math.h>
#include <string.h>

#define SINE_LEN OPLL_SINE_LEN
#define SINE_QUARTER OPLL_SINE_QUARTER
#define PHASE_BITS 20u

static void opll_build_sine(uint16_t* t) {
    for (int i = 0; i < SINE_QUARTER; ++i) {
        double s = sin((double)i / ((double)SINE_LEN / 4.0) * 1.5707963267948966);
        t[i] = (uint16_t)(s * 4095.0 + 0.5);
    }
}

static OpllPatch opll_make_patch(uint8_t mult, uint8_t tl, uint8_t fb,
                                 uint8_t ar, uint8_t dr, uint8_t sl,
                                 uint8_t rr, uint8_t kl, uint8_t am, uint8_t wf) {
    OpllPatch p;
    p.mult = mult; p.tl = tl; p.fb = fb; p.ar = ar; p.dr = dr;
    p.sl = sl; p.rr = rr; p.kl = kl; p.am = am; p.wf = wf;
    return p;
}

static OpllPatch opll_patch_from_regs(const uint8_t* b) {
    OpllPatch p;
    p.mult = (uint8_t)(b[0] & 0x0Fu);
    p.tl = (uint8_t)(((b[0] >> 4) & 0x0Fu) | ((b[2] & 0x03u) << 4));
    p.fb = (uint8_t)(b[1] & 0x07u);
    p.ar = (uint8_t)((b[2] >> 2) & 0x0Fu);
    p.dr = (uint8_t)(b[3] & 0x0Fu);
    p.sl = (uint8_t)((b[3] >> 4) & 0x0Fu);
    p.rr = (uint8_t)(b[4] & 0x0Fu);
    p.kl = (uint8_t)((b[4] >> 4) & 0x03u);
    p.am = (uint8_t)(b[5] & 0x07u);
    p.wf = (uint8_t)(b[7] & 0x03u);
    return p;
}

void opll_init(Opll* o) {
    memset(o, 0, sizeof(*o));
    /* Default VRC7-style patch set (hand-tuned; not the exact VRC7 ROM). */
    o->patches[0]  = opll_make_patch(1, 24, 0, 15, 7, 3, 10, 0, 0, 0);
    o->patches[1]  = opll_make_patch(1, 16, 2, 15, 6, 5, 8, 1, 0, 0);
    o->patches[2]  = opll_make_patch(1, 8, 5, 15, 8, 5, 8, 0, 0, 0);
    o->patches[3]  = opll_make_patch(1, 12, 0, 15, 7, 5, 8, 1, 0, 0);
    o->patches[4]  = opll_make_patch(1, 8, 0, 15, 5, 3, 8, 1, 0, 0);
    o->patches[5]  = opll_make_patch(1, 16, 3, 15, 7, 5, 8, 1, 0, 0);
    o->patches[6]  = opll_make_patch(2, 16, 3, 15, 8, 5, 8, 1, 0, 0);
    o->patches[7]  = opll_make_patch(1, 24, 0, 15, 7, 3, 10, 0, 0, 0);
    o->patches[8]  = opll_make_patch(1, 12, 1, 15, 5, 4, 8, 1, 0, 0);
    o->patches[9]  = opll_make_patch(1, 8, 4, 15, 8, 5, 8, 0, 0, 0);
    o->patches[10] = opll_make_patch(1, 16, 0, 15, 5, 3, 8, 1, 0, 0);
    o->patches[11] = opll_make_patch(1, 8, 3, 15, 7, 5, 8, 1, 0, 0);
    o->patches[12] = opll_make_patch(1, 16, 0, 15, 5, 3, 8, 1, 0, 0);
    o->patches[13] = opll_make_patch(1, 16, 0, 15, 5, 3, 8, 1, 0, 0);
    o->patches[14] = opll_make_patch(1, 16, 0, 15, 5, 3, 8, 1, 0, 0);
    /* patches[15] = default (zeroed by memset); updated by user-instrument writes. */
    opll_build_sine(o->sine);
}

void opll_write_addr(Opll* o, uint8_t addr) {
    o->addr_latch = (uint8_t)(addr & 0x7Fu);
}

static void opll_key_on(Opll* o, uint8_t ch) {
    for (uint8_t i = 0u; i < 2u; ++i) {
        o->ops[ch][i].env_phase = OPLL_ENV_ATTACK;
        o->ops[ch][i].env_amp = 0;
        o->ops[ch][i].phase = 0u;
    }
}

static void opll_key_off(Opll* o, uint8_t ch) {
    for (uint8_t i = 0u; i < 2u; ++i) {
        o->ops[ch][i].env_phase = OPLL_ENV_RELEASE;
    }
}

static void opll_apply_reg(Opll* o, uint8_t addr, uint8_t value) {
    if (addr >= 0x10u && addr <= 0x17u) {
        uint8_t i = (uint8_t)(addr - 0x10u);
        o->user_inst_buf[i] = value;
        if (i == 7u) {
            o->patches[15] = opll_patch_from_regs(o->user_inst_buf);
        }
    } else if (addr >= 0x30u && addr <= 0x35u) {
        uint8_t ch = (uint8_t)(addr - 0x30u);
        o->chan[ch].volume = (uint8_t)(value & 0x0Fu);
        o->chan[ch].instrument = (uint8_t)((value >> 4) & 0x0Fu);
    } else if (addr >= 0x20u && addr <= 0x25u) {
        uint8_t ch = (uint8_t)(addr - 0x20u);
        o->chan[ch].fnum = (uint16_t)((o->chan[ch].fnum & 0x0100u) | value);
    } else if (addr >= 0x40u && addr <= 0x45u) {
        uint8_t ch = (uint8_t)(addr - 0x40u);
        bool was_on = o->chan[ch].key_on;
        o->chan[ch].fnum = (uint16_t)((o->chan[ch].fnum & 0x00FFu) | ((uint16_t)(value & 0x01u) << 8));
        o->chan[ch].block = (uint8_t)((value >> 1) & 0x07u);
        o->chan[ch].key_on = (value & 0x10u) != 0u;
        bool now_on = o->chan[ch].key_on;
        if (now_on && !was_on) opll_key_on(o, ch);
        else if (!now_on && was_on) opll_key_off(o, ch);
    }
}

void opll_write_data(Opll* o, uint8_t value) {
    uint8_t addr = o->addr_latch;
    if (addr >= 0x80u) return;
    o->regs[addr] = value;
    opll_apply_reg(o, addr, value);
}

static uint32_t opll_phase_inc(const OpllChanReg* ch, uint8_t mult) {
    uint32_t m = (mult == 0u) ? 1u : (uint32_t)mult;
    return ((uint32_t)ch->fnum * m) << ch->block;
}

static int32_t opll_env_step(uint8_t rate) {
    if (rate == 0u) return 0;
    return (int32_t)rate << 3;
}

static void opll_clock_env(OpllOperator* op, const OpllPatch* patch, bool is_carrier, uint8_t ch_vol) {
    int32_t sl = (int32_t)patch->sl << 7;
    const int32_t MAX = 4095;
    switch (op->env_phase) {
        case OPLL_ENV_OFF: op->env_amp = MAX; break;
        case OPLL_ENV_ATTACK:
            op->env_amp -= opll_env_step(patch->ar) * 4;
            if (op->env_amp <= 0) { op->env_amp = 0; op->env_phase = OPLL_ENV_DECAY; }
            break;
        case OPLL_ENV_DECAY:
            op->env_amp += opll_env_step(patch->dr);
            if (op->env_amp >= sl) { op->env_amp = sl; op->env_phase = OPLL_ENV_SUSTAIN; }
            break;
        case OPLL_ENV_SUSTAIN: break;
        case OPLL_ENV_RELEASE:
            op->env_amp += opll_env_step(patch->rr) * 2;
            if (op->env_amp >= MAX) { op->env_amp = MAX; op->env_phase = OPLL_ENV_OFF; }
            break;
    }
    int32_t ea = op->env_amp;
    if (ea < 0) ea = 0;
    if (ea > MAX) ea = MAX;
    int32_t level = (MAX - ea) >> 2;
    if (is_carrier) {
        int32_t tl = (int32_t)ch_vol + ((int32_t)patch->tl >> 2);
        if (tl > 63) tl = 63;
        level = level - tl * 16;
        if (level < 0) level = 0;
    } else {
        level = level - (int32_t)patch->tl * 16;
        if (level < 0) level = 0;
    }
    op->level = level;
}

static int32_t opll_sin(const Opll* o, uint32_t phase) {
    uint32_t idx = (phase >> 10) & 0x3FFu;
    uint8_t quad;
    uint32_t intra;
    switch (idx >> 8) {
        case 0u: quad = 0; intra = idx; break;
        case 1u: quad = 1; intra = 0x3FFu - idx; break;
        case 2u: quad = 2; intra = idx - 0x200u; break;
        default: quad = 3; intra = 0x3FFu - (idx - 0x200u); break;
    }
    uint32_t i = intra >> 2;
    if (i > (uint32_t)(SINE_LEN / 4)) i = SINE_LEN / 4;
    int32_t v = (int32_t)o->sine[i];
    return (quad == 1 || quad == 3) ? -v : v;
}

void opll_clock(Opll* o, uint32_t apu_cycles) {
    for (uint32_t c = 0u; c < apu_cycles; ++c) {
        for (uint8_t ch = 0u; ch < OPLL_CHANNELS; ++ch) {
            const OpllPatch* patch = &o->patches[o->chan[ch].instrument];
            uint8_t ch_vol = o->chan[ch].volume;
            uint32_t pinc_mod = opll_phase_inc(&o->chan[ch], patch->mult);
            uint32_t pinc_car = opll_phase_inc(&o->chan[ch], 1u);
            o->ops[ch][0].phase = o->ops[ch][0].phase + pinc_mod;
            opll_clock_env(&o->ops[ch][0], patch, false, ch_vol);
            o->ops[ch][1].phase = o->ops[ch][1].phase + pinc_car;
            opll_clock_env(&o->ops[ch][1], patch, true, ch_vol);
        }
    }
}

float opll_sample(const Opll* o) {
    int32_t sum = 0;
    for (uint8_t ch = 0u; ch < OPLL_CHANNELS; ++ch) {
        if (!o->chan[ch].key_on && o->ops[ch][1].env_phase == OPLL_ENV_OFF) {
            continue;
        }
        const OpllPatch* patch = &o->patches[o->chan[ch].instrument];
        int32_t fb = (patch->fb != 0u) ? (o->ops[ch][0].level >> (7 - patch->fb)) : 0;
        int32_t mod_out = opll_sin(o, o->ops[ch][0].phase + (uint32_t)fb * 64u) * o->ops[ch][0].level / 4095;
        uint32_t car_phase = o->ops[ch][1].phase + (uint32_t)(mod_out * 16);
        int32_t car = opll_sin(o, car_phase) * o->ops[ch][1].level / 4095;
        sum += car;
    }
    float s = (float)sum / ((float)OPLL_CHANNELS * 4095.0f);
    if (s < -1.0f) s = -1.0f;
    if (s > 1.0f) s = 1.0f;
    return s;
}
