// cpu.go - 6502 CPU core: registers, flags, step/reset/nmi/irq, stack, helpers.
package cpu

import "nes-core-go/internal/bus"

// Status flag bit masks.
const (
	FlagC uint8 = 0x01
	FlagZ uint8 = 0x02
	FlagI uint8 = 0x04
	FlagD uint8 = 0x08
	FlagB uint8 = 0x10
	FlagU uint8 = 0x20
	FlagV uint8 = 0x40
	FlagN uint8 = 0x80
)

// Interrupt vectors.
const (
	VectorNMI   uint16 = 0xFFFA
	VectorReset uint16 = 0xFFFC
	VectorIRQ   uint16 = 0xFFFE
)

// Packed internal flags byte.
const (
	NmiPending uint8 = 0x01
	IrqPending uint8 = 0x02
	Halted     uint8 = 0x04
)

// Cpu is the 6502 CPU. Field order matches the C struct.
type Cpu struct {
	A      uint8
	X      uint8
	Y      uint8
	SP     uint8
	PC     uint16
	Status uint8
	Flags  uint8
}

// Init constructs a CPU in the simplified power-on state.
func (c *Cpu) Init() {
	c.A = 0
	c.X = 0
	c.Y = 0
	c.SP = 0xFD
	c.PC = 0
	c.Status = FlagU | FlagI
	c.Flags = 0
}

// ---- Flag helpers ----

func (c *Cpu) setFlag(flag uint8, v bool) {
	if v {
		c.Status |= flag
	} else {
		c.Status &^= flag
	}
}

func (c *Cpu) Carry() bool            { return (c.Status & FlagC) != 0 }
func (c *Cpu) Zero() bool             { return (c.Status & FlagZ) != 0 }
func (c *Cpu) InterruptDisable() bool { return (c.Status & FlagI) != 0 }
func (c *Cpu) Decimal() bool          { return (c.Status & FlagD) != 0 }
func (c *Cpu) Overflow() bool         { return (c.Status & FlagV) != 0 }
func (c *Cpu) Negative() bool         { return (c.Status & FlagN) != 0 }

func (c *Cpu) SetCarry(v bool)            { c.setFlag(FlagC, v) }
func (c *Cpu) SetZero(v bool)             { c.setFlag(FlagZ, v) }
func (c *Cpu) SetInterruptDisable(v bool) { c.setFlag(FlagI, v) }
func (c *Cpu) SetDecimal(v bool)          { c.setFlag(FlagD, v) }
func (c *Cpu) SetOverflow(v bool)         { c.setFlag(FlagV, v) }
func (c *Cpu) SetNegative(v bool)         { c.setFlag(FlagN, v) }

func (c *Cpu) setNz(value uint8) {
	c.SetZero(value == 0)
	c.SetNegative((value & 0x80) != 0)
}

// ---- Fetch helpers ----

func (c *Cpu) fetchByte(b *bus.Bus) uint8 {
	v := b.Read(c.PC)
	c.PC++
	return v
}

func (c *Cpu) fetchWord(b *bus.Bus) uint16 {
	lo := uint16(c.fetchByte(b))
	hi := uint16(c.fetchByte(b))
	return lo | (hi << 8)
}

// ---- Stack access ----

func (c *Cpu) Push(b *bus.Bus, value uint8) {
	addr := 0x0100 | uint16(c.SP)
	b.Write(addr, value)
	c.SP--
}

func (c *Cpu) Pull(b *bus.Bus) uint8 {
	c.SP++
	addr := 0x0100 | uint16(c.SP)
	return b.Read(addr)
}

func (c *Cpu) pushPC(b *bus.Bus, pc uint16) {
	c.Push(b, byte(pc>>8))
	c.Push(b, byte(pc&0xFF))
}

func (c *Cpu) pullPC(b *bus.Bus) uint16 {
	lo := uint16(c.Pull(b))
	hi := uint16(c.Pull(b))
	return lo | (hi << 8)
}

func (c *Cpu) pushStatus(b *bus.Bus, withBreak bool) {
	p := c.Status | FlagU
	if withBreak {
		p |= FlagB
	} else {
		p &^= FlagB
	}
	c.Push(b, p)
}

func (c *Cpu) pullStatus(b *bus.Bus) {
	p := c.Pull(b)
	c.Status = (p &^ FlagB) | FlagU
}

// ---- Operand read/write helpers ----

func (c *Cpu) readOperand(b *bus.Bus, op Operand) uint8 {
	switch op.Tag {
	case OperandNone:
		return 0
	case OperandAcc:
		return c.A
	case OperandAddr:
		return b.Read(op.Addr)
	default:
		return 0
	}
}

func (c *Cpu) writeOperand(b *bus.Bus, op Operand, value uint8) {
	switch op.Tag {
	case OperandNone:
	case OperandAcc:
		c.A = value
	case OperandAddr:
		b.Write(op.Addr, value)
	}
}

// ---- Interrupt service ----

func (c *Cpu) serviceInterrupt(b *bus.Bus, vector uint16) {
	c.pushPC(b, c.PC)
	c.pushStatus(b, false)
	c.SetInterruptDisable(true)
	c.PC = c.readVector(b, vector)
}

// NMI services a non-maskable interrupt.
func (c *Cpu) NMI(b *bus.Bus) { c.serviceInterrupt(b, VectorNMI) }

// IRQ services a maskable interrupt (caller checks I flag).
func (c *Cpu) IRQ(b *bus.Bus) { c.serviceInterrupt(b, VectorIRQ) }

// Reset performs a 6502 RESET.
func (c *Cpu) Reset(b *bus.Bus) {
	c.SP = 0xFD
	c.SetInterruptDisable(true)
	c.Status |= FlagU
	c.PC = c.readVector(b, VectorReset)
	c.Flags &^= Halted
}

// readVector reads a little-endian 16-bit vector.
func (c *Cpu) readVector(b *bus.Bus, addr uint16) uint16 {
	lo := uint16(b.Read(addr))
	hi := uint16(b.Read(addr + 1))
	return lo | (hi << 8)
}

// Step fetches and executes one instruction; returns cycle count.
func (c *Cpu) Step(b *bus.Bus) uint8 {
	if (c.Flags & Halted) != 0 {
		return 1
	}
	if (c.Flags & NmiPending) != 0 {
		c.Flags &^= NmiPending
		c.NMI(b)
		return 7
	}
	if (c.Flags&IrqPending) != 0 && !c.InterruptDisable() {
		c.Flags &^= IrqPending
		c.IRQ(b)
		return 7
	}
	opcode := c.fetchByte(b)
	return c.Execute(b, opcode)
}

// ---- Halt / interrupt pending accessors ----

func (c *Cpu) IsHalted() bool { return (c.Flags & Halted) != 0 }
func (c *Cpu) SetHalted(v bool) {
	if v {
		c.Flags |= Halted
	} else {
		c.Flags &^= Halted
	}
}
func (c *Cpu) NmiPending() bool { return (c.Flags & NmiPending) != 0 }
func (c *Cpu) IrqPending() bool { return (c.Flags & IrqPending) != 0 }
func (c *Cpu) SetNmiPending(v bool) {
	if v {
		c.Flags |= NmiPending
	} else {
		c.Flags &^= NmiPending
	}
}
func (c *Cpu) SetIrqPending(v bool) {
	if v {
		c.Flags |= IrqPending
	} else {
		c.Flags &^= IrqPending
	}
}

// ---- Read helpers (opcodes.rs rd_*) ----

func (c *Cpu) rdImm(b *bus.Bus) uint8 { return c.fetchByte(b) }

func (c *Cpu) rdZp(b *bus.Bus) uint8 {
	op := c.Resolve(b, ModeZeroPage)
	return c.readOperand(b, op)
}

func (c *Cpu) rdZpX(b *bus.Bus) uint8 {
	a := c.amZeroPageX(b, DummyRMW)
	return b.Read(a)
}

func (c *Cpu) rdZpY(b *bus.Bus) uint8 {
	a := c.amZeroPageY(b, DummyRMW)
	return b.Read(a)
}

func (c *Cpu) rdAbs(b *bus.Bus) uint8 {
	op := c.Resolve(b, ModeAbsolute)
	return c.readOperand(b, op)
}

func (c *Cpu) rdAbsX(b *bus.Bus) (uint8, bool) {
	pc, a := c.amAbsoluteX(b, DummyRead)
	return b.Read(a), pc
}

func (c *Cpu) rdAbsY(b *bus.Bus) (uint8, bool) {
	pc, a := c.amAbsoluteY(b, DummyRead)
	return b.Read(a), pc
}

func (c *Cpu) rdIndX(b *bus.Bus) uint8 {
	a := c.amIndirectX(b)
	return b.Read(a)
}

func (c *Cpu) rdIndY(b *bus.Bus) (uint8, bool) {
	pc, a := c.amIndirectY(b, DummyRead)
	return b.Read(a), pc
}

// ---- Write helpers (opcodes.rs wr_*) ----

func (c *Cpu) wrZp(b *bus.Bus, v uint8) {
	op := c.Resolve(b, ModeZeroPage)
	c.writeOperand(b, op, v)
}
func (c *Cpu) wrZpX(b *bus.Bus, v uint8) {
	a := c.amZeroPageX(b, DummyRMW)
	b.Write(a, v)
}
func (c *Cpu) wrZpY(b *bus.Bus, v uint8) {
	a := c.amZeroPageY(b, DummyRMW)
	b.Write(a, v)
}
func (c *Cpu) wrAbs(b *bus.Bus, v uint8) {
	op := c.Resolve(b, ModeAbsolute)
	c.writeOperand(b, op, v)
}
func (c *Cpu) wrAbsX(b *bus.Bus, v uint8) {
	_, a := c.amAbsoluteX(b, DummyRMW)
	b.Write(a, v)
}
func (c *Cpu) wrAbsY(b *bus.Bus, v uint8) {
	_, a := c.amAbsoluteY(b, DummyRMW)
	b.Write(a, v)
}
func (c *Cpu) wrIndX(b *bus.Bus, v uint8) {
	a := c.amIndirectX(b)
	b.Write(a, v)
}
func (c *Cpu) wrIndY(b *bus.Bus, v uint8) {
	_, a := c.amIndirectY(b, DummyRMW)
	b.Write(a, v)
}

// ---- RMW helpers ----

// valueFn is a value transform: takes Cpu + value, returns new value.
type valueFn func(c *Cpu, v uint8) uint8

func (c *Cpu) rmw(b *bus.Bus, op Operand, f valueFn) {
	v := c.readOperand(b, op)
	neu := f(c, v)
	c.writeOperand(b, op, neu)
	c.setNz(neu)
}

func (c *Cpu) rmwZp(b *bus.Bus, f valueFn) {
	op := c.Resolve(b, ModeZeroPage)
	c.rmw(b, op, f)
}
func (c *Cpu) rmwZpX(b *bus.Bus, f valueFn) {
	a := c.amZeroPageX(b, DummyRMW)
	c.rmw(b, operandAddr(a), f)
}
func (c *Cpu) rmwAbs(b *bus.Bus, f valueFn) {
	op := c.Resolve(b, ModeAbsolute)
	c.rmw(b, op, f)
}
func (c *Cpu) rmwAbsX(b *bus.Bus, f valueFn) {
	_, a := c.amAbsoluteX(b, DummyRMW)
	c.rmw(b, operandAddr(a), f)
}

// ---- operand-resolving helpers for unofficial combo opcodes ----

func (c *Cpu) opZp(b *bus.Bus) Operand {
	return c.Resolve(b, ModeZeroPage)
}
func (c *Cpu) opZpX(b *bus.Bus) Operand {
	return operandAddr(c.amZeroPageX(b, DummyRMW))
}
func (c *Cpu) opAbs(b *bus.Bus) Operand {
	return c.Resolve(b, ModeAbsolute)
}
func (c *Cpu) opAbsX(b *bus.Bus) Operand {
	_, a := c.amAbsoluteX(b, DummyRMW)
	return operandAddr(a)
}
func (c *Cpu) opAbsY(b *bus.Bus) Operand {
	_, a := c.amAbsoluteY(b, DummyRMW)
	return operandAddr(a)
}
func (c *Cpu) opIndX(b *bus.Bus) Operand {
	return operandAddr(c.amIndirectX(b))
}
func (c *Cpu) opIndY(b *bus.Bus) Operand {
	_, a := c.amIndirectY(b, DummyRMW)
	return operandAddr(a)
}

// ---- load / store / transfer handlers ----

func (c *Cpu) lda(v uint8) { c.A = v; c.setNz(v) }
func (c *Cpu) ldx(v uint8) { c.X = v; c.setNz(v) }
func (c *Cpu) ldy(v uint8) { c.Y = v; c.setNz(v) }
func (c *Cpu) tax()        { c.X = c.A; c.setNz(c.X) }
func (c *Cpu) tay()        { c.Y = c.A; c.setNz(c.Y) }
func (c *Cpu) txa()        { c.A = c.X; c.setNz(c.A) }
func (c *Cpu) tya()        { c.A = c.Y; c.setNz(c.A) }
func (c *Cpu) tsx()        { c.X = c.SP; c.setNz(c.X) }
func (c *Cpu) setSpFromX() { c.SP = c.X } // TXS - no flags

// ---- logic handlers ----

func (c *Cpu) andOp(v uint8) { c.A &= v; c.setNz(c.A) }
func (c *Cpu) oraOp(v uint8) { c.A |= v; c.setNz(c.A) }
func (c *Cpu) eorOp(v uint8) { c.A ^= v; c.setNz(c.A) }

func (c *Cpu) bitOp(m uint8) {
	result := c.A & m
	c.SetZero(result == 0)
	c.SetNegative((m & 0x80) != 0)
	c.SetOverflow((m & 0x40) != 0)
}

// ---- arithmetic ----

func (c *Cpu) adc(m uint8) {
	a := uint16(c.A)
	mm := uint16(m)
	var cc uint16
	if c.Carry() {
		cc = 1
	}
	sum := a + mm + cc
	c.SetCarry(sum > 0xFF)
	result := uint8(sum & 0xFF)
	c.SetOverflow(((a^mm)&0x80) == 0 && ((a^sum)&0x80) != 0)
	c.A = result
	c.setNz(result)
}

func (c *Cpu) sbc(m uint8) {
	a := uint16(c.A)
	mm := uint16(^m)
	var cc uint16
	if c.Carry() {
		cc = 1
	}
	sum := a + mm + cc
	c.SetCarry(sum > 0xFF)
	result := uint8(sum & 0xFF)
	c.SetOverflow(((a^mm)&0x80) == 0 && ((a^sum)&0x80) != 0)
	c.A = result
	c.setNz(result)
}

// ---- compare ----

func (c *Cpu) cmpOp(r, m uint8) {
	diff := r - m
	c.SetCarry(r >= m)
	c.setNz(diff)
}

// ---- shifts / rotates / inc / dec value transforms ----

func aslValue(c *Cpu, v uint8) uint8 { c.SetCarry((v & 0x80) != 0); return v << 1 }
func lsrValue(c *Cpu, v uint8) uint8 { c.SetCarry((v & 0x01) != 0); return v >> 1 }
func rolValue(c *Cpu, v uint8) uint8 {
	newC := (v & 0x80) != 0
	var carryIn uint8
	if c.Carry() {
		carryIn = 1
	}
	result := (v << 1) | carryIn
	c.SetCarry(newC)
	return result
}
func rorValue(c *Cpu, v uint8) uint8 {
	newC := (v & 0x01) != 0
	var carryIn uint8
	if c.Carry() {
		carryIn = 0x80
	}
	result := (v >> 1) | carryIn
	c.SetCarry(newC)
	return result
}
func incValue(c *Cpu, v uint8) uint8 { return v + 1 }
func decValue(c *Cpu, v uint8) uint8 { return v - 1 }

// ---- branches ----

func (c *Cpu) branch(b *bus.Bus, branchOnSet bool, condFlag uint8) uint8 {
	flagSet := (c.Status & condFlag) != 0
	take := flagSet == branchOnSet
	if !take {
		_ = c.fetchByte(b)
		return 2
	}
	pcBefore := c.PC
	target := c.amRelative(b)
	pcAfter := pcBefore + 1
	pageCross := (pcAfter & 0xFF00) != (target & 0xFF00)
	c.PC = target
	if pageCross {
		return 4
	}
	return 3
}
