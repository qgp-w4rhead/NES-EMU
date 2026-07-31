// mmc2.go - Mapper 9 (MMC2). Port of cores/c/src/mmc2.c.
package mapper

const mmc2PrgBankSize = 8192

func Mmc2Create(m *Mapper, prg, chr []byte, mirroring Mirroring, hasBattery bool) int {
	s := &m.MMC2
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
	s.Mirroring = mirroring
	s.HasBattery = hasBattery
	s.LatchFD = 0xFE
	s.LatchFE = 0xFE
	return 0
}

func (s *Mmc2State) prgBankCount() uint32 {
	n := s.PrgSize / mmc2PrgBankSize
	if n == 0 {
		return 1
	}
	return n
}

func mmc2ReadPRG(m *Mapper, addr uint16) uint8 {
	s := &m.MMC2
	if addr < 0x8000 {
		return 0
	}
	local := uint32(addr - 0x8000)
	bankSlot := local / mmc2PrgBankSize
	offset := local % mmc2PrgBankSize
	last := s.prgBankCount() - 1
	switch bankSlot {
	case 0:
		return s.PrgRom[uint32(s.PrgBank&0x0F)*mmc2PrgBankSize+offset]
	case 1:
		return s.PrgRom[last*mmc2PrgBankSize+offset]
	case 2:
		return s.PrgRom[(last-1)*mmc2PrgBankSize+offset]
	case 3:
		return s.PrgRom[last*mmc2PrgBankSize+offset]
	}
	return 0
}

func mmc2WritePRG(m *Mapper, addr uint16, value uint8) {
	s := &m.MMC2
	if addr >= 0x8000 {
		s.PrgBank = value & 0x0F
	}
}

func mmc2ChrBankFor(s *Mmc2State, addr uint16) uint8 {
	if addr < 0x1000 {
		if s.LatchFD == 0xFD {
			return s.ChrBank0 & 0x1F
		}
		return s.ChrBank1 & 0x1F
	}
	if s.LatchFE == 0xFD {
		return s.ChrBank2 & 0x1F
	}
	return s.ChrBank3 & 0x1F
}

func mmc2ReadCHR(m *Mapper, addr uint16) uint8 {
	s := &m.MMC2
	bank := uint32(mmc2ChrBankFor(s, addr))
	idx := bank*4096 + (uint32(addr) & 0x0FFF)
	if idx >= s.ChrSize {
		return 0
	}
	return s.Chr[idx]
}

func mmc2ReadCHRLatched(m *Mapper, addr uint16) uint8 {
	v := mmc2ReadCHR(m, addr)
	s := &m.MMC2
	// Latch logic: tile $FD/$FE in pattern table 0/1
	if addr == 0x0FD8 || addr == 0x1FD8 {
		s.LatchFD = 0xFD
	} else if addr == 0x0FE8 || addr == 0x1FE8 {
		s.LatchFD = 0xFE
	} else if addr >= 0x2FD8 && addr <= 0x2FEF {
		s.LatchFE = 0xFD
	} else if addr >= 0x2FE8 && addr <= 0x2FFF {
		s.LatchFE = 0xFE
	}
	return v
}

func mmc2WriteCHR(m *Mapper, addr uint16, value uint8) {
	s := &m.MMC2
	if !s.ChrIsRAM {
		return
	}
	if s.ChrSize == 0 {
		return
	}
	s.Chr[uint32(addr)%s.ChrSize] = value
}

func mmc2SaveState(m *Mapper, buf []byte) int {
	s := &m.MMC2
	total := 4 + int(s.PrgSize) + 4 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 2 + int(s.ChrSize)
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
	buf[off] = uint8(s.Mirroring); off++
	buf[off] = boolByte(s.HasBattery); off++
	buf[off] = s.PrgBank; off++
	buf[off] = s.ChrBank0; off++
	buf[off] = s.ChrBank1; off++
	buf[off] = s.ChrBank2; off++
	buf[off] = s.ChrBank3; off++
	buf[off] = s.LatchFD; off++
	buf[off] = s.LatchFE; off++
	copy(buf[off:], s.Chr); off += int(s.ChrSize)
	return off
}

func mmc2LoadState(m *Mapper, buf []byte) bool {
	s := &m.MMC2
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
	if len(buf) < off+4+10 {
		return false
	}
	chrSize := getU32(buf[off:]); off += 4
	if chrSize != s.ChrSize {
		return false
	}
	s.ChrIsRAM = buf[off] != 0; off++
	s.Mirroring = Mirroring(buf[off]); off++
	s.HasBattery = buf[off] != 0; off++
	s.PrgBank = buf[off]; off++
	s.ChrBank0 = buf[off]; off++
	s.ChrBank1 = buf[off]; off++
	s.ChrBank2 = buf[off]; off++
	s.ChrBank3 = buf[off]; off++
	s.LatchFD = buf[off]; off++
	s.LatchFE = buf[off]; off++
	if len(buf) < off+int(s.ChrSize) {
		return false
	}
	copy(s.Chr, buf[off:off+int(s.ChrSize)]); off += int(s.ChrSize)
	return true
}
