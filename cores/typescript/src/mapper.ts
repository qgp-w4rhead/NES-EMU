// Mapper base class + mirroring enum + iNES header + factory.
// Port of cores/c/src/mappers.c.

export enum Mirroring {
    Horizontal = 0,
    Vertical = 1,
    FourScreen = 2,
    SingleScreen0 = 3,
    SingleScreen1 = 4,
    SingleScreen2 = 5,
    SingleScreen3 = 6,
}

export interface InesHeader {
    prgRomBanks: number;
    chrRomBanks: number;
    mapperNumber: number;
    mirroring: number;
    hasTrainer: boolean;
    hasBattery: boolean;
    tvSystem: number;
}

export function makeInesHeader(): InesHeader {
    return { prgRomBanks: 0, chrRomBanks: 0, mapperNumber: 0, mirroring: Mirroring.Horizontal, hasTrainer: false, hasBattery: false, tvSystem: 0 };
}

// DMC DMA read callback type.
export type DmcReadFn = (addr: number) => number;
// CHR pattern-table read callback type (render).
export type ChrReader = (addr: number) => number;

// Mapper base class. Subclasses implement the virtual methods.
// PRG addresses are $6000..=FFFF; CHR are $0000..=1FFF.
export abstract class Mapper {
    mapperNum: number = 0;

    abstract readPrg(addr: number): number;
    readPrgMut(addr: number): number { return this.readPrg(addr); }
    abstract writePrg(addr: number, value: number): void;
    abstract readChr(addr: number): number;
    readChrLatched(addr: number): number { return this.readChr(addr); }
    abstract writeChr(addr: number, value: number): void;
    abstract mirrorMode(): number;
    chrIsRam(): boolean { return false; }
    hasBattery(): boolean { return false; }
    irqPending(): boolean { return false; }
    clockIrq(): void { }
    resetScanlineCounter(): void { }
    clockCpu(cpuCycles: number): void { }
    expansionAudioSample(): number { return 0.0; }
    // Save state: if buf is null, return required size; else write and return bytes written.
    saveState(buf: Uint8Array | null): number { return 0; }
    loadState(buf: Uint8Array, len: number): boolean { return true; }
}

// Mapper factory (port of mappers/mod.rs rom_ines).
export function createMapper(mapperNumber: number, prgRom: Uint8Array, prgSize: number,
    chrRom: Uint8Array | null, chrSize: number, mirroring: number, hasBattery: boolean): Mapper {
    switch (mapperNumber) {
        case 0: return new (require('./mappers/nrom').Nrom)(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery);
        case 1: return new (require('./mappers/mmc1').Mmc1)(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery);
        case 2: return new (require('./mappers/uxrom').Uxrom)(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery);
        case 3: return new (require('./mappers/cnrom').Cnrom)(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery);
        case 4: return new (require('./mappers/mmc3').Mmc3)(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery);
        case 5: return new (require('./mappers/mmc5').Mmc5)(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery);
        case 7: return new (require('./mappers/axrom').Axrom)(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery);
        case 9: return new (require('./mappers/mmc2').Mmc2)(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery);
        case 19: return new (require('./mappers/namco163').Namco163)(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery);
        case 24: return new (require('./mappers/vrc6').Vrc6)(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery, false);
        case 26: return new (require('./mappers/vrc6').Vrc6)(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery, true);
        case 69: return new (require('./mappers/fme7').Fme7)(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery);
        case 85: return new (require('./mappers/vrc7').Vrc7)(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery);
        default: throw new Error(`unsupported mapper ${mapperNumber}`);
    }
}

// FDS mapper factory (mapper 20). FDS uses BIOS + disk data instead of PRG/CHR ROM.
export function createFdsMapper(bios: Uint8Array, biosLen: number, diskData: Uint8Array | null, diskLen: number): Mapper {
    return new (require('./mappers/fds').Fds)(bios, biosLen, diskData, diskLen);
}
