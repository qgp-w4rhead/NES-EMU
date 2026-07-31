using System.Runtime.CompilerServices;

namespace NesCore;

// Nametable mirroring mode (mappers Mirroring).
public enum Mirroring : byte
{
    Horizontal = 0,
    Vertical = 1,
    FourScreen = 2,
    SingleScreen0 = 3,
    SingleScreen1 = 4,
    SingleScreen2 = 5,
    SingleScreen3 = 6,
}

// Parsed iNES header fields (cartridge.rs InesHeader).
public struct InesHeader
{
    public byte PrgRomBanks;
    public byte ChrRomBanks;
    public ushort MapperNumber;
    public Mirroring Mirroring;
    public bool HasTrainer;
    public bool HasBattery;
    public byte TvSystem;
}

// Mapper base class. Subclasses (sealed) implement the virtual methods.
// PRG addresses are $6000..=FFFF; CHR are $0000..=1FFF.
public abstract class Mapper
{
    public ushort MapperNum;

    public abstract byte ReadPrg(ushort addr);
    public virtual byte ReadPrgMut(ushort addr) => ReadPrg(addr);
    public abstract void WritePrg(ushort addr, byte value);
    public abstract byte ReadChr(ushort addr);
    public virtual byte ReadChrLatched(ushort addr) => ReadChr(addr);
    public abstract void WriteChr(ushort addr, byte value);
    public abstract Mirroring MirrorMode();
    public virtual bool ChrIsRam() => false;
    public virtual bool HasBattery() => false;
    public virtual bool IrqPending() => false;
    public virtual void ClockIrq() { }
    public virtual void ResetScanlineCounter() { }
    public virtual void ClockCpu(uint cpuCycles) { }
    public virtual float ExpansionAudioSample() => 0.0f;

    // Save state: if buf is null, return required size; else write and return bytes written.
    public virtual uint SaveState(byte[] buf) => 0;
    public virtual bool LoadState(byte[] buf, uint len) => true;
}

// Mapper factory (port of mappers/mod.rs rom_ines).
public static class MapperFactory
{
    public static Mapper Create(ushort mapperNumber, byte[] prgRom, uint prgSize,
        byte[] chrRom, uint chrSize, Mirroring mirroring, bool hasBattery)
    {
        return mapperNumber switch
        {
            0 => new Nrom(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery),
            1 => new Mmc1(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery),
            2 => new Uxrom(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery),
            3 => new Cnrom(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery),
            4 => new Mmc3(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery),
            5 => new Mmc5(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery),
            7 => new Axrom(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery),
            9 => new Mmc2(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery),
            19 => new Namco163(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery),
            24 => new Vrc6(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery, false),
            26 => new Vrc6(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery, true),
            69 => new Fme7(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery),
            85 => new Vrc7(prgRom, prgSize, chrRom, chrSize, mirroring, hasBattery),
            _ => throw new System.ArgumentException($"unsupported mapper {mapperNumber}"),
        };
    }
}
