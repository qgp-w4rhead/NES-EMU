// nrom.go - Mapper 0 (NROM). Port of cores/c/src/nrom.c.
package mapper

// NromCreate initializes the NROM state from PRG/CHR data.
func NromCreate(m *Mapper, prg, chr []byte, mirroring Mirroring, hasBattery bool) int {
	s := &m.NROM
	s.PrgSize = uint32(len(prg))
	if s.PrgSize > 0 {
		s.PrgRom = make([]byte, s.PrgSize)
		copy(s.PrgRom, prg)
	}
	if len(chr) > 0 {
		s.ChrSize = uint32(len(chr))
		s.Chr = make([]byte, s.ChrSize)
		copy(s.Chr, chr)
		s.ChrIsRAM = false
	} else {
		s.ChrSize = 8192
		s.Chr = make([]byte, s.ChrSize)
		s.ChrIsRAM = true
	}
	s.Mirroring = mirroring
	s.HasBattery = hasBattery
	return 0
}

func nromPrgIndex(s *NromState, addr uint16) uint32 {
	local := uint32(addr - 0x8000)
	bank := s.PrgSize
	if bank == 0 {
		return 0
	}
	return local % bank
}

func nromReadPRG(m *Mapper, addr uint16) uint8 {
	s := &m.NROM
	if addr < 0x8000 {
		return 0
	}
	idx := nromPrgIndex(s, addr)
	if idx >= s.PrgSize {
		return 0
	}
	return s.PrgRom[idx]
}

func nromReadCHR(m *Mapper, addr uint16) uint8 {
	s := &m.NROM
	if s.ChrSize == 0 {
		return 0
	}
	return s.Chr[uint32(addr)%s.ChrSize]
}

func nromWriteCHR(m *Mapper, addr uint16, value uint8) {
	s := &m.NROM
	if !s.ChrIsRAM {
		return
	}
	if s.ChrSize == 0 {
		return
	}
	s.Chr[uint32(addr)%s.ChrSize] = value
}

func nromSaveState(m *Mapper, buf []byte) int {
	s := &m.NROM
	total := nromStateSize(s)
	if buf == nil {
		return total
	}
	// Layout: [fixed scalar fields] + [prg_rom] + [chr if ram]
	off := 0
	putU32(buf[off:], s.PrgSize); off += 4
	if s.PrgSize > 0 {
		copy(buf[off:], s.PrgRom); off += int(s.PrgSize)
	}
	putU32(buf[off:], s.ChrSize); off += 4
	buf[off] = boolByte(s.ChrIsRAM); off++
	buf[off] = uint8(s.Mirroring); off++
	buf[off] = boolByte(s.HasBattery); off++
	if s.ChrIsRAM && s.ChrSize > 0 {
		copy(buf[off:], s.Chr); off += int(s.ChrSize)
	}
	return off
}

func nromStateSize(s *NromState) int {
	total := 4 + int(s.PrgSize) + 4 + 1 + 1 + 1
	if s.ChrIsRAM {
		total += int(s.ChrSize)
	}
	return total
}

func nromLoadState(m *Mapper, buf []byte) bool {
	s := &m.NROM
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
	if len(buf) < off+4+3 {
		return false
	}
	chrSize := getU32(buf[off:]); off += 4
	if chrSize != s.ChrSize {
		return false
	}
	s.ChrIsRAM = buf[off] != 0; off++
	s.Mirroring = Mirroring(buf[off]); off++
	s.HasBattery = buf[off] != 0; off++
	if s.ChrIsRAM && s.ChrSize > 0 {
		if len(buf) < off+int(s.ChrSize) {
			return false
		}
		copy(s.Chr, buf[off:off+int(s.ChrSize)]); off += int(s.ChrSize)
	}
	return true
}

// ---- shared little-endian helpers (used by all mappers) ----

func putU32(p []byte, v uint32) {
	p[0] = byte(v)
	p[1] = byte(v >> 8)
	p[2] = byte(v >> 16)
	p[3] = byte(v >> 24)
}

func getU32(p []byte) uint32 {
	return uint32(p[0]) | (uint32(p[1]) << 8) | (uint32(p[2]) << 16) | (uint32(p[3]) << 24)
}

func putU16(p []byte, v uint16) {
	p[0] = byte(v)
	p[1] = byte(v >> 8)
}

func getU16(p []byte) uint16 {
	return uint16(p[0]) | (uint16(p[1]) << 8)
}

func boolByte(b bool) byte {
	if b {
		return 1
	}
	return 0
}
