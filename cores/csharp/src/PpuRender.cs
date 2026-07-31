using System.Runtime.CompilerServices;

namespace NesCore;

// PPU rendering pipeline: background + sprite pixel generation.
// Port of src/ppu/render.rs.
public static class PpuRender
{
    private const ushort NtCols = 32;
    private const ushort NtRows = 30;
    private const ushort AttrTableOffset = 0x03C0;
    private const ushort NtBase = 0x2000;
    private const ushort PalBase = 0x3F00;
    private const ushort SpritePalBase = 0x3F10;

    private const byte AttrPaletteMask = 0x03;
    private const byte AttrPriorityBehind = 0x20;
    private const byte AttrHflip = 0x40;
    private const byte AttrVflip = 0x80;

    private const ushort SpriteCount = 64;
    private const ushort SpriteHeight8x8 = 8;
    private const ushort SpriteHeight8x16 = 16;
    private const ushort SpriteWidth = 8;
    private const byte OamYHidden = 0xEF;
    private const byte OamYHalt = 0xFF;
    private const ushort SpriteZeroHitMaxX = 255;

    private const uint Alpha = 0xFF;

    public static readonly byte[,] NesPalette = new byte[64, 3] {
        {0x7C, 0x7C, 0x7C}, {0x00, 0x00, 0xFC}, {0x00, 0x00, 0xBC}, {0x44, 0x28, 0xBC},
        {0x94, 0x00, 0x84}, {0xA8, 0x00, 0x20}, {0xA8, 0x10, 0x00}, {0x88, 0x14, 0x00},
        {0x50, 0x30, 0x00}, {0x00, 0x78, 0x00}, {0x00, 0x68, 0x00}, {0x00, 0x58, 0x00},
        {0x00, 0x40, 0x58}, {0x00, 0x00, 0x00}, {0x00, 0x00, 0x00}, {0x00, 0x00, 0x00},
        {0xBC, 0xBC, 0xBC}, {0x00, 0x78, 0xF8}, {0x00, 0x58, 0xF8}, {0x68, 0x44, 0xFC},
        {0xD8, 0x00, 0xCC}, {0xE4, 0x00, 0x58}, {0xF8, 0x38, 0x00}, {0xE4, 0x5C, 0x10},
        {0xAC, 0x7C, 0x00}, {0x00, 0xB8, 0x00}, {0x00, 0xA8, 0x00}, {0x00, 0xA8, 0x44},
        {0x00, 0x88, 0x88}, {0x00, 0x00, 0x00}, {0x00, 0x00, 0x00}, {0x00, 0x00, 0x00},
        {0xF8, 0xF8, 0xF8}, {0x3C, 0xBC, 0xFC}, {0x68, 0x88, 0xFC}, {0x98, 0x78, 0xF8},
        {0xF8, 0x78, 0xF8}, {0xF8, 0x58, 0x98}, {0xF8, 0x78, 0x58}, {0xFC, 0xA0, 0x44},
        {0xF8, 0xB8, 0x00}, {0xB8, 0xF8, 0x18}, {0x58, 0xD8, 0x54}, {0x58, 0xF8, 0x98},
        {0x00, 0xE8, 0xD8}, {0x78, 0x78, 0x78}, {0x00, 0x00, 0x00}, {0x00, 0x00, 0x00},
        {0xFC, 0xFC, 0xFC}, {0xA4, 0xE4, 0xFC}, {0xB8, 0xB8, 0xF8}, {0xD8, 0xB8, 0xF8},
        {0xF8, 0xB8, 0xF8}, {0xF8, 0xA4, 0xC0}, {0xF0, 0xD0, 0xB0}, {0xFC, 0xE0, 0xA8},
        {0xF8, 0xD8, 0x78}, {0xD8, 0xF8, 0x78}, {0xB8, 0xF8, 0xB8}, {0xB8, 0xF8, 0xD8},
        {0x00, 0xFC, 0xFC}, {0xF8, 0xD8, 0xF8}, {0x00, 0x00, 0x00}, {0x00, 0x00, 0x00}
    };

    public static readonly byte[,] NesPaletteInaccurate = new byte[64, 3] {
        {0x84, 0x84, 0x84}, {0x00, 0x1D, 0x2C}, {0x1C, 0x0C, 0x54}, {0x30, 0x04, 0x64},
        {0x48, 0x00, 0x5C}, {0x58, 0x00, 0x44}, {0x58, 0x00, 0x24}, {0x4C, 0x0C, 0x00},
        {0x38, 0x18, 0x00}, {0x20, 0x28, 0x00}, {0x0C, 0x3C, 0x00}, {0x00, 0x40, 0x00},
        {0x00, 0x3C, 0x1C}, {0x00, 0x38, 0x3C}, {0x00, 0x00, 0x00}, {0x00, 0x00, 0x00},
        {0xB4, 0xB4, 0xB4}, {0x38, 0x6C, 0xBC}, {0x54, 0x58, 0xEC}, {0x70, 0x44, 0xF4},
        {0x90, 0x38, 0xE4}, {0xA8, 0x34, 0xC8}, {0xB8, 0x34, 0x88}, {0xC0, 0x34, 0x44},
        {0xC4, 0x40, 0x14}, {0xC8, 0x50, 0x00}, {0xA8, 0x60, 0x00}, {0x88, 0x70, 0x00},
        {0x6C, 0x80, 0x00}, {0x44, 0x88, 0x00}, {0x00, 0x94, 0x00}, {0x00, 0x00, 0x00},
        {0xFF, 0xFF, 0xFF}, {0x9C, 0xDC, 0xFF}, {0xB8, 0xB8, 0xFF}, {0xD0, 0xB8, 0xFF},
        {0xFF, 0xB0, 0xF4}, {0xFF, 0xA8, 0xE0}, {0xFF, 0xA4, 0xC0}, {0xFF, 0xA0, 0x90},
        {0xF8, 0x94, 0x58}, {0xF0, 0xA0, 0x38}, {0xD8, 0xA8, 0x20}, {0xB8, 0xB0, 0x14},
        {0x98, 0xB8, 0x14}, {0x70, 0xC0, 0x14}, {0x50, 0xC8, 0x24}, {0x00, 0x00, 0x00},
        {0xFF, 0xFF, 0xFF}, {0x9C, 0xDC, 0xFF}, {0xB8, 0xB8, 0xFF}, {0xD0, 0xB8, 0xFF},
        {0xFF, 0xB0, 0xF4}, {0xFF, 0xA8, 0xE0}, {0xFF, 0xA4, 0xC0}, {0xFF, 0xA0, 0x90},
        {0xF8, 0x94, 0x58}, {0xF0, 0xA0, 0x38}, {0xD8, 0xA8, 0x20}, {0xB8, 0xB0, 0x14},
        {0x98, 0xB8, 0x14}, {0x70, 0xC0, 0x14}, {0x50, 0xC8, 0x24}, {0x00, 0x00, 0x00}
    };

    public static readonly byte[,] PalPalette = new byte[64, 3] {
        {0x84, 0x84, 0x84}, {0x00, 0x1D, 0x2C}, {0x0C, 0x0C, 0x44}, {0x24, 0x04, 0x54},
        {0x3C, 0x00, 0x4C}, {0x4C, 0x00, 0x34}, {0x4C, 0x00, 0x18}, {0x40, 0x0C, 0x00},
        {0x2C, 0x18, 0x00}, {0x18, 0x28, 0x00}, {0x08, 0x3C, 0x00}, {0x00, 0x40, 0x00},
        {0x00, 0x3C, 0x1C}, {0x00, 0x38, 0x3C}, {0x04, 0x04, 0x04}, {0x00, 0x00, 0x00},
        {0xB4, 0xB4, 0xB4}, {0x30, 0x60, 0xA4}, {0x48, 0x48, 0xC8}, {0x60, 0x38, 0xD8},
        {0x80, 0x30, 0xC8}, {0x98, 0x2C, 0xAC}, {0xA8, 0x2C, 0x70}, {0xB0, 0x2C, 0x38},
        {0xB4, 0x38, 0x10}, {0xB8, 0x48, 0x00}, {0x98, 0x58, 0x00}, {0x78, 0x68, 0x00},
        {0x5C, 0x78, 0x00}, {0x38, 0x80, 0x00}, {0x00, 0x8C, 0x00}, {0x04, 0x04, 0x04},
        {0xFF, 0xFF, 0xFF}, {0x8C, 0xCC, 0xFF}, {0xA8, 0xA8, 0xFF}, {0xC0, 0xA8, 0xFF},
        {0xE8, 0xA4, 0xF0}, {0xE8, 0xA0, 0xD8}, {0xE8, 0x9C, 0xB8}, {0xE8, 0x98, 0x88},
        {0xE0, 0x8C, 0x54}, {0xD8, 0x98, 0x38}, {0xC0, 0xA0, 0x24}, {0xA0, 0xA8, 0x18},
        {0x84, 0xB0, 0x18}, {0x60, 0xB8, 0x18}, {0x44, 0xC0, 0x2C}, {0x04, 0x04, 0x04},
        {0xFF, 0xFF, 0xFF}, {0x8C, 0xCC, 0xFF}, {0xA8, 0xA8, 0xFF}, {0xC0, 0xA8, 0xFF},
        {0xE8, 0xA4, 0xF0}, {0xE8, 0xA0, 0xD8}, {0xE8, 0x9C, 0xB8}, {0xE8, 0x98, 0x88},
        {0xE0, 0x8C, 0x54}, {0xD8, 0x98, 0x38}, {0xC0, 0xA0, 0x24}, {0xA0, 0xA8, 0x18},
        {0x84, 0xB0, 0x18}, {0x60, 0xB8, 0x18}, {0x44, 0xC0, 0x2C}, {0x04, 0x04, 0x04}
    };

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public static uint NesColorToArgbFor(byte index, Region region)
    {
        uint idx = (uint)(index & 0x3F);
        byte r, g, b;
        if (RegionTiming.IsPalPalette(region))
        {
            r = PalPalette[idx, 0]; g = PalPalette[idx, 1]; b = PalPalette[idx, 2];
        }
        else
        {
            r = NesPalette[idx, 0]; g = NesPalette[idx, 1]; b = NesPalette[idx, 2];
        }
        return (Alpha << 24) | ((uint)r << 16) | ((uint)g << 8) | b;
    }

    public static uint NesColorToArgb(byte index) => NesColorToArgbFor(index, Region.Ntsc);

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    private static uint ColorToArgb(Ppu p, byte index)
    {
        uint idx = (uint)(index & 0x3F);
        byte r, g, b;
        if (RegionTiming.IsPalPalette(p.Region))
        {
            r = PalPalette[idx, 0]; g = PalPalette[idx, 1]; b = PalPalette[idx, 2];
        }
        else if (p.Render.UseInaccuratePalette)
        {
            r = NesPaletteInaccurate[idx, 0]; g = NesPaletteInaccurate[idx, 1]; b = NesPaletteInaccurate[idx, 2];
        }
        else
        {
            r = NesPalette[idx, 0]; g = NesPalette[idx, 1]; b = NesPalette[idx, 2];
        }
        return (Alpha << 24) | ((uint)r << 16) | ((uint)g << 8) | b;
    }

    public static uint UniversalBgArgb(Ppu p) => ColorToArgb(p, p.ReadPalette(PalBase));

    public static void ClearFramebuffer(Ppu p, uint argb)
    {
        for (uint i = 0; i < Ppu.FramebufferSize; ++i) p.Framebuffer[i] = argb;
    }

    public static uint Pixel(Ppu p, uint x, uint y)
    {
        if (x < Ppu.ScreenWidth && y < Ppu.ScreenHeight)
            return p.Framebuffer[y * Ppu.ScreenWidth + x];
        return UniversalBgArgb(p);
    }

    public static void RenderBackground(Ppu p, ChrReader chr)
    {
        for (ushort py = 0; py < Ppu.ScreenHeight; ++py)
            RenderBackgroundScanline(p, py, chr);
    }

    public static void RenderSprites(Ppu p, ChrReader chr)
    {
        p.SetSpriteOverflow(false);
        p.SetSpriteZeroHit(false);
        bool overflowThisFrame = false;
        bool spriteZeroHitSet = false;
        for (ushort sl = 0; sl < Ppu.ScreenHeight; ++sl)
            RenderSpritesScanline(p, sl, chr, ref overflowThisFrame, ref spriteZeroHitSet);
        if (overflowThisFrame) p.SetSpriteOverflow(true);
    }

    public static void RenderFrame(Ppu p, ChrReader chr)
    {
        p.SetSpriteOverflow(false);
        p.SetSpriteZeroHit(false);
        bool overflowThisFrame = false;
        bool spriteZeroHitSet = false;
        for (ushort py = 0; py < Ppu.ScreenHeight; ++py)
        {
            RenderBackgroundScanline(p, py, chr);
            RenderSpritesScanline(p, py, chr, ref overflowThisFrame, ref spriteZeroHitSet);
        }
        if (overflowThisFrame) p.SetSpriteOverflow(true);
    }

    private static void RenderBackgroundScanline(Ppu p, ushort py, ChrReader chr)
    {
        bool bgEnabled = (p.PpuMask & Ppu.MaskShowBg) != 0;
        bool bgLeftEnabled = (p.PpuMask & Ppu.MaskShowBgLeft) != 0;
        uint universalBg = UniversalBgArgb(p);
        uint rowBase = (uint)py * Ppu.ScreenWidth;

        if (!bgEnabled)
        {
            for (uint px = 0; px < Ppu.ScreenWidth; ++px)
            {
                p.Framebuffer[rowBase + px] = universalBg;
                p.BgPattern[px] = 0;
            }
            return;
        }

        ushort bgTable = ((p.PpuCtrl & Ppu.CtrlBgPattern1000) != 0) ? (ushort)0x1000 : (ushort)0x0000;
        ushort baseNt = (ushort)((ushort)(p.PpuCtrl & Ppu.CtrlBaseNtMask) << 10);
        ushort coarseX = (ushort)(p.T & 0x001F);
        ushort coarseY = (ushort)((p.T >> 5) & 0x001F);
        ushort fineY = (ushort)((p.T >> 12) & 0x0007);
        ushort scrollX = (ushort)((coarseX << 3) | p.FineX);
        ushort scrollY = (ushort)((coarseY << 3) | fineY);

        ushort gy = (ushort)(py + scrollY);
        fineY = (ushort)(gy & 0x0007);
        ushort tileRow = (ushort)((gy >> 3) & 0x001F);
        ushort ntV = (ushort)((gy >> 8) & 1);

        for (ushort px = 0; px < Ppu.ScreenWidth; ++px)
        {
            if (px < 8 && !bgLeftEnabled)
            {
                p.Framebuffer[rowBase + px] = universalBg;
                p.BgPattern[px] = 0;
                continue;
            }
            ushort gx = (ushort)(px + scrollX);
            ushort fineX = (ushort)(gx & 0x0007);
            ushort tileCol = (ushort)((gx >> 3) & 0x001F);
            ushort ntH = (ushort)((gx >> 8) & 1);
            ushort nt = (ushort)(((baseNt >> 10) ^ ntH ^ (ushort)(ntV << 1)) & 0x03);
            ushort ntAddr = (ushort)(NtBase | (nt << 10) | (tileRow << 5) | tileCol);
            ushort tileIndex = p.ReadNametable(ntAddr);
            ushort patternAddr = (ushort)(bgTable | (tileIndex << 4) | fineY);
            byte plane0 = chr(patternAddr);
            byte plane1 = chr((ushort)(patternAddr | 0x08));
            ushort bit = (ushort)(7 - fineX);
            byte pattern = (byte)(((plane0 >> bit) & 1) | (byte)(((plane1 >> bit) & 1) << 1));
            ushort attrCol = (ushort)(tileCol >> 2);
            ushort attrRow = (ushort)(tileRow >> 2);
            ushort attrAddr = (ushort)(NtBase | (nt << 10) | AttrTableOffset | (attrRow << 3) | attrCol);
            byte attrByte = p.ReadNametable(attrAddr);
            ushort shift = (ushort)(((tileRow & 0x02) << 1) | (tileCol & 0x02));
            byte palSelect = (byte)((attrByte >> shift) & 0x03);
            p.BgPattern[px] = pattern;
            ushort colorAddr = (pattern == 0)
                ? PalBase
                : (ushort)(PalBase | ((ushort)palSelect << 2) | pattern);
            byte nesIndex = p.ReadPalette(colorAddr);
            p.Framebuffer[rowBase + px] = ColorToArgb(p, nesIndex);
        }
    }

    private static void RenderSpritesScanline(Ppu p, ushort scanline, ChrReader chr,
                                              ref bool overflowThisFrame, ref bool spriteZeroHitSet)
    {
        bool spritesEnabled = (p.PpuMask & Ppu.MaskShowSprites) != 0;
        if (!spritesEnabled) return;
        bool spritesLeftEnabled = (p.PpuMask & Ppu.MaskShowSpritesLeft) != 0;
        bool spriteSize16 = (p.PpuCtrl & Ppu.CtrlSpriteSize16) != 0;
        ushort spriteHeight = spriteSize16 ? SpriteHeight8x16 : SpriteHeight8x8;
        ushort spriteTable8x8 = ((p.PpuCtrl & Ppu.CtrlSpritePattern1000) != 0) ? (ushort)0x1000 : (ushort)0x0000;

        var selected = new RenderPipeline.ScanlineSprite[RenderPipeline.MaxSpritesPerScanline];
        byte count = 0;
        for (ushort i = 0; i < SpriteCount; ++i)
        {
            ushort oamIdx = (ushort)(i * 4);
            byte y = p.Oam[oamIdx];
            if (y == OamYHalt) break;
            if (y >= OamYHidden) continue;
            ushort top = (ushort)(y + 1);
            if (scanline < top || scanline >= top + spriteHeight) continue;
            if (count < RenderPipeline.MaxSpritesPerScanline)
            {
                selected[count].OamI = (byte)i;
                selected[count].Y = y;
                selected[count].Tile = p.Oam[oamIdx + 1];
                selected[count].Attr = p.Oam[oamIdx + 2];
                selected[count].Sx = p.Oam[oamIdx + 3];
                count++;
            }
            else
            {
                overflowThisFrame = true;
            }
        }

        uint rowBase = (uint)scanline * Ppu.ScreenWidth;
        for (ushort px = 0; px < Ppu.ScreenWidth; ++px)
        {
            if (px < 8 && !spritesLeftEnabled) continue;
            for (byte idx = 0; idx < count; ++idx)
            {
                byte oamI = selected[idx].OamI;
                byte y = selected[idx].Y;
                byte tile = selected[idx].Tile;
                byte attr = selected[idx].Attr;
                ushort sx = selected[idx].Sx;
                if (px < sx || px >= sx + SpriteWidth) continue;
                byte tileCol = (byte)(px - sx);
                byte tileRow = (byte)(scanline - (y + 1));
                byte row = (attr & AttrVflip) != 0 ? (byte)((byte)(spriteHeight - 1) - tileRow) : tileRow;
                byte col = (attr & AttrHflip) != 0 ? (byte)(7 - tileCol) : tileCol;

                ushort table, tileBase;
                if (spriteSize16)
                {
                    table = (tile & 1) != 0 ? (ushort)0x1000 : (ushort)0x0000;
                    tileBase = (ushort)(tile & 0xFE);
                }
                else
                {
                    table = spriteTable8x8;
                    tileBase = tile;
                }
                byte rowInTile = spriteSize16 ? (byte)(row & 0x07) : row;
                ushort tileForRow = (spriteSize16 && row >= 8) ? (ushort)(tileBase + 1) : tileBase;

                ushort patternAddr = (ushort)(table | (tileForRow << 4) | rowInTile);
                byte plane0 = chr(patternAddr);
                byte plane1 = chr((ushort)(patternAddr | 0x08));
                ushort bit = (ushort)(7 - col);
                byte pattern = (byte)(((plane0 >> bit) & 1) | (byte)(((plane1 >> bit) & 1) << 1));

                if (pattern == 0) continue;

                uint colIdx = rowBase + px;
                bool bgOpaque = p.BgPattern[px] != 0;

                if (!spriteZeroHitSet && oamI == 0 && px < SpriteZeroHitMaxX && bgOpaque)
                {
                    p.SetSpriteZeroHit(true);
                    spriteZeroHitSet = true;
                }

                bool behindBg = (attr & AttrPriorityBehind) != 0;
                if (behindBg && bgOpaque) break;

                ushort pal = (ushort)(attr & AttrPaletteMask);
                ushort colorAddr = (ushort)(SpritePalBase | (pal << 2) | pattern);
                byte nesIndex = p.ReadPalette(colorAddr);
                p.Framebuffer[colIdx] = ColorToArgb(p, nesIndex);
                break;
            }
        }
    }

    private static void SnapshotRenderPosition(Ppu p, uint px)
    {
        p.Render.CoarseY = (ushort)((p.V >> 5) & 0x001F);
        p.Render.FineY = (ushort)((p.V >> 12) & 0x0007);
        p.Render.NtV = (ushort)(p.V & Ppu.NtVBit);
        p.Render.CoarseXStart = (ushort)(p.V & Ppu.CoarseXMask);
        p.Render.NtHStart = (ushort)(p.V & Ppu.NtHBit);
        p.Render.FineXStart = p.FineX;
        p.Render.ResyncPx = (ushort)px;
        p.Render.VDirty = false;
    }

    private static void EffectiveRenderX(Ppu p, uint px, out ushort coarseX, out ushort ntH, out byte fineX)
    {
        coarseX = p.Render.CoarseXStart;
        ntH = p.Render.NtHStart;
        ushort fineXStart = (ushort)p.Render.FineXStart;
        ushort advance = (ushort)(px - p.Render.ResyncPx);
        ushort newFineX = (ushort)((fineXStart + advance) & 0x0007);
        ushort cxInc = (ushort)((fineXStart + advance) >> 3);
        uint total = (uint)coarseX + cxInc;
        uint wraps = total / 32;
        coarseX = (ushort)(total % 32);
        if ((wraps & 1) != 0) ntH ^= Ppu.NtHBit;
        fineX = (byte)newFineX;
    }

    private static RenderPipeline.BgFetch FetchBgPixel(Ppu p, uint px, ChrReader chr)
    {
        EffectiveRenderX(p, px, out ushort coarseX, out ushort ntH, out byte fineX);
        ushort nt = (ushort)(((ntH >> 10) | (p.Render.NtV >> 10)) & 0x03);
        ushort coarseY = p.Render.CoarseY;

        ushort ntAddr = (ushort)(NtBase | (nt << 10) | (coarseY << 5) | coarseX);
        ushort tileIndex = p.ReadNametable(ntAddr);

        ushort bgTable = ((p.PpuCtrl & Ppu.CtrlBgPattern1000) != 0) ? (ushort)0x1000 : (ushort)0x0000;
        ushort patternAddr = (ushort)(bgTable | (tileIndex << 4) | p.Render.FineY);
        byte plane0 = chr(patternAddr);
        byte plane1 = chr((ushort)(patternAddr | 0x08));
        ushort bit = (ushort)(7 - fineX);
        byte pattern = (byte)(((plane0 >> bit) & 1) | (byte)(((plane1 >> bit) & 1) << 1));

        ushort attrCol = (ushort)(coarseX >> 2);
        ushort attrRow = (ushort)(coarseY >> 2);
        ushort attrAddr = (ushort)(NtBase | (nt << 10) | AttrTableOffset | (attrRow << 3) | attrCol);
        byte attrByte = p.ReadNametable(attrAddr);
        ushort shift = (ushort)(((coarseY & 0x02) << 1) | (coarseX & 0x02));
        byte palSelect = (byte)((attrByte >> shift) & 0x03);

        return new RenderPipeline.BgFetch { Pattern = pattern, PalSelect = palSelect };
    }

    private static void EvaluateScanlineSprites(Ppu p)
    {
        p.Render.SpriteCount = 0;
        p.Render.Overflow = false;

        bool spriteSize16 = (p.PpuCtrl & Ppu.CtrlSpriteSize16) != 0;
        ushort spriteHeight = spriteSize16 ? SpriteHeight8x16 : SpriteHeight8x8;
        ushort scanline = p.Scanline;

        for (ushort i = 0; i < SpriteCount; ++i)
        {
            ushort oamIdx = (ushort)(i * 4);
            byte y = p.Oam[oamIdx];
            if (y == OamYHalt) break;
            if (y >= OamYHidden) continue;
            ushort top = (ushort)(y + 1);
            if (scanline < top || scanline >= top + spriteHeight) continue;
            if (p.Render.SpriteCount < RenderPipeline.MaxSpritesPerScanline)
            {
                byte idx = p.Render.SpriteCount;
                p.Render.Sprites[idx].OamI = (byte)i;
                p.Render.Sprites[idx].Y = y;
                p.Render.Sprites[idx].Tile = p.Oam[oamIdx + 1];
                p.Render.Sprites[idx].Attr = p.Oam[oamIdx + 2];
                p.Render.Sprites[idx].Sx = p.Oam[oamIdx + 3];
                p.Render.SpriteCount = (byte)(idx + 1);
            }
            else
            {
                p.Render.Overflow = true;
            }
        }

        if (p.Render.Overflow) p.SetSpriteOverflow(true);
    }

    private static bool FetchSpritePixel(Ppu p, uint px, uint py, ChrReader chr, out byte outPattern, out byte outPal)
    {
        outPattern = 0; outPal = 0;
        bool spriteSize16 = (p.PpuCtrl & Ppu.CtrlSpriteSize16) != 0;
        ushort spriteHeight = spriteSize16 ? SpriteHeight8x16 : SpriteHeight8x8;
        ushort spriteTable8x8 = ((p.PpuCtrl & Ppu.CtrlSpritePattern1000) != 0) ? (ushort)0x1000 : (ushort)0x0000;
        ushort scanline = (ushort)py;
        bool bgOpaque = p.BgPattern[px] != 0;

        for (byte idx = 0; idx < p.Render.SpriteCount; ++idx)
        {
            byte oamI = p.Render.Sprites[idx].OamI;
            byte y = p.Render.Sprites[idx].Y;
            byte tile = p.Render.Sprites[idx].Tile;
            byte attr = p.Render.Sprites[idx].Attr;
            ushort sx = p.Render.Sprites[idx].Sx;
            if (px < sx || px >= sx + SpriteWidth) continue;
            byte tileCol = (byte)(px - sx);
            byte tileRow = (byte)(scanline - (y + 1));
            byte row = (attr & AttrVflip) != 0 ? (byte)((byte)(spriteHeight - 1) - tileRow) : tileRow;
            byte col = (attr & AttrHflip) != 0 ? (byte)(7 - tileCol) : tileCol;

            ushort table, tileBase;
            if (spriteSize16)
            {
                table = (tile & 1) != 0 ? (ushort)0x1000 : (ushort)0x0000;
                tileBase = (ushort)(tile & 0xFE);
            }
            else
            {
                table = spriteTable8x8;
                tileBase = tile;
            }
            byte rowInTile = spriteSize16 ? (byte)(row & 0x07) : row;
            ushort tileForRow = (spriteSize16 && row >= 8) ? (ushort)(tileBase + 1) : tileBase;

            ushort patternAddr = (ushort)(table | (tileForRow << 4) | rowInTile);
            byte plane0 = chr(patternAddr);
            byte plane1 = chr((ushort)(patternAddr | 0x08));
            ushort bit = (ushort)(7 - col);
            byte pattern = (byte)(((plane0 >> bit) & 1) | (byte)(((plane1 >> bit) & 1) << 1));

            if (pattern == 0) continue;

            if (!p.Render.SpriteZeroHit && oamI == 0 && px < SpriteZeroHitMaxX && bgOpaque)
            {
                p.SetSpriteZeroHit(true);
                p.Render.SpriteZeroHit = true;
            }

            bool behindBg = (attr & AttrPriorityBehind) != 0;
            if (behindBg && bgOpaque) continue;

            outPattern = pattern;
            outPal = (byte)(attr & AttrPaletteMask);
            return true;
        }
        return false;
    }

    public static void RenderOnePixel(Ppu p, ChrReader chr)
    {
        uint px = (uint)(p.Cycle - 1);
        uint py = p.Scanline;
        uint col = py * Ppu.ScreenWidth + px;

        if (!p.Render.ScanlineInitialized)
        {
            SnapshotRenderPosition(p, px);
            EvaluateScanlineSprites(p);
            p.Render.ScanlineInitialized = true;
        }

        if (p.Render.VDirty) SnapshotRenderPosition(p, px);

        bool bgEnabled = (p.PpuMask & Ppu.MaskShowBg) != 0;
        bool bgLeftEnabled = (p.PpuMask & Ppu.MaskShowBgLeft) != 0;
        bool spritesEnabled = (p.PpuMask & Ppu.MaskShowSprites) != 0;
        bool spritesLeftEnabled = (p.PpuMask & Ppu.MaskShowSpritesLeft) != 0;

        uint pixelArgb;

        if (!bgEnabled)
        {
            pixelArgb = UniversalBgArgb(p);
            p.BgPattern[px] = 0;
            p.Render.PipelinePrimed = false;
        }
        else
        {
            if (!p.Render.PipelinePrimed)
            {
                p.Render.FetchBuffer[0] = FetchBgPixel(p, px, chr);
                p.Render.FetchBuffer[1] = FetchBgPixel(p, px + 1, chr);
                p.Render.FetchIdx = 0;
                p.Render.PipelinePrimed = true;
            }

            byte idx = p.Render.FetchIdx;
            byte pattern = p.Render.FetchBuffer[idx].Pattern;
            byte palSelect = p.Render.FetchBuffer[idx].PalSelect;

            var lookahead = FetchBgPixel(p, px + 2, chr);
            p.Render.FetchBuffer[idx] = lookahead;
            p.Render.FetchIdx = (byte)(idx ^ 1);

            if (px < 8 && !bgLeftEnabled)
            {
                pixelArgb = UniversalBgArgb(p);
                p.BgPattern[px] = 0;
            }
            else
            {
                p.BgPattern[px] = pattern;
                ushort colorAddr = (pattern == 0)
                    ? PalBase
                    : (ushort)(PalBase | ((ushort)palSelect << 2) | pattern);
                byte nesIndex = p.ReadPalette(colorAddr);
                pixelArgb = ColorToArgb(p, nesIndex);
            }
        }

        if (spritesEnabled && (px >= 8 || spritesLeftEnabled))
        {
            if (FetchSpritePixel(p, px, py, chr, out byte spritePattern, out byte spritePal))
            {
                ushort colorAddr = (ushort)(SpritePalBase | ((ushort)spritePal << 2) | spritePattern);
                byte nesIndex = p.ReadPalette(colorAddr);
                pixelArgb = ColorToArgb(p, nesIndex);
            }
        }

        p.Framebuffer[col] = pixelArgb;
    }
}
