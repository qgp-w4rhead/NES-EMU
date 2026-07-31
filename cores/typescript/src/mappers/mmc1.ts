// Mapper 1 (MMC1). Port of cores/c/src/mappers/mmc1.c.

import { Mapper, Mirroring } from '../mapper';

export class Mmc1 extends Mapper {
    private static readonly PrgBankSize = 16384;
    private static readonly Chr4kSize = 4096;
    private static readonly Chr8kSize = 8192;
    private static readonly PrgRamSize = 8192;
    private static readonly ControlPrgMode3 = 0x0C;

    private _prgRom: Uint8Array;
    private _prgSize: number;
    private _chr: Uint8Array;
    private _chrSize: number;
    private _chrIsRam: boolean;
    private _prgRam: Uint8Array = new Uint8Array(Mmc1.PrgRamSize);
    private _hasBattery: boolean;
    private _shiftReg: number;
    private _shiftCount: number;
    private _control: number;
    private _chrBank0: number;
    private _chrBank1: number;
    private _prgBank: number;

    constructor(prg: Uint8Array, prgSize: number, chr: Uint8Array | null, chrSize: number, mirroring: number, hasBattery: boolean) {
        super();
        this.mapperNum = 1;
        this._prgSize = prgSize;
        this._prgRom = new Uint8Array(prgSize);
        if (prgSize > 0) this._prgRom.set(prg.subarray(0, prgSize));
        if (chrSize === 0) {
            this._chrSize = Mmc1.Chr8kSize;
            this._chr = new Uint8Array(Mmc1.Chr8kSize);
            this._chrIsRam = true;
        } else {
            this._chrSize = chrSize;
            this._chr = new Uint8Array(chrSize);
            this._chr.set(chr!.subarray(0, chrSize));
            this._chrIsRam = false;
        }
        this._hasBattery = hasBattery;
        this._shiftReg = 0;
        this._shiftCount = 0;
        let mirrBits: number;
        switch (mirroring) {
            case Mirroring.SingleScreen0: mirrBits = 0x00; break;
            case Mirroring.SingleScreen1: mirrBits = 0x01; break;
            case Mirroring.SingleScreen2: mirrBits = 0x01; break;
            case Mirroring.SingleScreen3: mirrBits = 0x01; break;
            case Mirroring.Vertical: mirrBits = 0x02; break;
            default: mirrBits = 0x03; break;
        }
        this._control = (Mmc1.ControlPrgMode3 | mirrBits) & 0xFF;
        this._chrBank0 = 0;
        this._chrBank1 = 0;
        this._prgBank = 0;
    }

    private prgBankCount(): number {
        const c = Math.floor(this._prgSize / Mmc1.PrgBankSize);
        return c > 0 ? c : 1;
    }
    private chrBankCount(): number {
        const c = Math.floor(this._chrSize / Mmc1.Chr4kSize);
        return c > 0 ? c : 1;
    }
    private prgMode(): number { return (this._control >> 2) & 0x03; }
    private chrMode(): number { return (this._control >> 4) & 1; }

    private prgReadBank(bank: number, offset: number): number {
        const count = this.prgBankCount();
        bank %= count;
        const idx = bank * Mmc1.PrgBankSize + offset;
        if (idx >= this._prgSize) return 0x00;
        return this._prgRom[idx];
    }

    private serialWrite(addr: number, value: number): void {
        if ((value & 0x80) !== 0) {
            this._shiftReg = 0;
            this._shiftCount = 0;
            this._control = ((this._control & 0x13) | Mmc1.ControlPrgMode3) & 0xFF;
            return;
        }
        this._shiftReg = ((this._shiftReg >> 1) | ((value & 1) << 4)) & 0xFF;
        this._shiftCount++;
        if (this._shiftCount === 5) {
            const reg = (addr >> 13) & 0x03;
            switch (reg) {
                case 0: this._control = this._shiftReg; break;
                case 1: this._chrBank0 = this._shiftReg; break;
                case 2: this._chrBank1 = this._shiftReg; break;
                case 3: this._prgBank = this._shiftReg; break;
            }
            this._shiftReg = 0;
            this._shiftCount = 0;
        }
    }

    readPrg(addr: number): number {
        if (addr >= 0x6000 && addr < 0x8000)
            return this._prgRam[(addr - 0x6000) & (Mmc1.PrgRamSize - 1)];
        const local = addr - 0x8000;
        const inLow = local < Mmc1.PrgBankSize;
        const offset = local & (Mmc1.PrgBankSize - 1);
        switch (this.prgMode()) {
            case 0: case 1:
                return this.prgReadBank(this._prgBank & 0x0E, local);
            case 2:
                if (inLow) return this.prgReadBank(0, offset);
                return this.prgReadBank(this._prgBank & 0x0F, offset);
            default:
                if (inLow) return this.prgReadBank(this._prgBank & 0x0F, offset);
                return this.prgReadBank(this.prgBankCount() - 1, offset);
        }
    }

    writePrg(addr: number, value: number): void {
        if (addr >= 0x6000 && addr < 0x8000) {
            this._prgRam[(addr - 0x6000) & (Mmc1.PrgRamSize - 1)] = value;
            return;
        }
        this.serialWrite(addr, value);
    }

    private chrIndex(a: number): number {
        const count = this.chrBankCount();
        if (this.chrMode() === 0) {
            const bank = (this._chrBank0 & 0x1E) % count;
            return bank * Mmc1.Chr4kSize + (a & (Mmc1.Chr8kSize - 1));
        } else if (a < Mmc1.Chr4kSize) {
            const bank = (this._chrBank0 & 0x1F) % count;
            return bank * Mmc1.Chr4kSize + a;
        } else {
            const bank = (this._chrBank1 & 0x1F) % count;
            return bank * Mmc1.Chr4kSize + (a - Mmc1.Chr4kSize);
        }
    }

    readChr(addr: number): number {
        const idx = this.chrIndex(addr);
        if (idx >= this._chrSize) return 0x00;
        return this._chr[idx];
    }

    writeChr(addr: number, value: number): void {
        if (!this._chrIsRam) return;
        const idx = this.chrIndex(addr);
        if (idx < this._chrSize) this._chr[idx] = value;
    }

    mirrorMode(): number {
        switch (this._control & 0x03) {
            case 0: return Mirroring.SingleScreen0;
            case 1: return Mirroring.SingleScreen1;
            case 2: return Mirroring.Vertical;
            default: return Mirroring.Horizontal;
        }
    }

    chrIsRam(): boolean { return this._chrIsRam; }
    hasBattery(): boolean { return this._hasBattery; }

    saveState(buf: Uint8Array | null): number {
        const total = 6 + Mmc1.PrgRamSize + (this._chrIsRam ? this._chrSize : 0);
        if (buf === null) return total;
        let p = 0;
        buf[p++] = this._shiftReg;
        buf[p++] = this._shiftCount;
        buf[p++] = this._control;
        buf[p++] = this._chrBank0;
        buf[p++] = this._chrBank1;
        buf[p++] = this._prgBank;
        buf.set(this._prgRam, p); p += Mmc1.PrgRamSize;
        if (this._chrIsRam && this._chrSize > 0) {
            buf.set(this._chr.subarray(0, this._chrSize), p);
            p += this._chrSize;
        }
        return p;
    }

    loadState(buf: Uint8Array, len: number): boolean {
        const need = 6 + Mmc1.PrgRamSize + (this._chrIsRam ? this._chrSize : 0);
        if (len < need) return false;
        let p = 0;
        this._shiftReg = buf[p++];
        this._shiftCount = buf[p++];
        this._control = buf[p++];
        this._chrBank0 = buf[p++];
        this._chrBank1 = buf[p++];
        this._prgBank = buf[p++];
        this._prgRam.set(buf.subarray(p, p + Mmc1.PrgRamSize)); p += Mmc1.PrgRamSize;
        if (this._chrIsRam && this._chrSize > 0) {
            this._chr.set(buf.subarray(p, p + this._chrSize));
            p += this._chrSize;
        }
        return true;
    }
}
