// iNES cartridge loader + mapper dispatch (port of cores/c/src/cartridge.c).

import { InesHeader, makeInesHeader, Mapper, Mirroring, createMapper } from './mapper';

export class Cartridge {
    static readonly HeaderSize = 16;
    static readonly TrainerSize = 512;
    static readonly PrgRomUnit = 16384;
    static readonly ChrRomUnit = 8192;

    static readonly InesMagic = [0x4E, 0x45, 0x53, 0x1A]; // "NES\x1A"

    header: InesHeader = makeInesHeader();
    mapper!: Mapper;

    static parseHeader(bytes: Uint8Array, len: number, hdr: InesHeader): number {
        if (len < Cartridge.HeaderSize) return 1;
        for (let i = 0; i < 4; ++i)
            if (bytes[i] !== Cartridge.InesMagic[i]) return 2;

        const prgRomBanks = bytes[4];
        const chrRomBanks = bytes[5];
        const flags6 = bytes[6];
        const flags7 = bytes[7];

        const hasTrainer = (flags6 & 0x04) !== 0;
        const hasBattery = (flags6 & 0x02) !== 0;
        const fourScreen = (flags6 & 0x08) !== 0;
        const vertical = (flags6 & 0x01) !== 0;

        let mirroring: number;
        if (fourScreen) mirroring = Mirroring.FourScreen;
        else if (vertical) mirroring = Mirroring.Vertical;
        else mirroring = Mirroring.Horizontal;

        const mapperNumber = ((flags6 >> 4) | ((flags7 >> 4) << 4)) & 0xFFFF;
        const tvSystem = bytes[9] & 0x03;

        hdr.prgRomBanks = prgRomBanks;
        hdr.chrRomBanks = chrRomBanks;
        hdr.mapperNumber = mapperNumber;
        hdr.mirroring = mirroring;
        hdr.hasTrainer = hasTrainer;
        hdr.hasBattery = hasBattery;
        hdr.tvSystem = tvSystem;
        return 0;
    }

    static fromBytes(bytes: Uint8Array, len: number): Cartridge | null {
        const cart = new Cartridge();
        const hdr = makeInesHeader();
        const rc = Cartridge.parseHeader(bytes, len, hdr);
        if (rc !== 0) return null;

        const prgSize = hdr.prgRomBanks * Cartridge.PrgRomUnit;
        let prgOff = Cartridge.HeaderSize;
        if (hdr.hasTrainer) prgOff += Cartridge.TrainerSize;
        const chrSize = (hdr.chrRomBanks > 0) ? hdr.chrRomBanks * Cartridge.ChrRomUnit : 0;
        if (len < prgOff + prgSize + chrSize) return null;

        const prgRom = new Uint8Array(prgSize);
        prgRom.set(bytes.subarray(prgOff, prgOff + prgSize));
        let chrRom: Uint8Array | null = null;
        if (chrSize > 0) {
            chrRom = new Uint8Array(chrSize);
            chrRom.set(bytes.subarray(prgOff + prgSize, prgOff + prgSize + chrSize));
        }

        cart.header = hdr;
        cart.mapper = createMapper(hdr.mapperNumber, prgRom, prgSize, chrRom, chrSize, hdr.mirroring, hdr.hasBattery);
        return cart;
    }

    readPrg(addr: number): number { return this.mapper.readPrg(addr); }
    readPrgMut(addr: number): number { return this.mapper.readPrgMut(addr); }
    writePrg(addr: number, value: number): void { this.mapper.writePrg(addr, value); }
    readChr(addr: number): number { return this.mapper.readChr(addr); }
    readChrLatched(addr: number): number { return this.mapper.readChrLatched(addr); }
    writeChr(addr: number, value: number): void { this.mapper.writeChr(addr, value); }
    mirrorMode(): number { return this.mapper.mirrorMode(); }
    chrIsRam(): boolean { return this.mapper.chrIsRam(); }
    hasBattery(): boolean { return this.mapper.hasBattery(); }
    irqPending(): boolean { return this.mapper.irqPending(); }
    clockIrq(): void { this.mapper.clockIrq(); }
    resetScanlineCounter(): void { this.mapper.resetScanlineCounter(); }
    clockCpu(cpuCycles: number): void { this.mapper.clockCpu(cpuCycles); }
    expansionAudioSample(): number { return this.mapper.expansionAudioSample(); }
    saveState(buf: Uint8Array | null): number { return this.mapper.saveState(buf); }
    loadState(buf: Uint8Array, len: number): boolean { return this.mapper.loadState(buf, len); }
}
