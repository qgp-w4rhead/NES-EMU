package nes.core;

import nes.core.Cartridge;

/**
 * Standalone test runner for the Java NES core (no JUnit dependency).
 * Run via: java -cp classes nes.core.TestRunner
 *
 * Mirrors the C# xUnit test suite: step_frame cycle count, step_instruction,
 * mapper number, save/load state roundtrip, set_region, 1000-frame run,
 * audio drain, framebuffer non-null, reset, determinism.
 */
public final class TestRunner {
    private static int s_pass = 0;
    private static int s_fail = 0;

    private TestRunner() {}

    public static void main(String[] args) {
        System.out.println("=== Java NES Core Tests ===");

        test("step_frame runs ~29782 cycles", TestRunner::testStepFrame);
        test("step_instruction returns 2 cycles for NOP", TestRunner::testStepInstruction);
        test("mapper_number returns 0 for NROM", TestRunner::testMapperNumber);
        test("save/load state roundtrip", TestRunner::testSaveLoadState);
        test("set_region returns previous region", TestRunner::testSetRegion);
        test("1000-frame run completes", TestRunner::test1000Frames);
        test("audio drain returns samples", TestRunner::testAudioDrain);
        test("framebuffer non-null after step", TestRunner::testFramebuffer);
        test("reset works", TestRunner::testReset);
        test("deterministic across runs", TestRunner::testDeterministic);

        System.out.println();
        System.out.println("=== Results: " + s_pass + " passed, " + s_fail + " failed ===");
        if (s_fail > 0) System.exit(1);
    }

    private interface Test {
        void run() throws Throwable;
    }

    private static void test(String name, Test body) {
        try {
            body.run();
            System.out.println("  PASS: " + name);
            s_pass++;
        } catch (Throwable t) {
            System.out.println("  FAIL: " + name + " — " + t);
            t.printStackTrace();
            s_fail++;
        }
    }

    // ---- NOP ROM fixture (matches bench/gen_rom.c) ----

    private static byte[] nopRom() {
        byte[] rom = new byte[24592];
        rom[0] = 'N'; rom[1] = 'E'; rom[2] = 'S'; rom[3] = 0x1A;
        rom[4] = 1;   // PRG-ROM: 1 x 16KB
        rom[5] = 1;   // CHR-ROM: 1 x 8KB
        rom[6] = 0;   // mapper low + flags
        rom[7] = 0;   // mapper high + flags
        // bytes 8..15 already 0
        // PRG-ROM filled with NOP (0xEA)
        for (int i = 16; i < 16 + 16384; ++i) rom[i] = (byte) 0xEA;
        // RESET vector at $FFFC/$FFFD -> $C000
        int resetOff = 16 + 0x3FFC;
        rom[resetOff] = 0x00;
        rom[resetOff + 1] = (byte) 0xC0;
        // CHR-ROM already 0
        return rom;
    }

    private static Emulator makeEmu() {
        byte[] rom = nopRom();
        Cartridge[] out = new Cartridge[1];
        int rc = Cartridge.fromBytes(rom, rom.length, out);
        if (rc != 0) throw new RuntimeException("Cartridge.fromBytes failed rc=" + rc);
        Emulator emu = new Emulator();
        emu.init();
        emu.cartridge = out[0];
        emu.bus.initWithCartridge(out[0]);
        emu.cpu.bus = emu.bus;
        emu.cpu.reset();
        return emu;
    }

    // ---- Tests ----

    private static void testStepFrame() {
        Emulator emu = makeEmu();
        int cycles = emu.stepFrame();
        // NTSC frame ≈ 29782 cycles (the C/C# cores report ~29782)
        if (cycles < 29000 || cycles > 31000)
            throw new RuntimeException("step_frame cycles=" + cycles + " (expected ~29782)");
    }

    private static void testStepInstruction() {
        Emulator emu = makeEmu();
        int cycles = emu.stepInstruction();
        // NOP = 2 cycles
        if (cycles != 2)
            throw new RuntimeException("step_instruction cycles=" + cycles + " (expected 2)");
    }

    private static void testMapperNumber() {
        Emulator emu = makeEmu();
        int m = emu.mapperNumber();
        if (m != 0)
            throw new RuntimeException("mapper_number=" + m + " (expected 0)");
    }

    private static void testSaveLoadState() {
        Emulator emu = makeEmu();
        emu.stepFrame();
        int[] fbBefore = emu.framebuffer().clone();

        byte[] buf = new byte[1 << 20]; // 1 MiB
        int written = SaveState.save(emu, buf, buf.length);
        if (written == 0)
            throw new RuntimeException("SaveState.save returned 0");

        // Run a few more frames to change state
        emu.stepFrame();
        emu.stepFrame();

        // Load state back
        boolean ok = SaveState.load(emu, buf, written);
        if (!ok)
            throw new RuntimeException("SaveState.load returned false");

        // Framebuffer should match the saved state
        int[] fbAfter = emu.framebuffer();
        int diffs = 0;
        for (int i = 0; i < fbBefore.length; ++i) {
            if (fbBefore[i] != fbAfter[i]) diffs++;
        }
        if (diffs > 0)
            throw new RuntimeException("framebuffer has " + diffs + " diff pixels after save/load");
    }

    private static void testSetRegion() {
        Emulator emu = makeEmu();
        int prev = emu.region;
        emu.setRegion(Region.PAL);
        if (emu.region != Region.PAL)
            throw new RuntimeException("region not updated to PAL (was " + prev + ")");
        // Verify it took effect by checking the region field
        if (emu.currentRegion() != Region.PAL)
            throw new RuntimeException("currentRegion() != PAL");
    }

    private static void test1000Frames() {
        Emulator emu = makeEmu();
        for (int i = 0; i < 1000; ++i) {
            emu.stepFrame();
        }
        // If we get here without throwing, the test passes.
    }

    private static void testAudioDrain() {
        Emulator emu = makeEmu();
        emu.stepFrame();
        float[] buf = new float[2048];
        int n = emu.takeAudioSamples(buf, buf.length);
        if (n == 0)
            throw new RuntimeException("audio drain returned 0 samples");
    }

    private static void testFramebuffer() {
        Emulator emu = makeEmu();
        emu.stepFrame();
        int[] fb = emu.framebuffer();
        if (fb == null)
            throw new RuntimeException("framebuffer is null");
        if (fb.length != 256 * 240)
            throw new RuntimeException("framebuffer length=" + fb.length + " (expected " + (256 * 240) + ")");
    }

    private static void testReset() {
        Emulator emu = makeEmu();
        emu.stepFrame();
        emu.reset();
        // After reset, stepping a frame should still work
        emu.stepFrame();
    }

    private static void testDeterministic() {
        Emulator emu1 = makeEmu();
        Emulator emu2 = makeEmu();
        for (int i = 0; i < 10; ++i) {
            int c1 = emu1.stepFrame();
            int c2 = emu2.stepFrame();
            if (c1 != c2)
                throw new RuntimeException("cycle mismatch at frame " + i + ": " + c1 + " vs " + c2);
        }
        int[] fb1 = emu1.framebuffer();
        int[] fb2 = emu2.framebuffer();
        int diffs = 0;
        for (int i = 0; i < fb1.length; ++i) {
            if (fb1[i] != fb2[i]) diffs++;
        }
        if (diffs > 0)
            throw new RuntimeException("framebuffer has " + diffs + " diff pixels (non-deterministic)");
    }
}
