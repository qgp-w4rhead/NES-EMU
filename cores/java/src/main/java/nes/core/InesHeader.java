package nes.core;

// Parsed iNES header fields (port of cores/csharp/src/Mapper.cs InesHeader).
public final class InesHeader {
    public int prgRomBanks;
    public int chrRomBanks;
    public int mapperNumber;
    public int mirroring;
    public boolean hasTrainer;
    public boolean hasBattery;
    public int tvSystem;

    public InesHeader() {}
}
