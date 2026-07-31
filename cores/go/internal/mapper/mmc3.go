// mmc3.go - Mapper 4 (MMC3). Port of cores/c/src/mmc3.c.
package mapper

const (
	mmc3PrgBankSize = 8192
	mmc3ChrBankSize = 1024
	mmc3PrgRAMSize  = 8192
)

func Mmc3Create(m *Mapper, prg, chr []byte, mirroring Mirroring, hasBattery bool) int {
	s := &m.MMC3
	s.PrgSize = uint32(len(prg))
	if s.PrgSize > 0 {
		s.PrgRom = make([]byte, s.PrgSize)
		copy(s.PrgRom, prg)
	}
	if len(chr) == 0 {
		s.ChrSize = 8192
		s.Chr = make([]byte, s.ChrSize)
		s.ChrIsRAM = true
	} else {
		s.ChrSize = uint32(len(chr))
		s.Chr = make([]byte, s.ChrSize)
		copy(s.Chr, chr)
		s.ChrIsRAM = false
	}
	for i := range s.PrgRAM {
		s.PrgRAM[i] = 0
	}
	s.HasBattery = hasBattery
	s.Mirroring = mirroring
	return 0
}

func (s *Mmc3State) prgBankCount() uint32 {
	n := s.PrgSize / mmc3PrgBankSize
	if n == 0 {
		return 1
	}
	return n
}

func (s *Mmc3State) chrBankCount() uint32 {
	n := s.ChrSize / mmc3ChrBankSize
	if n == 0 {
		return 1
	}
	return n
}

func (s *Mmc3State) prgReadBank(bank, offset uint32) uint8 {
	count := s.prgBankCount()
	bank %= count
	idx := bank*mmc3PrgBankSize + offset
	if idx >= s.PrgSize {
		return 0
	}
	return s.PrgRom[idx]
}

func mmc3WritePRG(m *Mapper, addr uint16, value uint8) {
	s := &m.MMC3
	if addr >= 0x6000 && addr < 0x8000 {
		s.PrgRAM[(addr-0x6000)&(mmc3PrgRAMSize-1)] = value
		return
	}
	if addr < 0x8000 {
		return
	}
	switch addr & 0xE001 {
	case 0x8000:
		s.BankSelect = value & 0x07
		s.PRGMode = (value >> 6) & 1
		s.CHRMode = (value >> 7) & 1
	case 0x8001:
		idx := s.BankSelect
		s.BankValues[idx] = value
	case 0xA000:
		if (value & 1) != 0 {
			s.Mirroring = MirrorVertical
		} else {
			s.Mirroring = MirrorHorizontal
		}
	case 0xA001:
		// PRG-RAM protect (ignored)
	case 0xC000:
		s.IrqLatch = value
	case 0xC001:
		s.IrqCounter = 0
		s.IrqReload = true
	case 0xE000:
		s.IrqEnabled = false
		s.IrqPending = false
	case 0xE001:
		s.IrqEnabled = true
	}
}

func mmc3ReadPRG(m *Mapper, addr uint16) uint8 {
	s := &m.MMC3
	if addr >= 0x6000 && addr < 0x8000 {
		return s.PrgRAM[(addr-0x6000)&(mmc3PrgRAMSize-1)]
	}
	if addr < 0x8000 {
		return 0
	}
	local := uint32(addr - 0x8000)
	bankSlot := local / mmc3PrgBankSize
	offset := local % mmc3PrgBankSize
	last := s.prgBankCount() - 1
	if s.PRGMode == 0 {
		// R6@8000, R7@A000, last-1@C000, last@E000
		switch bankSlot {
		case 0:
			return s.prgReadBank(uint32(s.BankValues[6]&0x3F), offset)
		case 1:
			return s.prgReadBank(uint32(s.BankValues[7]&0x3F), offset)
		case 2:
			return s.prgReadBank(last-1, offset)
		case 3:
			return s.prgReadBank(last, offset)
		}
	} else {
		// last-1@8000, R7@A000, R6@C000, last@E000
		switch bankSlot {
		case 0:
			return s.prgReadBank(last-1, offset)
		case 1:
			return s.prgReadBank(uint32(s.BankValues[7]&0x3F), offset)
		case 2:
			return s.prgReadBank(uint32(s.BankValues[6]&0x3F), offset)
		case 3:
			return s.prgReadBank(last, offset)
		}
	}
	return 0
}

func mmc3ChrBank(s *Mmc3State, slot uint32) uint32 {
	count := s.chrBankCount()
	var regIdx uint8
	if s.CHRMode == 0 {
		// R0,R1 @ 2KB each (0000,0800); R2..R5 @ 1KB each (1000..3C00)
		switch slot {
		case 0:
			regIdx = 0
			return (uint32(s.BankValues[0]) >> 1) % count
		case 1:
			regIdx = 0
			return (uint32(s.BankValues[0])>>1 + 1) % count
		case 2:
			regIdx = 1
			return (uint32(s.BankValues[1]) >> 1) % count
		case 3:
			regIdx = 1
			return (uint32(s.BankValues[1])>>1 + 1) % count
		case 4:
			regIdx = 2
		case 5:
			regIdx = 3
		case 6:
			regIdx = 4
		case 7:
			regIdx = 5
		}
	} else {
		// R2..R5 @ 1KB each (0000..0C00); R0,R1 @ 2KB each (1000,1800)
		switch slot {
		case 0:
			regIdx = 2
		case 1:
			regIdx = 3
		case 2:
			regIdx = 4
		case 3:
			regIdx = 5
		case 4:
			regIdx = 0
			return (uint32(s.BankValues[0]) >> 1) % count
		case 5:
			regIdx = 0
			return (uint32(s.BankValues[0])>>1 + 1) % count
		case 6:
			regIdx = 1
			return (uint32(s.BankValues[1]) >> 1) % count
		case 7:
			regIdx = 1
			return (uint32(s.BankValues[1])>>1 + 1) % count
		}
	}
	return uint32(s.BankValues[regIdx]) % count
}

func mmc3ReadCHR(m *Mapper, addr uint16) uint8 {
	s := &m.MMC3
	slot := uint32(addr) / mmc3ChrBankSize
	offset := uint32(addr) % mmc3ChrBankSize
	bank := mmc3ChrBank(s, slot)
	idx := bank*mmc3ChrBankSize + offset
	if idx >= s.ChrSize {
		return 0
	}
	return s.Chr[idx]
}

func mmc3WriteCHR(m *Mapper, addr uint16, value uint8) {
	s := &m.MMC3
	if !s.ChrIsRAM {
		return
	}
	slot := uint32(addr) / mmc3ChrBankSize
	offset := uint32(addr) % mmc3ChrBankSize
	bank := mmc3ChrBank(s, slot)
	idx := bank*mmc3ChrBankSize + offset
	if idx < s.ChrSize {
		s.Chr[idx] = value
	}
}

func mmc3ClockIRQ(m *Mapper) {
	s := &m.MMC3
	if s.IrqReload {
		s.IrqCounter = s.IrqLatch
		s.IrqReload = false
	} else if s.IrqCounter > 0 {
		s.IrqCounter--
	}
	if s.IrqCounter == 0 && s.IrqEnabled {
		s.IrqPending = true
	}
}

func mmc3SaveState(m *Mapper, buf []byte) int {
	s := &m.MMC3
	total := 4 + int(s.PrgSize) + 4 + 1 + 1 + 1 + 1 + 8 + 1 + 1 + 1 + 1 + 1 + 1 + mmc3PrgRAMSize
	if s.ChrIsRAM {
		total += int(s.ChrSize)
	}
	if buf == nil {
		return total
	}
	off := 0
	putU32(buf[off:], s.PrgSize); off += 4
	if s.PrgSize > 0 {
		copy(buf[off:], s.PrgRom); off += int(s.PrgSize)
	}
	putU32(buf[off:], s.ChrSize); off += 4
	buf[off] = boolByte(s.ChrIsRAM); off++
	buf[off] = boolByte(s.HasBattery); off++
	buf[off] = uint8(s.Mirroring); off++
	buf[off] = s.BankSelect; off++
	copy(buf[off:], s.BankValues[:]); off += 8
	buf[off] = s.PRGMode; off++
	buf[off] = s.CHRMode; off++
	buf[off] = s.IrqLatch; off++
	buf[off] = s.IrqCounter; off++
	buf[off] = boolByte(s.IrqReload); off++
	buf[off] = boolByte(s.IrqEnabled); off++
	buf[off] = boolByte(s.IrqPending); off++
	copy(buf[off:], s.PrgRAM[:]); off += mmc3PrgRAMSize
	if s.ChrIsRAM && s.ChrSize > 0 {
		copy(buf[off:], s.Chr); off += int(s.ChrSize)
	}
	return off
}

func mmc3LoadState(m *Mapper, buf []byte) bool {
	s := &m.MMC3
	if len(buf) < 4 {
		return false
	}
	off := 0
	prgSize := getU32(buf[off:]); off += 4
	if prgSize != s.PrgSize {
		return false
	}
	if s.PrgSize > 0 {
		copy(s.PrgRom, buf[off:off+int(s.PrgSize)]); off += int(s.PrgSize)
	}
	need := off + 4 + 1 + 1 + 1 + 1 + 8 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + mmc3PrgRAMSize
	if len(buf) < need {
		return false
	}
	chrSize := getU32(buf[off:]); off += 4
	if chrSize != s.ChrSize {
		return false
	}
	s.ChrIsRAM = buf[off] != 0; off++
	s.HasBattery = buf[off] != 0; off++
	s.Mirroring = Mirroring(buf[off]); off++
	s.BankSelect = buf[off]; off++
	copy(s.BankValues[:], buf[off:off+8]); off += 8
	s.PRGMode = buf[off]; off++
	s.CHRMode = buf[off]; off++
	s.IrqLatch = buf[off]; off++
	s.IrqCounter = buf[off]; off++
	s.IrqReload = buf[off] != 0; off++
	s.IrqEnabled = buf[off] != 0; off++
	s.IrqPending = buf[off] != 0; off++
	copy(s.PrgRAM[:], buf[off:off+mmc3PrgRAMSize]); off += mmc3PrgRAMSize
	if s.ChrIsRAM && s.ChrSize > 0 {
		if len(buf) < off+int(s.ChrSize) {
			return false
		}
		copy(s.Chr, buf[off:off+int(s.ChrSize)]); off += int(s.ChrSize)
	}
	return true
}
