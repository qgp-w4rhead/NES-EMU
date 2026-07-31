// vrc6.go - Mapper 24/26 (VRC6). Port of cores/c/src/vrc6.c.
package mapper

const vrc6PrgBankSize = 8192

func Vrc6Create(m *Mapper, prg, chr []byte, mirroring Mirroring, hasBattery bool, is26 bool) int {
	s := &m.VRC6
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
	s.Is26 = is26
	return 0
}

func (s *Vrc6State) prgBankCount() uint32 {
	n := s.PrgSize / vrc6PrgBankSize
	if n == 0 {
		return 1
	}
	return n
}

func vrc6ReadPRG(m *Mapper, addr uint16) uint8 {
	s := &m.VRC6
	if addr < 0x8000 {
		return 0
	}
	local := uint32(addr - 0x8000)
	bankSlot := local / vrc6PrgBankSize
	offset := local % vrc6PrgBankSize
	count := s.prgBankCount()
	switch bankSlot {
	case 0:
		return s.PrgRom[uint32(s.PrgBank0&0x0F)%count*vrc6PrgBankSize+offset]
	case 1:
		return s.PrgRom[uint32(s.PrgBank1&0x0F)%count*vrc6PrgBankSize+offset]
	case 2:
		return s.PrgRom[uint32(s.PrgBank2&0x0F)%count*vrc6PrgBankSize+offset]
	case 3:
		return s.PrgRom[(count-1)*vrc6PrgBankSize+offset]
	}
	return 0
}

func vrc6WritePRG(m *Mapper, addr uint16, value uint8) {
	s := &m.VRC6
	switch addr & 0xF003 {
	case 0x8000:
		s.PrgBank0 = value & 0x0F
	case 0x8001:
		s.PrgBank1 = value & 0x0F
	case 0x9000:
		s.PrgBank2 = value & 0x0F
	case 0xB003:
		if (value & 1) != 0 {
			s.Mirroring = MirrorVertical
		} else {
			s.Mirroring = MirrorHorizontal
		}
	}
}

func vrc6ReadCHR(m *Mapper, addr uint16) uint8 {
	s := &m.VRC6
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

func vrc6WriteCHR(m *Mapper, addr uint16, value uint8) {
	s := &m.VRC6
	if !s.ChrIsRAM {
		return
	}
	if s.ChrSize == 0 {
		return
	}
	s.Chr[uint32(addr)%s.ChrSize] = value
}

func vrc6ClockCPU(m *Mapper, cpuCycles uint32) {
	// VRC6 IRQ + sound (simplified)
}

func vrc6ExpansionAudio(m *Mapper) float32 { return 0 }

func vrc6SaveState(m *Mapper, buf []byte) int {
	s := &m.VRC6
	total := 4 + int(s.PrgSize) + 4 + 1 + 1 + 1 + 1 + 1 + 1 + 1 + 8 + int(s.ChrSize)
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
	buf[off] = boolByte(s.Is26); off++
	buf[off] = s.PrgBank0; off++
	buf[off] = s.PrgBank1; off++
	buf[off] = s.PrgBank2; off++
	copy(buf[off:], s.ChrBank[:]); off += 8
	copy(buf[off:], s.Chr); off += int(s.ChrSize)
	return off
}

func vrc6LoadState(m *Mapper, buf []byte) bool {
	s := &m.VRC6
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
	if len(buf) < off+4+8 {
		return false
	}
	chrSize := getU32(buf[off:]); off += 4
	if chrSize != s.ChrSize {
		return false
	}
	s.ChrIsRAM = buf[off] != 0; off++
	s.Mirroring = Mirroring(buf[off]); off++
	s.HasBattery = buf[off] != 0; off++
	s.Is26 = buf[off] != 0; off++
	s.PrgBank0 = buf[off]; off++
	s.PrgBank1 = buf[off]; off++
	s.PrgBank2 = buf[off]; off++
	copy(s.ChrBank[:], buf[off:off+8]); off += 8
	if len(buf) < off+int(s.ChrSize) {
		return false
	}
	copy(s.Chr, buf[off:off+int(s.ChrSize)]); off += int(s.ChrSize)
	return true
}
