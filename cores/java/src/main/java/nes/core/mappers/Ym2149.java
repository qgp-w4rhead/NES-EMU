package nes.core.mappers;

// YM2149 / AY-3-8910 PSG for Sunsoft 5B (mapper 69 audio).
// Port of cores/csharp/src/Audio/Ym2149.cs.
public final class Ym2149 {
    public byte[] regs = new byte[16];
    public byte addrLatch;
    public int[] toneTimer = new int[3];
    public byte[] toneOut = new byte[3];
    public int noiseTimer;
    public int noiseLfsr = 0x10000;
    public byte noiseOut;
    public int envTimer;
    public byte envPos;
    public boolean envHolding;

    public Ym2149() {}

    public void writeAddr(int addr) { addrLatch = (byte) (addr & 0x0F); }

    public void writeData(int value) {
        int a = addrLatch & 0xFF;
        if (a >= 16) return;
        regs[a] = (byte) value;
    }

    public int readData() {
        int a = addrLatch & 0xFF;
        if (a >= 16) return 0;
        return regs[a] & 0xFF;
    }

    private int tonePeriod(int ch) {
        int lo = regs[ch * 2] & 0xFF;
        int hi = (regs[ch * 2 + 1] & 0xFF) & 0x0F;
        int p = lo | (hi << 8);
        return p > 0 ? p : 1;
    }

    private int noisePeriod() {
        int p = (regs[6] & 0xFF) & 0x1F;
        return p > 0 ? p : 1;
    }

    private int envPeriod() {
        int lo = regs[0x0B] & 0xFF;
        int hi = regs[0x0C] & 0xFF;
        int p = lo | (hi << 8);
        return p > 0 ? p : 1;
    }

    private int envShape() { return (regs[0x0D] & 0xFF) & 0x0F; }

    private void applyShapeEnd() {
        int shape = envShape();
        boolean hold = (shape & 0x08) != 0;
        boolean alternate = (shape & 0x04) != 0;
        if (hold) {
            envHolding = true;
            if (alternate) envPos = (byte) (31 - (envPos & 0xFF));
        } else {
            envPos = 0;
        }
    }

    public void clock(int apuCycles) {
        for (int c = 0; c < apuCycles; ++c) {
            for (int ch = 0; ch < 3; ++ch) {
                if (toneTimer[ch] == 0) {
                    toneTimer[ch] = tonePeriod(ch);
                    toneOut[ch] ^= 1;
                } else {
                    toneTimer[ch]--;
                }
            }
            if (noiseTimer == 0) {
                noiseTimer = noisePeriod();
                int bit = (noiseLfsr ^ (noiseLfsr >> 3)) & 1;
                noiseLfsr = (noiseLfsr >> 1) | (bit << 16);
                noiseOut = (byte) (noiseLfsr & 1);
            } else {
                noiseTimer--;
            }
            if (!envHolding) {
                if (envTimer == 0) {
                    envTimer = envPeriod();
                    if ((envPos & 0xFF) < 31) envPos++;
                    else applyShapeEnd();
                } else {
                    envTimer--;
                }
            }
        }
    }

    private int envAmplitude() {
        int shape = envShape();
        boolean attack = (shape & 0x02) != 0;
        boolean alternate = (shape & 0x04) != 0;
        int pos = envPos & 0xFF;
        int half = pos / 2;
        if (half > 15) half = 15;
        int amp = attack ? half : (15 - half);
        if (pos >= 16) {
            if (alternate) {
                int second = (pos - 16) / 2;
                if (second > 15) second = 15;
                return attack ? (15 - second) : second;
            } else if (attack) {
                return 15;
            } else {
                return 0;
            }
        }
        return amp;
    }

    private int chanOut(int ch) {
        int disable = regs[7] & 0xFF;
        boolean toneEn = ((disable >> ch) & 1) == 0;
        boolean noiseEn = ((disable >> (ch + 3)) & 1) == 0;
        int v = regs[8 + ch] & 0xFF;
        int vol = v & 0x0F;
        boolean envMode = (v & 0x10) != 0;
        int amp = envMode ? envAmplitude() : vol;
        int tone = toneEn ? (toneOut[ch] & 0xFF) : 0;
        int noise = noiseEn ? (noiseOut & 0xFF) : 0;
        return ((tone | noise) != 0) ? amp : 0;
    }

    public float sample() {
        int a = chanOut(0);
        int b = chanOut(1);
        int c = chanOut(2);
        int sum = a + b + c;
        float s = sum / 45.0f;
        if (s < -1.0f) s = -1.0f;
        if (s > 1.0f) s = 1.0f;
        return s;
    }
}
