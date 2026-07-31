using System.Runtime.CompilerServices;

namespace NesCore;

// Mapper 19 (Namco 163 with wavetable expansion audio).
// Port of src/mappers/namco163.rs.
public sealed class Namco163 : Mapper
{
    private const uint Prg8kSize = 8192;
    private const uint Chr1kSize = 1024;
    private const uint PrgRamSize = 8192;
    private const uint WaveRamSize = 0x80;
    private const uint WaveChannels = 8;

    public struct WaveChan
    {
        public ushort Freq;
        public byte Length;
        public byte Volume;
        public byte Offset;
        public uint Phase;
        public bool Enabled;
    }

    private byte[] _prgRom;
    private uint _prgSize;
    private byte[] _chr;
    private uint _chrSize;
    private bool _chrIsRam;
    private byte[] _prgRam = new byte[PrgRamSize];
    private bool _hasBattery;
    private byte[] _prgBanks = new byte[4];
    private byte[] _chrBanks = new byte[8];
    private Mirroring _mirror;
    private bool _prgRamEnable;
    private bool _prgRamWriteProtect;
    private ushort _irqLatch;
    private ushort _irqCounter;
    private bool _irqEnable;
    private bool _irqPending;
    private bool _irqLatchHigh;
    private byte[] _waveRam = new byte[WaveRamSize];
    private byte _waveAddr;
    private WaveChan[] _chans = new WaveChan[WaveChannels];

    public Namco163(byte[] prg, uint prgSize, byte[] chr, uint chrSize, Mirroring mirroring, bool hasBattery)
    {
        MapperNum = 19;
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
        _mirror = mirroring;
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

    private byte WaveRead() => _waveRam[_waveAddr & 0x7F];

    private void WaveWrite(byte value)
    {
        byte a = (byte)(_waveAddr & 0x7F);
        _waveRam[a] = value;
        if ((_waveAddr & 0x80) != 0)
            _waveAddr = (byte)((_waveAddr & 0x80) | ((_waveAddr + 1) & 0x7F));
    }

    private byte ChanParamRead(ushort addr)
    {
        uint off = (uint)(addr - 0x5000);
        uint ch = off / 8;
        uint sub = off & 0x07;
        if (ch >= WaveChannels) return 0;
        ref WaveChan c = ref _chans[ch];
        return sub switch
        {
            0 => (byte)(c.Freq & 0xFF),
            1 => (byte)(((c.Freq >> 8) & 0x0F) | ((c.Length & 0x0F) << 4)),
            2 => (byte)(c.Volume & 0x0F),
            3 => (byte)((c.Offset & 0x60) | (c.Enabled ? 0x80 : 0x00)),
            _ => 0,
        };
    }

    private void ChanParamWrite(ushort addr, byte value)
    {
        uint off = (uint)(addr - 0x5000);
        uint ch = off / 8;
        uint sub = off & 0x07;
        if (ch >= WaveChannels) return;
        ref WaveChan c = ref _chans[ch];
        switch (sub)
        {
            case 0: c.Freq = (ushort)((c.Freq & 0x0F00) | value); break;
            case 1:
                c.Freq = (ushort)((c.Freq & 0x00FF) | ((value & 0x0F) << 8));
                c.Length = (byte)((value >> 4) & 0x0F);
                break;
            case 2: c.Volume = (byte)(value & 0x0F); break;
            case 3:
                c.Offset = (byte)(value & 0x60);
                c.Enabled = (value & 0x80) != 0;
                break;
        }
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private byte ChanSample(byte ch)
    {
        ref WaveChan c = ref _chans[ch];
        if (!c.Enabled) return 0;
        uint lengthNibbles = ((uint)c.Length + 1) * 8;
        if (lengthNibbles == 0) lengthNibbles = 1;
        uint phaseNibble = (c.Phase >> 16) % lengthNibbles;
        uint byteIdx = ((uint)c.Offset + phaseNibble / 2) & 0x7F;
        byte b = _waveRam[byteIdx];
        byte nibble = (phaseNibble & 1) != 0 ? (byte)(b >> 4) : (byte)(b & 0x0F);
        return (byte)((nibble * c.Volume) / 15);
    }

    public override byte ReadPrg(ushort addr)
    {
        if (addr >= 0x6000 && addr < 0x8000)
        {
            if (_prgRamEnable) return _prgRam[(uint)(addr - 0x6000) & (PrgRamSize - 1)];
            return 0x00;
        }
        if (addr >= 0x4800 && addr < 0x5000) return WaveRead();
        if (addr >= 0x5000 && addr < 0x5800) return ChanParamRead(addr);
        uint local = (uint)(addr - 0x8000);
        uint slot = local / Prg8kSize;
        uint offset = local & (Prg8kSize - 1);
        uint bank = _prgBanks[slot] % Prg8kCount();
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
        if (addr >= 0x4800 && addr < 0x5000) { WaveWrite(value); return; }
        if (addr >= 0x5000 && addr < 0x5800) { ChanParamWrite(addr, value); return; }
        ushort reg = (ushort)(addr & 0xF801);
        switch (reg)
        {
            case 0x8000: _chrBanks[0] = value; break;
            case 0x8800: _chrBanks[1] = value; break;
            case 0x9000: _chrBanks[2] = value; break;
            case 0x9800: _chrBanks[3] = value; break;
            case 0xA000: _chrBanks[4] = value; break;
            case 0xA800: _chrBanks[5] = value; break;
            case 0xB000: _chrBanks[6] = value; break;
            case 0xB800: _chrBanks[7] = value; break;
            case 0xC000: _prgBanks[0] = (byte)(value & 0x3F); break;
            case 0xC800: _prgBanks[1] = (byte)(value & 0x3F); break;
            case 0xD000: _prgBanks[2] = (byte)(value & 0x3F); break;
            case 0xD800: _prgBanks[3] = (byte)(value & 0x3F); break;
            case 0xE000:
                _mirror = (value & 0x03) switch
                {
                    0 => Mirroring.Vertical,
                    1 => Mirroring.Horizontal,
                    2 => Mirroring.SingleScreen0,
                    _ => Mirroring.SingleScreen1,
                };
                break;
            case 0xE800:
                _prgRamEnable = (value & 0x80) != 0;
                _prgRamWriteProtect = (value & 0x40) != 0;
                break;
            case 0xF000:
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
            case 0xF800: _waveAddr = value; break;
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

    public override Mirroring MirrorMode() => _mirror;
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
        uint active = 0;
        for (byte ch = 0; ch < WaveChannels; ++ch)
            if (_chans[ch].Enabled) ++active;
        if (active == 0) active = 1;
        for (uint i = 0; i < cpuCycles; ++i)
        {
            for (byte ch = 0; ch < WaveChannels; ++ch)
            {
                ref WaveChan c = ref _chans[ch];
                if (!c.Enabled) continue;
                uint inc = (uint)c.Freq << 8;
                c.Phase = c.Phase + (inc / active);
            }
        }
    }

    public override float ExpansionAudioSample()
    {
        int sum = 0;
        for (byte ch = 0; ch < WaveChannels; ++ch)
            sum += ChanSample(ch);
        const float N163Gain = 0.4f;
        float v = sum / 120.0f;
        if (v < -1.0f) v = -1.0f;
        if (v > 1.0f) v = 1.0f;
        return v * N163Gain;
    }
}
