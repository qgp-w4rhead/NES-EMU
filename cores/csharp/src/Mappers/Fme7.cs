using System.Runtime.CompilerServices;

namespace NesCore;

// Mapper 69 (FME-7 / Sunsoft 5B). Port of src/mappers/fme7.rs.
public sealed class Fme7 : Mapper
{
    private const uint PrgBankSize = 8192;
    private const uint Chr1kSize = 1024;
    private const uint PrgRamSize = 8192;

    private byte[] _prgRom;
    private uint _prgSize;
    private byte[] _chr;
    private uint _chrSize;
    private bool _chrIsRam;
    private byte[] _prgRam = new byte[PrgRamSize];
    private bool _hasBattery;
    private byte _command;
    private byte[] _prgBanks = new byte[4];
    private byte[] _chrBanks = new byte[8];
    private bool _mirrorHorizontal;
    private bool _prgRamEnable;
    private bool _prgRamWriteProtect;
    private ushort _irqLatch;
    private ushort _irqCounter;
    private bool _irqLatchHigh;
    private bool _irqEnable;
    private bool _irqPending;
    private Ym2149 _ym = new Ym2149();

    public Fme7(byte[] prg, uint prgSize, byte[] chr, uint chrSize, Mirroring mirroring, bool hasBattery)
    {
        MapperNum = 69;
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
        _mirrorHorizontal = (mirroring == Mirroring.Horizontal);
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

    private void WriteData(byte value)
    {
        byte cmd = (byte)(_command & 0x0F);
        if (cmd <= 7)
        {
            _chrBanks[cmd & 0x07] = value;
        }
        else
        {
            switch (cmd)
            {
                case 8: _prgBanks[0] = (byte)(value & 0x3F); break;
                case 9: _prgBanks[1] = (byte)(value & 0x3F); break;
                case 10: _prgBanks[2] = (byte)(value & 0x3F); break;
                case 11: _prgBanks[3] = (byte)(value & 0x3F); break;
                case 12: _mirrorHorizontal = (value & 0x01) != 0; break;
                case 13:
                    _prgRamEnable = (value & 0x80) != 0;
                    _prgRamWriteProtect = (value & 0x40) != 0;
                    break;
                case 14:
                    if (!_irqLatchHigh)
                    {
                        _irqLatch = (ushort)((_irqLatch & 0xFF00) | value);
                        _irqLatchHigh = true;
                    }
                    else
                    {
                        _irqLatch = (ushort)((_irqLatch & 0x00FF) | ((ushort)value << 8));
                        _irqLatchHigh = false;
                    }
                    break;
                case 15:
                    if ((value & 0x02) != 0)
                    {
                        _irqEnable = true;
                        _irqPending = false;
                        _irqCounter = _irqLatch;
                    }
                    else if ((value & 0x01) != 0)
                    {
                        _irqEnable = true;
                        _irqCounter = _irqLatch;
                    }
                    else
                    {
                        _irqEnable = false;
                    }
                    break;
            }
        }
    }

    public override byte ReadPrg(ushort addr)
    {
        if (addr >= 0x6000 && addr < 0x8000)
        {
            if (_prgRamEnable) return _prgRam[(uint)(addr - 0x6000) & (PrgRamSize - 1)];
            return 0x00;
        }
        uint local = (uint)(addr - 0x8000);
        uint slot = local / PrgBankSize;
        uint offset = local & (PrgBankSize - 1);
        uint bank = _prgBanks[slot] % PrgBankCount();
        return PrgReadBank(bank, offset);
    }

    public override void WritePrg(ushort addr, byte value)
    {
        if (addr >= 0x6000 && addr < 0x8000)
        {
            if (_prgRamEnable && !_prgRamWriteProtect)
                _prgRam[(uint)(addr - 0x6000) & (PrgRamSize - 1)] = value;
            return;
        }
        switch (addr)
        {
            case 0x8000: _command = value; break;
            case 0x8001: WriteData(value); break;
            case 0xC000: _ym.WriteAddr(value); break;
            case 0xE000: _ym.WriteData(value); break;
        }
    }

    public override byte ReadChr(ushort addr)
    {
        uint slot = addr / Chr1kSize;
        uint offset = addr & (Chr1kSize - 1);
        uint count = Chr1kCount();
        uint bank = _chrBanks[slot] % count;
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
        uint bank = _chrBanks[slot] % count;
        uint idx = bank * Chr1kSize + offset;
        if (idx < _chrSize) _chr[idx] = value;
    }

    public override Mirroring MirrorMode() => _mirrorHorizontal ? Mirroring.Horizontal : Mirroring.Vertical;
    public override bool ChrIsRam() => _chrIsRam;
    public override bool HasBattery() => _hasBattery;
    public override bool IrqPending() => _irqPending;

    public override void ClockCpu(uint cpuCycles)
    {
        for (uint i = 0; i < cpuCycles; ++i)
        {
            if (_irqCounter == 0)
            {
                _irqCounter = _irqLatch;
                if (_irqEnable) _irqPending = true;
            }
            else
            {
                _irqCounter--;
            }
        }
        _ym.Clock(cpuCycles / 2);
    }

    public override float ExpansionAudioSample()
    {
        const float S5bGain = 0.5f;
        return _ym.Sample() * S5bGain;
    }
}
