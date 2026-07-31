// Audio Processing Unit: pulse, triangle, noise, and DMC channels.
// Port of cores/c/src/apu.c.

import { DmcReadFn } from './mapper';

export const PulseDutyPatterns: Uint8Array[] = [
    new Uint8Array([0, 1, 0, 0, 0, 0, 0, 0]),
    new Uint8Array([0, 1, 1, 0, 0, 0, 0, 0]),
    new Uint8Array([0, 1, 1, 1, 1, 0, 0, 0]),
    new Uint8Array([1, 0, 0, 0, 1, 1, 1, 1]),
];

export const PulseLengthTable = new Uint8Array([
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14,
    12, 16, 24, 18, 48, 20, 96, 22, 192, 24, 72, 26, 16, 28, 32, 30,
]);

export const TriangleSequence = new Uint8Array([
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0,
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
]);

export const NoisePeriodTable = new Uint16Array([
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
]);

export const DmcRateTable = new Uint16Array([
    214, 190, 170, 160, 149, 138, 127, 113, 107, 95, 85, 80, 71, 63, 54, 42,
]);

export class PulseChannel {
    static readonly MaxPeriod = 0x7FF;
    static readonly MinAudiblePeriod = 8;

    pulse2: boolean;
    duty: number = 0;
    halt: boolean = false;
    constantVolume: boolean = false;
    volume: number = 0;
    sweepEnabled: boolean = false;
    sweepPeriod: number = 0;
    sweepNegate: boolean = false;
    sweepShift: number = 0;
    sweepDivider: number = 0;
    sweepReload: boolean = false;
    timerPeriod: number = 0;
    timer: number = 0;
    sequence: number = 0;
    lengthCounter: number = 0;
    enabled: boolean = false;
    envelopeDivider: number = 0;
    envelopeDecay: number = 0;
    envelopeStart: boolean = false;

    constructor(pulse2: boolean) {
        this.pulse2 = pulse2;
    }

    writeRegister(reg: number, value: number): void {
        switch (reg) {
            case 0:
                this.duty = (value >> 6) & 0x03;
                this.halt = (value & 0x20) !== 0;
                this.constantVolume = (value & 0x10) !== 0;
                this.volume = value & 0x0F;
                break;
            case 1:
                this.sweepEnabled = (value & 0x80) !== 0;
                this.sweepPeriod = (value >> 4) & 0x07;
                this.sweepNegate = (value & 0x08) !== 0;
                this.sweepShift = value & 0x07;
                this.sweepReload = true;
                break;
            case 2:
                this.timerPeriod = (this.timerPeriod & 0xFF00) | value;
                break;
            case 3: {
                const high = value & 0x07;
                this.timerPeriod = (this.timerPeriod & 0x00FF) | (high << 8);
                this.timer = (this.timer & 0x00FF) | (high << 8);
                if (this.enabled) {
                    const idx = value >> 3;
                    this.lengthCounter = PulseLengthTable[idx];
                }
                this.envelopeStart = true;
                this.sequence = 0;
                break;
            }
        }
    }

    setEnabled(enabled: boolean): void {
        this.enabled = enabled;
        if (!enabled) this.lengthCounter = 0;
    }

    sweepTarget(): number {
        if (!this.sweepEnabled || this.sweepShift === 0) return this.timerPeriod;
        const shifted = this.timerPeriod >> this.sweepShift;
        if (this.sweepNegate) {
            if (this.pulse2) return this.timerPeriod - shifted - 1;
            return this.timerPeriod - shifted;
        }
        return this.timerPeriod + shifted;
    }

    isMuted(): boolean {
        return this.timerPeriod < PulseChannel.MinAudiblePeriod || this.sweepTarget() > PulseChannel.MaxPeriod;
    }

    tick(): void {
        if (this.timer === 0) {
            this.timer = this.timerPeriod;
            this.sequence = (this.sequence + 1) & 0x07;
        } else this.timer--;
    }

    clockEnvelope(): void {
        if (this.envelopeStart) {
            this.envelopeStart = false;
            this.envelopeDecay = 15;
            this.envelopeDivider = this.volume;
        } else if (this.envelopeDivider === 0) {
            this.envelopeDivider = this.volume;
            if (this.envelopeDecay > 0) this.envelopeDecay--;
            else if (this.halt) this.envelopeDecay = 15;
        } else this.envelopeDivider--;
    }

    private clockLength(): void {
        if (!this.halt && this.lengthCounter > 0) this.lengthCounter--;
    }

    private clockSweep(): void {
        const dividerZero = this.sweepDivider === 0;
        if (dividerZero && this.sweepEnabled && this.sweepShift !== 0) {
            const target = this.sweepTarget();
            if (target <= PulseChannel.MaxPeriod && this.timerPeriod >= PulseChannel.MinAudiblePeriod)
                this.timerPeriod = target;
        }
        if (dividerZero || this.sweepReload) {
            this.sweepDivider = this.sweepPeriod;
            this.sweepReload = false;
        } else this.sweepDivider--;
    }

    clockQuarterFrame(): void { this.clockEnvelope(); }
    clockHalfFrame(): void { this.clockLength(); this.clockSweep(); }

    sample(): number {
        if (!this.enabled || this.lengthCounter === 0 || this.isMuted()) return 0;
        const dutyBit = PulseDutyPatterns[this.duty][this.sequence];
        if (dutyBit === 0) return 0;
        return this.constantVolume ? this.volume : this.envelopeDecay;
    }
}

export class TriangleChannel {
    halt: boolean = false;
    linearReload: number = 0;
    linearCounter: number = 0;
    linearStart: boolean = false;
    timerPeriod: number = 0;
    timer: number = 0;
    sequence: number = 0;
    lengthCounter: number = 0;
    enabled: boolean = false;

    writeRegister(reg: number, value: number): void {
        switch (reg) {
            case 0:
                this.halt = (value & 0x80) !== 0;
                this.linearReload = value & 0x7F;
                break;
            case 1: break;
            case 2:
                this.timerPeriod = (this.timerPeriod & 0xFF00) | value;
                break;
            case 3: {
                const high = value & 0x07;
                this.timerPeriod = (this.timerPeriod & 0x00FF) | (high << 8);
                this.timer = (this.timer & 0x00FF) | (high << 8);
                if (this.enabled) {
                    const idx = value >> 3;
                    this.lengthCounter = PulseLengthTable[idx];
                }
                this.linearStart = true;
                this.sequence = 0;
                break;
            }
        }
    }

    setEnabled(enabled: boolean): void {
        this.enabled = enabled;
        if (!enabled) this.lengthCounter = 0;
    }

    tick(): void {
        if (this.timer === 0) {
            this.timer = this.timerPeriod;
            this.sequence = (this.sequence + 1) & 0x1F;
        } else this.timer--;
    }

    private clockLinear(): void {
        if (this.linearStart) {
            this.linearStart = false;
            this.linearCounter = this.linearReload;
        } else if (this.linearCounter > 0) this.linearCounter--;
        if (this.halt) this.linearStart = true;
    }

    private clockLength(): void {
        if (!this.halt && this.lengthCounter > 0) this.lengthCounter--;
    }

    clockQuarterFrame(): void { this.clockLinear(); }
    clockHalfFrame(): void { this.clockLength(); }

    sample(): number {
        if (!this.enabled || this.lengthCounter === 0 || this.linearCounter === 0) return 0;
        return TriangleSequence[this.sequence];
    }
}

export class NoiseChannel {
    halt: boolean = false;
    constantVolume: boolean = false;
    volume: number = 0;
    mode: boolean = false;
    periodIndex: number = 0;
    timerPeriod: number = NoisePeriodTable[0];
    timer: number = 0;
    lfsr: number = 1;
    lengthCounter: number = 0;
    enabled: boolean = false;
    envelopeDivider: number = 0;
    envelopeDecay: number = 0;
    envelopeStart: boolean = false;

    constructor() {
        this.timerPeriod = NoisePeriodTable[0];
        this.lfsr = 1;
    }

    writeRegister(reg: number, value: number): void {
        switch (reg) {
            case 0:
                this.halt = (value & 0x20) !== 0;
                this.constantVolume = (value & 0x10) !== 0;
                this.volume = value & 0x0F;
                break;
            case 1: break;
            case 2:
                this.mode = (value & 0x80) !== 0;
                this.periodIndex = value & 0x0F;
                this.timerPeriod = NoisePeriodTable[this.periodIndex];
                break;
            case 3:
                if (this.enabled) {
                    const idx = value >> 3;
                    this.lengthCounter = PulseLengthTable[idx];
                }
                this.envelopeStart = true;
                break;
        }
    }

    setEnabled(enabled: boolean): void {
        this.enabled = enabled;
        if (!enabled) this.lengthCounter = 0;
    }

    tick(): void {
        if (this.timer === 0) {
            this.timer = this.timerPeriod;
            const bit0 = this.lfsr & 0x0001;
            const tap = this.mode ? 6 : 1;
            const tapBit = (this.lfsr >> tap) & 0x0001;
            const feedback = bit0 ^ tapBit;
            this.lfsr = (this.lfsr >> 1);
            if (feedback !== 0) this.lfsr |= 0x4000;
        } else this.timer--;
    }

    clockEnvelope(): void {
        if (this.envelopeStart) {
            this.envelopeStart = false;
            this.envelopeDecay = 15;
            this.envelopeDivider = this.volume;
        } else if (this.envelopeDivider === 0) {
            this.envelopeDivider = this.volume;
            if (this.envelopeDecay > 0) this.envelopeDecay--;
            else if (this.halt) this.envelopeDecay = 15;
        } else this.envelopeDivider--;
    }

    private clockLength(): void {
        if (!this.halt && this.lengthCounter > 0) this.lengthCounter--;
    }

    clockQuarterFrame(): void { this.clockEnvelope(); }
    clockHalfFrame(): void { this.clockLength(); }

    sample(): number {
        if (!this.enabled || this.lengthCounter === 0) return 0;
        if ((this.lfsr & 1) !== 0) return 0;
        return this.constantVolume ? this.volume : this.envelopeDecay;
    }
}

export class DmcChannel {
    irqEnable: boolean = false;
    loopFlag: boolean = false;
    rateIndex: number = 0;
    timerPeriod: number = DmcRateTable[0];
    timer: number = 0;
    outputCounter: number = 0;
    sampleBuffer: number = 0;
    bufferBits: number = 0;
    sampleAddrBase: number = 0xC000;
    sampleAddress: number = 0xC000;
    sampleLength: number = 1;
    bytesRemaining: number = 0;
    enabled: boolean = false;
    irqFlag: boolean = false;

    constructor() {
        this.timerPeriod = DmcRateTable[0];
        this.sampleAddrBase = 0xC000;
        this.sampleAddress = 0xC000;
        this.sampleLength = 1;
    }

    writeRegister(reg: number, value: number): void {
        switch (reg) {
            case 0:
                this.irqEnable = (value & 0x80) !== 0;
                this.loopFlag = (value & 0x40) !== 0;
                this.rateIndex = value & 0x0F;
                this.timerPeriod = DmcRateTable[this.rateIndex];
                break;
            case 1:
                this.outputCounter = value & 0x7F;
                break;
            case 2:
                this.sampleAddrBase = ((value << 6) | 0xC000) & 0xFFFF;
                break;
            case 3:
                this.sampleLength = ((value << 4) | 1) & 0xFFFF;
                break;
        }
    }

    setEnabled(enabled: boolean): void {
        this.enabled = enabled;
        if (enabled) {
            if (this.bytesRemaining === 0) {
                this.sampleAddress = this.sampleAddrBase;
                this.bytesRemaining = this.sampleLength;
                this.bufferBits = 0;
            }
        } else this.bytesRemaining = 0;
    }

    clearIrq(): void { this.irqFlag = false; }

    private clockOutputUnit(read: DmcReadFn): void {
        if (this.bufferBits === 0 && this.bytesRemaining > 0) {
            this.sampleBuffer = read(this.sampleAddress);
            this.bufferBits = 8;
            this.sampleAddress = (this.sampleAddress + 1) & 0xFFFF;
            if (this.sampleAddress === 0) this.sampleAddress = 0x8000;
            this.bytesRemaining--;
            if (this.bytesRemaining === 0) {
                if (this.loopFlag) {
                    this.sampleAddress = this.sampleAddrBase;
                    this.bytesRemaining = this.sampleLength;
                } else if (this.irqEnable) this.irqFlag = true;
            }
        }

        if (this.bufferBits > 0) {
            const bit = this.sampleBuffer & 1;
            if (bit === 0)
                this.outputCounter = (this.outputCounter >= 2) ? this.outputCounter - 2 : 0;
            else {
                let v = this.outputCounter + 2;
                if (v > 127) v = 127;
                this.outputCounter = v;
            }
            this.sampleBuffer >>= 1;
            this.bufferBits--;
        }
    }

    tick(read: DmcReadFn): void {
        if (this.timer === 0) {
            this.timer = this.timerPeriod;
            this.clockOutputUnit(read);
        } else this.timer--;
    }

    sample(): number { return this.outputCounter; }
}
