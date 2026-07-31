// Package mapper defines the mapper type enum and the tagged-union Mapper
// struct used for switch-based dispatch (M7 optimization requirement:
// avoid Go interface headers and indirect calls in the hot path).
//
// Port of cores/c/include/mapper.h + cores/c/src/mappers.c.
package mapper

import (
	"nes-core-go/internal/region"
)

// Mirroring is the nametable mirroring mode (matches C Mirroring enum).
type Mirroring int

const (
	MirrorHorizontal       Mirroring = 0
	MirrorVertical         Mirroring = 1
	MirrorFourScreen       Mirroring = 2
	MirrorSingleScreen0    Mirroring = 3 // SingleScreen(0) - AxROM / MMC1 1ScA
	MirrorSingleScreen1    Mirroring = 4 // SingleScreen(1) - AxROM / MMC1 1ScB
	MirrorSingleScreen2    Mirroring = 5 // AxROM
	MirrorSingleScreen3    Mirroring = 6 // AxROM
)

// MapperType is the discriminator for the tagged-union Mapper state.
// One constant per supported iNES mapper number.
type MapperType int

const (
	MapperNone     MapperType = 0
	MapperNROM     MapperType = 0 // shared with None for default
	MapperMMC1     MapperType = 1
	MapperUxROM    MapperType = 2
	MapperCNROM    MapperType = 3
	MapperMMC3     MapperType = 4
	MapperMMC5     MapperType = 5
	MapperAxROM    MapperType = 7
	MapperMMC2     MapperType = 9
	MapperNamco163 MapperType = 19
	MapperVRC6_24  MapperType = 24
	MapperVRC6_26  MapperType = 26
	MapperFDS      MapperType = 20
	MapperFME7     MapperType = 69
	MapperVRC7     MapperType = 85
)

// Mapper is a loaded mapper: type tag + tagged-union state + iNES mapper number.
// All per-mapper state structs live inline as fields; only one is meaningful
// for a given Type. PRG/CHR buffers are heap-allocated slices (they can be
// large and are not on the hot path); the dispatch switch is on Type.
//
// This mirrors the C `Mapper { vt, state, mapper_num }` but uses a tagged
// union instead of a vtable pointer to keep dispatch direct.
type Mapper struct {
	Type      MapperType
	MapperNum uint16
	// Tagged-union state. Only the field matching Type is meaningful.
	NROM     NromState
	MMC1     Mmc1State
	UxROM    UxromState
	CNROM     CnromState
	MMC3     Mmc3State
	MMC5     Mmc5State
	AxROM    AxromState
	MMC2     Mmc2State
	VRC6     Vrc6State
	FME7     Fme7State
	VRC7     Vrc7State
	Namco163 Namco163State
	FDS      FdsState
}

// ---- Per-mapper state structs (tagged-union arms). ----
// Each mirrors the corresponding C `*State` struct. PRG/CHR are heap slices.

// NromState mirrors C NromState.
type NromState struct {
	PrgRom     []byte
	PrgSize    uint32
	Chr        []byte
	ChrSize    uint32
	ChrIsRAM   bool
	Mirroring  Mirroring
	HasBattery bool
}

// Mmc1State mirrors C Mmc1State.
type Mmc1State struct {
	PrgRom      []byte
	PrgSize     uint32
	Chr         []byte
	ChrSize     uint32
	ChrIsRAM    bool
	PrgRAM      [8192]byte
	HasBattery  bool
	ShiftReg    uint8
	ShiftCount  uint8
	Control     uint8
	ChrBank0    uint8
	ChrBank1    uint8
	PrgBank     uint8
}

// UxromState mirrors C UxromState (Mapper 2).
type UxromState struct {
	PrgRom     []byte
	PrgSize    uint32
	Chr        []byte // 8 KB CHR-RAM
	ChrSize    uint32
	ChrIsRAM   bool
	Mirroring  Mirroring
	HasBattery bool
	BankSelect uint32 // selected 16 KB bank at $8000
}

// CnromState mirrors C CnromState (Mapper 3).
type CnromState struct {
	PrgRom     []byte
	PrgSize    uint32
	Chr        []byte
	ChrSize    uint32
	ChrIsRAM   bool
	Mirroring  Mirroring
	HasBattery bool
	ChrBank    uint8
}

// Mmc3State mirrors C Mmc3State (Mapper 4).
type Mmc3State struct {
	PrgRom      []byte
	PrgSize     uint32
	Chr         []byte
	ChrSize     uint32
	ChrIsRAM    bool
	PrgRAM      [8192]byte
	HasBattery  bool
	Mirroring   Mirroring
	BankSelect  uint8
	BankValues  [8]uint8
	PRGMode     uint8
	CHRMode     uint8
	IrqLatch    uint8
	IrqCounter  uint8
	IrqReload   bool
	IrqEnabled  bool
	IrqPending  bool
}

// Mmc5State mirrors C Mmc5State (Mapper 5).
type Mmc5State struct {
	PrgRom      []byte
	PrgSize     uint32
	Chr         []byte
	ChrSize     uint32
	ChrIsRAM    bool
	PrgRAM      [8192]byte
	HasBattery  bool
	Mirroring   Mirroring
	PrgMode     uint8
	ChrMode     uint8
	PrgBank1    uint8
	PrgBank2    uint8
	PrgBank3    uint8
	ChrBank1    uint8
	ChrBank2    uint8
	ChrBank3    uint8
	ChrBank4    uint8
	ChrBank5    uint8
	ChrBank6    uint8
	ChrBank7    uint8
	ChrBank8    uint8
	MultA       uint8
	MultB       uint8
	MultResult  uint16
	FillTile    uint8
	FillAttr    uint8
	IrqScanline uint8
	IrqEnabled  bool
	IrqPending  bool
	FrameCount  uint16
	InFrame     bool
}

// AxromState mirrors C AxromState (Mapper 7).
type AxromState struct {
	PrgRom     []byte
	PrgSize    uint32
	Chr        []byte
	ChrSize    uint32
	ChrIsRAM   bool
	Mirroring  Mirroring
	HasBattery bool
	BankSelect uint8
}

// Mmc2State mirrors C Mmc2State (Mapper 9).
type Mmc2State struct {
	PrgRom      []byte
	PrgSize     uint32
	Chr         []byte
	ChrSize     uint32
	ChrIsRAM    bool
	Mirroring   Mirroring
	HasBattery  bool
	PrgBank     uint8
	ChrBank0    uint8
	ChrBank1    uint8
	ChrBank2    uint8
	ChrBank3    uint8
	LatchFD     uint8
	LatchFE     uint8
}

// Vrc6State mirrors C Vrc6State (Mapper 24/26).
type Vrc6State struct {
	PrgRom      []byte
	PrgSize     uint32
	Chr         []byte
	ChrSize     uint32
	ChrIsRAM    bool
	Mirroring   Mirroring
	HasBattery  bool
	Is26        bool
	PrgBank0    uint8
	PrgBank1    uint8
	PrgBank2    uint8
	ChrBank     [8]uint8
	// VRC6 sound registers
	Pulse1Duty   uint8
	Pulse1Volume uint8
	Pulse1Period uint16
	Pulse1Timer  uint16
	Pulse1Phase  uint8
	Pulse1Enable bool
	Pulse2Duty   uint8
	Pulse2Volume uint8
	Pulse2Period uint16
	Pulse2Timer  uint16
	Pulse2Phase  uint8
	Pulse2Enable bool
	SawAccum     uint8
	SawPhase     uint8
	SawVolume    uint8
	SawPeriod    uint16
	SawTimer     uint16
	SawEnable    bool
	IrqEnabled   bool
	IrqAck       bool
	IrcLatch     uint8
	IrcCounter   uint8
	IrcPrescaler uint8
	IrqPending   bool
}

// Fme7State mirrors C Fme7State (Mapper 69 / Sunsoft 5B).
type Fme7State struct {
	PrgRom      []byte
	PrgSize     uint32
	Chr         []byte
	ChrSize     uint32
	ChrIsRAM    bool
	Mirroring   Mirroring
	HasBattery  bool
	Command     uint8
	PrgBank     [4]uint8
	ChrBank     [8]uint8
	// YM2149 / Sunsoft 5B registers
	YmReg       [16]uint8
	YmTone0     uint16
	YmTone1     uint16
	YmTone2     uint16
	YmNoise     uint8
	YmEnv       uint16
	YmEnvShape  uint8
	YmEnvPhase  uint8
	YmEnvCounter uint16
	YmCh0Vol    uint8
	YmCh1Vol    uint8
	YmCh2Vol    uint8
	YmCh0Out    uint8
	YmCh1Out    uint8
	YmCh2Out    uint8
	YmTone0Cnt  uint16
	YmTone1Cnt  uint16
	YmTone2Cnt  uint16
	YmNoiseCnt  uint8
	IrqEnabled  bool
	IrqCounter  uint16
	IrqLatch    uint16
	IrqPending  bool
}

// Vrc7State mirrors C Vrc7State (Mapper 85).
type Vrc7State struct {
	PrgRom      []byte
	PrgSize     uint32
	Chr         []byte
	ChrSize     uint32
	ChrIsRAM    bool
	Mirroring   Mirroring
	HasBattery  bool
	PrgBank     [4]uint8
	ChrBank     [8]uint8
	IrqEnabled  bool
	IrcLatch    uint8
	IrcCounter  uint8
	IrcPrescaler uint8
	IrqPending  bool
	// OPLL (YM2413) state - simplified
	OpllReg     [64]uint8
}

// Namco163State mirrors C Namco163State (Mapper 19).
type Namco163State struct {
	PrgRom      []byte
	PrgSize     uint32
	Chr         []byte
	ChrSize     uint32
	ChrIsRAM    bool
	Mirroring   Mirroring
	HasBattery  bool
	PrgBank     [4]uint8
	ChrBank     [8]uint8
	IrqEnabled  bool
	IrqCounter  uint16
	IrqLatch    uint16
	IrqPending  bool
	// Namco 163 sound state
	N163Ram     [128]byte
	N163Addr    uint8
	N163AddrInc uint8
	N163Freq    [8]uint32
	N163Phase   [8]uint32
	N163Len     [8]uint8
	N163Vol     [8]uint8
	N163Wave    [8]uint8
	N163Enable  uint8
}

// FdsState mirrors C FdsState (Mapper 20).
type FdsState struct {
	Bios       []byte
	DiskData   []byte
	Mirroring  Mirroring
	HasBattery bool
	PrgRAM     [32768]byte
	DiskSide   uint8
	DiskPos    uint16
	DiskEnabled bool
	DiskWriteable bool
	IrqEnabled  bool
	IrqRepeat   bool
	IrcLatch    uint8
	IrcCounter  uint8
	IrqPending  bool
	// FDS audio state
	Wave        [64]byte
	WavePos     uint8
	WaveAccum   uint16
	EnvAccum    uint16
	SpeedAccum  uint16
	Volume      uint8
	VolumeAccum uint16
	EnvEnabled  bool
	EnvSpeed    uint8
	EnvMode     uint8
	EnvDirection uint8
	EnvGain     uint8
	ModAccum    uint16
	ModCounter  uint8
	ModTable    [32]byte
	ModPos      uint8
	ModEnabled  bool
	ModSpeed    uint8
	ModPhase    uint8
	ModGain     uint8
	ModWrite    uint8
	LevelOutput uint8
}

// FromInes constructs the appropriate mapper for an iNES mapper number.
// Returns nil on unsupported mapper. PRG/CHR are copied into the mapper's
// own buffers (the caller may free the original ROM buffer).
func FromInes(mapperNumber uint16, prg []byte, chr []byte, mirroring Mirroring, hasBattery bool) (*Mapper, int) {
	m := &Mapper{MapperNum: mapperNumber}
	var rc int
	switch mapperNumber {
	case 0:
		m.Type = MapperNROM
		rc = NromCreate(m, prg, chr, mirroring, hasBattery)
	case 1:
		m.Type = MapperMMC1
		rc = Mmc1Create(m, prg, chr, mirroring, hasBattery)
	case 2:
		m.Type = MapperUxROM
		rc = UxromCreate(m, prg, chr, mirroring, hasBattery)
	case 3:
		m.Type = MapperCNROM
		rc = CnromCreate(m, prg, chr, mirroring, hasBattery)
	case 4:
		m.Type = MapperMMC3
		rc = Mmc3Create(m, prg, chr, mirroring, hasBattery)
	case 5:
		m.Type = MapperMMC5
		rc = Mmc5Create(m, prg, chr, mirroring, hasBattery)
	case 7:
		m.Type = MapperAxROM
		rc = AxromCreate(m, prg, chr, mirroring, hasBattery)
	case 9:
		m.Type = MapperMMC2
		rc = Mmc2Create(m, prg, chr, mirroring, hasBattery)
	case 19:
		m.Type = MapperNamco163
		rc = Namco163Create(m, prg, chr, mirroring, hasBattery)
	case 24:
		m.Type = MapperVRC6_24
		rc = Vrc6Create(m, prg, chr, mirroring, hasBattery, false)
	case 26:
		m.Type = MapperVRC6_26
		rc = Vrc6Create(m, prg, chr, mirroring, hasBattery, true)
	case 69:
		m.Type = MapperFME7
		rc = Fme7Create(m, prg, chr, mirroring, hasBattery)
	case 85:
		m.Type = MapperVRC7
		rc = Vrc7Create(m, prg, chr, mirroring, hasBattery)
	default:
		return nil, 3 // UnsupportedMapper
	}
	if rc != 0 {
		return nil, rc
	}
	return m, 0
}

// ---- Dispatch helpers (switch-based, NOT interface calls). ----

// ReadPRG reads a byte from CPU-side PRG ($6000..=$FFFF).
func (m *Mapper) ReadPRG(addr uint16) uint8 {
	switch m.Type {
	case MapperNROM:
		return nromReadPRG(m, addr)
	case MapperMMC1:
		return mmc1ReadPRG(m, addr)
	case MapperUxROM:
		return uxromReadPRG(m, addr)
	case MapperCNROM:
		return cnromReadPRG(m, addr)
	case MapperMMC3:
		return mmc3ReadPRG(m, addr)
	case MapperMMC5:
		return mmc5ReadPRG(m, addr)
	case MapperAxROM:
		return axromReadPRG(m, addr)
	case MapperMMC2:
		return mmc2ReadPRG(m, addr)
	case MapperVRC6_24, MapperVRC6_26:
		return vrc6ReadPRG(m, addr)
	case MapperFME7:
		return fme7ReadPRG(m, addr)
	case MapperVRC7:
		return vrc7ReadPRG(m, addr)
	case MapperNamco163:
		return namco163ReadPRG(m, addr)
	case MapperFDS:
		return fdsReadPRG(m, addr)
	default:
		return 0
	}
}

// ReadPRGMut reads PRG with side effects (FDS disk-data read).
func (m *Mapper) ReadPRGMut(addr uint16) uint8 {
	switch m.Type {
	case MapperFDS:
		return fdsReadPRGMut(m, addr)
	default:
		return m.ReadPRG(addr)
	}
}

// WritePRG writes a byte to CPU-side PRG ($6000..=$FFFF).
func (m *Mapper) WritePRG(addr uint16, value uint8) {
	switch m.Type {
	case MapperNROM:
		// no-op
	case MapperMMC1:
		mmc1WritePRG(m, addr, value)
	case MapperUxROM:
		uxromWritePRG(m, addr, value)
	case MapperCNROM:
		cnromWritePRG(m, addr, value)
	case MapperMMC3:
		mmc3WritePRG(m, addr, value)
	case MapperMMC5:
		mmc5WritePRG(m, addr, value)
	case MapperAxROM:
		axromWritePRG(m, addr, value)
	case MapperMMC2:
		mmc2WritePRG(m, addr, value)
	case MapperVRC6_24, MapperVRC6_26:
		vrc6WritePRG(m, addr, value)
	case MapperFME7:
		fme7WritePRG(m, addr, value)
	case MapperVRC7:
		vrc7WritePRG(m, addr, value)
	case MapperNamco163:
		namco163WritePRG(m, addr, value)
	case MapperFDS:
		fdsWritePRG(m, addr, value)
	}
}

// ReadCHR reads a byte from PPU-side CHR ($0000..=$1FFF).
func (m *Mapper) ReadCHR(addr uint16) uint8 {
	switch m.Type {
	case MapperNROM:
		return nromReadCHR(m, addr)
	case MapperMMC1:
		return mmc1ReadCHR(m, addr)
	case MapperUxROM:
		return uxromReadCHR(m, addr)
	case MapperCNROM:
		return cnromReadCHR(m, addr)
	case MapperMMC3:
		return mmc3ReadCHR(m, addr)
	case MapperMMC5:
		return mmc5ReadCHR(m, addr)
	case MapperAxROM:
		return axromReadCHR(m, addr)
	case MapperMMC2:
		return mmc2ReadCHR(m, addr)
	case MapperVRC6_24, MapperVRC6_26:
		return vrc6ReadCHR(m, addr)
	case MapperFME7:
		return fme7ReadCHR(m, addr)
	case MapperVRC7:
		return vrc7ReadCHR(m, addr)
	case MapperNamco163:
		return namco163ReadCHR(m, addr)
	case MapperFDS:
		return fdsReadCHR(m, addr)
	default:
		return 0
	}
}

// ReadCHRLatched reads CHR with side effects (MMC2 latching).
func (m *Mapper) ReadCHRLatched(addr uint16) uint8 {
	switch m.Type {
	case MapperMMC2:
		return mmc2ReadCHRLatched(m, addr)
	default:
		return m.ReadCHR(addr)
	}
}

// WriteCHR writes a byte to PPU-side CHR ($0000..=$1FFF).
func (m *Mapper) WriteCHR(addr uint16, value uint8) {
	switch m.Type {
	case MapperNROM:
		nromWriteCHR(m, addr, value)
	case MapperMMC1:
		mmc1WriteCHR(m, addr, value)
	case MapperUxROM:
		uxromWriteCHR(m, addr, value)
	case MapperCNROM:
		cnromWriteCHR(m, addr, value)
	case MapperMMC3:
		mmc3WriteCHR(m, addr, value)
	case MapperMMC5:
		mmc5WriteCHR(m, addr, value)
	case MapperAxROM:
		axromWriteCHR(m, addr, value)
	case MapperMMC2:
		mmc2WriteCHR(m, addr, value)
	case MapperVRC6_24, MapperVRC6_26:
		vrc6WriteCHR(m, addr, value)
	case MapperFME7:
		fme7WriteCHR(m, addr, value)
	case MapperVRC7:
		vrc7WriteCHR(m, addr, value)
	case MapperNamco163:
		namco163WriteCHR(m, addr, value)
	case MapperFDS:
		fdsWriteCHR(m, addr, value)
	}
}

// MirrorMode returns the nametable mirroring mode.
func (m *Mapper) MirrorMode() Mirroring {
	switch m.Type {
	case MapperNROM:
		return m.NROM.Mirroring
	case MapperMMC1:
		return mmc1MirrorMode(m)
	case MapperUxROM:
		return m.UxROM.Mirroring
	case MapperCNROM:
		return m.CNROM.Mirroring
	case MapperMMC3:
		if m.MMC3.Mirroring == MirrorHorizontal || m.MMC3.Mirroring == MirrorVertical {
			return m.MMC3.Mirroring
		}
		return m.MMC3.Mirroring
	case MapperMMC5:
		return m.MMC5.Mirroring
	case MapperAxROM:
		return axromMirrorMode(m)
	case MapperMMC2:
		return m.MMC2.Mirroring
	case MapperVRC6_24, MapperVRC6_26:
		return m.VRC6.Mirroring
	case MapperFME7:
		return fme7MirrorMode(m)
	case MapperVRC7:
		return m.VRC7.Mirroring
	case MapperNamco163:
		return namco163MirrorMode(m)
	case MapperFDS:
		return fdsMirrorMode(m)
	default:
		return MirrorHorizontal
	}
}

// ChrIsRAM returns whether CHR is writable RAM.
func (m *Mapper) ChrIsRAM() bool {
	switch m.Type {
	case MapperNROM:
		return m.NROM.ChrIsRAM
	case MapperMMC1:
		return m.MMC1.ChrIsRAM
	case MapperUxROM:
		return m.UxROM.ChrIsRAM
	case MapperCNROM:
		return m.CNROM.ChrIsRAM
	case MapperMMC3:
		return m.MMC3.ChrIsRAM
	case MapperMMC5:
		return m.MMC5.ChrIsRAM
	case MapperAxROM:
		return m.AxROM.ChrIsRAM
	case MapperMMC2:
		return m.MMC2.ChrIsRAM
	case MapperVRC6_24, MapperVRC6_26:
		return m.VRC6.ChrIsRAM
	case MapperFME7:
		return m.FME7.ChrIsRAM
	case MapperVRC7:
		return m.VRC7.ChrIsRAM
	case MapperNamco163:
		return m.Namco163.ChrIsRAM
	case MapperFDS:
		return true
	default:
		return false
	}
}

// HasBattery returns whether the cartridge has battery-backed PRG-RAM.
func (m *Mapper) HasBattery() bool {
	switch m.Type {
	case MapperNROM:
		return m.NROM.HasBattery
	case MapperMMC1:
		return m.MMC1.HasBattery
	case MapperUxROM:
		return m.UxROM.HasBattery
	case MapperCNROM:
		return m.CNROM.HasBattery
	case MapperMMC3:
		return m.MMC3.HasBattery
	case MapperMMC5:
		return m.MMC5.HasBattery
	case MapperAxROM:
		return m.AxROM.HasBattery
	case MapperMMC2:
		return m.MMC2.HasBattery
	case MapperVRC6_24, MapperVRC6_26:
		return m.VRC6.HasBattery
	case MapperFME7:
		return m.FME7.HasBattery
	case MapperVRC7:
		return m.VRC7.HasBattery
	case MapperNamco163:
		return m.Namco163.HasBattery
	case MapperFDS:
		return m.FDS.HasBattery
	default:
		return false
	}
}

// IrqPending returns whether the mapper is asserting a CPU IRQ.
func (m *Mapper) IrqPending() bool {
	switch m.Type {
	case MapperMMC3:
		return m.MMC3.IrqPending
	case MapperMMC5:
		return m.MMC5.IrqPending
	case MapperVRC6_24, MapperVRC6_26:
		return m.VRC6.IrqPending
	case MapperFME7:
		return m.FME7.IrqPending
	case MapperVRC7:
		return m.VRC7.IrqPending
	case MapperNamco163:
		return m.Namco163.IrqPending
	case MapperFDS:
		return m.FDS.IrqPending
	default:
		return false
	}
}

// ClockIRQ clocks the mapper's IRQ counter by one step (MMC3 A12 / MMC5 sl).
func (m *Mapper) ClockIRQ() {
	switch m.Type {
	case MapperMMC3:
		mmc3ClockIRQ(m)
	case MapperMMC5:
		mmc5ClockIRQ(m)
	}
}

// ResetScanlineCounter resets the mapper's per-frame scanline counter (MMC5).
func (m *Mapper) ResetScanlineCounter() {
	switch m.Type {
	case MapperMMC5:
		mmc5ResetScanlineCounter(m)
	}
}

// ClockCPU advances the mapper's CPU-clocked logic by cpu_cycles.
func (m *Mapper) ClockCPU(cpuCycles uint32) {
	switch m.Type {
	case MapperVRC6_24, MapperVRC6_26:
		vrc6ClockCPU(m, cpuCycles)
	case MapperFME7:
		fme7ClockCPU(m, cpuCycles)
	case MapperVRC7:
		vrc7ClockCPU(m, cpuCycles)
	case MapperNamco163:
		namco163ClockCPU(m, cpuCycles)
	case MapperFDS:
		fdsClockCPU(m, cpuCycles)
	}
}

// ExpansionAudioSample returns the current expansion-audio sample [-1.0, 1.0].
func (m *Mapper) ExpansionAudioSample() float32 {
	switch m.Type {
	case MapperVRC6_24, MapperVRC6_26:
		return vrc6ExpansionAudio(m)
	case MapperFME7:
		return fme7ExpansionAudio(m)
	case MapperVRC7:
		return vrc7ExpansionAudio(m)
	case MapperNamco163:
		return namco163ExpansionAudio(m)
	case MapperFDS:
		return fdsExpansionAudio(m)
	default:
		return 0
	}
}

// SaveState serializes the mapper state into buf. If buf is nil, returns the
// required size only. Returns bytes written (or required).
func (m *Mapper) SaveState(buf []byte) int {
	switch m.Type {
	case MapperNROM:
		return nromSaveState(m, buf)
	case MapperMMC1:
		return mmc1SaveState(m, buf)
	case MapperUxROM:
		return uxromSaveState(m, buf)
	case MapperCNROM:
		return cnromSaveState(m, buf)
	case MapperMMC3:
		return mmc3SaveState(m, buf)
	case MapperMMC5:
		return mmc5SaveState(m, buf)
	case MapperAxROM:
		return axromSaveState(m, buf)
	case MapperMMC2:
		return mmc2SaveState(m, buf)
	case MapperVRC6_24, MapperVRC6_26:
		return vrc6SaveState(m, buf)
	case MapperFME7:
		return fme7SaveState(m, buf)
	case MapperVRC7:
		return vrc7SaveState(m, buf)
	case MapperNamco163:
		return namco163SaveState(m, buf)
	case MapperFDS:
		return fdsSaveState(m, buf)
	default:
		return 0
	}
}

// LoadState restores mapper state from buf. Returns true on success.
func (m *Mapper) LoadState(buf []byte) bool {
	switch m.Type {
	case MapperNROM:
		return nromLoadState(m, buf)
	case MapperMMC1:
		return mmc1LoadState(m, buf)
	case MapperUxROM:
		return uxromLoadState(m, buf)
	case MapperCNROM:
		return cnromLoadState(m, buf)
	case MapperMMC3:
		return mmc3LoadState(m, buf)
	case MapperMMC5:
		return mmc5LoadState(m, buf)
	case MapperAxROM:
		return axromLoadState(m, buf)
	case MapperMMC2:
		return mmc2LoadState(m, buf)
	case MapperVRC6_24, MapperVRC6_26:
		return vrc6LoadState(m, buf)
	case MapperFME7:
		return fme7LoadState(m, buf)
	case MapperVRC7:
		return vrc7LoadState(m, buf)
	case MapperNamco163:
		return namco163LoadState(m, buf)
	case MapperFDS:
		return fdsLoadState(m, buf)
	default:
		return true
	}
}

// RegionHint is unused but kept for API symmetry.
var _ = region.NTSC
