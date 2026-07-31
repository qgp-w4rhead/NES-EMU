using System.Runtime.CompilerServices;

namespace NesCore;

// iNES cartridge loader + mapper dispatch (port of src/cartridge.rs).
public sealed class Cartridge
{
    public const int HeaderSize = 16;
    public const int TrainerSize = 512;
    public const uint PrgRomUnit = 16384;
    public const uint ChrRomUnit = 8192;

    private static readonly byte[] InesMagic = { (byte)'N', (byte)'E', (byte)'S', 0x1A };
    private static readonly byte[] FdsMagic = { (byte)'F', (byte)'D', (byte)'S', 0x1A };
    private const uint FdsHeaderSize = 16;
    private const uint FdsDiskSideSize = 65500;

    public InesHeader Header;
    public Mapper Mapper;

    public Cartridge() { }

    public static int ParseHeader(byte[] bytes, int len, out InesHeader hdr)
    {
        hdr = default;
        if (len < HeaderSize) return 1;
        for (int i = 0; i < 4; ++i)
            if (bytes[i] != InesMagic[i]) return 2;

        byte prgRomBanks = bytes[4];
        byte chrRomBanks = bytes[5];
        byte flags6 = bytes[6];
        byte flags7 = bytes[7];

        bool hasTrainer = (flags6 & 0x04) != 0;
        bool hasBattery = (flags6 & 0x02) != 0;
        bool fourScreen = (flags6 & 0x08) != 0;
        bool vertical = (flags6 & 0x01) != 0;

        Mirroring mirroring;
        if (fourScreen) mirroring = Mirroring.FourScreen;
        else if (vertical) mirroring = Mirroring.Vertical;
        else mirroring = Mirroring.Horizontal;

        ushort mapperNumber = (ushort)(((ushort)(flags6 >> 4)) | (ushort)(((ushort)(flags7 >> 4)) << 4));
        byte tvSystem = (byte)(bytes[9] & 0x03);

        hdr.PrgRomBanks = prgRomBanks;
        hdr.ChrRomBanks = chrRomBanks;
        hdr.MapperNumber = mapperNumber;
        hdr.Mirroring = mirroring;
        hdr.HasTrainer = hasTrainer;
        hdr.HasBattery = hasBattery;
        hdr.TvSystem = tvSystem;
        return 0;
    }

    public static int FromBytes(byte[] bytes, int len, out Cartridge cart)
    {
        cart = new Cartridge();
        int rc = ParseHeader(bytes, len, out InesHeader hdr);
        if (rc != 0) return rc;

        uint prgSize = (uint)hdr.PrgRomBanks * PrgRomUnit;
        int prgOff = HeaderSize;
        if (hdr.HasTrainer) prgOff += TrainerSize;
        uint chrSize = (hdr.ChrRomBanks > 0) ? (uint)hdr.ChrRomBanks * ChrRomUnit : 0u;
        if (len < prgOff + prgSize + chrSize) return 5;

        byte[] prgRom = new byte[prgSize];
        System.Array.Copy(bytes, prgOff, prgRom, 0, (int)prgSize);
        byte[] chrRom = (chrSize > 0) ? new byte[chrSize] : null;
        if (chrRom != null)
            System.Array.Copy(bytes, prgOff + (int)prgSize, chrRom, 0, (int)chrSize);

        cart.Header = hdr;
        cart.Mapper = MapperFactory.Create(hdr.MapperNumber, prgRom, prgSize, chrRom, chrSize,
                                            hdr.Mirroring, hdr.HasBattery);
        return 0;
    }

    public static int FromFdsBytes(byte[] diskData, int diskLen, byte[] bios, int biosLen, out Cartridge cart)
    {
        cart = new Cartridge();
        if (diskLen < FdsHeaderSize) return 1;
        for (int i = 0; i < 4; ++i)
            if (diskData[i] != FdsMagic[i]) return 2;
        byte diskCount = diskData[4];
        if (diskCount == 0) return 6;
        uint needed = FdsHeaderSize + (uint)diskCount * FdsDiskSideSize;
        if (diskLen < needed) return 1;

        uint rawSize = (uint)diskCount * FdsDiskSideSize;
        byte[] rawDisk = new byte[rawSize];
        System.Array.Copy(diskData, (int)FdsHeaderSize, rawDisk, 0, (int)rawSize);

        cart.Header = new InesHeader
        {
            MapperNumber = 20,
            Mirroring = Mirroring.Vertical,
            PrgRomBanks = 0,
            ChrRomBanks = 0,
        };
        cart.Mapper = new Fds(bios, biosLen, rawDisk, rawSize);
        return 0;
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public byte ReadPrg(ushort addr) => Mapper.ReadPrg(addr);
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public byte ReadPrgMut(ushort addr) => Mapper.ReadPrgMut(addr);
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public void WritePrg(ushort addr, byte value) => Mapper.WritePrg(addr, value);
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public byte ReadChr(ushort addr) => Mapper.ReadChr(addr);
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public byte ReadChrLatched(ushort addr) => Mapper.ReadChrLatched(addr);
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public void WriteChr(ushort addr, byte value) => Mapper.WriteChr(addr, value);
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public Mirroring MirrorMode() => Mapper.MirrorMode();
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public bool ChrIsRam() => Mapper.ChrIsRam();
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public bool HasBattery() => Mapper.HasBattery();
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public bool IrqPending() => Mapper.IrqPending();
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public void ClockIrq() => Mapper.ClockIrq();
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public void ResetScanlineCounter() => Mapper.ResetScanlineCounter();
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public void ClockCpu(uint cpuCycles) => Mapper.ClockCpu(cpuCycles);
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public float ExpansionAudioSample() => Mapper.ExpansionAudioSample();
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public uint SaveState(byte[] buf) => Mapper.SaveState(buf);
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public bool LoadState(byte[] buf, uint len) => Mapper.LoadState(buf, len);
}
