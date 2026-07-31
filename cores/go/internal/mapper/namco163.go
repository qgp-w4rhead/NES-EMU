// namco163.go - Mapper 19 (Namco 163). Port of cores/c/src/namco163.c.
package mapper

const namcoPrgBankSize = 8192

func Namco163Create(m *Mapper, prg, chr []byte, mirroring Mirroring, hasBattery bool) int {
	s := &m.Namco163
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

func (s *Namco163State) prgBankCount() uint32 {
	n := s.PrgSize / namcoPrgBankSize
	if n == 0 {
		return 1
	}
	return n
}

func namco163ReadPRG(m *Mapper, addr uint16) uint8 {
	s := &m.Namco163
	if addr < 0x8000 {
		return 0
	}
	local := uint32(addr - 0x8000)
	bankSlot := local / namcoPrgBankSize
	offset := local % namcoPrgBankSize
	count := s.prgBankCount()
	switch bankSlot {
	case 0:
		return s.PrgRom[uint32(s.PrgBank[0]&0x3F)%count*namcoPrgBankSize+offset]
	case 1:
		return s.PrgRom[uint32(s.PrgBank[1]&0x3F)%count*namcoPrgBankSize+offset]
	case 2:
		return s.PrgRom[uint32(s.PrgBank[2]&0x3F)%count*namcoPrgBankSize+offset]
	case 3:
		return s.PrgRom[(count-1)*namcoPrgBankSize+offset]
	}
	return 0
}

func namco163WritePRG(m *Mapper, addr uint16, value uint8) {
	s := &m.Namco163
	switch addr & 0xF800 {
	case 0x8000:
		s.PrgBank[0] = value & 0x3F
	case 0x8800:
		s.PrgBank[1] = value & 0x3F
	case 0x9000:
		s.PrgBank[2] = value & 0x3F
	case 0x4800:
		// N163 address port
		s.N163Addr = value & 0x7F
		s.N163AddrInc = (value >> 6) & 1
	case 0x5000:
		// N163 data port
		s.N163Ram[s.N163Addr] = value
		if s.N163AddrInc != 0 {
			s.N163Addr = (s.N163Addr + 1) & 0x7F
		}
	}
}

func namco163MirrorMode(m *Mapper) Mirroring { return m.Namco163.Mirroring }

func namco163ReadCHR(m *Mapper, addr uint16) uint8 {
	s := &m.Namco163
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

func namco163WriteCHR(m *Mapper, addr uint16, value uint8) {
	s := &m.Namco163
	if !s.ChrIsRAM {
		return
	}
	if s.ChrSize == 0 {
		return
	}
	s.Chr[uint32(addr)%s.ChrSize] = value
}

func namco163ClockCPU(m *Mapper, cpuCycles uint32) {}
func namco163ExpansionAudio(m *Mapper) float32    { return 0 }

func namco163SaveState(m *Mapper, buf []byte) int {
	s := &m.Namco163
	total := 4 + int(s.PrgSize) + 4 + 1 + 1 + 1 + 4 + 8 + 128 + 1 + 1 + int(s.ChrSize)
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
	copy(buf[off:], s.PrgBank[:]); off += 4
	copy(buf[off:], s.ChrBank[:]); off += 8
	copy(buf[off:], s.N163Ram[:]); off += 128
	buf[off] = s.N163Addr; off++
	buf[off] = s.N163AddrInc; off++
	copy(buf[off:], s.Chr); off += int(s.ChrSize)
	return off
}

func namco163LoadState(m *Mapper, buf []byte) bool {
	s := &m.Namco163
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
	if len(buf) < off+4+4+8+128+2 {
		return false
	}
	chrSize := getU32(buf[off:]); off += 4
	if chrSize != s.ChrSize {
		return false
	}
	s.ChrIsRAM = buf[off] != 0; off++
	s.Mirroring = Mirroring(buf[off]); off++
	s.HasBattery = buf[off] != 0; off++
	copy(s.PrgBank[:], buf[off:off+4]); off += 4
	copy(s.ChrBank[:], buf[off:off+8]); off += 8
	copy(s.N163Ram[:], buf[off:off+128]); off += 128
	s.N163Addr = buf[off]; off++
	s.N163AddrInc = buf[off]; off++
	if len(buf) < off+int(s.ChrSize) {
		return false
	}
	copy(s.Chr, buf[off:off+int(s.ChrSize)]); off += int(s.ChrSize)
	return true
}
