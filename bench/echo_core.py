#!/usr/bin/env python3
"""bench/echo_core.py — minimal subprocess core for the M3 protocol echo test.

Implements just enough of the binary protocol from tasklist/M3.md to let
the harness's `--subprocess-test` verify framing end-to-end. This is NOT a
real NES core — it echoes back canned responses:

  INIT        -> OK
  RESET       -> OK
  STEP_FRAME  -> RESULT (cycles=29830, empty fb/audio)
  STEP_INSTR  -> RESULT (cycles=2, empty fb/audio)
  SAVE_STATE  -> SAVE_RESULT (a 4-byte sentinel blob)
  LOAD_STATE  -> OK

All integers are little-endian, matching the harness's put_u32_le framing.
Run the echo test with:

  bench\\harness.exe --subprocess-test "python bench\\echo_core.py"
"""
import os
import struct
import sys

# Opcodes (harness -> core).
MSG_INIT       = 0x01
MSG_RESET      = 0x02
MSG_STEP_FRAME = 0x03
MSG_STEP_INSTR = 0x04
MSG_SAVE_STATE = 0x05
MSG_LOAD_STATE = 0x06

# Response tags (core -> harness).
OK = b"\x00"

# Sentinel save blob returned by SAVE_STATE.
SAVE_BLOB = b"SAVE"


def read_exact(n):
    """Read exactly n bytes from stdin (binary). Returns None on EOF."""
    buf = b""
    while len(buf) < n:
        chunk = sys.stdin.buffer.read(n - len(buf))
        if not chunk:
            return None
        buf += chunk
    return buf


def send(data):
    """Write bytes to stdout (binary) and flush."""
    sys.stdout.buffer.write(data)
    sys.stdout.buffer.flush()


def send_result(cycles, fb_len=0, fb=b"", audio_len=0, audio=b""):
    """RESULT: [cycles:u32 LE][fb_len:u32 LE][fb][audio_len:u32 LE][audio]."""
    send(struct.pack("<III", cycles, fb_len, audio_len) + fb + audio)


def send_save_result(blob):
    """SAVE_RESULT: [len:u32 LE][bytes]."""
    send(struct.pack("<I", len(blob)) + blob)


def main():
    # Binary mode on Windows: ensure stdin/stdout are not translated.
    try:
        sys.stdin.buffer.raw  # touch to force binary layer
    except Exception:
        pass
    if hasattr(os, "setmode"):
        try:
            os.setmode(0, os.O_BINARY)
            os.setmode(1, os.O_BINARY)
        except Exception:
            pass

    while True:
        op = read_exact(1)
        if op is None:
            return 0  # harness closed stdin -> exit cleanly
        op = op[0]

        if op == MSG_INIT:
            hdr = read_exact(4)
            if hdr is None:
                return 1
            (rom_len,) = struct.unpack("<I", hdr)
            if rom_len:
                if read_exact(rom_len) is None:
                    return 1
            send(OK)

        elif op == MSG_RESET:
            send(OK)

        elif op == MSG_STEP_FRAME:
            hdr = read_exact(4)
            if hdr is None:
                return 1
            (count,) = struct.unpack("<I", hdr)
            # Echo one RESULT per batch; the harness sends count=1 for the
            # echo test, so a single RESULT suffices.
            send_result(cycles=29830 * count, fb_len=0, audio_len=0)

        elif op == MSG_STEP_INSTR:
            hdr = read_exact(4)
            if hdr is None:
                return 1
            (count,) = struct.unpack("<I", hdr)
            send_result(cycles=2 * count, fb_len=0, audio_len=0)

        elif op == MSG_SAVE_STATE:
            send_save_result(SAVE_BLOB)

        elif op == MSG_LOAD_STATE:
            hdr = read_exact(4)
            if hdr is None:
                return 1
            (blen,) = struct.unpack("<I", hdr)
            if blen:
                if read_exact(blen) is None:
                    return 1
            send(OK)

        else:
            # Unknown opcode -> ERROR [0xFF][msg_len:u16 LE][msg].
            msg = f"unknown opcode 0x{op:02x}".encode()
            send(b"\xFF" + struct.pack("<H", len(msg)) + msg)
            return 1


if __name__ == "__main__":
    sys.exit(main())
