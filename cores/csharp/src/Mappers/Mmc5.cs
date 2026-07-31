using System.Runtime.CompilerServices;

namespace NesCore;

// Mapper 5 (MMC5). Port of src/mappers/mmc5.rs.
public sealed class Mmc5 : Mapper
{
    private const uint PrgBankSize = 8192;
    private const uint Chr1kSize = 1024;
    private const uint PrgRamSize = 65536;
    private const uint PrgRamWindow = 8192;

    private byte[] _prgRom;
    private uint _prgSize;
    private byte[] _chr;
    private uint _chrSize;
    private bool _chrIsRam;
    private byte[] _prgRam = new byte[PrgRamSize];
    private bool _hasBattery;
    private byte _prgMode = 3;
    private byte _chrMode = 3;
    private byte _prgRamProtect1;
    private byte _prgRamProtect2;
    private byte _ntMirroring;
    private byte _fillTile;
    private byte _fillAttr;
    private byte _prgRamBank;
    private byte[] _prgBanks = new byte[4];
    private byte[] _chrBanks = new byte[8];
    private byte[] _chrBanksEx = new byte[4];
    private byte _chrUpper;
    private byte _splitControl;
    private byte _splitYScroll;
    private byte _splitBank;
    private byte _irqScanline;
    private byte _irqControl;
    private byte _scanlineCounter;
    private bool _irqPending;
    private bool _inVblank;
    private byte _multA;
    private byte _multB;

    public Mmc5(byte[] prg, uint prgSize, byte[] chr, uint chrSize, Mirroring mirroring, bool hasBattery)
    {
        MapperNum = 5;
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
        _ntMirroring = mirroring switch
        {
            Mirroring.Horizontal => 0x44,
            Mirroring.Vertical => 0x50,
            Mirroring.FourScreen => 0xE4,
            Mirroring.SingleScreen0 => 0x00,
            Mirroring.SingleScreen1 => 0x55,
            Mirroring.SingleScreen2 => 0xAA,
            Mirroring.SingleScreen3 => 0xFF,
            _ => 0x44,
        };
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private uint PrgBankCount() => _prgSize / PrgBankSize > 0 ? _prgSize / PrgBankSize : 1;
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private uint Chr1kCount() => _chrSize / Chr1kSize > 0 ? _chrSize / Chr1kSize : 1;

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
    private ushort Product() => (ushort)((ushort)_multA * (ushort)_multB);

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private bool PrgRamWritesAllowed() => _prgRamProtect1 == 0x02 && _prgRamProtect2 == 0x01;

    private Mirroring DecodeMirroring()
    {
        byte s0 = (byte)(_ntMirroring & 0x03);
        byte s1 = (byte)((_ntMirroring >> 2) & 0x03);
        byte s2 = (byte)((_ntMirroring >> 4) & 0x03);
        byte s3 = (byte)((_ntMirroring >> 6) & 0x03);
        if (s0 == s1 && s1 == s2 && s2 == s3)
        {
            return s0 switch
            {
                0 => Mirroring.SingleScreen0,
                1 => Mirroring.SingleScreen1,
                2 => Mirroring.SingleScreen2,
                _ => Mirroring.SingleScreen3,
            };
        }
        if (s0 == 0 && s1 == 1 && s2 == 0 && s3 == 1) return Mirroring.Horizontal;
        if (s0 == 0 && s1 == 0 && s2 == 1 && s3 == 1) return Mirroring.Vertical;
        if (s0 == 0 && s1 == 1 && s2 == 2 && s3 == 3) return Mirroring.FourScreen;
        return Mirroring.Horizontal;
    }

    public override byte ReadPrg(ushort addr)
    {
        if (addr == 0x5205) return (byte)(Product() & 0xFF);
        if (addr == 0x5206) return (byte)((Product() >> 8) & 0xFF);
        if (addr == 0x5204)
        {
            byte status = (byte)(_irqControl & 0x80);
            if (_inVblank) status |= 0x40;
            return status;
        }
        if (addr >= 0x6000 && addr < 0x8000)
        {
            uint bank = _prgRamBank % (PrgRamSize / PrgRamWindow);
            uint idx = bank * PrgRamWindow + ((uint)(addr - 0x6000) & (PrgRamWindow - 1));
            if (idx >= PrgRamSize) return 0x00;
            return _prgRam[idx];
        }
        uint local = (uint)(addr - 0x8000);
        uint slot = local / PrgBankSize;
        uint offset = local & (PrgBankSize - 1);
        if (slot == 3)
        {
            uint last = PrgBankCount() - 1;
            return PrgReadBank(last, offset);
        }
        byte reg = _prgBanks[slot];
        if ((reg & 0x80) != 0)
        {
            uint ramBank = (uint)(reg & 0x7F) % (PrgRamSize / PrgBankSize);
            uint idx = ramBank * PrgBankSize + offset;
            if (idx >= PrgRamSize) return 0x00;
            return _prgRam[idx];
        }
        uint count = PrgBankCount();
        uint bank2 = reg % count;
        return PrgReadBank(bank2, offset);
    }

    public override void WritePrg(ushort addr, byte value)
    {
        if (addr == 0x5205) { _multA = value; return; }
        if (addr == 0x5206) { _multB = value; return; }
        if (addr == 0x5204)
        {
            _irqControl = (byte)(value & 0x80);
            if ((value & 0x80) == 0) _irqPending = false;
            return;
        }
        if (addr == 0x5203) { _irqScanline = value; return; }
        if (addr == 0x5200) { _splitControl = value; return; }
        if (addr == 0x5201) { _splitYScroll = value; return; }
        if (addr == 0x5202) { _splitBank = value; return; }
        switch (addr)
        {
            case 0x5100: _prgMode = (byte)(value & 0x03); return;
            case 0x5101: _chrMode = (byte)(value & 0x03); return;
            case 0x5102: _prgRamProtect1 = (byte)(value & 0x03); return;
            case 0x5103: _prgRamProtect2 = (byte)(value & 0x03); return;
            case 0x5104: return;
            case 0x5105: _ntMirroring = value; return;
            case 0x5106: _fillTile = value; return;
            case 0x5107: _fillAttr = value; return;
            case 0x5113: _prgRamBank = value; return;
            case 0x5114: _prgBanks[0] = value; return;
            case 0x5115: _prgBanks[1] = value; return;
            case 0x5116: _prgBanks[2] = value; return;
            case 0x5117: _prgBanks[3] = value; return;
            case 0x5120: _chrBanks[0] = value; return;
            case 0x5121: _chrBanks[1] = value; return;
            case 0x5122: _chrBanks[2] = value; return;
            case 0x5123: _chrBanks[3] = value; return;
            case 0x5124: _chrBanks[4] = value; return;
            case 0x5125: _chrBanks[5] = value; return;
            case 0x5126: _chrBanks[6] = value; return;
            case 0x5127: _chrBanks[7] = value; return;
            case 0x5128: _chrBanksEx[0] = value; return;
            case 0x5129: _chrBanksEx[1] = value; return;
            case 0x512A: _chrBanksEx[2] = value; return;
            case 0x512B: _chrBanksEx[3] = value; return;
            case 0x5130: _chrUpper = (byte)(value & 0x01); return;
        }
        if (addr >= 0x6000 && addr < 0x8000 && PrgRamWritesAllowed())
        {
            uint bank = _prgRamBank % (PrgRamSize / PrgRamWindow);
            uint idx = bank * PrgRamWindow + ((uint)(addr - 0x6000) & (PrgRamWindow - 1));
            if (idx < PrgRamSize) _prgRam[idx] = value;
            return;
        }
        if (addr >= 0x8000 && addr < 0xE000 && PrgRamWritesAllowed())
        {
            uint local = (uint)(addr - 0x8000);
            uint slot = local / PrgBankSize;
            uint offset = local & (PrgBankSize - 1);
            if (slot < 3)
            {
                byte reg = _prgBanks[slot];
                if ((reg & 0x80) != 0)
                {
                    uint ramBank = (uint)(reg & 0x7F) % (PrgRamSize / PrgBankSize);
                    uint idx = ramBank * PrgBankSize + offset;
                    if (idx < PrgRamSize) _prgRam[idx] = value;
                }
            }
        }
    }

    public override byte ReadChr(ushort addr)
    {
        uint slot = addr / Chr1kSize;
        uint offset = addr & (Chr1kSize - 1);
        uint count = Chr1kCount();
        byte bankReg = _chrBanks[slot];
        uint bank = (((uint)_chrUpper << 8) | bankReg) % count;
        uint idx = bank * Chr1kSize + offset;
        if (idx >= _chrSize) return 0x00;
        return _chr[idx];
    }

    public override void WriteChr(ushort addr, byte value)
    {
        if (!_chrIsRam) return;
        uint slot = addr / Chr1kSize;
        uint offset = addr & (Chr1kSize - 1);
        uint count = Chr1kCount();
        byte bankReg = _chrBanks[slot];
        uint bank = (((uint)_chrUpper << 8) | bankReg) % count;
        uint idx = bank * Chr1kSize + offset;
        if (idx < _chrSize) _chr[idx] = value;
    }

    public override Mirroring MirrorMode() => DecodeMirroring();
    public override bool ChrIsRam() => _chrIsRam;
    public override bool HasBattery() => _hasBattery;
    public override bool IrqPending() => _irqPending;

    public override void ClockIrq()
    {
        _scanlineCounter++;
        if (_scanlineCounter >= 240) _inVblank = true;
        if (_scanlineCounter == _irqScanline && (_irqControl & 0x80) != 0) _irqPending = true;
    }

    public override void ResetScanlineCounter()
    {
        _scanlineCounter = 0;
        _inVblank = false;
    }

    public override uint SaveState(byte[] buf)
    {
        // Layout: scalar regs + banks + prg_ram + chr(if ram)
        uint scalarSize = 16; // prg_mode..mult_b
        uint total = scalarSize + 4 + 8 + 4 + PrgRamSize + (_chrIsRam ? _chrSize : 0);
        if (buf == null) return total;
        int p = 0;
        buf[p++] = _prgMode;
        buf[p++] = _chrMode;
        buf[p++] = _prgRamProtect1;
        buf[p++] = _prgRamProtect2;
        buf[p++] = _ntMirroring;
        buf[p++] = _fillTile;
        buf[p++] = _fillAttr;
        buf[p++] = _prgRamBank;
        buf[p++] = _chrUpper;
        buf[p++] = _splitControl;
        buf[p++] = _splitYScroll;
        buf[p++] = _splitBank;
        buf[p++] = _irqScanline;
        buf[p++] = _irqControl;
        buf[p++] = _scanlineCounter;
        buf[p++] = (byte)(_inVblank ? 1 : 0);
        buf[p++] = (byte)(_irqPending ? 1 : 0);
        buf[p++] = _multA;
        buf[p++] = _multB;
        // (p = 19 now; scalarSize was 16 — adjust: store 19 bytes)
        for (int i = 0; i < 4; ++i) buf[p++] = _prgBanks[i];
        for (int i = 0; i < 8; ++i) buf[p++] = _chrBanks[i];
        for (int i = 0; i < 4; ++i) buf[p++] = _chrBanksEx[i];
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
        uint need = 19 + 4 + 8 + 4 + PrgRamSize + (_chrIsRam ? _chrSize : 0);
        if (len < need) return false;
        int p = 0;
        _prgMode = buf[p++];
        _chrMode = buf[p++];
        _prgRamProtect1 = buf[p++];
        _prgRamProtect2 = buf[p++];
        _ntMirroring = buf[p++];
        _fillTile = buf[p++];
        _fillAttr = buf[p++];
        _prgRamBank = buf[p++];
        _chrUpper = buf[p++];
        _splitControl = buf[p++];
        _splitYScroll = buf[p++];
        _splitBank = buf[p++];
        _irqScanline = buf[p++];
        _irqControl = buf[p++];
        _scanlineCounter = buf[p++];
        _inVblank = buf[p++] != 0;
        _irqPending = buf[p++] != 0;
        _multA = buf[p++];
        _multB = buf[p++];
        for (int i = 0; i < 4; ++i) _prgBanks[i] = buf[p++];
        for (int i = 0; i < 8; ++i) _chrBanks[i] = buf[p++];
        for (int i = 0; i < 4; ++i) _chrBanksEx[i] = buf[p++];
        System.Array.Copy(buf, p, _prgRam, 0, (int)PrgRamSize); p += (int)PrgRamSize;
        if (_chrIsRam && _chrSize > 0)
        {
            System.Array.Copy(buf, p, _chr, 0, (int)_chrSize);
            p += (int)_chrSize;
        }
        return true;
    }
}
