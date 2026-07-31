// Package savestate implements full emulator state serialization
// (port of cores/c/src/save_state.c). Uses a custom binary format with a
// magic + version tag. All integers are little-endian.
package savestate

import (
	"math"

	"nes-core-go/internal/apu"
	"nes-core-go/internal/bus"
	"nes-core-go/internal/cartridge"
	"nes-core-go/internal/cpu"
	"nes-core-go/internal/joypad"
	"nes-core-go/internal/mapper"
	"nes-core-go/internal/ppu"
	"nes-core-go/internal/region"
)

// Magic / version.
const (
	Magic      uint32 = 0x5353454E // "NESS" LE
	Version    uint32 = 1
	HeaderSize = 12
	RAMSize    = bus.RAMSize
	ApuIORegCount = bus.APUIORegCount
)

// ---- LE helpers ----

func putU32(p []byte, v uint32) {
	p[0] = byte(v)
	p[1] = byte(v >> 8)
	p[2] = byte(v >> 16)
	p[3] = byte(v >> 24)
}
func getU32(p []byte) uint32 {
	return uint32(p[0]) | (uint32(p[1]) << 8) | (uint32(p[2]) << 16) | (uint32(p[3]) << 24)
}
func putU16(p []byte, v uint16) {
	p[0] = byte(v)
	p[1] = byte(v >> 8)
}
func getU16(p []byte) uint16 {
	return uint16(p[0]) | (uint16(p[1]) << 8)
}
func putU64(p []byte, v uint64) {
	for i := 0; i < 8; i++ {
		p[i] = byte(v >> (i * 8))
	}
}
func getU64(p []byte) uint64 {
	var v uint64
	for i := 0; i < 8; i++ {
		v |= uint64(p[i]) << (i * 8)
	}
	return v
}
func putFloat(p []byte, f float32) { putU32(p, math.Float32bits(f)) }
func getFloat(p []byte) float32    { return math.Float32frombits(getU32(p)) }
func boolByte(b bool) byte {
	if b {
		return 1
	}
	return 0
}

// ---- PPU architectural-state payload size ----

func ppuStateSize(vramSize uint32) int {
	return 4 + int(vramSize) + 256 + 32 + 4 + 2 + 2 + 1 + 1 + 1 + 1 + 2 + 2 + 1 + 4 + 4
}

// State is the input bundle for Save/Load (avoids import cycles: this package
// imports all leaf packages; the caller assembles State from its Emulator).
type State struct {
	Cpu           *cpu.Cpu
	Bus           *bus.Bus
	Cartridge     *cartridge.Cartridge
	Region        region.Region
	SampleAcc     float32
	AudioBuffer   []float32
	PpuCycleCarry uint32
}

// RequiredSize returns the total bytes needed to save the given state.
func RequiredSize(s *State) int {
	payload := 0
	payload += cpuStateSize()
	payload += RAMSize
	payload += ppuStateSize(s.Bus.PPU.VramSize)
	payload += ApuIORegCount
	payload += apuStateSize()
	payload += joypadStateSize()
	if s.Cartridge != nil {
		payload += 1 + inesHeaderSize()
		payload += s.Cartridge.SaveState(nil)
	} else {
		payload += 1
	}
	payload += 4 + 8 + 4 + 4 + len(s.AudioBuffer)*4 + 4 + 4
	return HeaderSize + payload
}

func cpuStateSize() int    { return 8 }
func apuStateSize() int    { return apu.SerializedSize() }
func joypadStateSize() int { return joypad.SerializedSize() }
func inesHeaderSize() int  { return 16 }

// Save serializes the state into buf. If buf is nil, returns the required size.
// Returns bytes written, or 0 if buf is too small.
func Save(s *State, buf []byte) int {
	required := RequiredSize(s)
	if buf == nil {
		return required
	}
	if len(buf) < required {
		return 0
	}
	payload := required - HeaderSize
	off := 0
	putU32(buf[off:], Magic); off += 4
	putU32(buf[off:], Version); off += 4
	putU32(buf[off:], uint32(payload)); off += 4

	off += cpuWrite(s.Cpu, buf[off:])
	copy(buf[off:], s.Bus.RAM[:]); off += RAMSize
	off += ppuWrite(&s.Bus.PPU, buf[off:])
	copy(buf[off:], s.Bus.ApuOpenBus[:]); off += ApuIORegCount
	off += apu.Serialize(&s.Bus.APU, buf[off:])
	off += joypad.Serialize(&s.Bus.Joypad, buf[off:])
	if s.Cartridge != nil {
		buf[off] = 1; off++
		off += inesHeaderWrite(&s.Cartridge.Header, buf[off:])
		msz := s.Cartridge.SaveState(buf[off:])
		off += msz
	} else {
		buf[off] = 0; off++
	}
	putU32(buf[off:], s.Bus.DmaStallCycles); off += 4
	putU64(buf[off:], s.Bus.CpuCycleCount); off += 8
	putFloat(buf[off:], s.SampleAcc); off += 4
	putU32(buf[off:], uint32(len(s.AudioBuffer))); off += 4
	for i, v := range s.AudioBuffer {
		putFloat(buf[off+i*4:], v)
	}
	off += len(s.AudioBuffer) * 4
	putU32(buf[off:], s.PpuCycleCarry); off += 4
	putU32(buf[off:], uint32(s.Region)); off += 4
	return off
}

// Load restores state from buf. Returns true on success.
func Load(s *State, buf []byte) bool {
	if len(buf) < HeaderSize {
		return false
	}
	magic := getU32(buf[0:])
	version := getU32(buf[4:])
	payload := getU32(buf[8:])
	if magic != Magic || version != Version {
		return false
	}
	if int(payload)+HeaderSize > len(buf) {
		return false
	}
	off := HeaderSize
	end := HeaderSize + int(payload)

	if off+cpuStateSize() > end {
		return false
	}
	off += cpuRead(s.Cpu, buf[off:])
	if off+RAMSize > end {
		return false
	}
	copy(s.Bus.RAM[:], buf[off:off+RAMSize]); off += RAMSize
	if off+4 > end {
		return false
	}
	vramSize := getU32(buf[off:])
	ppuNeed := ppuStateSize(vramSize)
	if off+ppuNeed > end {
		return false
	}
	n := ppuRead(&s.Bus.PPU, buf[off:], end-off)
	if n == 0 {
		return false
	}
	off += n
	if off+ApuIORegCount > end {
		return false
	}
	copy(s.Bus.ApuOpenBus[:], buf[off:off+ApuIORegCount]); off += ApuIORegCount
	if off+apuStateSize() > end {
		return false
	}
	off += apu.Deserialize(&s.Bus.APU, buf[off:])
	if off+joypadStateSize() > end {
		return false
	}
	off += joypad.Deserialize(&s.Bus.Joypad, buf[off:])
	if off+1 > end {
		return false
	}
	cartPresent := buf[off]; off++
	if cartPresent != 0 {
		if off+inesHeaderSize() > end {
			return false
		}
		var hdr cartridge.InesHeader
		inesHeaderRead(&hdr, buf[off:]); off += inesHeaderSize()
		if s.Cartridge == nil {
			return false
		}
		if hdr.MapperNumber != s.Cartridge.Header.MapperNumber {
			return false
		}
		s.Cartridge.Header = hdr
		msz := s.Cartridge.SaveState(nil)
		if off+msz > end {
			return false
		}
		if !s.Cartridge.LoadState(buf[off : off+msz]) {
			return false
		}
		off += msz
		s.Bus.SetMirroringFromCart()
	} else {
		s.Bus.RemoveCartridge()
	}
	if off+4 > end {
		return false
	}
	s.Bus.SetDmaStallCycles(getU32(buf[off:])); off += 4
	if off+8 > end {
		return false
	}
	s.Bus.SetCPUCycleCount(getU64(buf[off:])); off += 8
	if off+4 > end {
		return false
	}
	s.SampleAcc = getFloat(buf[off:]); off += 4
	if off+4 > end {
		return false
	}
	audioCount := getU32(buf[off:]); off += 4
	if audioCount > uint32(end-off)/4 {
		return false
	}
	if int(audioCount) > cap(s.AudioBuffer) {
		s.AudioBuffer = make([]float32, audioCount)
	} else {
		s.AudioBuffer = s.AudioBuffer[:audioCount]
	}
	for i := uint32(0); i < audioCount; i++ {
		s.AudioBuffer[i] = getFloat(buf[off+int(i)*4:])
	}
	off += int(audioCount) * 4
	if off+4 > end {
		return false
	}
	s.PpuCycleCarry = getU32(buf[off:]); off += 4
	if off+4 > end {
		return false
	}
	regn := getU32(buf[off:]); off += 4
	if regn <= uint32(region.DENDY) {
		s.Region = region.Region(regn)
		s.Bus.PPU.SetRegion(s.Region)
		s.Bus.APU.SetRegion(s.Region)
	}
	ub := s.Bus.PPU.UniversalBgArgb()
	s.Bus.PPU.ClearFramebuffer(ub)
	return true
}

// ---- per-component serializers ----

func cpuWrite(c *cpu.Cpu, buf []byte) int {
	buf[0] = c.A
	buf[1] = c.X
	buf[2] = c.Y
	buf[3] = c.SP
	putU16(buf[4:], c.PC)
	buf[6] = c.Status
	buf[7] = c.Flags
	return 8
}

func cpuRead(c *cpu.Cpu, buf []byte) int {
	c.A = buf[0]
	c.X = buf[1]
	c.Y = buf[2]
	c.SP = buf[3]
	c.PC = getU16(buf[4:])
	c.Status = buf[6]
	c.Flags = buf[7]
	return 8
}

func ppuWrite(p *ppu.Ppu, buf []byte) int {
	off := 0
	putU32(buf[off:], p.VramSize); off += 4
	copy(buf[off:], p.Vram[:p.VramSize]); off += int(p.VramSize)
	copy(buf[off:], p.Oam[:]); off += 256
	copy(buf[off:], p.Palette[:]); off += 32
	buf[off] = p.Ppuctrl; off++
	buf[off] = p.Ppumask; off++
	buf[off] = p.Oamaddr; off++
	buf[off] = p.Ppustatus; off++
	putU16(buf[off:], p.V); off += 2
	putU16(buf[off:], p.T); off += 2
	buf[off] = p.FineX; off++
	buf[off] = boolByte(p.W); off++
	buf[off] = p.PpudataBuffer; off++
	buf[off] = p.OpenBus; off++
	putU16(buf[off:], p.Scanline); off += 2
	putU16(buf[off:], p.Cycle); off += 2
	buf[off] = boolByte(p.NmiRequest); off++
	putU32(buf[off:], uint32(p.Mirroring)); off += 4
	putU32(buf[off:], uint32(p.Region)); off += 4
	return off
}

func ppuRead(p *ppu.Ppu, buf []byte, available int) int {
	if available < 4 {
		return 0
	}
	vramSize := getU32(buf[0:])
	need := ppuStateSize(vramSize)
	if available < need {
		return 0
	}
	if vramSize > 0x1000 {
		return 0
	}
	mirrOff := 4 + int(vramSize) + 256 + 32 + 4 + 2 + 2 + 1 + 1 + 1 + 1 + 2 + 2 + 1
	mirr := getU32(buf[mirrOff:])
	regn := getU32(buf[mirrOff+4:])
	if mirr <= uint32(mapper.MirrorSingleScreen3) {
		p.SetMirroring(mapper.Mirroring(mirr))
	}
	if regn <= uint32(region.DENDY) {
		p.SetRegion(region.Region(regn))
	}
	off := 4
	copy(p.Vram[:vramSize], buf[off:off+int(vramSize)]); off += int(vramSize)
	p.VramSize = vramSize
	copy(p.Oam[:], buf[off:off+256]); off += 256
	copy(p.Palette[:], buf[off:off+32]); off += 32
	p.Ppuctrl = buf[off]; off++
	p.Ppumask = buf[off]; off++
	p.Oamaddr = buf[off]; off++
	p.Ppustatus = buf[off]; off++
	p.V = getU16(buf[off:]); off += 2
	p.T = getU16(buf[off:]); off += 2
	p.FineX = buf[off]; off++
	p.W = buf[off] != 0; off++
	p.PpudataBuffer = buf[off]; off++
	p.OpenBus = buf[off]; off++
	p.Scanline = getU16(buf[off:]); off += 2
	p.Cycle = getU16(buf[off:]); off += 2
	p.NmiRequest = buf[off] != 0; off++
	off += 4
	off += 4
	return off
}

func inesHeaderWrite(h *cartridge.InesHeader, buf []byte) int {
	buf[0] = h.PrgRomBanks
	buf[1] = h.ChrRomBanks
	putU16(buf[2:], h.MapperNumber)
	buf[4] = byte(h.Mirroring)
	buf[5] = boolByte(h.HasTrainer)
	buf[6] = boolByte(h.HasBattery)
	buf[7] = h.TvSystem
	for i := 8; i < 16; i++ {
		buf[i] = 0
	}
	return 16
}

func inesHeaderRead(h *cartridge.InesHeader, buf []byte) {
	h.PrgRomBanks = buf[0]
	h.ChrRomBanks = buf[1]
	h.MapperNumber = getU16(buf[2:])
	h.Mirroring = mapper.Mirroring(buf[4])
	h.HasTrainer = buf[5] != 0
	h.HasBattery = buf[6] != 0
	h.TvSystem = buf[7]
}
