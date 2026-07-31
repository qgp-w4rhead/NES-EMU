package nes.core.mappers;

// YM2413 OPLL FM synthesiser for VRC7 (mapper 85 audio).
// Port of cores/csharp/src/Audio/Opll.cs. Compact, deterministic 2-operator FM synth.
public final class Opll {
    public static final int SINE_LEN = 1024;
    public static final int SINE_QUARTER = 256;
    public static final int CHANNELS = 6;
    public static final int ENV_ATTACK = 0;
    public static final int ENV_DECAY = 1;
    public static final int ENV_SUSTAIN = 2;
    public static final int ENV_RELEASE = 3;
    public static final int ENV_OFF = 4;

    public static final class Patch {
        public int mult, tl, fb, ar, dr, sl, rr, kl, am, wf;
    }

    public static final class ChanReg {
        public int fnum;
        public int block;
        public int volume;
        public int instrument;
        public boolean keyOn;
    }

    public static final class Operator {
        public int phase;
        public int envPhase;
        public int envAmp;
        public int level;
    }

    public byte[] regs = new byte[128];
    public byte addrLatch;
    public Patch[] patches = new Patch[16];
    public ChanReg[] chan = new ChanReg[CHANNELS];
    public Operator[][] ops = new Operator[CHANNELS][];
    public byte[] userInstBuf = new byte[8];
    public int[] sine = new int[SINE_QUARTER];

    public Opll() {
        for (int i = 0; i < CHANNELS; ++i) {
            ops[i] = new Operator[2];
            ops[i][0] = new Operator();
            ops[i][1] = new Operator();
            chan[i] = new ChanReg();
        }
        for (int i = 0; i < 16; ++i) patches[i] = new Patch();
        init();
    }

    private static Patch makePatch(Patch p, int mult, int tl, int fb, int ar, int dr, int sl, int rr, int kl, int am, int wf) {
        p.mult = mult; p.tl = tl; p.fb = fb; p.ar = ar; p.dr = dr; p.sl = sl; p.rr = rr; p.kl = kl; p.am = am; p.wf = wf;
        return p;
    }

    private static Patch patchFromRegs(byte[] b) {
        Patch p = new Patch();
        p.mult = (b[0] & 0xFF) & 0x0F;
        p.tl = (((b[0] & 0xFF) >> 4) & 0x0F) | (((b[2] & 0xFF) & 0x03) << 4);
        p.fb = (b[1] & 0xFF) & 0x07;
        p.ar = ((b[2] & 0xFF) >> 2) & 0x0F;
        p.dr = (b[3] & 0xFF) & 0x0F;
        p.sl = ((b[3] & 0xFF) >> 4) & 0x0F;
        p.rr = (b[4] & 0xFF) & 0x0F;
        p.kl = ((b[4] & 0xFF) >> 4) & 0x03;
        p.am = (b[5] & 0xFF) & 0x07;
        p.wf = (b[7] & 0xFF) & 0x03;
        return p;
    }

    private void buildSine() {
        for (int i = 0; i < SINE_QUARTER; ++i) {
            double s = Math.sin((double) i / ((double) SINE_LEN / 4.0) * 1.5707963267948966);
            sine[i] = (int) (s * 4095.0 + 0.5);
        }
    }

    public void init() {
        makePatch(patches[0], 1, 24, 0, 15, 7, 3, 10, 0, 0, 0);
        makePatch(patches[1], 1, 16, 2, 15, 6, 5, 8, 1, 0, 0);
        makePatch(patches[2], 1, 8, 5, 15, 8, 5, 8, 0, 0, 0);
        makePatch(patches[3], 1, 12, 0, 15, 7, 5, 8, 1, 0, 0);
        makePatch(patches[4], 1, 8, 0, 15, 5, 3, 8, 1, 0, 0);
        makePatch(patches[5], 1, 16, 3, 15, 7, 5, 8, 1, 0, 0);
        makePatch(patches[6], 2, 16, 3, 15, 8, 5, 8, 1, 0, 0);
        makePatch(patches[7], 1, 24, 0, 15, 7, 3, 10, 0, 0, 0);
        makePatch(patches[8], 1, 12, 1, 15, 5, 4, 8, 1, 0, 0);
        makePatch(patches[9], 1, 8, 4, 15, 8, 5, 8, 0, 0, 0);
        makePatch(patches[10], 1, 16, 0, 15, 5, 3, 8, 1, 0, 0);
        makePatch(patches[11], 1, 8, 3, 15, 7, 5, 8, 1, 0, 0);
        makePatch(patches[12], 1, 16, 0, 15, 5, 3, 8, 1, 0, 0);
        makePatch(patches[13], 1, 16, 0, 15, 5, 3, 8, 1, 0, 0);
        makePatch(patches[14], 1, 16, 0, 15, 5, 3, 8, 1, 0, 0);
        buildSine();
    }

    public void writeAddr(int addr) { addrLatch = (byte) (addr & 0x7F); }

    private void keyOn(int ch) {
        for (int i = 0; i < 2; ++i) {
            ops[ch][i].envPhase = ENV_ATTACK;
            ops[ch][i].envAmp = 0;
            ops[ch][i].phase = 0;
        }
    }

    private void keyOff(int ch) {
        for (int i = 0; i < 2; ++i)
            ops[ch][i].envPhase = ENV_RELEASE;
    }

    private void applyReg(int addr, int value) {
        if (addr >= 0x10 && addr <= 0x17) {
            int i = addr - 0x10;
            userInstBuf[i] = (byte) value;
            if (i == 7) patches[15] = patchFromRegs(userInstBuf);
        } else if (addr >= 0x30 && addr <= 0x35) {
            int ch = addr - 0x30;
            chan[ch].volume = value & 0x0F;
            chan[ch].instrument = (value >> 4) & 0x0F;
        } else if (addr >= 0x20 && addr <= 0x25) {
            int ch = addr - 0x20;
            chan[ch].fnum = (chan[ch].fnum & 0x0100) | value;
        } else if (addr >= 0x40 && addr <= 0x45) {
            int ch = addr - 0x40;
            boolean wasOn = chan[ch].keyOn;
            chan[ch].fnum = (chan[ch].fnum & 0x00FF) | ((value & 0x01) << 8);
            chan[ch].block = (value >> 1) & 0x07;
            chan[ch].keyOn = (value & 0x10) != 0;
            boolean nowOn = chan[ch].keyOn;
            if (nowOn && !wasOn) keyOn(ch);
            else if (!nowOn && wasOn) keyOff(ch);
        }
    }

    public void writeData(int value) {
        int addr = addrLatch & 0xFF;
        if (addr >= 0x80) return;
        regs[addr] = (byte) value;
        applyReg(addr, value);
    }

    private static int phaseInc(ChanReg ch, int mult) {
        int m = (mult == 0) ? 1 : mult;
        return (ch.fnum * m) << ch.block;
    }

    private static int envStep(int rate) {
        if (rate == 0) return 0;
        return rate << 3;
    }

    private void clockEnv(Operator op, Patch patch, boolean isCarrier, int chVol) {
        int sl = patch.sl << 7;
        final int MAX = 4095;
        switch (op.envPhase) {
            case ENV_OFF: op.envAmp = MAX; break;
            case ENV_ATTACK:
                op.envAmp -= envStep(patch.ar) * 4;
                if (op.envAmp <= 0) { op.envAmp = 0; op.envPhase = ENV_DECAY; }
                break;
            case ENV_DECAY:
                op.envAmp += envStep(patch.dr);
                if (op.envAmp >= sl) { op.envAmp = sl; op.envPhase = ENV_SUSTAIN; }
                break;
            case ENV_SUSTAIN: break;
            case ENV_RELEASE:
                op.envAmp += envStep(patch.rr) * 2;
                if (op.envAmp >= MAX) { op.envAmp = MAX; op.envPhase = ENV_OFF; }
                break;
        }
        int ea = op.envAmp;
        if (ea < 0) ea = 0;
        if (ea > MAX) ea = MAX;
        int level = (MAX - ea) >> 2;
        if (isCarrier) {
            int tl = chVol + (patch.tl >> 2);
            if (tl > 63) tl = 63;
            level = level - tl * 16;
            if (level < 0) level = 0;
        } else {
            level = level - patch.tl * 16;
            if (level < 0) level = 0;
        }
        op.level = level;
    }

    private int sin(int phase) {
        int idx = (phase >>> 10) & 0x3FF;
        int quad;
        int intra;
        switch (idx >>> 8) {
            case 0: quad = 0; intra = idx; break;
            case 1: quad = 1; intra = 0x3FF - idx; break;
            case 2: quad = 2; intra = idx - 0x200; break;
            default: quad = 3; intra = 0x3FF - (idx - 0x200); break;
        }
        int i = intra >>> 2;
        if (i > SINE_LEN / 4) i = SINE_LEN / 4;
        int v = sine[i];
        return (quad == 1 || quad == 3) ? -v : v;
    }

    public void clock(int apuCycles) {
        for (int c = 0; c < apuCycles; ++c) {
            for (int ch = 0; ch < CHANNELS; ++ch) {
                Patch patch = patches[chan[ch].instrument];
                int chVol = chan[ch].volume;
                int pincMod = phaseInc(chan[ch], patch.mult);
                int pincCar = phaseInc(chan[ch], 1);
                ops[ch][0].phase += pincMod;
                clockEnv(ops[ch][0], patch, false, chVol);
                ops[ch][1].phase += pincCar;
                clockEnv(ops[ch][1], patch, true, chVol);
            }
        }
    }

    public float sample() {
        int sum = 0;
        for (int ch = 0; ch < CHANNELS; ++ch) {
            if (!chan[ch].keyOn && ops[ch][1].envPhase == ENV_OFF) continue;
            Patch patch = patches[chan[ch].instrument];
            int fb = (patch.fb != 0) ? (ops[ch][0].level >> (7 - patch.fb)) : 0;
            int modOut = sin(ops[ch][0].phase + fb * 64) * ops[ch][0].level / 4095;
            int carPhase = ops[ch][1].phase + modOut * 16;
            int car = sin(carPhase) * ops[ch][1].level / 4095;
            sum += car;
        }
        float s = sum / ((float) CHANNELS * 4095.0f);
        if (s < -1.0f) s = -1.0f;
        if (s > 1.0f) s = 1.0f;
        return s;
    }
}
