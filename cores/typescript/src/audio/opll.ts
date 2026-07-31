// YM2413 OPLL FM synthesiser for VRC7 (mapper 85 audio).
// Port of cores/c/src/opll.c.

export const OPLL_CHANNELS = 6;
export const OPLL_SINE_LEN = 256;
export const OPLL_SINE_QUARTER = OPLL_SINE_LEN / 4 + 1;

export enum OpllEnvPhase {
    Off = 0,
    Attack,
    Decay,
    Sustain,
    Release,
}

export interface OpllPatch {
    mult: number; tl: number; fb: number; ar: number; dr: number;
    sl: number; rr: number; kl: number; am: number; wf: number;
}

class OpllOperator {
    phase: number = 0;
    level: number = 0;
    envPhase: OpllEnvPhase = OpllEnvPhase.Off;
    envAmp: number = 0;
}

class OpllChanReg {
    fnum: number = 0;
    block: number = 0;
    keyOn: boolean = false;
    volume: number = 0;
    instrument: number = 0;
}

function makePatch(mult: number, tl: number, fb: number, ar: number, dr: number,
                   sl: number, rr: number, kl: number, am: number, wf: number): OpllPatch {
    return { mult, tl, fb, ar, dr, sl, rr, kl, am, wf };
}

function patchFromRegs(b: Uint8Array): OpllPatch {
    return {
        mult: b[0] & 0x0F,
        tl: ((b[0] >> 4) & 0x0F) | ((b[2] & 0x03) << 4),
        fb: b[1] & 0x07,
        ar: (b[2] >> 2) & 0x0F,
        dr: b[3] & 0x0F,
        sl: (b[3] >> 4) & 0x0F,
        rr: b[4] & 0x0F,
        kl: (b[4] >> 4) & 0x03,
        am: b[5] & 0x07,
        wf: b[7] & 0x03,
    };
}

export class Opll {
    regs: Uint8Array = new Uint8Array(0x80);
    addrLatch: number = 0;
    chan: OpllChanReg[] = Array.from({ length: OPLL_CHANNELS }, () => new OpllChanReg());
    ops: OpllOperator[][] = Array.from({ length: OPLL_CHANNELS }, () => [new OpllOperator(), new OpllOperator()]);
    patches: OpllPatch[] = Array.from({ length: 16 }, () => makePatch(0, 0, 0, 0, 0, 0, 0, 0, 0, 0));
    userInstBuf: Uint8Array = new Uint8Array(8);
    sine: Uint16Array = new Uint16Array(OPLL_SINE_QUARTER);

    init(): void {
        this.regs.fill(0);
        this.addrLatch = 0;
        for (let i = 0; i < OPLL_CHANNELS; ++i) {
            this.chan[i] = new OpllChanReg();
            this.ops[i] = [new OpllOperator(), new OpllOperator()];
        }
        this.patches[0] = makePatch(1, 24, 0, 15, 7, 3, 10, 0, 0, 0);
        this.patches[1] = makePatch(1, 16, 2, 15, 6, 5, 8, 1, 0, 0);
        this.patches[2] = makePatch(1, 8, 5, 15, 8, 5, 8, 0, 0, 0);
        this.patches[3] = makePatch(1, 12, 0, 15, 7, 5, 8, 1, 0, 0);
        this.patches[4] = makePatch(1, 8, 0, 15, 5, 3, 8, 1, 0, 0);
        this.patches[5] = makePatch(1, 16, 3, 15, 7, 5, 8, 1, 0, 0);
        this.patches[6] = makePatch(2, 16, 3, 15, 8, 5, 8, 1, 0, 0);
        this.patches[7] = makePatch(1, 24, 0, 15, 7, 3, 10, 0, 0, 0);
        this.patches[8] = makePatch(1, 12, 1, 15, 5, 4, 8, 1, 0, 0);
        this.patches[9] = makePatch(1, 8, 4, 15, 8, 5, 8, 0, 0, 0);
        this.patches[10] = makePatch(1, 16, 0, 15, 5, 3, 8, 1, 0, 0);
        this.patches[11] = makePatch(1, 8, 3, 15, 7, 5, 8, 1, 0, 0);
        this.patches[12] = makePatch(1, 16, 0, 15, 5, 3, 8, 1, 0, 0);
        this.patches[13] = makePatch(1, 16, 0, 15, 5, 3, 8, 1, 0, 0);
        this.patches[14] = makePatch(1, 16, 0, 15, 5, 3, 8, 1, 0, 0);
        this.userInstBuf.fill(0);
        this.buildSine();
    }

    private buildSine(): void {
        for (let i = 0; i < OPLL_SINE_QUARTER; ++i) {
            const s = Math.sin((i / (OPLL_SINE_LEN / 4.0)) * 1.5707963267948966);
            this.sine[i] = Math.floor(s * 4095.0 + 0.5);
        }
    }

    writeAddr(addr: number): void { this.addrLatch = addr & 0x7F; }

    private keyOn(ch: number): void {
        for (let i = 0; i < 2; ++i) {
            this.ops[ch][i].envPhase = OpllEnvPhase.Attack;
            this.ops[ch][i].envAmp = 0;
            this.ops[ch][i].phase = 0;
        }
    }
    private keyOff(ch: number): void {
        for (let i = 0; i < 2; ++i) this.ops[ch][i].envPhase = OpllEnvPhase.Release;
    }

    private applyReg(addr: number, value: number): void {
        if (addr >= 0x10 && addr <= 0x17) {
            const i = addr - 0x10;
            this.userInstBuf[i] = value;
            if (i === 7) this.patches[15] = patchFromRegs(this.userInstBuf);
        } else if (addr >= 0x30 && addr <= 0x35) {
            const ch = addr - 0x30;
            this.chan[ch].volume = value & 0x0F;
            this.chan[ch].instrument = (value >> 4) & 0x0F;
        } else if (addr >= 0x20 && addr <= 0x25) {
            const ch = addr - 0x20;
            this.chan[ch].fnum = (this.chan[ch].fnum & 0x0100) | value;
        } else if (addr >= 0x40 && addr <= 0x45) {
            const ch = addr - 0x40;
            const wasOn = this.chan[ch].keyOn;
            this.chan[ch].fnum = (this.chan[ch].fnum & 0x00FF) | ((value & 0x01) << 8);
            this.chan[ch].block = (value >> 1) & 0x07;
            this.chan[ch].keyOn = (value & 0x10) !== 0;
            const nowOn = this.chan[ch].keyOn;
            if (nowOn && !wasOn) this.keyOn(ch);
            else if (!nowOn && wasOn) this.keyOff(ch);
        }
    }

    writeData(value: number): void {
        const addr = this.addrLatch;
        if (addr >= 0x80) return;
        this.regs[addr] = value;
        this.applyReg(addr, value);
    }

    private phaseInc(ch: OpllChanReg, mult: number): number {
        const m = (mult === 0) ? 1 : mult;
        return ((ch.fnum * m) << ch.block) >>> 0;
    }
    private envStep(rate: number): number {
        if (rate === 0) return 0;
        return rate << 3;
    }

    private clockEnv(op: OpllOperator, patch: OpllPatch, isCarrier: boolean, chVol: number): void {
        const sl = patch.sl << 7;
        const MAX = 4095;
        switch (op.envPhase) {
            case OpllEnvPhase.Off: op.envAmp = MAX; break;
            case OpllEnvPhase.Attack:
                op.envAmp -= this.envStep(patch.ar) * 4;
                if (op.envAmp <= 0) { op.envAmp = 0; op.envPhase = OpllEnvPhase.Decay; }
                break;
            case OpllEnvPhase.Decay:
                op.envAmp += this.envStep(patch.dr);
                if (op.envAmp >= sl) { op.envAmp = sl; op.envPhase = OpllEnvPhase.Sustain; }
                break;
            case OpllEnvPhase.Sustain: break;
            case OpllEnvPhase.Release:
                op.envAmp += this.envStep(patch.rr) * 2;
                if (op.envAmp >= MAX) { op.envAmp = MAX; op.envPhase = OpllEnvPhase.Off; }
                break;
        }
        let ea = op.envAmp;
        if (ea < 0) ea = 0;
        if (ea > MAX) ea = MAX;
        let level = (MAX - ea) >> 2;
        if (isCarrier) {
            let tl = chVol + (patch.tl >> 2);
            if (tl > 63) tl = 63;
            level = level - tl * 16;
            if (level < 0) level = 0;
        } else {
            level = level - patch.tl * 16;
            if (level < 0) level = 0;
        }
        op.level = level;
    }

    private sinValue(phase: number): number {
        const idx = (phase >> 10) & 0x3FF;
        let quad: number; let intra: number;
        switch (idx >> 8) {
            case 0: quad = 0; intra = idx; break;
            case 1: quad = 1; intra = 0x3FF - idx; break;
            case 2: quad = 2; intra = idx - 0x200; break;
            default: quad = 3; intra = 0x3FF - (idx - 0x200); break;
        }
        let i = intra >> 2;
        if (i > (OPLL_SINE_LEN / 4)) i = OPLL_SINE_LEN / 4;
        const v = this.sine[i];
        return (quad === 1 || quad === 3) ? -v : v;
    }

    clock(apuCycles: number): void {
        for (let c = 0; c < apuCycles; ++c) {
            for (let ch = 0; ch < OPLL_CHANNELS; ++ch) {
                const patch = this.patches[this.chan[ch].instrument];
                const chVol = this.chan[ch].volume;
                const pincMod = this.phaseInc(this.chan[ch], patch.mult);
                const pincCar = this.phaseInc(this.chan[ch], 1);
                this.ops[ch][0].phase = (this.ops[ch][0].phase + pincMod) >>> 0;
                this.clockEnv(this.ops[ch][0], patch, false, chVol);
                this.ops[ch][1].phase = (this.ops[ch][1].phase + pincCar) >>> 0;
                this.clockEnv(this.ops[ch][1], patch, true, chVol);
            }
        }
    }

    sample(): number {
        let sum = 0;
        for (let ch = 0; ch < OPLL_CHANNELS; ++ch) {
            if (!this.chan[ch].keyOn && this.ops[ch][1].envPhase === OpllEnvPhase.Off) continue;
            const patch = this.patches[this.chan[ch].instrument];
            const fb = (patch.fb !== 0) ? (this.ops[ch][0].level >> (7 - patch.fb)) : 0;
            const modOut = Math.floor(this.sinValue((this.ops[ch][0].phase + fb * 64) >>> 0) * this.ops[ch][0].level / 4095);
            const carPhase = (this.ops[ch][1].phase + modOut * 16) >>> 0;
            const car = Math.floor(this.sinValue(carPhase) * this.ops[ch][1].level / 4095);
            sum += car;
        }
        let s = sum / (OPLL_CHANNELS * 4095.0);
        if (s < -1.0) s = -1.0;
        if (s > 1.0) s = 1.0;
        return s;
    }
}
