// Picture Processing Unit (port of cores/c/src/ppu.c).

import { Region, scanlinesPerFrame, scanlinePrerender, isPalPalette } from './region';
import { Mirroring } from './mapper';
import { RenderPipeline, PpuRender } from './ppu_render';

export class Ppu {
    static readonly VramSize4k = 0x1000;
    static readonly VramSize2k = 0x0800;
    static readonly OamSize = 256;
    static readonly PaletteSize = 32;
    static readonly ScanlinesPerFrameNtsc = 262;
    static readonly CyclesPerScanline = 341;
    static readonly ScanlineVblankStart = 241;
    static readonly ScanlinePrerenderNtsc = 261;

    static readonly CtrlNmi = 0x80;
    static readonly CtrlIncrement32 = 0x04;
    static readonly CtrlSpritePattern1000 = 0x08;
    static readonly CtrlBgPattern1000 = 0x10;
    static readonly CtrlSpriteSize16 = 0x20;
    static readonly CtrlBaseNtMask = 0x03;

    static readonly MaskShowBgLeft = 0x02;
    static readonly MaskShowSpritesLeft = 0x04;
    static readonly MaskShowBg = 0x08;
    static readonly MaskShowSprites = 0x10;

    static readonly StatusVblank = 0x80;
    static readonly StatusSpriteZero = 0x40;
    static readonly StatusOverflow = 0x20;
    static readonly StatusFlagMask = 0xE0;
    static readonly StatusOpenBusMask = 0x1F;

    static readonly NtSelectMask = 0x0C00;
    static readonly CoarseXMask = 0x001F;
    static readonly CoarseYMask = 0x03E0;
    static readonly FineYMask = 0x7000;
    static readonly NtHBit = 0x0400;
    static readonly NtVBit = 0x0800;

    static readonly ScreenWidth = 256;
    static readonly ScreenHeight = 240;
    static readonly FramebufferSize = Ppu.ScreenWidth * Ppu.ScreenHeight;

    // Internal timing constants
    static readonly VblankNmiCycle = 1;
    static readonly VertScrollIncCycle = 256;
    static readonly HCopyCycle = 257;
    static readonly VCopyCycleStart = 280;
    static readonly VCopyCycleEnd = 304;
    static readonly HScrollIncStep = 8;
    static readonly HScrollIncLast = 248;

    ppuCtrl: number = 0;
    ppuMask: number = 0;
    oamAddr: number = 0;
    ppuStatus: number = 0;
    v: number = 0;
    t: number = 0;
    fineX: number = 0;
    w: boolean = false;
    ppuDataBuffer: number = 0;
    openBus: number = 0;

    vram: Uint8Array = new Uint8Array(Ppu.VramSize4k);
    vramSize: number = Ppu.VramSize2k;
    oam: Uint8Array = new Uint8Array(Ppu.OamSize);
    palette: Uint8Array = new Uint8Array(Ppu.PaletteSize);
    framebuffer: Uint32Array = new Uint32Array(Ppu.FramebufferSize);
    bgPattern: Uint8Array = new Uint8Array(Ppu.ScreenWidth);

    scanline: number = 0;
    cycle: number = 0;
    nmiRequest: boolean = false;

    mirroring: number = Mirroring.Horizontal;
    region: number = Region.Ntsc;
    render: RenderPipeline = new RenderPipeline();

    setMirroring(m: number): void {
        const needed = (m === Mirroring.FourScreen) ? Ppu.VramSize4k : Ppu.VramSize2k;
        if (this.vramSize !== needed) {
            if (needed > this.vramSize) this.vram.fill(0, this.vramSize, needed);
            this.vramSize = needed;
        }
        this.mirroring = m;
    }

    setRegion(r: number): void {
        this.region = r;
        const maxSl = scanlinesPerFrame(r);
        if (this.scanline >= maxSl) this.scanline = maxSl - 1;
    }

    setSlantCorruption(enabled: boolean): void { this.render.slantCorruption = enabled; }
    setInaccuratePalette(enabled: boolean): void { this.render.useInaccuratePalette = enabled; }
    setNmiRetrigger(enabled: boolean): void { this.render.nmiRetrigger = enabled; }

    readRegister(reg: number): number {
        switch (reg & 0x07) {
            case 0: case 1: case 3: case 5: case 6: return this.openBus;
            case 2: return this.readStatus();
            case 4: return this.readOamData();
            case 7: return this.ppuDataBuffer;
            default: return this.openBus;
        }
    }

    writeRegister(reg: number, value: number): void {
        this.openBus = value;
        switch (reg & 0x07) {
            case 0: this.writePpuCtrl(value); break;
            case 1: this.ppuMask = value; break;
            case 2: break;
            case 3: this.oamAddr = value; break;
            case 4: this.writeOamData(value); break;
            case 5: this.writePpuScroll(value); break;
            case 6: this.writePpuAddr(value); break;
            case 7: break;
        }
    }

    private readStatus(): number {
        const result = (this.ppuStatus & Ppu.StatusFlagMask) | (this.openBus & Ppu.StatusOpenBusMask);
        this.ppuStatus &= (Ppu.StatusVblank ^ 0xFF);
        this.w = false;
        this.openBus = result;
        return result;
    }

    private readOamData(): number {
        const value = this.oam[this.oamAddr];
        this.oamAddr = (this.oamAddr + 1) & 0xFF;
        this.openBus = value;
        return value;
    }

    private writeOamData(value: number): void {
        this.oam[this.oamAddr] = value;
        this.oamAddr = (this.oamAddr + 1) & 0xFF;
    }

    private writePpuCtrl(value: number): void {
        const nmiWasEnabled = (this.ppuCtrl & Ppu.CtrlNmi) !== 0;
        this.ppuCtrl = value;
        const nt = value & Ppu.CtrlBaseNtMask;
        this.t = (this.t & (Ppu.NtSelectMask ^ 0xFFFF)) | (nt << 10);
        if ((value & Ppu.CtrlNmi) !== 0 && this.inVblank()) {
            if (this.render.nmiRetrigger || !nmiWasEnabled) this.nmiRequest = true;
        }
    }

    private writePpuScroll(value: number): void {
        if (!this.w) {
            this.t = (this.t & 0xFFE0) | (value >> 3);
            this.fineX = value & 0x07;
            this.w = true;
            this.render.vDirty = true;
        } else {
            this.t = (this.t & 0x8C1F) |
                ((value & 0xF8) << 2) |
                ((value & 0x07) << 12);
            this.w = false;
        }
    }

    private writePpuAddr(value: number): void {
        if (!this.w) {
            this.t = (this.t & 0x00FF) | ((value & 0x3F) << 8);
            this.w = true;
        } else {
            this.t = (this.t & 0xFF00) | value;
            this.v = this.t;
            this.w = false;
        }
    }

    vramAddr(): number { return this.v; }
    vramIncrement(): number { return ((this.ppuCtrl & Ppu.CtrlIncrement32) !== 0) ? 32 : 1; }

    advanceVramAddr(): void {
        const inc = this.vramIncrement();
        this.v = (this.v + inc) & 0x3FFF;
    }

    ppuDataBufferValue(): number { return this.ppuDataBuffer; }
    setPpuDataBuffer(value: number): void { this.ppuDataBuffer = value; }
    openBusValue(): number { return this.openBus; }
    setOpenBus(value: number): void { this.openBus = value; }

    private mapNametable(addr: number): number {
        const a = addr & 0x2FFF;
        const local = a - 0x2000;
        const nt = local >> 10;
        const offset = local & 0x03FF;
        let phys: number;
        switch (this.mirroring) {
            case Mirroring.Horizontal: phys = nt >> 1; break;
            case Mirroring.Vertical: phys = nt & 1; break;
            case Mirroring.FourScreen: phys = nt; break;
            case Mirroring.SingleScreen0: phys = 0; break;
            case Mirroring.SingleScreen1: phys = 1; break;
            case Mirroring.SingleScreen2: phys = 2; break;
            case Mirroring.SingleScreen3: phys = 3; break;
            default: phys = nt >> 1; break;
        }
        return phys * 0x400 + offset;
    }

    readNametable(addr: number): number { return this.vram[this.mapNametable(addr)]; }
    writeNametable(addr: number, value: number): void { this.vram[this.mapNametable(addr)] = value; }

    private static mapPalette(addr: number): number {
        const a = addr & 0x1F;
        if ((a & 0x13) === 0x10) return a & 0x0F;
        return a;
    }

    readPalette(addr: number): number { return this.palette[Ppu.mapPalette(addr)]; }
    writePalette(addr: number, value: number): void { this.palette[Ppu.mapPalette(addr)] = value; }

    oamDma(data: Uint8Array): void {
        this.oam.set(data.subarray(0, 256));
        this.oamAddr = 0;
    }

    setVblank(on: boolean): void {
        if (on) this.ppuStatus |= Ppu.StatusVblank;
        else this.ppuStatus &= (Ppu.StatusVblank ^ 0xFF);
    }

    setSpriteZeroHit(on: boolean): void {
        if (on) this.ppuStatus |= Ppu.StatusSpriteZero;
        else this.ppuStatus &= (Ppu.StatusSpriteZero ^ 0xFF);
    }

    setSpriteOverflow(on: boolean): void {
        if (on) this.ppuStatus |= Ppu.StatusOverflow;
        else this.ppuStatus &= (Ppu.StatusOverflow ^ 0xFF);
    }

    nmiEnabled(): boolean { return (this.ppuCtrl & Ppu.CtrlNmi) !== 0; }
    inVblank(): boolean { return (this.ppuStatus & Ppu.StatusVblank) !== 0; }
    spriteZeroHit(): boolean { return (this.ppuStatus & Ppu.StatusSpriteZero) !== 0; }
    spriteOverflow(): boolean { return (this.ppuStatus & Ppu.StatusOverflow) !== 0; }

    takeNmiRequest(): boolean {
        const r = this.nmiRequest;
        this.nmiRequest = false;
        return r;
    }

    private incrementHScroll(): void {
        if (this.render.slantCorruption) {
            if (this.fineX < 7) {
                this.fineX++;
                this.render.fineXStart = (this.render.fineXStart + 1) & 0x07;
            } else {
                this.fineX = 0;
                this.render.fineXStart = 0;
                const coarseX = this.v & Ppu.CoarseXMask;
                if (coarseX === 31) {
                    this.v &= (Ppu.CoarseXMask ^ 0xFFFF);
                    this.v ^= Ppu.NtHBit;
                    this.render.coarseXStart = 0;
                    this.render.ntHStart ^= Ppu.NtHBit;
                } else {
                    this.v = (this.v & (Ppu.CoarseXMask ^ 0xFFFF)) | (coarseX + 1);
                    this.render.coarseXStart = (coarseX + 1) & Ppu.CoarseXMask;
                }
            }
        } else {
            const coarseX = this.v & Ppu.CoarseXMask;
            if (coarseX === 31) {
                this.v &= (Ppu.CoarseXMask ^ 0xFFFF);
                this.v ^= Ppu.NtHBit;
            } else {
                this.v = (this.v & (Ppu.CoarseXMask ^ 0xFFFF)) | (coarseX + 1);
            }
        }
    }

    private incrementVScroll(): void {
        const fineY = (this.v & Ppu.FineYMask) >> 12;
        if (fineY < 7) {
            this.v = (this.v & (Ppu.FineYMask ^ 0xFFFF)) | ((fineY + 1) << 12);
        } else {
            this.v &= (Ppu.FineYMask ^ 0xFFFF);
            const coarseY = (this.v & Ppu.CoarseYMask) >> 5;
            if (coarseY === 29) {
                this.v &= (Ppu.CoarseYMask ^ 0xFFFF);
                this.v ^= Ppu.NtVBit;
            } else if (coarseY === 31) {
                this.v &= (Ppu.CoarseYMask ^ 0xFFFF);
            } else {
                this.v = (this.v & (Ppu.CoarseYMask ^ 0xFFFF)) | ((coarseY + 1) << 5);
            }
        }
    }

    private copyHTtoV(): void {
        const hBits = this.t & (Ppu.CoarseXMask | Ppu.NtHBit);
        this.v = (this.v & ((Ppu.CoarseXMask | Ppu.NtHBit) ^ 0xFFFF)) | hBits;
    }

    private copyVTtoV(): void {
        const vBits = this.t & (Ppu.CoarseYMask | Ppu.NtVBit | Ppu.FineYMask);
        this.v = (this.v & ((Ppu.CoarseYMask | Ppu.NtVBit | Ppu.FineYMask) ^ 0xFFFF)) | vBits;
    }

    step(): boolean {
        let nmi = false;
        const scanlinesPerFrameVal = scanlinesPerFrame(this.region);
        const prerender = scanlinePrerender(this.region);

        this.cycle++;
        if (this.cycle >= Ppu.CyclesPerScanline) {
            this.cycle = 0;
            this.scanline++;
            if (this.scanline >= scanlinesPerFrameVal) this.scanline = 0;
        }

        if (this.scanline === Ppu.ScanlineVblankStart && this.cycle === Ppu.VblankNmiCycle) {
            this.setVblank(true);
            if (this.nmiEnabled()) {
                this.nmiRequest = true;
                nmi = true;
            }
        } else if (this.scanline === prerender && this.cycle === Ppu.VblankNmiCycle) {
            this.setVblank(false);
            this.setSpriteOverflow(false);
            this.setSpriteZeroHit(false);
            this.render.spriteZeroHit = false;
        }

        const rendering = (this.ppuMask & (Ppu.MaskShowBg | Ppu.MaskShowSprites)) !== 0;
        if (rendering) {
            const doesScrollInc = (this.scanline < Ppu.ScreenHeight) || (this.scanline === prerender);
            if (doesScrollInc &&
                this.cycle >= Ppu.HScrollIncStep &&
                this.cycle <= Ppu.HScrollIncLast &&
                (this.cycle % Ppu.HScrollIncStep) === 0) {
                this.incrementHScroll();
            }
            if (doesScrollInc && this.cycle === Ppu.VertScrollIncCycle) this.incrementVScroll();
            if (doesScrollInc && this.cycle === Ppu.HCopyCycle) this.copyHTtoV();
            if (this.scanline === prerender &&
                this.cycle >= Ppu.VCopyCycleStart &&
                this.cycle <= Ppu.VCopyCycleEnd) {
                this.copyVTtoV();
            }
        }

        return nmi;
    }

    stepRendered(chrRead: (addr: number) => number): boolean {
        const nmi = this.step();
        if (this.cycle === 0) {
            this.render.scanlineInitialized = false;
            this.render.pipelinePrimed = false;
        }
        if (this.scanline < Ppu.ScreenHeight && this.cycle >= 1 && this.cycle <= Ppu.ScreenWidth) {
            PpuRender.renderOnePixel(this, chrRead);
            this.render.renderedThisFrame = true;
        }
        return nmi;
    }

    isRendering(): boolean { return (this.ppuMask & (Ppu.MaskShowBg | Ppu.MaskShowSprites)) !== 0; }
    renderedThisFrame(): boolean { return this.render.renderedThisFrame; }
    resetRenderedFlag(): void { this.render.renderedThisFrame = false; }
}
