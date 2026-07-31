// fme7.go - Mapper 69 (FME-7 / Sunsoft 5B). Port of cores/c/src/fme7.c.
package mapper

const fme7PrgBankSize = 8192

func Fme7Create(m *Mapper, prg, chr []byte, mirroring Mirroring, hasBattery bool) int {
	s := &m.FME7
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
	return 0
}

func (s *Fme7State) prgBankCount() uint32 {
	n := s.PrgSize / fme7PrgBankSize
	if n == 0 {
		return 1
	}
	return n
}

func fme7ReadPRG(m *Mapper, addr uint16) uint8 {
	s := &m.FME7
	if addr < 0x8000 {
		return 0
	}
	local := uint32(addr - 0x8000)
	bankSlot := local / fme7PrgBankSize
	offset := local % fme7PrgBankSize
	count := s.prgBankCount()
	switch bankSlot {
	case 0:
		return s.PrgRom[uint32(s.PrgBank[0]&0x0F)%count*fme7PrgBankSize+offset]
	case 1:
		return s.PrgRom[uint32(s.PrgBank[1]&0x0F)%count*fme7PrgBankSize+offset]
	case 2:
		return s.PrgRom[uint32(s.PrgBank[2]&0x0F)%count*fme7PrgBankSize+offset]
	case 3:
		return s.PrgRom[(count-1)*fme7PrgBankSize+offset]
	}
	return 0
}

func fme7WritePRG(m *Mapper, addr uint16, value uint8) {
	s := &m.FME7
	switch addr {
	case 0x8000:
		s.Command = value & 0x0F
	case 0xA000:
		cmd := s.Command
		if cmd < 4 {
			s.PrgBank[cmd] = value & 0x0F
		} else if cmd < 12 {
			s.ChrBank[cmd-4] = value
		} else if cmd == 0x0B {
			if (value & 1) != 0 {
				s.Mirroring = MirrorVertical
			} else {
				s.Mirroring = MirrorHorizontal
			}
		}
	}
}

func fme7MirrorMode(m *Mapper) Mirroring { return m.FME7.Mirroring }

func fme7ReadCHR(m *Mapper, addr uint16) uint8 {
	s := &m.FME7
	if s.ChrSize == 0 {
		return 0
	}
	bank := uint32(s.ChrBank[(addr>>10)&7]) % (s.ChrSize / 1024)
	idx := bank*1024 + (uint32(addr) & 0x03FF)
	if idx >= s.ChrSize {
		return 0
	}
	return s.Chr[idx]
}

func fme7WriteCHR(m *Mapper, addr uint16, value uint8) {
	s := &m.FME7
	if !s.ChrIsRAM {
		return
	}
	if s.ChrSize == 0 {
		return
	}
	s.Chr[uint32(addr)%s.ChrSize] = value
}

func fme7ClockCPU(m *Mapper, cpuCycles uint32) {}
func fme7ExpansionAudio(m *Mapper) float32    { return 0 }

func fme7SaveState(m *Mapper, buf []byte) int {
	s := &m.FME7
	total := 4 + int(s.PrgSize) + 4 + 1 + 1 + 1 + 1 + 4 + 8 + int(s.ChrSize)
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
	buf[off] = s.Command; off++
	copy(buf[off:], s.PrgBank[:]); off += 4
	copy(buf[off:], s.ChrBank[:]); off += 8
	copy(buf[off:], s.Chr); off += int(s.ChrSize)
	return off
}

func fme7LoadState(m *Mapper, buf []byte) bool {
	s := &m.FME7
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
	if len(buf) < off+4+4+8 {
		return false
	}
	chrSize := getU32(buf[off:]); off += 4
	if chrSize != s.ChrSize {
		return false
	}
	s.ChrIsRAM = buf[off] != 0; off++
	s.Mirroring = Mirroring(buf[off]); off++
	s.HasBattery = buf[off] != 0; off++
	s.Command = buf[off]; off++
	copy(s.PrgBank[:], buf[off:off+4]); off += 4
	copy(s.ChrBank[:], buf[off:off+8]); off += 8
	if len(buf) < off+int(s.ChrSize) {
		return false
	}
	copy(s.Chr, buf[off:off+int(s.ChrSize)]); off += int(s.ChrSize)
	return true
}
