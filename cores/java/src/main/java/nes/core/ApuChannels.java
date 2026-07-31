package nes.core;

// APU channels: pulse, triangle, noise, DMC. Port of cores/csharp/src/Audio/ApuChannels.cs.

final class PulseChannel {
    public static final int MAX_PERIOD = 0x7FF;
    public static final int MIN_AUDIBLE_PERIOD = 8;

    public static final byte[][] DUTY_PATTERNS = {
        {0,1,0,0,0,0,0,0},
        {0,1,1,0,0,0,0,0},
        {0,1,1,1,1,0,0,0},
        {1,0,0,0,1,1,1,1},
    };

    public static final byte[] LENGTH_TABLE = {
        10,(byte)254,20,2,40,4,80,6,(byte)160,8,60,10,14,12,26,14,
        12,16,24,18,48,20,96,22,(byte)192,24,72,26,16,28,32,30,
    };

    public final boolean pulse2;
    public int duty;
    public boolean halt;
    public boolean constantVolume;
    public int volume;
    public boolean sweepEnabled;
    public int sweepPeriod;
    public boolean sweepNegate;
    public int sweepShift;
    public int sweepDivider;
    public boolean sweepReload;
    public int timerPeriod;
    public int timer;
    public int sequence;
    public int lengthCounter;
    public boolean enabled;
    public int envelopeDivider;
    public int envelopeDecay;
    public boolean envelopeStart;

    public PulseChannel(boolean p2) { pulse2 = p2; }

    public void writeRegister(int reg, int value) {
        switch (reg) {
            case 0:
                duty = (value >> 6) & 0x03;
                halt = (value & 0x20) != 0;
                constantVolume = (value & 0x10) != 0;
                volume = value & 0x0F;
                break;
            case 1:
                sweepEnabled = (value & 0x80) != 0;
                sweepPeriod = (value >> 4) & 0x07;
                sweepNegate = (value & 0x08) != 0;
                sweepShift = value & 0x07;
                sweepReload = true;
                break;
            case 2:
                timerPeriod = (timerPeriod & 0xFF00) | value;
                break;
            case 3: {
                int high = value & 0x07;
                timerPeriod = (timerPeriod & 0x00FF) | (high << 8);
                timer = (timer & 0x00FF) | (high << 8);
                if (enabled) lengthCounter = LENGTH_TABLE[(value >> 3) & 0x1F] & 0xFF;
                envelopeStart = true;
                sequence = 0;
                break;
            }
        }
    }

    public void setEnabled(boolean en) { enabled = en; if (!en) lengthCounter = 0; }

    public int sweepTarget() {
        if (!sweepEnabled || sweepShift == 0) return timerPeriod;
        int shifted = timerPeriod >> sweepShift;
        if (sweepNegate) {
            if (pulse2) return timerPeriod - shifted - 1;
            return timerPeriod - shifted;
        }
        return timerPeriod + shifted;
    }

    public boolean isMuted() { return timerPeriod < MIN_AUDIBLE_PERIOD || sweepTarget() > MAX_PERIOD; }

    public void tick() {
        if (timer == 0) { timer = timerPeriod; sequence = (sequence + 1) & 0x07; }
        else timer--;
    }

    public void clockEnvelope() {
        if (envelopeStart) { envelopeStart = false; envelopeDecay = 15; envelopeDivider = volume; }
        else if (envelopeDivider == 0) {
            envelopeDivider = volume;
            if (envelopeDecay > 0) envelopeDecay--;
            else if (halt) envelopeDecay = 15;
        } else envelopeDivider--;
    }

    private void clockLength() { if (!halt && lengthCounter > 0) lengthCounter--; }

    private void clockSweep() {
        boolean dividerZero = sweepDivider == 0;
        if (dividerZero && sweepEnabled && sweepShift != 0) {
            int target = sweepTarget();
            if (target <= MAX_PERIOD && timerPeriod >= MIN_AUDIBLE_PERIOD) timerPeriod = target;
        }
        if (dividerZero || sweepReload) { sweepDivider = sweepPeriod; sweepReload = false; }
        else sweepDivider--;
    }

    public void clockQuarterFrame() { clockEnvelope(); }
    public void clockHalfFrame() { clockLength(); clockSweep(); }

    public int sample() {
        if (!enabled || lengthCounter == 0 || isMuted()) return 0;
        int dutyBit = DUTY_PATTERNS[duty][sequence];
        if (dutyBit == 0) return 0;
        return constantVolume ? volume : envelopeDecay;
    }
}

final class TriangleChannel {
    public static final byte[] TRIANGLE_SEQUENCE = {
        15,14,13,12,11,10,9,8,7,6,5,4,3,2,1,0,
        0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,
    };

    public boolean halt;
    public int linearReload;
    public int linearCounter;
    public boolean linearStart;
    public int timerPeriod;
    public int timer;
    public int sequence;
    public int lengthCounter;
    public boolean enabled;

    public void writeRegister(int reg, int value) {
        switch (reg) {
            case 0: halt = (value & 0x80) != 0; linearReload = value & 0x7F; break;
            case 1: break;
            case 2: timerPeriod = (timerPeriod & 0xFF00) | value; break;
            case 3: {
                int high = value & 0x07;
                timerPeriod = (timerPeriod & 0x00FF) | (high << 8);
                timer = (timer & 0x00FF) | (high << 8);
                if (enabled) lengthCounter = PulseChannel.LENGTH_TABLE[(value >> 3) & 0x1F] & 0xFF;
                linearStart = true;
                sequence = 0;
                break;
            }
        }
    }

    public void setEnabled(boolean en) { enabled = en; if (!en) lengthCounter = 0; }

    public void tick() {
        if (timer == 0) { timer = timerPeriod; sequence = (sequence + 1) & 0x1F; }
        else timer--;
    }

    private void clockLinear() {
        if (linearStart) { linearStart = false; linearCounter = linearReload; }
        else if (linearCounter > 0) linearCounter--;
        if (halt) linearStart = true;
    }

    private void clockLength() { if (!halt && lengthCounter > 0) lengthCounter--; }

    public void clockQuarterFrame() { clockLinear(); }
    public void clockHalfFrame() { clockLength(); }

    public int sample() {
        if (!enabled || lengthCounter == 0 || linearCounter == 0) return 0;
        return TRIANGLE_SEQUENCE[sequence];
    }
}

final class NoiseChannel {
    public static final int[] NOISE_PERIOD_TABLE = {
        4,8,16,32,64,96,128,160,202,254,380,508,762,1016,2034,4068,
    };

    public boolean halt;
    public boolean constantVolume;
    public int volume;
    public boolean mode;
    public int periodIndex;
    public int timerPeriod;
    public int timer;
    public int lfsr;
    public int lengthCounter;
    public boolean enabled;
    public int envelopeDivider;
    public int envelopeDecay;
    public boolean envelopeStart;

    public NoiseChannel() { timerPeriod = NOISE_PERIOD_TABLE[0]; lfsr = 1; }

    public void writeRegister(int reg, int value) {
        switch (reg) {
            case 0: halt = (value & 0x20) != 0; constantVolume = (value & 0x10) != 0; volume = value & 0x0F; break;
            case 1: break;
            case 2: mode = (value & 0x80) != 0; periodIndex = value & 0x0F; timerPeriod = NOISE_PERIOD_TABLE[periodIndex]; break;
            case 3:
                if (enabled) lengthCounter = PulseChannel.LENGTH_TABLE[(value >> 3) & 0x1F] & 0xFF;
                envelopeStart = true;
                break;
        }
    }

    public void setEnabled(boolean en) { enabled = en; if (!en) lengthCounter = 0; }

    public void tick() {
        if (timer == 0) {
            timer = timerPeriod;
            int bit0 = lfsr & 0x0001;
            int tap = mode ? 6 : 1;
            int tapBit = (lfsr >> tap) & 0x0001;
            int feedback = bit0 ^ tapBit;
            lfsr = lfsr >> 1;
            if (feedback != 0) lfsr |= 0x4000;
        } else timer--;
    }

    public void clockEnvelope() {
        if (envelopeStart) { envelopeStart = false; envelopeDecay = 15; envelopeDivider = volume; }
        else if (envelopeDivider == 0) {
            envelopeDivider = volume;
            if (envelopeDecay > 0) envelopeDecay--;
            else if (halt) envelopeDecay = 15;
        } else envelopeDivider--;
    }

    private void clockLength() { if (!halt && lengthCounter > 0) lengthCounter--; }

    public void clockQuarterFrame() { clockEnvelope(); }
    public void clockHalfFrame() { clockLength(); }

    public int sample() {
        if (!enabled || lengthCounter == 0) return 0;
        if ((lfsr & 1) != 0) return 0;
        return constantVolume ? volume : envelopeDecay;
    }
}

final class DmcChannel {
    public static final int[] DMC_RATE_TABLE = {
        214,190,170,160,149,138,127,113,107,95,85,80,71,63,54,42,
    };

    public boolean irqEnable;
    public boolean loopFlag;
    public int rateIndex;
    public int timerPeriod;
    public int timer;
    public int outputCounter;
    public int sampleBuffer;
    public int bufferBits;
    public int sampleAddrBase;
    public int sampleAddress;
    public int sampleLength;
    public int bytesRemaining;
    public boolean enabled;
    public boolean irqFlag;

    public DmcChannel() { timerPeriod = DMC_RATE_TABLE[0]; sampleAddrBase = 0xC000; sampleAddress = 0xC000; sampleLength = 1; }

    public void writeRegister(int reg, int value) {
        switch (reg) {
            case 0: irqEnable = (value & 0x80) != 0; loopFlag = (value & 0x40) != 0; rateIndex = value & 0x0F; timerPeriod = DMC_RATE_TABLE[rateIndex]; break;
            case 1: outputCounter = value & 0x7F; break;
            case 2: sampleAddrBase = ((value << 6) | 0xC000) & 0xFFFF; break;
            case 3: sampleLength = (value << 4) | 1; break;
        }
    }

    public void setEnabled(boolean en) {
        enabled = en;
        if (en) {
            if (bytesRemaining == 0) { sampleAddress = sampleAddrBase; bytesRemaining = sampleLength; bufferBits = 0; }
        } else bytesRemaining = 0;
    }

    public void clearIrq() { irqFlag = false; }

    private void clockOutputUnit(DmcReadFn read) {
        if (bufferBits == 0 && bytesRemaining > 0) {
            sampleBuffer = read.read(sampleAddress) & 0xFF;
            bufferBits = 8;
            sampleAddress = (sampleAddress + 1) & 0xFFFF;
            if (sampleAddress == 0) sampleAddress = 0x8000;
            bytesRemaining--;
            if (bytesRemaining == 0) {
                if (loopFlag) { sampleAddress = sampleAddrBase; bytesRemaining = sampleLength; }
                else if (irqEnable) irqFlag = true;
            }
        }
        if (bufferBits > 0) {
            int bit = sampleBuffer & 1;
            if (bit == 0) outputCounter = (outputCounter >= 2) ? (outputCounter - 2) : 0;
            else { int v = outputCounter + 2; if (v > 127) v = 127; outputCounter = v; }
            sampleBuffer = (sampleBuffer >> 1) & 0xFF;
            bufferBits--;
        }
    }

    public void tick(DmcReadFn read) {
        if (timer == 0) { timer = timerPeriod; clockOutputUnit(read); }
        else timer--;
    }

    public int sample() { return outputCounter; }
}

