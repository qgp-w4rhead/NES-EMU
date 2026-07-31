// Package bus implements the CPU memory bus: address-space routing + mirroring
// (port of cores/c/src/bus.c).
package bus

import (
	"nes-core-go/internal/apu"
	"nes-core-go/internal/cartridge"
	"nes-core-go/internal/joypad"
	"nes-core-go/internal/mapper"
	"nes-core-go/internal/ppu"
	"nes-core-go/internal/region"
)

const (
	RAMSize        = 0x0800
	RAMMask        = 0x07FF
	PPURegCount    = 8
	PPURegMask     = 0x0007
	PPURegBase     = 0x2000
	APUIOBase      = 0x4000
	APUIORegCount  = 0x18
	CartBase       = 0x4020
	APUIOTestEnd   = 0x401F
	mmc3IrqCycle   = 260
)

// Bus is the CPU memory bus: owns RAM, the PPU, APU, joypad, open-bus latches,
// and a non-owning reference to the loaded cartridge.
type Bus struct {
	RAM         [RAMSize]byte
	PPU         ppu.Ppu
	APU         apu.Apu
	ApuOpenBus  [APUIORegCount]byte
	Joypad      joypad.Joypad
	Cartridge   *cartridge.Cartridge
	DmaStallCycles uint32
	CpuCycleCount uint64
}

// Init constructs an empty bus with no cartridge.
func (b *Bus) Init() {
	*b = Bus{}
	b.PPU.Init()
	b.APU.Init()
	b.Joypad.Init()
}

// InitWithCartridge constructs a bus with a loaded cartridge (non-owning).
func (b *Bus) InitWithCartridge(c *cartridge.Cartridge) {
	b.Init()
	b.Cartridge = c
	if c != nil {
		b.PPU.SetMirroring(c.MirrorMode())
	}
}

// InsertCartridge replaces the loaded cartridge and returns the previous one.
func (b *Bus) InsertCartridge(c *cartridge.Cartridge) *cartridge.Cartridge {
	prev := b.Cartridge
	b.Cartridge = c
	if c != nil {
		b.PPU.SetMirroring(c.MirrorMode())
	}
	return prev
}

// RemoveCartridge removes the loaded cartridge, returning the previous one.
func (b *Bus) RemoveCartridge() *cartridge.Cartridge {
	prev := b.Cartridge
	b.Cartridge = nil
	return prev
}

// ---- PPUDATA routing helpers ----

func (b *Bus) ppuReadPpudata() uint8 {
	addr := b.PPU.VramAddr()
	if addr >= 0x3F00 {
		pal := b.PPU.ReadPalette(addr)
		val := (pal & 0x3F) | (b.PPU.OpenBus & 0xC0)
		ntAddr := addr & 0x2FFF
		var bufferedFill uint8
		if ntAddr < 0x2000 {
			if b.Cartridge != nil {
				bufferedFill = b.Cartridge.ReadCHR(ntAddr)
			}
		} else {
			bufferedFill = b.PPU.ReadNametable(ntAddr)
		}
		b.PPU.SetPpudataBuffer(bufferedFill)
		b.PPU.AdvanceVramAddr()
		b.PPU.SetOpenBus(val)
		return val
	}
	buffered := b.PPU.PpudataBuffer
	var raw uint8
	if addr < 0x2000 {
		if b.Cartridge != nil {
			raw = b.Cartridge.ReadCHR(addr)
		}
	} else {
		raw = b.PPU.ReadNametable(addr)
	}
	b.PPU.SetPpudataBuffer(raw)
	b.PPU.AdvanceVramAddr()
	b.PPU.SetOpenBus(buffered)
	return buffered
}

func (b *Bus) ppuWritePpudata(value uint8) {
	addr := b.PPU.VramAddr()
	if addr >= 0x3F00 {
		b.PPU.WritePalette(addr, value)
	} else if addr < 0x2000 {
		if b.Cartridge != nil {
			b.Cartridge.WriteCHR(addr, value)
		}
	} else {
		b.PPU.WriteNametable(addr, value)
	}
	b.PPU.WriteRegister(7, value) // latches open_bus; reg 7 no-op
	b.PPU.AdvanceVramAddr()
}

func (b *Bus) ppuRead(reg uint16) uint8 {
	if (reg & 0x07) == 7 {
		return b.ppuReadPpudata()
	}
	return b.PPU.ReadRegister(reg)
}

func (b *Bus) ppuWrite(reg uint16, value uint8) {
	if (reg & 0x07) == 7 {
		b.ppuWritePpudata(value)
		return
	}
	b.PPU.WriteRegister(reg, value)
}

// ---- APU register routing ----

func (b *Bus) apuPulseWrite(offset uint16, value uint8) {
	if offset < 4 {
		b.APU.Pulse1.WriteRegister(uint8(offset), value)
	} else {
		b.APU.Pulse2.WriteRegister(uint8(offset-4), value)
	}
}

func (b *Bus) apuTriangleWrite(offset uint16, value uint8) {
	b.APU.Triangle.WriteRegister(uint8(offset-0x08), value)
}

func (b *Bus) apuNoiseWrite(offset uint16, value uint8) {
	b.APU.Noise.WriteRegister(uint8(offset-0x0C), value)
}

func (b *Bus) apuDmcWrite(offset uint16, value uint8) {
	b.APU.Dmc.WriteRegister(uint8(offset-0x10), value)
}

func (b *Bus) apuStatusRead() uint8 {
	status := b.APU.ReadStatus()
	return (status & 0xDF) | (b.ApuOpenBus[0x15] & 0x20)
}

// ---- OAM-DMA ----

func (b *Bus) oamDMA(page uint8) {
	base := uint16(page) << 8
	var data [256]byte
	for i := uint16(0); i < 256; i++ {
		data[i] = b.Read(base + i)
	}
	b.PPU.OamDMA(data)
	b.ApuOpenBus[0x14] = page
	b.PPU.SetOpenBus(page)
	var stall uint32
	if b.CpuCycleCount&1 != 0 {
		stall = 513
	} else {
		stall = 512
	}
	b.DmaStallCycles += stall
}

func (b *Bus) cartRead(addr uint16) uint8 {
	if b.Cartridge != nil {
		return b.Cartridge.ReadPRGMut(addr)
	}
	return 0
}

func (b *Bus) cartWrite(addr uint16, value uint8) {
	if b.Cartridge != nil {
		b.Cartridge.WritePRG(addr, value)
		b.PPU.SetMirroring(b.Cartridge.MirrorMode())
	}
}

// Read reads a byte from CPU address space (side-effectful).
func (b *Bus) Read(addr uint16) uint8 {
	switch {
	case addr <= 0x1FFF:
		return b.RAM[addr&RAMMask]
	case addr <= 0x3FFF:
		return b.ppuRead(addr & PPURegMask)
	case addr <= 0x4007:
		return b.ApuOpenBus[addr-APUIOBase]
	case addr <= 0x400B:
		return b.ApuOpenBus[addr-APUIOBase]
	case addr <= 0x400F:
		return b.ApuOpenBus[addr-APUIOBase]
	case addr <= 0x4013:
		return b.ApuOpenBus[addr-APUIOBase]
	case addr == 0x4014:
		return b.ApuOpenBus[0x14]
	case addr == 0x4015:
		return b.apuStatusRead()
	case addr == 0x4016:
		ob := b.ApuOpenBus[0x16]
		jb := b.Joypad.Read(0)
		return (jb & 0x01) | (ob & 0xFE)
	case addr == 0x4017:
		ob := b.ApuOpenBus[0x17]
		jb := b.Joypad.Read(1)
		return (jb & 0x01) | (ob & 0xFE)
	case addr <= 0x401F:
		return 0
	default:
		return b.cartRead(addr)
	}
}

// Write writes a byte to CPU address space.
func (b *Bus) Write(addr uint16, value uint8) {
	switch {
	case addr <= 0x1FFF:
		b.RAM[addr&RAMMask] = value
	case addr <= 0x3FFF:
		b.ppuWrite(addr&PPURegMask, value)
	case addr <= 0x4007:
		offset := addr - APUIOBase
		b.apuPulseWrite(offset, value)
		b.ApuOpenBus[offset] = value
	case addr <= 0x400B:
		offset := addr - APUIOBase
		b.apuTriangleWrite(offset, value)
		b.ApuOpenBus[offset] = value
	case addr <= 0x400F:
		offset := addr - APUIOBase
		b.apuNoiseWrite(offset, value)
		b.ApuOpenBus[offset] = value
	case addr <= 0x4013:
		offset := addr - APUIOBase
		b.apuDmcWrite(offset, value)
		b.ApuOpenBus[offset] = value
	case addr == 0x4014:
		b.oamDMA(value)
	case addr == 0x4015:
		b.ApuOpenBus[0x15] = value
		b.APU.WriteStatus(value)
	case addr == 0x4016:
		b.ApuOpenBus[0x16] = value
		b.Joypad.WriteStrobe(value)
	case addr == 0x4017:
		b.ApuOpenBus[0x17] = value
		b.APU.WriteFrameCounter(value)
	case addr <= 0x401F:
		// disabled test region
	default:
		b.cartWrite(addr, value)
	}
}

// Peek is a side-effect-free read for debug tools.
func (b *Bus) Peek(addr uint16) uint8 {
	switch {
	case addr <= 0x1FFF:
		return b.RAM[addr&RAMMask]
	case addr <= 0x401F:
		return 0
	default:
		if b.Cartridge != nil {
			return b.Cartridge.ReadPRG(addr)
		}
		return 0
	}
}

// TakeDmaStallCycles consumes and returns pending OAM-DMA stall cycles.
func (b *Bus) TakeDmaStallCycles() uint32 {
	c := b.DmaStallCycles
	b.DmaStallCycles = 0
	return c
}

// AdvanceCPUCycles advances the CPU cycle counter.
func (b *Bus) AdvanceCPUCycles(cycles uint32) {
	b.CpuCycleCount += uint64(cycles)
}

// SetCPUCycleCount sets the CPU cycle counter.
func (b *Bus) SetCPUCycleCount(count uint64) { b.CpuCycleCount = count }

// SetDmaStallCycles sets the pending DMA stall cycles.
func (b *Bus) SetDmaStallCycles(cycles uint32) { b.DmaStallCycles = cycles }

// ---- APU stepping ----

func (b *Bus) dmcRead(addr uint16) uint8 {
	if addr <= 0x1FFF {
		return b.RAM[addr&RAMMask]
	}
	if addr >= 0x8000 {
		if b.Cartridge != nil {
			return b.Cartridge.ReadPRG(addr)
		}
	}
	return 0
}

// StepAPU advances the APU by cpuCycles CPU cycles.
func (b *Bus) StepAPU(cpuCycles uint32) {
	b.APU.Step(cpuCycles, b.dmcRead)
}

// ApuIrqPending returns whether the APU has a pending IRQ.
func (b *Bus) ApuIrqPending() bool { return b.APU.IrqPending() }

// ---- Cartridge mapper integration ----

func (b *Bus) CartIrqPending() bool {
	if b.Cartridge != nil {
		return b.Cartridge.IrqPending()
	}
	return false
}

func (b *Bus) ClockCartCPU(cpuCycles uint32) {
	if b.Cartridge != nil {
		b.Cartridge.ClockCPU(cpuCycles)
	}
}

func (b *Bus) ExpansionAudioSample() float32 {
	if b.Cartridge != nil {
		return b.Cartridge.ExpansionAudioSample()
	}
	return 0
}

func (b *Bus) CartResetScanlineCounter() {
	if b.Cartridge != nil {
		b.Cartridge.ResetScanlineCounter()
	}
}

// ---- PPU stepping + rendering ----

func (b *Bus) chrRead(addr uint16) uint8 {
	if b.Cartridge != nil {
		return b.Cartridge.ReadCHRLatched(addr)
	}
	return 0
}

// StepPPU advances the PPU by cycles PPU cycles; returns true if NMI requested.
func (b *Bus) StepPPU(cycles uint32) bool {
	var nmi bool
	prerender := region.ScanlinePrerender(b.PPU.Region)
	rendering := b.PPU.IsRendering()
	for i := uint32(0); i < cycles; i++ {
		if b.PPU.StepRendered(b.chrRead) {
			nmi = true
		}
		cyc := b.PPU.Cycle
		sl := b.PPU.Scanline
		if rendering && cyc == mmc3IrqCycle && (sl < ppu.ScreenHeight || sl == prerender) {
			if b.Cartridge != nil {
				b.Cartridge.ClockIRQ()
			}
		}
		if sl == prerender && cyc == 1 {
			if b.Cartridge != nil {
				b.Cartridge.ResetScanlineCounter()
			}
		}
	}
	return nmi
}

// TakeNmiRequest consumes and returns the pending PPU NMI request flag.
func (b *Bus) TakeNmiRequest() bool { return b.PPU.TakeNmiRequest() }

// RenderFrame renders the full frame into the PPU framebuffer.
func (b *Bus) RenderFrame() {
	b.PPU.RenderFrame(b.chrRead)
}

// ---- Mirroring helper for save_state ----

// SetMirroringFromCart re-syncs PPU mirroring from the loaded cartridge.
func (b *Bus) SetMirroringFromCart() {
	if b.Cartridge != nil {
		b.PPU.SetMirroring(b.Cartridge.MirrorMode())
	}
}

// keep mapper import used
var _ = mapper.MirrorHorizontal
