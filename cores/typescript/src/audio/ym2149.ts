// YM2149 / AY-3-8910 PSG for Sunsoft 5B (mapper 69 audio).
// Port of cores/c/src/ym2149.c.

export class Ym2149 {
    regs: Uint8Array = new Uint8Array(16);
    addrLatch: number = 0;
    toneTimer: Uint16Array = new Uint16Array(3);
    toneOut: Uint8Array = new Uint8Array(3);
    noiseTimer: number = 0;
    noiseLfsr: number = 0x10000;
    noiseOut: number = 0;
    envTimer: number = 0;
    envPos: number = 0;
    envHolding: boolean = false;

    init(): void {
        this.regs.fill(0);
        this.addrLatch = 0;
        this.toneTimer.fill(0);
        this.toneOut.fill(0);
        this.noiseTimer = 0;
        this.noiseLfsr = 0x10000;
        this.noiseOut = 0;
        this.envTimer = 0;
        this.envPos = 0;
        this.envHolding = false;
    }

    writeAddr(addr: number): void { this.addrLatch = addr & 0x0F; }
    writeData(value: number): void {
        const a = this.addrLatch;
        if (a >= 16) return;
        this.regs[a] = value;
    }
    readData(): number {
        const a = this.addrLatch;
        if (a >= 16) return 0;
        return this.regs[a];
    }

    private tonePeriod(ch: number): number {
        const lo = this.regs[ch * 2];
        const hi = this.regs[ch * 2 + 1] & 0x0F;
        const p = lo | (hi << 8);
        return p > 0 ? p : 1;
    }
    private noisePeriod(): number {
        const p = this.regs[6] & 0x1F;
        return p > 0 ? p : 1;
    }
    private envPeriod(): number {
        const lo = this.regs[0x0B];
        const hi = this.regs[0x0C];
        const p = lo | (hi << 8);
        return p > 0 ? p : 1;
    }
    private envShape(): number { return this.regs[0x0D] & 0x0F; }

    private applyShapeEnd(): void {
        const shape = this.envShape();
        const hold = (shape & 0x08) !== 0;
        const alternate = (shape & 0x04) !== 0;
        if (hold) {
            this.envHolding = true;
            if (alternate) this.envPos = 31 - this.envPos;
        } else {
            this.envPos = 0;
        }
    }

    clock(apuCycles: number): void {
        for (let c = 0; c < apuCycles; ++c) {
            for (let ch = 0; ch < 3; ++ch) {
                if (this.toneTimer[ch] === 0) {
                    this.toneTimer[ch] = this.tonePeriod(ch);
                    this.toneOut[ch] ^= 1;
                } else this.toneTimer[ch] = (this.toneTimer[ch] - 1) & 0xFFFF;
            }
            if (this.noiseTimer === 0) {
                this.noiseTimer = this.noisePeriod();
                const bit = (this.noiseLfsr ^ (this.noiseLfsr >> 3)) & 1;
                this.noiseLfsr = (this.noiseLfsr >> 1) | (bit << 16);
                this.noiseOut = this.noiseLfsr & 1;
            } else this.noiseTimer = (this.noiseTimer - 1) & 0xFFFF;
            if (!this.envHolding) {
                if (this.envTimer === 0) {
                    this.envTimer = this.envPeriod();
                    if (this.envPos < 31) this.envPos = (this.envPos + 1) & 0xFF;
                    else this.applyShapeEnd();
                } else this.envTimer = (this.envTimer - 1) & 0xFFFF;
            }
        }
    }

    private envAmplitude(): number {
        const shape = this.envShape();
        const attack = (shape & 0x02) !== 0;
        const alternate = (shape & 0x04) !== 0;
        const pos = this.envPos;
        let half = Math.floor(pos / 2);
        if (half > 15) half = 15;
        const amp = attack ? half : (15 - half);
        if (pos >= 16) {
            if (alternate) {
                let second = Math.floor((pos - 16) / 2);
                if (second > 15) second = 15;
                return attack ? (15 - second) : second;
            } else if (attack) return 15;
            else return 0;
        }
        return amp;
    }

    private chanOut(ch: number): number {
        const disable = this.regs[7];
        const toneEn = ((disable >> ch) & 1) === 0;
        const noiseEn = ((disable >> (ch + 3)) & 1) === 0;
        const v = this.regs[8 + ch];
        const vol = v & 0x0F;
        const envMode = (v & 0x10) !== 0;
        const amp = envMode ? this.envAmplitude() : vol;
        const tone = toneEn ? this.toneOut[ch] : 0;
        const noise = noiseEn ? this.noiseOut : 0;
        return ((tone | noise) !== 0) ? amp : 0;
    }

    sample(): number {
        const a = this.chanOut(0);
        const b = this.chanOut(1);
        const c = this.chanOut(2);
        const sum = a + b + c;
        let s = sum / 45.0;
        if (s < -1.0) s = -1.0;
        if (s > 1.0) s = 1.0;
        return s;
    }
}
