using System.Runtime.CompilerServices;

namespace NesCore;

// YM2413 OPLL FM synthesiser for VRC7 (mapper 85 audio).
// Port of src/mappers/opll.rs. Compact, deterministic 2-operator FM synth.
public sealed class Opll
{
    public const int SineLen = 1024;
    public const int SineQuarter = 256;
    public const int Channels = 6;
    public const int EnvAttack = 0;
    public const int EnvDecay = 1;
    public const int EnvSustain = 2;
    public const int EnvRelease = 3;
    public const int EnvOff = 4;

    public struct Patch
    {
        public byte Mult, Tl, Fb, Ar, Dr, Sl, Rr, Kl, Am, Wf;
    }

    public struct ChanReg
    {
        public ushort Fnum;
        public byte Block;
        public byte Volume;
        public byte Instrument;
        public bool KeyOn;
    }

    public struct Operator
    {
        public uint Phase;
        public int EnvPhase;
        public int EnvAmp;
        public int Level;
    }

    public byte[] Regs = new byte[128];
    public byte AddrLatch;
    public Patch[] Patches = new Patch[16];
    public ChanReg[] Chan = new ChanReg[Channels];
    public Operator[][] Ops = new Operator[Channels][];
    public byte[] UserInstBuf = new byte[8];
    public ushort[] Sine = new ushort[SineQuarter];

    public Opll()
    {
        for (int i = 0; i < Channels; ++i) Ops[i] = new Operator[2];
        Init();
    }

    private static Patch MakePatch(byte mult, byte tl, byte fb, byte ar, byte dr, byte sl, byte rr, byte kl, byte am, byte wf)
        => new Patch { Mult = mult, Tl = tl, Fb = fb, Ar = ar, Dr = dr, Sl = sl, Rr = rr, Kl = kl, Am = am, Wf = wf };

    private static Patch PatchFromRegs(byte[] b)
    {
        Patch p;
        p.Mult = (byte)(b[0] & 0x0F);
        p.Tl = (byte)(((b[0] >> 4) & 0x0F) | ((b[2] & 0x03) << 4));
        p.Fb = (byte)(b[1] & 0x07);
        p.Ar = (byte)((b[2] >> 2) & 0x0F);
        p.Dr = (byte)(b[3] & 0x0F);
        p.Sl = (byte)((b[3] >> 4) & 0x0F);
        p.Rr = (byte)(b[4] & 0x0F);
        p.Kl = (byte)((b[4] >> 4) & 0x03);
        p.Am = (byte)(b[5] & 0x07);
        p.Wf = (byte)(b[7] & 0x03);
        return p;
    }

    private void BuildSine()
    {
        for (int i = 0; i < SineQuarter; ++i)
        {
            double s = System.Math.Sin((double)i / ((double)SineLen / 4.0) * 1.5707963267948966);
            Sine[i] = (ushort)(s * 4095.0 + 0.5);
        }
    }

    public void Init()
    {
        Patches[0] = MakePatch(1, 24, 0, 15, 7, 3, 10, 0, 0, 0);
        Patches[1] = MakePatch(1, 16, 2, 15, 6, 5, 8, 1, 0, 0);
        Patches[2] = MakePatch(1, 8, 5, 15, 8, 5, 8, 0, 0, 0);
        Patches[3] = MakePatch(1, 12, 0, 15, 7, 5, 8, 1, 0, 0);
        Patches[4] = MakePatch(1, 8, 0, 15, 5, 3, 8, 1, 0, 0);
        Patches[5] = MakePatch(1, 16, 3, 15, 7, 5, 8, 1, 0, 0);
        Patches[6] = MakePatch(2, 16, 3, 15, 8, 5, 8, 1, 0, 0);
        Patches[7] = MakePatch(1, 24, 0, 15, 7, 3, 10, 0, 0, 0);
        Patches[8] = MakePatch(1, 12, 1, 15, 5, 4, 8, 1, 0, 0);
        Patches[9] = MakePatch(1, 8, 4, 15, 8, 5, 8, 0, 0, 0);
        Patches[10] = MakePatch(1, 16, 0, 15, 5, 3, 8, 1, 0, 0);
        Patches[11] = MakePatch(1, 8, 3, 15, 7, 5, 8, 1, 0, 0);
        Patches[12] = MakePatch(1, 16, 0, 15, 5, 3, 8, 1, 0, 0);
        Patches[13] = MakePatch(1, 16, 0, 15, 5, 3, 8, 1, 0, 0);
        Patches[14] = MakePatch(1, 16, 0, 15, 5, 3, 8, 1, 0, 0);
        BuildSine();
    }

    public void WriteAddr(byte addr) { AddrLatch = (byte)(addr & 0x7F); }

    private void KeyOn(byte ch)
    {
        for (byte i = 0; i < 2; ++i)
        {
            Ops[ch][i].EnvPhase = EnvAttack;
            Ops[ch][i].EnvAmp = 0;
            Ops[ch][i].Phase = 0;
        }
    }

    private void KeyOff(byte ch)
    {
        for (byte i = 0; i < 2; ++i)
            Ops[ch][i].EnvPhase = EnvRelease;
    }

    private void ApplyReg(byte addr, byte value)
    {
        if (addr >= 0x10 && addr <= 0x17)
        {
            byte i = (byte)(addr - 0x10);
            UserInstBuf[i] = value;
            if (i == 7) Patches[15] = PatchFromRegs(UserInstBuf);
        }
        else if (addr >= 0x30 && addr <= 0x35)
        {
            byte ch = (byte)(addr - 0x30);
            Chan[ch].Volume = (byte)(value & 0x0F);
            Chan[ch].Instrument = (byte)((value >> 4) & 0x0F);
        }
        else if (addr >= 0x20 && addr <= 0x25)
        {
            byte ch = (byte)(addr - 0x20);
            Chan[ch].Fnum = (ushort)((Chan[ch].Fnum & 0x0100) | value);
        }
        else if (addr >= 0x40 && addr <= 0x45)
        {
            byte ch = (byte)(addr - 0x40);
            bool wasOn = Chan[ch].KeyOn;
            Chan[ch].Fnum = (ushort)((Chan[ch].Fnum & 0x00FF) | ((value & 0x01) << 8));
            Chan[ch].Block = (byte)((value >> 1) & 0x07);
            Chan[ch].KeyOn = (value & 0x10) != 0;
            bool nowOn = Chan[ch].KeyOn;
            if (nowOn && !wasOn) KeyOn(ch);
            else if (!nowOn && wasOn) KeyOff(ch);
        }
    }

    public void WriteData(byte value)
    {
        byte addr = AddrLatch;
        if (addr >= 0x80) return;
        Regs[addr] = value;
        ApplyReg(addr, value);
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private static uint PhaseInc(ref ChanReg ch, byte mult)
    {
        uint m = (mult == 0) ? 1u : mult;
        return ((uint)ch.Fnum * m) << ch.Block;
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private static int EnvStep(byte rate)
    {
        if (rate == 0) return 0;
        return (int)rate << 3;
    }

    private void ClockEnv(ref Operator op, ref Patch patch, bool isCarrier, byte chVol)
    {
        int sl = (int)patch.Sl << 7;
        const int MAX = 4095;
        switch (op.EnvPhase)
        {
            case EnvOff: op.EnvAmp = MAX; break;
            case EnvAttack:
                op.EnvAmp -= EnvStep(patch.Ar) * 4;
                if (op.EnvAmp <= 0) { op.EnvAmp = 0; op.EnvPhase = EnvDecay; }
                break;
            case EnvDecay:
                op.EnvAmp += EnvStep(patch.Dr);
                if (op.EnvAmp >= sl) { op.EnvAmp = sl; op.EnvPhase = EnvSustain; }
                break;
            case EnvSustain: break;
            case EnvRelease:
                op.EnvAmp += EnvStep(patch.Rr) * 2;
                if (op.EnvAmp >= MAX) { op.EnvAmp = MAX; op.EnvPhase = EnvOff; }
                break;
        }
        int ea = op.EnvAmp;
        if (ea < 0) ea = 0;
        if (ea > MAX) ea = MAX;
        int level = (MAX - ea) >> 2;
        if (isCarrier)
        {
            int tl = chVol + (patch.Tl >> 2);
            if (tl > 63) tl = 63;
            level = level - tl * 16;
            if (level < 0) level = 0;
        }
        else
        {
            level = level - patch.Tl * 16;
            if (level < 0) level = 0;
        }
        op.Level = level;
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private int Sin(uint phase)
    {
        uint idx = (phase >> 10) & 0x3FF;
        byte quad;
        uint intra;
        switch (idx >> 8)
        {
            case 0: quad = 0; intra = idx; break;
            case 1: quad = 1; intra = 0x3FF - idx; break;
            case 2: quad = 2; intra = idx - 0x200; break;
            default: quad = 3; intra = 0x3FF - (idx - 0x200); break;
        }
        uint i = intra >> 2;
        if (i > (uint)(SineLen / 4)) i = (uint)(SineLen / 4);
        int v = Sine[i];
        return (quad == 1 || quad == 3) ? -v : v;
    }

    public void Clock(uint apuCycles)
    {
        for (uint c = 0; c < apuCycles; ++c)
        {
            for (byte ch = 0; ch < Channels; ++ch)
            {
                ref Patch patch = ref Patches[Chan[ch].Instrument];
                byte chVol = Chan[ch].Volume;
                uint pincMod = PhaseInc(ref Chan[ch], patch.Mult);
                uint pincCar = PhaseInc(ref Chan[ch], 1);
                Ops[ch][0].Phase += pincMod;
                ClockEnv(ref Ops[ch][0], ref patch, false, chVol);
                Ops[ch][1].Phase += pincCar;
                ClockEnv(ref Ops[ch][1], ref patch, true, chVol);
            }
        }
    }

    public float Sample()
    {
        int sum = 0;
        for (byte ch = 0; ch < Channels; ++ch)
        {
            if (!Chan[ch].KeyOn && Ops[ch][1].EnvPhase == EnvOff) continue;
            ref Patch patch = ref Patches[Chan[ch].Instrument];
            int fb = (patch.Fb != 0) ? (Ops[ch][0].Level >> (7 - patch.Fb)) : 0;
            int modOut = Sin(Ops[ch][0].Phase + (uint)fb * 64) * Ops[ch][0].Level / 4095;
            uint carPhase = Ops[ch][1].Phase + (uint)(modOut * 16);
            int car = Sin(carPhase) * Ops[ch][1].Level / 4095;
            sum += car;
        }
        float s = sum / ((float)Channels * 4095.0f);
        if (s < -1.0f) s = -1.0f;
        if (s > 1.0f) s = 1.0f;
        return s;
    }
}
