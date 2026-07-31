"""M3 subprocess binary protocol loop for the Cython NES core.

Reads binary messages from stdin and writes responses to stdout in binary
mode. Messages (little-endian):

  INIT:        [0x01][rom_len:u32][rom_bytes]            -> OK / ERROR
  RESET:       [0x02]                                    -> OK / ERROR
  STEP_FRAME:  [0x03][count:u32]                         -> RESULT
  STEP_INSTR:  [0x04][count:u32]                         -> RESULT
  SAVE_STATE:  [0x05]                                    -> SAVE_RESULT
  LOAD_STATE:  [0x06][len:u32][bytes]                    -> OK / ERROR

Responses:
  OK:          [0x00]
  ERROR:       [0xFF][msg_len:u16][msg]
  RESULT:      [cycles:u32][fb_len:u32][fb bytes][audio_len:u32][audio int16 bytes]
  SAVE_RESULT: [len:u32][bytes]
"""

import sys
import os
import struct

from nes_core._core import Emulator

# Framebuffer = 256*240 ARGB uint32 = 245760 bytes.
FB_PIXELS = 256 * 240
FB_BYTES = FB_PIXELS * 4
# Audio drain cap per batch (NTSC ~735/frame, PAL ~882/frame; 100-frame batch
# needs ~73500 — use 128*1024 for headroom).
AUDIO_DRAIN_CAP = 128 * 1024


def main():
    # Use os.read for stdin (raw single-syscall read that returns as soon as
    # any data is available — sys.stdin.buffer.read blocks until the requested
    # number of bytes or EOF, which deadlocks the request/response protocol).
    stdin_fd = 0
    stdout = sys.stdout.buffer

    emu = None
    buf = bytearray()

    while True:
        chunk = os.read(stdin_fd, 65536)
        if not chunk:
            break
        buf.extend(chunk)

        # Parse as many complete messages as possible.
        while True:
            if not buf:
                break
            op = buf[0]

            if op == 0x01:  # INIT
                if len(buf) < 5:
                    break
                rom_len = struct.unpack_from("<I", buf, 1)[0]
                if len(buf) < 5 + rom_len:
                    break
                rom = bytes(buf[5:5 + rom_len])
                del buf[:5 + rom_len]
                emu = Emulator()
                if not emu.py_load_rom(rom):
                    emu = None
                    _send_error(stdout, "loadRom failed: invalid iNES image")
                    continue
                _send_ok(stdout)

            elif op == 0x02:  # RESET
                if len(buf) < 1:
                    break
                del buf[:1]
                if emu is None:
                    _send_error(stdout, "no emulator")
                    continue
                emu.py_reset()
                _send_ok(stdout)

            elif op == 0x03:  # STEP_FRAME
                if len(buf) < 5:
                    break
                count = struct.unpack_from("<I", buf, 1)[0]
                del buf[:5]
                if emu is None:
                    _send_error(stdout, "no emulator")
                    continue
                total_cycles = 0
                audio_parts = []
                for _ in range(count):
                    total_cycles += emu.py_step_frame()
                    audio_bytes = emu.py_take_audio(AUDIO_DRAIN_CAP)
                    if audio_bytes:
                        audio_parts.append(audio_bytes)
                fb = emu.py_framebuffer_bytes()
                audio = b"".join(audio_parts)
                _send_result(stdout, total_cycles, fb, audio)

            elif op == 0x04:  # STEP_INSTR
                if len(buf) < 5:
                    break
                count = struct.unpack_from("<I", buf, 1)[0]
                del buf[:5]
                if emu is None:
                    _send_error(stdout, "no emulator")
                    continue
                total_cycles = 0
                audio_parts = []
                for _ in range(count):
                    total_cycles += emu.py_step_instruction()
                    audio_bytes = emu.py_take_audio(AUDIO_DRAIN_CAP)
                    if audio_bytes:
                        audio_parts.append(audio_bytes)
                fb = emu.py_framebuffer_bytes()
                audio = b"".join(audio_parts)
                _send_result(stdout, total_cycles, fb, audio)

            elif op == 0x05:  # SAVE_STATE
                if len(buf) < 1:
                    break
                del buf[:1]
                if emu is None:
                    _send_error(stdout, "no emulator")
                    continue
                data = emu.py_save_state()
                stdout.write(struct.pack("<I", len(data)))
                stdout.write(data)
                stdout.flush()

            elif op == 0x06:  # LOAD_STATE
                if len(buf) < 5:
                    break
                state_len = struct.unpack_from("<I", buf, 1)[0]
                if len(buf) < 5 + state_len:
                    break
                data = bytes(buf[5:5 + state_len])
                del buf[:5 + state_len]
                if emu is None:
                    _send_error(stdout, "no emulator")
                    continue
                if emu.py_load_state(data):
                    _send_ok(stdout)
                else:
                    _send_error(stdout, "loadState failed: bad magic/version/size")

            else:
                _send_error(stdout, "unknown opcode 0x%02x" % op)
                sys.exit(1)


def _send_ok(stdout):
    stdout.write(b"\x00")
    stdout.flush()


def _send_error(stdout, msg):
    msg_bytes = msg.encode("utf-8")
    stdout.write(b"\xff")
    stdout.write(struct.pack("<H", len(msg_bytes)))
    stdout.write(msg_bytes)
    stdout.flush()


def _send_result(stdout, cycles, fb, audio):
    stdout.write(struct.pack("<I", cycles & 0xFFFFFFFF))
    stdout.write(struct.pack("<I", FB_PIXELS))
    stdout.write(struct.pack("<I", len(audio) // 2))
    stdout.write(fb)
    stdout.write(audio)
    stdout.flush()
