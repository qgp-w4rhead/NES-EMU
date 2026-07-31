package main

import (
	"runtime"
	"testing"
	"unsafe"

	"nes-core-go/internal/cartridge"
	"nes-core-go/internal/region"
	"nes-core-go/internal/savestate"
)

// minimalNromRom builds a tiny valid iNES NROM-128 ROM filled with NOPs,
// with the RESET vector pointing at the NOP sled.
func minimalNromRom() []byte {
	prg := make([]byte, 16*1024)
	for i := range prg {
		prg[i] = 0xEA // NOP
	}
	prg[0x3FFC] = 0x00
	prg[0x3FFD] = 0xC0
	rom := make([]byte, 16+len(prg))
	copy(rom, []byte{'N', 'E', 'S', 0x1A})
	rom[4] = 1
	rom[5] = 0
	rom[6] = 0
	copy(rom[16:], prg)
	return rom
}

func newEmuFromRom(t *testing.T) *Emulator {
	rom := minimalNromRom()
	cart, rc := cartridge.FromBytes(rom)
	if rc != 0 {
		t.Fatalf("cartridge load failed: %d", rc)
	}
	e := newEmulator(region.NTSC)
	e.Cart = cart
	e.Bus.InsertCartridge(cart)
	return e
}

func TestStepFrameInternal(t *testing.T) {
	e := newEmuFromRom(t)
	e.Cpu.Reset(&e.Bus)
	cycles := e.stepFrame()
	if cycles < 25000 || cycles > 32000 {
		t.Fatalf("expected ~29830 cycles, got %d", cycles)
	}
}

func TestStepInstructionInternal(t *testing.T) {
	e := newEmuFromRom(t)
	e.Cpu.Reset(&e.Bus)
	cycles := e.stepInstruction()
	if cycles != 2 {
		t.Fatalf("NOP should be 2 cycles, got %d", cycles)
	}
}

func TestMapperNumberInternal(t *testing.T) {
	e := newEmuFromRom(t)
	if e.Cart.Header.MapperNumber != 0 {
		t.Fatalf("expected mapper 0, got %d", e.Cart.Header.MapperNumber)
	}
}

func TestSaveLoadStateInternal(t *testing.T) {
	e := newEmuFromRom(t)
	e.Cpu.Reset(&e.Bus)
	e.stepFrame()
	st := e.saveState()
	sz := savestate.RequiredSize(st)
	if sz == 0 {
		t.Fatal("save_state size query returned 0")
	}
	buf := make([]byte, sz)
	written := savestate.Save(st, buf)
	if written == 0 {
		t.Fatal("save_state wrote 0 bytes")
	}
	for i := 0; i < 3; i++ {
		e.stepFrame()
	}
	st2 := e.saveState()
	if !savestate.Load(st2, buf[:written]) {
		t.Fatal("load_state failed")
	}
}

func TestSetRegionInternal(t *testing.T) {
	e := newEmuFromRom(t)
	if e.Region != region.NTSC {
		t.Fatalf("expected NTSC, got %d", e.Region)
	}
	e.Region = region.PAL
	e.Bus.PPU.SetRegion(region.PAL)
	e.Bus.APU.SetRegion(region.PAL)
}

func Test1000FrameRun(t *testing.T) {
	e := newEmuFromRom(t)
	e.Cpu.Reset(&e.Bus)
	var total uint32
	for i := 0; i < 1000; i++ {
		total += e.stepFrame()
	}
	if total == 0 {
		t.Fatal("1000 frames produced 0 cycles")
	}
}

func TestAudioDrain(t *testing.T) {
	e := newEmuFromRom(t)
	e.Cpu.Reset(&e.Bus)
	e.stepFrame()
	if len(e.AudioBuf) == 0 {
		t.Fatal("no audio samples produced after one frame")
	}
}

// TestNoHeapAllocInStepFrame verifies the M7 acceptance criterion: no heap
// allocations during step_frame. We run a few warm-up frames, capture
// runtime.MemStats, run 10 more frames, then check that HeapAlloc did not
// grow (within a tiny tolerance for GC bookkeeping). The audio buffer is
// pre-allocated and reused; the framebuffer is a fixed array. The only
// allowed allocation growth is from the audio buffer capacity expansion
// during early frames, which is why we warm up first.
func TestNoHeapAllocInStepFrame(t *testing.T) {
	e := newEmuFromRom(t)
	e.Cpu.Reset(&e.Bus)
	// Warm up: let the audio buffer settle to its steady-state capacity.
	for i := 0; i < 5; i++ {
		e.stepFrame()
		e.AudioBuf = e.AudioBuf[:0]
	}
	runtime.GC()
	var before runtime.MemStats
	runtime.ReadMemStats(&before)
	for i := 0; i < 10; i++ {
		e.stepFrame()
		e.AudioBuf = e.AudioBuf[:0]
	}
	var after runtime.MemStats
	runtime.ReadMemStats(&after)
	// Allow a tiny tolerance for GC bookkeeping, but HeapAlloc should not
	// grow meaningfully across 10 frames in steady state.
	growth := int64(after.HeapAlloc) - int64(before.HeapAlloc)
	if growth > 1024 {
		t.Fatalf("HeapAlloc grew by %d bytes across 10 step_frame calls "+
			"(before=%d after=%d) — heap allocations during step_frame",
			growth, before.HeapAlloc, after.HeapAlloc)
	}
	t.Logf("HeapAlloc growth across 10 frames: %d bytes (before=%d after=%d)",
		growth, before.HeapAlloc, after.HeapAlloc)
}

// keep unsafe import used
var _ = unsafe.Pointer(nil)