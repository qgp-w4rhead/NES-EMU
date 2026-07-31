// Mapper 0 (NROM). Port of cores/c/src/mappers/nrom.c.

import { Mapper, Mirroring } from '../mapper';

export class Nrom extends Mapper {
    private _prgRom: Uint8Array;
    private _prgSize: number;
    private _chr: Uint8Array;
    private _chrSize: number;
    private _chrIsRam: boolean;
    private _mirroring: number;
    private _hasBattery: boolean;

    constructor(prg: Uint8Array, prgSize: number, chr: Uint8Array | null, chrSize: number, mirroring: number, hasBattery: boolean) {
        super();
        this.mapperNum = 0;
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
        this._mirroring = mirroring;
        this._hasBattery = hasBattery;
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

    writePrg(addr: number, value: number): void { }

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
        const total = 7 + (this._chrIsRam ? this._chrSize : 0);
        if (buf === null) return total;
        let p = 0;
        buf[p++] = this._chrIsRam ? 1 : 0;
        buf[p++] = this._mirroring;
        buf[p++] = this._hasBattery ? 1 : 0;
        new DataView(buf.buffer).setUint32(p, this._chrSize, true); p += 4;
        if (this._chrIsRam && this._chrSize > 0) {
            buf.set(this._chr.subarray(0, this._chrSize), p);
            p += this._chrSize;
        }
        return p;
    }

    loadState(buf: Uint8Array, len: number): boolean {
        if (len < 7) return false;
        let p = 0;
        this._chrIsRam = buf[p++] !== 0;
        this._mirroring = buf[p++];
        this._hasBattery = buf[p++] !== 0;
        this._chrSize = new DataView(buf.buffer).getUint32(p, true); p += 4;
        if (this._chrIsRam && this._chrSize > 0) {
            if (len < p + this._chrSize) return false;
            this._chr.set(buf.subarray(p, p + this._chrSize));
            p += this._chrSize;
        }
        return true;
    }
}
