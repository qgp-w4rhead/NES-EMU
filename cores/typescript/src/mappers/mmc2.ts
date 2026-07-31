// Mapper 9 (MMC2). Port of cores/c/src/mappers/mmc2.c.

import { Mapper, Mirroring } from '../mapper';

export class Mmc2 extends Mapper {
    private static readonly PrgBankSize = 8192;
    private static readonly Chr4kSize = 4096;
    private static readonly PrgRamSize = 1024;
    private static readonly LatchLeftB0Lo = 0x0FD8;
    private static readonly LatchLeftB0Hi = 0x0FDF;
    private static readonly LatchLeftB1Lo = 0x0FE8;
    private static readonly LatchLeftB1Hi = 0x0FEF;
    private static readonly LatchRightB0Lo = 0x1FD8;
    private static readonly LatchRightB0Hi = 0x1FDF;
    private static readonly LatchRightB1Lo = 0x1FE8;
    private static readonly LatchRightB1Hi = 0x1FEF;

    private _prgRom: Uint8Array;
    private _prgSize: number;
    private _chr: Uint8Array;
    private _chrSize: number;
    private _chrIsRam: boolean;
    private _prgRam: Uint8Array = new Uint8Array(Mmc2.PrgRamSize);
    private _hasBattery: boolean;
    private _prgBank: number;
    private _chrBanks: Uint8Array = new Uint8Array(4);
    private _latchLeft: number;
    private _latchRight: number;
    private _mirroring: number;

    constructor(prg: Uint8Array, prgSize: number, chr: Uint8Array | null, chrSize: number, mirroring: number, hasBattery: boolean) {
        super();
        this.mapperNum = 9;
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
        this._prgBank = 0;
        this._mirroring = mirroring;
        this._latchLeft = 0;
        this._latchRight = 0;
    }

    private prgBankCount(): number {
        const c = Math.floor(this._prgSize / Mmc2.PrgBankSize);
        return c > 0 ? c : 1;
    }
    private chrBankCount(): number {
        const c = Math.floor(this._chrSize / Mmc2.Chr4kSize);
        return c > 0 ? c : 1;
    }

    private prgReadBank(bank: number, offset: number): number {
        const count = this.prgBankCount();
        bank %= count;
        const idx = bank * Mmc2.PrgBankSize + offset;
        if (idx >= this._prgSize) return 0x00;
        return this._prgRom[idx];
    }

    private leftBank(): number { return this._latchLeft === 0 ? this._chrBanks[0] : this._chrBanks[1]; }
    private rightBank(): number { return this._latchRight === 0 ? this._chrBanks[2] : this._chrBanks[3]; }

    private updateLatches(addr: number): void {
        if (addr >= Mmc2.LatchLeftB0Lo && addr <= Mmc2.LatchLeftB0Hi) this._latchLeft = 0;
        else if (addr >= Mmc2.LatchLeftB1Lo && addr <= Mmc2.LatchLeftB1Hi) this._latchLeft = 1;
        else if (addr >= Mmc2.LatchRightB0Lo && addr <= Mmc2.LatchRightB0Hi) this._latchRight = 0;
        else if (addr >= Mmc2.LatchRightB1Lo && addr <= Mmc2.LatchRightB1Hi) this._latchRight = 1;
    }

    readPrg(addr: number): number {
        if (addr >= 0x6000 && addr < 0x8000)
            return this._prgRam[(addr - 0x6000) & (Mmc2.PrgRamSize - 1)];
        const local = addr - 0x8000;
        const count = this.prgBankCount();
        if (local < Mmc2.PrgBankSize) {
            const bank = this._prgBank % count;
            return this.prgReadBank(bank, local);
        }
        const fixedOffset = local - Mmc2.PrgBankSize;
        const fixedBankBase = count >= 3 ? count - 3 : 0;
        const bank2 = fixedBankBase + Math.floor(fixedOffset / Mmc2.PrgBankSize);
        const offset = fixedOffset & (Mmc2.PrgBankSize - 1);
        return this.prgReadBank(bank2, offset);
    }

    writePrg(addr: number, value: number): void {
        if (addr >= 0x6000 && addr < 0x8000) {
            this._prgRam[(addr - 0x6000) & (Mmc2.PrgRamSize - 1)] = value;
            return;
        }
        switch (addr) {
            case 0xA000: this._prgBank = value & 0x0F; break;
            case 0xB000: this._chrBanks[0] = value & 0x3F; break;
            case 0xB001: this._chrBanks[1] = value & 0x3F; break;
            case 0xB002: this._chrBanks[2] = value & 0x3F; break;
            case 0xB003: this._chrBanks[3] = value & 0x3F; break;
        }
    }

    readChr(addr: number): number {
        const bank = (addr < Mmc2.Chr4kSize) ? this.leftBank() : this.rightBank();
        const count = this.chrBankCount();
        const b = bank % count;
        const offset = addr & (Mmc2.Chr4kSize - 1);
        const idx = b * Mmc2.Chr4kSize + offset;
        if (idx >= this._chrSize) return 0x00;
        return this._chr[idx];
    }

    readChrLatched(addr: number): number {
        this.updateLatches(addr);
        return this.readChr(addr);
    }

    writeChr(addr: number, value: number): void {
        if (!this._chrIsRam) return;
        const bank = (addr < Mmc2.Chr4kSize) ? this.leftBank() : this.rightBank();
        const count = this.chrBankCount();
        const b = bank % count;
        const offset = addr & (Mmc2.Chr4kSize - 1);
        const idx = b * Mmc2.Chr4kSize + offset;
        if (idx < this._chrSize) this._chr[idx] = value;
    }

    mirrorMode(): number { return this._mirroring; }
    chrIsRam(): boolean { return this._chrIsRam; }
    hasBattery(): boolean { return this._hasBattery; }

    saveState(buf: Uint8Array | null): number {
        const total = 8 + Mmc2.PrgRamSize + (this._chrIsRam ? this._chrSize : 0);
        if (buf === null) return total;
        let p = 0;
        buf[p++] = this._prgBank;
        for (let i = 0; i < 4; ++i) buf[p++] = this._chrBanks[i];
        buf[p++] = this._latchLeft;
        buf[p++] = this._latchRight;
        buf.set(this._prgRam, p); p += Mmc2.PrgRamSize;
        if (this._chrIsRam && this._chrSize > 0) {
            buf.set(this._chr.subarray(0, this._chrSize), p);
            p += this._chrSize;
        }
        return p;
    }

    loadState(buf: Uint8Array, len: number): boolean {
        const need = 8 + Mmc2.PrgRamSize + (this._chrIsRam ? this._chrSize : 0);
        if (len < need) return false;
        let p = 0;
        this._prgBank = buf[p++];
        for (let i = 0; i < 4; ++i) this._chrBanks[i] = buf[p++];
        this._latchLeft = buf[p++];
        this._latchRight = buf[p++];
        this._prgRam.set(buf.subarray(p, p + Mmc2.PrgRamSize)); p += Mmc2.PrgRamSize;
        if (this._chrIsRam && this._chrSize > 0) {
            this._chr.set(buf.subarray(p, p + this._chrSize));
            p += this._chrSize;
        }
        return true;
    }
}
