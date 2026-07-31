package nes.core;

// CHR pattern-table read functional interface (render.rs ChrReader trait).
// The bus implements this to route reads through the cartridge.
interface ChrReader {
    int read(int addr);
}

// DMC DMA read callback: fetches a byte from CPU memory for the DMC.
interface DmcReadFn {
    int read(int addr);
}

// Transient per-pixel rendering pipeline state (render.rs RenderPipeline).
final class RenderPipeline {
    public static final int MAX_SPRITES_PER_SCANLINE = 8;

    public static final class ScanlineSprite {
        public int oamI;
        public int y;
        public int tile;
        public int attr;
        public int sx;
    }

    public static final class BgFetch {
        public int pattern;
        public int palSelect;
    }

    public int coarseY;
    public int fineY;
    public int ntV;

    public int coarseXStart;
    public int ntHStart;
    public int fineXStart;
    public int resyncPx;

    public boolean scanlineInitialized;
    public boolean vDirty;
    public boolean renderedThisFrame;

    public final ScanlineSprite[] sprites = new ScanlineSprite[MAX_SPRITES_PER_SCANLINE];
    public int spriteCount;
    public boolean overflow;
    public boolean spriteZeroHit;

    public boolean slantCorruption;
    public boolean useInaccuratePalette;
    public boolean nmiRetrigger;

    public final BgFetch[] fetchBuffer = new BgFetch[2];
    public int fetchIdx;
    public boolean pipelinePrimed;

    public RenderPipeline() {
        for (int i = 0; i < MAX_SPRITES_PER_SCANLINE; ++i) sprites[i] = new ScanlineSprite();
        fetchBuffer[0] = new BgFetch();
        fetchBuffer[1] = new BgFetch();
    }
}

// Picture Processing Unit (mod.rs struct Ppu).
public final class Ppu {
    public static final int VRAM_SIZE_4K = 0x1000;
    public static final int VRAM_SIZE_2K = 0x0800;
    public static final int OAM_SIZE = 256;
    public static final int PALETTE_SIZE = 32;
    public static final int SCANLINES_PER_FRAME_NTSC = 262;
    public static final int CYCLES_PER_SCANLINE = 341;
    public static final int SCANLINE_VBLANK_START = 241;
    public static final int SCANLINE_PRERENDER_NTSC = 261;

    public static final int CTRL_NMI = 0x80;
    public static final int CTRL_INCREMENT_32 = 0x04;
    public static final int CTRL_SPRITE_PATTERN_1000 = 0x08;
    public static final int CTRL_BG_PATTERN_1000 = 0x10;
    public static final int CTRL_SPRITE_SIZE_16 = 0x20;
    public static final int CTRL_BASE_NT_MASK = 0x03;

    public static final int MASK_SHOW_BG_LEFT = 0x02;
    public static final int MASK_SHOW_SPRITES_LEFT = 0x04;
    public static final int MASK_SHOW_BG = 0x08;
    public static final int MASK_SHOW_SPRITES = 0x10;

    public static final int STATUS_VBLANK = 0x80;
    public static final int STATUS_SPRITE_ZERO = 0x40;
    public static final int STATUS_OVERFLOW = 0x20;
    public static final int STATUS_FLAG_MASK = 0xE0;
    public static final int STATUS_OPENBUS_MASK = 0x1F;

    public static final int NT_SELECT_MASK = 0x0C00;
    public static final int COARSE_X_MASK = 0x001F;
    public static final int COARSE_Y_MASK = 0x03E0;
    public static final int FINE_Y_MASK = 0x7000;
    public static final int NT_H_BIT = 0x0400;
    public static final int NT_V_BIT = 0x0800;

    public static final int SCREEN_WIDTH = 256;
    public static final int SCREEN_HEIGHT = 240;
    public static final int FRAMEBUFFER_SIZE = SCREEN_WIDTH * SCREEN_HEIGHT;

    private static final int VBLANK_NMI_CYCLE = 1;
    private static final int VERT_SCROLL_INC_CYCLE = 256;
    private static final int H_COPY_CYCLE = 257;
    private static final int V_COPY_CYCLE_START = 280;
    private static final int V_COPY_CYCLE_END = 304;
    private static final int H_SCROLL_INC_STEP = 8;
    private static final int H_SCROLL_INC_LAST = 248;

    public int ppuCtrl;
    public int ppuMask;
    public int oamAddr;
    public int ppuStatus;
    public int v;
    public int t;
    public int fineX;
    public boolean w;
    public int ppuDataBuffer;
    public int openBus;

    public final byte[] vram = new byte[VRAM_SIZE_4K];
    public int vramSize = VRAM_SIZE_2K;
    public final byte[] oam = new byte[OAM_SIZE];
    public final byte[] palette = new byte[PALETTE_SIZE];
    public final int[] framebuffer = new int[FRAMEBUFFER_SIZE];
    public final byte[] bgPattern = new byte[SCREEN_WIDTH];

    public int scanline;
    public int cycle;
    public boolean nmiRequest;

    public int mirroring = Mirroring.HORIZONTAL;
    public int region = Region.NTSC;
    public final RenderPipeline render = new RenderPipeline();

    public Ppu() {}

    public void setMirroring(int m) {
        int needed = (m == Mirroring.FOUR_SCREEN) ? VRAM_SIZE_4K : VRAM_SIZE_2K;
        if (vramSize != needed) {
            if (needed > vramSize)
                for (int i = vramSize; i < needed; ++i) vram[i] = 0;
            vramSize = needed;
        }
        mirroring = m;
    }

    public void setRegion(int r) {
        region = r;
        int maxSl = RegionTiming.scanlinesPerFrame(r);
        if (scanline >= maxSl) scanline = maxSl - 1;
    }

    public void setSlantCorruption(boolean enabled) { render.slantCorruption = enabled; }
    public void setInaccuratePalette(boolean enabled) { render.useInaccuratePalette = enabled; }
    public void setNmiRetrigger(boolean enabled) { render.nmiRetrigger = enabled; }

    public int readRegister(int reg) {
        switch (reg & 0x07) {
            case 0: case 1: case 3: case 5: case 6: return openBus;
            case 2: return readStatus();
            case 4: return readOamData();
            case 7: return ppuDataBuffer;
            default: return openBus;
        }
    }

    public void writeRegister(int reg, int value) {
        openBus = value & 0xFF;
        switch (reg & 0x07) {
            case 0: writePpuCtrl(value & 0xFF); break;
            case 1: ppuMask = value & 0xFF; break;
            case 2: break;
            case 3: oamAddr = value & 0xFF; break;
            case 4: writeOamData(value & 0xFF); break;
            case 5: writePpuScroll(value & 0xFF); break;
            case 6: writePpuAddr(value & 0xFF); break;
            case 7: break;
        }
    }

    private int readStatus() {
        int result = ((ppuStatus & STATUS_FLAG_MASK) | (openBus & STATUS_OPENBUS_MASK)) & 0xFF;
        ppuStatus &= ~STATUS_VBLANK;
        w = false;
        openBus = result;
        return result;
    }

    private int readOamData() {
        int value = oam[oamAddr & 0xFF] & 0xFF;
        oamAddr = (oamAddr + 1) & 0xFF;
        openBus = value;
        return value;
    }

    private void writeOamData(int value) {
        oam[oamAddr & 0xFF] = (byte) value;
        oamAddr = (oamAddr + 1) & 0xFF;
    }

    private void writePpuCtrl(int value) {
        boolean nmiWasEnabled = (ppuCtrl & CTRL_NMI) != 0;
        ppuCtrl = value;
        int nt = value & CTRL_BASE_NT_MASK;
        t = (t & ~NT_SELECT_MASK) | (nt << 10);
        if ((value & CTRL_NMI) != 0 && inVblank()) {
            if (render.nmiRetrigger || !nmiWasEnabled) nmiRequest = true;
        }
    }

    private void writePpuScroll(int value) {
        if (!w) {
            t = (t & 0xFFE0) | (value >> 3);
            fineX = value & 0x07;
            w = true;
            render.vDirty = true;
        } else {
            t = (t & 0x8C1F) | ((value & 0xF8) << 2) | ((value & 0x07) << 12);
            w = false;
        }
    }

    private void writePpuAddr(int value) {
        if (!w) {
            t = (t & 0x00FF) | ((value & 0x3F) << 8);
            w = true;
        } else {
            t = (t & 0xFF00) | value;
            v = t;
            w = false;
        }
    }

    public int vramAddr() { return v; }
    public int vramIncrement() { return ((ppuCtrl & CTRL_INCREMENT_32) != 0) ? 32 : 1; }

    public void advanceVramAddr() {
        int inc = vramIncrement();
        v = (v + inc) & 0x3FFF;
    }

    public int ppuDataBufferValue() { return ppuDataBuffer; }
    public void setPpuDataBuffer(int value) { ppuDataBuffer = value & 0xFF; }
    public int openBusValue() { return openBus; }
    public void setOpenBus(int value) { openBus = value & 0xFF; }

    private int mapNametable(int addr) {
        int a = addr & 0x2FFF;
        int local = a - 0x2000;
        int nt = local >> 10;
        int offset = local & 0x03FF;
        int phys;
        switch (mirroring) {
            case Mirroring.HORIZONTAL: phys = nt >> 1; break;
            case Mirroring.VERTICAL: phys = nt & 1; break;
            case Mirroring.FOUR_SCREEN: phys = nt; break;
            case Mirroring.SINGLE_SCREEN0: phys = 0; break;
            case Mirroring.SINGLE_SCREEN1: phys = 1; break;
            case Mirroring.SINGLE_SCREEN2: phys = 2; break;
            case Mirroring.SINGLE_SCREEN3: phys = 3; break;
            default: phys = nt >> 1; break;
        }
        return phys * 0x400 + offset;
    }

    public int readNametable(int addr) { return vram[mapNametable(addr)] & 0xFF; }
    public void writeNametable(int addr, int value) { vram[mapNametable(addr)] = (byte) value; }

    private static int mapPalette(int addr) {
        int a = addr & 0x1F;
        if ((a & 0x13) == 0x10) return a & 0x0F;
        return a;
    }

    public int readPalette(int addr) { return palette[mapPalette(addr)] & 0xFF; }
    public void writePalette(int addr, int value) { palette[mapPalette(addr)] = (byte) value; }

    public void oamDma(byte[] data) {
        System.arraycopy(data, 0, oam, 0, 256);
        oamAddr = 0;
    }

    public void setVblank(boolean on) {
        if (on) ppuStatus |= STATUS_VBLANK;
        else ppuStatus &= ~STATUS_VBLANK;
    }

    public void setSpriteZeroHit(boolean on) {
        if (on) ppuStatus |= STATUS_SPRITE_ZERO;
        else ppuStatus &= ~STATUS_SPRITE_ZERO;
    }

    public void setSpriteOverflow(boolean on) {
        if (on) ppuStatus |= STATUS_OVERFLOW;
        else ppuStatus &= ~STATUS_OVERFLOW;
    }

    public boolean nmiEnabled() { return (ppuCtrl & CTRL_NMI) != 0; }
    public boolean inVblank() { return (ppuStatus & STATUS_VBLANK) != 0; }
    public boolean spriteZeroHit() { return (ppuStatus & STATUS_SPRITE_ZERO) != 0; }
    public boolean spriteOverflow() { return (ppuStatus & STATUS_OVERFLOW) != 0; }

    public boolean takeNmiRequest() {
        boolean r = nmiRequest;
        nmiRequest = false;
        return r;
    }

    private void incrementHScroll() {
        if (render.slantCorruption) {
            if (fineX < 7) {
                fineX++;
                render.fineXStart = (render.fineXStart + 1) & 0x07;
            } else {
                fineX = 0;
                render.fineXStart = 0;
                int coarseX = v & COARSE_X_MASK;
                if (coarseX == 31) {
                    v &= ~COARSE_X_MASK;
                    v ^= NT_H_BIT;
                    render.coarseXStart = 0;
                    render.ntHStart ^= NT_H_BIT;
                } else {
                    v = (v & ~COARSE_X_MASK) | (coarseX + 1);
                    render.coarseXStart = (coarseX + 1) & COARSE_X_MASK;
                }
            }
        } else {
            int coarseX = v & COARSE_X_MASK;
            if (coarseX == 31) {
                v &= ~COARSE_X_MASK;
                v ^= NT_H_BIT;
            } else {
                v = (v & ~COARSE_X_MASK) | (coarseX + 1);
            }
        }
    }

    private void incrementVScroll() {
        int fineYv = (v & FINE_Y_MASK) >> 12;
        if (fineYv < 7) {
            v = (v & ~FINE_Y_MASK) | ((fineYv + 1) << 12);
        } else {
            v &= ~FINE_Y_MASK;
            int coarseY = (v & COARSE_Y_MASK) >> 5;
            if (coarseY == 29) {
                v &= ~COARSE_Y_MASK;
                v ^= NT_V_BIT;
            } else if (coarseY == 31) {
                v &= ~COARSE_Y_MASK;
            } else {
                v = (v & ~COARSE_Y_MASK) | ((coarseY + 1) << 5);
            }
        }
    }

    private void copyHToV() {
        int hBits = t & (COARSE_X_MASK | NT_H_BIT);
        v = (v & ~(COARSE_X_MASK | NT_H_BIT)) | hBits;
    }

    private void copyVToV() {
        int vBits = t & (COARSE_Y_MASK | NT_V_BIT | FINE_Y_MASK);
        v = (v & ~(COARSE_Y_MASK | NT_V_BIT | FINE_Y_MASK)) | vBits;
    }

    public boolean step() {
        boolean nmi = false;
        int scanlinesPerFrame = RegionTiming.scanlinesPerFrame(region);
        int prerender = RegionTiming.scanlinePrerender(region);

        cycle++;
        if (cycle >= CYCLES_PER_SCANLINE) {
            cycle = 0;
            scanline++;
            if (scanline >= scanlinesPerFrame) scanline = 0;
        }

        if (scanline == SCANLINE_VBLANK_START && cycle == VBLANK_NMI_CYCLE) {
            setVblank(true);
            if (nmiEnabled()) {
                nmiRequest = true;
                nmi = true;
            }
        } else if (scanline == prerender && cycle == VBLANK_NMI_CYCLE) {
            setVblank(false);
            setSpriteOverflow(false);
            setSpriteZeroHit(false);
            render.spriteZeroHit = false;
        }

        boolean rendering = (ppuMask & (MASK_SHOW_BG | MASK_SHOW_SPRITES)) != 0;
        if (rendering) {
            boolean doesScrollInc = (scanline < SCREEN_HEIGHT) || (scanline == prerender);
            if (doesScrollInc &&
                cycle >= H_SCROLL_INC_STEP &&
                cycle <= H_SCROLL_INC_LAST &&
                (cycle % H_SCROLL_INC_STEP) == 0) {
                incrementHScroll();
            }
            if (doesScrollInc && cycle == VERT_SCROLL_INC_CYCLE) incrementVScroll();
            if (doesScrollInc && cycle == H_COPY_CYCLE) copyHToV();
            if (scanline == prerender &&
                cycle >= V_COPY_CYCLE_START &&
                cycle <= V_COPY_CYCLE_END) {
                copyVToV();
            }
        }

        return nmi;
    }

    public boolean stepRendered(ChrReader chrRead) {
        boolean nmi = step();
        if (cycle == 0) {
            render.scanlineInitialized = false;
            render.pipelinePrimed = false;
        }
        if (scanline < SCREEN_HEIGHT && cycle >= 1 && cycle <= SCREEN_WIDTH) {
            PpuRender.renderOnePixel(this, chrRead);
            render.renderedThisFrame = true;
        }
        return nmi;
    }

    public boolean isRendering() { return (ppuMask & (MASK_SHOW_BG | MASK_SHOW_SPRITES)) != 0; }
    public boolean renderedThisFrame() { return render.renderedThisFrame; }
    public void resetRenderedFlag() { render.renderedThisFrame = false; }
}
