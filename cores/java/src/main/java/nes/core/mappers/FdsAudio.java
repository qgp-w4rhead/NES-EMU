package nes.core.mappers;

// Famicom Disk System expansion audio (wavetable + modulator).
// Port of cores/csharp/src/Audio/FdsAudio.cs.
public final class FdsAudio {
    public static final int WAVE_TABLE_SIZE = 64;
    public static final int MOD_TABLE_SIZE = 64;

    public byte[] waveRam = new byte[WAVE_TABLE_SIZE];
    public byte waveAddr;
    public byte[] modWave = new byte[MOD_TABLE_SIZE];
    public byte modWaveAddr;
    public int masterVolume;
    public boolean envDisabled;
    public boolean envIncrease;
    public int volumeGain;
    public int freq;
    public int modFreq;
    public boolean modDisabled = true;
    public int modSweepNeg;
    public int modSweepPos;
    public byte modGain;
    public int modGainOutput;
    public int modSweepCounter;
    public int phaseAcc;
    public int modPhaseAcc;

    public FdsAudio() {}

    private static byte satSubI8(byte a, byte b) {
        int r = a - b;
        if (r < -128) r = -128;
        if (r > 127) r = 127;
        return (byte) r;
    }

    private static byte satAddI8(byte a, byte b) {
        int r = a + b;
        if (r < -128) r = -128;
        if (r > 127) r = 127;
        return (byte) r;
    }

    public void writeRegister(int addr, int value) {
        if (addr >= 0x4040 && addr <= 0x407F) {
            waveRam[waveAddr % WAVE_TABLE_SIZE] = (byte) (value & 0x3F);
            waveAddr = (byte) ((waveAddr + 1) & 0x3F);
            return;
        }
        switch (addr) {
            case 0x4080:
                masterVolume = value & 0x3F;
                envDisabled = (value & 0x40) != 0;
                envIncrease = (value & 0x80) != 0;
                if (envDisabled) volumeGain = masterVolume;
                else volumeGain = envIncrease ? 0 : 0x3F;
                break;
            case 0x4081:
                freq = (freq & 0x00FF) | ((value & 0x0F) << 8);
                break;
            case 0x4082:
                freq = (freq & 0x0F00) | value;
                break;
            case 0x4083:
                modFreq = (modFreq & 0x00FF) | ((value & 0x0F) << 8);
                if ((value & 0x80) != 0) { modDisabled = true; modPhaseAcc = 0; }
                break;
            case 0x4084:
                modSweepNeg = value & 0x0F;
                break;
            case 0x4085:
                modSweepPos = value & 0x0F;
                break;
            case 0x4086:
                modGain = satSubI8(modGain, (byte) (value & 0x3F));
                break;
            case 0x4087:
                modGain = satAddI8(modGain, (byte) (value & 0x3F));
                if ((value & 0x80) != 0) modDisabled = true;
                break;
            case 0x4088:
                modWave[modWaveAddr % MOD_TABLE_SIZE] = (byte) (value & 0x07);
                modWaveAddr = (byte) ((modWaveAddr + 1) & 0x3F);
                break;
            case 0x4089:
                masterVolume = (masterVolume & 0x3C) | (value & 0x03);
                break;
            case 0x408A:
                if ((value & 0x80) != 0) modDisabled = true;
                break;
        }
    }

    public int readRegister(int addr) {
        switch (addr) {
            case 0x4090: return volumeGain & 0x3F;
            case 0x4092: return modGainOutput & 0x3F;
            default: return 0;
        }
    }

    public void clock(int apuCycles) {
        for (int c = 0; c < apuCycles; ++c) {
            phaseAcc += freq;
            if (!modDisabled) {
                modPhaseAcc += modFreq;
                modSweepCounter++;
                if (modSweepCounter >= 8) {
                    modSweepCounter = 0;
                    if (modSweepNeg > 0)
                        modGain = satSubI8(modGain, (byte) modSweepNeg);
                    if (modSweepPos > 0)
                        modGain = satAddI8(modGain, (byte) modSweepPos);
                }
            }
            if (!envDisabled && modSweepCounter == 0) {
                if (envIncrease && volumeGain < 0x3F) volumeGain++;
                else if (!envIncrease && volumeGain > 0) volumeGain--;
            }
        }
    }

    public float sample() {
        if (freq == 0) return 0.0f;
        int waveIdx = (phaseAcc >>> 12) & 0x3F;
        int modOffset = 0;
        if (!modDisabled) {
            int modIdx = (modPhaseAcc >>> 12) & 0x3F;
            int modVal = modWave[modIdx] & 0xFF;
            modOffset = (modGain * modVal) >> 3;
        }
        int effIdx = (waveIdx + modOffset) & 0x3F;
        int waveVal = waveRam[effIdx] & 0xFF;
        int vol = volumeGain & 0x3F;
        int sample = (waveVal - 32) * vol;
        float normalized = sample / 2016.0f;
        if (normalized < -1.0f) normalized = -1.0f;
        if (normalized > 1.0f) normalized = 1.0f;
        return normalized;
    }
}
