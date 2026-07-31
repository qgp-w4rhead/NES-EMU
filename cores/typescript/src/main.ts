// Subprocess binary protocol entry point (M3 spec). Compiled to dist/main.js
// and spawned by the harness via `node dist/main.js`. Reads binary messages
// from stdin and writes responses to stdout in BINARY mode.
//
// Messages harness -> core (little-endian):
//   INIT:         [0x01][rom_len:u32 LE][rom_bytes...]
//   RESET:        [0x02]
//   STEP_FRAME:   [0x03][count:u32 LE]   — run `count` frames, ONE RESULT back
//   STEP_INSTR:   [0x04][count:u32 LE]   — run `count` instrs, ONE RESULT back
//   SAVE_STATE:   [0x05]
//   LOAD_STATE:   [0x06][len:u32 LE][bytes...]
//
// Responses core -> harness (distinguished by context):
//   After INIT/RESET/LOAD_STATE: OK = [0x00], or ERROR = [0xFF][msg_len:u16 LE][msg]
//   After STEP_FRAME/STEP_INSTR: RESULT = [cycles:u32 LE][fb_len:u32 LE][fb bytes]
//                                [audio_len:u32 LE][audio int16 bytes]
//   After SAVE_STATE:            [len:u32 LE][bytes...]

import { Emulator } from './emulator';

const stdin = process.stdin;
const stdout = process.stdout;

let emu: Emulator | null = null;
let buf: Buffer = Buffer.alloc(0);

// Pre-allocated audio drain buffer for STEP_FRAME/STEP_INSTR batches. Grows
// if a batch produces more samples than the cap. Allocated once here, outside
// the per-message hot path where possible.
const FB_PIXELS = 256 * 240;
const AUDIO_DRAIN_CAP = 16384;
let audioDrainBuf: Int16Array = new Int16Array(AUDIO_DRAIN_CAP);

stdin.on('data', (chunk: Buffer) => {
    buf = Buffer.concat([buf, chunk]);
    try {
        tryParse();
    } catch (e: any) {
        sendError('exception: ' + (e && e.message ? e.message : String(e)));
        process.exit(1);
    }
});

stdin.on('end', () => { process.exit(0); });
stdin.on('error', () => { process.exit(1); });

function ensureAudioCap(needed: number, validLen: number): void {
    if (needed <= audioDrainBuf.length) return;
    let cap = audioDrainBuf.length;
    while (cap < needed) cap *= 2;
    const nb = new Int16Array(cap);
    // Copy previously drained samples so batched STEP_FRAME doesn't lose audio.
    nb.set(audioDrainBuf.subarray(0, validLen));
    audioDrainBuf = nb;
}

function tryParse(): void {
    while (true) {
        if (buf.length < 1) return;
        const op = buf[0];

        if (op === 0x01) { // INIT
            if (buf.length < 5) return;
            const romLen = buf.readUInt32LE(1);
            if (buf.length < 5 + romLen) return;
            const rom = buf.subarray(5, 5 + romLen);
            emu = new Emulator();
            if (!emu.loadRom(new Uint8Array(rom))) {
                emu = null;
                buf = buf.subarray(5 + romLen);
                sendError('loadRom failed: invalid iNES image');
                continue;
            }
            buf = buf.subarray(5 + romLen);
            sendOk();
        } else if (op === 0x02) { // RESET
            if (buf.length < 1) return;
            if (!emu) { buf = buf.subarray(1); sendError('no emulator'); continue; }
            emu.reset();
            buf = buf.subarray(1);
            sendOk();
        } else if (op === 0x03) { // STEP_FRAME
            if (buf.length < 5) return;
            if (!emu) { buf = buf.subarray(5); sendError('no emulator'); continue; }
            const count = buf.readUInt32LE(1);
            buf = buf.subarray(5);
            let totalCycles = 0;
            let totalAudio = 0;
            for (let i = 0; i < count; i++) {
                totalCycles += emu!.stepFrame();
                // Ensure space before draining (one frame ~735 NTSC samples).
                if (totalAudio + 1024 > audioDrainBuf.length) {
                    ensureAudioCap(totalAudio + 1024, totalAudio);
                }
                const drained = emu!.takeAudio(audioDrainBuf.subarray(totalAudio), audioDrainBuf.length - totalAudio);
                totalAudio += drained;
            }
            sendResult(totalCycles, emu!.framebuffer(), audioDrainBuf, totalAudio);
        } else if (op === 0x04) { // STEP_INSTR
            if (buf.length < 5) return;
            if (!emu) { buf = buf.subarray(5); sendError('no emulator'); continue; }
            const count = buf.readUInt32LE(1);
            buf = buf.subarray(5);
            let totalCycles = 0;
            let totalAudio = 0;
            for (let i = 0; i < count; i++) {
                totalCycles += emu!.stepInstruction();
                if (totalAudio + 256 > audioDrainBuf.length) {
                    ensureAudioCap(totalAudio + 256, totalAudio);
                }
                const drained = emu!.takeAudio(audioDrainBuf.subarray(totalAudio), audioDrainBuf.length - totalAudio);
                totalAudio += drained;
            }
            sendResult(totalCycles, emu!.framebuffer(), audioDrainBuf, totalAudio);
        } else if (op === 0x05) { // SAVE_STATE
            if (buf.length < 1) return;
            if (!emu) { buf = buf.subarray(1); sendError('no emulator'); continue; }
            const data = emu.saveState();
            const hdr = Buffer.alloc(4);
            hdr.writeUInt32LE(data.length, 0);
            stdout.write(Buffer.concat([hdr, Buffer.from(data.buffer, data.byteOffset, data.byteLength)]));
            buf = buf.subarray(1);
        } else if (op === 0x06) { // LOAD_STATE
            if (buf.length < 5) return;
            const len = buf.readUInt32LE(1);
            if (buf.length < 5 + len) return;
            if (!emu) { buf = buf.subarray(5 + len); sendError('no emulator'); continue; }
            const data = buf.subarray(5, 5 + len);
            const ok = emu.loadState(new Uint8Array(data));
            buf = buf.subarray(5 + len);
            if (ok) sendOk();
            else sendError('loadState failed: bad magic/version/size');
        } else {
            sendError('unknown opcode 0x' + op.toString(16));
            process.exit(1);
        }
    }
}

function sendOk(): void {
    stdout.write(Buffer.from([0x00]));
}

function sendError(msg: string): void {
    const msgBuf = Buffer.from(msg, 'utf8');
    const hdr = Buffer.alloc(3);
    hdr[0] = 0xFF;
    hdr.writeUInt16LE(msgBuf.length, 1);
    stdout.write(Buffer.concat([hdr, msgBuf]));
}

function sendResult(cycles: number, fb: Uint32Array, audio: Int16Array, audioCount: number): void {
    const hdr = Buffer.alloc(12);
    hdr.writeUInt32LE(cycles >>> 0, 0);
    hdr.writeUInt32LE(FB_PIXELS, 4);
    hdr.writeUInt32LE(audioCount, 8);
    const fbBytes = Buffer.from(fb.buffer, fb.byteOffset, fb.byteLength);
    const audioBytes = Buffer.from(audio.buffer, audio.byteOffset, audioCount * 2);
    stdout.write(Buffer.concat([hdr, fbBytes, audioBytes]));
}
