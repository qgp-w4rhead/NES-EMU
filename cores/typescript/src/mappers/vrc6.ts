// Mappers 24 (VRC6a) and 26 (VRC6b). Port of cores/c/src/vrc6.c.

import { Mapper, Mirroring } from '../mapper';

class PulseChannel {
    control: number = 0;
    period: number = 0;
    timer: number = 0;
    step: number = 0;
    enabled: boolean = false;
    scale: number = 0;
}

class SawChannel {
    rate: number = 0;
    period: number = 0;
    timer: number = 0;
    accum: number = 0;
    enabled: boolean = false;
}

export class Vrc6 extends Mapper {
    private static readonly Prg16kSize = 16384;
    private static readonly Prg8kSize = 8192;
    private static readonly Chr1kSize = 1024;
    private static readonly PrgRamSize = 8192;

    private _prgRom: Uint8Array;
    private _prgSize: number;
    private _chr: Uint8Array;
    private _chrSize: number;
    private _chrIsRam: boolean;
    private _prgRam: Uint8Array = new Uint8Array(Vrc6.PrgRamSize);
    private _hasBattery: boolean;
    private _prgBank16k: number;
    private _prgBank8k: number;
    private _chrBanks: Uint8Array = new Uint8Array(8);
    private _mirrorHorizontal: boolean;
    private _irqLatch: number;
    private _irqCounter: number;
    private _irqEnable: boolean;
    private _irqEnableAfterAck: boolean;
    private _irqPending: boolean;
    private _pulse1: PulseChannel = new PulseChannel();
    private _pulse2: PulseChannel = new PulseChannel();
    private _saw: SawChannel = new SawChannel();
    private _swapAddr: boolean;

    constructor(prg: Uint8Array, prgSize: number, chr: Uint8Array | null, chrSize: number, mirroring: number, hasBattery: boolean, is26: boolean) {
        super();
        this.mapperNum = is26 ? 26 : 24;
        this._prgSize = prgSize;
        this._prgRom = new Uint8Array(prgSize);
        if (prgSize > 0) this._prgRom.set(prg.subarray(0, prgSize));
        if (chrSize > 0) {
            this._chrSize = chrSize;
            this._chr = new Uint8Array(chrSize);
            this._chr.set(chr!.subarray(0, chrSize));
            this._chrIsRam = false;
        } else {
            this._chrSize = 8192;
            this._chr = new Uint8Array(8192);
            this._chrIsRam = true;
        }
        this._hasBattery = hasBattery;
        this._prgBank16k = 0;
        this._prgBank8k = 0;
        this._mirrorHorizontal = (mirroring === Mirroring.Horizontal);
        this._irqLatch = 0;
        this._irqCounter = 0;
        this._irqEnable = false;
        this._irqEnableAfterAck = false;
        this._irqPending = false;
        this._swapAddr = is26;
    }

    private prg16kCount(): number { const n = Math.floor(this._prgSize / Vrc6.Prg16kSize); return n > 0 ? n : 1; }
    private prg8kCount(): number { const n = Math.floor(this._prgSize / Vrc6.Prg8kSize); return n > 0 ? n : 1; }
    private chr1kCount(): number { const n = Math.floor(this._chrSize / Vrc6.Chr1kSize); return n > 0 ? n : 1; }

    private decode(addr: number): number {
        const masked = addr & 0xF003;
        if (!this._swapAddr) return masked;
        const a0 = (masked & 0x001) << 1;
        const a1 = (masked & 0x002) >> 1;
        return (masked & 0xFFFC) | a0 | a1;
    }

    private prgRead8k(bank: number, offset: number): number {
        const count = this.prg8kCount();
        bank %= count;
        const idx = bank * Vrc6.Prg8kSize + offset;
        if (idx >= this._prgSize) return 0x00;
        return this._prgRom[idx];
    }
    private prgRead16k(bank: number, offset: number): number {
        const count = this.prg16kCount();
        bank %= count;
        const idx = bank * Vrc6.Prg16kSize + offset;
        if (idx >= this._prgSize) return 0x00;
        return this._prgRom[idx];
    }

    private pulseDuty(p: PulseChannel): number { return ((p.control >> 4) & 0x07) + 1; }
    private pulseVolume(p: PulseChannel): number { return p.control & 0x0F; }

    private pulseClock(p: PulseChannel, cpuCycles: number): void {
        if (!p.enabled) return;
        let period = p.period;
        if (p.scale & 1) { period = period * 2; if (period === 0) period = 1; }
        if (period === 0) period = 1;
        for (let i = 0; i < cpuCycles; ++i) {
            if (p.timer === 0) { p.timer = period; p.step = (p.step + 1) & 0x0F; }
            else p.timer = (p.timer - 1) & 0xFFFF;
        }
    }

    private sawClock(saw: SawChannel, cpuCycles: number): void {
        if (!saw.enabled) return;
        let period = saw.period;
        if (period === 0) period = 1;
        for (let i = 0; i < cpuCycles; ++i) {
            if (saw.timer === 0) {
                saw.timer = period;
                saw.accum = (saw.accum + (saw.rate & 0x3F)) & 0xFF;
                if (saw.accum >= 0x80) saw.accum = 0;
            } else saw.timer = (saw.timer - 1) & 0xFFFF;
        }
    }

    private pulse1Sample(): number {
        if (this._pulse1.enabled && this._pulse1.step < this.pulseDuty(this._pulse1)) return this.pulseVolume(this._pulse1);
        return 0;
    }
    private pulse2Sample(): number {
        if (this._pulse2.enabled && this._pulse2.step < this.pulseDuty(this._pulse2)) return this.pulseVolume(this._pulse2);
        return 0;
    }
    private sawSample(): number { return this._saw.enabled ? (this._saw.accum >> 2) : 0; }

    readPrg(addr: number): number {
        if (addr >= 0x6000 && addr < 0x8000) return this._prgRam[(addr - 0x6000) & (Vrc6.PrgRamSize - 1)];
        const local = addr - 0x8000;
        if (local < Vrc6.Prg16kSize) {
            const bank = this._prgBank16k % this.prg16kCount();
            return this.prgRead16k(bank, local);
        }
        if (local < Vrc6.Prg16kSize + Vrc6.Prg8kSize) {
            const off = local - Vrc6.Prg16kSize;
            const bank = this._prgBank8k % this.prg8kCount();
            return this.prgRead8k(bank, off);
        }
        const off = local - Vrc6.Prg16kSize - Vrc6.Prg8kSize;
        const last = this.prg8kCount() - 1;
        return this.prgRead8k(last, off);
    }

    writePrg(addr: number, value: number): void {
        if (addr >= 0x6000 && addr < 0x8000) { this._prgRam[(addr - 0x6000) & (Vrc6.PrgRamSize - 1)] = value; return; }
        const reg = this.decode(addr);
        switch (reg) {
            case 0x8000: this._prgBank16k = value & 0x3F; break;
            case 0x9000: this._pulse1.control = value; break;
            case 0x9001: this._pulse1.period = (this._pulse1.period & 0x0F00) | value; break;
            case 0x9002:
                this._pulse1.period = (this._pulse1.period & 0x00FF) | ((value & 0x0F) << 8);
                this._pulse1.enabled = (value & 0x80) !== 0;
                break;
            case 0x9003: this._pulse1.scale = value; break;
            case 0xA000: this._pulse2.control = value; break;
            case 0xA001: this._pulse2.period = (this._pulse2.period & 0x0F00) | value; break;
            case 0xA002:
                this._pulse2.period = (this._pulse2.period & 0x00FF) | ((value & 0x0F) << 8);
                this._pulse2.enabled = (value & 0x80) !== 0;
                break;
            case 0xB000: this._saw.rate = value; break;
            case 0xB001: this._saw.period = (this._saw.period & 0x0F00) | value; break;
            case 0xB002:
                this._saw.period = (this._saw.period & 0x00FF) | ((value & 0x0F) << 8);
                this._saw.enabled = (value & 0x80) !== 0;
                break;
            case 0xB003: this._mirrorHorizontal = (value & 0x01) !== 0; break;
            case 0xC000: this._prgBank8k = value & 0x3F; break;
            case 0xD000: this._chrBanks[0] = value; break;
            case 0xD001: this._chrBanks[1] = value; break;
            case 0xD002: this._chrBanks[2] = value; break;
            case 0xD003: this._chrBanks[3] = value; break;
            case 0xE000: this._chrBanks[4] = value; break;
            case 0xE001: this._chrBanks[5] = value; break;
            case 0xE002: this._chrBanks[6] = value; break;
            case 0xE003: this._chrBanks[7] = value; break;
            case 0xF000: this._irqLatch = value; break;
            case 0xF001:
                this._irqEnable = (value & 0x01) !== 0;
                this._irqCounter = this._irqLatch;
                break;
            case 0xF002:
                this._irqEnableAfterAck = (value & 0x02) !== 0;
                this._irqEnable = (value & 0x01) !== 0;
                this._irqPending = false;
                break;
        }
    }

    readChr(addr: number): number {
        const slot = Math.floor(addr / Vrc6.Chr1kSize);
        const offset = addr & (Vrc6.Chr1kSize - 1);
        const count = this.chr1kCount();
        const bank = this._chrBanks[slot] % count;
        const idx = bank * Vrc6.Chr1kSize + offset;
        if (idx >= this._chrSize) return 0x00;
        return this._chr[idx];
    }

    writeChr(addr: number, value: number): void {
        if (!this._chrIsRam) return;
        const slot = Math.floor(addr / Vrc6.Chr1kSize);
        const offset = addr & (Vrc6.Chr1kSize - 1);
        const count = this.chr1kCount();
        const bank = this._chrBanks[slot] % count;
        const idx = bank * Vrc6.Chr1kSize + offset;
        if (idx < this._chrSize) this._chr[idx] = value;
    }

    mirrorMode(): number { return this._mirrorHorizontal ? Mirroring.Horizontal : Mirroring.Vertical; }
    chrIsRam(): boolean { return this._chrIsRam; }
    hasBattery(): boolean { return this._hasBattery; }
    irqPending(): boolean { return this._irqPending; }

    clockCpu(cpuCycles: number): void {
        for (let i = 0; i < cpuCycles; ++i) {
            if (this._irqCounter === 0) {
                this._irqCounter = this._irqLatch;
                if (this._irqEnable) {
                    this._irqPending = true;
                    if (!this._irqEnableAfterAck) this._irqEnable = false;
                }
            } else {
                this._irqCounter = (this._irqCounter - 1) & 0xFF;
            }
        }
        this.pulseClock(this._pulse1, cpuCycles);
        this.pulseClock(this._pulse2, cpuCycles);
        this.sawClock(this._saw, cpuCycles);
    }

    expansionAudioSample(): number {
        const p1 = this.pulse1Sample();
        const p2 = this.pulse2Sample();
        const saw = this.sawSample();
        let sum = p1 + p2;
        sum = sum > 63 ? 63 : (sum + saw);
        if (sum > 63) sum = 63;
        const VRC6_GAIN = 0.75;
        const normalized = (sum / 63.0) * 2.0 - 1.0;
        return normalized * VRC6_GAIN;
    }

    saveState(buf: Uint8Array | null): number {
        // Layout: scalar fields + prg_ram + chr(if ram)
        const total = 64 + Vrc6.PrgRamSize + (this._chrIsRam ? this._chrSize : 0);
        if (buf === null) return total;
        let p = 0;
        buf[p++] = this._prgBank16k; buf[p++] = this._prgBank8k;
        buf[p++] = this._mirrorHorizontal ? 1 : 0;
        buf[p++] = this._irqLatch; buf[p++] = this._irqCounter;
        buf[p++] = this._irqEnable ? 1 : 0;
        buf[p++] = this._irqEnableAfterAck ? 1 : 0;
        buf[p++] = this._irqPending ? 1 : 0;
        buf[p++] = this._swapAddr ? 1 : 0;
        // pulse1
        buf[p++] = this._pulse1.control;
        new DataView(buf.buffer).setUint16(p, this._pulse1.period, true); p += 2;
        new DataView(buf.buffer).setUint16(p, this._pulse1.timer, true); p += 2;
        buf[p++] = this._pulse1.step;
        buf[p++] = this._pulse1.enabled ? 1 : 0;
        buf[p++] = this._pulse1.scale;
        // pulse2
        buf[p++] = this._pulse2.control;
        new DataView(buf.buffer).setUint16(p, this._pulse2.period, true); p += 2;
        new DataView(buf.buffer).setUint16(p, this._pulse2.timer, true); p += 2;
        buf[p++] = this._pulse2.step;
        buf[p++] = this._pulse2.enabled ? 1 : 0;
        buf[p++] = this._pulse2.scale;
        // saw
        buf[p++] = this._saw.rate;
        new DataView(buf.buffer).setUint16(p, this._saw.period, true); p += 2;
        new DataView(buf.buffer).setUint16(p, this._saw.timer, true); p += 2;
        buf[p++] = this._saw.accum;
        buf[p++] = this._saw.enabled ? 1 : 0;
        for (let i = 0; i < 8; ++i) buf[p++] = this._chrBanks[i];
        while (p < 64) buf[p++] = 0;
        buf.set(this._prgRam, p); p += Vrc6.PrgRamSize;
        if (this._chrIsRam && this._chrSize > 0) {
            buf.set(this._chr.subarray(0, this._chrSize), p);
            p += this._chrSize;
        }
        return p;
    }

    loadState(buf: Uint8Array, len: number): boolean {
        const need = 64 + Vrc6.PrgRamSize + (this._chrIsRam ? this._chrSize : 0);
        if (len < need) return false;
        let p = 0;
        this._prgBank16k = buf[p++]; this._prgBank8k = buf[p++];
        this._mirrorHorizontal = buf[p++] !== 0;
        this._irqLatch = buf[p++]; this._irqCounter = buf[p++];
        this._irqEnable = buf[p++] !== 0;
        this._irqEnableAfterAck = buf[p++] !== 0;
        this._irqPending = buf[p++] !== 0;
        this._swapAddr = buf[p++] !== 0;
        this._pulse1.control = buf[p++];
        this._pulse1.period = new DataView(buf.buffer).getUint16(p, true); p += 2;
        this._pulse1.timer = new DataView(buf.buffer).getUint16(p, true); p += 2;
        this._pulse1.step = buf[p++];
        this._pulse1.enabled = buf[p++] !== 0;
        this._pulse1.scale = buf[p++];
        this._pulse2.control = buf[p++];
        this._pulse2.period = new DataView(buf.buffer).getUint16(p, true); p += 2;
        this._pulse2.timer = new DataView(buf.buffer).getUint16(p, true); p += 2;
        this._pulse2.step = buf[p++];
        this._pulse2.enabled = buf[p++] !== 0;
        this._pulse2.scale = buf[p++];
        this._saw.rate = buf[p++];
        this._saw.period = new DataView(buf.buffer).getUint16(p, true); p += 2;
        this._saw.timer = new DataView(buf.buffer).getUint16(p, true); p += 2;
        this._saw.accum = buf[p++];
        this._saw.enabled = buf[p++] !== 0;
        for (let i = 0; i < 8; ++i) this._chrBanks[i] = buf[p++];
        p = 64;
        this._prgRam.set(buf.subarray(p, p + Vrc6.PrgRamSize)); p += Vrc6.PrgRamSize;
        if (this._chrIsRam && this._chrSize > 0) {
            this._chr.set(buf.subarray(p, p + this._chrSize));
            p += this._chrSize;
        }
        return true;
    }
}
