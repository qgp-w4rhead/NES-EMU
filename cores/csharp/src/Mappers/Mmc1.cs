using System.Runtime.CompilerServices;

namespace NesCore;

// Mapper 1 (MMC1). Port of src/mappers/mmc1.rs.
public sealed class Mmc1 : Mapper
{
    private const uint PrgBankSize = 16384;
    private const uint Chr4kSize = 4096;
    private const uint Chr8kSize = 8192;
    private const uint PrgRamSize = 8192;
    private const byte ControlPrgMode3 = 0x0C;

    private byte[] _prgRom;
    private uint _prgSize;
    private byte[] _chr;
    private uint _chrSize;
    private bool _chrIsRam;
    private byte[] _prgRam = new byte[PrgRamSize];
    private bool _hasBattery;
    private byte _shiftReg;
    private byte _shiftCount;
    private byte _control;
    private byte _chrBank0;
    private byte _chrBank1;
    private byte _prgBank;

    public Mmc1(byte[] prg, uint prgSize, byte[] chr, uint chrSize, Mirroring mirroring, bool hasBattery)
    {
        MapperNum = 1;
        _prgSize = prgSize;
        _prgRom = new byte[prgSize];
        if (prgSize > 0) System.Array.Copy(prg, _prgRom, (int)prgSize);
        if (chrSize == 0)
        {
            _chrSize = Chr8kSize;
            _chr = new byte[Chr8kSize];
            _chrIsRam = true;
        }
        else
        {
            _chrSize = chrSize;
            _chr = new byte[chrSize];
            System.Array.Copy(chr, _chr, (int)chrSize);
            _chrIsRam = false;
        }
        _hasBattery = hasBattery;
        _shiftReg = 0;
        _shiftCount = 0;
        byte mirrBits = mirroring switch
        {
            Mirroring.SingleScreen0 => 0x00,
            Mirroring.SingleScreen1 => 0x01,
            Mirroring.SingleScreen2 => 0x01,
            Mirroring.SingleScreen3 => 0x01,
            Mirroring.Vertical => 0x02,
            _ => 0x03,
        };
        _control = (byte)(ControlPrgMode3 | mirrBits);
        _chrBank0 = 0;
        _chrBank1 = 0;
        _prgBank = 0;
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private uint PrgBankCount() => _prgSize / PrgBankSize > 0 ? _prgSize / PrgBankSize : 1;
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private uint ChrBankCount() => _chrSize / Chr4kSize > 0 ? _chrSize / Chr4kSize : 1;
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private byte PrgMode() => (byte)((_control >> 2) & 0x03);
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private byte ChrMode() => (byte)((_control >> 4) & 1);

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private byte PrgReadBank(uint bank, uint offset)
    {
        uint count = PrgBankCount();
        bank %= count;
        uint idx = bank * PrgBankSize + offset;
        if (idx >= _prgSize) return 0x00;
        return _prgRom[idx];
    }

    private void SerialWrite(ushort addr, byte value)
    {
        if ((value & 0x80) != 0)
        {
            _shiftReg = 0;
            _shiftCount = 0;
            _control = (byte)((_control & 0x13) | ControlPrgMode3);
            return;
        }
        _shiftReg = (byte)((_shiftReg >> 1) | ((value & 1) << 4));
        _shiftCount++;
        if (_shiftCount == 5)
        {
            byte reg = (byte)((addr >> 13) & 0x03);
            switch (reg)
            {
                case 0: _control = _shiftReg; break;
                case 1: _chrBank0 = _shiftReg; break;
                case 2: _chrBank1 = _shiftReg; break;
                case 3: _prgBank = _shiftReg; break;
            }
            _shiftReg = 0;
            _shiftCount = 0;
        }
    }

    public override byte ReadPrg(ushort addr)
    {
        if (addr >= 0x6000 && addr < 0x8000)
            return _prgRam[(uint)(addr - 0x6000) & (PrgRamSize - 1)];
        uint local = (uint)(addr - 0x8000);
        bool inLow = local < PrgBankSize;
        uint offset = local & (PrgBankSize - 1);
        switch (PrgMode())
        {
            case 0: case 1:
                return PrgReadBank((uint)(_prgBank & 0x0E), local);
            case 2:
                if (inLow) return PrgReadBank(0, offset);
                return PrgReadBank((uint)(_prgBank & 0x0F), offset);
            default:
                if (inLow) return PrgReadBank((uint)(_prgBank & 0x0F), offset);
                return PrgReadBank(PrgBankCount() - 1, offset);
        }
    }

    public override void WritePrg(ushort addr, byte value)
    {
        if (addr >= 0x6000 && addr < 0x8000)
        {
            _prgRam[(uint)(addr - 0x6000) & (PrgRamSize - 1)] = value;
            return;
        }
        SerialWrite(addr, value);
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private uint ChrIndex(uint a)
    {
        uint count = ChrBankCount();
        if (ChrMode() == 0)
        {
            uint bank = ((uint)(_chrBank0 & 0x1E)) % count;
            return bank * Chr4kSize + (a & (Chr8kSize - 1));
        }
        else if (a < Chr4kSize)
        {
            uint bank = ((uint)(_chrBank0 & 0x1F)) % count;
            return bank * Chr4kSize + a;
        }
        else
        {
            uint bank = ((uint)(_chrBank1 & 0x1F)) % count;
            return bank * Chr4kSize + (a - Chr4kSize);
        }
    }

    public override byte ReadChr(ushort addr)
    {
        uint idx = ChrIndex(addr);
        if (idx >= _chrSize) return 0x00;
        return _chr[idx];
    }

    public override void WriteChr(ushort addr, byte value)
    {
        if (!_chrIsRam) return;
        uint idx = ChrIndex(addr);
        if (idx < _chrSize) _chr[idx] = value;
    }

    public override Mirroring MirrorMode() => (_control & 0x03) switch
    {
        0 => Mirroring.SingleScreen0,
        1 => Mirroring.SingleScreen1,
        2 => Mirroring.Vertical,
        _ => Mirroring.Horizontal,
    };

    public override bool ChrIsRam() => _chrIsRam;
    public override bool HasBattery() => _hasBattery;

    public override uint SaveState(byte[] buf)
    {
        // Layout: shift_reg, shift_count, control, chr_bank0, chr_bank1, prg_bank (5 bytes)
        //         + prg_ram[8192] + chr (if ram)
        uint total = 5 + PrgRamSize + (_chrIsRam ? _chrSize : 0);
        if (buf == null) return total;
        int p = 0;
        buf[p++] = _shiftReg;
        buf[p++] = _shiftCount;
        buf[p++] = _control;
        buf[p++] = _chrBank0;
        buf[p++] = _chrBank1;
        buf[p++] = _prgBank;
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
        _shiftReg = buf[p++];
        _shiftCount = buf[p++];
        _control = buf[p++];
        _chrBank0 = buf[p++];
        _chrBank1 = buf[p++];
        _prgBank = buf[p++];
        System.Array.Copy(buf, p, _prgRam, 0, (int)PrgRamSize); p += (int)PrgRamSize;
        if (_chrIsRam && _chrSize > 0)
        {
            System.Array.Copy(buf, p, _chr, 0, (int)_chrSize);
            p += (int)_chrSize;
        }
        return true;
    }
}
