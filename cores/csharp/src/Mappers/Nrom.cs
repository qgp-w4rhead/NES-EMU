using System.Runtime.CompilerServices;

namespace NesCore;

// Mapper 0 (NROM). Port of src/mappers/nrom.rs.
public sealed class Nrom : Mapper
{
    private byte[] _prgRom;
    private uint _prgSize;
    private byte[] _chr;
    private uint _chrSize;
    private bool _chrIsRam;
    private Mirroring _mirroring;
    private bool _hasBattery;

    public Nrom(byte[] prg, uint prgSize, byte[] chr, uint chrSize, Mirroring mirroring, bool hasBattery)
    {
        MapperNum = 0;
        _prgSize = prgSize;
        _prgRom = new byte[prgSize];
        if (prgSize > 0) System.Array.Copy(prg, _prgRom, (int)prgSize);
        if (chrSize > 0)
        {
            _chrSize = chrSize;
            _chr = new byte[chrSize];
            System.Array.Copy(chr, _chr, (int)chrSize);
            _chrIsRam = false;
        }
        else
        {
            _chrSize = 8192;
            _chr = new byte[8192];
            _chrIsRam = true;
        }
        _mirroring = mirroring;
        _hasBattery = hasBattery;
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private uint PrgIndex(ushort addr)
    {
        uint local = (uint)(addr - 0x8000);
        uint bank = _prgSize;
        if (bank == 0) return 0;
        return local % bank;
    }

    public override byte ReadPrg(ushort addr)
    {
        if (addr < 0x8000) return 0x00;
        uint idx = PrgIndex(addr);
        if (idx >= _prgSize) return 0x00;
        return _prgRom[idx];
    }

    public override void WritePrg(ushort addr, byte value) { }

    public override byte ReadChr(ushort addr)
    {
        uint sz = _chrSize;
        if (sz == 0) return 0x00;
        return _chr[addr % sz];
    }

    public override void WriteChr(ushort addr, byte value)
    {
        if (!_chrIsRam) return;
        uint sz = _chrSize;
        if (sz == 0) return;
        _chr[addr % sz] = value;
    }

    public override Mirroring MirrorMode() => _mirroring;
    public override bool ChrIsRam() => _chrIsRam;
    public override bool HasBattery() => _hasBattery;

    public override uint SaveState(byte[] buf)
    {
        uint total = 0;
        // We serialize the mutable fields: chr (if RAM) + mirroring/has_battery flags.
        // Layout: [u8 chr_is_ram][u8 mirroring][u8 has_battery][u32 chr_size][chr bytes if ram]
        if (buf == null)
        {
            total = 7 + (_chrIsRam ? _chrSize : 0);
            return total;
        }
        int p = 0;
        buf[p++] = (byte)(_chrIsRam ? 1 : 0);
        buf[p++] = (byte)_mirroring;
        buf[p++] = (byte)(_hasBattery ? 1 : 0);
        System.BitConverter.GetBytes(_chrSize).CopyTo(buf, p); p += 4;
        if (_chrIsRam && _chrSize > 0)
        {
            System.Array.Copy(_chr, 0, buf, p, (int)_chrSize);
            p += (int)_chrSize;
        }
        return (uint)p;
    }

    public override bool LoadState(byte[] buf, uint len)
    {
        if (len < 7) return false;
        int p = 0;
        _chrIsRam = buf[p++] != 0;
        _mirroring = (Mirroring)buf[p++];
        _hasBattery = buf[p++] != 0;
        _chrSize = System.BitConverter.ToUInt32(buf, p); p += 4;
        if (_chrIsRam && _chrSize > 0)
        {
            if (len < (uint)(p + _chrSize)) return false;
            System.Array.Copy(buf, p, _chr, 0, (int)_chrSize);
            p += (int)_chrSize;
        }
        return true;
    }
}
