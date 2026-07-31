// Package ppu implements the NES Picture Processing Unit
// (port of cores/c/src/ppu.c). The per-pixel renderer lives in render.go.
package ppu

import (
	"nes-core-go/internal/mapper"
	"nes-core-go/internal/region"
)

// ---- Constants ----
const (
	VRAMSize4K       = 0x1000
	VRAMSize2K       = 0x0800
	OAMSize          = 256
	PaletteSize      = 32
	ScanlinesPerFrame = 262
	CyclesPerScanline = 341
	ScanlineVblankStart = 241
	ScanlinePrerender   = 261
	ScreenWidth   = 256
	ScreenHeight  = 240
	FramebufferSize = ScreenWidth * ScreenHeight
	MaxSpritesPerScanline = 8
)

// PPUCTRL bit masks
const (
	CtrlNMI                 uint8 = 0x80
	CtrlIncrement32         uint8 = 0x04
	CtrlSpritePattern1000   uint8 = 0x08
	CtrlBgPattern1000       uint8 = 0x10
	CtrlSpriteSize16        uint8 = 0x20
	CtrlBaseNTMask          uint8 = 0x03
)

// PPUMASK bit masks
const (
	MaskShowBgLeft      uint8 = 0x02
	MaskShowSpritesLeft uint8 = 0x04
	MaskShowBg          uint8 = 0x08
	MaskShowSprites     uint8 = 0x10
)

// PPUSTATUS bit masks
const (
	StatusVblank    uint8 = 0x80
	StatusSpriteZero uint8 = 0x40
	StatusOverflow  uint8 = 0x20
	StatusFlagMask  uint8 = 0xE0
	StatusOpenBusMask uint8 = 0x1F
)

// v / t register bit masks
const (
	NtSelectMask uint16 = 0x0C00
	CoarseXMask  uint16 = 0x001F
	CoarseYMask  uint16 = 0x03E0
	FineYMask    uint16 = 0x7000
	NtHBit       uint16 = 0x0400
	NtVBit       uint16 = 0x0800
)

// Internal timing constants
const (
	vblankNmiCycle      uint16 = 1
	vertScrollIncCycle  uint16 = 256
	hCopyCycle          uint16 = 257
	vCopyCycleStart     uint16 = 280
	vCopyCycleEnd       uint16 = 304
	hScrollIncStep      uint16 = 8
	hScrollIncLast      uint16 = 248
)

// ScanlineSprite is a sprite selected for the current scanline.
type ScanlineSprite struct {
	OamI uint8
	Y    uint8
	Tile uint8
	Attr uint8
	Sx   uint8
}

// BgFetch is a prefetched background pixel.
type BgFetch struct {
	Pattern   uint8
	PalSelect uint8
}

// RenderPipeline is the transient per-pixel rendering pipeline state.
type RenderPipeline struct {
	CoarseY            uint16
	FineY              uint16
	NtV                uint16
	CoarseXStart       uint16
	NtHStart           uint16
	FineXStart         uint8
	ResyncPx           uint16
	ScanlineInitialized bool
	VDirty             bool
	RenderedThisFrame  bool
	Sprites            [MaxSpritesPerScanline]ScanlineSprite
	SpriteCount        uint8
	Overflow           bool
	SpriteZeroHit      bool
	SlantCorruption    bool
	UseInaccuratePalette bool
	NmiRetrigger       bool
	FetchBuffer        [2]BgFetch
	FetchIdx           uint8
	PipelinePrimed     bool
}

// Ppu is the NES Picture Processing Unit.
type Ppu struct {
	Ppuctrl       uint8
	Ppumask       uint8
	Oamaddr       uint8
	Ppustatus     uint8
	V             uint16
	T             uint16
	FineX         uint8
	W             bool
	PpudataBuffer uint8
	OpenBus       uint8

	Vram     [VRAMSize4K]byte
	VramSize uint32
	Oam      [OAMSize]byte
	Palette  [PaletteSize]byte

	Framebuffer [FramebufferSize]uint32
	BgPattern   [ScreenWidth]uint8

	Scanline   uint16
	Cycle      uint16
	NmiRequest bool

	Mirroring mapper.Mirroring
	Region    region.Region

	Render RenderPipeline
}

// Init constructs a PPU in power-on state.
func (p *Ppu) Init() {
	*p = Ppu{}
	p.VramSize = VRAMSize2K
	p.Mirroring = mapper.MirrorHorizontal
	p.Region = region.NTSC
}

// SetMirroring sets the mirroring mode and resizes VRAM if needed.
func (p *Ppu) SetMirroring(m mapper.Mirroring) {
	var needed uint32
	if m == mapper.MirrorFourScreen {
		needed = VRAMSize4K
	} else {
		needed = VRAMSize2K
	}
	if p.VramSize != needed {
		if needed > p.VramSize {
			// Zero the newly-exposed bytes.
			for i := p.VramSize; i < needed; i++ {
				p.Vram[i] = 0
			}
		}
		p.VramSize = needed
	}
	p.Mirroring = m
}

// SetRegion sets the TV system / region.
func (p *Ppu) SetRegion(r region.Region) {
	p.Region = r
	maxSl := region.ScanlinesPerFrame(r)
	if p.Scanline >= maxSl {
		p.Scanline = maxSl - 1
	}
}

// ReadRegister reads a PPU register by de-mirrored index (0..=7).
func (p *Ppu) ReadRegister(reg uint16) uint8 {
	switch reg & 0x07 {
	case 0, 1, 3, 5, 6:
		return p.OpenBus
	case 2:
		return p.readStatus()
	case 4:
		return p.readOamdata()
	case 7:
		return p.PpudataBuffer
	default:
		return p.OpenBus
	}
}

// WriteRegister writes a PPU register by de-mirrored index (0..=7).
func (p *Ppu) WriteRegister(reg uint16, value uint8) {
	p.OpenBus = value
	switch reg & 0x07 {
	case 0:
		p.writePpuctrl(value)
	case 1:
		p.Ppumask = value
	case 2:
		// read-only
	case 3:
		p.Oamaddr = value
	case 4:
		p.writeOamdata(value)
	case 5:
		p.writePpuscroll(value)
	case 6:
		p.writePpuaddr(value)
	case 7:
		// handled by bus
	}
}

func (p *Ppu) readStatus() uint8 {
	result := (p.Ppustatus & StatusFlagMask) | (p.OpenBus & StatusOpenBusMask)
	p.Ppustatus &^= StatusVblank
	p.W = false
	p.OpenBus = result
	return result
}

func (p *Ppu) readOamdata() uint8 {
	value := p.Oam[p.Oamaddr]
	p.Oamaddr++
	p.OpenBus = value
	return value
}

func (p *Ppu) writeOamdata(value uint8) {
	p.Oam[p.Oamaddr] = value
	p.Oamaddr++
}

func (p *Ppu) writePpuctrl(value uint8) {
	nmiWasEnabled := (p.Ppuctrl & CtrlNMI) != 0
	p.Ppuctrl = value
	nt := uint16(value & CtrlBaseNTMask)
	p.T = (p.T & ^NtSelectMask) | (nt << 10)
	if (value&CtrlNMI) != 0 && p.InVblank() {
		if p.Render.NmiRetrigger || !nmiWasEnabled {
			p.NmiRequest = true
		}
	}
}

func (p *Ppu) writePpuscroll(value uint8) {
	if !p.W {
		p.T = (p.T & 0xFFE0) | uint16(value>>3)
		p.FineX = value & 0x07
		p.W = true
		p.Render.VDirty = true
	} else {
		p.T = (p.T & 0x8C1F) | (uint16(value&0xF8)<<2) | (uint16(value&0x07)<<12)
		p.W = false
	}
}

func (p *Ppu) writePpuaddr(value uint8) {
	if !p.W {
		p.T = (p.T & 0x00FF) | (uint16(value&0x3F) << 8)
		p.W = true
	} else {
		p.T = (p.T & 0xFF00) | uint16(value)
		p.V = p.T
		p.W = false
	}
}

// VramAddr returns the current VRAM address (v register).
func (p *Ppu) VramAddr() uint16 { return p.V }

// VramIncrement returns the VRAM address increment per PPUDATA access.
func (p *Ppu) VramIncrement() uint16 {
	if (p.Ppuctrl & CtrlIncrement32) != 0 {
		return 32
	}
	return 1
}

// AdvanceVramAddr advances v by the current increment, wrapping within 14-bit.
func (p *Ppu) AdvanceVramAddr() {
	inc := p.VramIncrement()
	p.V = (p.V + inc) & 0x3FFF
}

// SetPpudataBuffer sets the PPUDATA read buffer.
func (p *Ppu) SetPpudataBuffer(value uint8) { p.PpudataBuffer = value }

// SetOpenBus sets the open-bus latch.
func (p *Ppu) SetOpenBus(value uint8) { p.OpenBus = value }

func (p *Ppu) mapNametable(addr uint16) uint16 {
	a := addr & 0x2FFF
	local := a - 0x2000
	nt := local >> 10
	offset := local & 0x03FF
	var phys uint16
	switch p.Mirroring {
	case mapper.MirrorHorizontal:
		phys = nt >> 1
	case mapper.MirrorVertical:
		phys = nt & 1
	case mapper.MirrorFourScreen:
		phys = nt
	case mapper.MirrorSingleScreen0:
		phys = 0
	case mapper.MirrorSingleScreen1:
		phys = 1
	case mapper.MirrorSingleScreen2:
		phys = 2
	case mapper.MirrorSingleScreen3:
		phys = 3
	default:
		phys = nt >> 1
	}
	return phys*0x400 + offset
}

// ReadNametable reads a nametable byte at addr (in $2000-$3EFF).
func (p *Ppu) ReadNametable(addr uint16) uint8 {
	return p.Vram[p.mapNametable(addr)]
}

// WriteNametable writes a nametable byte at addr (in $2000-$3EFF).
func (p *Ppu) WriteNametable(addr uint16, value uint8) {
	p.Vram[p.mapNametable(addr)] = value
}

func mapPalette(addr uint16) uint16 {
	a := uint8(addr & 0x1F)
	if (a & 0x13) == 0x10 {
		return uint16(a & 0x0F)
	}
	return uint16(a)
}

// ReadPalette reads a palette byte at addr (in $3F00-$3FFF).
func (p *Ppu) ReadPalette(addr uint16) uint8 {
	return p.Palette[mapPalette(addr)]
}

// WritePalette writes a palette byte at addr (in $3F00-$3FFF).
func (p *Ppu) WritePalette(addr uint16, value uint8) {
	p.Palette[mapPalette(addr)] = value
}

// OamDMA copies 256 bytes into OAM, resetting oamaddr to 0.
func (p *Ppu) OamDMA(data [256]byte) {
	p.Oam = data
	p.Oamaddr = 0
}

func (p *Ppu) SetVblank(on bool) {
	if on {
		p.Ppustatus |= StatusVblank
	} else {
		p.Ppustatus &^= StatusVblank
	}
}

func (p *Ppu) SetSpriteZeroHit(on bool) {
	if on {
		p.Ppustatus |= StatusSpriteZero
	} else {
		p.Ppustatus &^= StatusSpriteZero
	}
}

func (p *Ppu) SetSpriteOverflow(on bool) {
	if on {
		p.Ppustatus |= StatusOverflow
	} else {
		p.Ppustatus &^= StatusOverflow
	}
}

// NmiEnabled returns true if VBlank NMI is enabled.
func (p *Ppu) NmiEnabled() bool { return (p.Ppuctrl & CtrlNMI) != 0 }

// InVblank returns true if the VBlank flag is set.
func (p *Ppu) InVblank() bool { return (p.Ppustatus & StatusVblank) != 0 }

// IsRendering returns true if background or sprites are enabled.
func (p *Ppu) IsRendering() bool {
	return (p.Ppumask & (MaskShowBg | MaskShowSprites)) != 0
}

// TakeNmiRequest consumes and returns the pending NMI request.
func (p *Ppu) TakeNmiRequest() bool {
	r := p.NmiRequest
	p.NmiRequest = false
	return r
}

func (p *Ppu) incrementHScroll() {
	if p.Render.SlantCorruption {
		if p.FineX < 7 {
			p.FineX++
			p.Render.FineXStart = (p.Render.FineXStart + 1) & 0x07
		} else {
			p.FineX = 0
			p.Render.FineXStart = 0
			coarseX := p.V & CoarseXMask
			if coarseX == 31 {
				p.V &^= CoarseXMask
				p.V ^= NtHBit
				p.Render.CoarseXStart = 0
				p.Render.NtHStart ^= NtHBit
			} else {
				p.V = (p.V & ^CoarseXMask) | (coarseX + 1)
				p.Render.CoarseXStart = (coarseX + 1) & CoarseXMask
			}
		}
	} else {
		coarseX := p.V & CoarseXMask
		if coarseX == 31 {
			p.V &^= CoarseXMask
			p.V ^= NtHBit
		} else {
			p.V = (p.V & ^CoarseXMask) | (coarseX + 1)
		}
	}
}

func (p *Ppu) incrementVScroll() {
	fineY := (p.V & FineYMask) >> 12
	if fineY < 7 {
		p.V = (p.V & ^FineYMask) | ((fineY + 1) << 12)
	} else {
		p.V &^= FineYMask
		coarseY := (p.V & CoarseYMask) >> 5
		if coarseY == 29 {
			p.V &^= CoarseYMask
			p.V ^= NtVBit
		} else if coarseY == 31 {
			p.V &^= CoarseYMask
		} else {
			p.V = (p.V & ^CoarseYMask) | ((coarseY + 1) << 5)
		}
	}
}

func (p *Ppu) copyHTtoV() {
	hBits := p.T & (CoarseXMask | NtHBit)
	p.V = (p.V & ^(CoarseXMask | NtHBit)) | hBits
}

func (p *Ppu) copyVTtoV() {
	vBits := p.T & (CoarseYMask | NtVBit | FineYMask)
	p.V = (p.V & ^(CoarseYMask | NtVBit | FineYMask)) | vBits
}

// Step advances the PPU by one cycle, returning true if NMI should be raised.
func (p *Ppu) Step() bool {
	var nmi bool
	scanlinesPerFrame := region.ScanlinesPerFrame(p.Region)
	prerender := region.ScanlinePrerender(p.Region)

	p.Cycle++
	if p.Cycle >= CyclesPerScanline {
		p.Cycle = 0
		p.Scanline++
		if p.Scanline >= scanlinesPerFrame {
			p.Scanline = 0
		}
	}

	if p.Scanline == ScanlineVblankStart && p.Cycle == vblankNmiCycle {
		p.SetVblank(true)
		if p.NmiEnabled() {
			p.NmiRequest = true
			nmi = true
		}
	} else if p.Scanline == prerender && p.Cycle == vblankNmiCycle {
		p.SetVblank(false)
		p.SetSpriteOverflow(false)
		p.SetSpriteZeroHit(false)
		p.Render.SpriteZeroHit = false
	}

	rendering := (p.Ppumask & (MaskShowBg | MaskShowSprites)) != 0
	if rendering {
		doesScrollInc := p.Scanline < ScreenHeight || p.Scanline == prerender
		if doesScrollInc &&
			p.Cycle >= hScrollIncStep &&
			p.Cycle <= hScrollIncLast &&
			(p.Cycle%hScrollIncStep) == 0 {
			p.incrementHScroll()
		}
		if doesScrollInc && p.Cycle == vertScrollIncCycle {
			p.incrementVScroll()
		}
		if doesScrollInc && p.Cycle == hCopyCycle {
			p.copyHTtoV()
		}
		if p.Scanline == prerender &&
			p.Cycle >= vCopyCycleStart &&
			p.Cycle <= vCopyCycleEnd {
			p.copyVTtoV()
		}
	}
	return nmi
}

// StepRendered advances the PPU by one cycle AND performs per-pixel rendering.
func (p *Ppu) StepRendered(chrRead ChrReader) bool {
	nmi := p.Step()
	if p.Cycle == 0 {
		p.Render.ScanlineInitialized = false
		p.Render.PipelinePrimed = false
	}
	if p.Scanline < ScreenHeight && p.Cycle >= 1 && p.Cycle <= ScreenWidth {
		p.RenderOnePixel(chrRead)
		p.Render.RenderedThisFrame = true
	}
	return nmi
}

// RenderedThisFrame returns whether the per-pixel renderer produced output.
func (p *Ppu) RenderedThisFrame() bool { return p.Render.RenderedThisFrame }

// ResetRenderedFlag resets the rendered_this_frame flag.
func (p *Ppu) ResetRenderedFlag() { p.Render.RenderedThisFrame = false }
