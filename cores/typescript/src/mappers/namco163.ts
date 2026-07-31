// Mapper 19 (Namco 163 with wavetable expansion audio).
// Port of cores/c/src/namco163.c.

import { Mapper, Mirroring } from '../mapper';

class WaveChan {
    freq: number = 0;
    length: number = 0;
    volume: number = 0;
    offset: number = 0;
    phase: number = 0;
    enabled: boolean = false;
}

export class Namco163 extends Mapper {
    private static readonly Prg8kSize = 8192;
    private static readonly Chr1kSize = 1024;
    private static readonly PrgRamSize = 8192;
    private static readonly WaveRamSize = 0x80;
    private static readonly WaveChannels = 8;

    private _prgRom: Uint8Array;
    private _prgSize: number;
    private _chr: Uint8Array;
    private _chrSize: number;
    private _chrIsRam: boolean;
    private _prgRam: Uint8Array = new Uint8Array(Namco163.PrgRamSize);
    private _hasBattery: boolean;
    private _prgBanks: Uint8Array = new Uint8Array(4);
    private _chrBanks: Uint8Array = new Uint8Array(8);
    private _mirror: number;
    private _prgRamEnable: boolean;
    private _prgRamWriteProtect: boolean;
    private _irqLatch: number;
    private _irqCounter: number;
    private _irqEnable: boolean;
    private _irqPending: boolean;
    private _irqLatchHigh: boolean;
    private _waveRam: Uint8Array = new Uint8Array(Namco163.WaveRamSize);
    private _waveAddr: number;
    private _chans: WaveChan[] = Array.from({ length: Namco163.WaveChannels }, () => new WaveChan());

    constructor(prg: Uint8Array, prgSize: number, chr: Uint8Array | null, chrSize: number, mirroring: number, hasBattery: boolean) {
        super();
        this.mapperNum = 19;
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
        this._mirror = mirroring;
        this._prgRamEnable = false;
        this._prgRamWriteProtect = false;
        this._irqLatch = 0;
        this._irqCounter = 0;
        this._irqEnable = false;
        this._irqPending = false;
        this._irqLatchHigh = false;
        this._waveAddr = 0;
    }

    private prg8kCount(): number { const n = Math.floor(this._prgSize / Namco163.Prg8kSize); return n > 0 ? n : 1; }
    private chr1kCount(): number { const n = Math.floor(this._chrSize / Namco163.Chr1kSize); return n > 0 ? n : 1; }

    private prgReadBank(bank: number, offset: number): number {
        const count = this.prg8kCount();
        bank %= count;
        const idx = bank * Namco163.Prg8kSize + offset;
        if (idx >= this._prgSize) return 0x00;
        return this._prgRom[idx];
    }

    private waveRead(): number { return this._waveRam[this._waveAddr & 0x7F]; }
    private waveWrite(value: number): void {
        const a = this._waveAddr & 0x7F;
        this._waveRam[a] = value;
        if (this._waveAddr & 0x80) this._waveAddr = (this._waveAddr & 0x80) | ((this._waveAddr + 1) & 0x7F);
    }

    private chanParamRead(addr: number): number {
        const off = addr - 0x5000;
        const ch = Math.floor(off / 8);
        const sub = off & 0x07;
        if (ch >= Namco163.WaveChannels) return 0;
        const c = this._chans[ch];
        switch (sub) {
            case 0: return c.freq & 0xFF;
            case 1: return ((c.freq >> 8) & 0x0F) | ((c.length & 0x0F) << 4);
            case 2: return c.volume & 0x0F;
            case 3: return (c.offset & 0x60) | (c.enabled ? 0x80 : 0x00);
            default: return 0;
        }
    }

    private chanParamWrite(addr: number, value: number): void {
        const off = addr - 0x5000;
        const ch = Math.floor(off / 8);
        const sub = off & 0x07;
        if (ch >= Namco163.WaveChannels) return;
        const c = this._chans[ch];
        switch (sub) {
            case 0: c.freq = (c.freq & 0x0F00) | value; break;
            case 1:
                c.freq = (c.freq & 0x00FF) | ((value & 0x0F) << 8);
                c.length = (value >> 4) & 0x0F;
                break;
            case 2: c.volume = value & 0x0F; break;
            case 3:
                c.offset = value & 0x60;
                c.enabled = (value & 0x80) !== 0;
                break;
        }
    }

    private chanSample(ch: number): number {
        const c = this._chans[ch];
        if (!c.enabled) return 0;
        let lengthNibbles = (c.length + 1) * 8;
        const phaseNibble = (c.phase >> 16) % lengthNibbles;
        if (lengthNibbles === 0) lengthNibbles = 1;
        const byteIdx = (c.offset + Math.floor(phaseNibble / 2)) & 0x7F;
        const byte = this._waveRam[byteIdx];
        const nibble = (phaseNibble & 1) ? (byte >> 4) : (byte & 0x0F);
        return Math.floor((nibble * c.volume) / 15);
    }

    readPrg(addr: number): number {
        if (addr >= 0x6000 && addr < 0x8000) {
            if (this._prgRamEnable) return this._prgRam[(addr - 0x6000) & (Namco163.PrgRamSize - 1)];
            return 0x00;
        }
        if (addr >= 0x4800 && addr < 0x5000) return this.waveRead();
        if (addr >= 0x5000 && addr < 0x5800) return this.chanParamRead(addr);
        const local = addr - 0x8000;
        const slot = Math.floor(local / Namco163.Prg8kSize);
        const offset = local & (Namco163.Prg8kSize - 1);
        const bank = this._prgBanks[slot] % this.prg8kCount();
        return this.prgReadBank(bank, offset);
    }

    writePrg(addr: number, value: number): void {
        if (addr >= 0x6000 && addr < 0x8000) {
            if (this._prgRamEnable && !this._prgRamWriteProtect)
                this._prgRam[(addr - 0x6000) & (Namco163.PrgRamSize - 1)] = value;
            return;
        }
        if (addr >= 0x4800 && addr < 0x5000) { this.waveWrite(value); return; }
        if (addr >= 0x5000 && addr < 0x5800) { this.chanParamWrite(addr, value); return; }
        const reg = addr & 0xF801;
        switch (reg) {
            case 0x8000: this._chrBanks[0] = value; break;
            case 0x8800: this._chrBanks[1] = value; break;
            case 0x9000: this._chrBanks[2] = value; break;
            case 0x9800: this._chrBanks[3] = value; break;
            case 0xA000: this._chrBanks[4] = value; break;
            case 0xA800: this._chrBanks[5] = value; break;
            case 0xB000: this._chrBanks[6] = value; break;
            case 0xB800: this._chrBanks[7] = value; break;
            case 0xC000: this._prgBanks[0] = value & 0x3F; break;
            case 0xC800: this._prgBanks[1] = value & 0x3F; break;
            case 0xD000: this._prgBanks[2] = value & 0x3F; break;
            case 0xD800: this._prgBanks[3] = value & 0x3F; break;
            case 0xE000:
                switch (value & 0x03) {
                    case 0: this._mirror = Mirroring.Vertical; break;
                    case 1: this._mirror = Mirroring.Horizontal; break;
                    case 2: this._mirror = Mirroring.SingleScreen0; break;
                    default: this._mirror = Mirroring.SingleScreen1; break;
                }
                break;
            case 0xE800:
                this._prgRamEnable = (value & 0x80) !== 0;
                this._prgRamWriteProtect = (value & 0x40) !== 0;
                break;
            case 0xF000:
                if (!this._irqLatchHigh) {
                    this._irqLatch = (this._irqLatch & 0xFF00) | value;
                    this._irqLatchHigh = true;
                } else {
                    this._irqLatch = (this._irqLatch & 0x00FF) | (value << 8);
                    this._irqLatchHigh = false;
                }
                break;
            case 0xF800: this._waveAddr = value; break;
        }
    }

    readChr(addr: number): number {
        const slot = Math.floor(addr / Namco163.Chr1kSize);
        const offset = addr & (Namco163.Chr1kSize - 1);
        const count = this.chr1kCount();
        const bank = this._chrBanks[slot] % count;
        const idx = bank * Namco163.Chr1kSize + offset;
        if (idx >= this._chrSize) return 0x00;
        return this._chr[idx];
    }

    writeChr(addr: number, value: number): void {
        if (!this._chrIsRam) return;
        const slot = Math.floor(addr / Namco163.Chr1kSize);
        const offset = addr & (Namco163.Chr1kSize - 1);
        const count = this.chr1kCount();
        const bank = this._chrBanks[slot] % count;
        const idx = bank * Namco163.Chr1kSize + offset;
        if (idx < this._chrSize) this._chr[idx] = value;
    }

    mirrorMode(): number { return this._mirror; }
    chrIsRam(): boolean { return this._chrIsRam; }
    hasBattery(): boolean { return this._hasBattery; }
    irqPending(): boolean { return this._irqPending; }

    clockCpu(cpuCycles: number): void {
        for (let i = 0; i < cpuCycles; ++i) {
            if (this._irqCounter === 0) {
                this._irqCounter = this._irqLatch;
                if (this._irqEnable) this._irqPending = true;
            } else this._irqCounter = (this._irqCounter - 1) & 0xFFFF;
        }
        let active = 0;
        for (let ch = 0; ch < Namco163.WaveChannels; ++ch) if (this._chans[ch].enabled) ++active;
        if (active === 0) active = 1;
        for (let i = 0; i < cpuCycles; ++i) {
            for (let ch = 0; ch < Namco163.WaveChannels; ++ch) {
                const c = this._chans[ch];
                if (!c.enabled) continue;
                const inc = c.freq << 8;
                c.phase = (c.phase + Math.floor(inc / active)) >>> 0;
            }
        }
    }

    expansionAudioSample(): number {
        let sum = 0;
        for (let ch = 0; ch < Namco163.WaveChannels; ++ch) sum += this.chanSample(ch);
        const N163_GAIN = 0.4;
        let v = sum / 120.0;
        if (v < -1.0) v = -1.0;
        if (v > 1.0) v = 1.0;
        return v * N163_GAIN;
    }

    saveState(buf: Uint8Array | null): number {
        const total = 64 + Namco163.PrgRamSize + Namco163.WaveRamSize + (this._chrIsRam ? this._chrSize : 0);
        if (buf === null) return total;
        let p = 0;
        for (let i = 0; i < 4; ++i) buf[p++] = this._prgBanks[i];
        for (let i = 0; i < 8; ++i) buf[p++] = this._chrBanks[i];
        buf[p++] = this._mirror;
        buf[p++] = this._prgRamEnable ? 1 : 0;
        buf[p++] = this._prgRamWriteProtect ? 1 : 0;
        new DataView(buf.buffer).setUint16(p, this._irqLatch, true); p += 2;
        new DataView(buf.buffer).setUint16(p, this._irqCounter, true); p += 2;
        buf[p++] = this._irqEnable ? 1 : 0;
        buf[p++] = this._irqPending ? 1 : 0;
        buf[p++] = this._irqLatchHigh ? 1 : 0;
        buf[p++] = this._waveAddr;
        for (let ch = 0; ch < Namco163.WaveChannels; ++ch) {
            const c = this._chans[ch];
            new DataView(buf.buffer).setUint16(p, c.freq, true); p += 2;
            buf[p++] = c.length; buf[p++] = c.volume; buf[p++] = c.offset;
            new DataView(buf.buffer).setUint32(p, c.phase, true); p += 4;
            buf[p++] = c.enabled ? 1 : 0;
        }
        while (p < 64) buf[p++] = 0;
        buf.set(this._prgRam, p); p += Namco163.PrgRamSize;
        buf.set(this._waveRam, p); p += Namco163.WaveRamSize;
        if (this._chrIsRam && this._chrSize > 0) {
            buf.set(this._chr.subarray(0, this._chrSize), p);
            p += this._chrSize;
        }
        return p;
    }

    loadState(buf: Uint8Array, len: number): boolean {
        const need = 64 + Namco163.PrgRamSize + Namco163.WaveRamSize + (this._chrIsRam ? this._chrSize : 0);
        if (len < need) return false;
        let p = 0;
        for (let i = 0; i < 4; ++i) this._prgBanks[i] = buf[p++];
        for (let i = 0; i < 8; ++i) this._chrBanks[i] = buf[p++];
        this._mirror = buf[p++];
        this._prgRamEnable = buf[p++] !== 0;
        this._prgRamWriteProtect = buf[p++] !== 0;
        this._irqLatch = new DataView(buf.buffer).getUint16(p, true); p += 2;
        this._irqCounter = new DataView(buf.buffer).getUint16(p, true); p += 2;
        this._irqEnable = buf[p++] !== 0;
        this._irqPending = buf[p++] !== 0;
        this._irqLatchHigh = buf[p++] !== 0;
        this._waveAddr = buf[p++];
        for (let ch = 0; ch < Namco163.WaveChannels; ++ch) {
            const c = this._chans[ch];
            c.freq = new DataView(buf.buffer).getUint16(p, true); p += 2;
            c.length = buf[p++]; c.volume = buf[p++]; c.offset = buf[p++];
            c.phase = new DataView(buf.buffer).getUint32(p, true); p += 4;
            c.enabled = buf[p++] !== 0;
        }
        p = 64;
        this._prgRam.set(buf.subarray(p, p + Namco163.PrgRamSize)); p += Namco163.PrgRamSize;
        this._waveRam.set(buf.subarray(p, p + Namco163.WaveRamSize)); p += Namco163.WaveRamSize;
        if (this._chrIsRam && this._chrSize > 0) {
            this._chr.set(buf.subarray(p, p + this._chrSize));
            p += this._chrSize;
        }
        return true;
    }
}
