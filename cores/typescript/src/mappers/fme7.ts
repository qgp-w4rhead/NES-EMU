// Mapper 69 (FME-7 / Sunsoft 5B). Port of cores/c/src/fme7.c.

import { Mapper, Mirroring } from '../mapper';
import { Ym2149 } from '../audio/ym2149';

export class Fme7 extends Mapper {
    private static readonly PrgBankSize = 8192;
    private static readonly Chr1kSize = 1024;
    private static readonly PrgRamSize = 8192;

    private _prgRom: Uint8Array;
    private _prgSize: number;
    private _chr: Uint8Array;
    private _chrSize: number;
    private _chrIsRam: boolean;
    private _prgRam: Uint8Array = new Uint8Array(Fme7.PrgRamSize);
    private _hasBattery: boolean;
    private _command: number;
    private _prgBanks: Uint8Array = new Uint8Array(4);
    private _chrBanks: Uint8Array = new Uint8Array(8);
    private _mirrorHorizontal: boolean;
    private _prgRamEnable: boolean;
    private _prgRamWriteProtect: boolean;
    private _irqLatch: number;
    private _irqCounter: number;
    private _irqLatchHigh: boolean;
    private _irqEnable: boolean;
    private _irqPending: boolean;
    private _ym2149: Ym2149 = new Ym2149();

    constructor(prg: Uint8Array, prgSize: number, chr: Uint8Array | null, chrSize: number, mirroring: number, hasBattery: boolean) {
        super();
        this.mapperNum = 69;
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
        this._command = 0;
        this._mirrorHorizontal = (mirroring === Mirroring.Horizontal);
        this._prgRamEnable = false;
        this._prgRamWriteProtect = false;
        this._irqLatch = 0;
        this._irqCounter = 0;
        this._irqLatchHigh = false;
        this._irqEnable = false;
        this._irqPending = false;
        this._ym2149.init();
    }

    private prgBankCount(): number { const n = Math.floor(this._prgSize / Fme7.PrgBankSize); return n > 0 ? n : 1; }
    private chr1kCount(): number { const n = Math.floor(this._chrSize / Fme7.Chr1kSize); return n > 0 ? n : 1; }

    private prgReadBank(bank: number, offset: number): number {
        const count = this.prgBankCount();
        bank %= count;
        const idx = bank * Fme7.PrgBankSize + offset;
        if (idx >= this._prgSize) return 0x00;
        return this._prgRom[idx];
    }

    private writeData(value: number): void {
        const cmd = this._command & 0x0F;
        if (cmd <= 7) {
            this._chrBanks[cmd & 0x07] = value;
        } else {
            switch (cmd) {
                case 8: this._prgBanks[0] = value & 0x3F; break;
                case 9: this._prgBanks[1] = value & 0x3F; break;
                case 10: this._prgBanks[2] = value & 0x3F; break;
                case 11: this._prgBanks[3] = value & 0x3F; break;
                case 12: this._mirrorHorizontal = (value & 0x01) !== 0; break;
                case 13:
                    this._prgRamEnable = (value & 0x80) !== 0;
                    this._prgRamWriteProtect = (value & 0x40) !== 0;
                    break;
                case 14:
                    if (!this._irqLatchHigh) {
                        this._irqLatch = (this._irqLatch & 0xFF00) | value;
                        this._irqLatchHigh = true;
                    } else {
                        this._irqLatch = (this._irqLatch & 0x00FF) | (value << 8);
                        this._irqLatchHigh = false;
                    }
                    break;
                case 15:
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
            }
        }
    }

    readPrg(addr: number): number {
        if (addr >= 0x6000 && addr < 0x8000) {
            if (this._prgRamEnable) return this._prgRam[(addr - 0x6000) & (Fme7.PrgRamSize - 1)];
            return 0x00;
        }
        const local = addr - 0x8000;
        const slot = Math.floor(local / Fme7.PrgBankSize);
        const offset = local & (Fme7.PrgBankSize - 1);
        const bank = this._prgBanks[slot] % this.prgBankCount();
        return this.prgReadBank(bank, offset);
    }

    writePrg(addr: number, value: number): void {
        if (addr >= 0x6000 && addr < 0x8000) {
            if (this._prgRamEnable && !this._prgRamWriteProtect)
                this._prgRam[(addr - 0x6000) & (Fme7.PrgRamSize - 1)] = value;
            return;
        }
        switch (addr) {
            case 0x8000: this._command = value; break;
            case 0x8001: this.writeData(value); break;
            case 0xC000: this._ym2149.writeAddr(value); break;
            case 0xE000: this._ym2149.writeData(value); break;
        }
    }

    readChr(addr: number): number {
        const slot = Math.floor(addr / Fme7.Chr1kSize);
        const offset = addr & (Fme7.Chr1kSize - 1);
        const count = this.chr1kCount();
        const bank = this._chrBanks[slot] % count;
        const idx = bank * Fme7.Chr1kSize + offset;
        if (idx >= this._chrSize) return 0x00;
        return this._chr[idx];
    }

    writeChr(addr: number, value: number): void {
        if (!this._chrIsRam) return;
        const slot = Math.floor(addr / Fme7.Chr1kSize);
        const offset = addr & (Fme7.Chr1kSize - 1);
        const count = this.chr1kCount();
        const bank = this._chrBanks[slot] % count;
        const idx = bank * Fme7.Chr1kSize + offset;
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
        this._ym2149.clock(Math.floor(cpuCycles / 2));
    }

    expansionAudioSample(): number {
        const S5B_GAIN = 0.5;
        return this._ym2149.sample() * S5B_GAIN;
    }

    saveState(buf: Uint8Array | null): number {
        const total = 32 + Fme7.PrgRamSize + 16 + (this._chrIsRam ? this._chrSize : 0);
        if (buf === null) return total;
        let p = 0;
        buf[p++] = this._command;
        buf[p++] = this._mirrorHorizontal ? 1 : 0;
        buf[p++] = this._prgRamEnable ? 1 : 0;
        buf[p++] = this._prgRamWriteProtect ? 1 : 0;
        new DataView(buf.buffer).setUint16(p, this._irqLatch, true); p += 2;
        new DataView(buf.buffer).setUint16(p, this._irqCounter, true); p += 2;
        buf[p++] = this._irqLatchHigh ? 1 : 0;
        buf[p++] = this._irqEnable ? 1 : 0;
        buf[p++] = this._irqPending ? 1 : 0;
        for (let i = 0; i < 4; ++i) buf[p++] = this._prgBanks[i];
        for (let i = 0; i < 8; ++i) buf[p++] = this._chrBanks[i];
        while (p < 32) buf[p++] = 0;
        buf.set(this._prgRam, p); p += Fme7.PrgRamSize;
        buf.set(this._ym2149.regs, p); p += 16;
        if (this._chrIsRam && this._chrSize > 0) {
            buf.set(this._chr.subarray(0, this._chrSize), p);
            p += this._chrSize;
        }
        return p;
    }

    loadState(buf: Uint8Array, len: number): boolean {
        const need = 32 + Fme7.PrgRamSize + 16 + (this._chrIsRam ? this._chrSize : 0);
        if (len < need) return false;
        let p = 0;
        this._command = buf[p++];
        this._mirrorHorizontal = buf[p++] !== 0;
        this._prgRamEnable = buf[p++] !== 0;
        this._prgRamWriteProtect = buf[p++] !== 0;
        this._irqLatch = new DataView(buf.buffer).getUint16(p, true); p += 2;
        this._irqCounter = new DataView(buf.buffer).getUint16(p, true); p += 2;
        this._irqLatchHigh = buf[p++] !== 0;
        this._irqEnable = buf[p++] !== 0;
        this._irqPending = buf[p++] !== 0;
        for (let i = 0; i < 4; ++i) this._prgBanks[i] = buf[p++];
        for (let i = 0; i < 8; ++i) this._chrBanks[i] = buf[p++];
        p = 32;
        this._prgRam.set(buf.subarray(p, p + Fme7.PrgRamSize)); p += Fme7.PrgRamSize;
        this._ym2149.regs.set(buf.subarray(p, p + 16)); p += 16;
        if (this._chrIsRam && this._chrSize > 0) {
            this._chr.set(buf.subarray(p, p + this._chrSize));
            p += this._chrSize;
        }
        return true;
    }
}
