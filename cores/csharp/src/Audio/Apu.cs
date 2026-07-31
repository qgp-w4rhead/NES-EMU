using System.Runtime.CompilerServices;

namespace NesCore;

// Audio Processing Unit (src/apu.rs Apu).
public sealed class Apu
{
    public const int ChannelCount = 5;
    public const int SampleRate = 44100;

    public PulseChannel Pulse1 = new PulseChannel(false);
    public PulseChannel Pulse2 = new PulseChannel(true);
    public TriangleChannel Triangle = new TriangleChannel();
    public NoiseChannel Noise = new NoiseChannel();
    public DmcChannel Dmc = new DmcChannel();

    public uint CycleAccumulator;

    public float[] ChannelVolumes = new float[ChannelCount];
    public bool[] ChannelMuted = new bool[ChannelCount];
    public byte SelectedChannel;

    public float LpfPrev;
    public float DcPrevX;
    public float DcPrevY;
    public float MixAccumulator;
    public uint MixCount;
    public float SampleAccumulator;
    public float LastDecimated;

    public bool FrameMode5Step;
    public bool FrameIrqInhibit;
    public uint FrameCycle;
    public bool FrameIrq;
    public uint FrameResetDelay;

    public Region Region = Region.Ntsc;

    // Pre-allocated threshold array (avoids per-step allocation in hot path).
    private ApuFrameThreshold[] _thresholds = new ApuFrameThreshold[4];

    public Apu()
    {
        for (int i = 0; i < ChannelCount; ++i)
        {
            ChannelVolumes[i] = 1.0f;
            ChannelMuted[i] = false;
        }
        LpfPrev = -1.0f;
        DcPrevX = -1.0f;
        DcPrevY = 0.0f;
        LastDecimated = -1.0f;
    }

    public void WriteStatus(byte value)
    {
        Pulse1.SetEnabled((value & 0x01) != 0);
        Pulse2.SetEnabled((value & 0x02) != 0);
        Triangle.SetEnabled((value & 0x04) != 0);
        Noise.SetEnabled((value & 0x08) != 0);
        Dmc.SetEnabled((value & 0x10) != 0);
    }

    public byte ReadStatus()
    {
        byte v = 0;
        if (Pulse1.LengthCounter > 0) v |= 0x01;
        if (Pulse2.LengthCounter > 0) v |= 0x02;
        if (Triangle.LengthCounter > 0) v |= 0x04;
        if (Noise.LengthCounter > 0) v |= 0x08;
        if (Dmc.BytesRemaining > 0) v |= 0x10;
        if (FrameIrq) v |= 0x40;
        if (Dmc.IrqFlag) v |= 0x80;
        FrameIrq = false;
        Dmc.IrqFlag = false;
        return v;
    }

    public void WriteFrameCounter(byte value)
    {
        bool newMode5Step = (value & 0x80) != 0;
        FrameIrqInhibit = (value & 0x40) != 0;
        if (FrameIrqInhibit) FrameIrq = false;
        if (newMode5Step)
        {
            ClockQuarterFrame();
            ClockHalfFrame();
        }
        FrameResetDelay = 4;
        FrameMode5Step = newMode5Step;
    }

    public bool IrqPending() => FrameIrq || Dmc.IrqFlag;

    public void SetRegion(Region region) => Region = region;

    private void StepFrameCounter(uint cpuCycles)
    {
        if (FrameResetDelay > 0)
        {
            uint advance = (cpuCycles < FrameResetDelay) ? cpuCycles : FrameResetDelay;
            FrameResetDelay -= advance;
            if (FrameResetDelay == 0) FrameCycle = 0;
            uint remaining = cpuCycles - advance;
            if (remaining == 0) return;
            FrameCycle += remaining;
        }
        else
        {
            FrameCycle += cpuCycles;
        }

        uint prev = (FrameCycle >= cpuCycles) ? (FrameCycle - cpuCycles) : 0u;
        if (FrameMode5Step) RegionTiming.Apu5StepThresholds(Region, _thresholds);
        else RegionTiming.Apu4StepThresholds(Region, _thresholds);
        uint irqThreshold = RegionTiming.Apu4StepIrqThreshold(Region);

        for (int i = 0; i < 4; ++i)
        {
            uint threshold = _thresholds[i].Threshold;
            bool quarter = _thresholds[i].Quarter;
            bool half = _thresholds[i].Half;
            if (prev < threshold && FrameCycle >= threshold)
            {
                if (quarter) ClockQuarterFrame();
                if (half) ClockHalfFrame();
                if (!FrameMode5Step && threshold == irqThreshold && !FrameIrqInhibit)
                    FrameIrq = true;
            }
        }

        uint resetAt = RegionTiming.ApuResetAt(Region, FrameMode5Step);
        if (FrameCycle >= resetAt) FrameCycle -= resetAt;
    }

    public void Step(uint cpuCycles, DmcReadFn read)
    {
        StepFrameCounter(cpuCycles);

        float apuCyclesPerSample = RegionTiming.CpuCyclesPerSample(Region) / 2.0f;
        CycleAccumulator += cpuCycles;
        while (CycleAccumulator >= 2)
        {
            CycleAccumulator -= 2;
            Pulse1.Tick();
            Pulse2.Tick();
            Triangle.Tick();
            Noise.Tick();
            Dmc.Tick(read);

            MixAccumulator += MixRaw();
            MixCount += 1;
            SampleAccumulator += 1.0f;

            if (SampleAccumulator >= apuCyclesPerSample)
            {
                SampleAccumulator -= apuCyclesPerSample;
                if (MixCount > 0)
                {
                    LastDecimated = MixAccumulator / MixCount;
                    MixAccumulator = 0.0f;
                    MixCount = 0;
                }
            }
        }
    }

    public void ClockQuarterFrame()
    {
        Pulse1.ClockQuarterFrame();
        Pulse2.ClockQuarterFrame();
        Triangle.ClockQuarterFrame();
        Noise.ClockQuarterFrame();
    }

    public void ClockHalfFrame()
    {
        Pulse1.ClockHalfFrame();
        Pulse2.ClockHalfFrame();
        Triangle.ClockHalfFrame();
        Noise.ClockHalfFrame();
    }

    public byte Mix()
    {
        ushort s = (ushort)(Pulse1.Sample() + Pulse2.Sample() + Triangle.Sample() + Noise.Sample());
        return (s > 15) ? (byte)15 : (byte)s;
    }

    public float Output()
    {
        float clamped = LastDecimated;
        const float LpfAlpha = 0.8192f;
        float filtered = LpfAlpha * clamped + (1.0f - LpfAlpha) * LpfPrev;
        LpfPrev = filtered;
        const float DcR = 0.99715f;
        float dcOut = filtered - DcPrevX + DcR * DcPrevY;
        DcPrevX = filtered;
        DcPrevY = dcOut;
        return dcOut;
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private float ScaledSample(int idx, byte raw)
    {
        if (idx >= ChannelCount || ChannelMuted[idx]) return 0.0f;
        return raw * ChannelVolumes[idx];
    }

    private float MixRaw()
    {
        float p1 = ScaledSample(0, Pulse1.Sample());
        float p2 = ScaledSample(1, Pulse2.Sample());
        float tri = ScaledSample(2, Triangle.Sample());
        float noise = ScaledSample(3, Noise.Sample());
        float dmc = ScaledSample(4, Dmc.Sample());

        float pulseSum = p1 + p2;
        float pulseOut = (pulseSum > 0.0f)
            ? (95.52f / (8128.0f / pulseSum + 100.0f))
            : 0.0f;

        float tndInner = tri / 8227.0f + noise / 12241.0f + dmc / 22638.0f;
        float tndOut = (tndInner > 0.0f)
            ? (163.67f / (1.0f / tndInner + 100.0f))
            : 0.0f;

        float mixed = (pulseOut + tndOut) * 2.0f - 1.0f;
        if (mixed < -1.0f) mixed = -1.0f;
        if (mixed > 1.0f) mixed = 1.0f;
        return mixed;
    }

    // ---- Per-channel volume / mute accessors ----

    public void SetChannelVolume(int idx, float vol)
    {
        if (idx < 0 || idx >= ChannelCount) return;
        if (vol < 0.0f) vol = 0.0f;
        if (vol > 1.0f) vol = 1.0f;
        ChannelVolumes[idx] = vol;
    }

    public float ChannelVolume(int idx)
        => (idx < 0 || idx >= ChannelCount) ? 0.0f : ChannelVolumes[idx];

    public bool ChannelMutedAt(int idx)
        => (idx < 0 || idx >= ChannelCount) ? true : ChannelMuted[idx];

    public bool ToggleChannelMute(int idx)
    {
        if (idx < 0 || idx >= ChannelCount) return false;
        ChannelMuted[idx] = !ChannelMuted[idx];
        return ChannelMuted[idx];
    }

    public void SetChannelMuted(int idx, bool muted)
    {
        if (idx < 0 || idx >= ChannelCount) return;
        ChannelMuted[idx] = muted;
    }

    public void SetSelectedChannel(byte idx)
        => SelectedChannel = (idx > 4) ? (byte)4 : idx;

    public void ResetChannelMix()
    {
        for (int i = 0; i < ChannelCount; ++i)
        {
            ChannelVolumes[i] = 1.0f;
            ChannelMuted[i] = false;
        }
    }

    public void ApplyChannelVolumes(float[] vols)
    {
        for (int i = 0; i < vols.Length && i < ChannelCount; ++i)
        {
            float v = vols[i];
            if (v < 0.0f) v = 0.0f;
            if (v > 1.0f) v = 1.0f;
            ChannelVolumes[i] = v;
        }
    }
}
