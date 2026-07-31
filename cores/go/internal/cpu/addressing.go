// Package cpu implements the 6502 CPU core (port of cores/c/src/cpu.c +
// addressing.c + cpu_unofficial.c).
package cpu

import "nes-core-go/internal/bus"

// AddrMode is the 6502 addressing mode enum.
type AddrMode int

const (
	ModeImplied AddrMode = iota
	ModeAccumulator
	ModeImmediate
	ModeZeroPage
	ModeZeroPageX
	ModeZeroPageY
	ModeAbsolute
	ModeAbsoluteX
	ModeAbsoluteY
	ModeIndirect
	ModeIndirectX
	ModeIndirectY
	ModeRelative
)

// Dummy is the dummy-read behaviour for indexed addressing modes.
type Dummy int

const (
	DummyNone Dummy = iota
	DummyRead // page-cross only
	DummyRMW  // unconditional
)

// Operand tag values for the tagged-union Operand.
const (
	OperandNone = 0
	OperandAcc  = 1
	OperandAddr = 2
)

// Operand is the result of resolving an addressing mode.
type Operand struct {
	Tag  int
	Addr uint16
}

func operandNone() Operand          { return Operand{Tag: OperandNone} }
func operandAcc() Operand           { return Operand{Tag: OperandAcc} }
func operandAddr(a uint16) Operand  { return Operand{Tag: OperandAddr, Addr: a} }

// Resolve resolves the effective address for mode (no dummy reads).
func (c *Cpu) Resolve(b *bus.Bus, mode AddrMode) Operand {
	switch mode {
	case ModeImplied:
		return operandNone()
	case ModeAccumulator:
		return operandAcc()
	case ModeImmediate:
		return operandAddr(c.amImmediate())
	case ModeZeroPage:
		return operandAddr(c.amZeroPage(b))
	case ModeZeroPageX:
		return operandAddr(c.amZeroPageX(b, DummyNone))
	case ModeZeroPageY:
		return operandAddr(c.amZeroPageY(b, DummyNone))
	case ModeAbsolute:
		return operandAddr(c.amAbsolute(b))
	case ModeAbsoluteX:
		_, a := c.amAbsoluteX(b, DummyNone)
		return operandAddr(a)
	case ModeAbsoluteY:
		_, a := c.amAbsoluteY(b, DummyNone)
		return operandAddr(a)
	case ModeIndirect:
		return operandAddr(c.amIndirect(b))
	case ModeIndirectX:
		return operandAddr(c.amIndirectX(b))
	case ModeIndirectY:
		_, a := c.amIndirectY(b, DummyNone)
		return operandAddr(a)
	case ModeRelative:
		return operandAddr(c.amRelative(b))
	default:
		return operandNone()
	}
}

func (c *Cpu) amImmediate() uint16 {
	addr := c.PC
	c.PC++
	return addr
}

func (c *Cpu) amZeroPage(b *bus.Bus) uint16 {
	return uint16(c.fetchByte(b))
}

func (c *Cpu) amZeroPageX(b *bus.Bus, dummy Dummy) uint16 {
	base := c.fetchByte(b)
	if dummy != DummyNone {
		_ = b.Read(uint16(base))
	}
	return uint16(base + c.X)
}

func (c *Cpu) amZeroPageY(b *bus.Bus, dummy Dummy) uint16 {
	base := c.fetchByte(b)
	if dummy != DummyNone {
		_ = b.Read(uint16(base))
	}
	return uint16(base + c.Y)
}

func (c *Cpu) amAbsolute(b *bus.Bus) uint16 {
	return c.fetchWord(b)
}

func (c *Cpu) amAbsoluteX(b *bus.Bus, dummy Dummy) (pageCross bool, eff uint16) {
	base := c.fetchWord(b)
	eff = base + uint16(c.X)
	cross := (base & 0xFF00) != (eff & 0xFF00)
	if (dummy == DummyRead && cross) || dummy == DummyRMW {
		_ = b.Read((base & 0xFF00) | (eff & 0x00FF))
	}
	return cross, eff
}

func (c *Cpu) amAbsoluteY(b *bus.Bus, dummy Dummy) (pageCross bool, eff uint16) {
	base := c.fetchWord(b)
	eff = base + uint16(c.Y)
	cross := (base & 0xFF00) != (eff & 0xFF00)
	if (dummy == DummyRead && cross) || dummy == DummyRMW {
		_ = b.Read((base & 0xFF00) | (eff & 0x00FF))
	}
	return cross, eff
}

func (c *Cpu) amIndirect(b *bus.Bus) uint16 {
	ptr := c.fetchWord(b)
	lo := b.Read(ptr)
	hiAddr := (ptr & 0xFF00) | uint16(byte(ptr+1))
	hi := b.Read(hiAddr)
	return uint16(lo) | (uint16(hi) << 8)
}

func (c *Cpu) amIndirectX(b *bus.Bus) uint16 {
	zp := c.fetchByte(b)
	ptr := zp + c.X
	lo := b.Read(uint16(ptr))
	hi := b.Read(uint16(byte(ptr + 1)))
	return uint16(lo) | (uint16(hi) << 8)
}

func (c *Cpu) amIndirectY(b *bus.Bus, dummy Dummy) (pageCross bool, eff uint16) {
	zp := c.fetchByte(b)
	lo := b.Read(uint16(zp))
	hi := b.Read(uint16(byte(zp + 1)))
	base := uint16(lo) | (uint16(hi) << 8)
	eff = base + uint16(c.Y)
	cross := (base & 0xFF00) != (eff & 0xFF00)
	if (dummy == DummyRead && cross) || dummy == DummyRMW {
		_ = b.Read((base & 0xFF00) | (eff & 0x00FF))
	}
	return cross, eff
}

func (c *Cpu) amRelative(b *bus.Bus) uint16 {
	offset := int8(c.fetchByte(b))
	return c.PC + uint16(int16(offset))
}

// amIndirectYBase reads the (zp),Y base pointer without adding Y.
func (c *Cpu) amIndirectYBase(b *bus.Bus) uint16 {
	zp := c.fetchByte(b)
	lo := b.Read(uint16(zp))
	hi := b.Read(uint16(byte(zp + 1)))
	return uint16(lo) | (uint16(hi) << 8)
}
