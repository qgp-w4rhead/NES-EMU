using System.Runtime.CompilerServices;

namespace NesCore;

// Mapper 3 (CNROM). Port of src/mappers/cnrom.rs.
public sealed class Cnrom : Mapper
{
    private const uint PrgBankSize = 16384;
    private const uint ChrBankSize = 8192;

    private byte[] _prgRom;
    private uint _prgSize;
    private byte[] _chr;
    private uint _chrSize;
    private bool _chrIsRam;
    private Mirroring _mirroring;
    private bool _hasBattery;
    private byte _chrBank;

    public Cnrom(byte[] prg, uint prgSize, byte[] chr, uint chrSize, Mirroring mirroring, bool hasBattery)
    {
        MapperNum = 3;
        _prgSize = prgSize;
        _prgRom = new byte[prgSize];
        if (prgSize > 0) System.Array.Copy(prg, _prgRom, (int)prgSize);
        if (chrSize == 0)
        {
            _chrSize = ChrBankSize;
            _chr = new byte[ChrBankSize];
            _chrIsRam = true;
        }
        else
        {
            _chrSize = chrSize;
            _chr = new byte[chrSize];
            System.Array.Copy(chr, _chr, (int)chrSize);
            _chrIsRam = false;
        }
        _mirroring = mirroring;
        _hasBattery = hasBattery;
        _chrBank = 0;
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private uint ChrBankCount() => _chrSize / ChrBankSize > 0 ? _chrSize / ChrBankSize : 1;
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

    public override void WritePrg(ushort addr, byte value)
    {
        if (addr < 0x8000) return;
        _chrBank = value;
    }

    public override byte ReadChr(ushort addr)
    {
        uint count = ChrBankCount();
        uint bank = _chrBank % count;
        uint idx = bank * ChrBankSize + ((uint)addr & (ChrBankSize - 1));
        if (idx >= _chrSize) return 0x00;
        return _chr[idx];
    }

    public override void WriteChr(ushort addr, byte value)
    {
        if (!_chrIsRam) return;
        uint count = ChrBankCount();
        uint bank = _chrBank % count;
        uint idx = bank * ChrBankSize + ((uint)addr & (ChrBankSize - 1));
        if (idx < _chrSize) _chr[idx] = value;
    }

    public override Mirroring MirrorMode() => _mirroring;
    public override bool ChrIsRam() => _chrIsRam;
    public override bool HasBattery() => _hasBattery;

    public override uint SaveState(byte[] buf)
    {
        uint total = 1 + (_chrIsRam ? _chrSize : 0);
        if (buf == null) return total;
        int p = 0;
        buf[p++] = _chrBank;
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
        _chrBank = buf[p++];
        if (_chrIsRam && _chrSize > 0)
        {
            System.Array.Copy(buf, p, _chr, 0, (int)_chrSize);
            p += (int)_chrSize;
        }
        return true;
    }
}
