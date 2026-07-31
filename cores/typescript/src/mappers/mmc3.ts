// Mapper 4 (MMC3). Port of cores/c/src/mappers/mmc3.c.

import { Mapper, Mirroring } from '../mapper';

export class Mmc3 extends Mapper {
    private static readonly PrgBankSize = 8192;
    private static readonly Chr1kSize = 1024;
    private static readonly Chr2kSize = 2048;
    private static readonly PrgRamSize = 8192;

    private _prgRom: Uint8Array;
    private _prgSize: number;
    private _chr: Uint8Array;
    private _chrSize: number;
    private _chrIsRam: boolean;
    private _prgRam: Uint8Array = new Uint8Array(Mmc3.PrgRamSize);
    private _hasBattery: boolean;
    private _bankSelect: number;
    private _bankValues: Uint8Array = new Uint8Array(8);
    private _mirroring: number;
    private _prgRamEnable: boolean;
    private _prgRamWriteProtect: boolean;
    private _irqLatch: number;
    private _irqCounter: number;
    private _irqReloadFlag: boolean;
    private _irqEnable: boolean;
    private _irqPending: boolean;

    constructor(prg: Uint8Array, prgSize: number, chr: Uint8Array | null, chrSize: number, mirroring: number, hasBattery: boolean) {
        super();
        this.mapperNum = 4;
        this._prgSize = prgSize;
        this._prgRom = new Uint8Array(prgSize);
        if (prgSize > 0) this._prgRom.set(prg.subarray(0, prgSize));
        if (chrSize === 0) {
            this._chrSize = Mmc3.Chr2kSize * 4;
            this._chr = new Uint8Array(this._chrSize);
            this._chrIsRam = true;
        } else {
            this._chrSize = chrSize;
            this._chr = new Uint8Array(chrSize);
            this._chr.set(chr!.subarray(0, chrSize));
            this._chrIsRam = false;
        }
        this._hasBattery = hasBattery;
        this._bankSelect = 0;
        this._mirroring = mirroring;
        this._prgRamEnable = false;
        this._prgRamWriteProtect = false;
        this._irqLatch = 0;
        this._irqCounter = 0;
        this._irqReloadFlag = false;
        this._irqEnable = false;
        this._irqPending = false;
    }

    private prgBankCount(): number {
        const c = Math.floor(this._prgSize / Mmc3.PrgBankSize);
        return c > 0 ? c : 1;
    }
    private chr1kCount(): number {
        const c = Math.floor(this._chrSize / Mmc3.Chr1kSize);
        return c > 0 ? c : 1;
    }
    private prgMode(): number { return (this._bankSelect >> 6) & 1; }
    private chrMode(): number { return (this._bankSelect >> 7) & 1; }

    private prgReadBank(bank: number, offset: number): number {
        const count = this.prgBankCount();
        bank %= count;
        const idx = bank * Mmc3.PrgBankSize + offset;
        if (idx >= this._prgSize) return 0x00;
        return this._prgRom[idx];
    }

    private chrBankForSlot(slot: number): number {
        const r = this._bankValues;
        const cm = this.chrMode();
        if (cm === 0) {
            switch (slot) {
                case 0: return r[0] & 0xFE;
                case 1: return (r[0] & 0xFE) + 1;
                case 2: return r[1] & 0xFE;
                case 3: return (r[1] & 0xFE) + 1;
                case 4: return r[2];
                case 5: return r[3];
                case 6: return r[4];
                case 7: return r[5];
                default: return 0;
            }
        } else {
            switch (slot) {
                case 0: return r[2];
                case 1: return r[3];
                case 2: return r[4];
                case 3: return r[5];
                case 4: return r[0] & 0xFE;
                case 5: return (r[0] & 0xFE) + 1;
                case 6: return r[1] & 0xFE;
                case 7: return (r[1] & 0xFE) + 1;
                default: return 0;
            }
        }
    }

    private chrIndex(addr: number): number {
        const a = addr;
        const slot = Math.floor(a / Mmc3.Chr1kSize);
        const offset = a & (Mmc3.Chr1kSize - 1);
        const bank = this.chrBankForSlot(slot);
        const count = this.chr1kCount();
        return (bank % count) * Mmc3.Chr1kSize + offset;
    }

    readPrg(addr: number): number {
        if (addr >= 0x6000 && addr < 0x8000) {
            if (this._prgRamEnable)
                return this._prgRam[(addr - 0x6000) & (Mmc3.PrgRamSize - 1)];
            return 0x00;
        }
        const local = addr - 0x8000;
        const slot = Math.floor(local / Mmc3.PrgBankSize);
        const offset = local & (Mmc3.PrgBankSize - 1);
        const count = this.prgBankCount();
        const last = count - 1;
        const secondLast = count >= 2 ? count - 2 : 0;
        let bank: number;
        const pm = this.prgMode();
        if (slot === 0)
            bank = (pm === 0) ? (this._bankValues[6] % count) : secondLast;
        else if (slot === 1)
            bank = this._bankValues[7] % count;
        else if (slot === 2)
            bank = (pm === 0) ? secondLast : (this._bankValues[6] % count);
        else
            bank = last;
        return this.prgReadBank(bank, offset);
    }

    writePrg(addr: number, value: number): void {
        if (addr >= 0x6000 && addr < 0x8000) {
            if (this._prgRamEnable && !this._prgRamWriteProtect)
                this._prgRam[(addr - 0x6000) & (Mmc3.PrgRamSize - 1)] = value;
            return;
        }
        if (addr >= 0x8000 && addr <= 0x9FFF) {
            if ((addr & 1) === 0) this._bankSelect = value;
            else { const reg = this._bankSelect & 0x07; this._bankValues[reg] = value; }
        } else if (addr >= 0xA000 && addr <= 0xBFFF) {
            if ((addr & 1) === 0) this._mirroring = (value & 1) === 0 ? Mirroring.Vertical : Mirroring.Horizontal;
            else { this._prgRamEnable = (value & 0x80) !== 0; this._prgRamWriteProtect = (value & 0x40) !== 0; }
        } else if (addr >= 0xC000 && addr <= 0xDFFF) {
            if ((addr & 1) === 0) this._irqLatch = value;
            else this._irqReloadFlag = true;
        } else if (addr >= 0xE000 && addr <= 0xFFFF) {
            if ((addr & 1) === 0) { this._irqEnable = false; this._irqPending = false; }
            else this._irqEnable = true;
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

    mirrorMode(): number { return this._mirroring; }
    chrIsRam(): boolean { return this._chrIsRam; }
    hasBattery(): boolean { return this._hasBattery; }
    irqPending(): boolean { return this._irqPending; }

    clockIrq(): void {
        if (this._irqReloadFlag) {
            this._irqCounter = this._irqLatch;
            this._irqReloadFlag = false;
        } else if (this._irqCounter === 0) {
            this._irqCounter = this._irqLatch;
            if (this._irqEnable) this._irqPending = true;
        } else {
            this._irqCounter = (this._irqCounter - 1) & 0xFF;
        }
    }

    saveState(buf: Uint8Array | null): number {
        const total = 17 + Mmc3.PrgRamSize + (this._chrIsRam ? this._chrSize : 0);
        if (buf === null) return total;
        let p = 0;
        buf[p++] = this._bankSelect;
        for (let i = 0; i < 8; ++i) buf[p++] = this._bankValues[i];
        buf[p++] = this._mirroring;
        buf[p++] = this._prgRamEnable ? 1 : 0;
        buf[p++] = this._prgRamWriteProtect ? 1 : 0;
        buf[p++] = this._irqLatch;
        buf[p++] = this._irqCounter;
        buf[p++] = this._irqReloadFlag ? 1 : 0;
        buf[p++] = this._irqEnable ? 1 : 0;
        buf[p++] = this._irqPending ? 1 : 0;
        buf.set(this._prgRam, p); p += Mmc3.PrgRamSize;
        if (this._chrIsRam && this._chrSize > 0) {
            buf.set(this._chr.subarray(0, this._chrSize), p);
            p += this._chrSize;
        }
        return p;
    }

    loadState(buf: Uint8Array, len: number): boolean {
        const need = 17 + Mmc3.PrgRamSize + (this._chrIsRam ? this._chrSize : 0);
        if (len < need) return false;
        let p = 0;
        this._bankSelect = buf[p++];
        for (let i = 0; i < 8; ++i) this._bankValues[i] = buf[p++];
        this._mirroring = buf[p++];
        this._prgRamEnable = buf[p++] !== 0;
        this._prgRamWriteProtect = buf[p++] !== 0;
        this._irqLatch = buf[p++];
        this._irqCounter = buf[p++];
        this._irqReloadFlag = buf[p++] !== 0;
        this._irqEnable = buf[p++] !== 0;
        this._irqPending = buf[p++] !== 0;
        this._prgRam.set(buf.subarray(p, p + Mmc3.PrgRamSize)); p += Mmc3.PrgRamSize;
        if (this._chrIsRam && this._chrSize > 0) {
            this._chr.set(buf.subarray(p, p + this._chrSize));
            p += this._chrSize;
        }
        return true;
    }
}
