// Mapper 3 (CNROM). Port of cores/c/src/mappers/cnrom.c.

import { Mapper, Mirroring } from '../mapper';

export class Cnrom extends Mapper {
    private static readonly PrgBankSize = 16384;
    private static readonly ChrBankSize = 8192;

    private _prgRom: Uint8Array;
    private _prgSize: number;
    private _chr: Uint8Array;
    private _chrSize: number;
    private _chrIsRam: boolean;
    private _mirroring: number;
    private _hasBattery: boolean;
    private _chrBank: number;

    constructor(prg: Uint8Array, prgSize: number, chr: Uint8Array | null, chrSize: number, mirroring: number, hasBattery: boolean) {
        super();
        this.mapperNum = 3;
        this._prgSize = prgSize;
        this._prgRom = new Uint8Array(prgSize);
        if (prgSize > 0) this._prgRom.set(prg.subarray(0, prgSize));
        if (chrSize === 0) {
            this._chrSize = Cnrom.ChrBankSize;
            this._chr = new Uint8Array(Cnrom.ChrBankSize);
            this._chrIsRam = true;
        } else {
            this._chrSize = chrSize;
            this._chr = new Uint8Array(chrSize);
            this._chr.set(chr!.subarray(0, chrSize));
            this._chrIsRam = false;
        }
        this._mirroring = mirroring;
        this._hasBattery = hasBattery;
        this._chrBank = 0;
    }

    private chrBankCount(): number {
        const c = Math.floor(this._chrSize / Cnrom.ChrBankSize);
        return c > 0 ? c : 1;
    }

    private prgIndex(addr: number): number {
        const local = addr - 0x8000;
        const bank = this._prgSize;
        if (bank === 0) return 0;
        return local % bank;
    }

    readPrg(addr: number): number {
        if (addr < 0x8000) return 0x00;
        const idx = this.prgIndex(addr);
        if (idx >= this._prgSize) return 0x00;
        return this._prgRom[idx];
    }

    writePrg(addr: number, value: number): void {
        if (addr < 0x8000) return;
        this._chrBank = value;
    }

    readChr(addr: number): number {
        const count = this.chrBankCount();
        const bank = this._chrBank % count;
        const idx = bank * Cnrom.ChrBankSize + (addr & (Cnrom.ChrBankSize - 1));
        if (idx >= this._chrSize) return 0x00;
        return this._chr[idx];
    }

    writeChr(addr: number, value: number): void {
        if (!this._chrIsRam) return;
        const count = this.chrBankCount();
        const bank = this._chrBank % count;
        const idx = bank * Cnrom.ChrBankSize + (addr & (Cnrom.ChrBankSize - 1));
        if (idx < this._chrSize) this._chr[idx] = value;
    }

    mirrorMode(): number { return this._mirroring; }
    chrIsRam(): boolean { return this._chrIsRam; }
    hasBattery(): boolean { return this._hasBattery; }

    saveState(buf: Uint8Array | null): number {
        const total = 1 + (this._chrIsRam ? this._chrSize : 0);
        if (buf === null) return total;
        let p = 0;
        buf[p++] = this._chrBank;
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
        this._chrBank = buf[p++];
        if (this._chrIsRam && this._chrSize > 0) {
            this._chr.set(buf.subarray(p, p + this._chrSize));
            p += this._chrSize;
        }
        return true;
    }
}
