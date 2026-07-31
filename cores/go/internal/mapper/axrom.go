// axrom.go - Mapper 7 (AxROM). Port of cores/c/src/axrom.c.
package mapper

const axromPrgBankSize = 32768

func AxromCreate(m *Mapper, prg, chr []byte, mirroring Mirroring, hasBattery bool) int {
	s := &m.AxROM
	s.PrgSize = uint32(len(prg))
	if s.PrgSize > 0 {
		s.PrgRom = make([]byte, s.PrgSize)
		copy(s.PrgRom, prg)
	}
	s.ChrSize = 8192
	s.Chr = make([]byte, s.ChrSize)
	s.ChrIsRAM = true
	s.Mirroring = mirroring
	s.HasBattery = hasBattery
	s.BankSelect = 0
	return 0
}

func axromReadPRG(m *Mapper, addr uint16) uint8 {
	s := &m.AxROM
	if addr < 0x8000 {
		return 0
	}
	local := uint32(addr - 0x8000)
	bankCount := s.PrgSize / axromPrgBankSize
	if bankCount == 0 {
		return 0
	}
	bank := uint32(s.BankSelect&0x07) % bankCount
	idx := bank*axromPrgBankSize + local
	if idx >= s.PrgSize {
		return 0
	}
	return s.PrgRom[idx]
}

func axromWritePRG(m *Mapper, addr uint16, value uint8) {
	s := &m.AxROM
	if addr >= 0x8000 {
		s.BankSelect = value & 0x0F
	}
}

func axromReadCHR(m *Mapper, addr uint16) uint8 {
	s := &m.AxROM
	if s.ChrSize == 0 {
		return 0
	}
	return s.Chr[uint32(addr)%s.ChrSize]
}

func axromWriteCHR(m *Mapper, addr uint16, value uint8) {
	s := &m.AxROM
	if !s.ChrIsRAM {
		return
	}
	if s.ChrSize == 0 {
		return
	}
	s.Chr[uint32(addr)%s.ChrSize] = value
}

func axromMirrorMode(m *Mapper) Mirroring {
	if m.AxROM.BankSelect&0x10 != 0 {
		return MirrorSingleScreen1
	}
	return MirrorSingleScreen0
}

func axromSaveState(m *Mapper, buf []byte) int {
	s := &m.AxROM
	total := 4 + int(s.PrgSize) + 4 + 1 + 1 + 1 + 1 + int(s.ChrSize)
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
	buf[off] = s.BankSelect; off++
	copy(buf[off:], s.Chr); off += int(s.ChrSize)
	return off
}

func axromLoadState(m *Mapper, buf []byte) bool {
	s := &m.AxROM
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
	s.BankSelect = buf[off]; off++
	if len(buf) < off+int(s.ChrSize) {
		return false
	}
	copy(s.Chr, buf[off:off+int(s.ChrSize)]); off += int(s.ChrSize)
	return true
}
