using System.Runtime.CompilerServices;

namespace NesCore;

// YM2149 / AY-3-8910 PSG for Sunsoft 5B (mapper 69 audio).
// Port of src/mappers/ym2149.rs.
public sealed class Ym2149
{
    public byte[] Regs = new byte[16];
    public byte AddrLatch;
    public ushort[] ToneTimer = new ushort[3];
    public byte[] ToneOut = new byte[3];
    public ushort NoiseTimer;
    public uint NoiseLfsr = 0x10000;
    public byte NoiseOut;
    public ushort EnvTimer;
    public byte EnvPos;
    public bool EnvHolding;

    public Ym2149() { }

    public void WriteAddr(byte addr) { AddrLatch = (byte)(addr & 0x0F); }

    public void WriteData(byte value)
    {
        byte a = AddrLatch;
        if (a >= 16) return;
        Regs[a] = value;
    }

    public byte ReadData()
    {
        byte a = AddrLatch;
        if (a >= 16) return 0;
        return Regs[a];
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private ushort TonePeriod(byte ch)
    {
        ushort lo = Regs[ch * 2];
        ushort hi = (ushort)(Regs[ch * 2 + 1] & 0x0F);
        ushort p = (ushort)(lo | (hi << 8));
        return p > 0 ? p : (ushort)1;
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private ushort NoisePeriod()
    {
        ushort p = (ushort)(Regs[6] & 0x1F);
        return p > 0 ? p : (ushort)1;
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private ushort EnvPeriod()
    {
        ushort lo = Regs[0x0B];
        ushort hi = Regs[0x0C];
        ushort p = (ushort)(lo | (hi << 8));
        return p > 0 ? p : (ushort)1;
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private byte EnvShape() => (byte)(Regs[0x0D] & 0x0F);

    private void ApplyShapeEnd()
    {
        byte shape = EnvShape();
        bool hold = (shape & 0x08) != 0;
        bool alternate = (shape & 0x04) != 0;
        if (hold)
        {
            EnvHolding = true;
            if (alternate) EnvPos = (byte)(31 - EnvPos);
        }
        else
        {
            EnvPos = 0;
        }
    }

    public void Clock(uint apuCycles)
    {
        for (uint c = 0; c < apuCycles; ++c)
        {
            for (byte ch = 0; ch < 3; ++ch)
            {
                if (ToneTimer[ch] == 0)
                {
                    ToneTimer[ch] = TonePeriod(ch);
                    ToneOut[ch] ^= 1;
                }
                else
                {
                    ToneTimer[ch]--;
                }
            }
            if (NoiseTimer == 0)
            {
                NoiseTimer = NoisePeriod();
                uint bit = (NoiseLfsr ^ (NoiseLfsr >> 3)) & 1;
                NoiseLfsr = (NoiseLfsr >> 1) | (bit << 16);
                NoiseOut = (byte)(NoiseLfsr & 1);
            }
            else
            {
                NoiseTimer--;
            }
            if (!EnvHolding)
            {
                if (EnvTimer == 0)
                {
                    EnvTimer = EnvPeriod();
                    if (EnvPos < 31) EnvPos++;
                    else ApplyShapeEnd();
                }
                else
                {
                    EnvTimer--;
                }
            }
        }
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private byte EnvAmplitude()
    {
        byte shape = EnvShape();
        bool attack = (shape & 0x02) != 0;
        bool alternate = (shape & 0x04) != 0;
        byte pos = EnvPos;
        byte half = (byte)(pos / 2);
        if (half > 15) half = 15;
        byte amp = attack ? half : (byte)(15 - half);
        if (pos >= 16)
        {
            if (alternate)
            {
                byte second = (byte)((pos - 16) / 2);
                if (second > 15) second = 15;
                return attack ? (byte)(15 - second) : second;
            }
            else if (attack)
            {
                return 15;
            }
            else
            {
                return 0;
            }
        }
        return amp;
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private byte ChanOut(byte ch)
    {
        byte disable = Regs[7];
        bool toneEn = ((disable >> ch) & 1) == 0;
        bool noiseEn = ((disable >> (ch + 3)) & 1) == 0;
        byte v = Regs[8 + ch];
        byte vol = (byte)(v & 0x0F);
        bool envMode = (v & 0x10) != 0;
        byte amp = envMode ? EnvAmplitude() : vol;
        byte tone = toneEn ? ToneOut[ch] : (byte)0;
        byte noise = noiseEn ? NoiseOut : (byte)0;
        return ((tone | noise) != 0) ? amp : (byte)0;
    }

    public float Sample()
    {
        int a = ChanOut(0);
        int b = ChanOut(1);
        int c = ChanOut(2);
        int sum = a + b + c;
        float s = sum / 45.0f;
        if (s < -1.0f) s = -1.0f;
        if (s > 1.0f) s = 1.0f;
        return s;
    }
}
