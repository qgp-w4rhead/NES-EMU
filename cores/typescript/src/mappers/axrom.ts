// Mapper 7 (AxROM). Port of cores/c/src/mappers/axrom.c.

import { Mapper, Mirroring } from '../mapper';

export class Axrom extends Mapper {
    private static readonly PrgBankSize = 32768;
    private static readonly ChrSize = 8192;

    private _prgRom: Uint8Array;
    private _prgSize: number;
    private _chr: Uint8Array;
    private _chrSize: number;
    private _chrIsRam: boolean;
    private _hasBattery: boolean;
    private _prgBank: number;
    private _mirrorNt: number;

    constructor(prg: Uint8Array, prgSize: number, chr: Uint8Array | null, chrSize: number, mirroring: number, hasBattery: boolean) {
        super();
        this.mapperNum = 7;
        this._prgSize = prgSize;
        this._prgRom = new Uint8Array(prgSize);
        if (prgSize > 0) this._prgRom.set(prg.subarray(0, prgSize));
        if (chrSize > 0) {
            this._chrSize = chrSize;
            this._chr = new Uint8Array(chrSize);
            this._chr.set(chr!.subarray(0, chrSize));
            this._chrIsRam = false;
        } else {
            this._chrSize = Axrom.ChrSize;
            this._chr = new Uint8Array(Axrom.ChrSize);
            this._chrIsRam = true;
        }
        this._hasBattery = hasBattery;
        this._prgBank = 0;
        this._mirrorNt = 0;
    }

    private prgBankCount(): number {
        const c = Math.floor(this._prgSize / Axrom.PrgBankSize);
        return c > 0 ? c : 1;
    }

    readPrg(addr: number): number {
        if (addr < 0x8000) return 0x00;
        const local = addr - 0x8000;
        const count = this.prgBankCount();
        const bank = this._prgBank % count;
        const idx = bank * Axrom.PrgBankSize + local;
        if (idx >= this._prgSize) return 0x00;
        return this._prgRom[idx];
    }

    writePrg(addr: number, value: number): void {
        if (addr < 0x8000) return;
        this._mirrorNt = (value >> 4) & 1;
        this._prgBank = value & 0x07;
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

    mirrorMode(): number { return this._mirrorNt === 0 ? Mirroring.SingleScreen0 : Mirroring.SingleScreen1; }
    chrIsRam(): boolean { return this._chrIsRam; }
    hasBattery(): boolean { return this._hasBattery; }

    saveState(buf: Uint8Array | null): number {
        const total = 2 + (this._chrIsRam ? this._chrSize : 0);
        if (buf === null) return total;
        let p = 0;
        buf[p++] = this._prgBank;
        buf[p++] = this._mirrorNt;
        if (this._chrIsRam && this._chrSize > 0) {
            buf.set(this._chr.subarray(0, this._chrSize), p);
            p += this._chrSize;
        }
        return p;
    }

    loadState(buf: Uint8Array, len: number): boolean {
        const need = 2 + (this._chrIsRam ? this._chrSize : 0);
        if (len < need) return false;
        let p = 0;
        this._prgBank = buf[p++];
        this._mirrorNt = buf[p++];
        if (this._chrIsRam && this._chrSize > 0) {
            this._chr.set(buf.subarray(p, p + this._chrSize));
            p += this._chrSize;
        }
        return true;
    }
}
