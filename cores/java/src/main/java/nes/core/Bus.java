package nes.core;

// CPU memory bus: address-space routing + mirroring (port of cores/csharp/src/Bus.cs).
public final class Bus {
    public static final int RAM_SIZE = 0x0800;
    public static final int RAM_MASK = 0x07FF;
    public static final int PPU_REG_BASE = 0x2000;
    public static final int APU_IO_BASE = 0x4000;
    public static final int APU_IO_REG_COUNT = 0x18;
    public static final int CART_BASE = 0x4020;
    public static final int MMC3_IRQ_CLOCK_CYCLE = 260;

    public final byte[] ram = new byte[RAM_SIZE];
    public Ppu ppu = new Ppu();
    public Apu apu = new Apu();
    public final byte[] apuOpenBus = new byte[APU_IO_REG_COUNT];
    public final Joypad joypad = new Joypad();
    public Cartridge cartridge;
    public int dmaStallCycles;
    public long cpuCycleCount;

    private final byte[] _oamDmaTemp = new byte[256];

    private final DmcReadFn _dmcReadCb = this::dmcReadCb;
    private final ChrReader _chrRead = this::chrRead;

    public Bus() {}

    public void init() {
        for (int i = 0; i < RAM_SIZE; ++i) ram[i] = 0;
        ppu = new Ppu();
        apu = new Apu();
        for (int i = 0; i < APU_IO_REG_COUNT; ++i) apuOpenBus[i] = 0;
        joypad.clear();
        cartridge = null;
        dmaStallCycles = 0;
        cpuCycleCount = 0;
    }

    public void initWithCartridge(Cartridge cart) {
        init();
        cartridge = cart;
        if (cart != null) ppu.setMirroring(cart.mirrorMode());
    }

    public Cartridge insertCartridge(Cartridge cart) {
        Cartridge prev = cartridge;
        cartridge = cart;
        if (cart != null) ppu.setMirroring(cart.mirrorMode());
        return prev;
    }

    public Cartridge removeCartridge() {
        Cartridge prev = cartridge;
        cartridge = null;
        return prev;
    }

    private int ppuReadPpuData() {
        int addr = ppu.v;
        if (addr >= 0x3F00) {
            int pal = ppu.readPalette(addr);
            int val = (pal & 0x3F) | (ppu.openBusValue() & 0xC0);
            int ntAddr = addr & 0x2FFF;
            int bufferedFill;
            if (ntAddr < 0x2000)
                bufferedFill = (cartridge != null) ? cartridge.readChr(ntAddr) : 0;
            else
                bufferedFill = ppu.readNametable(ntAddr);
            ppu.setPpuDataBuffer(bufferedFill);
            ppu.advanceVramAddr();
            ppu.setOpenBus(val);
            return val;
        }

        int buffered = ppu.ppuDataBufferValue();
        int raw;
        if (addr < 0x2000)
            raw = (cartridge != null) ? cartridge.readChr(addr) : 0;
        else
            raw = ppu.readNametable(addr);
        ppu.setPpuDataBuffer(raw);
        ppu.advanceVramAddr();
        ppu.setOpenBus(buffered);
        return buffered;
    }

    private void ppuWritePpuData(int value) {
        int addr = ppu.v;
        if (addr >= 0x3F00)
            ppu.writePalette(addr, value);
        else if (addr < 0x2000) {
            if (cartridge != null) cartridge.writeChr(addr, value);
        } else
            ppu.writeNametable(addr, value);
        ppu.writeRegister(7, value);
        ppu.advanceVramAddr();
    }

    private int ppuRead(int reg) {
        if ((reg & 0x07) == 7) return ppuReadPpuData();
        return ppu.readRegister(reg);
    }

    private void ppuWrite(int reg, int value) {
        if ((reg & 0x07) == 7) { ppuWritePpuData(value); return; }
        ppu.writeRegister(reg, value);
    }

    private void oamDma(int page) {
        int b = page << 8;
        for (int i = 0; i < 256; ++i)
            _oamDmaTemp[i] = (byte) read((b + i) & 0xFFFF);
        ppu.oamDma(_oamDmaTemp);
        apuOpenBus[0x14] = (byte) page;
        ppu.setOpenBus(page);
        int stall = (cpuCycleCount & 1) != 0 ? 513 : 512;
        if (dmaStallCycles > 0xFFFFFFFFL - stall) dmaStallCycles = 0xFFFFFFFF;
        else dmaStallCycles += stall;
    }

    private int cartRead(int addr) {
        return (cartridge != null) ? cartridge.readPrgMut(addr) : 0;
    }

    private void cartWrite(int addr, int value) {
        if (cartridge == null) return;
        cartridge.writePrg(addr, value);
        ppu.setMirroring(cartridge.mirrorMode());
    }

    public int read(int addr) {
        if (addr <= 0x1FFF) return ram[addr & RAM_MASK] & 0xFF;
        if (addr <= 0x3FFF) return ppuRead(addr & 0x0007);
        if (addr <= 0x4007) return apuOpenBus[addr - APU_IO_BASE] & 0xFF;
        if (addr <= 0x400B) return apuOpenBus[addr - APU_IO_BASE] & 0xFF;
        if (addr <= 0x400F) return apuOpenBus[addr - APU_IO_BASE] & 0xFF;
        if (addr <= 0x4013) return apuOpenBus[addr - APU_IO_BASE] & 0xFF;
        if (addr == 0x4014) return apuOpenBus[0x14] & 0xFF;
        if (addr == 0x4015) {
            int status = apu.readStatus();
            return (status & 0xDF) | (apuOpenBus[0x15] & 0x20);
        }
        if (addr == 0x4016) {
            int ob = apuOpenBus[0x16] & 0xFF;
            int jb = joypad.read(0);
            return (jb & 0x01) | (ob & 0xFE);
        }
        if (addr == 0x4017) {
            int ob = apuOpenBus[0x17] & 0xFF;
            int jb = joypad.read(1);
            return (jb & 0x01) | (ob & 0xFE);
        }
        if (addr <= 0x401F) return 0x00;
        return cartRead(addr);
    }

    public void write(int addr, int value) {
        int v = value & 0xFF;
        if (addr <= 0x1FFF) { ram[addr & RAM_MASK] = (byte) v; return; }
        if (addr <= 0x3FFF) { ppuWrite(addr & 0x0007, v); return; }
        if (addr <= 0x4007) {
            int offset = addr - APU_IO_BASE;
            if (offset < 4) pulse1Write(offset, v);
            else pulse2Write(offset - 4, v);
            apuOpenBus[offset] = (byte) v;
            return;
        }
        if (addr <= 0x400B) {
            int offset = addr - APU_IO_BASE;
            apu.triangle.writeRegister(offset - 0x08, v);
            apuOpenBus[offset] = (byte) v;
            return;
        }
        if (addr <= 0x400F) {
            int offset = addr - APU_IO_BASE;
            apu.noise.writeRegister(offset - 0x0C, v);
            apuOpenBus[offset] = (byte) v;
            return;
        }
        if (addr <= 0x4013) {
            int offset = addr - APU_IO_BASE;
            apu.dmc.writeRegister(offset - 0x10, v);
            apuOpenBus[offset] = (byte) v;
            return;
        }
        if (addr == 0x4014) { oamDma(v); return; }
        if (addr == 0x4015) { apuOpenBus[0x15] = (byte) v; apu.writeStatus(v); return; }
        if (addr == 0x4016) { apuOpenBus[0x16] = (byte) v; joypad.writeStrobe(v); return; }
        if (addr == 0x4017) { apuOpenBus[0x17] = (byte) v; apu.writeFrameCounter(v); return; }
        if (addr <= 0x401F) return;
        cartWrite(addr, v);
    }

    private void pulse1Write(int reg, int value) { apu.pulse1.writeRegister(reg, value); }
    private void pulse2Write(int reg, int value) { apu.pulse2.writeRegister(reg, value); }

    public int peek(int addr) {
        if (addr <= 0x1FFF) return ram[addr & RAM_MASK] & 0xFF;
        if (addr <= 0x401F) return 0x00;
        return (cartridge != null) ? cartridge.readPrg(addr) : 0;
    }

    public int takeDmaStallCycles() {
        int c = dmaStallCycles;
        dmaStallCycles = 0;
        return c;
    }

    public void advanceCpuCycles(int cycles) { cpuCycleCount += cycles; }
    public void setCpuCycleCount(long count) { cpuCycleCount = count; }

    private int dmcReadCb(int addr) {
        if (addr <= 0x1FFF) return ram[addr & RAM_MASK] & 0xFF;
        if (addr >= 0x8000) return (cartridge != null) ? cartridge.readPrg(addr) : 0;
        return 0;
    }

    public void stepApu(int cpuCycles) { apu.step(cpuCycles, _dmcReadCb); }

    public boolean apuIrqPending() { return apu.irqPending(); }
    public boolean cartIrqPending() { return cartridge != null && cartridge.irqPending(); }

    public void clockCartCpu(int cpuCycles) {
        if (cartridge != null) cartridge.clockCpu(cpuCycles);
    }

    public float expansionAudioSample() {
        return (cartridge != null) ? cartridge.expansionAudioSample() : 0.0f;
    }

    public void cartResetScanlineCounter() {
        if (cartridge != null) cartridge.resetScanlineCounter();
    }

    private int chrRead(int addr) {
        return (cartridge != null) ? cartridge.readChrLatched(addr) : 0;
    }

    public boolean stepPpu(int cycles) {
        boolean nmi = false;
        int prerender = RegionTiming.scanlinePrerender(ppu.region);
        boolean rendering = ppu.isRendering();
        for (int i = 0; i < cycles; ++i) {
            if (ppu.stepRendered(_chrRead)) nmi = true;
            int cyc = ppu.cycle;
            int sl = ppu.scanline;
            if (rendering && cyc == MMC3_IRQ_CLOCK_CYCLE
                && (sl < Ppu.SCREEN_HEIGHT || sl == prerender)) {
                if (cartridge != null) cartridge.clockIrq();
            }
            if (sl == prerender && cyc == 1) {
                if (cartridge != null) cartridge.resetScanlineCounter();
            }
        }
        return nmi;
    }

    public boolean takeNmiRequest() { return ppu.takeNmiRequest(); }

    public void renderFrame() { PpuRender.renderFrame(ppu, _chrRead); }
}

