using System.Runtime.CompilerServices;

namespace NesCore;

// Mapper 85 (VRC7 with YM2413 OPLL audio). Port of src/mappers/vrc7.rs.
public sealed class Vrc7 : Mapper
{
    private const uint Prg8kSize = 8192;
    private const uint Chr1kSize = 1024;
    private const uint PrgRamSize = 8192;

    private byte[] _prgRom;
    private uint _prgSize;
    private byte[] _chr;
    private uint _chrSize;
    private bool _chrIsRam;
    private byte[] _prgRam = new byte[PrgRamSize];
    private bool _hasBattery;
    private byte[] _prgBanks = new byte[3];
    private byte[] _chrBanks = new byte[8];
    private bool _mirrorHorizontal;
    private ushort _irqLatch;
    private ushort _irqCounter;
    private bool _irqLatchHigh;
    private bool _irqEnable;
    private bool _irqPending;
    private Opll _opll = new Opll();

    public Vrc7(byte[] prg, uint prgSize, byte[] chr, uint chrSize, Mirroring mirroring, bool hasBattery)
    {
        MapperNum = 85;
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
    private uint Prg8kCount() => _prgSize / Prg8kSize > 0 ? _prgSize / Prg8kSize : 1;
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private uint Chr1kCount() => _chrSize / Chr1kSize > 0 ? _chrSize / Chr1kSize : 1;

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private byte PrgReadBank(uint bank, uint offset)
    {
        uint count = Prg8kCount();
        bank %= count;
        uint idx = bank * Prg8kSize + offset;
        if (idx >= _prgSize) return 0x00;
        return _prgRom[idx];
    }

    public override byte ReadPrg(ushort addr)
    {
        if (addr >= 0x6000 && addr < 0x8000)
            return _prgRam[(uint)(addr - 0x6000) & (PrgRamSize - 1)];
        uint local = (uint)(addr - 0x8000);
        uint slot = local / Prg8kSize;
        uint offset = local & (Prg8kSize - 1);
        if (slot < 3)
        {
            uint bank = _prgBanks[slot] % Prg8kCount();
            return PrgReadBank(bank, offset);
        }
        uint last = Prg8kCount() - 1;
        return PrgReadBank(last, offset);
    }

    public override void WritePrg(ushort addr, byte value)
    {
        if (addr >= 0x6000 && addr < 0x8000)
        {
            _prgRam[(uint)(addr - 0x6000) & (PrgRamSize - 1)] = value;
            return;
        }
        ushort reg = (ushort)(addr & 0xF03D);
        switch (reg)
        {
            case 0x8000: _prgBanks[0] = (byte)(value & 0x3F); break;
            case 0x8008: _prgBanks[1] = (byte)(value & 0x3F); break;
            case 0x9000: _prgBanks[2] = (byte)(value & 0x3F); break;
            case 0x9010: _opll.WriteAddr(value); break;
            case 0x9030: _opll.WriteData(value); break;
            case 0xB000: _mirrorHorizontal = (value & 0x01) != 0; break;
            case 0xC000: _chrBanks[0] = value; break;
            case 0xC004: _chrBanks[1] = value; break;
            case 0xC008: _chrBanks[2] = value; break;
            case 0xC00C: _chrBanks[3] = value; break;
            case 0xD000: _chrBanks[4] = value; break;
            case 0xD004: _chrBanks[5] = value; break;
            case 0xD008: _chrBanks[6] = value; break;
            case 0xD00C: _chrBanks[7] = value; break;
            case 0xE000:
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
            case 0xE008:
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
            case 0xE010: _irqPending = false; break;
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
        _opll.Clock(cpuCycles / 2);
    }

    public override float ExpansionAudioSample()
    {
        const float Vrc7Gain = 0.6f;
        return _opll.Sample() * Vrc7Gain;
    }
}
