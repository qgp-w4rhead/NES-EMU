using System.Runtime.CompilerServices;

namespace NesCore;

// EmulatorState: ties CPU, PPU, APU, bus, and cartridge together in a
// frame-locked loop. Port of cores/c/src/emulator.c.
//
// Frame loop: 1 CPU cycle = 3 PPU cycles; audio samples accumulated from
// the APU + expansion audio; frame boundary detected by the PPU scanline
// wrapping from the prerender scanline back to 0; leftover PPU cycles
// carried into the next frame.
//
// See: https://www.nesdev.org/wiki/Cycle_reference
public sealed class Emulator
{
    private const uint AudioCapNtsc = 800;
    private const uint AudioCapPal = 950;

    public Cpu Cpu = new Cpu();
    public Bus Bus = new Bus();
    public Cartridge Cartridge;
    public Region Region = Region.Ntsc;

    public float SampleAccumulator;
    public uint PpuCycleCarry;

    public float[] AudioBuffer;
    public uint AudioBufferCapacity;
    public uint AudioBufferCount;

    public Emulator() { }

    public void InitWithRegion(Region region)
    {
        Cpu = new Cpu();
        Bus = new Bus();
        Cpu.Bus = Bus;
        Bus.Ppu.SetRegion(region);
        Bus.Apu.SetRegion(region);
        Region = region;
        SampleAccumulator = 0.0f;
        PpuCycleCarry = 0u;
        Cartridge = null;
        uint cap = (RegionTiming.ScanlinesPerFrame(region) > 262u) ? AudioCapPal : AudioCapNtsc;
        AudioBuffer = new float[cap];
        AudioBufferCapacity = cap;
        AudioBufferCount = 0u;
    }

    public void Init() => InitWithRegion(Region.Ntsc);

    public void Destroy()
    {
        Cartridge = null;
        AudioBuffer = null;
        AudioBufferCount = 0u;
        AudioBufferCapacity = 0u;
    }

    public void Reset()
    {
        Cpu.Reset();
    }

    public Region CurrentRegion => Region;

    public void SetRegion(Region region)
    {
        Region = region;
        Bus.Ppu.SetRegion(region);
        Bus.Apu.SetRegion(region);
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private void PushAudio(float sample)
    {
        if (AudioBufferCount >= AudioBufferCapacity)
        {
            uint newCap = AudioBufferCapacity * 2u;
            if (newCap < 16u) newCap = 16u;
            float[] nb = new float[newCap];
            if (AudioBuffer != null && AudioBufferCount > 0)
                System.Array.Copy(AudioBuffer, nb, AudioBufferCount);
            AudioBuffer = nb;
            AudioBufferCapacity = newCap;
        }
        AudioBuffer[AudioBufferCount++] = sample;
    }

    // Run one CPU instruction plus its PPU/APU/mapper side-effects, returning
    // the CPU cycles consumed this tick and whether the frame just completed.
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private uint StepOneCpuTick(ushort prevScanline, float cyclesPerSample,
                                 ushort prerender, out bool frameDone)
    {
        uint cpuCycles = 0u;

        byte stepCycles = Cpu.Step();
        cpuCycles += stepCycles;

        uint dmaCycles = Bus.TakeDmaStallCycles();
        cpuCycles += dmaCycles;

        Bus.AdvanceCpuCycles((uint)stepCycles + dmaCycles);

        uint apuCycles = (uint)stepCycles + dmaCycles;
        Bus.StepApu(apuCycles);
        Bus.ClockCartCpu(apuCycles);

        if (Bus.ApuIrqPending()) Cpu.SetIrqPending(true);
        if (Bus.CartIrqPending()) Cpu.SetIrqPending(true);

        SampleAccumulator += apuCycles;
        while (SampleAccumulator >= cyclesPerSample)
        {
            SampleAccumulator -= cyclesPerSample;
            float internalSample = Bus.Apu.Output();
            float expansion = Bus.ExpansionAudioSample();
            float mixed = internalSample + expansion;
            if (mixed < -1.0f) mixed = -1.0f;
            if (mixed > 1.0f) mixed = 1.0f;
            PushAudio(mixed);
        }

        uint ppuCycles = 3u * apuCycles + PpuCycleCarry;
        PpuCycleCarry = 0u;
        uint remaining = ppuCycles;
        bool done = false;
        while (remaining > 0u)
        {
            uint chunk = remaining;
            if (chunk > Ppu.CyclesPerScanline) chunk = Ppu.CyclesPerScanline;
            Bus.StepPpu(chunk);
            remaining -= chunk;

            if (Bus.TakeNmiRequest()) Cpu.SetNmiPending(true);

            ushort currScanline = Bus.Ppu.Scanline;
            if (currScanline == 0 && prevScanline == prerender)
            {
                PpuCycleCarry = remaining;
                done = true;
                break;
            }
        }

        frameDone = done;
        return cpuCycles;
    }

    public uint StepFrame()
    {
        uint cpuCycles = 0u;

        // Clear framebuffer to universal background color first.
        uint universalBg = PpuRender.UniversalBgArgb(Bus.Ppu);
        PpuRender.ClearFramebuffer(Bus.Ppu, universalBg);
        Bus.Ppu.ResetRenderedFlag();

        float cyclesPerSample = RegionTiming.CpuCyclesPerSample(Region);
        ushort prerender = RegionTiming.ScanlinePrerender(Region);

        for (;;)
        {
            ushort prevScanline = Bus.Ppu.Scanline;
            uint tickCycles = StepOneCpuTick(prevScanline, cyclesPerSample,
                                              prerender, out bool frameDone);
            cpuCycles += tickCycles;
            if (frameDone) break;
        }

        // Fallback: if no per-pixel output happened, render the whole frame.
        if (!Bus.Ppu.RenderedThisFrame())
            Bus.RenderFrame();

        return cpuCycles;
    }

    public uint StepInstruction()
    {
        ushort prevScanline = Bus.Ppu.Scanline;
        float cyclesPerSample = RegionTiming.CpuCyclesPerSample(Region);
        ushort prerender = RegionTiming.ScanlinePrerender(Region);
        return StepOneCpuTick(prevScanline, cyclesPerSample, prerender, out _);
    }

    public uint[] Framebuffer => Bus.Ppu.Framebuffer;

    public ushort MapperNumber()
    {
        if (Cartridge != null) return Cartridge.Header.MapperNumber;
        return 0;
    }

    public uint TakeAudioSamples(float[] outBuf, uint cap)
    {
        uint n = AudioBufferCount;
        if (n > cap) n = cap;
        if (n > 0u && outBuf != null)
            System.Array.Copy(AudioBuffer, outBuf, n);
        AudioBufferCount = 0u;
        return n;
    }

    public void SetAudioBuffer(float[] buf, uint count)
    {
        if (count > AudioBufferCapacity)
        {
            AudioBuffer = new float[count];
            AudioBufferCapacity = count;
        }
        if (count > 0u && buf != null)
            System.Array.Copy(buf, AudioBuffer, count);
        AudioBufferCount = count;
    }
}
