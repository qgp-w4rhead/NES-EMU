using System.Runtime.CompilerServices;

namespace NesCore;

// Mapper 7 (AxROM). Port of src/mappers/axrom.rs.
public sealed class Axrom : Mapper
{
    private const uint PrgBankSize = 32768;
    private const uint ChrSize = 8192;

    private byte[] _prgRom;
    private uint _prgSize;
    private byte[] _chr;
    private uint _chrSize;
    private bool _chrIsRam;
    private bool _hasBattery;
    private byte _prgBank;
    private byte _mirrorNt;

    public Axrom(byte[] prg, uint prgSize, byte[] chr, uint chrSize, Mirroring mirroring, bool hasBattery)
    {
        MapperNum = 7;
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
        _hasBattery = hasBattery;
        _prgBank = 0;
        _mirrorNt = 0;
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private uint PrgBankCount() => _prgSize / PrgBankSize > 0 ? _prgSize / PrgBankSize : 1;

    public override byte ReadPrg(ushort addr)
    {
        if (addr < 0x8000) return 0x00;
        uint local = (uint)(addr - 0x8000);
        uint count = PrgBankCount();
        uint bank = _prgBank % count;
        uint idx = bank * PrgBankSize + local;
        if (idx >= _prgSize) return 0x00;
        return _prgRom[idx];
    }

    public override void WritePrg(ushort addr, byte value)
    {
        if (addr < 0x8000) return;
        _mirrorNt = (byte)((value >> 4) & 1);
        _prgBank = (byte)(value & 0x07);
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

    public override Mirroring MirrorMode() => _mirrorNt == 0 ? Mirroring.SingleScreen0 : Mirroring.SingleScreen1;
    public override bool ChrIsRam() => _chrIsRam;
    public override bool HasBattery() => _hasBattery;

    public override uint SaveState(byte[] buf)
    {
        uint total = 2 + (_chrIsRam ? _chrSize : 0);
        if (buf == null) return total;
        int p = 0;
        buf[p++] = _prgBank;
        buf[p++] = _mirrorNt;
        if (_chrIsRam && _chrSize > 0)
        {
            System.Array.Copy(_chr, 0, buf, p, (int)_chrSize);
            p += (int)_chrSize;
        }
        return (uint)p;
    }

    public override bool LoadState(byte[] buf, uint len)
    {
        uint need = 2 + (_chrIsRam ? _chrSize : 0);
        if (len < need) return false;
        int p = 0;
        _prgBank = buf[p++];
        _mirrorNt = buf[p++];
        if (_chrIsRam && _chrSize > 0)
        {
            System.Array.Copy(buf, p, _chr, 0, (int)_chrSize);
            p += (int)_chrSize;
        }
        return true;
    }
}
