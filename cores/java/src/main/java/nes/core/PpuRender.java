package nes.core;

// PPU rendering pipeline: background + sprite pixel generation.
// Port of cores/csharp/src/PpuRender.cs.
public final class PpuRender {
    private static final int ATTR_TABLE_OFFSET = 0x03C0;
    private static final int NT_BASE = 0x2000;
    private static final int PAL_BASE = 0x3F00;
    private static final int SPRITE_PAL_BASE = 0x3F10;
    private static final int ATTR_PALETTE_MASK = 0x03;
    private static final int ATTR_PRIORITY_BEHIND = 0x20;
    private static final int ATTR_HFLIP = 0x40;
    private static final int ATTR_VFLIP = 0x80;
    private static final int SPRITE_COUNT = 64;
    private static final int SPRITE_HEIGHT_8X8 = 8;
    private static final int SPRITE_HEIGHT_8X16 = 16;
    private static final int SPRITE_WIDTH = 8;
    private static final int OAM_Y_HIDDEN = 0xEF;
    private static final int OAM_Y_HALT = 0xFF;
    private static final int SPRITE_ZERO_HIT_MAX_X = 255;
    private static final int ALPHA = 0xFF;

    // Palettes stored as int[64] pre-computed ARGB for fast lookup.
    private static final int[] NES_ARGB = new int[64];
    private static final int[] NES_INACCURATE_ARGB = new int[64];
    private static final int[] PAL_ARGB = new int[64];

    private static final int[][] NES_RGB = {
        {0x7C,0x7C,0x7C},{0x00,0x00,0xFC},{0x00,0x00,0xBC},{0x44,0x28,0xBC},
        {0x94,0x00,0x84},{0xA8,0x00,0x20},{0xA8,0x10,0x00},{0x88,0x14,0x00},
        {0x50,0x30,0x00},{0x00,0x78,0x00},{0x00,0x68,0x00},{0x00,0x58,0x00},
        {0x00,0x40,0x58},{0x00,0x00,0x00},{0x00,0x00,0x00},{0x00,0x00,0x00},
        {0xBC,0xBC,0xBC},{0x00,0x78,0xF8},{0x00,0x58,0xF8},{0x68,0x44,0xFC},
        {0xD8,0x00,0xCC},{0xE4,0x00,0x58},{0xF8,0x38,0x00},{0xE4,0x5C,0x10},
        {0xAC,0x7C,0x00},{0x00,0xB8,0x00},{0x00,0xA8,0x00},{0x00,0xA8,0x44},
        {0x00,0x88,0x88},{0x00,0x00,0x00},{0x00,0x00,0x00},{0x00,0x00,0x00},
        {0xF8,0xF8,0xF8},{0x3C,0xBC,0xFC},{0x68,0x88,0xFC},{0x98,0x78,0xF8},
        {0xF8,0x78,0xF8},{0xF8,0x58,0x98},{0xF8,0x78,0x58},{0xFC,0xA0,0x44},
        {0xF8,0xB8,0x00},{0xB8,0xF8,0x18},{0x58,0xD8,0x54},{0x58,0xF8,0x98},
        {0x00,0xE8,0xD8},{0x78,0x78,0x78},{0x00,0x00,0x00},{0x00,0x00,0x00},
        {0xFC,0xFC,0xFC},{0xA4,0xE4,0xFC},{0xB8,0xB8,0xF8},{0xD8,0xB8,0xF8},
        {0xF8,0xB8,0xF8},{0xF8,0xA4,0xC0},{0xF0,0xD0,0xB0},{0xFC,0xE0,0xA8},
        {0xF8,0xD8,0x78},{0xD8,0xF8,0x78},{0xB8,0xF8,0xB8},{0xB8,0xF8,0xD8},
        {0x00,0xFC,0xFC},{0xF8,0xD8,0xF8},{0x00,0x00,0x00},{0x00,0x00,0x00}
    };
    private static final int[][] NES_INACCURATE_RGB = {
        {0x84,0x84,0x84},{0x00,0x1D,0x2C},{0x1C,0x0C,0x54},{0x30,0x04,0x64},
        {0x48,0x00,0x5C},{0x58,0x00,0x44},{0x58,0x00,0x24},{0x4C,0x0C,0x00},
        {0x38,0x18,0x00},{0x20,0x28,0x00},{0x0C,0x3C,0x00},{0x00,0x40,0x00},
        {0x00,0x3C,0x1C},{0x00,0x38,0x3C},{0x00,0x00,0x00},{0x00,0x00,0x00},
        {0xB4,0xB4,0xB4},{0x38,0x6C,0xBC},{0x54,0x58,0xEC},{0x70,0x44,0xF4},
        {0x90,0x38,0xE4},{0xA8,0x34,0xC8},{0xB8,0x34,0x88},{0xC0,0x34,0x44},
        {0xC4,0x40,0x14},{0xC8,0x50,0x00},{0xA8,0x60,0x00},{0x88,0x70,0x00},
        {0x6C,0x80,0x00},{0x44,0x88,0x00},{0x00,0x94,0x00},{0x00,0x00,0x00},
        {0xFF,0xFF,0xFF},{0x9C,0xDC,0xFF},{0xB8,0xB8,0xFF},{0xD0,0xB8,0xFF},
        {0xFF,0xB0,0xF4},{0xFF,0xA8,0xE0},{0xFF,0xA4,0xC0},{0xFF,0xA0,0x90},
        {0xF8,0x94,0x58},{0xF0,0xA0,0x38},{0xD8,0xA8,0x20},{0xB8,0xB0,0x14},
        {0x98,0xB8,0x14},{0x70,0xC0,0x14},{0x50,0xC8,0x24},{0x00,0x00,0x00},
        {0xFF,0xFF,0xFF},{0x9C,0xDC,0xFF},{0xB8,0xB8,0xFF},{0xD0,0xB8,0xFF},
        {0xFF,0xB0,0xF4},{0xFF,0xA8,0xE0},{0xFF,0xA4,0xC0},{0xFF,0xA0,0x90},
        {0xF8,0x94,0x58},{0xF0,0xA0,0x38},{0xD8,0xA8,0x20},{0xB8,0xB0,0x14},
        {0x98,0xB8,0x14},{0x70,0xC0,0x14},{0x50,0xC8,0x24},{0x00,0x00,0x00}
    };
    private static final int[][] PAL_RGB = {
        {0x84,0x84,0x84},{0x00,0x1D,0x2C},{0x0C,0x0C,0x44},{0x24,0x04,0x54},
        {0x3C,0x00,0x4C},{0x4C,0x00,0x34},{0x4C,0x00,0x18},{0x40,0x0C,0x00},
        {0x2C,0x18,0x00},{0x18,0x28,0x00},{0x08,0x3C,0x00},{0x00,0x40,0x00},
        {0x00,0x3C,0x1C},{0x00,0x38,0x3C},{0x04,0x04,0x04},{0x00,0x00,0x00},
        {0xB4,0xB4,0xB4},{0x30,0x60,0xA4},{0x48,0x48,0xC8},{0x60,0x38,0xD8},
        {0x80,0x30,0xC8},{0x98,0x2C,0xAC},{0xA8,0x2C,0x70},{0xB0,0x2C,0x38},
        {0xB4,0x38,0x10},{0xB8,0x48,0x00},{0x98,0x58,0x00},{0x78,0x68,0x00},
        {0x5C,0x78,0x00},{0x38,0x80,0x00},{0x00,0x8C,0x00},{0x04,0x04,0x04},
        {0xFF,0xFF,0xFF},{0x8C,0xCC,0xFF},{0xA8,0xA8,0xFF},{0xC0,0xA8,0xFF},
        {0xE8,0xA4,0xF0},{0xE8,0xA0,0xD8},{0xE8,0x9C,0xB8},{0xE8,0x98,0x88},
        {0xE0,0x8C,0x54},{0xD8,0x98,0x38},{0xC0,0xA0,0x24},{0xA0,0xA8,0x18},
        {0x84,0xB0,0x18},{0x60,0xB8,0x18},{0x44,0xC0,0x2C},{0x04,0x04,0x04},
        {0xFF,0xFF,0xFF},{0x8C,0xCC,0xFF},{0xA8,0xA8,0xFF},{0xC0,0xA8,0xFF},
        {0xE8,0xA4,0xF0},{0xE8,0xA0,0xD8},{0xE8,0x9C,0xB8},{0xE8,0x98,0x88},
        {0xE0,0x8C,0x54},{0xD8,0x98,0x38},{0xC0,0xA0,0x24},{0xA0,0xA8,0x18},
        {0x84,0xB0,0x18},{0x60,0xB8,0x18},{0x44,0xC0,0x2C},{0x04,0x04,0x04}
    };

    static {
        for (int i = 0; i < 64; ++i) {
            NES_ARGB[i] = (ALPHA << 24) | (NES_RGB[i][0] << 16) | (NES_RGB[i][1] << 8) | NES_RGB[i][2];
            NES_INACCURATE_ARGB[i] = (ALPHA << 24) | (NES_INACCURATE_RGB[i][0] << 16) | (NES_INACCURATE_RGB[i][1] << 8) | NES_INACCURATE_RGB[i][2];
            PAL_ARGB[i] = (ALPHA << 24) | (PAL_RGB[i][0] << 16) | (PAL_RGB[i][1] << 8) | PAL_RGB[i][2];
        }
    }

    private PpuRender() {}

    public static int nesColorToArgbFor(int index, int region) {
        int idx = index & 0x3F;
        return RegionTiming.isPalPalette(region) ? PAL_ARGB[idx] : NES_ARGB[idx];
    }
    public static int nesColorToArgb(int index) { return nesColorToArgbFor(index, Region.NTSC); }

    private static int colorToArgb(Ppu p, int index) {
        int idx = index & 0x3F;
        if (RegionTiming.isPalPalette(p.region)) return PAL_ARGB[idx];
        if (p.render.useInaccuratePalette) return NES_INACCURATE_ARGB[idx];
        return NES_ARGB[idx];
    }

    public static int universalBgArgb(Ppu p) { return colorToArgb(p, p.readPalette(PAL_BASE)); }

    public static void clearFramebuffer(Ppu p, int argb) {
        int[] fb = p.framebuffer;
        for (int i = 0; i < Ppu.FRAMEBUFFER_SIZE; ++i) fb[i] = argb;
    }

    public static int pixel(Ppu p, int x, int y) {
        if (x < Ppu.SCREEN_WIDTH && y < Ppu.SCREEN_HEIGHT)
            return p.framebuffer[y * Ppu.SCREEN_WIDTH + x];
        return universalBgArgb(p);
    }

    public static void renderBackground(Ppu p, ChrReader chr) {
        for (int py = 0; py < Ppu.SCREEN_HEIGHT; ++py)
            renderBackgroundScanline(p, py, chr);
    }

    public static void renderSprites(Ppu p, ChrReader chr) {
        p.setSpriteOverflow(false);
        p.setSpriteZeroHit(false);
        boolean[] overflowThisFrame = {false};
        boolean[] spriteZeroHitSet = {false};
        for (int sl = 0; sl < Ppu.SCREEN_HEIGHT; ++sl)
            renderSpritesScanline(p, sl, chr, overflowThisFrame, spriteZeroHitSet);
        if (overflowThisFrame[0]) p.setSpriteOverflow(true);
    }

    public static void renderFrame(Ppu p, ChrReader chr) {
        p.setSpriteOverflow(false);
        p.setSpriteZeroHit(false);
        boolean[] overflowThisFrame = {false};
        boolean[] spriteZeroHitSet = {false};
        for (int py = 0; py < Ppu.SCREEN_HEIGHT; ++py) {
            renderBackgroundScanline(p, py, chr);
            renderSpritesScanline(p, py, chr, overflowThisFrame, spriteZeroHitSet);
        }
        if (overflowThisFrame[0]) p.setSpriteOverflow(true);
    }

    private static void renderBackgroundScanline(Ppu p, int py, ChrReader chr) {
        boolean bgEnabled = (p.ppuMask & Ppu.MASK_SHOW_BG) != 0;
        boolean bgLeftEnabled = (p.ppuMask & Ppu.MASK_SHOW_BG_LEFT) != 0;
        int universalBg = universalBgArgb(p);
        int rowBase = py * Ppu.SCREEN_WIDTH;
        if (!bgEnabled) {
            for (int px = 0; px < Ppu.SCREEN_WIDTH; ++px) { p.framebuffer[rowBase + px] = universalBg; p.bgPattern[px] = 0; }
            return;
        }
        int bgTable = ((p.ppuCtrl & Ppu.CTRL_BG_PATTERN_1000) != 0) ? 0x1000 : 0x0000;
        int baseNt = (p.ppuCtrl & Ppu.CTRL_BASE_NT_MASK) << 10;
        int coarseX = p.t & 0x001F;
        int coarseY = (p.t >> 5) & 0x001F;
        int fineYv = (p.t >> 12) & 0x0007;
        int scrollX = (coarseX << 3) | p.fineX;
        int scrollY = (coarseY << 3) | fineYv;
        int gy = py + scrollY;
        fineYv = gy & 0x0007;
        int tileRow = (gy >> 3) & 0x001F;
        int ntV = (gy >> 8) & 1;
        for (int px = 0; px < Ppu.SCREEN_WIDTH; ++px) {
            if (px < 8 && !bgLeftEnabled) { p.framebuffer[rowBase + px] = universalBg; p.bgPattern[px] = 0; continue; }
            int gx = px + scrollX;
            int fineX = gx & 0x0007;
            int tileCol = (gx >> 3) & 0x001F;
            int ntH = (gx >> 8) & 1;
            int nt = ((baseNt >> 10) ^ ntH ^ (ntV << 1)) & 0x03;
            int ntAddr = NT_BASE | (nt << 10) | (tileRow << 5) | tileCol;
            int tileIndex = p.readNametable(ntAddr);
            int patternAddr = bgTable | (tileIndex << 4) | fineYv;
            int plane0 = chr.read(patternAddr) & 0xFF;
            int plane1 = chr.read(patternAddr | 0x08) & 0xFF;
            int bit = 7 - fineX;
            int pattern = ((plane0 >> bit) & 1) | (((plane1 >> bit) & 1) << 1);
            int attrCol = tileCol >> 2;
            int attrRow = tileRow >> 2;
            int attrAddr = NT_BASE | (nt << 10) | ATTR_TABLE_OFFSET | (attrRow << 3) | attrCol;
            int attrByte = p.readNametable(attrAddr);
            int shift = ((tileRow & 0x02) << 1) | (tileCol & 0x02);
            int palSelect = (attrByte >> shift) & 0x03;
            p.bgPattern[px] = (byte) pattern;
            int colorAddr = (pattern == 0) ? PAL_BASE : (PAL_BASE | (palSelect << 2) | pattern);
            int nesIndex = p.readPalette(colorAddr);
            p.framebuffer[rowBase + px] = colorToArgb(p, nesIndex);
        }
    }

    private static void renderSpritesScanline(Ppu p, int scanline, ChrReader chr,
                                              boolean[] overflowThisFrame, boolean[] spriteZeroHitSet) {
        boolean spritesEnabled = (p.ppuMask & Ppu.MASK_SHOW_SPRITES) != 0;
        if (!spritesEnabled) return;
        boolean spritesLeftEnabled = (p.ppuMask & Ppu.MASK_SHOW_SPRITES_LEFT) != 0;
        boolean spriteSize16 = (p.ppuCtrl & Ppu.CTRL_SPRITE_SIZE_16) != 0;
        int spriteHeight = spriteSize16 ? SPRITE_HEIGHT_8X16 : SPRITE_HEIGHT_8X8;
        int spriteTable8x8 = ((p.ppuCtrl & Ppu.CTRL_SPRITE_PATTERN_1000) != 0) ? 0x1000 : 0x0000;
        RenderPipeline.ScanlineSprite[] selected = new RenderPipeline.ScanlineSprite[RenderPipeline.MAX_SPRITES_PER_SCANLINE];
        for (int i = 0; i < RenderPipeline.MAX_SPRITES_PER_SCANLINE; ++i) selected[i] = new RenderPipeline.ScanlineSprite();
        int count = 0;
        for (int i = 0; i < SPRITE_COUNT; ++i) {
            int oamIdx = i * 4;
            int y = p.oam[oamIdx] & 0xFF;
            if (y == OAM_Y_HALT) break;
            if (y >= OAM_Y_HIDDEN) continue;
            int top = y + 1;
            if (scanline < top || scanline >= top + spriteHeight) continue;
            if (count < RenderPipeline.MAX_SPRITES_PER_SCANLINE) {
                selected[count].oamI = i; selected[count].y = y;
                selected[count].tile = p.oam[oamIdx + 1] & 0xFF;
                selected[count].attr = p.oam[oamIdx + 2] & 0xFF;
                selected[count].sx = p.oam[oamIdx + 3] & 0xFF;
                count++;
            } else overflowThisFrame[0] = true;
        }
        int rowBase = scanline * Ppu.SCREEN_WIDTH;
        for (int px = 0; px < Ppu.SCREEN_WIDTH; ++px) {
            if (px < 8 && !spritesLeftEnabled) continue;
            for (int idx = 0; idx < count; ++idx) {
                int oamI = selected[idx].oamI, y = selected[idx].y, tile = selected[idx].tile;
                int attr = selected[idx].attr, sx = selected[idx].sx;
                if (px < sx || px >= sx + SPRITE_WIDTH) continue;
                int tileCol = px - sx, tileRow = scanline - (y + 1);
                int row = (attr & ATTR_VFLIP) != 0 ? (spriteHeight - 1) - tileRow : tileRow;
                int col = (attr & ATTR_HFLIP) != 0 ? 7 - tileCol : tileCol;
                int table, tileBase;
                if (spriteSize16) { table = (tile & 1) != 0 ? 0x1000 : 0x0000; tileBase = tile & 0xFE; }
                else { table = spriteTable8x8; tileBase = tile; }
                int rowInTile = spriteSize16 ? (row & 0x07) : row;
                int tileForRow = (spriteSize16 && row >= 8) ? (tileBase + 1) : tileBase;
                int patternAddr = table | (tileForRow << 4) | rowInTile;
                int plane0 = chr.read(patternAddr) & 0xFF;
                int plane1 = chr.read(patternAddr | 0x08) & 0xFF;
                int bit = 7 - col;
                int pattern = ((plane0 >> bit) & 1) | (((plane1 >> bit) & 1) << 1);
                if (pattern == 0) continue;
                int colIdx = rowBase + px;
                boolean bgOpaque = p.bgPattern[px] != 0;
                if (!spriteZeroHitSet[0] && oamI == 0 && px < SPRITE_ZERO_HIT_MAX_X && bgOpaque) {
                    p.setSpriteZeroHit(true); spriteZeroHitSet[0] = true;
                }
                boolean behindBg = (attr & ATTR_PRIORITY_BEHIND) != 0;
                if (behindBg && bgOpaque) break;
                int pal = attr & ATTR_PALETTE_MASK;
                int colorAddr = SPRITE_PAL_BASE | (pal << 2) | pattern;
                int nesIndex = p.readPalette(colorAddr);
                p.framebuffer[colIdx] = colorToArgb(p, nesIndex);
                break;
            }
        }
    }

    private static void snapshotRenderPosition(Ppu p, int px) {
        p.render.coarseY = (p.v >> 5) & 0x001F;
        p.render.fineY = (p.v >> 12) & 0x0007;
        p.render.ntV = p.v & Ppu.NT_V_BIT;
        p.render.coarseXStart = p.v & Ppu.COARSE_X_MASK;
        p.render.ntHStart = p.v & Ppu.NT_H_BIT;
        p.render.fineXStart = p.fineX;
        p.render.resyncPx = px;
        p.render.vDirty = false;
    }

    private static int[] _effX = new int[3];
    // Reusable BgFetch for lookahead (avoids per-pixel allocation in renderOnePixel).
    private static RenderPipeline.BgFetch _lookahead = new RenderPipeline.BgFetch();
    private static void effectiveRenderX(Ppu p, int px) {
        int coarseX = p.render.coarseXStart, ntH = p.render.ntHStart;
        int fineXStart = p.render.fineXStart, advance = px - p.render.resyncPx;
        int newFineX = (fineXStart + advance) & 0x0007;
        int cxInc = (fineXStart + advance) >> 3;
        int total = coarseX + cxInc, wraps = total / 32;
        coarseX = total % 32;
        if ((wraps & 1) != 0) ntH ^= Ppu.NT_H_BIT;
        _effX[0] = coarseX; _effX[1] = ntH; _effX[2] = newFineX;
    }

    private static void fetchBgPixel(Ppu p, int px, ChrReader chr, RenderPipeline.BgFetch out) {
        effectiveRenderX(p, px);
        int coarseX = _effX[0], ntH = _effX[1], fineX = _effX[2];
        int nt = ((ntH >> 10) | (p.render.ntV >> 10)) & 0x03;
        int coarseY = p.render.coarseY;
        int ntAddr = NT_BASE | (nt << 10) | (coarseY << 5) | coarseX;
        int tileIndex = p.readNametable(ntAddr);
        int bgTable = ((p.ppuCtrl & Ppu.CTRL_BG_PATTERN_1000) != 0) ? 0x1000 : 0x0000;
        int patternAddr = bgTable | (tileIndex << 4) | p.render.fineY;
        int plane0 = chr.read(patternAddr) & 0xFF;
        int plane1 = chr.read(patternAddr | 0x08) & 0xFF;
        int bit = 7 - fineX;
        int pattern = ((plane0 >> bit) & 1) | (((plane1 >> bit) & 1) << 1);
        int attrCol = coarseX >> 2, attrRow = coarseY >> 2;
        int attrAddr = NT_BASE | (nt << 10) | ATTR_TABLE_OFFSET | (attrRow << 3) | attrCol;
        int attrByte = p.readNametable(attrAddr);
        int shift = ((coarseY & 0x02) << 1) | (coarseX & 0x02);
        out.pattern = pattern;
        out.palSelect = (attrByte >> shift) & 0x03;
    }

    private static void evaluateScanlineSprites(Ppu p) {
        p.render.spriteCount = 0; p.render.overflow = false;
        boolean spriteSize16 = (p.ppuCtrl & Ppu.CTRL_SPRITE_SIZE_16) != 0;
        int spriteHeight = spriteSize16 ? SPRITE_HEIGHT_8X16 : SPRITE_HEIGHT_8X8;
        int scanline = p.scanline;
        for (int i = 0; i < SPRITE_COUNT; ++i) {
            int oamIdx = i * 4;
            int y = p.oam[oamIdx] & 0xFF;
            if (y == OAM_Y_HALT) break;
            if (y >= OAM_Y_HIDDEN) continue;
            int top = y + 1;
            if (scanline < top || scanline >= top + spriteHeight) continue;
            if (p.render.spriteCount < RenderPipeline.MAX_SPRITES_PER_SCANLINE) {
                int idx = p.render.spriteCount;
                p.render.sprites[idx].oamI = i; p.render.sprites[idx].y = y;
                p.render.sprites[idx].tile = p.oam[oamIdx + 1] & 0xFF;
                p.render.sprites[idx].attr = p.oam[oamIdx + 2] & 0xFF;
                p.render.sprites[idx].sx = p.oam[oamIdx + 3] & 0xFF;
                p.render.spriteCount = idx + 1;
            } else p.render.overflow = true;
        }
        if (p.render.overflow) p.setSpriteOverflow(true);
    }

    private static int[] _sprOut = new int[2];
    private static boolean fetchSpritePixel(Ppu p, int px, int py, ChrReader chr) {
        boolean spriteSize16 = (p.ppuCtrl & Ppu.CTRL_SPRITE_SIZE_16) != 0;
        int spriteHeight = spriteSize16 ? SPRITE_HEIGHT_8X16 : SPRITE_HEIGHT_8X8;
        int spriteTable8x8 = ((p.ppuCtrl & Ppu.CTRL_SPRITE_PATTERN_1000) != 0) ? 0x1000 : 0x0000;
        int scanline = py;
        boolean bgOpaque = p.bgPattern[px] != 0;
        for (int idx = 0; idx < p.render.spriteCount; ++idx) {
            int oamI = p.render.sprites[idx].oamI, y = p.render.sprites[idx].y;
            int tile = p.render.sprites[idx].tile, attr = p.render.sprites[idx].attr;
            int sx = p.render.sprites[idx].sx;
            if (px < sx || px >= sx + SPRITE_WIDTH) continue;
            int tileCol = px - sx, tileRow = scanline - (y + 1);
            int row = (attr & ATTR_VFLIP) != 0 ? (spriteHeight - 1) - tileRow : tileRow;
            int col = (attr & ATTR_HFLIP) != 0 ? 7 - tileCol : tileCol;
            int table, tileBase;
            if (spriteSize16) { table = (tile & 1) != 0 ? 0x1000 : 0x0000; tileBase = tile & 0xFE; }
            else { table = spriteTable8x8; tileBase = tile; }
            int rowInTile = spriteSize16 ? (row & 0x07) : row;
            int tileForRow = (spriteSize16 && row >= 8) ? (tileBase + 1) : tileBase;
            int patternAddr = table | (tileForRow << 4) | rowInTile;
            int plane0 = chr.read(patternAddr) & 0xFF;
            int plane1 = chr.read(patternAddr | 0x08) & 0xFF;
            int bit = 7 - col;
            int pattern = ((plane0 >> bit) & 1) | (((plane1 >> bit) & 1) << 1);
            if (pattern == 0) continue;
            if (!p.render.spriteZeroHit && oamI == 0 && px < SPRITE_ZERO_HIT_MAX_X && bgOpaque) {
                p.setSpriteZeroHit(true); p.render.spriteZeroHit = true;
            }
            boolean behindBg = (attr & ATTR_PRIORITY_BEHIND) != 0;
            if (behindBg && bgOpaque) continue;
            _sprOut[0] = pattern; _sprOut[1] = attr & ATTR_PALETTE_MASK;
            return true;
        }
        return false;
    }

    public static void renderOnePixel(Ppu p, ChrReader chr) {
        int px = p.cycle - 1, py = p.scanline;
        int col = py * Ppu.SCREEN_WIDTH + px;
        if (!p.render.scanlineInitialized) {
            snapshotRenderPosition(p, px);
            evaluateScanlineSprites(p);
            p.render.scanlineInitialized = true;
        }
        if (p.render.vDirty) snapshotRenderPosition(p, px);
        boolean bgEnabled = (p.ppuMask & Ppu.MASK_SHOW_BG) != 0;
        boolean bgLeftEnabled = (p.ppuMask & Ppu.MASK_SHOW_BG_LEFT) != 0;
        boolean spritesEnabled = (p.ppuMask & Ppu.MASK_SHOW_SPRITES) != 0;
        boolean spritesLeftEnabled = (p.ppuMask & Ppu.MASK_SHOW_SPRITES_LEFT) != 0;
        int pixelArgb;
        if (!bgEnabled) {
            pixelArgb = universalBgArgb(p);
            p.bgPattern[px] = 0;
            p.render.pipelinePrimed = false;
        } else {
            if (!p.render.pipelinePrimed) {
                fetchBgPixel(p, px, chr, p.render.fetchBuffer[0]);
                fetchBgPixel(p, px + 1, chr, p.render.fetchBuffer[1]);
                p.render.fetchIdx = 0;
                p.render.pipelinePrimed = true;
            }
            int idx = p.render.fetchIdx;
            int pattern = p.render.fetchBuffer[idx].pattern;
            int palSelect = p.render.fetchBuffer[idx].palSelect;
            RenderPipeline.BgFetch lookahead = _lookahead;
            fetchBgPixel(p, px + 2, chr, lookahead);
            p.render.fetchBuffer[idx].pattern = lookahead.pattern;
            p.render.fetchBuffer[idx].palSelect = lookahead.palSelect;
            p.render.fetchIdx = idx ^ 1;
            if (px < 8 && !bgLeftEnabled) {
                pixelArgb = universalBgArgb(p);
                p.bgPattern[px] = 0;
            } else {
                p.bgPattern[px] = (byte) pattern;
                int colorAddr = (pattern == 0) ? PAL_BASE : (PAL_BASE | (palSelect << 2) | pattern);
                int nesIndex = p.readPalette(colorAddr);
                pixelArgb = colorToArgb(p, nesIndex);
            }
        }
        if (spritesEnabled && (px >= 8 || spritesLeftEnabled)) {
            if (fetchSpritePixel(p, px, py, chr)) {
                int spritePattern = _sprOut[0], spritePal = _sprOut[1];
                int colorAddr = SPRITE_PAL_BASE | (spritePal << 2) | spritePattern;
                int nesIndex = p.readPalette(colorAddr);
                pixelArgb = colorToArgb(p, nesIndex);
            }
        }
        p.framebuffer[col] = pixelArgb;
    }
}
