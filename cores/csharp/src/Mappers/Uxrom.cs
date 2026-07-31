using System.Runtime.CompilerServices;

namespace NesCore;

// Mapper 2 (UxROM). Port of src/mappers/uxrom.rs.
public sealed class Uxrom : Mapper
{
    private const uint PrgBankSize = 16384;
    private const uint ChrSize = 8192;

    private byte[] _prgRom;
    private uint _prgSize;
    private byte[] _chr;
    private uint _chrSize;
    private bool _chrIsRam;
    private Mirroring _mirroring;
    private bool _hasBattery;
    private byte _bank;

    public Uxrom(byte[] prg, uint prgSize, byte[] chr, uint chrSize, Mirroring mirroring, bool hasBattery)
    {
        MapperNum = 2;
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
            _chrSize = ChrSize;
            _chr = new byte[ChrSize];
            _chrIsRam = true;
        }
        _mirroring = mirroring;
        _hasBattery = hasBattery;
        _bank = 0;
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private uint PrgBankCount() => _prgSize / PrgBankSize > 0 ? _prgSize / PrgBankSize : 1;

    public override byte ReadPrg(ushort addr)
    {
        if (addr < 0x8000) return 0x00;
        uint local = (uint)(addr - 0x8000);
        uint offset = local & (PrgBankSize - 1);
        uint count = PrgBankCount();
        uint idx;
        if (local < PrgBankSize)
        {
            uint bank = _bank % count;
            idx = bank * PrgBankSize + offset;
        }
        else
        {
            uint last = count - 1;
            idx = last * PrgBankSize + offset;
        }
        if (idx >= _prgSize) return 0x00;
        return _prgRom[idx];
    }

    public override void WritePrg(ushort addr, byte value)
    {
        if (addr < 0x8000) return;
        _bank = value;
    }

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
        uint total = 1 + (_chrIsRam ? _chrSize : 0);
        if (buf == null) return total;
        int p = 0;
        buf[p++] = _bank;
        if (_chrIsRam && _chrSize > 0)
        {
            System.Array.Copy(_chr, 0, buf, p, (int)_chrSize);
            p += (int)_chrSize;
        }
        return (uint)p;
    }

    public override bool LoadState(byte[] buf, uint len)
    {
        uint need = 1 + (_chrIsRam ? _chrSize : 0);
        if (len < need) return false;
        int p = 0;
        _bank = buf[p++];
        if (_chrIsRam && _chrSize > 0)
        {
            System.Array.Copy(buf, p, _chr, 0, (int)_chrSize);
            p += (int)_chrSize;
        }
        return true;
    }
}
