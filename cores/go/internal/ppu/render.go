// render.go - PPU rendering pipeline (port of cores/c/src/ppu_render.c).
package ppu

import (
	"nes-core-go/internal/region"
)

// ChrReader is the CHR pattern-table read callback (the bus implements it).
type ChrReader func(addr uint16) uint8

// Render constants
const (
	ntCols             = 32
	attrTableOffset    = 0x03C0
	ntBase             = 0x2000
	palBase            = 0x3F00
	spritePalBase      = 0x3F10
	attrPaletteMask    = 0x03
	attrPriorityBehind = 0x20
	attrHflip          = 0x40
	attrVflip          = 0x80
	spriteCount        = 64
	spriteHeight8x8    = 8
	spriteHeight8x16   = 16
	spriteWidth        = 8
	oamYHidden         = 0xEF
	oamYHalt           = 0xFF
	spriteZeroHitMaxX  = 255
	alpha              = 0xFF
)

// Palette tables (NTSC 2C02, NTSC inaccurate, PAL 2C07).
var nesPalette = [64][3]uint8{
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
	{0x00, 0xFC, 0xFC}, {0xF8, 0xD8, 0xF8}, {0x00, 0x00, 0x00}, {0x00, 0x00, 0x00},
}

var nesPaletteInaccurate = [64][3]uint8{
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
	{0x98, 0xB8, 0x14}, {0x70, 0xC0, 0x14}, {0x50, 0xC8, 0x24}, {0x00, 0x00, 0x00},
}

var palPalette = [64][3]uint8{
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
	{0x84, 0xB0, 0x18}, {0x60, 0xB8, 0x18}, {0x44, 0xC0, 0x2C}, {0x04, 0x04, 0x04},
}

// NesColorToArgbFor converts a NES color index to ARGB using the region palette.
func NesColorToArgbFor(index uint8, r region.Region) uint32 {
	idx := uint32(index & 0x3F)
	var c [3]uint8
	if region.IsPalPalette(r) {
		c = palPalette[idx]
	} else {
		c = nesPalette[idx]
	}
	return (uint32(alpha) << 24) | (uint32(c[0]) << 16) | (uint32(c[1]) << 8) | uint32(c[2])
}

// NesColorToArgb converts a NES color index to ARGB using the NTSC palette.
func NesColorToArgb(index uint8) uint32 {
	return NesColorToArgbFor(index, region.NTSC)
}

func (p *Ppu) colorToArgb(index uint8) uint32 {
	idx := uint32(index & 0x3F)
	var c [3]uint8
	if region.IsPalPalette(p.Region) {
		c = palPalette[idx]
	} else if p.Render.UseInaccuratePalette {
		c = nesPaletteInaccurate[idx]
	} else {
		c = nesPalette[idx]
	}
	return (uint32(alpha) << 24) | (uint32(c[0]) << 16) | (uint32(c[1]) << 8) | uint32(c[2])
}

// UniversalBgArgb returns the universal background color as ARGB.
func (p *Ppu) UniversalBgArgb() uint32 {
	return p.colorToArgb(p.ReadPalette(palBase))
}

// ClearFramebuffer clears the framebuffer to a single ARGB value.
func (p *Ppu) ClearFramebuffer(argb uint32) {
	for i := range p.Framebuffer {
		p.Framebuffer[i] = argb
	}
}

// Pixel reads a single framebuffer pixel.
func (p *Ppu) Pixel(x, y int) uint32 {
	if x < ScreenWidth && y < ScreenHeight {
		return p.Framebuffer[y*ScreenWidth+x]
	}
	return p.UniversalBgArgb()
}

// RenderBackground renders the entire visible background.
func (p *Ppu) RenderBackground(chr ChrReader) {
	for py := uint16(0); py < ScreenHeight; py++ {
		p.renderBackgroundScanline(py, chr)
	}
}

// RenderSprites renders all sprites, compositing on top of the framebuffer.
func (p *Ppu) RenderSprites(chr ChrReader) {
	p.SetSpriteOverflow(false)
	p.SetSpriteZeroHit(false)
	overflowThisFrame := false
	spriteZeroHitSet := false
	for sl := uint16(0); sl < ScreenHeight; sl++ {
		p.renderSpritesScanline(sl, chr, &overflowThisFrame, &spriteZeroHitSet)
	}
	if overflowThisFrame {
		p.SetSpriteOverflow(true)
	}
}

// RenderFrame renders the full frame: background then sprites.
func (p *Ppu) RenderFrame(chr ChrReader) {
	p.SetSpriteOverflow(false)
	p.SetSpriteZeroHit(false)
	overflowThisFrame := false
	spriteZeroHitSet := false
	for py := uint16(0); py < ScreenHeight; py++ {
		p.renderBackgroundScanline(py, chr)
		p.renderSpritesScanline(py, chr, &overflowThisFrame, &spriteZeroHitSet)
	}
	if overflowThisFrame {
		p.SetSpriteOverflow(true)
	}
}

func (p *Ppu) renderBackgroundScanline(py uint16, chr ChrReader) {
	bgEnabled := (p.Ppumask & MaskShowBg) != 0
	bgLeftEnabled := (p.Ppumask & MaskShowBgLeft) != 0
	universalBg := p.UniversalBgArgb()
	rowBase := int(py) * ScreenWidth

	if !bgEnabled {
		for px := 0; px < ScreenWidth; px++ {
			p.Framebuffer[rowBase+px] = universalBg
			p.BgPattern[px] = 0
		}
		return
	}

	var bgTable uint16
	if (p.Ppuctrl & CtrlBgPattern1000) != 0 {
		bgTable = 0x1000
	}
	baseNt := uint16(p.Ppuctrl&CtrlBaseNTMask) << 10
	coarseX := p.T & 0x001F
	coarseY := (p.T >> 5) & 0x001F
	fineY := (p.T >> 12) & 0x0007
	scrollX := (coarseX << 3) | uint16(p.FineX)
	scrollY := (coarseY << 3) | fineY

	gy := py + scrollY
	fineY = gy & 0x0007
	tileRow := (gy >> 3) & 0x001F
	ntV := (gy >> 8) & 1

	for px := uint16(0); px < ScreenWidth; px++ {
		if px < 8 && !bgLeftEnabled {
			p.Framebuffer[rowBase+int(px)] = universalBg
			p.BgPattern[px] = 0
			continue
		}
		gx := px + scrollX
		fineX := gx & 0x0007
		tileCol := (gx >> 3) & 0x001F
		ntH := (gx >> 8) & 1
		nt := ((baseNt >> 10) ^ ntH ^ (ntV << 1)) & 0x03
		ntAddr := ntBase | (nt << 10) | (tileRow << 5) | tileCol
		tileIndex := uint16(p.ReadNametable(ntAddr))
		patternAddr := bgTable | (tileIndex << 4) | fineY
		plane0 := chr(patternAddr)
		plane1 := chr(patternAddr | 0x08)
		bit := 7 - fineX
		pattern := ((plane0 >> bit) & 1) | (((plane1 >> bit) & 1) << 1)
		attrCol := tileCol >> 2
		attrRow := tileRow >> 2
		attrAddr := ntBase | (nt << 10) | attrTableOffset | (attrRow << 3) | attrCol
		attrByte := p.ReadNametable(attrAddr)
		shift := ((tileRow & 0x02) << 1) | (tileCol & 0x02)
		palSelect := (attrByte >> shift) & 0x03
		p.BgPattern[px] = pattern
		var colorAddr uint16
		if pattern == 0 {
			colorAddr = palBase
		} else {
			colorAddr = palBase | (uint16(palSelect) << 2) | uint16(pattern)
		}
		nesIndex := p.ReadPalette(colorAddr)
		p.Framebuffer[rowBase+int(px)] = p.colorToArgb(nesIndex)
	}
}

func (p *Ppu) renderSpritesScanline(scanline uint16, chr ChrReader, overflowThisFrame, spriteZeroHitSet *bool) {
	spritesEnabled := (p.Ppumask & MaskShowSprites) != 0
	if !spritesEnabled {
		return
	}
	spritesLeftEnabled := (p.Ppumask & MaskShowSpritesLeft) != 0
	spriteSize16 := (p.Ppuctrl & CtrlSpriteSize16) != 0
	var spriteHeight uint16
	if spriteSize16 {
		spriteHeight = spriteHeight8x16
	} else {
		spriteHeight = spriteHeight8x8
	}
	var spriteTable8x8 uint16
	if (p.Ppuctrl & CtrlSpritePattern1000) != 0 {
		spriteTable8x8 = 0x1000
	}

	var selected [MaxSpritesPerScanline]ScanlineSprite
	var count uint8
	for i := uint16(0); i < spriteCount; i++ {
		oamIdx := i * 4
		y := p.Oam[oamIdx]
		if y == oamYHalt {
			break
		}
		if y >= oamYHidden {
			continue
		}
		top := y + 1
		if scanline < uint16(top) || scanline >= uint16(top)+spriteHeight {
			continue
		}
		if count < MaxSpritesPerScanline {
			selected[count] = ScanlineSprite{
				OamI: uint8(i),
				Y:    y,
				Tile: p.Oam[oamIdx+1],
				Attr: p.Oam[oamIdx+2],
				Sx:   p.Oam[oamIdx+3],
			}
			count++
		} else {
			*overflowThisFrame = true
		}
	}

	rowBase := int(scanline) * ScreenWidth
	for px := uint16(0); px < ScreenWidth; px++ {
		if px < 8 && !spritesLeftEnabled {
			continue
		}
		for idx := uint8(0); idx < count; idx++ {
			oamI := selected[idx].OamI
			y := selected[idx].Y
			tile := selected[idx].Tile
			attr := selected[idx].Attr
			sx := uint16(selected[idx].Sx)
			if px < sx || px >= sx+spriteWidth {
				continue
			}
			tileCol := uint8(px - sx)
			tileRow := uint8(scanline - uint16(y+1))
			var row uint8
			if (attr & attrVflip) != 0 {
				row = uint8(spriteHeight-1) - tileRow
			} else {
				row = tileRow
			}
			var col uint8
			if (attr & attrHflip) != 0 {
				col = 7 - tileCol
			} else {
				col = tileCol
			}

			var table, tileBase uint16
			if spriteSize16 {
				if (tile & 1) != 0 {
					table = 0x1000
				}
				tileBase = uint16(tile & 0xFE)
			} else {
				table = spriteTable8x8
				tileBase = uint16(tile)
			}
			var rowInTile uint8
			if spriteSize16 {
				rowInTile = row & 0x07
			} else {
				rowInTile = row
			}
			var tileForRow uint16 = tileBase
			if spriteSize16 && row >= 8 {
				tileForRow = tileBase + 1
			}

			patternAddr := table | (tileForRow << 4) | uint16(rowInTile)
			plane0 := chr(patternAddr)
			plane1 := chr(patternAddr | 0x08)
			bit := 7 - col
			pattern := ((plane0 >> bit) & 1) | (((plane1 >> bit) & 1) << 1)

			if pattern == 0 {
				continue
			}

			colIdx := rowBase + int(px)
			bgOpaque := p.BgPattern[px] != 0

			if !*spriteZeroHitSet && oamI == 0 && px < spriteZeroHitMaxX && bgOpaque {
				p.SetSpriteZeroHit(true)
				*spriteZeroHitSet = true
			}

			behindBg := (attr & attrPriorityBehind) != 0
			if behindBg && bgOpaque {
				break
			}

			pal := uint16(attr & attrPaletteMask)
			colorAddr := spritePalBase | (pal << 2) | uint16(pattern)
			nesIndex := p.ReadPalette(colorAddr)
			p.Framebuffer[colIdx] = p.colorToArgb(nesIndex)
			break
		}
	}
}

// ---- Per-pixel (cycle-accurate) rendering ----

func (p *Ppu) snapshotRenderPosition(px int) {
	p.Render.CoarseY = (p.V >> 5) & 0x001F
	p.Render.FineY = (p.V >> 12) & 0x0007
	p.Render.NtV = p.V & NtVBit
	p.Render.CoarseXStart = p.V & CoarseXMask
	p.Render.NtHStart = p.V & NtHBit
	p.Render.FineXStart = p.FineX
	p.Render.ResyncPx = uint16(px)
	p.Render.VDirty = false
}

func (p *Ppu) effectiveRenderX(px int) (coarseX, ntH uint16, fineX uint8) {
	coarseX = p.Render.CoarseXStart
	ntH = p.Render.NtHStart
	fineXStart := uint16(p.Render.FineXStart)
	advance := uint16(px) - p.Render.ResyncPx
	newFineX := (fineXStart + advance) & 0x0007
	cxInc := (fineXStart + advance) >> 3
	total := uint32(coarseX) + uint32(cxInc)
	wraps := total / 32
	coarseX = uint16(total % 32)
	if (wraps & 1) != 0 {
		ntH ^= NtHBit
	}
	fineX = uint8(newFineX)
	return
}

func (p *Ppu) fetchBgPixel(px int, chr ChrReader) BgFetch {
	coarseX, ntH, fineX := p.effectiveRenderX(px)
	nt := ((ntH >> 10) | (p.Render.NtV >> 10)) & 0x03
	coarseY := p.Render.CoarseY

	ntAddr := ntBase | (nt << 10) | (coarseY << 5) | coarseX
	tileIndex := uint16(p.ReadNametable(ntAddr))

	var bgTable uint16
	if (p.Ppuctrl & CtrlBgPattern1000) != 0 {
		bgTable = 0x1000
	}
	patternAddr := bgTable | (tileIndex << 4) | p.Render.FineY
	plane0 := chr(patternAddr)
	plane1 := chr(patternAddr | 0x08)
	bit := 7 - fineX
	pattern := ((plane0 >> bit) & 1) | (((plane1 >> bit) & 1) << 1)

	attrCol := coarseX >> 2
	attrRow := coarseY >> 2
	attrAddr := ntBase | (nt << 10) | attrTableOffset | (attrRow << 3) | attrCol
	attrByte := p.ReadNametable(attrAddr)
	shift := ((coarseY & 0x02) << 1) | (coarseX & 0x02)
	palSelect := (attrByte >> shift) & 0x03

	return BgFetch{Pattern: pattern, PalSelect: palSelect}
}

func (p *Ppu) evaluateScanlineSprites() {
	p.Render.SpriteCount = 0
	p.Render.Overflow = false
	spriteSize16 := (p.Ppuctrl & CtrlSpriteSize16) != 0
	var spriteHeight uint16
	if spriteSize16 {
		spriteHeight = spriteHeight8x16
	} else {
		spriteHeight = spriteHeight8x8
	}
	scanline := p.Scanline

	for i := uint16(0); i < spriteCount; i++ {
		oamIdx := i * 4
		y := p.Oam[oamIdx]
		if y == oamYHalt {
			break
		}
		if y >= oamYHidden {
			continue
		}
		top := y + 1
		if scanline < uint16(top) || scanline >= uint16(top)+spriteHeight {
			continue
		}
		if p.Render.SpriteCount < MaxSpritesPerScanline {
			idx := p.Render.SpriteCount
			p.Render.Sprites[idx] = ScanlineSprite{
				OamI: uint8(i),
				Y:    y,
				Tile: p.Oam[oamIdx+1],
				Attr: p.Oam[oamIdx+2],
				Sx:   p.Oam[oamIdx+3],
			}
			p.Render.SpriteCount = idx + 1
		} else {
			p.Render.Overflow = true
		}
	}
	if p.Render.Overflow {
		p.SetSpriteOverflow(true)
	}
}

func (p *Ppu) fetchSpritePixel(px int, py int, chr ChrReader) (bool, uint8, uint8) {
	spriteSize16 := (p.Ppuctrl & CtrlSpriteSize16) != 0
	var spriteHeight uint16
	if spriteSize16 {
		spriteHeight = spriteHeight8x16
	} else {
		spriteHeight = spriteHeight8x8
	}
	var spriteTable8x8 uint16
	if (p.Ppuctrl & CtrlSpritePattern1000) != 0 {
		spriteTable8x8 = 0x1000
	}
	scanline := uint16(py)
	bgOpaque := p.BgPattern[px] != 0

	for idx := uint8(0); idx < p.Render.SpriteCount; idx++ {
		oamI := p.Render.Sprites[idx].OamI
		y := p.Render.Sprites[idx].Y
		tile := p.Render.Sprites[idx].Tile
		attr := p.Render.Sprites[idx].Attr
		sx := uint16(p.Render.Sprites[idx].Sx)
		if uint16(px) < sx || uint16(px) >= sx+spriteWidth {
			continue
		}
		tileCol := uint8(uint16(px) - sx)
		tileRow := uint8(scanline - uint16(y+1))
		var row uint8
		if (attr & attrVflip) != 0 {
			row = uint8(spriteHeight-1) - tileRow
		} else {
			row = tileRow
		}
		var col uint8
		if (attr & attrHflip) != 0 {
			col = 7 - tileCol
		} else {
			col = tileCol
		}

		var table, tileBase uint16
		if spriteSize16 {
			if (tile & 1) != 0 {
				table = 0x1000
			}
			tileBase = uint16(tile & 0xFE)
		} else {
			table = spriteTable8x8
			tileBase = uint16(tile)
		}
		var rowInTile uint8
		if spriteSize16 {
			rowInTile = row & 0x07
		} else {
			rowInTile = row
		}
		var tileForRow uint16 = tileBase
		if spriteSize16 && row >= 8 {
			tileForRow = tileBase + 1
		}

		patternAddr := table | (tileForRow << 4) | uint16(rowInTile)
		plane0 := chr(patternAddr)
		plane1 := chr(patternAddr | 0x08)
		bit := 7 - col
		pattern := ((plane0 >> bit) & 1) | (((plane1 >> bit) & 1) << 1)

		if pattern == 0 {
			continue
		}

		if !p.Render.SpriteZeroHit && oamI == 0 && uint16(px) < spriteZeroHitMaxX && bgOpaque {
			p.SetSpriteZeroHit(true)
			p.Render.SpriteZeroHit = true
		}

		behindBg := (attr & attrPriorityBehind) != 0
		if behindBg && bgOpaque {
			continue
		}

		return true, pattern, (attr & attrPaletteMask)
	}
	return false, 0, 0
}

// RenderOnePixel renders one pixel at the current (cycle, scanline).
func (p *Ppu) RenderOnePixel(chr ChrReader) {
	px := int(p.Cycle - 1)
	py := int(p.Scanline)
	col := py*ScreenWidth + px

	if !p.Render.ScanlineInitialized {
		p.snapshotRenderPosition(px)
		p.evaluateScanlineSprites()
		p.Render.ScanlineInitialized = true
	}
	if p.Render.VDirty {
		p.snapshotRenderPosition(px)
	}

	bgEnabled := (p.Ppumask & MaskShowBg) != 0
	bgLeftEnabled := (p.Ppumask & MaskShowBgLeft) != 0
	spritesEnabled := (p.Ppumask & MaskShowSprites) != 0
	spritesLeftEnabled := (p.Ppumask & MaskShowSpritesLeft) != 0

	var pixelArgb uint32

	if !bgEnabled {
		pixelArgb = p.UniversalBgArgb()
		p.BgPattern[px] = 0
		p.Render.PipelinePrimed = false
	} else {
		if !p.Render.PipelinePrimed {
			f0 := p.fetchBgPixel(px, chr)
			f1 := p.fetchBgPixel(px+1, chr)
			p.Render.FetchBuffer[0] = f0
			p.Render.FetchBuffer[1] = f1
			p.Render.FetchIdx = 0
			p.Render.PipelinePrimed = true
		}
		idx := p.Render.FetchIdx
		pattern := p.Render.FetchBuffer[idx].Pattern
		palSelect := p.Render.FetchBuffer[idx].PalSelect

		lookahead := p.fetchBgPixel(px+2, chr)
		p.Render.FetchBuffer[idx] = lookahead
		p.Render.FetchIdx = idx ^ 1

		if px < 8 && !bgLeftEnabled {
			pixelArgb = p.UniversalBgArgb()
			p.BgPattern[px] = 0
		} else {
			p.BgPattern[px] = pattern
			var colorAddr uint16
			if pattern == 0 {
				colorAddr = palBase
			} else {
				colorAddr = palBase | (uint16(palSelect) << 2) | uint16(pattern)
			}
			nesIndex := p.ReadPalette(colorAddr)
			pixelArgb = p.colorToArgb(nesIndex)
		}
	}

	if spritesEnabled && (px >= 8 || spritesLeftEnabled) {
		ok, spritePattern, spritePal := p.fetchSpritePixel(px, py, chr)
		if ok {
			colorAddr := spritePalBase | (uint16(spritePal) << 2) | uint16(spritePattern)
			nesIndex := p.ReadPalette(colorAddr)
			pixelArgb = p.colorToArgb(nesIndex)
		}
	}

	p.Framebuffer[col] = pixelArgb
}
