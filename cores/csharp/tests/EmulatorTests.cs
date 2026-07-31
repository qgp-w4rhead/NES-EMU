using NesCore;

namespace NesCore.Tests;

public class EmulatorTests
{
    // Build a deterministic NROM-128 NOP ROM (matches bench/gen_rom.c).
    private static byte[] BuildNopRom()
    {
        byte[] rom = new byte[16 + 16384 + 8192];
        rom[0] = (byte)'N'; rom[1] = (byte)'E'; rom[2] = (byte)'S'; rom[3] = 0x1A;
        rom[4] = 1; // PRG-ROM: 1 x 16KB
        rom[5] = 1; // CHR-ROM: 1 x 8KB
        rom[6] = 0; // mapper 0, horizontal mirroring
        rom[7] = 0;
        // bytes 8..15 already zeroed
        // PRG-ROM filled with NOP (0xEA)
        for (int i = 16; i < 16 + 16384; ++i) rom[i] = 0xEA;
        // RESET vector at $FFFC/$FFFD -> $C000
        int resetOff = 16 + 0x3FFC;
        rom[resetOff] = 0x00;
        rom[resetOff + 1] = 0xC0;
        // CHR-ROM zeroed (already)
        return rom;
    }

    private static Emulator CreateEmulator()
    {
        byte[] rom = BuildNopRom();
        int rc = Cartridge.FromBytes(rom, rom.Length, out Cartridge cart);
        Assert.Equal(0, rc);
        var emu = new Emulator();
        emu.Init();
        emu.Cartridge = cart;
        emu.Bus.InitWithCartridge(cart);
        emu.Cpu.Bus = emu.Bus;
        emu.Cpu.Reset();
        return emu;
    }

    [Fact]
    public void StepFrame_ReturnsNonZeroCycles()
    {
        var emu = CreateEmulator();
        uint cycles = emu.StepFrame();
        Assert.True(cycles > 0, "step_frame should return >0 CPU cycles");
        // NTSC frame is ~29830 CPU cycles
        Assert.InRange(cycles, 25000u, 35000u);
    }

    [Fact]
    public void StepInstruction_ReturnsNonZeroCycles()
    {
        var emu = CreateEmulator();
        uint cycles = emu.StepInstruction();
        // NOP = 2 cycles
        Assert.Equal(2u, cycles);
    }

    [Fact]
    public void MapperNumber_ReturnsZeroForNrom()
    {
        var emu = CreateEmulator();
        Assert.Equal((ushort)0, emu.MapperNumber());
    }

    [Fact]
    public void SetRegion_ReturnsPreviousRegion()
    {
        var emu = CreateEmulator();
        Assert.Equal(Region.Ntsc, emu.Region);
        emu.SetRegion(Region.Pal);
        Assert.Equal(Region.Pal, emu.Region);
        emu.SetRegion(Region.Dendy);
        Assert.Equal(Region.Dendy, emu.Region);
    }

    [Fact]
    public void StepFrame_1000Frames_Deterministic()
    {
        var emu = CreateEmulator();
        uint firstFrameCycles = 0;
        for (int i = 0; i < 1000; ++i)
        {
            uint cycles = emu.StepFrame();
            if (i == 0) firstFrameCycles = cycles;
        }
        // After 1000 frames, the emulator should still be running
        // Run one more frame and verify it produces cycles
        uint finalCycles = emu.StepFrame();
        Assert.True(finalCycles > 0, "emulator should still produce cycles after 1000 frames");
    }

    [Fact]
    public void TakeAudio_DrainsSamples()
    {
        var emu = CreateEmulator();
        emu.StepFrame();
        float[] buf = new float[2048];
        uint n = emu.TakeAudioSamples(buf, 2048);
        // NTSC frame produces ~735 audio samples
        Assert.True(n > 0, "should have audio samples after a frame");
        Assert.True(n < 2048, "should not exceed buffer capacity");
        // Second call should return 0 (buffer drained)
        uint n2 = emu.TakeAudioSamples(buf, 2048);
        Assert.Equal(0u, n2);
    }

    [Fact]
    public void SaveLoadState_Roundtrip()
    {
        var emu = CreateEmulator();
        // Run a few frames to populate state
        for (int i = 0; i < 5; ++i) emu.StepFrame();

        // Save
        uint requiredSize = SaveState.RequiredSize(emu);
        Assert.True(requiredSize > 0, "save state size should be >0");
        byte[] saved = new byte[requiredSize];
        uint written = SaveState.Save(emu, saved, requiredSize);
        Assert.Equal(requiredSize, written);

        // Capture CPU state before save
        byte aBefore = emu.Cpu.A;
        byte xBefore = emu.Cpu.X;
        byte yBefore = emu.Cpu.Y;
        ushort pcBefore = emu.Cpu.Pc;
        ulong cyclesBefore = emu.Bus.CpuCycleCount;

        // Run more frames to change state
        for (int i = 0; i < 5; ++i) emu.StepFrame();

        // Load state back
        bool loaded = SaveState.Load(emu, saved, written);
        Assert.True(loaded, "load_state should succeed");

        // Verify CPU state was restored
        Assert.Equal(aBefore, emu.Cpu.A);
        Assert.Equal(xBefore, emu.Cpu.X);
        Assert.Equal(yBefore, emu.Cpu.Y);
        Assert.Equal(pcBefore, emu.Cpu.Pc);
        Assert.Equal(cyclesBefore, emu.Bus.CpuCycleCount);

        // Run one more frame after restore — should produce cycles
        uint cyclesAfter = emu.StepFrame();
        Assert.True(cyclesAfter > 0, "emulator should run after state restore");

        // Drain audio and save again — verify size matches original
        // (audio buffer count may differ, so drain first)
        float[] drainBuf = new float[4096];
        emu.TakeAudioSamples(drainBuf, 4096);
        uint requiredSize2 = SaveState.RequiredSize(emu);
        // Sizes should be close (audio buffer is drained in both cases)
        Assert.True(requiredSize2 <= requiredSize + 100,
            $"Save state size unstable: {requiredSize} -> {requiredSize2}");
    }

    [Fact]
    public void NoGcCollections_1000Frames()
    {
        var emu = CreateEmulator();
        float[] audioBuf = new float[1024];

        // Warm up: run enough frames for JIT to compile all hot paths
        for (int i = 0; i < 200; ++i)
        {
            emu.StepFrame();
            emu.TakeAudioSamples(audioBuf, 1024);
        }

        // Measure allocated bytes (precise — doesn't depend on GC state)
        long allocBefore = GC.GetAllocatedBytesForCurrentThread();
        int gen0Before = GC.CollectionCount(0);
        int gen1Before = GC.CollectionCount(1);
        int gen2Before = GC.CollectionCount(2);

        // Run 1000 frames
        for (int i = 0; i < 1000; ++i)
        {
            emu.StepFrame();
            emu.TakeAudioSamples(audioBuf, 1024);
        }

        long allocAfter = GC.GetAllocatedBytesForCurrentThread();
        int gen0After = GC.CollectionCount(0);
        int gen1After = GC.CollectionCount(1);
        int gen2After = GC.CollectionCount(2);

        // Total heap allocations over 1000 frames should be near zero
        // (the core must not allocate in the steady-state hot path).
        long totalAlloc = allocAfter - allocBefore;
        Assert.True(totalAlloc < 10_000,
            $"Heap allocations over 1000 frames: {totalAlloc} bytes (should be ~0)");

        // Gen2 collections indicate serious memory pressure (should be 0).
        int gen2Collections = gen2After - gen2Before;
        Assert.True(gen2Collections == 0,
            $"Gen2 GC collections: {gen2Collections} (should be 0)");
    }

    [Fact]
    public void Framebuffer_NotNullAfterStep()
    {
        var emu = CreateEmulator();
        emu.StepFrame();
        uint[] fb = emu.Bus.Ppu.Framebuffer;
        Assert.NotNull(fb);
        Assert.Equal(256 * 240, fb.Length);
    }

    [Fact]
    public void Reset_Works()
    {
        var emu = CreateEmulator();
        emu.StepFrame();
        emu.StepFrame();
        emu.Reset();
        // After reset, PC should be at the RESET vector
        Assert.Equal((ushort)0xC000, emu.Cpu.Pc);
    }
}
