// CPU memory bus: address-space routing + mirroring (port of cores/c/src/bus.c).

import { Ppu } from './ppu';
import { Apu } from './apu';
import { Joypad } from './joypad';
import { Cartridge } from './cartridge';
import { scanlinePrerender } from './region';
import { PpuRender } from './ppu_render';

export class Bus {
    static readonly RamSize = 0x0800;
    static readonly RamMask = 0x07FF;
    static readonly PpuRegBase = 0x2000;
    static readonly ApuIoBase = 0x4000;
    static readonly ApuIoRegCount = 0x18;
    static readonly CartBase = 0x4020;
    static readonly Mmc3IrqClockCycle = 260;

    ram: Uint8Array = new Uint8Array(Bus.RamSize);
    ppu: Ppu = new Ppu();
    apu: Apu = new Apu();
    apuOpenBus: Uint8Array = new Uint8Array(Bus.ApuIoRegCount);
    joypad: Joypad = new Joypad();
    cartridge: Cartridge | null = null;
    dmaStallCycles: number = 0;
    cpuCycleCount: number = 0;

    private _oamDmaTemp: Uint8Array = new Uint8Array(256);

    init(): void {
        this.ram.fill(0);
        this.ppu = new Ppu();
        this.apu = new Apu();
        this.apuOpenBus.fill(0);
        this.joypad = new Joypad();
        this.cartridge = null;
        this.dmaStallCycles = 0;
        this.cpuCycleCount = 0;
    }

    private get _apuOpenBusAlias(): Uint8Array { return this.apuOpenBus; }

    initWithCartridge(cartridge: Cartridge): void {
        this.init();
        this.cartridge = cartridge;
        if (cartridge !== null) this.ppu.setMirroring(cartridge.mirrorMode());
    }

    insertCartridge(cartridge: Cartridge): Cartridge | null {
        const prev = this.cartridge;
        this.cartridge = cartridge;
        if (cartridge !== null) this.ppu.setMirroring(cartridge.mirrorMode());
        return prev;
    }

    removeCartridge(): Cartridge | null {
        const prev = this.cartridge;
        this.cartridge = null;
        return prev;
    }

    private ppuReadPpuData(): number {
        const addr = this.ppu.v;
        if (addr >= 0x3F00) {
            const pal = this.ppu.readPalette(addr);
            const val = (pal & 0x3F) | (this.ppu.openBusValue() & 0xC0);
            const ntAddr = addr & 0x2FFF;
            let bufferedFill: number;
            if (ntAddr < 0x2000)
                bufferedFill = this.cartridge !== null ? this.cartridge.readChr(ntAddr) : 0;
            else
                bufferedFill = this.ppu.readNametable(ntAddr);
            this.ppu.setPpuDataBuffer(bufferedFill);
            this.ppu.advanceVramAddr();
            this.ppu.setOpenBus(val);
            return val;
        }

        const buffered = this.ppu.ppuDataBufferValue();
        let raw: number;
        if (addr < 0x2000)
            raw = this.cartridge !== null ? this.cartridge.readChr(addr) : 0;
        else
            raw = this.ppu.readNametable(addr);
        this.ppu.setPpuDataBuffer(raw);
        this.ppu.advanceVramAddr();
        this.ppu.setOpenBus(buffered);
        return buffered;
    }

    private ppuWritePpuData(value: number): void {
        const addr = this.ppu.v;
        if (addr >= 0x3F00)
            this.ppu.writePalette(addr, value);
        else if (addr < 0x2000) {
            if (this.cartridge !== null) this.cartridge.writeChr(addr, value);
        } else
            this.ppu.writeNametable(addr, value);
        this.ppu.writeRegister(7, value);
        this.ppu.advanceVramAddr();
    }

    private ppuRead(reg: number): number {
        if ((reg & 0x07) === 7) return this.ppuReadPpuData();
        return this.ppu.readRegister(reg);
    }

    private ppuWrite(reg: number, value: number): void {
        if ((reg & 0x07) === 7) { this.ppuWritePpuData(value); return; }
        this.ppu.writeRegister(reg, value);
    }

    private oamDma(page: number): void {
        const b = (page << 8) & 0xFFFF;
        for (let i = 0; i < 256; ++i)
            this._oamDmaTemp[i] = this.read((b + i) & 0xFFFF);
        this.ppu.oamDma(this._oamDmaTemp);
        this.apuOpenBus[0x14] = page;
        this.ppu.setOpenBus(page);
        const stall = (this.cpuCycleCount & 1) !== 0 ? 513 : 512;
        if (this.dmaStallCycles > 0xFFFFFFFF - stall) this.dmaStallCycles = 0xFFFFFFFF;
        else this.dmaStallCycles += stall;
    }

    private cartRead(addr: number): number {
        return this.cartridge !== null ? this.cartridge.readPrgMut(addr) : 0;
    }

    private cartWrite(addr: number, value: number): void {
        if (this.cartridge === null) return;
        this.cartridge.writePrg(addr, value);
        this.ppu.setMirroring(this.cartridge.mirrorMode());
    }

    read(addr: number): number {
        if (addr <= 0x1FFF) return this.ram[addr & Bus.RamMask];
        if (addr <= 0x3FFF) return this.ppuRead(addr & 0x0007);
        if (addr <= 0x4007) return this.apuOpenBus[addr - Bus.ApuIoBase];
        if (addr <= 0x400B) return this.apuOpenBus[addr - Bus.ApuIoBase];
        if (addr <= 0x400F) return this.apuOpenBus[addr - Bus.ApuIoBase];
        if (addr <= 0x4013) return this.apuOpenBus[addr - Bus.ApuIoBase];
        if (addr === 0x4014) return this.apuOpenBus[0x14];
        if (addr === 0x4015) {
            const status = this.apu.readStatus();
            return (status & 0xDF) | (this.apuOpenBus[0x15] & 0x20);
        }
        if (addr === 0x4016) {
            const ob = this.apuOpenBus[0x16];
            const jb = this.joypad.read(0);
            return (jb & 0x01) | (ob & 0xFE);
        }
        if (addr === 0x4017) {
            const ob = this.apuOpenBus[0x17];
            const jb = this.joypad.read(1);
            return (jb & 0x01) | (ob & 0xFE);
        }
        if (addr <= 0x401F) return 0x00;
        return this.cartRead(addr);
    }

    write(addr: number, value: number): void {
        if (addr <= 0x1FFF) { this.ram[addr & Bus.RamMask] = value; return; }
        if (addr <= 0x3FFF) { this.ppuWrite(addr & 0x0007, value); return; }
        if (addr <= 0x4007) {
            const offset = addr - Bus.ApuIoBase;
            if (offset < 4) this.pulse1Write(offset, value);
            else this.pulse2Write(offset - 4, value);
            this.apuOpenBus[offset] = value;
            return;
        }
        if (addr <= 0x400B) {
            const offset = addr - Bus.ApuIoBase;
            this.apu.triangle.writeRegister(offset - 0x08, value);
            this.apuOpenBus[offset] = value;
            return;
        }
        if (addr <= 0x400F) {
            const offset = addr - Bus.ApuIoBase;
            this.apu.noise.writeRegister(offset - 0x0C, value);
            this.apuOpenBus[offset] = value;
            return;
        }
        if (addr <= 0x4013) {
            const offset = addr - Bus.ApuIoBase;
            this.apu.dmc.writeRegister(offset - 0x10, value);
            this.apuOpenBus[offset] = value;
            return;
        }
        if (addr === 0x4014) { this.oamDma(value); return; }
        if (addr === 0x4015) {
            this.apuOpenBus[0x15] = value;
            this.apu.writeStatus(value);
            return;
        }
        if (addr === 0x4016) {
            this.apuOpenBus[0x16] = value;
            this.joypad.writeStrobe(value);
            return;
        }
        if (addr === 0x4017) {
            this.apuOpenBus[0x17] = value;
            this.apu.writeFrameCounter(value);
            return;
        }
        if (addr <= 0x401F) return;
        this.cartWrite(addr, value);
    }

    private pulse1Write(reg: number, value: number): void {
        this.apu.pulse1.writeRegister(reg, value);
    }

    private pulse2Write(reg: number, value: number): void {
        this.apu.pulse2.writeRegister(reg, value);
    }

    peek(addr: number): number {
        if (addr <= 0x1FFF) return this.ram[addr & Bus.RamMask];
        if (addr <= 0x401F) return 0x00;
        return this.cartridge !== null ? this.cartridge.readPrg(addr) : 0;
    }

    takeDmaStallCycles(): number {
        const c = this.dmaStallCycles;
        this.dmaStallCycles = 0;
        return c;
    }

    advanceCpuCycles(cycles: number): void { this.cpuCycleCount += cycles; }
    setCpuCycleCount(count: number): void { this.cpuCycleCount = count; }

    private dmcReadCb = (addr: number): number => {
        if (addr <= 0x1FFF) return this.ram[addr & Bus.RamMask];
        if (addr >= 0x8000) return this.cartridge !== null ? this.cartridge.readPrg(addr) : 0;
        return 0;
    };

    stepApu(cpuCycles: number): void {
        this.apu.step(cpuCycles, this.dmcReadCb);
    }

    apuIrqPending(): boolean { return this.apu.irqPending(); }
    cartIrqPending(): boolean { return this.cartridge !== null && this.cartridge.irqPending(); }

    clockCartCpu(cpuCycles: number): void {
        if (this.cartridge !== null) this.cartridge.clockCpu(cpuCycles);
    }

    expansionAudioSample(): number {
        return this.cartridge !== null ? this.cartridge.expansionAudioSample() : 0.0;
    }

    cartResetScanlineCounter(): void {
        if (this.cartridge !== null) this.cartridge.resetScanlineCounter();
    }

    private chrRead = (addr: number): number => {
        return this.cartridge !== null ? this.cartridge.readChrLatched(addr) : 0;
    };

    stepPpu(cycles: number): boolean {
        let nmi = false;
        const prerender = scanlinePrerender(this.ppu.region);
        const rendering = this.ppu.isRendering();
        for (let i = 0; i < cycles; ++i) {
            if (this.ppu.stepRendered(this.chrRead)) nmi = true;
            const cyc = this.ppu.cycle;
            const sl = this.ppu.scanline;
            if (rendering && cyc === Bus.Mmc3IrqClockCycle
                && (sl < Ppu.ScreenHeight || sl === prerender)) {
                if (this.cartridge !== null) this.cartridge.clockIrq();
            }
            if (sl === prerender && cyc === 1) {
                if (this.cartridge !== null) this.cartridge.resetScanlineCounter();
            }
        }
        return nmi;
    }

    takeNmiRequest(): boolean { return this.ppu.takeNmiRequest(); }

    renderFrame(): void {
        PpuRender.renderFrame(this.ppu, this.chrRead);
    }
}
