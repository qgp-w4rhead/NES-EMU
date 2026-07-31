// Package joypad implements the NES standard controller input
// (port of cores/c/src/joypad.c).
package joypad

import "unsafe"

const (
	ControllerCount = 2
	ButtonCount     = 8
)

// Button bit indices.
const (
	ButtonA      = 0
	ButtonB      = 1
	ButtonSelect = 2
	ButtonStart  = 3
	ButtonUp     = 4
	ButtonDown   = 5
	ButtonLeft   = 6
	ButtonRight  = 7
)

// Joypad is the NES joypad state for two standard controllers.
type Joypad struct {
	Current [ControllerCount]uint8
	Strobe  bool
	Shift   [ControllerCount]uint8
	Counter [ControllerCount]uint8
}

// Init constructs a joypad with no buttons pressed and the strobe low.
func (j *Joypad) Init() {
	*j = Joypad{}
}

// SetButton sets or clears a button on the given controller.
func (j *Joypad) SetButton(controller, button uint8, pressed bool) {
	if controller >= ControllerCount || button >= ButtonCount {
		return
	}
	mask := uint8(1 << button)
	if pressed {
		j.Current[controller] |= mask
	} else {
		j.Current[controller] &= ^mask
	}
	if j.Strobe {
		j.Shift[controller] = j.Current[controller]
	}
}

// WriteStrobe writes to $4016 bit 0 - the controller strobe line.
func (j *Joypad) WriteStrobe(value uint8) {
	newStrobe := (value & 0x01) != 0
	if j.Strobe && !newStrobe {
		for c := uint8(0); c < ControllerCount; c++ {
			j.Shift[c] = j.Current[c]
			j.Counter[c] = 0
		}
	}
	j.Strobe = newStrobe
	if j.Strobe {
		for c := uint8(0); c < ControllerCount; c++ {
			j.Shift[c] = j.Current[c]
		}
	}
}

// Read reads one bit from the given controller.
func (j *Joypad) Read(controller uint8) uint8 {
	if controller >= ControllerCount {
		return 1
	}
	if j.Strobe {
		return j.Current[controller] & 0x01
	}
	var bit uint8
	if j.Counter[controller] < ButtonCount {
		bit = (j.Shift[controller] >> j.Counter[controller]) & 0x01
	} else {
		bit = 1
	}
	if j.Counter[controller] < 0xFF {
		j.Counter[controller]++
	}
	return bit
}

// StrobeState returns the current strobe line state.
func (j *Joypad) StrobeState() bool { return j.Strobe }

// Clear clears all buttons on both controllers.
func (j *Joypad) Clear() {
	for c := uint8(0); c < ControllerCount; c++ {
		j.Current[c] = 0
		if j.Strobe {
			j.Shift[c] = 0
		}
	}
}

// CurrentState returns live button state for a controller.
func (j *Joypad) CurrentState(controller uint8) uint8 {
	if controller >= ControllerCount {
		return 0
	}
	return j.Current[controller]
}

// ShiftState returns the frozen snapshot for a controller.
func (j *Joypad) ShiftState(controller uint8) uint8 {
	if controller >= ControllerCount {
		return 0
	}
	return j.Shift[controller]
}

// SerializedSize returns the byte length of the joypad state serialization.
func SerializedSize() int { return int(unsafe.Sizeof(Joypad{})) }

// Serialize writes the joypad state into buf (must be >= SerializedSize).
// Returns bytes written.
func Serialize(j *Joypad, buf []byte) int {
	n := SerializedSize()
	if len(buf) < n {
		return 0
	}
	src := (*[1 << 30]byte)(unsafe.Pointer(j))[:n:n]
	copy(buf, src)
	return n
}

// Deserialize restores the joypad state from buf. Returns bytes read.
func Deserialize(j *Joypad, buf []byte) int {
	n := SerializedSize()
	if len(buf) < n {
		return 0
	}
	dst := (*[1 << 30]byte)(unsafe.Pointer(j))[:n:n]
	copy(dst, buf)
	return n
}
