using System.Runtime.CompilerServices;

namespace NesCore;

// Famicom Disk System expansion audio (wavetable + modulator).
// Port of src/mappers/fds_audio.rs.
public sealed class FdsAudio
{
    public const int WaveTableSize = 64;
    public const int ModTableSize = 64;

    public byte[] WaveRam = new byte[WaveTableSize];
    public byte WaveAddr;
    public byte[] ModWave = new byte[ModTableSize];
    public byte ModWaveAddr;
    public byte MasterVolume;
    public bool EnvDisabled;
    public bool EnvIncrease;
    public byte VolumeGain;
    public ushort Freq;
    public ushort ModFreq;
    public bool ModDisabled = true;
    public byte ModSweepNeg;
    public byte ModSweepPos;
    public sbyte ModGain;
    public byte ModGainOutput;
    public byte ModSweepCounter;
    public uint PhaseAcc;
    public uint ModPhaseAcc;

    public FdsAudio() { }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private static sbyte SatSubI8(sbyte a, sbyte b)
    {
        int r = a - b;
        if (r < -128) return -128;
        if (r > 127) return 127;
        return (sbyte)r;
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private static sbyte SatAddI8(sbyte a, sbyte b)
    {
        int r = a + b;
        if (r < -128) return -128;
        if (r > 127) return 127;
        return (sbyte)r;
    }

    public void WriteRegister(ushort addr, byte value)
    {
        if (addr >= 0x4040 && addr <= 0x407F)
        {
            WaveRam[WaveAddr % WaveTableSize] = (byte)(value & 0x3F);
            WaveAddr = (byte)((WaveAddr + 1) & 0x3F);
            return;
        }
        switch (addr)
        {
            case 0x4080:
                MasterVolume = (byte)(value & 0x3F);
                EnvDisabled = (value & 0x40) != 0;
                EnvIncrease = (value & 0x80) != 0;
                if (EnvDisabled) VolumeGain = MasterVolume;
                else VolumeGain = EnvIncrease ? (byte)0 : (byte)0x3F;
                break;
            case 0x4081:
                Freq = (ushort)((Freq & 0x00FF) | ((value & 0x0F) << 8));
                break;
            case 0x4082:
                Freq = (ushort)((Freq & 0x0F00) | value);
                break;
            case 0x4083:
                ModFreq = (ushort)((ModFreq & 0x00FF) | ((value & 0x0F) << 8));
                if ((value & 0x80) != 0) { ModDisabled = true; ModPhaseAcc = 0; }
                break;
            case 0x4084:
                ModSweepNeg = (byte)(value & 0x0F);
                break;
            case 0x4085:
                ModSweepPos = (byte)(value & 0x0F);
                break;
            case 0x4086:
                ModGain = SatSubI8(ModGain, (sbyte)(value & 0x3F));
                break;
            case 0x4087:
                ModGain = SatAddI8(ModGain, (sbyte)(value & 0x3F));
                if ((value & 0x80) != 0) ModDisabled = true;
                break;
            case 0x4088:
                ModWave[ModWaveAddr % ModTableSize] = (byte)(value & 0x07);
                ModWaveAddr = (byte)((ModWaveAddr + 1) & 0x3F);
                break;
            case 0x4089:
                MasterVolume = (byte)((MasterVolume & 0x3C) | (value & 0x03));
                break;
            case 0x408A:
                if ((value & 0x80) != 0) ModDisabled = true;
                break;
        }
    }

    public byte ReadRegister(ushort addr)
    {
        switch (addr)
        {
            case 0x4090: return (byte)(VolumeGain & 0x3F);
            case 0x4092: return (byte)(ModGainOutput & 0x3F);
            default: return 0;
        }
    }

    public void Clock(uint apuCycles)
    {
        for (uint c = 0; c < apuCycles; ++c)
        {
            PhaseAcc += Freq;
            if (!ModDisabled)
            {
                ModPhaseAcc += ModFreq;
                ModSweepCounter++;
                if (ModSweepCounter >= 8)
                {
                    ModSweepCounter = 0;
                    if (ModSweepNeg > 0)
                        ModGain = SatSubI8(ModGain, (sbyte)ModSweepNeg);
                    if (ModSweepPos > 0)
                        ModGain = SatAddI8(ModGain, (sbyte)ModSweepPos);
                }
            }
            if (!EnvDisabled && ModSweepCounter == 0)
            {
                if (EnvIncrease && VolumeGain < 0x3F) VolumeGain++;
                else if (!EnvIncrease && VolumeGain > 0) VolumeGain--;
            }
        }
    }

    public float Sample()
    {
        if (Freq == 0) return 0.0f;
        uint waveIdx = (PhaseAcc >> 12) & 0x3F;
        short modOffset = 0;
        if (!ModDisabled)
        {
            uint modIdx = (ModPhaseAcc >> 12) & 0x3F;
            short modVal = (short)ModWave[modIdx];
            modOffset = (short)((ModGain * modVal) >> 3);
        }
        short effIdx = (short)((waveIdx + modOffset) & 0x3F);
        short waveVal = (short)WaveRam[effIdx];
        short vol = (short)VolumeGain;
        short sample = (short)((waveVal - 32) * vol);
        float normalized = sample / 2016.0f;
        if (normalized < -1.0f) normalized = -1.0f;
        if (normalized > 1.0f) normalized = 1.0f;
        return normalized;
    }
}
