package nes.core;

import nes.core.mappers.*;

// Mapper factory (port of cores/csharp/src/Mapper.cs MapperFactory).
public final class MapperFactory {
    private MapperFactory() {}

    public static Mapper create(int mapperNumber, byte[] prgRom, int prgSize,
        byte[] chrRom, int chrSize, int mirroring, boolean hasBattery)
    {
        switch (mapperNumber) {
            case 0: return new Nrom(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery);
            case 1: return new Mmc1(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery);
            case 2: return new Uxrom(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery);
            case 3: return new Cnrom(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery);
            case 4: return new Mmc3(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery);
            case 5: return new Mmc5(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery);
            case 7: return new Axrom(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery);
            case 9: return new Mmc2(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery);
            case 19: return new Namco163(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery);
            case 24: return new Vrc6(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery, false);
            case 26: return new Vrc6(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery, true);
            case 69: return new Fme7(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery);
            case 85: return new Vrc7(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery);
            default: throw new IllegalArgumentException("unsupported mapper " + mapperNumber);
        }
    }
}
