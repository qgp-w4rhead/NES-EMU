// PPU rendering pipeline: background + sprite pixel generation.
// Port of cores/c/src/ppu/render.c.

import { Ppu } from './ppu';
import { isPalPalette } from './region';

const NtCols = 32;
const NtRows = 30;
const AttrTableOffset = 0x03C0;
const NtBase = 0x2000;
const PalBase = 0x3F00;
const SpritePalBase = 0x3F10;

const AttrPaletteMask = 0x03;
const AttrPriorityBehind = 0x20;
const AttrHflip = 0x40;
const AttrVflip = 0x80;

const SpriteCount = 64;
const SpriteHeight8x8 = 8;
const SpriteHeight8x16 = 16;
const SpriteWidth = 8;
const OamYHidden = 0xEF;
const OamYHalt = 0xFF;
const SpriteZeroHitMaxX = 255;

const Alpha = 0xFF;

export interface ScanlineSprite {
    oamI: number;
    y: number;
    tile: number;
    attr: number;
    sx: number;
}

export interface BgFetch {
    pattern: number;
    palSelect: number;
}

export class RenderPipeline {
    static readonly MaxSpritesPerScanline = 8;

    coarseY: number = 0;
    fineY: number = 0;
    ntV: number = 0;

    coarseXStart: number = 0;
    ntHStart: number = 0;
    fineXStart: number = 0;
    resyncPx: number = 0;

    scanlineInitialized: boolean = false;
    vDirty: boolean = false;
    renderedThisFrame: boolean = false;

    sprites: ScanlineSprite[] = [];
    spriteCount: number = 0;
    overflow: boolean = false;
    spriteZeroHit: boolean = false;

    slantCorruption: boolean = false;
    useInaccuratePalette: boolean = false;
    nmiRetrigger: boolean = false;

    fetchBuffer: BgFetch[] = [{ pattern: 0, palSelect: 0 }, { pattern: 0, palSelect: 0 }];
    fetchIdx: number = 0;
    pipelinePrimed: boolean = false;

    constructor() {
        for (let i = 0; i < RenderPipeline.MaxSpritesPerScanline; ++i)
            this.sprites.push({ oamI: 0, y: 0, tile: 0, attr: 0, sx: 0 });
    }
}

export class PpuRender {
    static readonly NesPalette: Uint32Array = new Uint32Array(64);
    static readonly NesPaletteInaccurate: Uint32Array = new Uint32Array(64);
    static readonly PalPalette: Uint32Array = new Uint32Array(64);

    static {
        const nesRgb: number[][] = [
            [0x7C, 0x7C, 0x7C], [0x00, 0x00, 0xFC], [0x00, 0x00, 0xBC], [0x44, 0x28, 0xBC],
            [0x94, 0x00, 0x84], [0xA8, 0x00, 0x20], [0xA8, 0x10, 0x00], [0x88, 0x14, 0x00],
            [0x50, 0x30, 0x00], [0x00, 0x78, 0x00], [0x00, 0x68, 0x00], [0x00, 0x58, 0x00],
            [0x00, 0x40, 0x58], [0x00, 0x00, 0x00], [0x00, 0x00, 0x00], [0x00, 0x00, 0x00],
            [0xBC, 0xBC, 0xBC], [0x00, 0x78, 0xF8], [0x00, 0x58, 0xF8], [0x68, 0x44, 0xFC],
            [0xD8, 0x00, 0xCC], [0xE4, 0x00, 0x58], [0xF8, 0x38, 0x00], [0xE4, 0x5C, 0x10],
            [0xAC, 0x7C, 0x00], [0x00, 0xB8, 0x00], [0x00, 0xA8, 0x00], [0x00, 0xA8, 0x44],
            [0x00, 0x88, 0x88], [0x00, 0x00, 0x00], [0x00, 0x00, 0x00], [0x00, 0x00, 0x00],
            [0xF8, 0xF8, 0xF8], [0x3C, 0xBC, 0xFC], [0x68, 0x88, 0xFC], [0x98, 0x78, 0xF8],
            [0xF8, 0x78, 0xF8], [0xF8, 0x58, 0x98], [0xF8, 0x78, 0x58], [0xFC, 0xA0, 0x44],
            [0xF8, 0xB8, 0x00], [0xB8, 0xF8, 0x18], [0x58, 0xD8, 0x54], [0x58, 0xF8, 0x98],
            [0x00, 0xE8, 0xD8], [0x78, 0x78, 0x78], [0x00, 0x00, 0x00], [0x00, 0x00, 0x00],
            [0xFC, 0xFC, 0xFC], [0xA4, 0xE4, 0xFC], [0xB8, 0xB8, 0xF8], [0xD8, 0xB8, 0xF8],
            [0xF8, 0xB8, 0xF8], [0xF8, 0xA4, 0xC0], [0xF0, 0xD0, 0xB0], [0xFC, 0xE0, 0xA8],
            [0xF8, 0xD8, 0x78], [0xD8, 0xF8, 0x78], [0xB8, 0xF8, 0xB8], [0xB8, 0xF8, 0xD8],
            [0x00, 0xFC, 0xFC], [0xF8, 0xD8, 0xF8], [0x00, 0x00, 0x00], [0x00, 0x00, 0x00],
        ];
        const inacc: number[][] = [
            [0x84, 0x84, 0x84], [0x00, 0x1D, 0x2C], [0x1C, 0x0C, 0x54], [0x30, 0x04, 0x64],
            [0x48, 0x00, 0x5C], [0x58, 0x00, 0x44], [0x58, 0x00, 0x24], [0x4C, 0x0C, 0x00],
            [0x38, 0x18, 0x00], [0x20, 0x28, 0x00], [0x0C, 0x3C, 0x00], [0x00, 0x40, 0x00],
            [0x00, 0x3C, 0x1C], [0x00, 0x38, 0x3C], [0x00, 0x00, 0x00], [0x00, 0x00, 0x00],
            [0xB4, 0xB4, 0xB4], [0x38, 0x6C, 0xBC], [0x54, 0x58, 0xEC], [0x70, 0x44, 0xF4],
            [0x90, 0x38, 0xE4], [0xA8, 0x34, 0xC8], [0xB8, 0x34, 0x88], [0xC0, 0x34, 0x44],
            [0xC4, 0x40, 0x14], [0xC8, 0x50, 0x00], [0xA8, 0x60, 0x00], [0x88, 0x70, 0x00],
            [0x6C, 0x80, 0x00], [0x44, 0x88, 0x00], [0x00, 0x94, 0x00], [0x00, 0x00, 0x00],
            [0xFF, 0xFF, 0xFF], [0x9C, 0xDC, 0xFF], [0xB8, 0xB8, 0xFF], [0xD0, 0xB8, 0xFF],
            [0xFF, 0xB0, 0xF4], [0xFF, 0xA8, 0xE0], [0xFF, 0xA4, 0xC0], [0xFF, 0xA0, 0x90],
            [0xF8, 0x94, 0x58], [0xF0, 0xA0, 0x38], [0xD8, 0xA8, 0x20], [0xB8, 0xB0, 0x14],
            [0x98, 0xB8, 0x14], [0x70, 0xC0, 0x14], [0x50, 0xC8, 0x24], [0x00, 0x00, 0x00],
            [0xFF, 0xFF, 0xFF], [0x9C, 0xDC, 0xFF], [0xB8, 0xB8, 0xFF], [0xD0, 0xB8, 0xFF],
            [0xFF, 0xB0, 0xF4], [0xFF, 0xA8, 0xE0], [0xFF, 0xA4, 0xC0], [0xFF, 0xA0, 0x90],
            [0xF8, 0x94, 0x58], [0xF0, 0xA0, 0x38], [0xD8, 0xA8, 0x20], [0xB8, 0xB0, 0x14],
            [0x98, 0xB8, 0x14], [0x70, 0xC0, 0x14], [0x50, 0xC8, 0x24], [0x00, 0x00, 0x00],
        ];
        const palRgb: number[][] = [
            [0x84, 0x84, 0x84], [0x00, 0x1D, 0x2C], [0x0C, 0x0C, 0x44], [0x24, 0x04, 0x54],
            [0x3C, 0x00, 0x4C], [0x4C, 0x00, 0x34], [0x4C, 0x00, 0x18], [0x40, 0x0C, 0x00],
            [0x2C, 0x18, 0x00], [0x18, 0x28, 0x00], [0x08, 0x3C, 0x00], [0x00, 0x40, 0x00],
            [0x00, 0x3C, 0x1C], [0x00, 0x38, 0x3C], [0x04, 0x04, 0x04], [0x00, 0x00, 0x00],
            [0xB4, 0xB4, 0xB4], [0x30, 0x60, 0xA4], [0x48, 0x48, 0xC8], [0x60, 0x38, 0xD8],
            [0x80, 0x30, 0xC8], [0x98, 0x2C, 0xAC], [0xA8, 0x2C, 0x70], [0xB0, 0x2C, 0x38],
            [0xB4, 0x38, 0x10], [0xB8, 0x48, 0x00], [0x98, 0x58, 0x00], [0x78, 0x68, 0x00],
            [0x5C, 0x78, 0x00], [0x38, 0x80, 0x00], [0x00, 0x8C, 0x00], [0x04, 0x04, 0x04],
            [0xFF, 0xFF, 0xFF], [0x8C, 0xCC, 0xFF], [0xA8, 0xA8, 0xFF], [0xC0, 0xA8, 0xFF],
            [0xE8, 0xA4, 0xF0], [0xE8, 0xA0, 0xD8], [0xE8, 0x9C, 0xB8], [0xE8, 0x98, 0x88],
            [0xE0, 0x8C, 0x54], [0xD8, 0x98, 0x38], [0xC0, 0xA0, 0x24], [0xA0, 0xA8, 0x18],
            [0x84, 0xB0, 0x18], [0x60, 0xB8, 0x18], [0x44, 0xC0, 0x2C], [0x04, 0x04, 0x04],
            [0xFF, 0xFF, 0xFF], [0x8C, 0xCC, 0xFF], [0xA8, 0xA8, 0xFF], [0xC0, 0xA8, 0xFF],
            [0xE8, 0xA4, 0xF0], [0xE8, 0xA0, 0xD8], [0xE8, 0x9C, 0xB8], [0xE8, 0x98, 0x88],
            [0xE0, 0x8C, 0x54], [0xD8, 0x98, 0x38], [0xC0, 0xA0, 0x24], [0xA0, 0xA8, 0x18],
            [0x84, 0xB0, 0x18], [0x60, 0xB8, 0x18], [0x44, 0xC0, 0x2C], [0x04, 0x04, 0x04],
        ];
        for (let i = 0; i < 64; ++i) {
            PpuRender.NesPalette[i] = (Alpha << 24) | (nesRgb[i][0] << 16) | (nesRgb[i][1] << 8) | nesRgb[i][2];
            PpuRender.NesPaletteInaccurate[i] = (Alpha << 24) | (inacc[i][0] << 16) | (inacc[i][1] << 8) | inacc[i][2];
            PpuRender.PalPalette[i] = (Alpha << 24) | (palRgb[i][0] << 16) | (palRgb[i][1] << 8) | palRgb[i][2];
        }
    }

    static nesColorToArgbFor(index: number, region: number): number {
        const idx = index & 0x3F;
        if (isPalPalette(region)) return PpuRender.PalPalette[idx];
        return PpuRender.NesPalette[idx];
    }

    static nesColorToArgb(index: number): number {
        return PpuRender.nesColorToArgbFor(index, 0);
    }

    private static colorToArgb(p: Ppu, index: number): number {
        const idx = index & 0x3F;
        if (isPalPalette(p.region)) return PpuRender.PalPalette[idx];
        if (p.render.useInaccuratePalette) return PpuRender.NesPaletteInaccurate[idx];
        return PpuRender.NesPalette[idx];
    }

    static universalBgArgb(p: Ppu): number {
        return PpuRender.colorToArgb(p, p.readPalette(PalBase));
    }

    static clearFramebuffer(p: Ppu, argb: number): void {
        p.framebuffer.fill(argb);
    }

    static pixel(p: Ppu, x: number, y: number): number {
        if (x < Ppu.ScreenWidth && y < Ppu.ScreenHeight)
            return p.framebuffer[y * Ppu.ScreenWidth + x];
        return PpuRender.universalBgArgb(p);
    }

    static renderBackground(p: Ppu, chr: (addr: number) => number): void {
        for (let py = 0; py < Ppu.ScreenHeight; ++py)
            PpuRender.renderBackgroundScanline(p, py, chr);
    }

    static renderSprites(p: Ppu, chr: (addr: number) => number): void {
        p.setSpriteOverflow(false);
        p.setSpriteZeroHit(false);
        const holder = { overflowThisFrame: false, spriteZeroHitSet: false };
        for (let sl = 0; sl < Ppu.ScreenHeight; ++sl)
            PpuRender.renderSpritesScanline(p, sl, chr, holder);
        if (holder.overflowThisFrame) p.setSpriteOverflow(true);
    }

    static renderFrame(p: Ppu, chr: (addr: number) => number): void {
        p.setSpriteOverflow(false);
        p.setSpriteZeroHit(false);
        const holder = { overflowThisFrame: false, spriteZeroHitSet: false };
        for (let py = 0; py < Ppu.ScreenHeight; ++py) {
            PpuRender.renderBackgroundScanline(p, py, chr);
            PpuRender.renderSpritesScanline(p, py, chr, holder);
        }
        if (holder.overflowThisFrame) p.setSpriteOverflow(true);
    }

    // Note: For renderSpritesScanline, the overflow/spriteZeroHit booleans are
    // captured by reference in C#. In JS we use a small holder object.
    private static renderSpritesScanline(p: Ppu, scanline: number, chr: (addr: number) => number,
        holder: { overflowThisFrame: boolean; spriteZeroHitSet: boolean }): void {
        const spritesEnabled = (p.ppuMask & Ppu.MaskShowSprites) !== 0;
        if (!spritesEnabled) return;
        const spritesLeftEnabled = (p.ppuMask & Ppu.MaskShowSpritesLeft) !== 0;
        const spriteSize16 = (p.ppuCtrl & Ppu.CtrlSpriteSize16) !== 0;
        const spriteHeight = spriteSize16 ? SpriteHeight8x16 : SpriteHeight8x8;
        const spriteTable8x8 = ((p.ppuCtrl & Ppu.CtrlSpritePattern1000) !== 0) ? 0x1000 : 0x0000;

        const selected: ScanlineSprite[] = [];
        for (let i = 0; i < RenderPipeline.MaxSpritesPerScanline; ++i)
            selected.push({ oamI: 0, y: 0, tile: 0, attr: 0, sx: 0 });
        let count = 0;
        for (let i = 0; i < SpriteCount; ++i) {
            const oamIdx = i * 4;
            const y = p.oam[oamIdx];
            if (y === OamYHalt) break;
            if (y >= OamYHidden) continue;
            const top = y + 1;
            if (scanline < top || scanline >= top + spriteHeight) continue;
            if (count < RenderPipeline.MaxSpritesPerScanline) {
                selected[count].oamI = i;
                selected[count].y = y;
                selected[count].tile = p.oam[oamIdx + 1];
                selected[count].attr = p.oam[oamIdx + 2];
                selected[count].sx = p.oam[oamIdx + 3];
                count++;
            } else {
                holder.overflowThisFrame = true;
            }
        }

        const rowBase = scanline * Ppu.ScreenWidth;
        for (let px = 0; px < Ppu.ScreenWidth; ++px) {
            if (px < 8 && !spritesLeftEnabled) continue;
            for (let idx = 0; idx < count; ++idx) {
                const oamI = selected[idx].oamI;
                const y = selected[idx].y;
                const tile = selected[idx].tile;
                const attr = selected[idx].attr;
                const sx = selected[idx].sx;
                if (px < sx || px >= sx + SpriteWidth) continue;
                const tileCol = px - sx;
                const tileRow = scanline - (y + 1);
                const row = (attr & AttrVflip) !== 0 ? (spriteHeight - 1 - tileRow) : tileRow;
                const col = (attr & AttrHflip) !== 0 ? (7 - tileCol) : tileCol;

                let table: number, tileBase: number;
                if (spriteSize16) {
                    table = (tile & 1) !== 0 ? 0x1000 : 0x0000;
                    tileBase = tile & 0xFE;
                } else {
                    table = spriteTable8x8;
                    tileBase = tile;
                }
                const rowInTile = spriteSize16 ? (row & 0x07) : row;
                const tileForRow = (spriteSize16 && row >= 8) ? (tileBase + 1) : tileBase;

                const patternAddr = table | (tileForRow << 4) | rowInTile;
                const plane0 = chr(patternAddr);
                const plane1 = chr(patternAddr | 0x08);
                const bit = 7 - col;
                const pattern = ((plane0 >> bit) & 1) | (((plane1 >> bit) & 1) << 1);

                if (pattern === 0) continue;

                const colIdx = rowBase + px;
                const bgOpaque = p.bgPattern[px] !== 0;

                if (!holder.spriteZeroHitSet && oamI === 0 && px < SpriteZeroHitMaxX && bgOpaque) {
                    p.setSpriteZeroHit(true);
                    holder.spriteZeroHitSet = true;
                }

                const behindBg = (attr & AttrPriorityBehind) !== 0;
                if (behindBg && bgOpaque) break;

                const pal = attr & AttrPaletteMask;
                const colorAddr = SpritePalBase | (pal << 2) | pattern;
                const nesIndex = p.readPalette(colorAddr);
                p.framebuffer[colIdx] = PpuRender.colorToArgb(p, nesIndex);
                break;
            }
        }
    }

    private static renderBackgroundScanline(p: Ppu, py: number, chr: (addr: number) => number): void {
        const bgEnabled = (p.ppuMask & Ppu.MaskShowBg) !== 0;
        const bgLeftEnabled = (p.ppuMask & Ppu.MaskShowBgLeft) !== 0;
        const universalBg = PpuRender.universalBgArgb(p);
        const rowBase = py * Ppu.ScreenWidth;

        if (!bgEnabled) {
            for (let px = 0; px < Ppu.ScreenWidth; ++px) {
                p.framebuffer[rowBase + px] = universalBg;
                p.bgPattern[px] = 0;
            }
            return;
        }

        const bgTable = ((p.ppuCtrl & Ppu.CtrlBgPattern1000) !== 0) ? 0x1000 : 0x0000;
        const baseNt = (p.ppuCtrl & Ppu.CtrlBaseNtMask) << 10;
        const coarseX = p.t & 0x001F;
        const coarseY = (p.t >> 5) & 0x001F;
        let fineY = (p.t >> 12) & 0x0007;
        const scrollX = (coarseX << 3) | p.fineX;
        const scrollY = (coarseY << 3) | fineY;

        const gy = py + scrollY;
        fineY = gy & 0x0007;
        const tileRow = (gy >> 3) & 0x001F;
        const ntV = (gy >> 8) & 1;

        for (let px = 0; px < Ppu.ScreenWidth; ++px) {
            if (px < 8 && !bgLeftEnabled) {
                p.framebuffer[rowBase + px] = universalBg;
                p.bgPattern[px] = 0;
                continue;
            }
            const gx = px + scrollX;
            const fineX = gx & 0x0007;
            const tileCol = (gx >> 3) & 0x001F;
            const ntH = (gx >> 8) & 1;
            const nt = ((baseNt >> 10) ^ ntH ^ (ntV << 1)) & 0x03;
            const ntAddr = NtBase | (nt << 10) | (tileRow << 5) | tileCol;
            const tileIndex = p.readNametable(ntAddr);
            const patternAddr = bgTable | (tileIndex << 4) | fineY;
            const plane0 = chr(patternAddr);
            const plane1 = chr(patternAddr | 0x08);
            const bit = 7 - fineX;
            const pattern = ((plane0 >> bit) & 1) | (((plane1 >> bit) & 1) << 1);
            const attrCol = tileCol >> 2;
            const attrRow = tileRow >> 2;
            const attrAddr = NtBase | (nt << 10) | AttrTableOffset | (attrRow << 3) | attrCol;
            const attrByte = p.readNametable(attrAddr);
            const shift = ((tileRow & 0x02) << 1) | (tileCol & 0x02);
            const palSelect = (attrByte >> shift) & 0x03;
            p.bgPattern[px] = pattern;
            const colorAddr = (pattern === 0)
                ? PalBase
                : (PalBase | (palSelect << 2) | pattern);
            const nesIndex = p.readPalette(colorAddr);
            p.framebuffer[rowBase + px] = PpuRender.colorToArgb(p, nesIndex);
        }
    }

    private static snapshotRenderPosition(p: Ppu, px: number): void {
        p.render.coarseY = (p.v >> 5) & 0x001F;
        p.render.fineY = (p.v >> 12) & 0x0007;
        p.render.ntV = p.v & Ppu.NtVBit;
        p.render.coarseXStart = p.v & Ppu.CoarseXMask;
        p.render.ntHStart = p.v & Ppu.NtHBit;
        p.render.fineXStart = p.fineX;
        p.render.resyncPx = px;
        p.render.vDirty = false;
    }

    private static effectiveRenderX(p: Ppu, px: number): { coarseX: number; ntH: number; fineX: number } {
        let coarseX = p.render.coarseXStart;
        let ntH = p.render.ntHStart;
        const fineXStart = p.render.fineXStart;
        const advance = px - p.render.resyncPx;
        const newFineX = (fineXStart + advance) & 0x0007;
        const cxInc = (fineXStart + advance) >> 3;
        const total = coarseX + cxInc;
        const wraps = Math.floor(total / 32);
        coarseX = total % 32;
        if ((wraps & 1) !== 0) ntH ^= Ppu.NtHBit;
        return { coarseX, ntH, fineX: newFineX };
    }

    private static fetchBgPixel(p: Ppu, px: number, chr: (addr: number) => number): BgFetch {
        const { coarseX, ntH, fineX } = PpuRender.effectiveRenderX(p, px);
        const nt = ((ntH >> 10) | (p.render.ntV >> 10)) & 0x03;
        const coarseY = p.render.coarseY;

        const ntAddr = NtBase | (nt << 10) | (coarseY << 5) | coarseX;
        const tileIndex = p.readNametable(ntAddr);

        const bgTable = ((p.ppuCtrl & Ppu.CtrlBgPattern1000) !== 0) ? 0x1000 : 0x0000;
        const patternAddr = bgTable | (tileIndex << 4) | p.render.fineY;
        const plane0 = chr(patternAddr);
        const plane1 = chr(patternAddr | 0x08);
        const bit = 7 - fineX;
        const pattern = ((plane0 >> bit) & 1) | (((plane1 >> bit) & 1) << 1);

        const attrCol = coarseX >> 2;
        const attrRow = coarseY >> 2;
        const attrAddr = NtBase | (nt << 10) | AttrTableOffset | (attrRow << 3) | attrCol;
        const attrByte = p.readNametable(attrAddr);
        const shift = ((coarseY & 0x02) << 1) | (coarseX & 0x02);
        const palSelect = (attrByte >> shift) & 0x03;

        return { pattern, palSelect };
    }

    private static evaluateScanlineSprites(p: Ppu): void {
        p.render.spriteCount = 0;
        p.render.overflow = false;

        const spriteSize16 = (p.ppuCtrl & Ppu.CtrlSpriteSize16) !== 0;
        const spriteHeight = spriteSize16 ? SpriteHeight8x16 : SpriteHeight8x8;
        const scanline = p.scanline;

        for (let i = 0; i < SpriteCount; ++i) {
            const oamIdx = i * 4;
            const y = p.oam[oamIdx];
            if (y === OamYHalt) break;
            if (y >= OamYHidden) continue;
            const top = y + 1;
            if (scanline < top || scanline >= top + spriteHeight) continue;
            if (p.render.spriteCount < RenderPipeline.MaxSpritesPerScanline) {
                const idx = p.render.spriteCount;
                p.render.sprites[idx].oamI = i;
                p.render.sprites[idx].y = y;
                p.render.sprites[idx].tile = p.oam[oamIdx + 1];
                p.render.sprites[idx].attr = p.oam[oamIdx + 2];
                p.render.sprites[idx].sx = p.oam[oamIdx + 3];
                p.render.spriteCount = idx + 1;
            } else {
                p.render.overflow = true;
            }
        }

        if (p.render.overflow) p.setSpriteOverflow(true);
    }

    private static fetchSpritePixel(p: Ppu, px: number, py: number, chr: (addr: number) => number): { hit: boolean; pattern: number; pal: number } {
        const spriteSize16 = (p.ppuCtrl & Ppu.CtrlSpriteSize16) !== 0;
        const spriteHeight = spriteSize16 ? SpriteHeight8x16 : SpriteHeight8x8;
        const spriteTable8x8 = ((p.ppuCtrl & Ppu.CtrlSpritePattern1000) !== 0) ? 0x1000 : 0x0000;
        const scanline = py;
        const bgOpaque = p.bgPattern[px] !== 0;

        for (let idx = 0; idx < p.render.spriteCount; ++idx) {
            const oamI = p.render.sprites[idx].oamI;
            const y = p.render.sprites[idx].y;
            const tile = p.render.sprites[idx].tile;
            const attr = p.render.sprites[idx].attr;
            const sx = p.render.sprites[idx].sx;
            if (px < sx || px >= sx + SpriteWidth) continue;
            const tileCol = px - sx;
            const tileRow = scanline - (y + 1);
            const row = (attr & AttrVflip) !== 0 ? (spriteHeight - 1 - tileRow) : tileRow;
            const col = (attr & AttrHflip) !== 0 ? (7 - tileCol) : tileCol;

            let table: number, tileBase: number;
            if (spriteSize16) {
                table = (tile & 1) !== 0 ? 0x1000 : 0x0000;
                tileBase = tile & 0xFE;
            } else {
                table = spriteTable8x8;
                tileBase = tile;
            }
            const rowInTile = spriteSize16 ? (row & 0x07) : row;
            const tileForRow = (spriteSize16 && row >= 8) ? (tileBase + 1) : tileBase;

            const patternAddr = table | (tileForRow << 4) | rowInTile;
            const plane0 = chr(patternAddr);
            const plane1 = chr(patternAddr | 0x08);
            const bit = 7 - col;
            const pattern = ((plane0 >> bit) & 1) | (((plane1 >> bit) & 1) << 1);

            if (pattern === 0) continue;

            if (!p.render.spriteZeroHit && oamI === 0 && px < SpriteZeroHitMaxX && bgOpaque) {
                p.setSpriteZeroHit(true);
                p.render.spriteZeroHit = true;
            }

            const behindBg = (attr & AttrPriorityBehind) !== 0;
            if (behindBg && bgOpaque) continue;

            return { hit: true, pattern, pal: attr & AttrPaletteMask };
        }
        return { hit: false, pattern: 0, pal: 0 };
    }

    static renderOnePixel(p: Ppu, chr: (addr: number) => number): void {
        const px = p.cycle - 1;
        const py = p.scanline;
        const col = py * Ppu.ScreenWidth + px;

        if (!p.render.scanlineInitialized) {
            PpuRender.snapshotRenderPosition(p, px);
            PpuRender.evaluateScanlineSprites(p);
            p.render.scanlineInitialized = true;
        }

        if (p.render.vDirty) PpuRender.snapshotRenderPosition(p, px);

        const bgEnabled = (p.ppuMask & Ppu.MaskShowBg) !== 0;
        const bgLeftEnabled = (p.ppuMask & Ppu.MaskShowBgLeft) !== 0;
        const spritesEnabled = (p.ppuMask & Ppu.MaskShowSprites) !== 0;
        const spritesLeftEnabled = (p.ppuMask & Ppu.MaskShowSpritesLeft) !== 0;

        let pixelArgb: number;

        if (!bgEnabled) {
            pixelArgb = PpuRender.universalBgArgb(p);
            p.bgPattern[px] = 0;
            p.render.pipelinePrimed = false;
        } else {
            if (!p.render.pipelinePrimed) {
                p.render.fetchBuffer[0] = PpuRender.fetchBgPixel(p, px, chr);
                p.render.fetchBuffer[1] = PpuRender.fetchBgPixel(p, px + 1, chr);
                p.render.fetchIdx = 0;
                p.render.pipelinePrimed = true;
            }

            const idx = p.render.fetchIdx;
            const pattern = p.render.fetchBuffer[idx].pattern;
            const palSelect = p.render.fetchBuffer[idx].palSelect;

            const lookahead = PpuRender.fetchBgPixel(p, px + 2, chr);
            p.render.fetchBuffer[idx] = lookahead;
            p.render.fetchIdx = idx ^ 1;

            if (px < 8 && !bgLeftEnabled) {
                pixelArgb = PpuRender.universalBgArgb(p);
                p.bgPattern[px] = 0;
            } else {
                p.bgPattern[px] = pattern;
                const colorAddr = (pattern === 0)
                    ? PalBase
                    : (PalBase | (palSelect << 2) | pattern);
                const nesIndex = p.readPalette(colorAddr);
                pixelArgb = PpuRender.colorToArgb(p, nesIndex);
            }
        }

        if (spritesEnabled && (px >= 8 || spritesLeftEnabled)) {
            const r = PpuRender.fetchSpritePixel(p, px, py, chr);
            if (r.hit) {
                const colorAddr = SpritePalBase | (r.pal << 2) | r.pattern;
                const nesIndex = p.readPalette(colorAddr);
                pixelArgb = PpuRender.colorToArgb(p, nesIndex);
            }
        }

        p.framebuffer[col] = pixelArgb;
    }
}
