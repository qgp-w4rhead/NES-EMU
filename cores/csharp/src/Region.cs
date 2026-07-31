using System.Runtime.CompilerServices;

namespace NesCore;

// TV system / region (src/region.rs Region).
public enum Region : byte
{
    Ntsc = 0,
    Pal = 1,
    Dendy = 2,
}

// APU frame-counter threshold: cycle position + quarter/half flags.
public struct ApuFrameThreshold
{
    public uint Threshold;
    public bool Quarter;
    public bool Half;
}

// Region timing accessors (port of src/region.rs).
public static class RegionTiming
{
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public static ushort ScanlinesPerFrame(Region r) => r switch
    {
        Region.Ntsc => 262,
        Region.Pal => 312,
        Region.Dendy => 312,
        _ => 262,
    };

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public static ushort ScanlinePrerender(Region r) => r switch
    {
        Region.Ntsc => 261,
        Region.Pal => 311,
        Region.Dendy => 311,
        _ => 261,
    };

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public static bool IsPalPalette(Region r) => r == Region.Pal;

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public static float CpuClockHz(Region r) => r switch
    {
        Region.Ntsc => 1789773.0f,
        Region.Pal => 1662607.0f,
        Region.Dendy => 1789773.0f,
        _ => 1789773.0f,
    };

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public static float CpuCyclesPerSample(Region r) => CpuClockHz(r) / 44100.0f;

    // Pre-allocated threshold tables (avoid per-call allocation in hot path).
    private static readonly uint[] Ntsc4Step = { 7457u, 14913u, 22371u, 29828u };
    private static readonly uint[] Pal4Step = { 8314u, 16627u, 24941u, 33255u };
    private static readonly uint[] Ntsc5Step = { 7457u, 14913u, 22371u, 37281u };
    private static readonly uint[] Pal5Step = { 8314u, 16627u, 24941u, 41568u };
    private static readonly bool[] QuarterFlags = { true, true, true, true };
    private static readonly bool[] HalfFlags = { false, true, false, true };

    public static void Apu4StepThresholds(Region r, ApuFrameThreshold[] outs)
    {
        uint[] t = (r == Region.Pal) ? Pal4Step : Ntsc4Step;
        for (int i = 0; i < 4; ++i)
        {
            outs[i].Threshold = t[i];
            outs[i].Quarter = QuarterFlags[i];
            outs[i].Half = HalfFlags[i];
        }
    }

    public static void Apu5StepThresholds(Region r, ApuFrameThreshold[] outs)
    {
        uint[] t = (r == Region.Pal) ? Pal5Step : Ntsc5Step;
        for (int i = 0; i < 4; ++i)
        {
            outs[i].Threshold = t[i];
            outs[i].Quarter = QuarterFlags[i];
            outs[i].Half = HalfFlags[i];
        }
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public static uint ApuResetAt(Region r, bool mode5step) => r switch
    {
        Region.Ntsc or Region.Dendy => mode5step ? 37282u : 29830u,
        Region.Pal => mode5step ? 41570u : 33257u,
        _ => mode5step ? 37282u : 29830u,
    };

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public static uint Apu4StepIrqThreshold(Region r) => r switch
    {
        Region.Ntsc or Region.Dendy => 29828u,
        Region.Pal => 33255u,
        _ => 29828u,
    };
}
