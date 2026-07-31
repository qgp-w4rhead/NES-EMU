// mmc5.go - Mapper 5 (MMC5). Port of cores/c/src/mmc5.c.
package mapper

const mmc5PrgBankSize = 8192

func Mmc5Create(m *Mapper, prg, chr []byte, mirroring Mirroring, hasBattery bool) int {
	s := &m.MMC5
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

func (s *Mmc5State) prgBankCount() uint32 {
	n := s.PrgSize / mmc5PrgBankSize
	if n == 0 {
		return 1
	}
	return n
}

func mmc5ReadPRG(m *Mapper, addr uint16) uint8 {
	s := &m.MMC5
	if addr >= 0x5C00 && addr < 0x6000 {
		// EXRAM (simplified)
		return 0
	}
	if addr >= 0x6000 && addr < 0x8000 {
		return s.PrgRAM[(addr-0x6000)&(mmc5PrgBankSize-1)]
	}
	if addr < 0x8000 {
		return 0
	}
	local := uint32(addr - 0x8000)
	bankSlot := local / mmc5PrgBankSize
	offset := local % mmc5PrgBankSize
	count := s.prgBankCount()
	switch bankSlot {
	case 0:
		return s.PrgRom[uint32(s.PrgBank1&0x7F)%count*mmc5PrgBankSize+offset]
	case 1:
		return s.PrgRom[uint32(s.PrgBank2&0x7F)%count*mmc5PrgBankSize+offset]
	case 2:
		return s.PrgRom[uint32(s.PrgBank3&0x7F)%count*mmc5PrgBankSize+offset]
	case 3:
		return s.PrgRom[(count-1)*mmc5PrgBankSize+offset]
	}
	return 0
}

func mmc5WritePRG(m *Mapper, addr uint16, value uint8) {
	s := &m.MMC5
	if addr >= 0x5C00 && addr < 0x6000 {
		return
	}
	if addr >= 0x6000 && addr < 0x8000 {
		s.PrgRAM[(addr-0x6000)&(mmc5PrgBankSize-1)] = value
		return
	}
	switch addr {
	case 0x5115:
		s.PrgMode = value & 0x03
	case 0x5120:
		s.ChrBank1 = value
	case 0x5121:
		s.ChrBank2 = value
	case 0x5122:
		s.ChrBank3 = value
	case 0x5123:
		s.ChrBank4 = value
	case 0x5124:
		s.ChrBank5 = value
	case 0x5125:
		s.ChrBank6 = value
	case 0x5126:
		s.ChrBank7 = value
	case 0x5127:
		s.ChrBank8 = value
	case 0x5200:
		// vertical split (ignored)
	case 0x5203:
		s.IrqScanline = value
	case 0x5204:
		s.IrqEnabled = (value & 0x80) != 0
	case 0x5100:
		s.PrgMode = value & 0x03
	case 0x5101:
		s.ChrMode = value & 0x03
	case 0x5205:
		s.MultA = value
		s.MultResult = uint16(s.MultA) * uint16(s.MultB)
	case 0x5206:
		s.MultB = value
		s.MultResult = uint16(s.MultA) * uint16(s.MultB)
	}
}

func mmc5ReadCHR(m *Mapper, addr uint16) uint8 {
	s := &m.MMC5
	if s.ChrSize == 0 {
		return 0
	}
	banks := [8]uint8{s.ChrBank1, s.ChrBank2, s.ChrBank3, s.ChrBank4, s.ChrBank5, s.ChrBank6, s.ChrBank7, s.ChrBank8}
	slot := (uint32(addr) >> 10) & 7
	offset := uint32(addr) & 0x03FF
	bank := uint32(banks[slot]) % (s.ChrSize / 1024)
	idx := bank*1024 + offset
	if idx >= s.ChrSize {
		return 0
	}
	return s.Chr[idx]
}

func mmc5WriteCHR(m *Mapper, addr uint16, value uint8) {
	s := &m.MMC5
	if !s.ChrIsRAM {
		return
	}
	if s.ChrSize == 0 {
		return
	}
	s.Chr[uint32(addr)%s.ChrSize] = value
}

func mmc5ClockIRQ(m *Mapper) {
	// MMC5 IRQ is scanline-based; simplified
}

func mmc5ResetScanlineCounter(m *Mapper) {
	s := &m.MMC5
	s.FrameCount = 0
	s.InFrame = true
}

func mmc5SaveState(m *Mapper, buf []byte) int {
	s := &m.MMC5
	total := 4 + int(s.PrgSize) + 4 + 1 + 1 + 1 + 1 + 1 + 3 + 8 + 1 + 1 + 1 + 2 + 2 + 1 + 1 + 1 + 1 + mmc5PrgBankSize
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
	buf[off] = s.PrgMode; off++
	buf[off] = s.ChrMode; off++
	buf[off] = s.PrgBank1; off++
	buf[off] = s.PrgBank2; off++
	buf[off] = s.PrgBank3; off++
	buf[off] = s.ChrBank1; off++
	buf[off] = s.ChrBank2; off++
	buf[off] = s.ChrBank3; off++
	buf[off] = s.ChrBank4; off++
	buf[off] = s.ChrBank5; off++
	buf[off] = s.ChrBank6; off++
	buf[off] = s.ChrBank7; off++
	buf[off] = s.ChrBank8; off++
	buf[off] = s.MultA; off++
	buf[off] = s.MultB; off++
	putU16(buf[off:], s.MultResult); off += 2
	buf[off] = s.FillTile; off++
	buf[off] = s.FillAttr; off++
	buf[off] = s.IrqScanline; off++
	buf[off] = boolByte(s.IrqEnabled); off++
	buf[off] = boolByte(s.IrqPending); off++
	putU16(buf[off:], s.FrameCount); off += 2
	buf[off] = boolByte(s.InFrame); off++
	copy(buf[off:], s.PrgRAM[:]); off += mmc5PrgBankSize
	if s.ChrIsRAM && s.ChrSize > 0 {
		copy(buf[off:], s.Chr); off += int(s.ChrSize)
	}
	return off
}

func mmc5LoadState(m *Mapper, buf []byte) bool {
	s := &m.MMC5
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
	if len(buf) < off+4 {
		return false
	}
	chrSize := getU32(buf[off:]); off += 4
	if chrSize != s.ChrSize {
		return false
	}
	s.ChrIsRAM = buf[off] != 0; off++
	s.HasBattery = buf[off] != 0; off++
	s.Mirroring = Mirroring(buf[off]); off++
	s.PrgMode = buf[off]; off++
	s.ChrMode = buf[off]; off++
	s.PrgBank1 = buf[off]; off++
	s.PrgBank2 = buf[off]; off++
	s.PrgBank3 = buf[off]; off++
	s.ChrBank1 = buf[off]; off++
	s.ChrBank2 = buf[off]; off++
	s.ChrBank3 = buf[off]; off++
	s.ChrBank4 = buf[off]; off++
	s.ChrBank5 = buf[off]; off++
	s.ChrBank6 = buf[off]; off++
	s.ChrBank7 = buf[off]; off++
	s.ChrBank8 = buf[off]; off++
	s.MultA = buf[off]; off++
	s.MultB = buf[off]; off++
	s.MultResult = getU16(buf[off:]); off += 2
	s.FillTile = buf[off]; off++
	s.FillAttr = buf[off]; off++
	s.IrqScanline = buf[off]; off++
	s.IrqEnabled = buf[off] != 0; off++
	s.IrqPending = buf[off] != 0; off++
	s.FrameCount = getU16(buf[off:]); off += 2
	s.InFrame = buf[off] != 0; off++
	if len(buf) < off+mmc5PrgBankSize {
		return false
	}
	copy(s.PrgRAM[:], buf[off:off+mmc5PrgBankSize]); off += mmc5PrgBankSize
	if s.ChrIsRAM && s.ChrSize > 0 {
		if len(buf) < off+int(s.ChrSize) {
			return false
		}
		copy(s.Chr, buf[off:off+int(s.ChrSize)]); off += int(s.ChrSize)
	}
	return true
}
