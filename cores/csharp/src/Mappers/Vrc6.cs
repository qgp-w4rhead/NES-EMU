using System.Runtime.CompilerServices;

namespace NesCore;

// Mappers 24 (VRC6a) and 26 (VRC6b). Port of src/mappers/vrc6.rs.
public sealed class Vrc6 : Mapper
{
    private const uint Prg16kSize = 16384;
    private const uint Prg8kSize = 8192;
    private const uint Chr1kSize = 1024;
    private const uint PrgRamSize = 8192;

    public struct PulseChan
    {
        public byte Control;
        public ushort Period;
        public ushort Timer;
        public byte Step;
        public bool Enabled;
        public byte Scale;
    }

    public struct SawChan
    {
        public byte Rate;
        public ushort Period;
        public ushort Timer;
        public byte Accum;
        public bool Enabled;
    }

    private byte[] _prgRom;
    private uint _prgSize;
    private byte[] _chr;
    private uint _chrSize;
    private bool _chrIsRam;
    private byte[] _prgRam = new byte[PrgRamSize];
    private bool _hasBattery;
    private byte _prgBank16k;
    private byte _prgBank8k;
    private byte[] _chrBanks = new byte[8];
    private bool _mirrorHorizontal;
    private byte _irqLatch;
    private byte _irqCounter;
    private bool _irqEnable;
    private bool _irqEnableAfterAck;
    private bool _irqPending;
    private PulseChan _pulse1;
    private PulseChan _pulse2;
    private SawChan _saw;
    private bool _swapAddr;

    public Vrc6(byte[] prg, uint prgSize, byte[] chr, uint chrSize, Mirroring mirroring, bool hasBattery, bool is26)
    {
        MapperNum = (ushort)(is26 ? 26 : 24);
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
        _swapAddr = is26;
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private uint Prg16kCount() => _prgSize / Prg16kSize > 0 ? _prgSize / Prg16kSize : 1;
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private uint Prg8kCount() => _prgSize / Prg8kSize > 0 ? _prgSize / Prg8kSize : 1;
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private uint Chr1kCount() => _chrSize / Chr1kSize > 0 ? _chrSize / Chr1kSize : 1;

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private ushort Decode(ushort addr)
    {
        ushort masked = (ushort)(addr & 0xF003);
        if (!_swapAddr) return masked;
        ushort a0 = (ushort)((masked & 0x001) << 1);
        ushort a1 = (ushort)((masked & 0x002) >> 1);
        return (ushort)((masked & 0xFFFC) | a0 | a1);
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private byte PrgRead8k(uint bank, uint offset)
    {
        uint count = Prg8kCount();
        bank %= count;
        uint idx = bank * Prg8kSize + offset;
        if (idx >= _prgSize) return 0x00;
        return _prgRom[idx];
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private byte PrgRead16k(uint bank, uint offset)
    {
        uint count = Prg16kCount();
        bank %= count;
        uint idx = bank * Prg16kSize + offset;
        if (idx >= _prgSize) return 0x00;
        return _prgRom[idx];
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private static byte PulseDuty(ref PulseChan p) => (byte)(((p.Control >> 4) & 0x07) + 1);
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private static byte PulseVolume(ref PulseChan p) => (byte)(p.Control & 0x0F);

    private static void PulseClock(ref PulseChan p, uint cpuCycles)
    {
        if (!p.Enabled) return;
        ushort period = p.Period;
        if ((p.Scale & 1) != 0)
        {
            period = (ushort)(period * 2);
            if (period == 0) period = 1;
        }
        if (period == 0) period = 1;
        for (uint i = 0; i < cpuCycles; ++i)
        {
            if (p.Timer == 0)
            {
                p.Timer = period;
                p.Step = (byte)((p.Step + 1) & 0x0F);
            }
            else
            {
                p.Timer--;
            }
        }
    }

    private static void SawClock(ref SawChan saw, uint cpuCycles)
    {
        if (!saw.Enabled) return;
        ushort period = saw.Period;
        if (period == 0) period = 1;
        for (uint i = 0; i < cpuCycles; ++i)
        {
            if (saw.Timer == 0)
            {
                saw.Timer = period;
                saw.Accum = (byte)(saw.Accum + (saw.Rate & 0x3F));
                if (saw.Accum >= 0x80) saw.Accum = 0;
            }
            else
            {
                saw.Timer--;
            }
        }
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private byte Pulse1Sample() => (_pulse1.Enabled && _pulse1.Step < PulseDuty(ref _pulse1)) ? PulseVolume(ref _pulse1) : (byte)0;
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private byte Pulse2Sample() => (_pulse2.Enabled && _pulse2.Step < PulseDuty(ref _pulse2)) ? PulseVolume(ref _pulse2) : (byte)0;
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private byte SawSample() => _saw.Enabled ? (byte)(_saw.Accum >> 2) : (byte)0;

    public override byte ReadPrg(ushort addr)
    {
        if (addr >= 0x6000 && addr < 0x8000)
            return _prgRam[(uint)(addr - 0x6000) & (PrgRamSize - 1)];
        uint local = (uint)(addr - 0x8000);
        if (local < Prg16kSize)
        {
            uint bank = _prgBank16k % Prg16kCount();
            return PrgRead16k(bank, local);
        }
        if (local < Prg16kSize + Prg8kSize)
        {
            uint off = local - Prg16kSize;
            uint bank = _prgBank8k % Prg8kCount();
            return PrgRead8k(bank, off);
        }
        uint off2 = local - Prg16kSize - Prg8kSize;
        uint last = Prg8kCount() - 1;
        return PrgRead8k(last, off2);
    }

    public override void WritePrg(ushort addr, byte value)
    {
        if (addr >= 0x6000 && addr < 0x8000)
        {
            _prgRam[(uint)(addr - 0x6000) & (PrgRamSize - 1)] = value;
            return;
        }
        ushort reg = Decode(addr);
        switch (reg)
        {
            case 0x8000: _prgBank16k = (byte)(value & 0x3F); break;
            case 0x9000: _pulse1.Control = value; break;
            case 0x9001: _pulse1.Period = (ushort)((_pulse1.Period & 0x0F00) | value); break;
            case 0x9002:
                _pulse1.Period = (ushort)((_pulse1.Period & 0x00FF) | ((value & 0x0F) << 8));
                _pulse1.Enabled = (value & 0x80) != 0;
                break;
            case 0x9003: _pulse1.Scale = value; break;
            case 0xA000: _pulse2.Control = value; break;
            case 0xA001: _pulse2.Period = (ushort)((_pulse2.Period & 0x0F00) | value); break;
            case 0xA002:
                _pulse2.Period = (ushort)((_pulse2.Period & 0x00FF) | ((value & 0x0F) << 8));
                _pulse2.Enabled = (value & 0x80) != 0;
                break;
            case 0xB000: _saw.Rate = value; break;
            case 0xB001: _saw.Period = (ushort)((_saw.Period & 0x0F00) | value); break;
            case 0xB002:
                _saw.Period = (ushort)((_saw.Period & 0x00FF) | ((value & 0x0F) << 8));
                _saw.Enabled = (value & 0x80) != 0;
                break;
            case 0xB003: _mirrorHorizontal = (value & 0x01) != 0; break;
            case 0xC000: _prgBank8k = (byte)(value & 0x3F); break;
            case 0xD000: _chrBanks[0] = value; break;
            case 0xD001: _chrBanks[1] = value; break;
            case 0xD002: _chrBanks[2] = value; break;
            case 0xD003: _chrBanks[3] = value; break;
            case 0xE000: _chrBanks[4] = value; break;
            case 0xE001: _chrBanks[5] = value; break;
            case 0xE002: _chrBanks[6] = value; break;
            case 0xE003: _chrBanks[7] = value; break;
            case 0xF000: _irqLatch = value; break;
            case 0xF001:
                _irqEnable = (value & 0x01) != 0;
                _irqCounter = _irqLatch;
                break;
            case 0xF002:
                _irqEnableAfterAck = (value & 0x02) != 0;
                _irqEnable = (value & 0x01) != 0;
                _irqPending = false;
                break;
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
                if (_irqEnable)
                {
                    _irqPending = true;
                    if (!_irqEnableAfterAck) _irqEnable = false;
                }
            }
            else
            {
                _irqCounter--;
            }
        }
        PulseClock(ref _pulse1, cpuCycles);
        PulseClock(ref _pulse2, cpuCycles);
        SawClock(ref _saw, cpuCycles);
    }

    public override float ExpansionAudioSample()
    {
        uint p1 = Pulse1Sample();
        uint p2 = Pulse2Sample();
        uint saw = SawSample();
        uint sum = p1 + p2;
        sum = (sum > 63) ? 63 : (sum + saw);
        if (sum > 63) sum = 63;
        const float Vrc6Gain = 0.75f;
        float normalized = (sum / 63.0f) * 2.0f - 1.0f;
        return normalized * Vrc6Gain;
    }
}
