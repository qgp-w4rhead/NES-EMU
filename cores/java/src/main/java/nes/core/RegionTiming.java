package nes.core;

// Region timing accessors (port of cores/csharp/src/Region.cs RegionTiming).
public final class RegionTiming {
    private RegionTiming() {}

    // APU frame-counter threshold: cycle position + quarter/half flags.
    public static final class ApuFrameThreshold {
        public int threshold;
        public boolean quarter;
        public boolean half;
    }

    public static int scanlinesPerFrame(int r) {
        switch (r) {
            case Region.PAL: return 312;
            case Region.DENDY: return 312;
            default: return 262;
        }
    }

    public static int scanlinePrerender(int r) {
        switch (r) {
            case Region.PAL: return 311;
            case Region.DENDY: return 311;
            default: return 261;
        }
    }

    public static boolean isPalPalette(int r) { return r == Region.PAL; }

    public static float cpuClockHz(int r) {
        switch (r) {
            case Region.PAL: return 1662607.0f;
            case Region.DENDY: return 1789773.0f;
            default: return 1789773.0f;
        }
    }

    public static float cpuCyclesPerSample(int r) {
        return cpuClockHz(r) / 44100.0f;
    }

    // Pre-allocated threshold tables (avoid per-call allocation in hot path).
    private static final int[] NTSC_4STEP = {7457, 14913, 22371, 29828};
    private static final int[] PAL_4STEP = {8314, 16627, 24941, 33255};
    private static final int[] NTSC_5STEP = {7457, 14913, 22371, 37281};
    private static final int[] PAL_5STEP = {8314, 16627, 24941, 41568};
    private static final boolean[] QUARTER_FLAGS = {true, true, true, true};
    private static final boolean[] HALF_FLAGS = {false, true, false, true};

    public static void apu4StepThresholds(int r, ApuFrameThreshold[] outs) {
        int[] t = (r == Region.PAL) ? PAL_4STEP : NTSC_4STEP;
        for (int i = 0; i < 4; ++i) {
            outs[i].threshold = t[i];
            outs[i].quarter = QUARTER_FLAGS[i];
            outs[i].half = HALF_FLAGS[i];
        }
    }

    public static void apu5StepThresholds(int r, ApuFrameThreshold[] outs) {
        int[] t = (r == Region.PAL) ? PAL_5STEP : NTSC_5STEP;
        for (int i = 0; i < 4; ++i) {
            outs[i].threshold = t[i];
            outs[i].quarter = QUARTER_FLAGS[i];
            outs[i].half = HALF_FLAGS[i];
        }
    }

    public static int apuResetAt(int r, boolean mode5step) {
        if (r == Region.PAL) return mode5step ? 41570 : 33257;
        return mode5step ? 37282 : 29830;
    }

    public static int apu4StepIrqThreshold(int r) {
        return (r == Region.PAL) ? 33255 : 29828;
    }
}
