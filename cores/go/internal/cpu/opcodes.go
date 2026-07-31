// opcodes.go - official 6502 opcode dispatch (port of cores/c/src/cpu.c execute).
package cpu

import "nes-core-go/internal/bus"

// Execute decodes and executes a single opcode (already fetched; PC advanced).
func (c *Cpu) Execute(b *bus.Bus, opcode uint8) uint8 {
	switch opcode {
	// ---- LDA ----
	case 0xA9:
		c.lda(c.rdImm(b))
		return 2
	case 0xA5:
		c.lda(c.rdZp(b))
		return 3
	case 0xB5:
		c.lda(c.rdZpX(b))
		return 4
	case 0xAD:
		c.lda(c.rdAbs(b))
		return 4
	case 0xBD:
		v, pc := c.rdAbsX(b)
		c.lda(v)
		if pc {
			return 5
		}
		return 4
	case 0xB9:
		v, pc := c.rdAbsY(b)
		c.lda(v)
		if pc {
			return 5
		}
		return 4
	case 0xA1:
		c.lda(c.rdIndX(b))
		return 6
	case 0xB1:
		v, pc := c.rdIndY(b)
		c.lda(v)
		if pc {
			return 6
		}
		return 5

	// ---- LDX ----
	case 0xA2:
		c.ldx(c.rdImm(b))
		return 2
	case 0xA6:
		c.ldx(c.rdZp(b))
		return 3
	case 0xB6:
		c.ldx(c.rdZpY(b))
		return 4
	case 0xAE:
		c.ldx(c.rdAbs(b))
		return 4
	case 0xBE:
		v, pc := c.rdAbsY(b)
		c.ldx(v)
		if pc {
			return 5
		}
		return 4

	// ---- LDY ----
	case 0xA0:
		c.ldy(c.rdImm(b))
		return 2
	case 0xA4:
		c.ldy(c.rdZp(b))
		return 3
	case 0xB4:
		c.ldy(c.rdZpX(b))
		return 4
	case 0xAC:
		c.ldy(c.rdAbs(b))
		return 4
	case 0xBC:
		v, pc := c.rdAbsX(b)
		c.ldy(v)
		if pc {
			return 5
		}
		return 4

	// ---- STA ----
	case 0x85:
		c.wrZp(b, c.A)
		return 3
	case 0x95:
		c.wrZpX(b, c.A)
		return 4
	case 0x8D:
		c.wrAbs(b, c.A)
		return 4
	case 0x9D:
		c.wrAbsX(b, c.A)
		return 5
	case 0x99:
		c.wrAbsY(b, c.A)
		return 5
	case 0x81:
		c.wrIndX(b, c.A)
		return 6
	case 0x91:
		c.wrIndY(b, c.A)
		return 6

	// ---- STX ----
	case 0x86:
		c.wrZp(b, c.X)
		return 3
	case 0x96:
		c.wrZpY(b, c.X)
		return 4
	case 0x8E:
		c.wrAbs(b, c.X)
		return 4

	// ---- STY ----
	case 0x84:
		c.wrZp(b, c.Y)
		return 3
	case 0x94:
		c.wrZpX(b, c.Y)
		return 4
	case 0x8C:
		c.wrAbs(b, c.Y)
		return 4

	// ---- register transfers ----
	case 0xAA:
		c.tax()
		return 2
	case 0xA8:
		c.tay()
		return 2
	case 0x8A:
		c.txa()
		return 2
	case 0x98:
		c.tya()
		return 2
	case 0xBA:
		c.tsx()
		return 2
	case 0x9A:
		c.setSpFromX()
		return 2

	// ---- stack ----
	case 0x48:
		c.Push(b, c.A)
		return 3
	case 0x08:
		c.pushStatus(b, true)
		return 3
	case 0x68:
		v := c.Pull(b)
		c.lda(v)
		return 4
	case 0x28:
		c.pullStatus(b)
		return 4

	// ---- AND ----
	case 0x29:
		c.andOp(c.rdImm(b))
		return 2
	case 0x25:
		c.andOp(c.rdZp(b))
		return 3
	case 0x35:
		c.andOp(c.rdZpX(b))
		return 4
	case 0x2D:
		c.andOp(c.rdAbs(b))
		return 4
	case 0x3D:
		v, pc := c.rdAbsX(b)
		c.andOp(v)
		if pc {
			return 5
		}
		return 4
	case 0x39:
		v, pc := c.rdAbsY(b)
		c.andOp(v)
		if pc {
			return 5
		}
		return 4
	case 0x21:
		c.andOp(c.rdIndX(b))
		return 6
	case 0x31:
		v, pc := c.rdIndY(b)
		c.andOp(v)
		if pc {
			return 6
		}
		return 5

	// ---- ORA ----
	case 0x09:
		c.oraOp(c.rdImm(b))
		return 2
	case 0x05:
		c.oraOp(c.rdZp(b))
		return 3
	case 0x15:
		c.oraOp(c.rdZpX(b))
		return 4
	case 0x0D:
		c.oraOp(c.rdAbs(b))
		return 4
	case 0x1D:
		v, pc := c.rdAbsX(b)
		c.oraOp(v)
		if pc {
			return 5
		}
		return 4
	case 0x19:
		v, pc := c.rdAbsY(b)
		c.oraOp(v)
		if pc {
			return 5
		}
		return 4
	case 0x01:
		c.oraOp(c.rdIndX(b))
		return 6
	case 0x11:
		v, pc := c.rdIndY(b)
		c.oraOp(v)
		if pc {
			return 6
		}
		return 5

	// ---- EOR ----
	case 0x49:
		c.eorOp(c.rdImm(b))
		return 2
	case 0x45:
		c.eorOp(c.rdZp(b))
		return 3
	case 0x55:
		c.eorOp(c.rdZpX(b))
		return 4
	case 0x4D:
		c.eorOp(c.rdAbs(b))
		return 4
	case 0x5D:
		v, pc := c.rdAbsX(b)
		c.eorOp(v)
		if pc {
			return 5
		}
		return 4
	case 0x59:
		v, pc := c.rdAbsY(b)
		c.eorOp(v)
		if pc {
			return 5
		}
		return 4
	case 0x41:
		c.eorOp(c.rdIndX(b))
		return 6
	case 0x51:
		v, pc := c.rdIndY(b)
		c.eorOp(v)
		if pc {
			return 6
		}
		return 5

	// ---- BIT ----
	case 0x24:
		c.bitOp(c.rdZp(b))
		return 3
	case 0x2C:
		c.bitOp(c.rdAbs(b))
		return 4

	// ---- ADC ----
	case 0x69:
		c.adc(c.rdImm(b))
		return 2
	case 0x65:
		c.adc(c.rdZp(b))
		return 3
	case 0x75:
		c.adc(c.rdZpX(b))
		return 4
	case 0x6D:
		c.adc(c.rdAbs(b))
		return 4
	case 0x7D:
		v, pc := c.rdAbsX(b)
		c.adc(v)
		if pc {
			return 5
		}
		return 4
	case 0x79:
		v, pc := c.rdAbsY(b)
		c.adc(v)
		if pc {
			return 5
		}
		return 4
	case 0x61:
		c.adc(c.rdIndX(b))
		return 6
	case 0x71:
		v, pc := c.rdIndY(b)
		c.adc(v)
		if pc {
			return 6
		}
		return 5

	// ---- SBC ----
	case 0xE9:
		c.sbc(c.rdImm(b))
		return 2
	case 0xE5:
		c.sbc(c.rdZp(b))
		return 3
	case 0xF5:
		c.sbc(c.rdZpX(b))
		return 4
	case 0xED:
		c.sbc(c.rdAbs(b))
		return 4
	case 0xFD:
		v, pc := c.rdAbsX(b)
		c.sbc(v)
		if pc {
			return 5
		}
		return 4
	case 0xF9:
		v, pc := c.rdAbsY(b)
		c.sbc(v)
		if pc {
			return 5
		}
		return 4
	case 0xE1:
		c.sbc(c.rdIndX(b))
		return 6
	case 0xF1:
		v, pc := c.rdIndY(b)
		c.sbc(v)
		if pc {
			return 6
		}
		return 5

	// ---- CMP ----
	case 0xC9:
		c.cmpOp(c.A, c.rdImm(b))
		return 2
	case 0xC5:
		c.cmpOp(c.A, c.rdZp(b))
		return 3
	case 0xD5:
		c.cmpOp(c.A, c.rdZpX(b))
		return 4
	case 0xCD:
		c.cmpOp(c.A, c.rdAbs(b))
		return 4
	case 0xDD:
		v, pc := c.rdAbsX(b)
		c.cmpOp(c.A, v)
		if pc {
			return 5
		}
		return 4
	case 0xD9:
		v, pc := c.rdAbsY(b)
		c.cmpOp(c.A, v)
		if pc {
			return 5
		}
		return 4
	case 0xC1:
		c.cmpOp(c.A, c.rdIndX(b))
		return 6
	case 0xD1:
		v, pc := c.rdIndY(b)
		c.cmpOp(c.A, v)
		if pc {
			return 6
		}
		return 5

	// ---- CPX / CPY ----
	case 0xE0:
		c.cmpOp(c.X, c.rdImm(b))
		return 2
	case 0xE4:
		c.cmpOp(c.X, c.rdZp(b))
		return 3
	case 0xEC:
		c.cmpOp(c.X, c.rdAbs(b))
		return 4
	case 0xC0:
		c.cmpOp(c.Y, c.rdImm(b))
		return 2
	case 0xC4:
		c.cmpOp(c.Y, c.rdZp(b))
		return 3
	case 0xCC:
		c.cmpOp(c.Y, c.rdAbs(b))
		return 4

	// ---- INC / DEC memory ----
	case 0xE6:
		c.rmwZp(b, incValue)
		return 5
	case 0xF6:
		c.rmwZpX(b, incValue)
		return 6
	case 0xEE:
		c.rmwAbs(b, incValue)
		return 6
	case 0xFE:
		c.rmwAbsX(b, incValue)
		return 7
	case 0xC6:
		c.rmwZp(b, decValue)
		return 5
	case 0xD6:
		c.rmwZpX(b, decValue)
		return 6
	case 0xCE:
		c.rmwAbs(b, decValue)
		return 6
	case 0xDE:
		c.rmwAbsX(b, decValue)
		return 7

	// ---- INX/INY/DEX/DEY ----
	case 0xE8:
		c.X++
		c.setNz(c.X)
		return 2
	case 0xC8:
		c.Y++
		c.setNz(c.Y)
		return 2
	case 0xCA:
		c.X--
		c.setNz(c.X)
		return 2
	case 0x88:
		c.Y--
		c.setNz(c.Y)
		return 2

	// ---- ASL ----
	case 0x0A:
		c.A = aslValue(c, c.A)
		c.setNz(c.A)
		return 2
	case 0x06:
		c.rmwZp(b, aslValue)
		return 5
	case 0x16:
		c.rmwZpX(b, aslValue)
		return 6
	case 0x0E:
		c.rmwAbs(b, aslValue)
		return 6
	case 0x1E:
		c.rmwAbsX(b, aslValue)
		return 7

	// ---- LSR ----
	case 0x4A:
		c.A = lsrValue(c, c.A)
		c.setNz(c.A)
		return 2
	case 0x46:
		c.rmwZp(b, lsrValue)
		return 5
	case 0x56:
		c.rmwZpX(b, lsrValue)
		return 6
	case 0x4E:
		c.rmwAbs(b, lsrValue)
		return 6
	case 0x5E:
		c.rmwAbsX(b, lsrValue)
		return 7

	// ---- ROL ----
	case 0x2A:
		c.A = rolValue(c, c.A)
		c.setNz(c.A)
		return 2
	case 0x26:
		c.rmwZp(b, rolValue)
		return 5
	case 0x36:
		c.rmwZpX(b, rolValue)
		return 6
	case 0x2E:
		c.rmwAbs(b, rolValue)
		return 6
	case 0x3E:
		c.rmwAbsX(b, rolValue)
		return 7

	// ---- ROR ----
	case 0x6A:
		c.A = rorValue(c, c.A)
		c.setNz(c.A)
		return 2
	case 0x66:
		c.rmwZp(b, rorValue)
		return 5
	case 0x76:
		c.rmwZpX(b, rorValue)
		return 6
	case 0x6E:
		c.rmwAbs(b, rorValue)
		return 6
	case 0x7E:
		c.rmwAbsX(b, rorValue)
		return 7

	// ---- branches ----
	case 0x10:
		return c.branch(b, false, FlagN)
	case 0x30:
		return c.branch(b, true, FlagN)
	case 0x50:
		return c.branch(b, false, FlagV)
	case 0x70:
		return c.branch(b, true, FlagV)
	case 0x90:
		return c.branch(b, false, FlagC)
	case 0xB0:
		return c.branch(b, true, FlagC)
	case 0xD0:
		return c.branch(b, false, FlagZ)
	case 0xF0:
		return c.branch(b, true, FlagZ)

	// ---- JMP / JSR / RTS / RTI / BRK ----
	case 0x4C:
		c.PC = c.amAbsolute(b)
		return 3
	case 0x6C:
		c.PC = c.amIndirect(b)
		return 5
	case 0x20:
		target := c.fetchWord(b)
		c.pushPC(b, c.PC-1)
		c.PC = target
		return 6
	case 0x60:
		c.PC = c.pullPC(b) + 1
		return 6
	case 0x40:
		c.pullStatus(b)
		c.PC = c.pullPC(b)
		return 6
	case 0x00:
		_ = c.fetchByte(b)
		c.pushPC(b, c.PC)
		c.pushStatus(b, true)
		c.SetInterruptDisable(true)
		c.PC = c.readVector(b, VectorIRQ)
		return 7

	// ---- flag operations ----
	case 0x18:
		c.SetCarry(false)
		return 2
	case 0x38:
		c.SetCarry(true)
		return 2
	case 0x58:
		c.SetInterruptDisable(false)
		return 2
	case 0x78:
		c.SetInterruptDisable(true)
		return 2
	case 0xB8:
		c.SetOverflow(false)
		return 2
	case 0xD8:
		c.SetDecimal(false)
		return 2
	case 0xF8:
		c.SetDecimal(true)
		return 2

	// ---- NOP ----
	case 0xEA:
		return 2

	default:
		return c.executeUnofficial(b, opcode)
	}
}
