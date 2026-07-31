// mmc1.go - Mapper 1 (MMC1). Port of cores/c/src/mmc1.c.
package mapper

const (
	mmc1PrgBankSize = 16384
	mmc1Chr4K       = 4096
	mmc1Chr8K       = 8192
	mmc1PrgRAMSize  = 8192
	mmc1CtrlPrgMode3 uint8 = 0x0C
)

func (s *Mmc1State) prgBankCount() uint32 {
	n := s.PrgSize / mmc1PrgBankSize
	if n == 0 {
		return 1
	}
	return n
}

func (s *Mmc1State) chrBankCount() uint32 {
	n := s.ChrSize / mmc1Chr4K
	if n == 0 {
		return 1
	}
	return n
}

func (s *Mmc1State) prgMode() uint8 { return (s.Control >> 2) & 0x03 }
func (s *Mmc1State) chrMode() uint8 { return (s.Control >> 4) & 1 }

func (s *Mmc1State) prgReadBank(bank, offset uint32) uint8 {
	count := s.prgBankCount()
	bank %= count
	idx := bank*mmc1PrgBankSize + offset
	if idx >= s.PrgSize {
		return 0
	}
	return s.PrgRom[idx]
}

func (s *Mmc1State) serialWrite(addr uint16, value uint8) {
	if value&0x80 != 0 {
		s.ShiftReg = 0
		s.ShiftCount = 0
		s.Control = (s.Control & 0x13) | mmc1CtrlPrgMode3
		return
	}
	s.ShiftReg = (s.ShiftReg >> 1) | ((value & 1) << 4)
	s.ShiftCount++
	if s.ShiftCount == 5 {
		reg := uint8((addr >> 13) & 0x03)
		switch reg {
		case 0:
			s.Control = s.ShiftReg
		case 1:
			s.ChrBank0 = s.ShiftReg
		case 2:
			s.ChrBank1 = s.ShiftReg
		case 3:
			s.PrgBank = s.ShiftReg
		}
		s.ShiftReg = 0
		s.ShiftCount = 0
	}
}

func mmc1ReadPRG(m *Mapper, addr uint16) uint8 {
	s := &m.MMC1
	if addr >= 0x6000 && addr < 0x8000 {
		return s.PrgRAM[(addr-0x6000)&(mmc1PrgRAMSize-1)]
	}
	local := uint32(addr - 0x8000)
	inLow := local < mmc1PrgBankSize
	offset := local & (mmc1PrgBankSize - 1)
	switch s.prgMode() {
	case 0, 1:
		return s.prgReadBank(uint32(s.PrgBank&0x0E), local)
	case 2:
		if inLow {
			return s.prgReadBank(0, offset)
		}
		return s.prgReadBank(uint32(s.PrgBank&0x0F), offset)
	default:
		if inLow {
			return s.prgReadBank(uint32(s.PrgBank&0x0F), offset)
		}
		return s.prgReadBank(s.prgBankCount()-1, offset)
	}
}

func mmc1WritePRG(m *Mapper, addr uint16, value uint8) {
	s := &m.MMC1
	if addr >= 0x6000 && addr < 0x8000 {
		s.PrgRAM[(addr-0x6000)&(mmc1PrgRAMSize-1)] = value
		return
	}
	s.serialWrite(addr, value)
}

func mmc1ChrIndex(s *Mmc1State, addr uint16) uint32 {
	a := uint32(addr)
	count := s.chrBankCount()
	if s.chrMode() == 0 {
		bank := (uint32(s.ChrBank0 & 0x1E)) % count
		return bank*mmc1Chr4K + (a & (mmc1Chr8K - 1))
	} else if a < mmc1Chr4K {
		bank := (uint32(s.ChrBank0 & 0x1F)) % count
		return bank*mmc1Chr4K + a
	}
	bank := (uint32(s.ChrBank1 & 0x1F)) % count
	return bank*mmc1Chr4K + (a - mmc1Chr4K)
}

func mmc1ReadCHR(m *Mapper, addr uint16) uint8 {
	s := &m.MMC1
	idx := mmc1ChrIndex(s, addr)
	if idx >= s.ChrSize {
		return 0
	}
	return s.Chr[idx]
}

func mmc1WriteCHR(m *Mapper, addr uint16, value uint8) {
	s := &m.MMC1
	if !s.ChrIsRAM {
		return
	}
	idx := mmc1ChrIndex(s, addr)
	if idx < s.ChrSize {
		s.Chr[idx] = value
	}
}

func mmc1MirrorMode(m *Mapper) Mirroring {
	switch m.MMC1.Control & 0x03 {
	case 0:
		return MirrorSingleScreen0
	case 1:
		return MirrorSingleScreen1
	case 2:
		return MirrorVertical
	default:
		return MirrorHorizontal
	}
}

func Mmc1Create(m *Mapper, prg, chr []byte, mirroring Mirroring, hasBattery bool) int {
	s := &m.MMC1
	s.PrgSize = uint32(len(prg))
	if s.PrgSize > 0 {
		s.PrgRom = make([]byte, s.PrgSize)
		copy(s.PrgRom, prg)
	}
	if len(chr) == 0 {
		s.ChrSize = mmc1Chr8K
		s.Chr = make([]byte, mmc1Chr8K)
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
	var mirrBits uint8
	switch mirroring {
	case MirrorSingleScreen0:
		mirrBits = 0x00
	case MirrorSingleScreen1, MirrorSingleScreen2, MirrorSingleScreen3:
		mirrBits = 0x01
	case MirrorVertical:
		mirrBits = 0x02
	default:
		mirrBits = 0x03
	}
	s.Control = mmc1CtrlPrgMode3 | mirrBits
	return 0
}

func mmc1SaveState(m *Mapper, buf []byte) int {
	s := &m.MMC1
	total := mmc1StateSize(s)
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
	buf[off] = s.ShiftReg; off++
	buf[off] = s.ShiftCount; off++
	buf[off] = s.Control; off++
	buf[off] = s.ChrBank0; off++
	buf[off] = s.ChrBank1; off++
	buf[off] = s.PrgBank; off++
	copy(buf[off:], s.PrgRAM[:]); off += mmc1PrgRAMSize
	if s.ChrIsRAM && s.ChrSize > 0 {
		copy(buf[off:], s.Chr); off += int(s.ChrSize)
	}
	return off
}

func mmc1StateSize(s *Mmc1State) int {
	total := 4 + int(s.PrgSize) + 4 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + mmc1PrgRAMSize
	if s.ChrIsRAM {
		total += int(s.ChrSize)
	}
	return total
}

func mmc1LoadState(m *Mapper, buf []byte) bool {
	s := &m.MMC1
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
	if len(buf) < off+4+8+mmc1PrgRAMSize {
		return false
	}
	chrSize := getU32(buf[off:]); off += 4
	if chrSize != s.ChrSize {
		return false
	}
	s.ChrIsRAM = buf[off] != 0; off++
	s.HasBattery = buf[off] != 0; off++
	s.ShiftReg = buf[off]; off++
	s.ShiftCount = buf[off]; off++
	s.Control = buf[off]; off++
	s.ChrBank0 = buf[off]; off++
	s.ChrBank1 = buf[off]; off++
	s.PrgBank = buf[off]; off++
	copy(s.PrgRAM[:], buf[off:off+mmc1PrgRAMSize]); off += mmc1PrgRAMSize
	if s.ChrIsRAM && s.ChrSize > 0 {
		if len(buf) < off+int(s.ChrSize) {
			return false
		}
		copy(s.Chr, buf[off:off+int(s.ChrSize)]); off += int(s.ChrSize)
	}
	return true
}
