// Mapper 85 (VRC7 with YM2413 OPLL audio). Port of cores/c/src/vrc7.c.

import { Mapper, Mirroring } from '../mapper';
import { Opll } from '../audio/opll';

export class Vrc7 extends Mapper {
    private static readonly Prg8kSize = 8192;
    private static readonly Chr1kSize = 1024;
    private static readonly PrgRamSize = 8192;

    private _prgRom: Uint8Array;
    private _prgSize: number;
    private _chr: Uint8Array;
    private _chrSize: number;
    private _chrIsRam: boolean;
    private _prgRam: Uint8Array = new Uint8Array(Vrc7.PrgRamSize);
    private _hasBattery: boolean;
    private _prgBanks: Uint8Array = new Uint8Array(3);
    private _chrBanks: Uint8Array = new Uint8Array(8);
    private _mirrorHorizontal: boolean;
    private _irqLatch: number;
    private _irqCounter: number;
    private _irqLatchHigh: boolean;
    private _irqEnable: boolean;
    private _irqPending: boolean;
    private _opll: Opll = new Opll();

    constructor(prg: Uint8Array, prgSize: number, chr: Uint8Array | null, chrSize: number, mirroring: number, hasBattery: boolean) {
        super();
        this.mapperNum = 85;
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
        this._mirrorHorizontal = (mirroring === Mirroring.Horizontal);
        this._irqLatch = 0;
        this._irqCounter = 0;
        this._irqLatchHigh = false;
        this._irqEnable = false;
        this._irqPending = false;
        this._opll.init();
    }

    private prg8kCount(): number { const n = Math.floor(this._prgSize / Vrc7.Prg8kSize); return n > 0 ? n : 1; }
    private chr1kCount(): number { const n = Math.floor(this._chrSize / Vrc7.Chr1kSize); return n > 0 ? n : 1; }

    private prgReadBank(bank: number, offset: number): number {
        const count = this.prg8kCount();
        bank %= count;
        const idx = bank * Vrc7.Prg8kSize + offset;
        if (idx >= this._prgSize) return 0x00;
        return this._prgRom[idx];
    }

    readPrg(addr: number): number {
        if (addr >= 0x6000 && addr < 0x8000) return this._prgRam[(addr - 0x6000) & (Vrc7.PrgRamSize - 1)];
        const local = addr - 0x8000;
        const slot = Math.floor(local / Vrc7.Prg8kSize);
        const offset = local & (Vrc7.Prg8kSize - 1);
        if (slot < 3) {
            const bank = this._prgBanks[slot] % this.prg8kCount();
            return this.prgReadBank(bank, offset);
        }
        const last = this.prg8kCount() - 1;
        return this.prgReadBank(last, offset);
    }

    writePrg(addr: number, value: number): void {
        if (addr >= 0x6000 && addr < 0x8000) { this._prgRam[(addr - 0x6000) & (Vrc7.PrgRamSize - 1)] = value; return; }
        const reg = addr & 0xF03D;
        switch (reg) {
            case 0x8000: this._prgBanks[0] = value & 0x3F; break;
            case 0x8008: this._prgBanks[1] = value & 0x3F; break;
            case 0x9000: this._prgBanks[2] = value & 0x3F; break;
            case 0x9010: this._opll.writeAddr(value); break;
            case 0x9030: this._opll.writeData(value); break;
            case 0xB000: this._mirrorHorizontal = (value & 0x01) !== 0; break;
            case 0xC000: this._chrBanks[0] = value; break;
            case 0xC004: this._chrBanks[1] = value; break;
            case 0xC008: this._chrBanks[2] = value; break;
            case 0xC00C: this._chrBanks[3] = value; break;
            case 0xD000: this._chrBanks[4] = value; break;
            case 0xD004: this._chrBanks[5] = value; break;
            case 0xD008: this._chrBanks[6] = value; break;
            case 0xD00C: this._chrBanks[7] = value; break;
            case 0xE000:
                if (!this._irqLatchHigh) {
                    this._irqLatch = (this._irqLatch & 0xFF00) | value;
                    this._irqLatchHigh = true;
                } else {
                    this._irqLatch = (this._irqLatch & 0x00FF) | (value << 8);
                    this._irqLatchHigh = false;
                }
                break;
            case 0xE008:
                if (value & 0x02) {
                    this._irqEnable = true;
                    this._irqPending = false;
                    this._irqCounter = this._irqLatch;
                } else if (value & 0x01) {
                    this._irqEnable = true;
                    this._irqCounter = this._irqLatch;
                } else {
                    this._irqEnable = false;
                }
                break;
            case 0xE010: this._irqPending = false; break;
        }
    }

    readChr(addr: number): number {
        const slot = Math.floor(addr / Vrc7.Chr1kSize);
        const offset = addr & (Vrc7.Chr1kSize - 1);
        const count = this.chr1kCount();
        const bank = this._chrBanks[slot] % count;
        const idx = bank * Vrc7.Chr1kSize + offset;
        if (idx >= this._chrSize) return 0x00;
        return this._chr[idx];
    }

    writeChr(addr: number, value: number): void {
        if (!this._chrIsRam) return;
        const slot = Math.floor(addr / Vrc7.Chr1kSize);
        const offset = addr & (Vrc7.Chr1kSize - 1);
        const count = this.chr1kCount();
        const bank = this._chrBanks[slot] % count;
        const idx = bank * Vrc7.Chr1kSize + offset;
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
                if (this._irqEnable) this._irqPending = true;
            } else this._irqCounter = (this._irqCounter - 1) & 0xFFFF;
        }
        this._opll.clock(Math.floor(cpuCycles / 2));
    }

    expansionAudioSample(): number {
        const VRC7_GAIN = 0.6;
        return this._opll.sample() * VRC7_GAIN;
    }

    saveState(buf: Uint8Array | null): number {
        // Layout: scalars + prg_ram + opll regs + chr(if ram)
        const total = 32 + Vrc7.PrgRamSize + 0x80 + (this._chrIsRam ? this._chrSize : 0);
        if (buf === null) return total;
        let p = 0;
        for (let i = 0; i < 3; ++i) buf[p++] = this._prgBanks[i];
        buf[p++] = this._mirrorHorizontal ? 1 : 0;
        new DataView(buf.buffer).setUint16(p, this._irqLatch, true); p += 2;
        new DataView(buf.buffer).setUint16(p, this._irqCounter, true); p += 2;
        buf[p++] = this._irqLatchHigh ? 1 : 0;
        buf[p++] = this._irqEnable ? 1 : 0;
        buf[p++] = this._irqPending ? 1 : 0;
        for (let i = 0; i < 8; ++i) buf[p++] = this._chrBanks[i];
        while (p < 32) buf[p++] = 0;
        buf.set(this._prgRam, p); p += Vrc7.PrgRamSize;
        buf.set(this._opll.regs, p); p += 0x80;
        if (this._chrIsRam && this._chrSize > 0) {
            buf.set(this._chr.subarray(0, this._chrSize), p);
            p += this._chrSize;
        }
        return p;
    }

    loadState(buf: Uint8Array, len: number): boolean {
        const need = 32 + Vrc7.PrgRamSize + 0x80 + (this._chrIsRam ? this._chrSize : 0);
        if (len < need) return false;
        let p = 0;
        for (let i = 0; i < 3; ++i) this._prgBanks[i] = buf[p++];
        this._mirrorHorizontal = buf[p++] !== 0;
        this._irqLatch = new DataView(buf.buffer).getUint16(p, true); p += 2;
        this._irqCounter = new DataView(buf.buffer).getUint16(p, true); p += 2;
        this._irqLatchHigh = buf[p++] !== 0;
        this._irqEnable = buf[p++] !== 0;
        this._irqPending = buf[p++] !== 0;
        for (let i = 0; i < 8; ++i) this._chrBanks[i] = buf[p++];
        p = 32;
        this._prgRam.set(buf.subarray(p, p + Vrc7.PrgRamSize)); p += Vrc7.PrgRamSize;
        this._opll.regs.set(buf.subarray(p, p + 0x80)); p += 0x80;
        if (this._chrIsRam && this._chrSize > 0) {
            this._chr.set(buf.subarray(p, p + this._chrSize));
            p += this._chrSize;
        }
        return true;
    }
}
