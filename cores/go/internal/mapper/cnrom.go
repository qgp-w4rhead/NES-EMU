// cnrom.go - Mapper 3 (CNROM). Port of cores/c/src/cnrom.c.
package mapper

func CnromCreate(m *Mapper, prg, chr []byte, mirroring Mirroring, hasBattery bool) int {
	s := &m.CNROM
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

func cnromReadPRG(m *Mapper, addr uint16) uint8 {
	s := &m.CNROM
	if addr < 0x8000 {
		return 0
	}
	local := uint32(addr - 0x8000)
	if s.PrgSize == 0 {
		return 0
	}
	return s.PrgRom[local%s.PrgSize]
}

func cnromWritePRG(m *Mapper, addr uint16, value uint8) {
	s := &m.CNROM
	if addr >= 0x8000 {
		s.ChrBank = value & 0x03
	}
}

func cnromReadCHR(m *Mapper, addr uint16) uint8 {
	s := &m.CNROM
	if s.ChrSize == 0 {
		return 0
	}
	bankSize := uint32(8192)
	if s.ChrSize < bankSize {
		bankSize = s.ChrSize
	}
	bank := uint32(s.ChrBank) % (s.ChrSize / bankSize)
	idx := bank*bankSize + (uint32(addr) % bankSize)
	if idx >= s.ChrSize {
		return 0
	}
	return s.Chr[idx]
}

func cnromWriteCHR(m *Mapper, addr uint16, value uint8) {
	s := &m.CNROM
	if !s.ChrIsRAM {
		return
	}
	if s.ChrSize == 0 {
		return
	}
	s.Chr[uint32(addr)%s.ChrSize] = value
}

func cnromSaveState(m *Mapper, buf []byte) int {
	s := &m.CNROM
	total := 4 + int(s.PrgSize) + 4 + 1 + 1 + 1 + 1
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
	buf[off] = uint8(s.Mirroring); off++
	buf[off] = boolByte(s.HasBattery); off++
	buf[off] = s.ChrBank; off++
	if s.ChrIsRAM && s.ChrSize > 0 {
		copy(buf[off:], s.Chr); off += int(s.ChrSize)
	}
	return off
}

func cnromLoadState(m *Mapper, buf []byte) bool {
	s := &m.CNROM
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
	if len(buf) < off+4+4 {
		return false
	}
	chrSize := getU32(buf[off:]); off += 4
	if chrSize != s.ChrSize {
		return false
	}
	s.ChrIsRAM = buf[off] != 0; off++
	s.Mirroring = Mirroring(buf[off]); off++
	s.HasBattery = buf[off] != 0; off++
	s.ChrBank = buf[off]; off++
	if s.ChrIsRAM && s.ChrSize > 0 {
		if len(buf) < off+int(s.ChrSize) {
			return false
		}
		copy(s.Chr, buf[off:off+int(s.ChrSize)]); off += int(s.ChrSize)
	}
	return true
}
