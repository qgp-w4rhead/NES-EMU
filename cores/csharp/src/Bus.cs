using System.Runtime.CompilerServices;

namespace NesCore;

// CPU memory bus: address-space routing + mirroring (port of src/bus.rs).
public sealed class Bus
{
    public const uint RamSize = 0x0800;
    public const uint RamMask = 0x07FF;
    public const uint PpuRegBase = 0x2000;
    public const uint ApuIoBase = 0x4000;
    public const uint ApuIoRegCount = 0x18;
    public const uint CartBase = 0x4020;
    public const uint Mmc3IrqClockCycle = 260;

    public byte[] Ram = new byte[RamSize];
    public Ppu Ppu = new Ppu();
    public Apu Apu = new Apu();
    public byte[] ApuOpenBus = new byte[ApuIoRegCount];
    public Joypad Joypad = new Joypad();
    public Cartridge Cartridge;
    public uint DmaStallCycles;
    public ulong CpuCycleCount;

    // Pre-allocated OAM DMA temp buffer (avoid per-call heap allocation).
    private readonly byte[] _oamDmaTemp = new byte[256];

    // Cached delegates (avoid per-call method-group allocation in hot path).
    private readonly DmcReadFn _dmcReadCb;
    private readonly ChrReader _chrRead;

    public Bus()
    {
        _dmcReadCb = DmcReadCb;
        _chrRead = ChrRead;
    }

    public void Init()
    {
        System.Array.Clear(Ram, 0, (int)RamSize);
        Ppu = new Ppu();
        Apu = new Apu();
        System.Array.Clear(ApuOpenBus, 0, (int)ApuIoRegCount);
        Joypad = new Joypad();
        Cartridge = null;
        DmaStallCycles = 0;
        CpuCycleCount = 0;
    }

    public void InitWithCartridge(Cartridge cartridge)
    {
        Init();
        Cartridge = cartridge;
        if (cartridge != null) Ppu.SetMirroring(cartridge.MirrorMode());
    }

    public Cartridge InsertCartridge(Cartridge cartridge)
    {
        Cartridge prev = Cartridge;
        Cartridge = cartridge;
        if (cartridge != null) Ppu.SetMirroring(cartridge.MirrorMode());
        return prev;
    }

    public Cartridge RemoveCartridge()
    {
        Cartridge prev = Cartridge;
        Cartridge = null;
        return prev;
    }

    private byte PpuReadPpuData()
    {
        ushort addr = Ppu.V;
        if (addr >= 0x3F00)
        {
            byte pal = Ppu.ReadPalette(addr);
            byte val = (byte)((pal & 0x3F) | (Ppu.OpenBusValue() & 0xC0));
            ushort ntAddr = (ushort)(addr & 0x2FFF);
            byte bufferedFill;
            if (ntAddr < 0x2000)
                bufferedFill = Cartridge != null ? Cartridge.ReadChr(ntAddr) : (byte)0;
            else
                bufferedFill = Ppu.ReadNametable(ntAddr);
            Ppu.SetPpuDataBuffer(bufferedFill);
            Ppu.AdvanceVramAddr();
            Ppu.SetOpenBus(val);
            return val;
        }

        byte buffered = Ppu.PpuDataBufferValue();
        byte raw;
        if (addr < 0x2000)
            raw = Cartridge != null ? Cartridge.ReadChr(addr) : (byte)0;
        else
            raw = Ppu.ReadNametable(addr);
        Ppu.SetPpuDataBuffer(raw);
        Ppu.AdvanceVramAddr();
        Ppu.SetOpenBus(buffered);
        return buffered;
    }

    private void PpuWritePpuData(byte value)
    {
        ushort addr = Ppu.V;
        if (addr >= 0x3F00)
            Ppu.WritePalette(addr, value);
        else if (addr < 0x2000)
        {
            if (Cartridge != null) Cartridge.WriteChr(addr, value);
        }
        else
            Ppu.WriteNametable(addr, value);
        Ppu.WriteRegister(7, value);
        Ppu.AdvanceVramAddr();
    }

    private byte PpuRead(ushort reg)
    {
        if ((reg & 0x07) == 7) return PpuReadPpuData();
        return Ppu.ReadRegister(reg);
    }

    private void PpuWrite(ushort reg, byte value)
    {
        if ((reg & 0x07) == 7) { PpuWritePpuData(value); return; }
        Ppu.WriteRegister(reg, value);
    }

    private void OamDma(byte page)
    {
        ushort b = (ushort)(page << 8);
        for (ushort i = 0; i < 256; ++i)
            _oamDmaTemp[i] = Read((ushort)(b + i));
        Ppu.OamDma(_oamDmaTemp);
        ApuOpenBus[0x14] = page;
        Ppu.SetOpenBus(page);
        uint stall = (CpuCycleCount & 1) != 0 ? 513u : 512u;
        if (DmaStallCycles > 0xFFFFFFFFu - stall) DmaStallCycles = 0xFFFFFFFFu;
        else DmaStallCycles += stall;
    }

    private byte CartRead(ushort addr)
        => Cartridge != null ? Cartridge.ReadPrgMut(addr) : (byte)0;

    private void CartWrite(ushort addr, byte value)
    {
        if (Cartridge == null) return;
        Cartridge.WritePrg(addr, value);
        Ppu.SetMirroring(Cartridge.MirrorMode());
    }

    public byte Read(ushort addr)
    {
        if (addr <= 0x1FFF) return Ram[addr & RamMask];
        if (addr <= 0x3FFF) return PpuRead((ushort)(addr & 0x0007));
        if (addr <= 0x4007) return ApuOpenBus[addr - ApuIoBase];
        if (addr <= 0x400B) return ApuOpenBus[addr - ApuIoBase];
        if (addr <= 0x400F) return ApuOpenBus[addr - ApuIoBase];
        if (addr <= 0x4013) return ApuOpenBus[addr - ApuIoBase];
        if (addr == 0x4014) return ApuOpenBus[0x14];
        if (addr == 0x4015)
        {
            byte status = Apu.ReadStatus();
            return (byte)((status & 0xDF) | (ApuOpenBus[0x15] & 0x20));
        }
        if (addr == 0x4016)
        {
            byte ob = ApuOpenBus[0x16];
            byte jb = Joypad.Read(0);
            return (byte)((jb & 0x01) | (ob & 0xFE));
        }
        if (addr == 0x4017)
        {
            byte ob = ApuOpenBus[0x17];
            byte jb = Joypad.Read(1);
            return (byte)((jb & 0x01) | (ob & 0xFE));
        }
        if (addr <= 0x401F) return 0x00;
        return CartRead(addr);
    }

    public void Write(ushort addr, byte value)
    {
        if (addr <= 0x1FFF) { Ram[addr & RamMask] = value; return; }
        if (addr <= 0x3FFF) { PpuWrite((ushort)(addr & 0x0007), value); return; }
        if (addr <= 0x4007)
        {
            ushort offset = (ushort)(addr - ApuIoBase);
            if (offset < 4) Pulse1Write((byte)offset, value);
            else Pulse2Write((byte)(offset - 4), value);
            ApuOpenBus[offset] = value;
            return;
        }
        if (addr <= 0x400B)
        {
            ushort offset = (ushort)(addr - ApuIoBase);
            Apu.Triangle.WriteRegister((byte)(offset - 0x08), value);
            ApuOpenBus[offset] = value;
            return;
        }
        if (addr <= 0x400F)
        {
            ushort offset = (ushort)(addr - ApuIoBase);
            Apu.Noise.WriteRegister((byte)(offset - 0x0C), value);
            ApuOpenBus[offset] = value;
            return;
        }
        if (addr <= 0x4013)
        {
            ushort offset = (ushort)(addr - ApuIoBase);
            Apu.Dmc.WriteRegister((byte)(offset - 0x10), value);
            ApuOpenBus[offset] = value;
            return;
        }
        if (addr == 0x4014) { OamDma(value); return; }
        if (addr == 0x4015)
        {
            ApuOpenBus[0x15] = value;
            Apu.WriteStatus(value);
            return;
        }
        if (addr == 0x4016)
        {
            ApuOpenBus[0x16] = value;
            Joypad.WriteStrobe(value);
            return;
        }
        if (addr == 0x4017)
        {
            ApuOpenBus[0x17] = value;
            Apu.WriteFrameCounter(value);
            return;
        }
        if (addr <= 0x401F) return;
        CartWrite(addr, value);
    }

    private void Pulse1Write(byte reg, byte value)
    {
        switch (reg)
        {
            case 0: Apu.Pulse1.WriteRegister(0, value); break;
            case 1: Apu.Pulse1.WriteRegister(1, value); break;
            case 2: Apu.Pulse1.WriteRegister(2, value); break;
            case 3: Apu.Pulse1.WriteRegister(3, value); break;
        }
    }

    private void Pulse2Write(byte reg, byte value)
    {
        switch (reg)
        {
            case 0: Apu.Pulse2.WriteRegister(0, value); break;
            case 1: Apu.Pulse2.WriteRegister(1, value); break;
            case 2: Apu.Pulse2.WriteRegister(2, value); break;
            case 3: Apu.Pulse2.WriteRegister(3, value); break;
        }
    }

    public byte Peek(ushort addr)
    {
        if (addr <= 0x1FFF) return Ram[addr & RamMask];
        if (addr <= 0x401F) return 0x00;
        return Cartridge != null ? Cartridge.ReadPrg(addr) : (byte)0;
    }

    public uint TakeDmaStallCycles()
    {
        uint c = DmaStallCycles;
        DmaStallCycles = 0;
        return c;
    }

    public void AdvanceCpuCycles(uint cycles) => CpuCycleCount += cycles;
    public void SetCpuCycleCount(ulong count) => CpuCycleCount = count;

    private byte DmcReadCb(ushort addr)
    {
        if (addr <= 0x1FFF) return Ram[addr & RamMask];
        if (addr >= 0x8000) return Cartridge != null ? Cartridge.ReadPrg(addr) : (byte)0;
        return 0;
    }

    public void StepApu(uint cpuCycles)
    {
        Apu.Step(cpuCycles, _dmcReadCb);
    }

    public bool ApuIrqPending() => Apu.IrqPending();
    public bool CartIrqPending() => Cartridge != null && Cartridge.IrqPending();

    public void ClockCartCpu(uint cpuCycles)
    {
        if (Cartridge != null) Cartridge.ClockCpu(cpuCycles);
    }

    public float ExpansionAudioSample()
        => Cartridge != null ? Cartridge.ExpansionAudioSample() : 0.0f;

    public void CartResetScanlineCounter()
    {
        if (Cartridge != null) Cartridge.ResetScanlineCounter();
    }

    private byte ChrRead(ushort addr)
        => Cartridge != null ? Cartridge.ReadChrLatched(addr) : (byte)0;

    public bool StepPpu(uint cycles)
    {
        bool nmi = false;
        ushort prerender = RegionTiming.ScanlinePrerender(Ppu.Region);
        bool rendering = Ppu.IsRendering();
        for (uint i = 0; i < cycles; ++i)
        {
            if (Ppu.StepRendered(_chrRead)) nmi = true;
            ushort cyc = Ppu.Cycle;
            ushort sl = Ppu.Scanline;
            if (rendering && cyc == Mmc3IrqClockCycle
                && (sl < Ppu.ScreenHeight || sl == prerender))
            {
                if (Cartridge != null) Cartridge.ClockIrq();
            }
            if (sl == prerender && cyc == 1)
            {
                if (Cartridge != null) Cartridge.ResetScanlineCounter();
            }
        }
        return nmi;
    }

    public bool TakeNmiRequest() => Ppu.TakeNmiRequest();

    public void RenderFrame()
    {
        PpuRender.RenderFrame(Ppu, _chrRead);
    }
}
