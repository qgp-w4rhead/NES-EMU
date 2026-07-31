// Package region implements TV system / region timing (port of cores/c/src/region.c).
package region

// Region is the TV system / region enum (matches C Region).
type Region int

const (
	NTSC  Region = 0 // REGION_NTSC
	PAL   Region = 1 // REGION_PAL
	DENDY Region = 2 // REGION_DENDY
)

// ApuFrameThreshold is a frame-counter threshold (cycle position + quarter/half flags).
type ApuFrameThreshold struct {
	Threshold uint32
	Quarter   bool
	Half      bool
}

// ScanlinesPerFrame returns total scanlines per frame. NTSC=262, PAL/Dendy=312.
func ScanlinesPerFrame(r Region) uint16 {
	switch r {
	case NTSC:
		return 262
	case PAL:
		return 312
	case DENDY:
		return 312
	default:
		return 262
	}
}

// ScanlinePrerender returns the prerender scanline index. NTSC=261, PAL/Dendy=311.
func ScanlinePrerender(r Region) uint16 {
	switch r {
	case NTSC:
		return 261
	case PAL:
		return 311
	case DENDY:
		return 311
	default:
		return 261
	}
}

// IsPalPalette returns true only for REGION_PAL (Dendy uses NTSC palette).
func IsPalPalette(r Region) bool {
	return r == PAL
}

// CPUClockHz returns the CPU clock rate in Hz.
func CPUClockHz(r Region) float32 {
	switch r {
	case NTSC:
		return 1789773.0
	case PAL:
		return 1662607.0
	case DENDY:
		return 1789773.0
	default:
		return 1789773.0
	}
}

// CPUCyclesPerSample returns CPU cycles per audio sample at 44.1 kHz.
func CPUCyclesPerSample(r Region) float32 {
	return CPUClockHz(r) / 44100.0
}

// APU4StepThresholds returns the 4-step mode frame-counter thresholds.
func APU4StepThresholds(r Region) [4]ApuFrameThreshold {
	ntsc := [4]uint32{7457, 14913, 22371, 29828}
	pal := [4]uint32{8314, 16627, 24941, 33255}
	quarter := [4]bool{true, true, true, true}
	half := [4]bool{false, true, false, true}
	var t [4]uint32
	if r == PAL {
		t = pal
	} else {
		t = ntsc
	}
	var out [4]ApuFrameThreshold
	for i := 0; i < 4; i++ {
		out[i] = ApuFrameThreshold{Threshold: t[i], Quarter: quarter[i], Half: half[i]}
	}
	return out
}

// APU5StepThresholds returns the 5-step mode frame-counter thresholds.
func APU5StepThresholds(r Region) [4]ApuFrameThreshold {
	ntsc := [4]uint32{7457, 14913, 22371, 37281}
	pal := [4]uint32{8314, 16627, 24941, 41568}
	quarter := [4]bool{true, true, true, true}
	half := [4]bool{false, true, false, true}
	var t [4]uint32
	if r == PAL {
		t = pal
	} else {
		t = ntsc
	}
	var out [4]ApuFrameThreshold
	for i := 0; i < 4; i++ {
		out[i] = ApuFrameThreshold{Threshold: t[i], Quarter: quarter[i], Half: half[i]}
	}
	return out
}

// APUResetAt returns the frame-counter reset point in CPU cycles.
func APUResetAt(r Region, mode5step bool) uint32 {
	switch r {
	case NTSC, DENDY:
		if mode5step {
			return 37282
		}
		return 29830
	case PAL:
		if mode5step {
			return 41570
		}
		return 33257
	default:
		if mode5step {
			return 37282
		}
		return 29830
	}
}

// APU4StepIRQThreshold returns the 4-step mode IRQ threshold.
func APU4StepIRQThreshold(r Region) uint32 {
	switch r {
	case NTSC, DENDY:
		return 29828
	case PAL:
		return 33255
	default:
		return 29828
	}
}
