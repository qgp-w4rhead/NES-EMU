"""Smoke tests for the Cython NES core (M11).

Run: python test/run_tests.py
Validates: step_frame, step_instruction, mapper_number, save/load roundtrip,
set_region, 1000-frame run, audio drain, framebuffer, reset, determinism.
"""

import sys
import os
import struct

# Add the parent directory to the path so we can import nes_core.
sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from nes_core._core import Emulator

# NOP ROM: NROM-128, mapper 0, 16KB PRG (0xEA NOP), 8KB CHR (0x00).
# RESET vector at $FFFC/$FFFD -> $C000.
def make_nop_rom():
    header = bytearray(16)
    header[0:4] = b"NES\x1A"
    header[4] = 1   # 1 PRG bank (16KB)
    header[5] = 1   # 1 CHR bank (8KB)
    header[6] = 0   # mapper 0, horizontal mirroring
    header[7] = 0
    prg = bytearray([0xEA] * 16384)  # NOP fill
    # Set RESET vector to $C000
    prg[0x3FFC] = 0x00
    prg[0x3FFD] = 0xC0
    chr_rom = bytearray([0x00] * 8192)
    return bytes(header + prg + chr_rom)

NOP_ROM = make_nop_rom()
FB_PIXELS = 256 * 240

passed = 0
failed = 0

def check(name, condition, detail=""):
    global passed, failed
    if condition:
        passed += 1
        print(f"  PASS: {name}")
    else:
        failed += 1
        print(f"  FAIL: {name} {detail}")

def test_step_frame():
    print("test_step_frame:")
    e = Emulator()
    assert e.py_load_rom(NOP_ROM), "loadRom failed"
    e.py_reset()
    cycles = e.py_step_frame()
    check("step_frame returns > 0 cycles", cycles > 0, f"(cycles={cycles})")
    # NOP ROM: ~29830 cycles per NTSC frame
    check("step_frame cycles in reasonable range", 25000 < cycles < 35000,
          f"(cycles={cycles})")

def test_step_instruction():
    print("test_step_instruction:")
    e = Emulator()
    assert e.py_load_rom(NOP_ROM), "loadRom failed"
    e.py_reset()
    cycles = e.py_step_instruction()
    check("step_instruction returns 2 for NOP", cycles == 2,
          f"(cycles={cycles})")

def test_mapper_number():
    print("test_mapper_number:")
    e = Emulator()
    e.py_load_rom(NOP_ROM)
    check("mapper_number == 0 for NROM", e.py_mapper_number() == 0,
          f"(mapper={e.py_mapper_number()})")

def test_save_load_roundtrip():
    print("test_save_load_roundtrip:")
    e = Emulator()
    assert e.py_load_rom(NOP_ROM), "loadRom failed"
    e.py_reset()
    e.py_step_frame()
    e.py_step_frame()
    state = e.py_save_state()
    check("save_state returns non-empty bytes", len(state) > 100,
          f"(len={len(state)})")
    # Run more frames to change state
    for _ in range(10):
        e.py_step_frame()
    # Load state back
    ok = e.py_load_state(state)
    check("load_state returns True", ok)
    # Verify state restored: step one more frame and compare cycles
    cycles_after_load = e.py_step_frame()
    # Create a fresh emulator, run 3 frames (same as before save)
    e2 = Emulator()
    e2.py_load_rom(NOP_ROM)
    e2.py_reset()
    e2.py_step_frame()
    e2.py_step_frame()
    cycles_ref = e2.py_step_frame()
    check("roundtrip: cycles match reference", cycles_after_load == cycles_ref,
          f"(loaded={cycles_after_load}, ref={cycles_ref})")

def test_set_region():
    print("test_set_region:")
    e = Emulator()
    e.py_load_rom(NOP_ROM)
    e.py_set_region(0)  # NTSC
    check("region set to NTSC", e.py_get_region() == 0)
    e.py_set_region(1)  # PAL
    check("region set to PAL", e.py_get_region() == 1)
    e.py_set_region(2)  # Dendy
    check("region set to Dendy", e.py_get_region() == 2)

def test_1000_frame_run():
    print("test_1000_frame_run:")
    e = Emulator()
    assert e.py_load_rom(NOP_ROM), "loadRom failed"
    e.py_reset()
    total = 0
    for _ in range(1000):
        total += e.py_step_frame()
    check("1000 frames complete without crash", total > 0,
          f"(total_cycles={total})")

def test_audio_drain():
    print("test_audio_drain:")
    e = Emulator()
    assert e.py_load_rom(NOP_ROM), "loadRom failed"
    e.py_reset()
    e.py_step_frame()
    audio = e.py_take_audio(8192)
    check("audio drain returns bytes", len(audio) > 0,
          f"(len={len(audio)})")
    check("audio is int16 samples (even length)", len(audio) % 2 == 0)

def test_framebuffer():
    print("test_framebuffer:")
    e = Emulator()
    assert e.py_load_rom(NOP_ROM), "loadRom failed"
    e.py_reset()
    e.py_step_frame()
    fb = e.py_framebuffer_bytes()
    check("framebuffer is 245760 bytes", len(fb) == FB_PIXELS * 4,
          f"(len={len(fb)})")
    # After a frame, framebuffer should have some non-zero pixels (palette).
    # With NOP ROM (no rendering enabled by default), it may be all the
    # universal background color. Check it's not all zero.
    check("framebuffer not all zero", any(b != 0 for b in fb[:1024]))

def test_reset():
    print("test_reset:")
    e = Emulator()
    assert e.py_load_rom(NOP_ROM), "loadRom failed"
    e.py_reset()
    c1 = e.py_step_instruction()
    e.py_reset()
    c2 = e.py_step_instruction()
    check("reset is deterministic (same cycles after reset)", c1 == c2,
          f"(c1={c1}, c2={c2})")

def test_determinism():
    print("test_determinism:")
    e1 = Emulator()
    e1.py_load_rom(NOP_ROM)
    e1.py_reset()
    e2 = Emulator()
    e2.py_load_rom(NOP_ROM)
    e2.py_reset()
    match = True
    for _ in range(100):
        c1 = e1.py_step_frame()
        c2 = e2.py_step_frame()
        if c1 != c2:
            match = False
            break
    check("100 frames deterministic across instances", match)

def test_no_python_object_creation():
    """Verify step_frame doesn't create Python objects (M11 acceptance).

    Uses sys.getrefcount as a proxy — in steady state, the refcount delta
    after a frame should be ~0 (allowing for the test harness itself).
    """
    print("test_no_python_object_creation:")
    import sys as _sys
    e = Emulator()
    assert e.py_load_rom(NOP_ROM), "loadRom failed"
    e.py_reset()
    # Warm up
    for _ in range(10):
        e.py_step_frame()
    # Measure refcount delta over 100 frames
    rc_before = _sys.getrefcount(e)
    for _ in range(100):
        e.py_step_frame()
    rc_after = _sys.getrefcount(e)
    delta = rc_after - rc_before
    check("no refcount growth over 100 frames", delta == 0,
          f"(delta={delta})")


if __name__ == "__main__":
    print("=== Cython NES Core Tests (M11) ===")
    print()
    test_step_frame()
    test_step_instruction()
    test_mapper_number()
    test_save_load_roundtrip()
    test_set_region()
    test_1000_frame_run()
    test_audio_drain()
    test_framebuffer()
    test_reset()
    test_determinism()
    test_no_python_object_creation()
    print()
    print(f"=== Results: {passed} passed, {failed} failed ===")
    if failed > 0:
        sys.exit(1)
