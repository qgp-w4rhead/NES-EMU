// fds.go - Mapper 20 (FDS). Port of cores/c/src/fds.c.
package mapper

// FdsCreate builds an FDS mapper from BIOS + disk data.
func FdsCreate(bios, diskData []byte) (*Mapper, int) {
	m := &Mapper{Type: MapperFDS, MapperNum: 20}
	s := &m.FDS
	if len(bios) > 0 {
		s.Bios = make([]byte, len(bios))
		copy(s.Bios, bios)
	}
	if len(diskData) > 0 {
		s.DiskData = make([]byte, len(diskData))
		copy(s.DiskData, diskData)
	}
	s.Mirroring = MirrorVertical
	s.DiskSide = 0
	s.DiskPos = 0
	s.DiskEnabled = false
	return m, 0
}

func fdsReadPRG(m *Mapper, addr uint16) uint8 {
	s := &m.FDS
	if addr < 0x6000 {
		return 0
	}
	if addr >= 0x6000 && addr < 0xE000 {
		return s.PrgRAM[addr-0x6000]
	}
	if len(s.Bios) > 0 && addr >= 0xE000 {
		return s.Bios[addr-0xE000]
	}
	return 0
}

func fdsReadPRGMut(m *Mapper, addr uint16) uint8 {
	return fdsReadPRG(m, addr)
}

func fdsWritePRG(m *Mapper, addr uint16, value uint8) {
	s := &m.FDS
	if addr >= 0x6000 && addr < 0xE000 {
		s.PrgRAM[addr-0x6000] = value
	}
}

func fdsReadCHR(m *Mapper, addr uint16) uint8 { return 0 }
func fdsWriteCHR(m *Mapper, addr uint16, value uint8) {}

func fdsMirrorMode(m *Mapper) Mirroring { return m.FDS.Mirroring }

func fdsClockCPU(m *Mapper, cpuCycles uint32)        {}
func fdsExpansionAudio(m *Mapper) float32            { return 0 }

func fdsSaveState(m *Mapper, buf []byte) int {
	s := &m.FDS
	total := 4 + len(s.Bios) + 4 + len(s.DiskData) + 1 + 2 + 1 + 1 + 1 + 1 + 1 + 1 + 32768
	if buf == nil {
		return total
	}
	off := 0
	putU32(buf[off:], uint32(len(s.Bios))); off += 4
	if len(s.Bios) > 0 {
		copy(buf[off:], s.Bios); off += len(s.Bios)
	}
	putU32(buf[off:], uint32(len(s.DiskData))); off += 4
	if len(s.DiskData) > 0 {
		copy(buf[off:], s.DiskData); off += len(s.DiskData)
	}
	buf[off] = uint8(s.Mirroring); off++
	buf[off] = boolByte(s.HasBattery); off++
	buf[off] = s.DiskSide; off++
	putU16(buf[off:], s.DiskPos); off += 2
	buf[off] = boolByte(s.DiskEnabled); off++
	buf[off] = boolByte(s.DiskWriteable); off++
	buf[off] = boolByte(s.IrqEnabled); off++
	buf[off] = boolByte(s.IrqRepeat); off++
	buf[off] = s.IrcLatch; off++
	buf[off] = s.IrcCounter; off++
	buf[off] = boolByte(s.IrqPending); off++
	copy(buf[off:], s.PrgRAM[:]); off += 32768
	return off
}

func fdsLoadState(m *Mapper, buf []byte) bool {
	s := &m.FDS
	if len(buf) < 4 {
		return false
	}
	off := 0
	biosLen := getU32(buf[off:]); off += 4
	if int(biosLen) != len(s.Bios) {
		return false
	}
	if biosLen > 0 {
		copy(s.Bios, buf[off:off+int(biosLen)]); off += int(biosLen)
	}
	if len(buf) < off+4 {
		return false
	}
	diskLen := getU32(buf[off:]); off += 4
	if int(diskLen) != len(s.DiskData) {
		return false
	}
	if diskLen > 0 {
		copy(s.DiskData, buf[off:off+int(diskLen)]); off += int(diskLen)
	}
	if len(buf) < off+1+1+1+2+1+1+1+1+1+1+1+32768 {
		return false
	}
	s.Mirroring = Mirroring(buf[off]); off++
	s.HasBattery = buf[off] != 0; off++
	s.DiskSide = buf[off]; off++
	s.DiskPos = getU16(buf[off:]); off += 2
	s.DiskEnabled = buf[off] != 0; off++
	s.DiskWriteable = buf[off] != 0; off++
	s.IrqEnabled = buf[off] != 0; off++
	s.IrqRepeat = buf[off] != 0; off++
	s.IrcLatch = buf[off]; off++
	s.IrcCounter = buf[off]; off++
	s.IrqPending = buf[off] != 0; off++
	copy(s.PrgRAM[:], buf[off:off+32768]); off += 32768
	return true
}
