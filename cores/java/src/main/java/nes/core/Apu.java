package nes.core;

// Audio Processing Unit (port of cores/csharp/src/Audio/Apu.cs).
public final class Apu {
    public static final int CHANNEL_COUNT = 5;
    public static final int SAMPLE_RATE = 44100;

    public final PulseChannel pulse1 = new PulseChannel(false);
    public final PulseChannel pulse2 = new PulseChannel(true);
    public final TriangleChannel triangle = new TriangleChannel();
    public final NoiseChannel noise = new NoiseChannel();
    public final DmcChannel dmc = new DmcChannel();

    public int cycleAccumulator;

    public final float[] channelVolumes = new float[CHANNEL_COUNT];
    public final boolean[] channelMuted = new boolean[CHANNEL_COUNT];
    public int selectedChannel;

    public float lpfPrev;
    public float dcPrevX;
    public float dcPrevY;
    public float mixAccumulator;
    public int mixCount;
    public float sampleAccumulator;
    public float lastDecimated;

    public boolean frameMode5Step;
    public boolean frameIrqInhibit;
    public int frameCycle;
    public boolean frameIrq;
    public int frameResetDelay;

    public int region = Region.NTSC;

    private final RegionTiming.ApuFrameThreshold[] _thresholds = new RegionTiming.ApuFrameThreshold[4];

    public Apu() {
        for (int i = 0; i < 4; ++i) _thresholds[i] = new RegionTiming.ApuFrameThreshold();
        for (int i = 0; i < CHANNEL_COUNT; ++i) { channelVolumes[i] = 1.0f; channelMuted[i] = false; }
        lpfPrev = -1.0f;
        dcPrevX = -1.0f;
        dcPrevY = 0.0f;
        lastDecimated = -1.0f;
    }

    public void writeStatus(int value) {
        pulse1.setEnabled((value & 0x01) != 0);
        pulse2.setEnabled((value & 0x02) != 0);
        triangle.setEnabled((value & 0x04) != 0);
        noise.setEnabled((value & 0x08) != 0);
        dmc.setEnabled((value & 0x10) != 0);
    }

    public int readStatus() {
        int v = 0;
        if (pulse1.lengthCounter > 0) v |= 0x01;
        if (pulse2.lengthCounter > 0) v |= 0x02;
        if (triangle.lengthCounter > 0) v |= 0x04;
        if (noise.lengthCounter > 0) v |= 0x08;
        if (dmc.bytesRemaining > 0) v |= 0x10;
        if (frameIrq) v |= 0x40;
        if (dmc.irqFlag) v |= 0x80;
        frameIrq = false;
        dmc.irqFlag = false;
        return v & 0xFF;
    }

    public void writeFrameCounter(int value) {
        boolean newMode5Step = (value & 0x80) != 0;
        frameIrqInhibit = (value & 0x40) != 0;
        if (frameIrqInhibit) frameIrq = false;
        if (newMode5Step) { clockQuarterFrame(); clockHalfFrame(); }
        frameResetDelay = 4;
        frameMode5Step = newMode5Step;
    }

    public boolean irqPending() { return frameIrq || dmc.irqFlag; }

    public void setRegion(int r) { region = r; }

    private void stepFrameCounter(int cpuCycles) {
        if (frameResetDelay > 0) {
            int advance = Math.min(cpuCycles, frameResetDelay);
            frameResetDelay -= advance;
            if (frameResetDelay == 0) frameCycle = 0;
            int remaining = cpuCycles - advance;
            if (remaining == 0) return;
            frameCycle += remaining;
        } else {
            frameCycle += cpuCycles;
        }

        int prev = (frameCycle >= cpuCycles) ? (frameCycle - cpuCycles) : 0;
        if (frameMode5Step) RegionTiming.apu5StepThresholds(region, _thresholds);
        else RegionTiming.apu4StepThresholds(region, _thresholds);
        int irqThreshold = RegionTiming.apu4StepIrqThreshold(region);

        for (int i = 0; i < 4; ++i) {
            int threshold = _thresholds[i].threshold;
            boolean quarter = _thresholds[i].quarter;
            boolean half = _thresholds[i].half;
            if (prev < threshold && frameCycle >= threshold) {
                if (quarter) clockQuarterFrame();
                if (half) clockHalfFrame();
                if (!frameMode5Step && threshold == irqThreshold && !frameIrqInhibit)
                    frameIrq = true;
            }
        }

        int resetAt = RegionTiming.apuResetAt(region, frameMode5Step);
        if (frameCycle >= resetAt) frameCycle -= resetAt;
    }

    public void step(int cpuCycles, DmcReadFn read) {
        stepFrameCounter(cpuCycles);

        float apuCyclesPerSample = RegionTiming.cpuCyclesPerSample(region) / 2.0f;
        cycleAccumulator += cpuCycles;
        while (cycleAccumulator >= 2) {
            cycleAccumulator -= 2;
            pulse1.tick();
            pulse2.tick();
            triangle.tick();
            noise.tick();
            dmc.tick(read);

            mixAccumulator += mixRaw();
            mixCount += 1;
            sampleAccumulator += 1.0f;

            if (sampleAccumulator >= apuCyclesPerSample) {
                sampleAccumulator -= apuCyclesPerSample;
                if (mixCount > 0) {
                    lastDecimated = mixAccumulator / mixCount;
                    mixAccumulator = 0.0f;
                    mixCount = 0;
                }
            }
        }
    }

    public void clockQuarterFrame() {
        pulse1.clockQuarterFrame();
        pulse2.clockQuarterFrame();
        triangle.clockQuarterFrame();
        noise.clockQuarterFrame();
    }

    public void clockHalfFrame() {
        pulse1.clockHalfFrame();
        pulse2.clockHalfFrame();
        triangle.clockHalfFrame();
        noise.clockHalfFrame();
    }

    public int mix() {
        int s = pulse1.sample() + pulse2.sample() + triangle.sample() + noise.sample();
        return (s > 15) ? 15 : s;
    }

    public float output() {
        float clamped = lastDecimated;
        final float LpfAlpha = 0.8192f;
        float filtered = LpfAlpha * clamped + (1.0f - LpfAlpha) * lpfPrev;
        lpfPrev = filtered;
        final float DcR = 0.99715f;
        float dcOut = filtered - dcPrevX + DcR * dcPrevY;
        dcPrevX = filtered;
        dcPrevY = dcOut;
        return dcOut;
    }

    private float scaledSample(int idx, int raw) {
        if (idx >= CHANNEL_COUNT || channelMuted[idx]) return 0.0f;
        return raw * channelVolumes[idx];
    }

    private float mixRaw() {
        float p1 = scaledSample(0, pulse1.sample());
        float p2 = scaledSample(1, pulse2.sample());
        float tri = scaledSample(2, triangle.sample());
        float n = scaledSample(3, noise.sample());
        float dmc = scaledSample(4, this.dmc.sample());

        float pulseSum = p1 + p2;
        float pulseOut = (pulseSum > 0.0f) ? (95.52f / (8128.0f / pulseSum + 100.0f)) : 0.0f;

        float tndInner = tri / 8227.0f + n / 12241.0f + dmc / 22638.0f;
        float tndOut = (tndInner > 0.0f) ? (163.67f / (1.0f / tndInner + 100.0f)) : 0.0f;

        float mixed = (pulseOut + tndOut) * 2.0f - 1.0f;
        if (mixed < -1.0f) mixed = -1.0f;
        if (mixed > 1.0f) mixed = 1.0f;
        return mixed;
    }

    public void setChannelVolume(int idx, float vol) {
        if (idx < 0 || idx >= CHANNEL_COUNT) return;
        if (vol < 0.0f) vol = 0.0f;
        if (vol > 1.0f) vol = 1.0f;
        channelVolumes[idx] = vol;
    }

    public float channelVolume(int idx) {
        return (idx < 0 || idx >= CHANNEL_COUNT) ? 0.0f : channelVolumes[idx];
    }

    public boolean channelMutedAt(int idx) {
        return (idx < 0 || idx >= CHANNEL_COUNT) || channelMuted[idx];
    }

    public boolean toggleChannelMute(int idx) {
        if (idx < 0 || idx >= CHANNEL_COUNT) return false;
        channelMuted[idx] = !channelMuted[idx];
        return channelMuted[idx];
    }

    public void setChannelMuted(int idx, boolean muted) {
        if (idx < 0 || idx >= CHANNEL_COUNT) return;
        channelMuted[idx] = muted;
    }

    public void setSelectedChannel(int idx) { selectedChannel = (idx > 4) ? 4 : idx; }

    public void resetChannelMix() {
        for (int i = 0; i < CHANNEL_COUNT; ++i) { channelVolumes[i] = 1.0f; channelMuted[i] = false; }
    }

    public void applyChannelVolumes(float[] vols) {
        for (int i = 0; i < vols.length && i < CHANNEL_COUNT; ++i) {
            float v = vols[i];
            if (v < 0.0f) v = 0.0f;
            if (v > 1.0f) v = 1.0f;
            channelVolumes[i] = v;
        }
    }
}

