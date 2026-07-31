// Mapper 5 (MMC5). Port of cores/c/src/mmc5.c.

import { Mapper, Mirroring } from '../mapper';

export class Mmc5 extends Mapper {
    private static readonly PrgBankSize = 8192;
    private static readonly Chr1kSize = 1024;
    private static readonly PrgRamSize = 65536;
    private static readonly PrgRamWindow = 8192;

    private _prgRom: Uint8Array;
    private _prgSize: number;
    private _chr: Uint8Array;
    private _chrSize: number;
    private _chrIsRam: boolean;
    private _prgRam: Uint8Array;
    private _hasBattery: boolean;
    private _prgMode: number;
    private _chrMode: number;
    private _prgRamProtect1: number;
    private _prgRamProtect2: number;
    private _ntMirroring: number;
    private _fillTile: number;
    private _fillAttr: number;
    private _prgRamBank: number;
    private _prgBanks: Uint8Array = new Uint8Array(4);
    private _chrBanks: Uint8Array = new Uint8Array(8);
    private _chrBanksEx: Uint8Array = new Uint8Array(4);
    private _chrUpper: number;
    private _splitControl: number;
    private _splitYScroll: number;
    private _splitBank: number;
    private _irqScanline: number;
    private _irqControl: number;
    private _scanlineCounter: number;
    private _irqPending: boolean;
    private _inVblank: boolean;
    private _multA: number;
    private _multB: number;

    constructor(prg: Uint8Array, prgSize: number, chr: Uint8Array | null, chrSize: number, mirroring: number, hasBattery: boolean) {
        super();
        this.mapperNum = 5;
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
        this._prgRam = new Uint8Array(Mmc5.PrgRamSize);
        this._hasBattery = hasBattery;
        this._prgMode = 3;
        this._chrMode = 3;
        this._prgRamProtect1 = 0;
        this._prgRamProtect2 = 0;
        let ntMirroring: number;
        switch (mirroring) {
            case Mirroring.Horizontal: ntMirroring = 0x44; break;
            case Mirroring.Vertical: ntMirroring = 0x50; break;
            case Mirroring.FourScreen: ntMirroring = 0xE4; break;
            case Mirroring.SingleScreen0: ntMirroring = 0x00; break;
            case Mirroring.SingleScreen1: ntMirroring = 0x55; break;
            case Mirroring.SingleScreen2: ntMirroring = 0xAA; break;
            case Mirroring.SingleScreen3: ntMirroring = 0xFF; break;
            default: ntMirroring = 0x44; break;
        }
        this._ntMirroring = ntMirroring;
        this._fillTile = 0;
        this._fillAttr = 0;
        this._prgRamBank = 0;
        this._chrUpper = 0;
        this._splitControl = 0;
        this._splitYScroll = 0;
        this._splitBank = 0;
        this._irqScanline = 0;
        this._irqControl = 0;
        this._scanlineCounter = 0;
        this._irqPending = false;
        this._inVblank = false;
        this._multA = 0;
        this._multB = 0;
    }

    private prgBankCount(): number {
        const n = Math.floor(this._prgSize / Mmc5.PrgBankSize);
        return n > 0 ? n : 1;
    }
    private chr1kCount(): number {
        const n = Math.floor(this._chrSize / Mmc5.Chr1kSize);
        return n > 0 ? n : 1;
    }
    private prgReadBank(bank: number, offset: number): number {
        const count = this.prgBankCount();
        bank %= count;
        const idx = bank * Mmc5.PrgBankSize + offset;
        if (idx >= this._prgSize) return 0x00;
        return this._prgRom[idx];
    }
    private product(): number { return (this._multA * this._multB) & 0xFFFF; }
    private prgRamWritesAllowed(): boolean { return this._prgRamProtect1 === 0x02 && this._prgRamProtect2 === 0x01; }

    private decodeMirroring(): number {
        const s0 = this._ntMirroring & 0x03;
        const s1 = (this._ntMirroring >> 2) & 0x03;
        const s2 = (this._ntMirroring >> 4) & 0x03;
        const s3 = (this._ntMirroring >> 6) & 0x03;
        if (s0 === s1 && s1 === s2 && s2 === s3) {
            switch (s0) {
                case 0: return Mirroring.SingleScreen0;
                case 1: return Mirroring.SingleScreen1;
                case 2: return Mirroring.SingleScreen2;
                default: return Mirroring.SingleScreen3;
            }
        }
        if (s0 === 0 && s1 === 1 && s2 === 0 && s3 === 1) return Mirroring.Horizontal;
        if (s0 === 0 && s1 === 0 && s2 === 1 && s3 === 1) return Mirroring.Vertical;
        if (s0 === 0 && s1 === 1 && s2 === 2 && s3 === 3) return Mirroring.FourScreen;
        return Mirroring.Horizontal;
    }

    readPrg(addr: number): number {
        if (addr === 0x5205) return this.product() & 0xFF;
        if (addr === 0x5206) return (this.product() >> 8) & 0xFF;
        if (addr === 0x5204) {
            let status = this._irqControl & 0x80;
            if (this._inVblank) status |= 0x40;
            return status;
        }
        if (addr >= 0x6000 && addr < 0x8000) {
            const bank = this._prgRamBank % (Mmc5.PrgRamSize / Mmc5.PrgRamWindow);
            const idx = bank * Mmc5.PrgRamWindow + ((addr - 0x6000) & (Mmc5.PrgRamWindow - 1));
            if (idx >= Mmc5.PrgRamSize) return 0x00;
            return this._prgRam[idx];
        }
        const local = addr - 0x8000;
        const slot = Math.floor(local / Mmc5.PrgBankSize);
        const offset = local & (Mmc5.PrgBankSize - 1);
        if (slot === 3) {
            const last = this.prgBankCount() - 1;
            return this.prgReadBank(last, offset);
        }
        const reg = this._prgBanks[slot];
        if (reg & 0x80) {
            const ramBank = (reg & 0x7F) % (Mmc5.PrgRamSize / Mmc5.PrgBankSize);
            const idx = ramBank * Mmc5.PrgBankSize + offset;
            if (idx >= Mmc5.PrgRamSize) return 0x00;
            return this._prgRam[idx];
        }
        const count = this.prgBankCount();
        const bank = reg % count;
        return this.prgReadBank(bank, offset);
    }

    writePrg(addr: number, value: number): void {
        if (addr === 0x5205) { this._multA = value; return; }
        if (addr === 0x5206) { this._multB = value; return; }
        if (addr === 0x5204) {
            this._irqControl = value & 0x80;
            if ((value & 0x80) === 0) this._irqPending = false;
            return;
        }
        if (addr === 0x5203) { this._irqScanline = value; return; }
        if (addr === 0x5200) { this._splitControl = value; return; }
        if (addr === 0x5201) { this._splitYScroll = value; return; }
        if (addr === 0x5202) { this._splitBank = value; return; }
        switch (addr) {
            case 0x5100: this._prgMode = value & 0x03; return;
            case 0x5101: this._chrMode = value & 0x03; return;
            case 0x5102: this._prgRamProtect1 = value & 0x03; return;
            case 0x5103: this._prgRamProtect2 = value & 0x03; return;
            case 0x5104: return;
            case 0x5105: this._ntMirroring = value; return;
            case 0x5106: this._fillTile = value; return;
            case 0x5107: this._fillAttr = value; return;
            case 0x5113: this._prgRamBank = value; return;
            case 0x5114: this._prgBanks[0] = value; return;
            case 0x5115: this._prgBanks[1] = value; return;
            case 0x5116: this._prgBanks[2] = value; return;
            case 0x5117: this._prgBanks[3] = value; return;
            case 0x5120: this._chrBanks[0] = value; return;
            case 0x5121: this._chrBanks[1] = value; return;
            case 0x5122: this._chrBanks[2] = value; return;
            case 0x5123: this._chrBanks[3] = value; return;
            case 0x5124: this._chrBanks[4] = value; return;
            case 0x5125: this._chrBanks[5] = value; return;
            case 0x5126: this._chrBanks[6] = value; return;
            case 0x5127: this._chrBanks[7] = value; return;
            case 0x5128: this._chrBanksEx[0] = value; return;
            case 0x5129: this._chrBanksEx[1] = value; return;
            case 0x512A: this._chrBanksEx[2] = value; return;
            case 0x512B: this._chrBanksEx[3] = value; return;
            case 0x5130: this._chrUpper = value & 0x01; return;
        }
        if (addr >= 0x6000 && addr < 0x8000 && this.prgRamWritesAllowed()) {
            const bank = this._prgRamBank % (Mmc5.PrgRamSize / Mmc5.PrgRamWindow);
            const idx = bank * Mmc5.PrgRamWindow + ((addr - 0x6000) & (Mmc5.PrgRamWindow - 1));
            if (idx < Mmc5.PrgRamSize) this._prgRam[idx] = value;
            return;
        }
        if (addr >= 0x8000 && addr < 0xE000 && this.prgRamWritesAllowed()) {
            const local = addr - 0x8000;
            const slot = Math.floor(local / Mmc5.PrgBankSize);
            const offset = local & (Mmc5.PrgBankSize - 1);
            if (slot < 3) {
                const reg = this._prgBanks[slot];
                if (reg & 0x80) {
                    const ramBank = (reg & 0x7F) % (Mmc5.PrgRamSize / Mmc5.PrgBankSize);
                    const idx = ramBank * Mmc5.PrgBankSize + offset;
                    if (idx < Mmc5.PrgRamSize) this._prgRam[idx] = value;
                }
            }
        }
    }

    readChr(addr: number): number {
        const slot = Math.floor(addr / Mmc5.Chr1kSize);
        const offset = addr & (Mmc5.Chr1kSize - 1);
        const count = this.chr1kCount();
        const bankReg = this._chrBanks[slot];
        const bank = ((this._chrUpper << 8) | bankReg) % count;
        const idx = bank * Mmc5.Chr1kSize + offset;
        if (idx >= this._chrSize) return 0x00;
        return this._chr[idx];
    }

    writeChr(addr: number, value: number): void {
        if (!this._chrIsRam) return;
        const slot = Math.floor(addr / Mmc5.Chr1kSize);
        const offset = addr & (Mmc5.Chr1kSize - 1);
        const count = this.chr1kCount();
        const bankReg = this._chrBanks[slot];
        const bank = ((this._chrUpper << 8) | bankReg) % count;
        const idx = bank * Mmc5.Chr1kSize + offset;
        if (idx < this._chrSize) this._chr[idx] = value;
    }

    mirrorMode(): number { return this.decodeMirroring(); }
    chrIsRam(): boolean { return this._chrIsRam; }
    hasBattery(): boolean { return this._hasBattery; }
    irqPending(): boolean { return this._irqPending; }

    clockIrq(): void {
        this._scanlineCounter = (this._scanlineCounter + 1) & 0xFF;
        if (this._scanlineCounter >= 240) this._inVblank = true;
        if (this._scanlineCounter === this._irqScanline && (this._irqControl & 0x80) !== 0)
            this._irqPending = true;
    }

    resetScanlineCounter(): void {
        this._scanlineCounter = 0;
        this._inVblank = false;
    }

    saveState(buf: Uint8Array | null): number {
        const total = 64 + Mmc5.PrgRamSize + (this._chrIsRam ? this._chrSize : 0);
        if (buf === null) return total;
        let p = 0;
        buf[p++] = this._prgMode; buf[p++] = this._chrMode;
        buf[p++] = this._prgRamProtect1; buf[p++] = this._prgRamProtect2;
        buf[p++] = this._ntMirroring; buf[p++] = this._fillTile;
        buf[p++] = this._fillAttr; buf[p++] = this._prgRamBank;
        for (let i = 0; i < 4; ++i) buf[p++] = this._prgBanks[i];
        for (let i = 0; i < 8; ++i) buf[p++] = this._chrBanks[i];
        for (let i = 0; i < 4; ++i) buf[p++] = this._chrBanksEx[i];
        buf[p++] = this._chrUpper; buf[p++] = this._splitControl;
        buf[p++] = this._splitYScroll; buf[p++] = this._splitBank;
        buf[p++] = this._irqScanline; buf[p++] = this._irqControl;
        buf[p++] = this._scanlineCounter;
        buf[p++] = this._irqPending ? 1 : 0;
        buf[p++] = this._inVblank ? 1 : 0;
        buf[p++] = this._multA; buf[p++] = this._multB;
        while (p < 64) buf[p++] = 0;
        buf.set(this._prgRam, p); p += Mmc5.PrgRamSize;
        if (this._chrIsRam && this._chrSize > 0) {
            buf.set(this._chr.subarray(0, this._chrSize), p);
            p += this._chrSize;
        }
        return p;
    }

    loadState(buf: Uint8Array, len: number): boolean {
        const need = 64 + Mmc5.PrgRamSize + (this._chrIsRam ? this._chrSize : 0);
        if (len < need) return false;
        let p = 0;
        this._prgMode = buf[p++]; this._chrMode = buf[p++];
        this._prgRamProtect1 = buf[p++]; this._prgRamProtect2 = buf[p++];
        this._ntMirroring = buf[p++]; this._fillTile = buf[p++];
        this._fillAttr = buf[p++]; this._prgRamBank = buf[p++];
        for (let i = 0; i < 4; ++i) this._prgBanks[i] = buf[p++];
        for (let i = 0; i < 8; ++i) this._chrBanks[i] = buf[p++];
        for (let i = 0; i < 4; ++i) this._chrBanksEx[i] = buf[p++];
        this._chrUpper = buf[p++]; this._splitControl = buf[p++];
        this._splitYScroll = buf[p++]; this._splitBank = buf[p++];
        this._irqScanline = buf[p++]; this._irqControl = buf[p++];
        this._scanlineCounter = buf[p++];
        this._irqPending = buf[p++] !== 0;
        this._inVblank = buf[p++] !== 0;
        this._multA = buf[p++]; this._multB = buf[p++];
        p = 64;
        this._prgRam.set(buf.subarray(p, p + Mmc5.PrgRamSize)); p += Mmc5.PrgRamSize;
        if (this._chrIsRam && this._chrSize > 0) {
            this._chr.set(buf.subarray(p, p + this._chrSize));
            p += this._chrSize;
        }
        return true;
    }
}
