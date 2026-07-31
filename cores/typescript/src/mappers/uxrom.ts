// Mapper 2 (UxROM). Port of cores/c/src/mappers/uxrom.c.

import { Mapper, Mirroring } from '../mapper';

export class Uxrom extends Mapper {
    private static readonly PrgBankSize = 16384;
    private static readonly ChrSize = 8192;

    private _prgRom: Uint8Array;
    private _prgSize: number;
    private _chr: Uint8Array;
    private _chrSize: number;
    private _chrIsRam: boolean;
    private _mirroring: number;
    private _hasBattery: boolean;
    private _bank: number;

    constructor(prg: Uint8Array, prgSize: number, chr: Uint8Array | null, chrSize: number, mirroring: number, hasBattery: boolean) {
        super();
        this.mapperNum = 2;
        this._prgSize = prgSize;
        this._prgRom = new Uint8Array(prgSize);
        if (prgSize > 0) this._prgRom.set(prg.subarray(0, prgSize));
        if (chrSize > 0) {
            this._chrSize = chrSize;
            this._chr = new Uint8Array(chrSize);
            this._chr.set(chr!.subarray(0, chrSize));
            this._chrIsRam = false;
        } else {
            this._chrSize = Uxrom.ChrSize;
            this._chr = new Uint8Array(Uxrom.ChrSize);
            this._chrIsRam = true;
        }
        this._mirroring = mirroring;
        this._hasBattery = hasBattery;
        this._bank = 0;
    }

    private prgBankCount(): number {
        const c = Math.floor(this._prgSize / Uxrom.PrgBankSize);
        return c > 0 ? c : 1;
    }

    readPrg(addr: number): number {
        if (addr < 0x8000) return 0x00;
        const local = addr - 0x8000;
        const offset = local & (Uxrom.PrgBankSize - 1);
        const count = this.prgBankCount();
        let idx: number;
        if (local < Uxrom.PrgBankSize) {
            const bank = this._bank % count;
            idx = bank * Uxrom.PrgBankSize + offset;
        } else {
            const last = count - 1;
            idx = last * Uxrom.PrgBankSize + offset;
        }
        if (idx >= this._prgSize) return 0x00;
        return this._prgRom[idx];
    }

    writePrg(addr: number, value: number): void {
        if (addr < 0x8000) return;
        this._bank = value;
    }

    readChr(addr: number): number {
        const sz = this._chrSize;
        if (sz === 0) return 0x00;
        return this._chr[addr % sz];
    }

    writeChr(addr: number, value: number): void {
        if (!this._chrIsRam) return;
        const sz = this._chrSize;
        if (sz === 0) return;
        this._chr[addr % sz] = value;
    }

    mirrorMode(): number { return this._mirroring; }
    chrIsRam(): boolean { return this._chrIsRam; }
    hasBattery(): boolean { return this._hasBattery; }

    saveState(buf: Uint8Array | null): number {
        const total = 1 + (this._chrIsRam ? this._chrSize : 0);
        if (buf === null) return total;
        let p = 0;
        buf[p++] = this._bank;
        if (this._chrIsRam && this._chrSize > 0) {
            buf.set(this._chr.subarray(0, this._chrSize), p);
            p += this._chrSize;
        }
        return p;
    }

    loadState(buf: Uint8Array, len: number): boolean {
        const need = 1 + (this._chrIsRam ? this._chrSize : 0);
        if (len < need) return false;
        let p = 0;
        this._bank = buf[p++];
        if (this._chrIsRam && this._chrSize > 0) {
            this._chr.set(buf.subarray(p, p + this._chrSize));
            p += this._chrSize;
        }
        return true;
    }
}
