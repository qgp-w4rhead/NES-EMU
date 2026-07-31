package nes.core;

// Full emulator state serialization (port of cores/csharp/src/SaveState.cs).
// Custom binary format with magic + version tag, manual byte-array writing
// (GraalVM Native Image compatible, no reflection, no stream overhead).
//
// Layout (all integers little-endian):
//   [4] magic "NESS" (0x5353454E LE)
//   [4] version (u32 = SAVE_STATE_VERSION)
//   [4] total_payload_size (u32)
//   --- payload ---
//   CPU: A, X, Y, Sp, Pc, Status, Flags (8 bytes)
//   RAM (2048 bytes)
//   PPU arch state
//   APU open-bus (24 bytes)
//   APU state (all channels + frame counter + mixer)
//   Joypad state
//   Cartridge present flag + header + mapper state
//   Emulator-level: dma_stall, cpu_cycle_count, sample_accumulator,
//     audio_buffer_count, audio_buffer, ppu_cycle_carry, region
public final class SaveState {
    public static final int MAGIC = 0x5353454E; // "NESS" LE
    public static final int VERSION = 1;
    public static final int HEADER_SIZE = 12;

    private static final int CPU_STATE_SIZE = 9;
    private static final int INES_HEADER_SIZE = 12;
    private static final int PULSE_STATE_SIZE = 20;
    private static final int TRIANGLE_STATE_SIZE = 12;
    private static final int NOISE_STATE_SIZE = 17;
    private static final int DMC_STATE_SIZE = 22;
    private static final int APU_LEVEL_SIZE = 48;
    private static final int APU_STATE_SIZE = PULSE_STATE_SIZE * 2 + TRIANGLE_STATE_SIZE + NOISE_STATE_SIZE + DMC_STATE_SIZE + APU_LEVEL_SIZE;
    private static final int JOYPAD_STATE_SIZE = 7;

    private SaveState() {}

    // ---- Required-size computation ----
    public static int requiredSize(Emulator emu) {
        int payload = 0;
        payload += CPU_STATE_SIZE;
        payload += Bus.RAM_SIZE;
        payload += ppuStateSize(emu.bus.ppu);
        payload += Bus.APU_IO_REG_COUNT;
        payload += APU_STATE_SIZE;
        payload += JOYPAD_STATE_SIZE;
        if (emu.cartridge != null)
            payload += 1 + INES_HEADER_SIZE + emu.cartridge.saveState(null);
        else
            payload += 1;
        payload += 4 + 8 + 4 + 4 + emu.audioBufferCount * 4 + 4 + 4;
        return HEADER_SIZE + payload;
    }

    private static int ppuStateSize(Ppu p) {
        return 4 + p.vramSize + Ppu.OAM_SIZE + Ppu.PALETTE_SIZE
             + 1 + 1 + 1 + 1 + 2 + 2 + 1 + 1 + 1 + 1 + 2 + 2 + 1 + 4 + 4;
    }

    // ---- LE write helpers ----
    private static void putU32(byte[] b, int off, int v) {
        b[off] = (byte) v; b[off+1] = (byte)(v >> 8); b[off+2] = (byte)(v >> 16); b[off+3] = (byte)(v >> 24);
    }
    private static int getU32(byte[] b, int off) {
        return (b[off] & 0xFF) | ((b[off+1] & 0xFF) << 8) | ((b[off+2] & 0xFF) << 16) | ((b[off+3] & 0xFF) << 24);
    }
    private static void putU64(byte[] b, int off, long v) {
        for (int i = 0; i < 8; ++i) b[off+i] = (byte)(v >>> (i * 8));
    }
    private static long getU64(byte[] b, int off) {
        long v = 0;
        for (int i = 0; i < 8; ++i) v |= ((long)(b[off+i] & 0xFF)) << (i * 8);
        return v;
    }
    private static void putU16(byte[] b, int off, int v) {
        b[off] = (byte) v; b[off+1] = (byte)(v >> 8);
    }
    private static int getU16(byte[] b, int off) {
        return (b[off] & 0xFF) | ((b[off+1] & 0xFF) << 8);
    }
    private static void putFloat(byte[] b, int off, float f) {
        putU32(b, off, Float.floatToRawIntBits(f));
    }
    private static float getFloat(byte[] b, int off) {
        return Float.intBitsToFloat(getU32(b, off));
    }

    // ---- Save ----
    public static int save(Emulator emu, byte[] buf, int cap) {
        if (emu == null) return 0;
        int required = requiredSize(emu);
        if (buf == null) return required;
        if (cap < required) return 0;

        int p = 0;
        int payload = required - HEADER_SIZE;
        putU32(buf, p, MAGIC); p += 4;
        putU32(buf, p, VERSION); p += 4;
        putU32(buf, p, payload); p += 4;

        // CPU
        buf[p++] = (byte) emu.cpu.a;
        buf[p++] = (byte) emu.cpu.x;
        buf[p++] = (byte) emu.cpu.y;
        buf[p++] = (byte) emu.cpu.sp;
        putU16(buf, p, emu.cpu.pc); p += 2;
        buf[p++] = (byte) emu.cpu.status;
        buf[p++] = (byte) emu.cpu.flags;
        buf[p++] = 0; // pad

        // RAM
        System.arraycopy(emu.bus.ram, 0, buf, p, Bus.RAM_SIZE);
        p += Bus.RAM_SIZE;

        // PPU arch state
        p = writePpuState(buf, p, emu.bus.ppu);

        // APU open-bus
        System.arraycopy(emu.bus.apuOpenBus, 0, buf, p, Bus.APU_IO_REG_COUNT);
        p += Bus.APU_IO_REG_COUNT;

        // APU state
        p = writeApuState(buf, p, emu.bus.apu);

        // Joypad
        p = writeJoypadState(buf, p, emu.bus.joypad);

        // Cartridge
        if (emu.cartridge != null) {
            buf[p++] = 1;
            p = writeInesHeader(buf, p, emu.cartridge.header);
            int msz = emu.cartridge.saveState(null);
            if (msz > 0) {
                byte[] mb = new byte[msz];
                emu.cartridge.saveState(mb);
                System.arraycopy(mb, 0, buf, p, msz);
                p += msz;
            }
        } else {
            buf[p++] = 0;
        }

        // Emulator-level
        putU32(buf, p, emu.bus.dmaStallCycles); p += 4;
        putU64(buf, p, emu.bus.cpuCycleCount); p += 8;
        putFloat(buf, p, emu.sampleAccumulator); p += 4;
        putU32(buf, p, emu.audioBufferCount); p += 4;
        for (int i = 0; i < emu.audioBufferCount; ++i) {
            putFloat(buf, p, emu.audioBuffer[i]); p += 4;
        }
        putU32(buf, p, emu.ppuCycleCarry); p += 4;
        putU32(buf, p, emu.region); p += 4;

        return p;
    }

    // ---- Load ----
    public static boolean load(Emulator emu, byte[] buf, int len) {
        if (emu == null || buf == null || len < HEADER_SIZE) return false;

        int p = 0;
        int end = len;

        int magic = getU32(buf, p); p += 4;
        if (magic != MAGIC) return false;
        int version = getU32(buf, p); p += 4;
        if (version != VERSION) return false;
        int payload = getU32(buf, p); p += 4;
        if (payload + HEADER_SIZE > len) return false;
        end = HEADER_SIZE + payload;

        // CPU
        if (p + CPU_STATE_SIZE > end) return false;
        emu.cpu.a = buf[p++] & 0xFF;
        emu.cpu.x = buf[p++] & 0xFF;
        emu.cpu.y = buf[p++] & 0xFF;
        emu.cpu.sp = buf[p++] & 0xFF;
        emu.cpu.pc = getU16(buf, p); p += 2;
        emu.cpu.status = buf[p++] & 0xFF;
        emu.cpu.flags = buf[p++] & 0xFF;
        p++; // pad

        // RAM
        if (p + Bus.RAM_SIZE > end) return false;
        System.arraycopy(buf, p, emu.bus.ram, 0, Bus.RAM_SIZE);
        p += Bus.RAM_SIZE;

        // PPU arch state
        int[] pp = new int[1];
        pp[0] = p;
        if (!readPpuState(buf, end, pp, emu.bus.ppu)) return false;
        p = pp[0];

        // APU open-bus
        if (p + Bus.APU_IO_REG_COUNT > end) return false;
        System.arraycopy(buf, p, emu.bus.apuOpenBus, 0, Bus.APU_IO_REG_COUNT);
        p += Bus.APU_IO_REG_COUNT;

        // APU state
        pp[0] = p;
        if (!readApuState(buf, end, pp, emu.bus.apu)) return false;
        p = pp[0];

        // Joypad
        pp[0] = p;
        if (!readJoypadState(buf, end, pp, emu.bus.joypad)) return false;
        p = pp[0];

        // Cartridge
        if (p + 1 > end) return false;
        int cartPresent = buf[p++] & 0xFF;
        if (cartPresent != 0) {
            if (p + INES_HEADER_SIZE > end) return false;
            InesHeader hdr = readInesHeader(buf, p); p += INES_HEADER_SIZE;
            if (emu.cartridge == null) return false;
            if (hdr.mapperNumber != emu.cartridge.header.mapperNumber) return false;
            emu.cartridge.header = hdr;
            int msz = emu.cartridge.saveState(null);
            if (p + msz > end) return false;
            byte[] mb = new byte[msz];
            System.arraycopy(buf, p, mb, 0, msz);
            p += msz;
            if (!emu.cartridge.loadState(mb, msz)) return false;
            emu.bus.ppu.setMirroring(emu.cartridge.mirrorMode());
            emu.bus.insertCartridge(emu.cartridge);
        } else {
            emu.bus.removeCartridge();
        }

        // Emulator-level
        if (p + 4 > end) return false;
        emu.bus.dmaStallCycles = getU32(buf, p); p += 4;
        if (p + 8 > end) return false;
        emu.bus.cpuCycleCount = getU64(buf, p); p += 8;
        if (p + 4 > end) return false;
        emu.sampleAccumulator = getFloat(buf, p); p += 4;
        if (p + 4 > end) return false;
        int audioCount = getU32(buf, p); p += 4;
        if (audioCount > (end - p) / 4) return false;
        float[] audio = new float[audioCount];
        for (int i = 0; i < audioCount; ++i) { audio[i] = getFloat(buf, p); p += 4; }
        emu.setAudioBuffer(audio, audioCount);
        if (p + 4 > end) return false;
        emu.ppuCycleCarry = getU32(buf, p); p += 4;
        if (p + 4 > end) return false;
        int regn = getU32(buf, p); p += 4;
        if (regn <= Region.DENDY) emu.setRegion(regn);

        // Clear framebuffer to universal bg (derived data)
        int universalBg = PpuRender.universalBgArgb(emu.bus.ppu);
        PpuRender.clearFramebuffer(emu.bus.ppu, universalBg);

        return true;
    }

    // ---- PPU state ----
    private static int writePpuState(byte[] b, int p, Ppu pp) {
        putU32(b, p, pp.vramSize); p += 4;
        System.arraycopy(pp.vram, 0, b, p, pp.vramSize);
        p += pp.vramSize;
        System.arraycopy(pp.oam, 0, b, p, Ppu.OAM_SIZE);
        p += Ppu.OAM_SIZE;
        System.arraycopy(pp.palette, 0, b, p, Ppu.PALETTE_SIZE);
        p += Ppu.PALETTE_SIZE;
        b[p++] = (byte) pp.ppuCtrl;
        b[p++] = (byte) pp.ppuMask;
        b[p++] = (byte) pp.oamAddr;
        b[p++] = (byte) pp.ppuStatus;
        putU16(b, p, pp.v); p += 2;
        putU16(b, p, pp.t); p += 2;
        b[p++] = (byte) pp.fineX;
        b[p++] = (byte)(pp.w ? 1 : 0);
        b[p++] = (byte) pp.ppuDataBuffer;
        b[p++] = (byte) pp.openBus;
        putU16(b, p, pp.scanline); p += 2;
        putU16(b, p, pp.cycle); p += 2;
        b[p++] = (byte)(pp.nmiRequest ? 1 : 0);
        putU32(b, p, pp.mirroring); p += 4;
        putU32(b, p, pp.region); p += 4;
        return p;
    }

    private static boolean readPpuState(byte[] b, int end, int[] pp, Ppu ppu) {
        int p = pp[0];
        if (p + 4 > end) return false;
        int vramSize = getU32(b, p); p += 4;
        if (vramSize > Ppu.VRAM_SIZE_4K) return false;
        int ppuNeed = 4 + vramSize + Ppu.OAM_SIZE + Ppu.PALETTE_SIZE
                     + 1 + 1 + 1 + 1 + 2 + 2 + 1 + 1 + 1 + 1 + 2 + 2 + 1 + 4 + 4;
        if (p + ppuNeed - 4 > end) return false;

        byte[] vram = new byte[vramSize];
        System.arraycopy(b, p, vram, 0, vramSize);
        p += vramSize;

        byte[] oam = new byte[Ppu.OAM_SIZE];
        System.arraycopy(b, p, oam, 0, Ppu.OAM_SIZE);
        p += Ppu.OAM_SIZE;

        byte[] palette = new byte[Ppu.PALETTE_SIZE];
        System.arraycopy(b, p, palette, 0, Ppu.PALETTE_SIZE);
        p += Ppu.PALETTE_SIZE;

        int ppuctrl = b[p++] & 0xFF;
        int ppumask = b[p++] & 0xFF;
        int oamaddr = b[p++] & 0xFF;
        int ppustatus = b[p++] & 0xFF;
        int v = getU16(b, p); p += 2;
        int t = getU16(b, p); p += 2;
        int fineX = b[p++] & 0xFF;
        boolean wFlag = (b[p++] & 0xFF) != 0;
        int ppudataBuffer = b[p++] & 0xFF;
        int openBus = b[p++] & 0xFF;
        int scanline = getU16(b, p); p += 2;
        int cycle = getU16(b, p); p += 2;
        boolean nmiRequest = (b[p++] & 0xFF) != 0;
        int mirroring = getU32(b, p); p += 4;
        int region = getU32(b, p); p += 4;

        if (mirroring <= Mirroring.SINGLE_SCREEN3) ppu.setMirroring(mirroring);
        if (region <= Region.DENDY) ppu.setRegion(region);
        System.arraycopy(vram, 0, ppu.vram, 0, vramSize);
        ppu.vramSize = vramSize;
        System.arraycopy(oam, 0, ppu.oam, 0, Ppu.OAM_SIZE);
        System.arraycopy(palette, 0, ppu.palette, 0, Ppu.PALETTE_SIZE);
        ppu.ppuCtrl = ppuctrl;
        ppu.ppuMask = ppumask;
        ppu.oamAddr = oamaddr;
        ppu.ppuStatus = ppustatus;
        ppu.v = v;
        ppu.t = t;
        ppu.fineX = fineX;
        ppu.w = wFlag;
        ppu.ppuDataBuffer = ppudataBuffer;
        ppu.openBus = openBus;
        ppu.scanline = scanline;
        ppu.cycle = cycle;
        ppu.nmiRequest = nmiRequest;
        pp[0] = p;
        return true;
    }

    // ---- Pulse channel state ----
    private static int writePulseState(byte[] b, int p, PulseChannel ch) {
        b[p++] = (byte) ch.duty;
        b[p++] = (byte)(ch.halt ? 1 : 0);
        b[p++] = (byte)(ch.constantVolume ? 1 : 0);
        b[p++] = (byte) ch.volume;
        b[p++] = (byte)(ch.sweepEnabled ? 1 : 0);
        b[p++] = (byte) ch.sweepPeriod;
        b[p++] = (byte)(ch.sweepNegate ? 1 : 0);
        b[p++] = (byte) ch.sweepShift;
        b[p++] = (byte) ch.sweepDivider;
        b[p++] = (byte)(ch.sweepReload ? 1 : 0);
        putU16(b, p, ch.timerPeriod); p += 2;
        putU16(b, p, ch.timer); p += 2;
        b[p++] = (byte) ch.sequence;
        b[p++] = (byte) ch.lengthCounter;
        b[p++] = (byte)(ch.enabled ? 1 : 0);
        b[p++] = (byte) ch.envelopeDivider;
        b[p++] = (byte) ch.envelopeDecay;
        b[p++] = (byte)(ch.envelopeStart ? 1 : 0);
        return p;
    }

    private static boolean readPulseState(byte[] b, int end, int[] pp, PulseChannel ch) {
        int p = pp[0];
        if (p + PULSE_STATE_SIZE > end) return false;
        ch.duty = b[p++] & 0xFF;
        ch.halt = (b[p++] & 0xFF) != 0;
        ch.constantVolume = (b[p++] & 0xFF) != 0;
        ch.volume = b[p++] & 0xFF;
        ch.sweepEnabled = (b[p++] & 0xFF) != 0;
        ch.sweepPeriod = b[p++] & 0xFF;
        ch.sweepNegate = (b[p++] & 0xFF) != 0;
        ch.sweepShift = b[p++] & 0xFF;
        ch.sweepDivider = b[p++] & 0xFF;
        ch.sweepReload = (b[p++] & 0xFF) != 0;
        ch.timerPeriod = getU16(b, p); p += 2;
        ch.timer = getU16(b, p); p += 2;
        ch.sequence = b[p++] & 0xFF;
        ch.lengthCounter = b[p++] & 0xFF;
        ch.enabled = (b[p++] & 0xFF) != 0;
        ch.envelopeDivider = b[p++] & 0xFF;
        ch.envelopeDecay = b[p++] & 0xFF;
        ch.envelopeStart = (b[p++] & 0xFF) != 0;
        pp[0] = p;
        return true;
    }

    // ---- Triangle channel state ----
    private static int writeTriangleState(byte[] b, int p, TriangleChannel ch) {
        b[p++] = (byte)(ch.halt ? 1 : 0);
        b[p++] = (byte) ch.linearReload;
        b[p++] = (byte) ch.linearCounter;
        putU16(b, p, ch.timerPeriod); p += 2;
        putU16(b, p, ch.timer); p += 2;
        b[p++] = (byte) ch.sequence;
        b[p++] = (byte) ch.lengthCounter;
        b[p++] = (byte)(ch.enabled ? 1 : 0);
        b[p++] = (byte)(ch.linearStart ? 1 : 0);
        b[p++] = 0; // pad
        return p;
    }

    private static boolean readTriangleState(byte[] b, int end, int[] pp, TriangleChannel ch) {
        int p = pp[0];
        if (p + TRIANGLE_STATE_SIZE > end) return false;
        ch.halt = (b[p++] & 0xFF) != 0;
        ch.linearReload = b[p++] & 0xFF;
        ch.linearCounter = b[p++] & 0xFF;
        ch.timerPeriod = getU16(b, p); p += 2;
        ch.timer = getU16(b, p); p += 2;
        ch.sequence = b[p++] & 0xFF;
        ch.lengthCounter = b[p++] & 0xFF;
        ch.enabled = (b[p++] & 0xFF) != 0;
        ch.linearStart = (b[p++] & 0xFF) != 0;
        p++; // pad
        pp[0] = p;
        return true;
    }

    // ---- Noise channel state ----
    private static int writeNoiseState(byte[] b, int p, NoiseChannel ch) {
        b[p++] = (byte)(ch.halt ? 1 : 0);
        b[p++] = (byte)(ch.constantVolume ? 1 : 0);
        b[p++] = (byte) ch.volume;
        b[p++] = (byte)(ch.mode ? 1 : 0);
        b[p++] = (byte) ch.periodIndex;
        putU16(b, p, ch.timerPeriod); p += 2;
        putU16(b, p, ch.timer); p += 2;
        putU16(b, p, ch.lfsr); p += 2;
        b[p++] = (byte) ch.lengthCounter;
        b[p++] = (byte)(ch.enabled ? 1 : 0);
        b[p++] = (byte) ch.envelopeDivider;
        b[p++] = (byte) ch.envelopeDecay;
        b[p++] = (byte)(ch.envelopeStart ? 1 : 0);
        b[p++] = 0; // pad
        return p;
    }

    private static boolean readNoiseState(byte[] b, int end, int[] pp, NoiseChannel ch) {
        int p = pp[0];
        if (p + NOISE_STATE_SIZE > end) return false;
        ch.halt = (b[p++] & 0xFF) != 0;
        ch.constantVolume = (b[p++] & 0xFF) != 0;
        ch.volume = b[p++] & 0xFF;
        ch.mode = (b[p++] & 0xFF) != 0;
        ch.periodIndex = b[p++] & 0xFF;
        ch.timerPeriod = getU16(b, p); p += 2;
        ch.timer = getU16(b, p); p += 2;
        ch.lfsr = getU16(b, p); p += 2;
        ch.lengthCounter = b[p++] & 0xFF;
        ch.enabled = (b[p++] & 0xFF) != 0;
        ch.envelopeDivider = b[p++] & 0xFF;
        ch.envelopeDecay = b[p++] & 0xFF;
        ch.envelopeStart = (b[p++] & 0xFF) != 0;
        p++; // pad
        pp[0] = p;
        return true;
    }

    // ---- DMC channel state ----
    private static int writeDmcState(byte[] b, int p, DmcChannel ch) {
        b[p++] = (byte)(ch.irqEnable ? 1 : 0);
        b[p++] = (byte)(ch.loopFlag ? 1 : 0);
        b[p++] = (byte) ch.rateIndex;
        putU16(b, p, ch.timerPeriod); p += 2;
        putU16(b, p, ch.timer); p += 2;
        b[p++] = (byte) ch.outputCounter;
        b[p++] = (byte) ch.sampleBuffer;
        b[p++] = (byte) ch.bufferBits;
        putU16(b, p, ch.sampleAddrBase); p += 2;
        putU16(b, p, ch.sampleAddress); p += 2;
        putU16(b, p, ch.sampleLength); p += 2;
        putU16(b, p, ch.bytesRemaining); p += 2;
        b[p++] = (byte)(ch.enabled ? 1 : 0);
        b[p++] = (byte)(ch.irqFlag ? 1 : 0);
        b[p++] = 0; // pad
        b[p++] = 0; // pad
        return p;
    }

    private static boolean readDmcState(byte[] b, int end, int[] pp, DmcChannel ch) {
        int p = pp[0];
        if (p + DMC_STATE_SIZE > end) return false;
        ch.irqEnable = (b[p++] & 0xFF) != 0;
        ch.loopFlag = (b[p++] & 0xFF) != 0;
        ch.rateIndex = b[p++] & 0xFF;
        ch.timerPeriod = getU16(b, p); p += 2;
        ch.timer = getU16(b, p); p += 2;
        ch.outputCounter = b[p++] & 0xFF;
        ch.sampleBuffer = b[p++] & 0xFF;
        ch.bufferBits = b[p++] & 0xFF;
        ch.sampleAddrBase = getU16(b, p); p += 2;
        ch.sampleAddress = getU16(b, p); p += 2;
        ch.sampleLength = getU16(b, p); p += 2;
        ch.bytesRemaining = getU16(b, p); p += 2;
        ch.enabled = (b[p++] & 0xFF) != 0;
        ch.irqFlag = (b[p++] & 0xFF) != 0;
        p++; // pad
        p++; // pad
        pp[0] = p;
        return true;
    }

    // ---- APU state ----
    private static int writeApuState(byte[] b, int p, Apu a) {
        p = writePulseState(b, p, a.pulse1);
        p = writePulseState(b, p, a.pulse2);
        p = writeTriangleState(b, p, a.triangle);
        p = writeNoiseState(b, p, a.noise);
        p = writeDmcState(b, p, a.dmc);
        putU32(b, p, a.cycleAccumulator); p += 4;
        putFloat(b, p, a.sampleAccumulator); p += 4;
        putFloat(b, p, a.lpfPrev); p += 4;
        putFloat(b, p, a.dcPrevX); p += 4;
        putFloat(b, p, a.dcPrevY); p += 4;
        putFloat(b, p, a.mixAccumulator); p += 4;
        putU32(b, p, a.mixCount); p += 4;
        putFloat(b, p, a.lastDecimated); p += 4;
        putU32(b, p, a.frameCycle); p += 4;
        b[p++] = (byte)(a.frameMode5Step ? 1 : 0);
        b[p++] = (byte)(a.frameIrqInhibit ? 1 : 0);
        b[p++] = (byte)(a.frameIrq ? 1 : 0);
        putU32(b, p, a.frameResetDelay); p += 4;
        putU32(b, p, a.region); p += 4;
        b[p++] = (byte) a.selectedChannel;
        return p;
    }

    private static boolean readApuState(byte[] b, int end, int[] pp, Apu a) {
        int p = pp[0];
        if (!readPulseState(b, end, pp, a.pulse1)) return false;
        p = pp[0];
        if (!readPulseState(b, end, pp, a.pulse2)) return false;
        p = pp[0];
        if (!readTriangleState(b, end, pp, a.triangle)) return false;
        p = pp[0];
        if (!readNoiseState(b, end, pp, a.noise)) return false;
        p = pp[0];
        if (!readDmcState(b, end, pp, a.dmc)) return false;
        p = pp[0];
        if (p + APU_LEVEL_SIZE > end) return false;
        a.cycleAccumulator = getU32(b, p); p += 4;
        a.sampleAccumulator = getFloat(b, p); p += 4;
        a.lpfPrev = getFloat(b, p); p += 4;
        a.dcPrevX = getFloat(b, p); p += 4;
        a.dcPrevY = getFloat(b, p); p += 4;
        a.mixAccumulator = getFloat(b, p); p += 4;
        a.mixCount = getU32(b, p); p += 4;
        a.lastDecimated = getFloat(b, p); p += 4;
        a.frameCycle = getU32(b, p); p += 4;
        a.frameMode5Step = (b[p++] & 0xFF) != 0;
        a.frameIrqInhibit = (b[p++] & 0xFF) != 0;
        a.frameIrq = (b[p++] & 0xFF) != 0;
        a.frameResetDelay = getU32(b, p); p += 4;
        int regn = getU32(b, p); p += 4;
        if (regn <= Region.DENDY) a.region = regn;
        a.selectedChannel = b[p++] & 0xFF;
        pp[0] = p;
        return true;
    }

    // ---- Joypad state ----
    private static int writeJoypadState(byte[] b, int p, Joypad j) {
        b[p++] = j.current[0];
        b[p++] = j.current[1];
        b[p++] = (byte)(j.strobe ? 1 : 0);
        b[p++] = j.shift[0];
        b[p++] = j.shift[1];
        b[p++] = j.counter[0];
        b[p++] = j.counter[1];
        return p;
    }

    private static boolean readJoypadState(byte[] b, int end, int[] pp, Joypad j) {
        int p = pp[0];
        if (p + JOYPAD_STATE_SIZE > end) return false;
        j.current[0] = b[p++];
        j.current[1] = b[p++];
        j.strobe = (b[p++] & 0xFF) != 0;
        j.shift[0] = b[p++];
        j.shift[1] = b[p++];
        j.counter[0] = b[p++];
        j.counter[1] = b[p++];
        pp[0] = p;
        return true;
    }

    // ---- InesHeader ----
    private static int writeInesHeader(byte[] b, int p, InesHeader h) {
        b[p++] = (byte) h.prgRomBanks;
        b[p++] = (byte) h.chrRomBanks;
        putU16(b, p, h.mapperNumber); p += 2;
        b[p++] = (byte) h.mirroring;
        b[p++] = (byte)(h.hasTrainer ? 1 : 0);
        b[p++] = (byte)(h.hasBattery ? 1 : 0);
        b[p++] = (byte) h.tvSystem;
        b[p++] = 0; b[p++] = 0; b[p++] = 0; b[p++] = 0; // pad
        return p;
    }

    private static InesHeader readInesHeader(byte[] b, int p) {
        InesHeader h = new InesHeader();
        h.prgRomBanks = b[p++] & 0xFF;
        h.chrRomBanks = b[p++] & 0xFF;
        h.mapperNumber = getU16(b, p); p += 2;
        int mirr = b[p++] & 0xFF;
        h.mirroring = (mirr <= Mirroring.FOUR_SCREEN) ? mirr : Mirroring.HORIZONTAL;
        h.hasTrainer = (b[p++] & 0xFF) != 0;
        h.hasBattery = (b[p++] & 0xFF) != 0;
        h.tvSystem = b[p++] & 0xFF;
        return h;
    }
}

