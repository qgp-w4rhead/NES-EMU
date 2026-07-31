using System;
using System.Runtime.CompilerServices;

namespace NesCore;

// Full emulator state serialization (port of cores/c/src/save_state.c).
// Custom binary format with magic + version tag, manual unsafe pointer
// writing (NativeAOT-compatible, no reflection, no MemoryStream overhead).
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
//
// See: https://www.nesdev.org/wiki/Save_state
public static unsafe class SaveState
{
    public const uint Magic = 0x5353454Eu; // "NESS" LE
    public const uint Version = 1u;
    public const uint HeaderSize = 12u;

    // ---- Size constants ----
    private const uint CpuStateSize = 9u;       // A,X,Y,Sp(4) + Pc(2) + Status,Flags(2) + pad(1)
    private const uint InesHeaderSize = 12u;    // 2 bytes + 2 bytes + 4 bytes + 4 pad
    private const uint PulseStateSize = 20u;    // 10 + 2*2 + 6
    private const uint TriangleStateSize = 12u; // 3 + 2*2 + 4 + 1 pad
    private const uint NoiseStateSize = 17u;    // 5 + 3*2 + 5 + 1 pad
    private const uint DmcStateSize = 22u;      // 3 + 2*2 + 3 + 2*4 + 2 + 2 pad
    private const uint ApuLevelSize = 48u;      // 9*4 + 3*1 + 2*4 + 1
    private const uint ApuStateSize = PulseStateSize * 2 + TriangleStateSize + NoiseStateSize + DmcStateSize + ApuLevelSize;
    private const uint JoypadStateSize = 7u;

    // ---- Required-size computation ----
    public static uint RequiredSize(Emulator emu)
    {
        uint payload = 0;
        payload += CpuStateSize;
        payload += Bus.RamSize;
        payload += PpuStateSize(emu.Bus.Ppu);
        payload += Bus.ApuIoRegCount;
        payload += ApuStateSize;
        payload += JoypadStateSize;
        if (emu.Cartridge != null)
            payload += 1 + InesHeaderSize + emu.Cartridge.Mapper.SaveState(null);
        else
            payload += 1;
        payload += 4 + 8 + 4 + 4 + emu.AudioBufferCount * 4u + 4 + 4;
        return HeaderSize + payload;
    }

    private static uint PpuStateSize(Ppu p)
    {
        return 4u + p.VramSize + Ppu.OamSize + Ppu.PaletteSize
             + 1u + 1u + 1u + 1u + 2u + 2u + 1u + 1u + 1u + 1u + 2u + 2u + 1u + 4u + 4u;
    }

    // ---- LE write helpers (unsafe pointer-based) ----
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private static void PutU32(byte* p, uint v)
    {
        p[0] = (byte)v; p[1] = (byte)(v >> 8); p[2] = (byte)(v >> 16); p[3] = (byte)(v >> 24);
    }
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private static uint GetU32(byte* p)
    {
        return (uint)p[0] | ((uint)p[1] << 8) | ((uint)p[2] << 16) | ((uint)p[3] << 24);
    }
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private static void PutU64(byte* p, ulong v)
    {
        for (int i = 0; i < 8; ++i) p[i] = (byte)(v >> (i * 8));
    }
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private static ulong GetU64(byte* p)
    {
        ulong v = 0;
        for (int i = 0; i < 8; ++i) v |= ((ulong)p[i]) << (i * 8);
        return v;
    }
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private static void PutU16(byte* p, ushort v)
    {
        p[0] = (byte)v; p[1] = (byte)(v >> 8);
    }
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private static ushort GetU16(byte* p)
    {
        return (ushort)((ushort)p[0] | ((ushort)p[1] << 8));
    }
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private static void PutFloat(byte* p, float f)
    {
        uint u = *(uint*)&f;
        PutU32(p, u);
    }
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private static float GetFloat(byte* p)
    {
        uint u = GetU32(p);
        return *(float*)&u;
    }

    // ---- Save ----
    public static uint Save(Emulator emu, byte[] buf, uint cap)
    {
        if (emu == null) return 0u;
        uint required = RequiredSize(emu);
        if (buf == null) return required;
        if (cap < required) return 0u;

        fixed (byte* pBuf = buf)
        {
            byte* p = pBuf;
            uint payload = required - HeaderSize;
            PutU32(p, Magic); p += 4;
            PutU32(p, Version); p += 4;
            PutU32(p, payload); p += 4;

            // CPU
            *p++ = emu.Cpu.A;
            *p++ = emu.Cpu.X;
            *p++ = emu.Cpu.Y;
            *p++ = emu.Cpu.Sp;
            PutU16(p, emu.Cpu.Pc); p += 2;
            *p++ = emu.Cpu.Status;
            *p++ = emu.Cpu.Flags;
            *p++ = 0; // pad

            // RAM
            fixed (byte* pRam = emu.Bus.Ram)
                Buffer.MemoryCopy(pRam, p, Bus.RamSize, Bus.RamSize);
            p += Bus.RamSize;

            // PPU arch state
            p = WritePpuState(p, emu.Bus.Ppu);

            // APU open-bus
            fixed (byte* pAob = emu.Bus.ApuOpenBus)
                Buffer.MemoryCopy(pAob, p, Bus.ApuIoRegCount, Bus.ApuIoRegCount);
            p += Bus.ApuIoRegCount;

            // APU state
            p = WriteApuState(p, emu.Bus.Apu);

            // Joypad
            p = WriteJoypadState(p, emu.Bus.Joypad);

            // Cartridge
            if (emu.Cartridge != null)
            {
                *p++ = 1;
                p = WriteInesHeader(p, emu.Cartridge.Header);
                uint msz = emu.Cartridge.Mapper.SaveState(null);
                if (msz > 0)
                {
                    byte[] mb = new byte[msz];
                    emu.Cartridge.Mapper.SaveState(mb);
                    fixed (byte* pMb = mb)
                        Buffer.MemoryCopy(pMb, p, msz, msz);
                    p += msz;
                }
            }
            else *p++ = 0;

            // Emulator-level
            PutU32(p, emu.Bus.DmaStallCycles); p += 4;
            PutU64(p, emu.Bus.CpuCycleCount); p += 8;
            PutFloat(p, emu.SampleAccumulator); p += 4;
            PutU32(p, emu.AudioBufferCount); p += 4;
            for (uint i = 0; i < emu.AudioBufferCount; ++i)
            {
                PutFloat(p, emu.AudioBuffer[i]); p += 4;
            }
            PutU32(p, emu.PpuCycleCarry); p += 4;
            PutU32(p, (uint)emu.Region); p += 4;

            return (uint)(p - pBuf);
        }
    }

    // ---- Load ----
    public static bool Load(Emulator emu, byte[] buf, uint len)
    {
        if (emu == null || buf == null || len < HeaderSize) return false;

        fixed (byte* pBuf = buf)
        {
            byte* p = pBuf;
            byte* end = pBuf + len;

            uint magic = GetU32(p); p += 4;
            if (magic != Magic) return false;
            uint version = GetU32(p); p += 4;
            if (version != Version) return false;
            uint payload = GetU32(p); p += 4;
            if (payload + HeaderSize > len) return false;
            end = pBuf + HeaderSize + payload;

            // CPU
            if (p + CpuStateSize > end) return false;
            emu.Cpu.A = *p++;
            emu.Cpu.X = *p++;
            emu.Cpu.Y = *p++;
            emu.Cpu.Sp = *p++;
            emu.Cpu.Pc = GetU16(p); p += 2;
            emu.Cpu.Status = *p++;
            emu.Cpu.Flags = *p++;
            p++; // pad

            // RAM
            if (p + Bus.RamSize > end) return false;
            fixed (byte* pRam = emu.Bus.Ram)
                Buffer.MemoryCopy(p, pRam, Bus.RamSize, Bus.RamSize);
            p += Bus.RamSize;

            // PPU arch state
            p = ReadPpuState(p, end, emu.Bus.Ppu);
            if (p == null) return false;

            // APU open-bus
            if (p + Bus.ApuIoRegCount > end) return false;
            fixed (byte* pAob = emu.Bus.ApuOpenBus)
                Buffer.MemoryCopy(p, pAob, Bus.ApuIoRegCount, Bus.ApuIoRegCount);
            p += Bus.ApuIoRegCount;

            // APU state
            p = ReadApuState(p, end, emu.Bus.Apu);
            if (p == null) return false;

            // Joypad
            p = ReadJoypadState(p, end, emu.Bus.Joypad);
            if (p == null) return false;

            // Cartridge
            if (p + 1 > end) return false;
            byte cartPresent = *p++;
            if (cartPresent != 0)
            {
                if (p + InesHeaderSize > end) return false;
                InesHeader hdr = ReadInesHeader(p); p += InesHeaderSize;
                if (emu.Cartridge == null) return false;
                if (hdr.MapperNumber != emu.Cartridge.Header.MapperNumber) return false;
                emu.Cartridge.Header = hdr;
                uint msz = emu.Cartridge.Mapper.SaveState(null);
                if (p + msz > end) return false;
                byte[] mb = new byte[msz];
                fixed (byte* pMb = mb)
                    Buffer.MemoryCopy(p, pMb, msz, msz);
                p += msz;
                if (!emu.Cartridge.Mapper.LoadState(mb, msz)) return false;
                emu.Bus.Ppu.SetMirroring(emu.Cartridge.MirrorMode());
                emu.Bus.InsertCartridge(emu.Cartridge);
            }
            else
            {
                emu.Bus.RemoveCartridge();
            }

            // Emulator-level
            if (p + 4 > end) return false;
            emu.Bus.DmaStallCycles = GetU32(p); p += 4;
            if (p + 8 > end) return false;
            emu.Bus.CpuCycleCount = GetU64(p); p += 8;
            if (p + 4 > end) return false;
            emu.SampleAccumulator = GetFloat(p); p += 4;
            if (p + 4 > end) return false;
            uint audioCount = GetU32(p); p += 4;
            if (audioCount > (uint)(end - p) / 4u) return false;
            float[] audio = new float[audioCount];
            for (uint i = 0; i < audioCount; ++i)
            {
                audio[i] = GetFloat(p); p += 4;
            }
            emu.SetAudioBuffer(audio, audioCount);
            if (p + 4 > end) return false;
            emu.PpuCycleCarry = GetU32(p); p += 4;
            if (p + 4 > end) return false;
            uint regn = GetU32(p); p += 4;
            if (regn <= (uint)Region.Dendy) emu.SetRegion((Region)regn);

            // Clear framebuffer to universal bg (derived data)
            uint universalBg = PpuRender.UniversalBgArgb(emu.Bus.Ppu);
            PpuRender.ClearFramebuffer(emu.Bus.Ppu, universalBg);

            return true;
        }
    }

    // ---- PPU state ----
    private static byte* WritePpuState(byte* p, Ppu pp)
    {
        PutU32(p, pp.VramSize); p += 4;
        fixed (byte* pVram = pp.Vram)
            Buffer.MemoryCopy(pVram, p, pp.VramSize, pp.VramSize);
        p += pp.VramSize;
        fixed (byte* pOam = pp.Oam)
            Buffer.MemoryCopy(pOam, p, Ppu.OamSize, Ppu.OamSize);
        p += Ppu.OamSize;
        fixed (byte* pPal = pp.Palette)
            Buffer.MemoryCopy(pPal, p, Ppu.PaletteSize, Ppu.PaletteSize);
        p += Ppu.PaletteSize;
        *p++ = pp.PpuCtrl;
        *p++ = pp.PpuMask;
        *p++ = pp.OamAddr;
        *p++ = pp.PpuStatus;
        PutU16(p, pp.V); p += 2;
        PutU16(p, pp.T); p += 2;
        *p++ = pp.FineX;
        *p++ = (byte)(pp.W ? 1 : 0);
        *p++ = pp.PpuDataBuffer;
        *p++ = pp.OpenBus;
        PutU16(p, pp.Scanline); p += 2;
        PutU16(p, pp.Cycle); p += 2;
        *p++ = (byte)(pp.NmiRequest ? 1 : 0);
        PutU32(p, (uint)pp.Mirroring); p += 4;
        PutU32(p, (uint)pp.Region); p += 4;
        return p;
    }

    private static byte* ReadPpuState(byte* p, byte* end, Ppu pp)
    {
        if (p + 4 > end) return null;
        uint vramSize = GetU32(p); p += 4;
        if (vramSize > Ppu.VramSize4k) return null;
        uint ppuNeed = 4u + vramSize + Ppu.OamSize + Ppu.PaletteSize
                     + 1u + 1u + 1u + 1u + 2u + 2u + 1u + 1u + 1u + 1u + 2u + 2u + 1u + 4u + 4u;
        // Same as PpuStateSize minus the 4-byte VramSize field already consumed.
        if (p + ppuNeed - 4u > end) return null;

        // Read VRAM into temp
        byte[] vram = new byte[vramSize];
        fixed (byte* pVram = vram)
            Buffer.MemoryCopy(p, pVram, vramSize, vramSize);
        p += vramSize;

        byte[] oam = new byte[Ppu.OamSize];
        fixed (byte* pOam = oam)
            Buffer.MemoryCopy(p, pOam, Ppu.OamSize, Ppu.OamSize);
        p += Ppu.OamSize;

        byte[] palette = new byte[Ppu.PaletteSize];
        fixed (byte* pPal = palette)
            Buffer.MemoryCopy(p, pPal, Ppu.PaletteSize, Ppu.PaletteSize);
        p += Ppu.PaletteSize;

        byte ppuctrl = *p++;
        byte ppumask = *p++;
        byte oamaddr = *p++;
        byte ppustatus = *p++;
        ushort v = GetU16(p); p += 2;
        ushort t = GetU16(p); p += 2;
        byte fineX = *p++;
        bool wFlag = *p++ != 0;
        byte ppudataBuffer = *p++;
        byte openBus = *p++;
        ushort scanline = GetU16(p); p += 2;
        ushort cycle = GetU16(p); p += 2;
        bool nmiRequest = *p++ != 0;
        uint mirroring = GetU32(p); p += 4;
        uint region = GetU32(p); p += 4;

        // Apply mirroring/region BEFORE copying VRAM (SetMirroring may resize)
        if (mirroring <= (uint)NesCore.Mirroring.SingleScreen3) pp.SetMirroring((NesCore.Mirroring)mirroring);
        if (region <= (uint)Region.Dendy) pp.SetRegion((Region)region);
        fixed (byte* pVram = vram, pDstVram = pp.Vram)
            Buffer.MemoryCopy(pVram, pDstVram, Ppu.VramSize4k, vramSize);
        pp.VramSize = vramSize;
        fixed (byte* pOam = oam, pDstOam = pp.Oam)
            Buffer.MemoryCopy(pOam, pDstOam, Ppu.OamSize, Ppu.OamSize);
        fixed (byte* pPal = palette, pDstPal = pp.Palette)
            Buffer.MemoryCopy(pPal, pDstPal, Ppu.PaletteSize, Ppu.PaletteSize);
        pp.PpuCtrl = ppuctrl;
        pp.PpuMask = ppumask;
        pp.OamAddr = oamaddr;
        pp.PpuStatus = ppustatus;
        pp.V = v;
        pp.T = t;
        pp.FineX = fineX;
        pp.W = wFlag;
        pp.PpuDataBuffer = ppudataBuffer;
        pp.OpenBus = openBus;
        pp.Scanline = scanline;
        pp.Cycle = cycle;
        pp.NmiRequest = nmiRequest;
        return p;
    }

    // ---- Pulse channel state ----
    private static byte* WritePulseState(byte* p, PulseChannel ch)
    {
        *p++ = ch.Duty;
        *p++ = (byte)(ch.Halt ? 1 : 0);
        *p++ = (byte)(ch.ConstantVolume ? 1 : 0);
        *p++ = ch.Volume;
        *p++ = (byte)(ch.SweepEnabled ? 1 : 0);
        *p++ = ch.SweepPeriod;
        *p++ = (byte)(ch.SweepNegate ? 1 : 0);
        *p++ = ch.SweepShift;
        *p++ = ch.SweepDivider;
        *p++ = (byte)(ch.SweepReload ? 1 : 0);
        PutU16(p, ch.TimerPeriod); p += 2;
        PutU16(p, ch.Timer); p += 2;
        *p++ = ch.Sequence;
        *p++ = ch.LengthCounter;
        *p++ = (byte)(ch.Enabled ? 1 : 0);
        *p++ = ch.EnvelopeDivider;
        *p++ = ch.EnvelopeDecay;
        *p++ = (byte)(ch.EnvelopeStart ? 1 : 0);
        return p;
    }

    private static byte* ReadPulseState(byte* p, byte* end, PulseChannel ch)
    {
        if (p + PulseStateSize > end) return null;
        ch.Duty = *p++;
        ch.Halt = *p++ != 0;
        ch.ConstantVolume = *p++ != 0;
        ch.Volume = *p++;
        ch.SweepEnabled = *p++ != 0;
        ch.SweepPeriod = *p++;
        ch.SweepNegate = *p++ != 0;
        ch.SweepShift = *p++;
        ch.SweepDivider = *p++;
        ch.SweepReload = *p++ != 0;
        ch.TimerPeriod = GetU16(p); p += 2;
        ch.Timer = GetU16(p); p += 2;
        ch.Sequence = *p++;
        ch.LengthCounter = *p++;
        ch.Enabled = *p++ != 0;
        ch.EnvelopeDivider = *p++;
        ch.EnvelopeDecay = *p++;
        ch.EnvelopeStart = *p++ != 0;
        return p;
    }

    // ---- Triangle channel state ----
    private static byte* WriteTriangleState(byte* p, TriangleChannel ch)
    {
        *p++ = (byte)(ch.Halt ? 1 : 0);
        *p++ = ch.LinearReload;
        *p++ = ch.LinearCounter;
        PutU16(p, ch.TimerPeriod); p += 2;
        PutU16(p, ch.Timer); p += 2;
        *p++ = ch.Sequence;
        *p++ = ch.LengthCounter;
        *p++ = (byte)(ch.Enabled ? 1 : 0);
        *p++ = (byte)(ch.LinearStart ? 1 : 0);
        *p++ = 0; // pad
        return p;
    }

    private static byte* ReadTriangleState(byte* p, byte* end, TriangleChannel ch)
    {
        if (p + TriangleStateSize > end) return null;
        ch.Halt = *p++ != 0;
        ch.LinearReload = *p++;
        ch.LinearCounter = *p++;
        ch.TimerPeriod = GetU16(p); p += 2;
        ch.Timer = GetU16(p); p += 2;
        ch.Sequence = *p++;
        ch.LengthCounter = *p++;
        ch.Enabled = *p++ != 0;
        ch.LinearStart = *p++ != 0;
        p++; // pad
        return p;
    }

    // ---- Noise channel state ----
    private static byte* WriteNoiseState(byte* p, NoiseChannel ch)
    {
        *p++ = (byte)(ch.Halt ? 1 : 0);
        *p++ = (byte)(ch.ConstantVolume ? 1 : 0);
        *p++ = ch.Volume;
        *p++ = (byte)(ch.Mode ? 1 : 0);
        *p++ = ch.PeriodIndex;
        PutU16(p, ch.TimerPeriod); p += 2;
        PutU16(p, ch.Timer); p += 2;
        PutU16(p, ch.Lfsr); p += 2;
        *p++ = ch.LengthCounter;
        *p++ = (byte)(ch.Enabled ? 1 : 0);
        *p++ = ch.EnvelopeDivider;
        *p++ = ch.EnvelopeDecay;
        *p++ = (byte)(ch.EnvelopeStart ? 1 : 0);
        *p++ = 0; // pad
        return p;
    }

    private static byte* ReadNoiseState(byte* p, byte* end, NoiseChannel ch)
    {
        if (p + NoiseStateSize > end) return null;
        ch.Halt = *p++ != 0;
        ch.ConstantVolume = *p++ != 0;
        ch.Volume = *p++;
        ch.Mode = *p++ != 0;
        ch.PeriodIndex = *p++;
        ch.TimerPeriod = GetU16(p); p += 2;
        ch.Timer = GetU16(p); p += 2;
        ch.Lfsr = GetU16(p); p += 2;
        ch.LengthCounter = *p++;
        ch.Enabled = *p++ != 0;
        ch.EnvelopeDivider = *p++;
        ch.EnvelopeDecay = *p++;
        ch.EnvelopeStart = *p++ != 0;
        p++; // pad
        return p;
    }

    // ---- DMC channel state ----
    private static byte* WriteDmcState(byte* p, DmcChannel ch)
    {
        *p++ = (byte)(ch.IrqEnable ? 1 : 0);
        *p++ = (byte)(ch.LoopFlag ? 1 : 0);
        *p++ = ch.RateIndex;
        PutU16(p, ch.TimerPeriod); p += 2;
        PutU16(p, ch.Timer); p += 2;
        *p++ = ch.OutputCounter;
        *p++ = ch.SampleBuffer;
        *p++ = ch.BufferBits;
        PutU16(p, ch.SampleAddrBase); p += 2;
        PutU16(p, ch.SampleAddress); p += 2;
        PutU16(p, ch.SampleLength); p += 2;
        PutU16(p, ch.BytesRemaining); p += 2;
        *p++ = (byte)(ch.Enabled ? 1 : 0);
        *p++ = (byte)(ch.IrqFlag ? 1 : 0);
        *p++ = 0; // pad
        *p++ = 0; // pad
        return p;
    }

    private static byte* ReadDmcState(byte* p, byte* end, DmcChannel ch)
    {
        if (p + DmcStateSize > end) return null;
        ch.IrqEnable = *p++ != 0;
        ch.LoopFlag = *p++ != 0;
        ch.RateIndex = *p++;
        ch.TimerPeriod = GetU16(p); p += 2;
        ch.Timer = GetU16(p); p += 2;
        ch.OutputCounter = *p++;
        ch.SampleBuffer = *p++;
        ch.BufferBits = *p++;
        ch.SampleAddrBase = GetU16(p); p += 2;
        ch.SampleAddress = GetU16(p); p += 2;
        ch.SampleLength = GetU16(p); p += 2;
        ch.BytesRemaining = GetU16(p); p += 2;
        ch.Enabled = *p++ != 0;
        ch.IrqFlag = *p++ != 0;
        p++; // pad
        p++; // pad
        return p;
    }

    // ---- APU state ----
    private static byte* WriteApuState(byte* p, Apu a)
    {
        p = WritePulseState(p, a.Pulse1);
        p = WritePulseState(p, a.Pulse2);
        p = WriteTriangleState(p, a.Triangle);
        p = WriteNoiseState(p, a.Noise);
        p = WriteDmcState(p, a.Dmc);
        PutU32(p, a.CycleAccumulator); p += 4;
        PutFloat(p, a.SampleAccumulator); p += 4;
        PutFloat(p, a.LpfPrev); p += 4;
        PutFloat(p, a.DcPrevX); p += 4;
        PutFloat(p, a.DcPrevY); p += 4;
        PutFloat(p, a.MixAccumulator); p += 4;
        PutU32(p, a.MixCount); p += 4;
        PutFloat(p, a.LastDecimated); p += 4;
        PutU32(p, a.FrameCycle); p += 4;
        *p++ = (byte)(a.FrameMode5Step ? 1 : 0);
        *p++ = (byte)(a.FrameIrqInhibit ? 1 : 0);
        *p++ = (byte)(a.FrameIrq ? 1 : 0);
        PutU32(p, a.FrameResetDelay); p += 4;
        PutU32(p, (uint)a.Region); p += 4;
        *p++ = a.SelectedChannel;
        return p;
    }

    private static byte* ReadApuState(byte* p, byte* end, Apu a)
    {
        p = ReadPulseState(p, end, a.Pulse1); if (p == null) return null;
        p = ReadPulseState(p, end, a.Pulse2); if (p == null) return null;
        p = ReadTriangleState(p, end, a.Triangle); if (p == null) return null;
        p = ReadNoiseState(p, end, a.Noise); if (p == null) return null;
        p = ReadDmcState(p, end, a.Dmc); if (p == null) return null;
        if (p + ApuLevelSize > end) return null;
        a.CycleAccumulator = GetU32(p); p += 4;
        a.SampleAccumulator = GetFloat(p); p += 4;
        a.LpfPrev = GetFloat(p); p += 4;
        a.DcPrevX = GetFloat(p); p += 4;
        a.DcPrevY = GetFloat(p); p += 4;
        a.MixAccumulator = GetFloat(p); p += 4;
        a.MixCount = GetU32(p); p += 4;
        a.LastDecimated = GetFloat(p); p += 4;
        a.FrameCycle = GetU32(p); p += 4;
        a.FrameMode5Step = *p++ != 0;
        a.FrameIrqInhibit = *p++ != 0;
        a.FrameIrq = *p++ != 0;
        a.FrameResetDelay = GetU32(p); p += 4;
        uint regn = GetU32(p); p += 4;
        if (regn <= (uint)Region.Dendy) a.Region = (Region)regn;
        a.SelectedChannel = *p++;
        return p;
    }

    // ---- Joypad state ----
    private static byte* WriteJoypadState(byte* p, Joypad j)
    {
        *p++ = j.Current[0];
        *p++ = j.Current[1];
        *p++ = (byte)(j.Strobe ? 1 : 0);
        *p++ = j.Shift[0];
        *p++ = j.Shift[1];
        *p++ = j.Counter[0];
        *p++ = j.Counter[1];
        return p;
    }

    private static byte* ReadJoypadState(byte* p, byte* end, Joypad j)
    {
        if (p + JoypadStateSize > end) return null;
        j.Current[0] = *p++;
        j.Current[1] = *p++;
        j.Strobe = *p++ != 0;
        j.Shift[0] = *p++;
        j.Shift[1] = *p++;
        j.Counter[0] = *p++;
        j.Counter[1] = *p++;
        return p;
    }

    // ---- InesHeader ----
    private static byte* WriteInesHeader(byte* p, InesHeader h)
    {
        *p++ = h.PrgRomBanks;
        *p++ = h.ChrRomBanks;
        PutU16(p, h.MapperNumber); p += 2;
        *p++ = (byte)h.Mirroring;
        *p++ = (byte)(h.HasTrainer ? 1 : 0);
        *p++ = (byte)(h.HasBattery ? 1 : 0);
        *p++ = h.TvSystem;
        *p++ = 0; *p++ = 0; *p++ = 0; *p++ = 0; // pad
        return p;
    }

    private static InesHeader ReadInesHeader(byte* p)
    {
        var h = new InesHeader();
        h.PrgRomBanks = *p++;
        h.ChrRomBanks = *p++;
        h.MapperNumber = GetU16(p); p += 2;
        byte mirr = *p++;
        h.Mirroring = (mirr <= (byte)NesCore.Mirroring.FourScreen)
            ? (NesCore.Mirroring)mirr
            : NesCore.Mirroring.Horizontal;
        h.HasTrainer = *p++ != 0;
        h.HasBattery = *p++ != 0;
        h.TvSystem = *p++;
        p += 4; // pad
        return h;
    }
}
