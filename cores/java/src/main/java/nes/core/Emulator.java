package nes.core;

// EmulatorState: ties CPU, PPU, APU, bus, and cartridge together in a
// frame-locked loop. Port of cores/csharp/src/Emulator.cs.
public final class Emulator {
    private static final int AUDIO_CAP_NTSC = 800;
    private static final int AUDIO_CAP_PAL = 950;

    public Cpu cpu = new Cpu();
    public Bus bus = new Bus();
    public Cartridge cartridge;
    public int region = Region.NTSC;

    public float sampleAccumulator;
    public int ppuCycleCarry;
    public boolean frameDoneFlag;  // reused across stepFrame/stepInstruction (avoids boolean[1] alloc)

    public float[] audioBuffer;
    public int audioBufferCapacity;
    public int audioBufferCount;

    public Emulator() {}

    public void initWithRegion(int r) {
        cpu = new Cpu();
        bus = new Bus();
        cpu.bus = bus;
        bus.ppu.setRegion(r);
        bus.apu.setRegion(r);
        region = r;
        sampleAccumulator = 0.0f;
        ppuCycleCarry = 0;
        cartridge = null;
        int cap = (RegionTiming.scanlinesPerFrame(r) > 262) ? AUDIO_CAP_PAL : AUDIO_CAP_NTSC;
        audioBuffer = new float[cap];
        audioBufferCapacity = cap;
        audioBufferCount = 0;
    }

    public void init() { initWithRegion(Region.NTSC); }

    public void destroy() {
        cartridge = null;
        audioBuffer = null;
        audioBufferCount = 0;
        audioBufferCapacity = 0;
    }

    public void reset() { cpu.reset(); }

    public int currentRegion() { return region; }

    public void setRegion(int r) {
        region = r;
        bus.ppu.setRegion(r);
        bus.apu.setRegion(r);
    }

    private void pushAudio(float sample) {
        if (audioBufferCount >= audioBufferCapacity) {
            int newCap = audioBufferCapacity * 2;
            if (newCap < 16) newCap = 16;
            float[] nb = new float[newCap];
            if (audioBuffer != null && audioBufferCount > 0)
                System.arraycopy(audioBuffer, 0, nb, 0, audioBufferCount);
            audioBuffer = nb;
            audioBufferCapacity = newCap;
        }
        audioBuffer[audioBufferCount++] = sample;
    }

    private int stepOneCpuTick(int prevScanline, float cyclesPerSample,
                                int prerender) {
        int cpuCycles = 0;

        int stepCycles = cpu.step();
        cpuCycles += stepCycles;

        int dmaCycles = bus.takeDmaStallCycles();
        cpuCycles += dmaCycles;

        bus.advanceCpuCycles((stepCycles + dmaCycles));

        int apuCycles = stepCycles + dmaCycles;
        bus.stepApu(apuCycles);
        bus.clockCartCpu(apuCycles);

        if (bus.apuIrqPending()) cpu.setIrqPending(true);
        if (bus.cartIrqPending()) cpu.setIrqPending(true);

        sampleAccumulator += apuCycles;
        while (sampleAccumulator >= cyclesPerSample) {
            sampleAccumulator -= cyclesPerSample;
            float internalSample = bus.apu.output();
            float expansion = bus.expansionAudioSample();
            float mixed = internalSample + expansion;
            if (mixed < -1.0f) mixed = -1.0f;
            if (mixed > 1.0f) mixed = 1.0f;
            pushAudio(mixed);
        }

        int ppuCycles = 3 * apuCycles + ppuCycleCarry;
        ppuCycleCarry = 0;
        int remaining = ppuCycles;
        boolean done = false;
        while (remaining > 0) {
            int chunk = remaining;
            if (chunk > Ppu.CYCLES_PER_SCANLINE) chunk = Ppu.CYCLES_PER_SCANLINE;
            bus.stepPpu(chunk);
            remaining -= chunk;

            if (bus.takeNmiRequest()) cpu.setNmiPending(true);

            int currScanline = bus.ppu.scanline;
            if (currScanline == 0 && prevScanline == prerender) {
                ppuCycleCarry = remaining;
                done = true;
                break;
            }
        }

        frameDoneFlag = done;
        return cpuCycles;
    }

    public int stepFrame() {
        int cpuCycles = 0;

        int universalBg = PpuRender.universalBgArgb(bus.ppu);
        PpuRender.clearFramebuffer(bus.ppu, universalBg);
        bus.ppu.resetRenderedFlag();

        float cyclesPerSample = RegionTiming.cpuCyclesPerSample(region);
        int prerender = RegionTiming.scanlinePrerender(region);

        for (;;) {
            int prevScanline = bus.ppu.scanline;
            int tickCycles = stepOneCpuTick(prevScanline, cyclesPerSample, prerender);
            cpuCycles += tickCycles;
            if (frameDoneFlag) break;
        }

        if (!bus.ppu.renderedThisFrame())
            bus.renderFrame();

        return cpuCycles;
    }

    public int stepInstruction() {
        int prevScanline = bus.ppu.scanline;
        float cyclesPerSample = RegionTiming.cpuCyclesPerSample(region);
        int prerender = RegionTiming.scanlinePrerender(region);
        return stepOneCpuTick(prevScanline, cyclesPerSample, prerender);
    }

    public int[] framebuffer() { return bus.ppu.framebuffer; }

    public int mapperNumber() {
        return (cartridge != null) ? cartridge.header.mapperNumber : 0;
    }

    public int takeAudioSamples(float[] outBuf, int cap) {
        int n = audioBufferCount;
        if (n > cap) n = cap;
        if (n > 0 && outBuf != null)
            System.arraycopy(audioBuffer, 0, outBuf, 0, n);
        audioBufferCount = 0;
        return n;
    }

    public void setAudioBuffer(float[] buf, int count) {
        if (count > audioBufferCapacity) {
            audioBuffer = new float[count];
            audioBufferCapacity = count;
        }
        if (count > 0 && buf != null)
            System.arraycopy(buf, 0, audioBuffer, 0, count);
        audioBufferCount = count;
    }
}

