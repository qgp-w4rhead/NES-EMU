// unofficial.go - unofficial / illegal 6502 opcodes (port of cpu_unofficial.c).
package cpu

import "nes-core-go/internal/bus"

const xaaMagic uint8 = 0x00

// rmwCombo mirrors the rmw wrapper (read, transform, write, set N/Z).
func (c *Cpu) rmwCombo(b *bus.Bus, op Operand, f valueFn) {
	v := c.readOperand(b, op)
	neu := f(c, v)
	c.writeOperand(b, op, neu)
	c.setNz(neu)
}

// ---- Immediate combined ops ----

func (c *Cpu) anc(m uint8) {
	c.A &= m
	c.setNz(c.A)
	c.SetCarry((c.A & 0x80) != 0)
}

func (c *Cpu) alr(m uint8) {
	v := c.A & m
	c.SetCarry((v & 0x01) != 0)
	result := v >> 1
	c.A = result
	c.setNz(result)
}

func (c *Cpu) arr(m uint8) {
	v := c.A & m
	var carryIn uint8
	if c.Carry() {
		carryIn = 0x80
	}
	result := (v >> 1) | carryIn
	c.A = result
	c.setNz(result)
	c.SetCarry((result & 0x40) != 0)
	c.SetOverflow(((result ^ (result << 1)) & 0x40) != 0)
}

func (c *Cpu) axs(m uint8) {
	ax := c.A & c.X
	c.SetCarry(ax >= m)
	result := ax - m
	c.X = result
	c.setNz(result)
}

func (c *Cpu) xaa(m uint8) {
	result := ((c.A | xaaMagic) & c.X) & m
	c.A = result
	c.setNz(result)
}

func (c *Cpu) lax(m uint8) {
	c.A = m
	c.X = m
	c.setNz(m)
}

// ---- Unstable indexed stores ----

func storeDummyReadAddr(base uint16, index uint8) uint16 {
	return (base & 0xFF00) | ((base + uint16(index)) & 0x00FF)
}

func quirkAddrY(base uint16, y, value uint8) uint16 {
	eff := base + uint16(y)
	if (base & 0xFF00) != (eff & 0xFF00) {
		return (uint16(value) << 8) | (eff & 0x00FF)
	}
	return eff
}

func quirkAddrX(base uint16, x, value uint8) uint16 {
	eff := base + uint16(x)
	if (base & 0xFF00) != (eff & 0xFF00) {
		return (uint16(value) << 8) | (eff & 0x00FF)
	}
	return eff
}

func (c *Cpu) tasStore(b *bus.Bus, base uint16) {
	c.SP = c.A & c.X
	h := uint8(base >> 8)
	value := c.SP & (h + 1)
	storeAddr := quirkAddrY(base, c.Y, value)
	_ = b.Read(storeDummyReadAddr(base, c.Y))
	b.Write(storeAddr, value)
}

func (c *Cpu) ahxStore(b *bus.Bus, base uint16, reg uint8) {
	h := uint8(base >> 8)
	value := reg & (h + 1)
	storeAddr := quirkAddrY(base, c.Y, value)
	_ = b.Read(storeDummyReadAddr(base, c.Y))
	b.Write(storeAddr, value)
}

func (c *Cpu) shyStore(b *bus.Bus, base uint16) {
	h := uint8(base >> 8)
	value := c.Y & (h + 1)
	storeAddr := quirkAddrX(base, c.X, value)
	_ = b.Read(storeDummyReadAddr(base, c.X))
	b.Write(storeAddr, value)
}

// ---- RMW-combo helpers ----

func (c *Cpu) dcp(b *bus.Bus, op Operand) {
	c.rmwCombo(b, op, decValue)
	m := c.readOperand(b, op)
	c.cmpOp(c.A, m)
}

func (c *Cpu) isc(b *bus.Bus, op Operand) {
	c.rmwCombo(b, op, incValue)
	m := c.readOperand(b, op)
	c.sbc(m)
}

func (c *Cpu) slo(b *bus.Bus, op Operand) {
	c.rmwCombo(b, op, aslValue)
	m := c.readOperand(b, op)
	c.oraOp(m)
}

func (c *Cpu) rla(b *bus.Bus, op Operand) {
	c.rmwCombo(b, op, rolValue)
	m := c.readOperand(b, op)
	c.andOp(m)
}

func (c *Cpu) sre(b *bus.Bus, op Operand) {
	c.rmwCombo(b, op, lsrValue)
	m := c.readOperand(b, op)
	c.eorOp(m)
}

func (c *Cpu) rra(b *bus.Bus, op Operand) {
	c.rmwCombo(b, op, rorValue)
	m := c.readOperand(b, op)
	c.adc(m)
}

func (c *Cpu) executeUnofficialRMW(b *bus.Bus, opcode uint8) uint8 {
	switch opcode {
	// ---- DCP ----
	case 0xC7:
		c.dcp(b, c.opZp(b))
		return 5
	case 0xD7:
		c.dcp(b, c.opZpX(b))
		return 6
	case 0xCF:
		c.dcp(b, c.opAbs(b))
		return 6
	case 0xDF:
		c.dcp(b, c.opAbsX(b))
		return 7
	case 0xDB:
		c.dcp(b, c.opAbsY(b))
		return 7
	case 0xC3:
		c.dcp(b, c.opIndX(b))
		return 8
	case 0xD3:
		c.dcp(b, c.opIndY(b))
		return 8

	// ---- ISC ----
	case 0xE7:
		c.isc(b, c.opZp(b))
		return 5
	case 0xF7:
		c.isc(b, c.opZpX(b))
		return 6
	case 0xEF:
		c.isc(b, c.opAbs(b))
		return 6
	case 0xFF:
		c.isc(b, c.opAbsX(b))
		return 7
	case 0xFB:
		c.isc(b, c.opAbsY(b))
		return 7
	case 0xE3:
		c.isc(b, c.opIndX(b))
		return 8
	case 0xF3:
		c.isc(b, c.opIndY(b))
		return 8

	// ---- SLO ----
	case 0x07:
		c.slo(b, c.opZp(b))
		return 5
	case 0x17:
		c.slo(b, c.opZpX(b))
		return 6
	case 0x0F:
		c.slo(b, c.opAbs(b))
		return 6
	case 0x1F:
		c.slo(b, c.opAbsX(b))
		return 7
	case 0x1B:
		c.slo(b, c.opAbsY(b))
		return 7
	case 0x03:
		c.slo(b, c.opIndX(b))
		return 8
	case 0x13:
		c.slo(b, c.opIndY(b))
		return 8

	// ---- RLA ----
	case 0x27:
		c.rla(b, c.opZp(b))
		return 5
	case 0x37:
		c.rla(b, c.opZpX(b))
		return 6
	case 0x2F:
		c.rla(b, c.opAbs(b))
		return 6
	case 0x3F:
		c.rla(b, c.opAbsX(b))
		return 7
	case 0x3B:
		c.rla(b, c.opAbsY(b))
		return 7
	case 0x23:
		c.rla(b, c.opIndX(b))
		return 8
	case 0x33:
		c.rla(b, c.opIndY(b))
		return 8

	// ---- SRE ----
	case 0x47:
		c.sre(b, c.opZp(b))
		return 5
	case 0x57:
		c.sre(b, c.opZpX(b))
		return 6
	case 0x4F:
		c.sre(b, c.opAbs(b))
		return 6
	case 0x5F:
		c.sre(b, c.opAbsX(b))
		return 7
	case 0x5B:
		c.sre(b, c.opAbsY(b))
		return 7
	case 0x43:
		c.sre(b, c.opIndX(b))
		return 8
	case 0x53:
		c.sre(b, c.opIndY(b))
		return 8

	// ---- RRA ----
	case 0x67:
		c.rra(b, c.opZp(b))
		return 5
	case 0x77:
		c.rra(b, c.opZpX(b))
		return 6
	case 0x6F:
		c.rra(b, c.opAbs(b))
		return 6
	case 0x7F:
		c.rra(b, c.opAbsX(b))
		return 7
	case 0x7B:
		c.rra(b, c.opAbsY(b))
		return 7
	case 0x63:
		c.rra(b, c.opIndX(b))
		return 8
	case 0x73:
		c.rra(b, c.opIndY(b))
		return 8

	// ---- TAS / SHS ----
	case 0x9B:
		base := c.fetchWord(b)
		c.tasStore(b, base)
		return 5

	// ---- AHX / SHA ----
	case 0x9F:
		base := c.fetchWord(b)
		c.ahxStore(b, base, c.A&c.X)
		return 5
	case 0x93:
		base := c.amIndirectYBase(b)
		c.ahxStore(b, base, c.A&c.X)
		return 6

	// ---- SHX / SXA ----
	case 0x9E:
		base := c.fetchWord(b)
		c.ahxStore(b, base, c.X)
		return 5

	// ---- SHY / SYA ----
	case 0x9C:
		base := c.fetchWord(b)
		c.shyStore(b, base)
		return 5

	default:
		return 2
	}
}

func (c *Cpu) executeUnofficial(b *bus.Bus, opcode uint8) uint8 {
	switch opcode {
	// ---- Implied NOPs (2 cycles) ----
	case 0x1A, 0x3A, 0x5A, 0x7A, 0xDA, 0xFA:
		return 2

	// ---- Immediate NOPs (2 cycles) ----
	case 0x80, 0x82, 0x89, 0xC2, 0xE2:
		_ = c.fetchByte(b)
		return 2

	// ---- Zero-page NOPs (3 cycles) ----
	case 0x04, 0x44, 0x64:
		_ = c.fetchByte(b)
		return 3

	// ---- Zero-page,X NOPs (4 cycles) ----
	case 0x14, 0x34, 0x54, 0x74, 0xD4, 0xF4:
		_ = c.amZeroPageX(b, DummyRMW)
		return 4

	// ---- Absolute NOP (4 cycles) ----
	case 0x0C:
		_ = c.fetchWord(b)
		return 4

	// ---- Absolute,X NOPs (4 + page-cross) ----
	case 0x1C, 0x3C, 0x5C, 0x7C, 0xDC, 0xFC:
		pc, _ := c.amAbsoluteX(b, DummyRead)
		if pc {
			return 5
		}
		return 4

	// ---- LAX ----
	case 0xA7:
		c.lax(c.rdZp(b))
		return 3
	case 0xB7:
		c.lax(c.rdZpY(b))
		return 4
	case 0xAF:
		c.lax(c.rdAbs(b))
		return 4
	case 0xBF:
		v, pc := c.rdAbsY(b)
		c.lax(v)
		if pc {
			return 5
		}
		return 4
	case 0xA3:
		c.lax(c.rdIndX(b))
		return 6
	case 0xB3:
		v, pc := c.rdIndY(b)
		c.lax(v)
		if pc {
			return 6
		}
		return 5

	// ---- SAX ----
	case 0x87:
		c.wrZp(b, c.A&c.X)
		return 3
	case 0x97:
		c.wrZpY(b, c.A&c.X)
		return 4
	case 0x8F:
		c.wrAbs(b, c.A&c.X)
		return 4
	case 0x83:
		c.wrIndX(b, c.A&c.X)
		return 6

	// ---- ANC ----
	case 0x0B, 0x2B:
		c.anc(c.rdImm(b))
		return 2

	// ---- ALR ----
	case 0x4B:
		c.alr(c.rdImm(b))
		return 2

	// ---- ARR ----
	case 0x6B:
		c.arr(c.rdImm(b))
		return 2

	// ---- AXS / SBX ----
	case 0xCB:
		c.axs(c.rdImm(b))
		return 2

	// ---- XAA (unstable) ----
	case 0x8B:
		c.xaa(c.rdImm(b))
		return 2

	// ---- LAS / LAR ----
	case 0xBB:
		v, pc := c.rdAbsY(b)
		r := v & c.SP
		c.A = r
		c.X = r
		c.SP = r
		c.setNz(r)
		if pc {
			return 5
		}
		return 4

	// ---- KIL / JAM / HLT ----
	case 0x02, 0x12, 0x22, 0x32, 0x42, 0x52, 0x62, 0x72, 0x92, 0xB2, 0xD2, 0xF2:
		c.SetHalted(true)
		return 1

	default:
		return c.executeUnofficialRMW(b, opcode)
	}
}
