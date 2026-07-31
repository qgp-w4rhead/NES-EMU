using System.Runtime.CompilerServices;

namespace NesCore;

// Mapper 9 (MMC2). Port of src/mappers/mmc2.rs.
public sealed class Mmc2 : Mapper
{
    private const uint PrgBankSize = 8192;
    private const uint Chr4kSize = 4096;
    private const uint PrgRamSize = 1024;
    private const ushort LatchLeftB0Lo = 0x0FD8, LatchLeftB0Hi = 0x0FDF;
    private const ushort LatchLeftB1Lo = 0x0FE8, LatchLeftB1Hi = 0x0FEF;
    private const ushort LatchRightB0Lo = 0x1FD8, LatchRightB0Hi = 0x1FDF;
    private const ushort LatchRightB1Lo = 0x1FE8, LatchRightB1Hi = 0x1FEF;

    private byte[] _prgRom;
    private uint _prgSize;
    private byte[] _chr;
    private uint _chrSize;
    private bool _chrIsRam;
    private byte[] _prgRam = new byte[PrgRamSize];
    private bool _hasBattery;
    private byte _prgBank;
    private byte[] _chrBanks = new byte[4];
    private byte _latchLeft;
    private byte _latchRight;
    private Mirroring _mirroring;

    public Mmc2(byte[] prg, uint prgSize, byte[] chr, uint chrSize, Mirroring mirroring, bool hasBattery)
    {
        MapperNum = 9;
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
        _hasBattery = hasBattery;
        _prgBank = 0;
        _mirroring = mirroring;
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private uint PrgBankCount() => _prgSize / PrgBankSize > 0 ? _prgSize / PrgBankSize : 1;
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private uint ChrBankCount() => _chrSize / Chr4kSize > 0 ? _chrSize / Chr4kSize : 1;

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private byte PrgReadBank(uint bank, uint offset)
    {
        uint count = PrgBankCount();
        bank %= count;
        uint idx = bank * PrgBankSize + offset;
        if (idx >= _prgSize) return 0x00;
        return _prgRom[idx];
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private byte LeftBank() => _latchLeft == 0 ? _chrBanks[0] : _chrBanks[1];
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private byte RightBank() => _latchRight == 0 ? _chrBanks[2] : _chrBanks[3];

    private void UpdateLatches(ushort addr)
    {
        if (addr >= LatchLeftB0Lo && addr <= LatchLeftB0Hi) _latchLeft = 0;
        else if (addr >= LatchLeftB1Lo && addr <= LatchLeftB1Hi) _latchLeft = 1;
        else if (addr >= LatchRightB0Lo && addr <= LatchRightB0Hi) _latchRight = 0;
        else if (addr >= LatchRightB1Lo && addr <= LatchRightB1Hi) _latchRight = 1;
    }

    public override byte ReadPrg(ushort addr)
    {
        if (addr >= 0x6000 && addr < 0x8000)
            return _prgRam[(uint)(addr - 0x6000) & (PrgRamSize - 1)];
        uint local = (uint)(addr - 0x8000);
        uint count = PrgBankCount();
        if (local < PrgBankSize)
        {
            uint bank = _prgBank % count;
            return PrgReadBank(bank, local);
        }
        uint fixedOffset = local - PrgBankSize;
        uint fixedBankBase = count >= 3 ? count - 3 : 0;
        uint bank2 = fixedBankBase + (fixedOffset / PrgBankSize);
        uint offset = fixedOffset & (PrgBankSize - 1);
        return PrgReadBank(bank2, offset);
    }

    public override void WritePrg(ushort addr, byte value)
    {
        if (addr >= 0x6000 && addr < 0x8000)
        {
            _prgRam[(uint)(addr - 0x6000) & (PrgRamSize - 1)] = value;
            return;
        }
        switch (addr)
        {
            case 0xA000: _prgBank = (byte)(value & 0x0F); break;
            case 0xB000: _chrBanks[0] = (byte)(value & 0x3F); break;
            case 0xB001: _chrBanks[1] = (byte)(value & 0x3F); break;
            case 0xB002: _chrBanks[2] = (byte)(value & 0x3F); break;
            case 0xB003: _chrBanks[3] = (byte)(value & 0x3F); break;
        }
    }

    public override byte ReadChr(ushort addr)
    {
        byte bank = (addr < Chr4kSize) ? LeftBank() : RightBank();
        uint count = ChrBankCount();
        uint b = bank % count;
        uint offset = addr & (Chr4kSize - 1);
        uint idx = b * Chr4kSize + offset;
        if (idx >= _chrSize) return 0x00;
        return _chr[idx];
    }

    public override byte ReadChrLatched(ushort addr)
    {
        UpdateLatches(addr);
        return ReadChr(addr);
    }

    public override void WriteChr(ushort addr, byte value)
    {
        if (!_chrIsRam) return;
        byte bank = (addr < Chr4kSize) ? LeftBank() : RightBank();
        uint count = ChrBankCount();
        uint b = bank % count;
        uint offset = addr & (Chr4kSize - 1);
        uint idx = b * Chr4kSize + offset;
        if (idx < _chrSize) _chr[idx] = value;
    }

    public override Mirroring MirrorMode() => _mirroring;
    public override bool ChrIsRam() => _chrIsRam;
    public override bool HasBattery() => _hasBattery;

    public override uint SaveState(byte[] buf)
    {
        uint total = 6 + PrgRamSize + (_chrIsRam ? _chrSize : 0);
        if (buf == null) return total;
        int p = 0;
        buf[p++] = _prgBank;
        for (int i = 0; i < 4; ++i) buf[p++] = _chrBanks[i];
        buf[p++] = _latchLeft;
        buf[p++] = _latchRight;
        System.Array.Copy(_prgRam, 0, buf, p, (int)PrgRamSize); p += (int)PrgRamSize;
        if (_chrIsRam && _chrSize > 0)
        {
            System.Array.Copy(_chr, 0, buf, p, (int)_chrSize);
            p += (int)_chrSize;
        }
        return (uint)p;
    }

    public override bool LoadState(byte[] buf, uint len)
    {
        uint need = 6 + PrgRamSize + (_chrIsRam ? _chrSize : 0);
        if (len < need) return false;
        int p = 0;
        _prgBank = buf[p++];
        for (int i = 0; i < 4; ++i) _chrBanks[i] = buf[p++];
        _latchLeft = buf[p++];
        _latchRight = buf[p++];
        System.Array.Copy(buf, p, _prgRam, 0, (int)PrgRamSize); p += (int)PrgRamSize;
        if (_chrIsRam && _chrSize > 0)
        {
            System.Array.Copy(buf, p, _chr, 0, (int)_chrSize);
            p += (int)_chrSize;
        }
        return true;
    }
}
