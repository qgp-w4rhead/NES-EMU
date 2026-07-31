using System.Runtime.CompilerServices;

namespace NesCore;

// Audio Processing Unit: pulse, triangle, noise, and DMC channels.
// Port of src/apu.rs.

// DMC DMA read callback: fetches a byte from CPU memory for the DMC.
public delegate byte DmcReadFn(ushort addr);

public sealed class PulseChannel
{
    public const ushort MaxPeriod = 0x7FF;
    public const ushort MinAudiblePeriod = 8;

    public static readonly byte[,] DutyPatterns = new byte[4, 8] {
        { 0, 1, 0, 0, 0, 0, 0, 0 },
        { 0, 1, 1, 0, 0, 0, 0, 0 },
        { 0, 1, 1, 1, 1, 0, 0, 0 },
        { 1, 0, 0, 0, 1, 1, 1, 1 },
    };

    public static readonly byte[] LengthTable = new byte[32] {
        10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14,
        12, 16, 24, 18, 48, 20, 96, 22, 192, 24, 72, 26, 16, 28, 32, 30,
    };

    public bool Pulse2;
    public byte Duty;
    public bool Halt;
    public bool ConstantVolume;
    public byte Volume;
    public bool SweepEnabled;
    public byte SweepPeriod;
    public bool SweepNegate;
    public byte SweepShift;
    public byte SweepDivider;
    public bool SweepReload;
    public ushort TimerPeriod;
    public ushort Timer;
    public byte Sequence;
    public byte LengthCounter;
    public bool Enabled;
    public byte EnvelopeDivider;
    public byte EnvelopeDecay;
    public bool EnvelopeStart;

    public PulseChannel(bool pulse2)
    {
        Pulse2 = pulse2;
    }

    public void WriteRegister(byte reg, byte value)
    {
        switch (reg)
        {
            case 0:
                Duty = (byte)((value >> 6) & 0x03);
                Halt = (value & 0x20) != 0;
                ConstantVolume = (value & 0x10) != 0;
                Volume = (byte)(value & 0x0F);
                break;
            case 1:
                SweepEnabled = (value & 0x80) != 0;
                SweepPeriod = (byte)((value >> 4) & 0x07);
                SweepNegate = (value & 0x08) != 0;
                SweepShift = (byte)(value & 0x07);
                SweepReload = true;
                break;
            case 2:
                TimerPeriod = (ushort)((TimerPeriod & 0xFF00) | value);
                break;
            case 3:
            {
                ushort high = (ushort)(value & 0x07);
                TimerPeriod = (ushort)((TimerPeriod & 0x00FF) | (ushort)(high << 8));
                Timer = (ushort)((Timer & 0x00FF) | (ushort)(high << 8));
                if (Enabled)
                {
                    byte idx = (byte)(value >> 3);
                    LengthCounter = LengthTable[idx];
                }
                EnvelopeStart = true;
                Sequence = 0;
                break;
            }
        }
    }

    public void SetEnabled(bool enabled)
    {
        Enabled = enabled;
        if (!enabled) LengthCounter = 0;
    }

    public ushort SweepTarget()
    {
        if (!SweepEnabled || SweepShift == 0) return TimerPeriod;
        ushort shifted = (ushort)(TimerPeriod >> SweepShift);
        if (SweepNegate)
        {
            if (Pulse2) return (ushort)(TimerPeriod - shifted - 1);
            return (ushort)(TimerPeriod - shifted);
        }
        return (ushort)(TimerPeriod + shifted);
    }

    public bool IsMuted() => TimerPeriod < MinAudiblePeriod || SweepTarget() > MaxPeriod;

    public void Tick()
    {
        if (Timer == 0)
        {
            Timer = TimerPeriod;
            Sequence = (byte)((Sequence + 1) & 0x07);
        }
        else Timer--;
    }

    public void ClockEnvelope()
    {
        if (EnvelopeStart)
        {
            EnvelopeStart = false;
            EnvelopeDecay = 15;
            EnvelopeDivider = Volume;
        }
        else if (EnvelopeDivider == 0)
        {
            EnvelopeDivider = Volume;
            if (EnvelopeDecay > 0) EnvelopeDecay--;
            else if (Halt) EnvelopeDecay = 15;
        }
        else EnvelopeDivider--;
    }

    private void ClockLength()
    {
        if (!Halt && LengthCounter > 0) LengthCounter--;
    }

    private void ClockSweep()
    {
        bool dividerZero = SweepDivider == 0;
        if (dividerZero && SweepEnabled && SweepShift != 0)
        {
            ushort target = SweepTarget();
            if (target <= MaxPeriod && TimerPeriod >= MinAudiblePeriod)
                TimerPeriod = target;
        }
        if (dividerZero || SweepReload)
        {
            SweepDivider = SweepPeriod;
            SweepReload = false;
        }
        else SweepDivider--;
    }

    public void ClockQuarterFrame() => ClockEnvelope();
    public void ClockHalfFrame() { ClockLength(); ClockSweep(); }

    public byte Sample()
    {
        if (!Enabled || LengthCounter == 0 || IsMuted()) return 0;
        byte dutyBit = DutyPatterns[Duty, Sequence];
        if (dutyBit == 0) return 0;
        return ConstantVolume ? Volume : EnvelopeDecay;
    }
}

public sealed class TriangleChannel
{
    public static readonly byte[] TriangleSequence = new byte[32] {
        15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0,
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
    };

    public bool Halt;
    public byte LinearReload;
    public byte LinearCounter;
    public bool LinearStart;
    public ushort TimerPeriod;
    public ushort Timer;
    public byte Sequence;
    public byte LengthCounter;
    public bool Enabled;

    public void WriteRegister(byte reg, byte value)
    {
        switch (reg)
        {
            case 0:
                Halt = (value & 0x80) != 0;
                LinearReload = (byte)(value & 0x7F);
                break;
            case 1: break;
            case 2:
                TimerPeriod = (ushort)((TimerPeriod & 0xFF00) | value);
                break;
            case 3:
            {
                ushort high = (ushort)(value & 0x07);
                TimerPeriod = (ushort)((TimerPeriod & 0x00FF) | (ushort)(high << 8));
                Timer = (ushort)((Timer & 0x00FF) | (ushort)(high << 8));
                if (Enabled)
                {
                    byte idx = (byte)(value >> 3);
                    LengthCounter = PulseChannel.LengthTable[idx];
                }
                LinearStart = true;
                Sequence = 0;
                break;
            }
        }
    }

    public void SetEnabled(bool enabled)
    {
        Enabled = enabled;
        if (!enabled) LengthCounter = 0;
    }

    public void Tick()
    {
        if (Timer == 0)
        {
            Timer = TimerPeriod;
            Sequence = (byte)((Sequence + 1) & 0x1F);
        }
        else Timer--;
    }

    private void ClockLinear()
    {
        if (LinearStart)
        {
            LinearStart = false;
            LinearCounter = LinearReload;
        }
        else if (LinearCounter > 0) LinearCounter--;
        if (Halt) LinearStart = true;
    }

    private void ClockLength()
    {
        if (!Halt && LengthCounter > 0) LengthCounter--;
    }

    public void ClockQuarterFrame() => ClockLinear();
    public void ClockHalfFrame() => ClockLength();

    public byte Sample()
    {
        if (!Enabled || LengthCounter == 0 || LinearCounter == 0) return 0;
        return TriangleSequence[Sequence];
    }
}

public sealed class NoiseChannel
{
    public static readonly ushort[] NoisePeriodTable = new ushort[16] {
        4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
    };

    public bool Halt;
    public bool ConstantVolume;
    public byte Volume;
    public bool Mode;
    public byte PeriodIndex;
    public ushort TimerPeriod;
    public ushort Timer;
    public ushort Lfsr;
    public byte LengthCounter;
    public bool Enabled;
    public byte EnvelopeDivider;
    public byte EnvelopeDecay;
    public bool EnvelopeStart;

    public NoiseChannel()
    {
        TimerPeriod = NoisePeriodTable[0];
        Lfsr = 1;
    }

    public void WriteRegister(byte reg, byte value)
    {
        switch (reg)
        {
            case 0:
                Halt = (value & 0x20) != 0;
                ConstantVolume = (value & 0x10) != 0;
                Volume = (byte)(value & 0x0F);
                break;
            case 1: break;
            case 2:
                Mode = (value & 0x80) != 0;
                PeriodIndex = (byte)(value & 0x0F);
                TimerPeriod = NoisePeriodTable[PeriodIndex];
                break;
            case 3:
                if (Enabled)
                {
                    byte idx = (byte)(value >> 3);
                    LengthCounter = PulseChannel.LengthTable[idx];
                }
                EnvelopeStart = true;
                break;
        }
    }

    public void SetEnabled(bool enabled)
    {
        Enabled = enabled;
        if (!enabled) LengthCounter = 0;
    }

    public void Tick()
    {
        if (Timer == 0)
        {
            Timer = TimerPeriod;
            ushort bit0 = (ushort)(Lfsr & 0x0001);
            ushort tap = Mode ? (ushort)6 : (ushort)1;
            ushort tapBit = (ushort)((Lfsr >> tap) & 0x0001);
            ushort feedback = (ushort)(bit0 ^ tapBit);
            Lfsr = (ushort)(Lfsr >> 1);
            if (feedback != 0) Lfsr |= 0x4000;
        }
        else Timer--;
    }

    public void ClockEnvelope()
    {
        if (EnvelopeStart)
        {
            EnvelopeStart = false;
            EnvelopeDecay = 15;
            EnvelopeDivider = Volume;
        }
        else if (EnvelopeDivider == 0)
        {
            EnvelopeDivider = Volume;
            if (EnvelopeDecay > 0) EnvelopeDecay--;
            else if (Halt) EnvelopeDecay = 15;
        }
        else EnvelopeDivider--;
    }

    private void ClockLength()
    {
        if (!Halt && LengthCounter > 0) LengthCounter--;
    }

    public void ClockQuarterFrame() => ClockEnvelope();
    public void ClockHalfFrame() => ClockLength();

    public byte Sample()
    {
        if (!Enabled || LengthCounter == 0) return 0;
        if ((Lfsr & 1) != 0) return 0;
        return ConstantVolume ? Volume : EnvelopeDecay;
    }
}

public sealed class DmcChannel
{
    public static readonly ushort[] DmcRateTable = new ushort[16] {
        214, 190, 170, 160, 149, 138, 127, 113, 107, 95, 85, 80, 71, 63, 54, 42,
    };

    public bool IrqEnable;
    public bool LoopFlag;
    public byte RateIndex;
    public ushort TimerPeriod;
    public ushort Timer;
    public byte OutputCounter;
    public byte SampleBuffer;
    public byte BufferBits;
    public ushort SampleAddrBase;
    public ushort SampleAddress;
    public ushort SampleLength;
    public ushort BytesRemaining;
    public bool Enabled;
    public bool IrqFlag;

    public DmcChannel()
    {
        TimerPeriod = DmcRateTable[0];
        SampleAddrBase = 0xC000;
        SampleAddress = 0xC000;
        SampleLength = 1;
    }

    public void WriteRegister(byte reg, byte value)
    {
        switch (reg)
        {
            case 0:
                IrqEnable = (value & 0x80) != 0;
                LoopFlag = (value & 0x40) != 0;
                RateIndex = (byte)(value & 0x0F);
                TimerPeriod = DmcRateTable[RateIndex];
                break;
            case 1:
                OutputCounter = (byte)(value & 0x7F);
                break;
            case 2:
                SampleAddrBase = (ushort)(((ushort)value << 6) | 0xC000);
                break;
            case 3:
                SampleLength = (ushort)(((ushort)value << 4) | 1);
                break;
        }
    }

    public void SetEnabled(bool enabled)
    {
        Enabled = enabled;
        if (enabled)
        {
            if (BytesRemaining == 0)
            {
                SampleAddress = SampleAddrBase;
                BytesRemaining = SampleLength;
                BufferBits = 0;
            }
        }
        else BytesRemaining = 0;
    }

    public void ClearIrq() => IrqFlag = false;

    private void ClockOutputUnit(DmcReadFn read)
    {
        if (BufferBits == 0 && BytesRemaining > 0)
        {
            SampleBuffer = read(SampleAddress);
            BufferBits = 8;
            SampleAddress++;
            if (SampleAddress == 0) SampleAddress = 0x8000;
            BytesRemaining--;
            if (BytesRemaining == 0)
            {
                if (LoopFlag)
                {
                    SampleAddress = SampleAddrBase;
                    BytesRemaining = SampleLength;
                }
                else if (IrqEnable) IrqFlag = true;
            }
        }

        if (BufferBits > 0)
        {
            byte bit = (byte)(SampleBuffer & 1);
            if (bit == 0)
                OutputCounter = (OutputCounter >= 2) ? (byte)(OutputCounter - 2) : (byte)0;
            else
            {
                ushort v = (ushort)(OutputCounter + 2);
                if (v > 127) v = 127;
                OutputCounter = (byte)v;
            }
            SampleBuffer >>= 1;
            BufferBits--;
        }
    }

    public void Tick(DmcReadFn read)
    {
        if (Timer == 0)
        {
            Timer = TimerPeriod;
            ClockOutputUnit(read);
        }
        else Timer--;
    }

    public byte Sample() => OutputCounter;
}
