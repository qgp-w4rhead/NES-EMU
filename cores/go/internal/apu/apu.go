// Package apu implements the NES Audio Processing Unit
// (port of cores/c/src/apu.c).
package apu

import (
	"nes-core-go/internal/region"
	"unsafe"
)

// ---- Constants ----

var DutyPatterns = [4][8]uint8{
	{0, 1, 0, 0, 0, 0, 0, 0},
	{0, 1, 1, 0, 0, 0, 0, 0},
	{0, 1, 1, 1, 1, 0, 0, 0},
	{1, 0, 0, 0, 1, 1, 1, 1},
}

var LengthTable = [32]uint8{
	10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14,
	12, 16, 24, 18, 48, 20, 96, 22, 192, 24, 72, 26, 16, 28, 32, 30,
}

var TriangleSequence = [32]uint8{
	15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0,
	0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
}

var NoisePeriodTable = [16]uint16{
	4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
}

var DmcRateTable = [16]uint16{
	214, 190, 170, 160, 149, 138, 127, 113, 107, 95, 85, 80, 71, 63, 54, 42,
}

const (
	MaxPeriod          uint16 = 0x7FF
	MinAudiblePeriod   uint16 = 8
	SampleRate         = 44100
	ChannelCount       = 5
)

// DmcReadFn fetches a byte from CPU memory for the DMC.
type DmcReadFn func(addr uint16) uint8

// PulseChannel is one of the two NES pulse wave channels.
type PulseChannel struct {
	Pulse2          bool
	Duty            uint8
	Halt            bool
	ConstantVolume  bool
	Volume          uint8
	SweepEnabled    bool
	SweepPeriod     uint8
	SweepNegate     bool
	SweepShift      uint8
	SweepDivider    uint8
	SweepReload     bool
	TimerPeriod     uint16
	Timer           uint16
	Sequence        uint8
	LengthCounter   uint8
	Enabled         bool
	EnvelopeDivider uint8
	EnvelopeDecay   uint8
	EnvelopeStart   bool
}

func (c *PulseChannel) Init(pulse2 bool) {
	*c = PulseChannel{Pulse2: pulse2}
}

func (c *PulseChannel) WriteRegister(reg uint8, value uint8) {
	switch reg {
	case 0:
		c.Duty = (value >> 6) & 0x03
		c.Halt = (value & 0x20) != 0
		c.ConstantVolume = (value & 0x10) != 0
		c.Volume = value & 0x0F
	case 1:
		c.SweepEnabled = (value & 0x80) != 0
		c.SweepPeriod = (value >> 4) & 0x07
		c.SweepNegate = (value & 0x08) != 0
		c.SweepShift = value & 0x07
		c.SweepReload = true
	case 2:
		c.TimerPeriod = (c.TimerPeriod & 0xFF00) | uint16(value)
	case 3:
		high := uint16(value & 0x07)
		c.TimerPeriod = (c.TimerPeriod & 0x00FF) | (high << 8)
		c.Timer = (c.Timer & 0x00FF) | (high << 8)
		if c.Enabled {
			idx := value >> 3
			c.LengthCounter = LengthTable[idx]
		}
		c.EnvelopeStart = true
		c.Sequence = 0
	}
}

func (c *PulseChannel) SetEnabled(enabled bool) {
	c.Enabled = enabled
	if !enabled {
		c.LengthCounter = 0
	}
}

func (c *PulseChannel) sweepTarget() uint16 {
	if !c.SweepEnabled || c.SweepShift == 0 {
		return c.TimerPeriod
	}
	shifted := c.TimerPeriod >> c.SweepShift
	if c.SweepNegate {
		if c.Pulse2 {
			return c.TimerPeriod - shifted - 1
		}
		return c.TimerPeriod - shifted
	}
	return c.TimerPeriod + shifted
}

func (c *PulseChannel) IsMuted() bool {
	return c.TimerPeriod < MinAudiblePeriod || c.sweepTarget() > MaxPeriod
}

func (c *PulseChannel) Tick() {
	if c.Timer == 0 {
		c.Timer = c.TimerPeriod
		c.Sequence = (c.Sequence + 1) & 0x07
	} else {
		c.Timer--
	}
}

func (c *PulseChannel) clockEnvelope() {
	if c.EnvelopeStart {
		c.EnvelopeStart = false
		c.EnvelopeDecay = 15
		c.EnvelopeDivider = c.Volume
	} else if c.EnvelopeDivider == 0 {
		c.EnvelopeDivider = c.Volume
		if c.EnvelopeDecay > 0 {
			c.EnvelopeDecay--
		} else if c.Halt {
			c.EnvelopeDecay = 15
		}
	} else {
		c.EnvelopeDivider--
	}
}

func (c *PulseChannel) clockLength() {
	if !c.Halt && c.LengthCounter > 0 {
		c.LengthCounter--
	}
}

func (c *PulseChannel) clockSweep() {
	dividerZero := c.SweepDivider == 0
	if dividerZero && c.SweepEnabled && c.SweepShift != 0 {
		target := c.sweepTarget()
		if target <= MaxPeriod && c.TimerPeriod >= MinAudiblePeriod {
			c.TimerPeriod = target
		}
	}
	if dividerZero || c.SweepReload {
		c.SweepDivider = c.SweepPeriod
		c.SweepReload = false
	} else {
		c.SweepDivider--
	}
}

func (c *PulseChannel) ClockQuarterFrame() { c.clockEnvelope() }
func (c *PulseChannel) ClockHalfFrame() {
	c.clockLength()
	c.clockSweep()
}

func (c *PulseChannel) Sample() uint8 {
	if !c.Enabled || c.LengthCounter == 0 || c.IsMuted() {
		return 0
	}
	if DutyPatterns[c.Duty][c.Sequence] == 0 {
		return 0
	}
	if c.ConstantVolume {
		return c.Volume
	}
	return c.EnvelopeDecay
}

// TriangleChannel is the NES triangle wave channel.
type TriangleChannel struct {
	Halt          bool
	LinearReload  uint8
	LinearCounter uint8
	LinearStart   bool
	TimerPeriod   uint16
	Timer         uint16
	Sequence      uint8
	LengthCounter uint8
	Enabled       bool
}

func (c *TriangleChannel) Init() { *c = TriangleChannel{} }

func (c *TriangleChannel) WriteRegister(reg uint8, value uint8) {
	switch reg {
	case 0:
		c.Halt = (value & 0x80) != 0
		c.LinearReload = value & 0x7F
	case 1:
		// unused
	case 2:
		c.TimerPeriod = (c.TimerPeriod & 0xFF00) | uint16(value)
	case 3:
		high := uint16(value & 0x07)
		c.TimerPeriod = (c.TimerPeriod & 0x00FF) | (high << 8)
		c.Timer = (c.Timer & 0x00FF) | (high << 8)
		if c.Enabled {
			idx := value >> 3
			c.LengthCounter = LengthTable[idx]
		}
		c.LinearStart = true
		c.Sequence = 0
	}
}

func (c *TriangleChannel) SetEnabled(enabled bool) {
	c.Enabled = enabled
	if !enabled {
		c.LengthCounter = 0
	}
}

func (c *TriangleChannel) Tick() {
	if c.Timer == 0 {
		c.Timer = c.TimerPeriod
		c.Sequence = (c.Sequence + 1) & 0x1F
	} else {
		c.Timer--
	}
}

func (c *TriangleChannel) clockLinear() {
	if c.LinearStart {
		c.LinearStart = false
		c.LinearCounter = c.LinearReload
	} else if c.LinearCounter > 0 {
		c.LinearCounter--
	}
	if c.Halt {
		c.LinearStart = true
	}
}

func (c *TriangleChannel) clockLength() {
	if !c.Halt && c.LengthCounter > 0 {
		c.LengthCounter--
	}
}

func (c *TriangleChannel) ClockQuarterFrame() { c.clockLinear() }
func (c *TriangleChannel) ClockHalfFrame()   { c.clockLength() }

func (c *TriangleChannel) Sample() uint8 {
	if !c.Enabled || c.LengthCounter == 0 || c.LinearCounter == 0 {
		return 0
	}
	return TriangleSequence[c.Sequence]
}

// NoiseChannel is the NES noise channel.
type NoiseChannel struct {
	Halt            bool
	ConstantVolume  bool
	Volume          uint8
	Mode            bool
	PeriodIndex     uint8
	TimerPeriod     uint16
	Timer           uint16
	Lfsr            uint16
	LengthCounter   uint8
	Enabled         bool
	EnvelopeDivider uint8
	EnvelopeDecay   uint8
	EnvelopeStart   bool
}

func (c *NoiseChannel) Init() {
	*c = NoiseChannel{}
	c.TimerPeriod = NoisePeriodTable[0]
	c.Lfsr = 1
}

func (c *NoiseChannel) WriteRegister(reg uint8, value uint8) {
	switch reg {
	case 0:
		c.Halt = (value & 0x20) != 0
		c.ConstantVolume = (value & 0x10) != 0
		c.Volume = value & 0x0F
	case 1:
		// unused
	case 2:
		c.Mode = (value & 0x80) != 0
		c.PeriodIndex = value & 0x0F
		c.TimerPeriod = NoisePeriodTable[c.PeriodIndex]
	case 3:
		if c.Enabled {
			idx := value >> 3
			c.LengthCounter = LengthTable[idx]
		}
		c.EnvelopeStart = true
	}
}

func (c *NoiseChannel) SetEnabled(enabled bool) {
	c.Enabled = enabled
	if !enabled {
		c.LengthCounter = 0
	}
}

func (c *NoiseChannel) Tick() {
	if c.Timer == 0 {
		c.Timer = c.TimerPeriod
		bit0 := uint16(c.Lfsr & 0x0001)
		var tap uint16
		if c.Mode {
			tap = 6
		} else {
			tap = 1
		}
		tapBit := (c.Lfsr >> tap) & 0x0001
		feedback := bit0 ^ tapBit
		c.Lfsr >>= 1
		if feedback != 0 {
			c.Lfsr |= 0x4000
		}
	} else {
		c.Timer--
	}
}

func (c *NoiseChannel) clockEnvelope() {
	if c.EnvelopeStart {
		c.EnvelopeStart = false
		c.EnvelopeDecay = 15
		c.EnvelopeDivider = c.Volume
	} else if c.EnvelopeDivider == 0 {
		c.EnvelopeDivider = c.Volume
		if c.EnvelopeDecay > 0 {
			c.EnvelopeDecay--
		} else if c.Halt {
			c.EnvelopeDecay = 15
		}
	} else {
		c.EnvelopeDivider--
	}
}

func (c *NoiseChannel) clockLength() {
	if !c.Halt && c.LengthCounter > 0 {
		c.LengthCounter--
	}
}

func (c *NoiseChannel) ClockQuarterFrame() { c.clockEnvelope() }
func (c *NoiseChannel) ClockHalfFrame()    { c.clockLength() }

func (c *NoiseChannel) Sample() uint8 {
	if !c.Enabled || c.LengthCounter == 0 {
		return 0
	}
	if (c.Lfsr & 1) != 0 {
		return 0
	}
	if c.ConstantVolume {
		return c.Volume
	}
	return c.EnvelopeDecay
}

// DmcChannel is the NES DMC channel.
type DmcChannel struct {
	IrqEnable       bool
	LoopFlag        bool
	RateIndex       uint8
	TimerPeriod     uint16
	Timer           uint16
	OutputCounter   uint8
	SampleBuffer    uint8
	BufferBits      uint8
	SampleAddrBase  uint16
	SampleAddress   uint16
	SampleLength    uint16
	BytesRemaining  uint16
	Enabled         bool
	IrqFlag         bool
}

func (c *DmcChannel) Init() {
	*c = DmcChannel{}
	c.TimerPeriod = DmcRateTable[0]
	c.SampleAddrBase = 0xC000
	c.SampleAddress = 0xC000
	c.SampleLength = 1
}

func (c *DmcChannel) WriteRegister(reg uint8, value uint8) {
	switch reg {
	case 0:
		c.IrqEnable = (value & 0x80) != 0
		c.LoopFlag = (value & 0x40) != 0
		c.RateIndex = value & 0x0F
		c.TimerPeriod = DmcRateTable[c.RateIndex]
	case 1:
		c.OutputCounter = value & 0x7F
	case 2:
		c.SampleAddrBase = (uint16(value) << 6) | 0xC000
	case 3:
		c.SampleLength = (uint16(value) << 4) | 1
	}
}

func (c *DmcChannel) SetEnabled(enabled bool) {
	c.Enabled = enabled
	if enabled {
		if c.BytesRemaining == 0 {
			c.SampleAddress = c.SampleAddrBase
			c.BytesRemaining = c.SampleLength
			c.BufferBits = 0
		}
	} else {
		c.BytesRemaining = 0
	}
}

func (c *DmcChannel) ClearIRQ() { c.IrqFlag = false }

func (c *DmcChannel) clockOutputUnit(read DmcReadFn) {
	if c.BufferBits == 0 && c.BytesRemaining > 0 {
		c.SampleBuffer = read(c.SampleAddress)
		c.BufferBits = 8
		c.SampleAddress++
		if c.SampleAddress == 0 {
			c.SampleAddress = 0x8000
		}
		c.BytesRemaining--
		if c.BytesRemaining == 0 {
			if c.LoopFlag {
				c.SampleAddress = c.SampleAddrBase
				c.BytesRemaining = c.SampleLength
			} else if c.IrqEnable {
				c.IrqFlag = true
			}
		}
	}
	if c.BufferBits > 0 {
		bit := c.SampleBuffer & 1
		if bit == 0 {
			if c.OutputCounter >= 2 {
				c.OutputCounter -= 2
			} else {
				c.OutputCounter = 0
			}
		} else {
			v := uint16(c.OutputCounter) + 2
			if v > 127 {
				v = 127
			}
			c.OutputCounter = uint8(v)
		}
		c.SampleBuffer >>= 1
		c.BufferBits--
	}
}

func (c *DmcChannel) Tick(read DmcReadFn) {
	if c.Timer == 0 {
		c.Timer = c.TimerPeriod
		c.clockOutputUnit(read)
	} else {
		c.Timer--
	}
}

func (c *DmcChannel) Sample() uint8 { return c.OutputCounter }

// Apu is the full NES APU.
type Apu struct {
	Pulse1    PulseChannel
	Pulse2    PulseChannel
	Triangle  TriangleChannel
	Noise     NoiseChannel
	Dmc       DmcChannel

	CycleAccumulator uint32

	ChannelVolumes [ChannelCount]float32
	ChannelMuted   [ChannelCount]bool
	SelectedChannel uint8

	LpfPrev       float32
	DcPrevX       float32
	DcPrevY       float32
	MixAccumulator float32
	MixCount      uint32
	SampleAccumulator float32
	LastDecimated float32

	FrameMode5Step   bool
	FrameIrqInhibit  bool
	FrameCycle       uint32
	FrameIrq         bool
	FrameResetDelay  uint32

	Region region.Region
}

// Init constructs a reset APU with all channels silenced.
func (a *Apu) Init() {
	*a = Apu{}
	a.Pulse1.Init(false)
	a.Pulse2.Init(true)
	a.Triangle.Init()
	a.Noise.Init()
	a.Dmc.Init()
	for i := 0; i < ChannelCount; i++ {
		a.ChannelVolumes[i] = 1.0
	}
	a.LpfPrev = -1.0
	a.DcPrevX = -1.0
	a.LastDecimated = -1.0
	a.Region = region.NTSC
}

func (a *Apu) WriteStatus(value uint8) {
	a.Pulse1.SetEnabled((value & 0x01) != 0)
	a.Pulse2.SetEnabled((value & 0x02) != 0)
	a.Triangle.SetEnabled((value & 0x04) != 0)
	a.Noise.SetEnabled((value & 0x08) != 0)
	a.Dmc.SetEnabled((value & 0x10) != 0)
}

func (a *Apu) ReadStatus() uint8 {
	var v uint8
	if a.Pulse1.LengthCounter > 0 {
		v |= 0x01
	}
	if a.Pulse2.LengthCounter > 0 {
		v |= 0x02
	}
	if a.Triangle.LengthCounter > 0 {
		v |= 0x04
	}
	if a.Noise.LengthCounter > 0 {
		v |= 0x08
	}
	if a.Dmc.BytesRemaining > 0 {
		v |= 0x10
	}
	if a.FrameIrq {
		v |= 0x40
	}
	if a.Dmc.IrqFlag {
		v |= 0x80
	}
	a.FrameIrq = false
	a.Dmc.IrqFlag = false
	return v
}

func (a *Apu) WriteFrameCounter(value uint8) {
	newMode5Step := (value & 0x80) != 0
	a.FrameIrqInhibit = (value & 0x40) != 0
	if a.FrameIrqInhibit {
		a.FrameIrq = false
	}
	if newMode5Step {
		a.ClockQuarterFrame()
		a.ClockHalfFrame()
	}
	a.FrameResetDelay = 4
	a.FrameMode5Step = newMode5Step
}

func (a *Apu) IrqPending() bool {
	return a.FrameIrq || a.Dmc.IrqFlag
}

func (a *Apu) SetRegion(r region.Region) { a.Region = r }

func (a *Apu) stepFrameCounter(cpuCycles uint32) {
	if a.FrameResetDelay > 0 {
		var advance uint32
		if cpuCycles < a.FrameResetDelay {
			advance = cpuCycles
		} else {
			advance = a.FrameResetDelay
		}
		a.FrameResetDelay -= advance
		if a.FrameResetDelay == 0 {
			a.FrameCycle = 0
		}
		remaining := cpuCycles - advance
		if remaining == 0 {
			return
		}
		a.FrameCycle += remaining
	} else {
		a.FrameCycle += cpuCycles
	}

	var prev uint32
	if a.FrameCycle >= cpuCycles {
		prev = a.FrameCycle - cpuCycles
	}
	var thresholds [4]region.ApuFrameThreshold
	if a.FrameMode5Step {
		thresholds = region.APU5StepThresholds(a.Region)
	} else {
		thresholds = region.APU4StepThresholds(a.Region)
	}
	irqThreshold := region.APU4StepIRQThreshold(a.Region)
	for i := 0; i < 4; i++ {
		t := thresholds[i]
		if prev < t.Threshold && a.FrameCycle >= t.Threshold {
			if t.Quarter {
				a.ClockQuarterFrame()
			}
			if t.Half {
				a.ClockHalfFrame()
			}
			if !a.FrameMode5Step && t.Threshold == irqThreshold && !a.FrameIrqInhibit {
				a.FrameIrq = true
			}
		}
	}
	resetAt := region.APUResetAt(a.Region, a.FrameMode5Step)
	if a.FrameCycle >= resetAt {
		a.FrameCycle -= resetAt
	}
}

// Step advances the APU by cpuCycles CPU cycles.
func (a *Apu) Step(cpuCycles uint32, read DmcReadFn) {
	a.stepFrameCounter(cpuCycles)
	apuCyclesPerSample := region.CPUCyclesPerSample(a.Region) / 2.0
	a.CycleAccumulator += cpuCycles
	for a.CycleAccumulator >= 2 {
		a.CycleAccumulator -= 2
		a.Pulse1.Tick()
		a.Pulse2.Tick()
		a.Triangle.Tick()
		a.Noise.Tick()
		a.Dmc.Tick(read)

		a.MixAccumulator += a.mixRaw()
		a.MixCount++
		a.SampleAccumulator += 1.0
		if a.SampleAccumulator >= apuCyclesPerSample {
			a.SampleAccumulator -= apuCyclesPerSample
			if a.MixCount > 0 {
				a.LastDecimated = a.MixAccumulator / float32(a.MixCount)
				a.MixAccumulator = 0
				a.MixCount = 0
			}
		}
	}
}

func (a *Apu) ClockQuarterFrame() {
	a.Pulse1.ClockQuarterFrame()
	a.Pulse2.ClockQuarterFrame()
	a.Triangle.ClockQuarterFrame()
	a.Noise.ClockQuarterFrame()
}

func (a *Apu) ClockHalfFrame() {
	a.Pulse1.ClockHalfFrame()
	a.Pulse2.ClockHalfFrame()
	a.Triangle.ClockHalfFrame()
	a.Noise.ClockHalfFrame()
}

// Output returns the full audio output [-1.0, 1.0] with LPF + DC blocker.
func (a *Apu) Output() float32 {
	clamped := a.LastDecimated
	const LPF_ALPHA float32 = 0.8192
	filtered := LPF_ALPHA*clamped + (1.0-LPF_ALPHA)*a.LpfPrev
	a.LpfPrev = filtered
	const DC_R float32 = 0.99715
	dcOut := filtered - a.DcPrevX + DC_R*a.DcPrevY
	a.DcPrevX = filtered
	a.DcPrevY = dcOut
	return dcOut
}

func (a *Apu) scaledSample(idx int, raw uint8) float32 {
	if idx >= ChannelCount || a.ChannelMuted[idx] {
		return 0
	}
	return float32(raw) * a.ChannelVolumes[idx]
}

func (a *Apu) mixRaw() float32 {
	p1 := a.scaledSample(0, a.Pulse1.Sample())
	p2 := a.scaledSample(1, a.Pulse2.Sample())
	tri := a.scaledSample(2, a.Triangle.Sample())
	noise := a.scaledSample(3, a.Noise.Sample())
	dmc := a.scaledSample(4, a.Dmc.Sample())

	pulseSum := p1 + p2
	var pulseOut float32
	if pulseSum > 0 {
		pulseOut = 95.52 / (8128.0/pulseSum + 100.0)
	}
	tndInner := tri/8227.0 + noise/12241.0 + dmc/22638.0
	var tndOut float32
	if tndInner > 0 {
		tndOut = 163.67 / (1.0/tndInner + 100.0)
	}
	mixed := (pulseOut + tndOut) * 2.0 - 1.0
	if mixed < -1.0 {
		mixed = -1.0
	}
	if mixed > 1.0 {
		mixed = 1.0
	}
	return mixed
}

// SerializedSize returns the byte length of the APU state serialization.
func SerializedSize() int { return int(unsafe.Sizeof(Apu{})) }

// Serialize writes the APU state into buf (must be >= SerializedSize).
// Returns bytes written.
func Serialize(a *Apu, buf []byte) int {
	n := SerializedSize()
	if len(buf) < n {
		return 0
	}
	src := (*[1 << 30]byte)(unsafe.Pointer(a))[:n:n]
	copy(buf, src)
	return n
}

// Deserialize restores the APU state from buf. Returns bytes read.
func Deserialize(a *Apu, buf []byte) int {
	n := SerializedSize()
	if len(buf) < n {
		return 0
	}
	dst := (*[1 << 30]byte)(unsafe.Pointer(a))[:n:n]
	copy(dst, buf)
	return n
}
