// Package main is the c-shared DLL entry point: exposes all 13 protocol.h
// symbols via //export. Port of cores/c/src/emulator.c + core_api.c.
package main

/*
#include <stdint.h>
#include <stddef.h>
#include <stdlib.h>
*/
import "C"

import (
	"sync"
	"unsafe"

	"nes-core-go/internal/bus"
	"nes-core-go/internal/cartridge"
	"nes-core-go/internal/cpu"
	"nes-core-go/internal/ppu"
	"nes-core-go/internal/region"
	"nes-core-go/internal/savestate"
)

// Emulator is the complete NES emulator state (port of C EmulatorState).
type Emulator struct {
	Cpu       cpu.Cpu
	Bus       bus.Bus
	Cart      *cartridge.Cartridge
	Region    region.Region
	SampleAcc float32
	AudioBuf  []float32
	PpuCarry  uint32
	// cFramebuffer is a C-allocated copy of the PPU framebuffer, used so
	// nes_core_framebuffer can return a C pointer (Go pointers are rejected
	// by the cgo pointer checker when returned to C).
	cFramebuffer unsafe.Pointer
}

const (
	audioCapNTSC = 800
	audioCapPAL  = 950
)

// ---- Handle registry ----
//
// The C ABI handle is an integer wrapped in unsafe.Pointer. The actual
// *Emulator lives in a Go map so the GC can see it. This avoids the cgo
// pointer-checker panic (Go pointers returned to C) and keeps Go-allocated
// objects (Cartridge, slices) alive.

var (
	handleNext uintptr
	handleMu   sync.Mutex
	handles    = make(map[uintptr]*Emulator)
)

func registerEmulator(e *Emulator) unsafe.Pointer {
	handleMu.Lock()
	defer handleMu.Unlock()
	handleNext++
	h := handleNext
	handles[h] = e
	return unsafe.Pointer(h)
}

func lookupEmulator(handle unsafe.Pointer) *Emulator {
	if handle == nil {
		return nil
	}
	handleMu.Lock()
	defer handleMu.Unlock()
	return handles[uintptr(handle)]
}

func unregisterEmulator(handle unsafe.Pointer) {
	if handle == nil {
		return
	}
	handleMu.Lock()
	defer handleMu.Unlock()
	delete(handles, uintptr(handle))
}

func newEmulator(r region.Region) *Emulator {
	e := &Emulator{Region: r}
	e.Cpu.Init()
	e.Bus.Init()
	e.Bus.PPU.SetRegion(r)
	e.Bus.APU.SetRegion(r)
	cap := audioCapNTSC
	if region.ScanlinesPerFrame(r) > 262 {
		cap = audioCapPAL
	}
	e.AudioBuf = make([]float32, 0, cap)
	return e
}

func (e *Emulator) pushAudio(sample float32) {
	e.AudioBuf = append(e.AudioBuf, sample)
}

func (e *Emulator) stepOneCpuTick(prevScanline uint16, cyclesPerSample float32, prerender uint16) (uint32, bool) {
	stepCycles := e.Cpu.Step(&e.Bus)
	cpuCycles := uint32(stepCycles)
	dmaCycles := e.Bus.TakeDmaStallCycles()
	cpuCycles += dmaCycles
	e.Bus.AdvanceCPUCycles(uint32(stepCycles) + dmaCycles)

	apuCycles := uint32(stepCycles) + dmaCycles
	e.Bus.StepAPU(apuCycles)
	e.Bus.ClockCartCPU(apuCycles)

	if e.Bus.ApuIrqPending() {
		e.Cpu.SetIrqPending(true)
	}
	if e.Bus.CartIrqPending() {
		e.Cpu.SetIrqPending(true)
	}

	e.SampleAcc += float32(apuCycles)
	for e.SampleAcc >= cyclesPerSample {
		e.SampleAcc -= cyclesPerSample
		internal := e.Bus.APU.Output()
		expansion := e.Bus.ExpansionAudioSample()
		mixed := internal + expansion
		if mixed < -1.0 {
			mixed = -1.0
		}
		if mixed > 1.0 {
			mixed = 1.0
		}
		e.pushAudio(mixed)
	}

	ppuCycles := 3*apuCycles + e.PpuCarry
	e.PpuCarry = 0
	remaining := ppuCycles
	frameDone := false
	const cyclesPerScanline = uint32(341)
	for remaining > 0 {
		chunk := remaining
		if chunk > cyclesPerScanline {
			chunk = cyclesPerScanline
		}
		e.Bus.StepPPU(chunk)
		remaining -= chunk
		if e.Bus.TakeNmiRequest() {
			e.Cpu.SetNmiPending(true)
		}
		currScanline := e.Bus.PPU.Scanline
		if currScanline == 0 && prevScanline == prerender {
			e.PpuCarry = remaining
			frameDone = true
			break
		}
	}
	return cpuCycles, frameDone
}

func (e *Emulator) stepFrame() uint32 {
	ub := e.Bus.PPU.UniversalBgArgb()
	e.Bus.PPU.ClearFramebuffer(ub)
	e.Bus.PPU.ResetRenderedFlag()
	cyclesPerSample := region.CPUCyclesPerSample(e.Region)
	prerender := region.ScanlinePrerender(e.Region)
	var total uint32
	for {
		prevScanline := e.Bus.PPU.Scanline
		cycles, done := e.stepOneCpuTick(prevScanline, cyclesPerSample, prerender)
		total += cycles
		if done {
			break
		}
	}
	if !e.Bus.PPU.RenderedThisFrame() {
		e.Bus.RenderFrame()
	}
	e.syncFramebuffer()
	return total
}

// syncFramebuffer copies the Go PPU framebuffer to the C-allocated buffer.
func (e *Emulator) syncFramebuffer() {
	if e.cFramebuffer == nil {
		return
	}
	dst := unsafe.Slice((*uint32)(e.cFramebuffer), ppu.FramebufferSize)
	copy(dst, e.Bus.PPU.Framebuffer[:])
}

func (e *Emulator) stepInstruction() uint32 {
	prevScanline := e.Bus.PPU.Scanline
	cyclesPerSample := region.CPUCyclesPerSample(e.Region)
	prerender := region.ScanlinePrerender(e.Region)
	cycles, _ := e.stepOneCpuTick(prevScanline, cyclesPerSample, prerender)
	// Keep the C-allocated framebuffer in sync so nes_core_framebuffer
	// returns current data even after single-instruction steps.
	e.syncFramebuffer()
	return cycles
}

func (e *Emulator) saveState() *savestate.State {
	return &savestate.State{
		Cpu:           &e.Cpu,
		Bus:           &e.Bus,
		Cartridge:     e.Cart,
		Region:        e.Region,
		SampleAcc:     e.SampleAcc,
		AudioBuffer:   e.AudioBuf,
		PpuCycleCarry: e.PpuCarry,
	}
}

// ---- C ABI exports (handle = integer wrapped in unsafe.Pointer) ----

//export nes_core_create
func nes_core_create(romData *byte, romLen C.size_t) unsafe.Pointer {
	if romData == nil || romLen == 0 {
		return nil
	}
	buf := unsafe.Slice(romData, int(romLen))
	cart, rc := cartridge.FromBytes(buf)
	if rc != 0 {
		return nil
	}
	e := newEmulator(region.NTSC)
	e.Cart = cart
	e.Bus.InsertCartridge(cart)
	// Allocate a zeroed C-heap framebuffer so its address can be returned
	// to C. calloc (not malloc) ensures deterministic bytes if the harness
	// reads the framebuffer before the first step_frame call.
	fbSize := C.size_t(ppu.FramebufferSize * 4)
	e.cFramebuffer = unsafe.Pointer(C.calloc(fbSize, 1))
	if e.cFramebuffer == nil {
		return nil
	}
	return registerEmulator(e)
}

//export nes_core_destroy
func nes_core_destroy(handle unsafe.Pointer) {
	if handle == nil {
		return
	}
	handleMu.Lock()
	e, ok := handles[uintptr(handle)]
	if ok {
		delete(handles, uintptr(handle))
	}
	handleMu.Unlock()
	if e != nil {
		if e.cFramebuffer != nil {
			C.free(e.cFramebuffer)
			e.cFramebuffer = nil
		}
	}
}

//export nes_core_reset
func nes_core_reset(handle unsafe.Pointer) {
	e := lookupEmulator(handle)
	if e == nil {
		return
	}
	e.Cpu.Reset(&e.Bus)
}

//export nes_core_set_region
func nes_core_set_region(handle unsafe.Pointer, r C.int) C.int {
	if handle == nil || r < 0 || r > C.int(region.DENDY) {
		return -1
	}
	e := lookupEmulator(handle)
	if e == nil {
		return -1
	}
	prev := C.int(e.Region)
	e.Region = region.Region(r)
	e.Bus.PPU.SetRegion(e.Region)
	e.Bus.APU.SetRegion(e.Region)
	return prev
}

//export nes_core_step_frame
func nes_core_step_frame(handle unsafe.Pointer) C.uint32_t {
	e := lookupEmulator(handle)
	if e == nil {
		return 0
	}
	return C.uint32_t(e.stepFrame())
}

//export nes_core_step_instruction
func nes_core_step_instruction(handle unsafe.Pointer) C.uint32_t {
	e := lookupEmulator(handle)
	if e == nil {
		return 0
	}
	return C.uint32_t(e.stepInstruction())
}

//export nes_core_framebuffer
func nes_core_framebuffer(handle unsafe.Pointer) *C.uint32_t {
	e := lookupEmulator(handle)
	if e == nil || e.cFramebuffer == nil {
		return nil
	}
	return (*C.uint32_t)(e.cFramebuffer)
}

//export nes_core_take_audio
func nes_core_take_audio(handle unsafe.Pointer, outBuf *C.int16_t, cap C.size_t) C.size_t {
	if outBuf == nil || cap == 0 {
		return 0
	}
	e := lookupEmulator(handle)
	if e == nil {
		return 0
	}
	n := C.size_t(len(e.AudioBuf))
	if n > cap {
		n = cap
	}
	out := unsafe.Slice(outBuf, int(cap))
	for i := C.size_t(0); i < n; i++ {
		s := e.AudioBuf[i]
		if s > 1.0 {
			s = 1.0
		}
		if s < -1.0 {
			s = -1.0
		}
		// Asymmetric saturation matching C core_api.c: positive *32767,
		// negative *32768, so -1.0 maps to -32768 (full-scale negative).
		var v int32
		if s >= 0.0 {
			v = int32(s * 32767.0)
			if v > 32767 {
				v = 32767
			}
		} else {
			v = int32(s * 32768.0)
			if v < -32768 {
				v = -32768
			}
		}
		out[i] = C.int16_t(v)
	}
	// Reset the audio buffer to length 0, reusing the same backing array.
	// This avoids heap allocations on subsequent append in pushAudio (the
	// C reference does audio_buffer_count = 0 in emulator.c:240). Using
	// e.AudioBuf[n:] would advance the slice window and eventually force
	// a reallocation when append exceeds the remaining capacity.
	e.AudioBuf = e.AudioBuf[:0]
	return n
}

//export nes_core_save_state
func nes_core_save_state(handle unsafe.Pointer, outBuf *byte, cap C.size_t) C.size_t {
	// Match the C reference (core_api.c:144): NULL/zero-cap buffer is an
	// error, not a size query. Callers allocate a generously-sized buffer.
	if outBuf == nil || cap == 0 {
		return 0
	}
	e := lookupEmulator(handle)
	if e == nil {
		return 0
	}
	st := e.saveState()
	required := C.size_t(savestate.RequiredSize(st))
	if cap < required {
		return 0
	}
	buf := unsafe.Slice(outBuf, int(cap))
	written := savestate.Save(st, buf)
	return C.size_t(written)
}

//export nes_core_load_state
func nes_core_load_state(handle unsafe.Pointer, inBuf *byte, length C.size_t) C.int {
	if inBuf == nil || length == 0 {
		return 0
	}
	e := lookupEmulator(handle)
	if e == nil {
		return 0
	}
	buf := unsafe.Slice(inBuf, int(length))
	st := e.saveState()
	if savestate.Load(st, buf) {
		e.SampleAcc = st.SampleAcc
		e.PpuCarry = st.PpuCycleCarry
		e.AudioBuf = st.AudioBuffer
		e.Region = st.Region
		return 1
	}
	return 0
}

//export nes_core_mapper_number
func nes_core_mapper_number(handle unsafe.Pointer) C.uint16_t {
	e := lookupEmulator(handle)
	if e == nil || e.Cart == nil {
		return 0
	}
	return C.uint16_t(e.Cart.Header.MapperNumber)
}

//export nes_core_impl_name
func nes_core_impl_name() *C.char {
	return (*C.char)(unsafe.Pointer(cImplName))
}

//export nes_core_impl_version
func nes_core_impl_version() *C.char {
	return (*C.char)(unsafe.Pointer(cImplVersion))
}

// C-allocated static strings (Go globals are Go pointers; cgo rejects them).
var (
	cImplName    = C.CString("go-nes")
	cImplVersion = C.CString("0.1.0")
)

func main() {}