// Package cartridge implements the iNES cartridge loader
// (port of cores/c/src/cartridge.c).
package cartridge

import (
	"nes-core-go/internal/mapper"
)

const (
	HeaderSize    = 16
	TrainerSize   = 512
	PrgROMUnit    = 16384
	ChrROMUnit    = 8192
)

// InesHeader is the parsed iNES header fields relevant to emulation.
type InesHeader struct {
	PrgRomBanks uint8
	ChrRomBanks uint8
	MapperNumber uint16
	Mirroring    mapper.Mirroring
	HasTrainer   bool
	HasBattery   bool
	TvSystem     uint8
}

// Cartridge is a loaded NES cartridge - owns the parsed header and a Mapper.
type Cartridge struct {
	Header InesHeader
	Mapper *mapper.Mapper
}

var inesMagic = [4]byte{'N', 'E', 'S', 0x1A}
var fdsMagic = [4]byte{'F', 'D', 'S', 0x1A}

const (
	fdsHeaderSize   = 16
	fdsDiskSideSize = 65500
)

// ParseHeader parses the 16-byte iNES header.
func ParseHeader(b []byte) (InesHeader, int) {
	if len(b) < HeaderSize {
		return InesHeader{}, 1 // TooShort
	}
	if b[0] != inesMagic[0] || b[1] != inesMagic[1] || b[2] != inesMagic[2] || b[3] != inesMagic[3] {
		return InesHeader{}, 2 // BadMagic
	}
	prgBanks := b[4]
	chrBanks := b[5]
	flags6 := b[6]
	flags7 := b[7]
	hasTrainer := (flags6 & 0x04) != 0
	hasBattery := (flags6 & 0x02) != 0
	fourScreen := (flags6 & 0x08) != 0
	vertical := (flags6 & 0x01) != 0
	var mirr mapper.Mirroring
	if fourScreen {
		mirr = mapper.MirrorFourScreen
	} else if vertical {
		mirr = mapper.MirrorVertical
	} else {
		mirr = mapper.MirrorHorizontal
	}
	mapperNumber := uint16((flags6 >> 4) | (flags7 >> 4 << 4))
	tvSystem := b[9] & 0x03
	return InesHeader{
		PrgRomBanks:  prgBanks,
		ChrRomBanks:  chrBanks,
		MapperNumber: mapperNumber,
		Mirroring:    mirr,
		HasTrainer:   hasTrainer,
		HasBattery:   hasBattery,
		TvSystem:     tvSystem,
	}, 0
}

// FromBytes loads and parses an iNES ROM image from a byte buffer.
func FromBytes(b []byte) (*Cartridge, int) {
	hdr, rc := ParseHeader(b)
	if rc != 0 {
		return nil, rc
	}
	prgSize := uint32(hdr.PrgRomBanks) * PrgROMUnit
	prgOff := HeaderSize
	if hdr.HasTrainer {
		prgOff += TrainerSize
	}
	var chrSize uint32
	if hdr.ChrRomBanks > 0 {
		chrSize = uint32(hdr.ChrRomBanks) * ChrROMUnit
	}
	if len(b) < prgOff+int(prgSize)+int(chrSize) {
		return nil, 5 // TooShort
	}
	prg := b[prgOff : prgOff+int(prgSize)]
	var chr []byte
	if chrSize > 0 {
		chr = b[prgOff+int(prgSize) : prgOff+int(prgSize)+int(chrSize)]
	}
	m, rc2 := mapper.FromInes(hdr.MapperNumber, prg, chr, hdr.Mirroring, hdr.HasBattery)
	if rc2 != 0 {
		return nil, rc2
	}
	return &Cartridge{Header: hdr, Mapper: m}, 0
}

// FromFdsBytes builds an FDS cartridge from raw .fds disk image + BIOS.
func FromFdsBytes(diskData, bios []byte) (*Cartridge, int) {
	if len(diskData) < fdsHeaderSize {
		return nil, 1
	}
	if diskData[0] != fdsMagic[0] || diskData[1] != fdsMagic[1] || diskData[2] != fdsMagic[2] || diskData[3] != fdsMagic[3] {
		return nil, 2
	}
	diskCount := diskData[4]
	if diskCount == 0 {
		return nil, 6
	}
	needed := fdsHeaderSize + int(diskCount)*fdsDiskSideSize
	if len(diskData) < needed {
		return nil, 1
	}
	rawDisk := diskData[fdsHeaderSize : fdsHeaderSize+int(diskCount)*fdsDiskSideSize]
	m, rc := mapper.FdsCreate(bios, rawDisk)
	if rc != 0 {
		return nil, rc
	}
	return &Cartridge{
		Header: InesHeader{
			MapperNumber: 20,
			Mirroring:     mapper.MirrorVertical,
		},
		Mapper: m,
	}, 0
}

// ReadPRG reads a PRG byte.
func (c *Cartridge) ReadPRG(addr uint16) uint8 {
	if c.Mapper == nil {
		return 0
	}
	return c.Mapper.ReadPRG(addr)
}

// ReadPRGMut reads a PRG byte with side effects.
func (c *Cartridge) ReadPRGMut(addr uint16) uint8 {
	if c.Mapper == nil {
		return 0
	}
	return c.Mapper.ReadPRGMut(addr)
}

// WritePRG writes a PRG byte.
func (c *Cartridge) WritePRG(addr uint16, value uint8) {
	if c.Mapper == nil {
		return
	}
	c.Mapper.WritePRG(addr, value)
}

// ReadCHR reads a CHR byte.
func (c *Cartridge) ReadCHR(addr uint16) uint8 {
	if c.Mapper == nil {
		return 0
	}
	return c.Mapper.ReadCHR(addr)
}

// ReadCHRLatched reads a CHR byte with side effects (MMC2 latching).
func (c *Cartridge) ReadCHRLatched(addr uint16) uint8 {
	if c.Mapper == nil {
		return 0
	}
	return c.Mapper.ReadCHRLatched(addr)
}

// WriteCHR writes a CHR byte.
func (c *Cartridge) WriteCHR(addr uint16, value uint8) {
	if c.Mapper == nil {
		return
	}
	c.Mapper.WriteCHR(addr, value)
}

// MirrorMode returns the nametable mirroring mode.
func (c *Cartridge) MirrorMode() mapper.Mirroring {
	if c.Mapper == nil {
		return c.Header.Mirroring
	}
	return c.Mapper.MirrorMode()
}

// ChrIsRAM returns whether CHR is writable RAM.
func (c *Cartridge) ChrIsRAM() bool {
	if c.Mapper == nil {
		return false
	}
	return c.Mapper.ChrIsRAM()
}

// HasBattery returns whether the cartridge has battery-backed PRG-RAM.
func (c *Cartridge) HasBattery() bool {
	if c.Mapper == nil {
		return false
	}
	return c.Mapper.HasBattery()
}

// IrqPending returns whether the mapper is asserting a CPU IRQ.
func (c *Cartridge) IrqPending() bool {
	if c.Mapper == nil {
		return false
	}
	return c.Mapper.IrqPending()
}

// ClockIRQ clocks the mapper's IRQ counter by one step.
func (c *Cartridge) ClockIRQ() {
	if c.Mapper == nil {
		return
	}
	c.Mapper.ClockIRQ()
}

// ResetScanlineCounter resets the mapper's per-frame scanline counter.
func (c *Cartridge) ResetScanlineCounter() {
	if c.Mapper == nil {
		return
	}
	c.Mapper.ResetScanlineCounter()
}

// ClockCPU advances the mapper's CPU-clocked logic by cpu_cycles.
func (c *Cartridge) ClockCPU(cpuCycles uint32) {
	if c.Mapper == nil {
		return
	}
	c.Mapper.ClockCPU(cpuCycles)
}

// ExpansionAudioSample returns the current expansion-audio sample.
func (c *Cartridge) ExpansionAudioSample() float32 {
	if c.Mapper == nil {
		return 0
	}
	return c.Mapper.ExpansionAudioSample()
}

// SaveState serializes the mapper state into buf (nil = size query).
func (c *Cartridge) SaveState(buf []byte) int {
	if c.Mapper == nil {
		return 0
	}
	return c.Mapper.SaveState(buf)
}

// LoadState restores mapper state from buf.
func (c *Cartridge) LoadState(buf []byte) bool {
	if c.Mapper == nil {
		return false
	}
	return c.Mapper.LoadState(buf)
}
