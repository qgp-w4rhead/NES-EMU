// Audio Processing Unit (port of cores/c/src/apu.c).

import { Region, ApuFrameThreshold, cpuCyclesPerSample, apu4StepThresholds, apu5StepThresholds, apuResetAt, apu4StepIrqThreshold } from './region';
import { DmcReadFn } from './mapper';
import { PulseChannel, TriangleChannel, NoiseChannel, DmcChannel } from './apu_channels';

export class Apu {
    static readonly ChannelCount = 5;
    static readonly SampleRate = 44100;

    pulse1: PulseChannel = new PulseChannel(false);
    pulse2: PulseChannel = new PulseChannel(true);
    triangle: TriangleChannel = new TriangleChannel();
    noise: NoiseChannel = new NoiseChannel();
    dmc: DmcChannel = new DmcChannel();

    cycleAccumulator: number = 0;

    channelVolumes: Float32Array = new Float32Array(Apu.ChannelCount);
    channelMuted: boolean[] = new Array(Apu.ChannelCount).fill(false);
    selectedChannel: number = 0;

    lpfPrev: number = -1.0;
    dcPrevX: number = -1.0;
    dcPrevY: number = 0.0;
    mixAccumulator: number = 0.0;
    mixCount: number = 0;
    sampleAccumulator: number = 0.0;
    lastDecimated: number = -1.0;

    frameMode5Step: boolean = false;
    frameIrqInhibit: boolean = false;
    frameCycle: number = 0;
    frameIrq: boolean = false;
    frameResetDelay: number = 0;

    region: number = Region.Ntsc;

    private thresholds: ApuFrameThreshold[] = [
        { threshold: 0, quarter: false, half: false },
        { threshold: 0, quarter: false, half: false },
        { threshold: 0, quarter: false, half: false },
        { threshold: 0, quarter: false, half: false },
    ];

    constructor() {
        for (let i = 0; i < Apu.ChannelCount; ++i) {
            this.channelVolumes[i] = 1.0;
            this.channelMuted[i] = false;
        }
        this.lpfPrev = -1.0;
        this.dcPrevX = -1.0;
        this.dcPrevY = 0.0;
        this.lastDecimated = -1.0;
    }

    writeStatus(value: number): void {
        this.pulse1.setEnabled((value & 0x01) !== 0);
        this.pulse2.setEnabled((value & 0x02) !== 0);
        this.triangle.setEnabled((value & 0x04) !== 0);
        this.noise.setEnabled((value & 0x08) !== 0);
        this.dmc.setEnabled((value & 0x10) !== 0);
    }

    readStatus(): number {
        let v = 0;
        if (this.pulse1.lengthCounter > 0) v |= 0x01;
        if (this.pulse2.lengthCounter > 0) v |= 0x02;
        if (this.triangle.lengthCounter > 0) v |= 0x04;
        if (this.noise.lengthCounter > 0) v |= 0x08;
        if (this.dmc.bytesRemaining > 0) v |= 0x10;
        if (this.frameIrq) v |= 0x40;
        if (this.dmc.irqFlag) v |= 0x80;
        this.frameIrq = false;
        this.dmc.irqFlag = false;
        return v;
    }

    writeFrameCounter(value: number): void {
        const newMode5Step = (value & 0x80) !== 0;
        this.frameIrqInhibit = (value & 0x40) !== 0;
        if (this.frameIrqInhibit) this.frameIrq = false;
        if (newMode5Step) {
            this.clockQuarterFrame();
            this.clockHalfFrame();
        }
        this.frameResetDelay = 4;
        this.frameMode5Step = newMode5Step;
    }

    irqPending(): boolean { return this.frameIrq || this.dmc.irqFlag; }

    setRegion(region: number): void { this.region = region; }

    private stepFrameCounter(cpuCycles: number): void {
        if (this.frameResetDelay > 0) {
            const advance = (cpuCycles < this.frameResetDelay) ? cpuCycles : this.frameResetDelay;
            this.frameResetDelay -= advance;
            if (this.frameResetDelay === 0) this.frameCycle = 0;
            const remaining = cpuCycles - advance;
            if (remaining === 0) return;
            this.frameCycle += remaining;
        } else {
            this.frameCycle += cpuCycles;
        }

        const prev = (this.frameCycle >= cpuCycles) ? (this.frameCycle - cpuCycles) : 0;
        if (this.frameMode5Step) apu5StepThresholds(this.region, this.thresholds);
        else apu4StepThresholds(this.region, this.thresholds);
        const irqThreshold = apu4StepIrqThreshold(this.region);

        for (let i = 0; i < 4; ++i) {
            const threshold = this.thresholds[i].threshold;
            const quarter = this.thresholds[i].quarter;
            const half = this.thresholds[i].half;
            if (prev < threshold && this.frameCycle >= threshold) {
                if (quarter) this.clockQuarterFrame();
                if (half) this.clockHalfFrame();
                if (!this.frameMode5Step && threshold === irqThreshold && !this.frameIrqInhibit)
                    this.frameIrq = true;
            }
        }

        const resetAt = apuResetAt(this.region, this.frameMode5Step);
        if (this.frameCycle >= resetAt) this.frameCycle -= resetAt;
    }

    step(cpuCycles: number, read: DmcReadFn): void {
        this.stepFrameCounter(cpuCycles);

        const apuCyclesPerSample = cpuCyclesPerSample(this.region) / 2.0;
        this.cycleAccumulator += cpuCycles;
        while (this.cycleAccumulator >= 2) {
            this.cycleAccumulator -= 2;
            this.pulse1.tick();
            this.pulse2.tick();
            this.triangle.tick();
            this.noise.tick();
            this.dmc.tick(read);

            this.mixAccumulator += this.mixRaw();
            this.mixCount += 1;
            this.sampleAccumulator += 1.0;

            if (this.sampleAccumulator >= apuCyclesPerSample) {
                this.sampleAccumulator -= apuCyclesPerSample;
                if (this.mixCount > 0) {
                    this.lastDecimated = this.mixAccumulator / this.mixCount;
                    this.mixAccumulator = 0.0;
                    this.mixCount = 0;
                }
            }
        }
    }

    clockQuarterFrame(): void {
        this.pulse1.clockQuarterFrame();
        this.pulse2.clockQuarterFrame();
        this.triangle.clockQuarterFrame();
        this.noise.clockQuarterFrame();
    }

    clockHalfFrame(): void {
        this.pulse1.clockHalfFrame();
        this.pulse2.clockHalfFrame();
        this.triangle.clockHalfFrame();
        this.noise.clockHalfFrame();
    }

    mix(): number {
        const s = (this.pulse1.sample() + this.pulse2.sample() + this.triangle.sample() + this.noise.sample());
        return (s > 15) ? 15 : s;
    }

    output(): number {
        const clamped = this.lastDecimated;
        const LpfAlpha = 0.8192;
        const filtered = LpfAlpha * clamped + (1.0 - LpfAlpha) * this.lpfPrev;
        this.lpfPrev = filtered;
        const DcR = 0.99715;
        const dcOut = filtered - this.dcPrevX + DcR * this.dcPrevY;
        this.dcPrevX = filtered;
        this.dcPrevY = dcOut;
        return dcOut;
    }

    private scaledSample(idx: number, raw: number): number {
        if (idx >= Apu.ChannelCount || this.channelMuted[idx]) return 0.0;
        return raw * this.channelVolumes[idx];
    }

    private mixRaw(): number {
        const p1 = this.scaledSample(0, this.pulse1.sample());
        const p2 = this.scaledSample(1, this.pulse2.sample());
        const tri = this.scaledSample(2, this.triangle.sample());
        const noise = this.scaledSample(3, this.noise.sample());
        const dmc = this.scaledSample(4, this.dmc.sample());

        const pulseSum = p1 + p2;
        const pulseOut = (pulseSum > 0.0)
            ? (95.52 / (8128.0 / pulseSum + 100.0))
            : 0.0;

        const tndInner = tri / 8227.0 + noise / 12241.0 + dmc / 22638.0;
        const tndOut = (tndInner > 0.0)
            ? (163.67 / (1.0 / tndInner + 100.0))
            : 0.0;

        let mixed = (pulseOut + tndOut) * 2.0 - 1.0;
        if (mixed < -1.0) mixed = -1.0;
        if (mixed > 1.0) mixed = 1.0;
        return mixed;
    }

    setChannelVolume(idx: number, vol: number): void {
        if (idx < 0 || idx >= Apu.ChannelCount) return;
        if (vol < 0.0) vol = 0.0;
        if (vol > 1.0) vol = 1.0;
        this.channelVolumes[idx] = vol;
    }

    channelVolume(idx: number): number {
        return (idx < 0 || idx >= Apu.ChannelCount) ? 0.0 : this.channelVolumes[idx];
    }

    channelMutedAt(idx: number): boolean {
        return (idx < 0 || idx >= Apu.ChannelCount) ? true : this.channelMuted[idx];
    }

    toggleChannelMute(idx: number): boolean {
        if (idx < 0 || idx >= Apu.ChannelCount) return false;
        this.channelMuted[idx] = !this.channelMuted[idx];
        return this.channelMuted[idx];
    }

    setChannelMuted(idx: number, muted: boolean): void {
        if (idx < 0 || idx >= Apu.ChannelCount) return;
        this.channelMuted[idx] = muted;
    }

    setSelectedChannel(idx: number): void {
        this.selectedChannel = (idx > 4) ? 4 : idx;
    }

    resetChannelMix(): void {
        for (let i = 0; i < Apu.ChannelCount; ++i) {
            this.channelVolumes[i] = 1.0;
            this.channelMuted[i] = false;
        }
    }

    applyChannelVolumes(vols: Float32Array | number[]): void {
        for (let i = 0; i < vols.length && i < Apu.ChannelCount; ++i) {
            let v = vols[i];
            if (v < 0.0) v = 0.0;
            if (v > 1.0) v = 1.0;
            this.channelVolumes[i] = v;
        }
    }
}
