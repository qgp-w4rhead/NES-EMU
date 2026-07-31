// Full emulator state serialization (port of cores/c/src/save_state.c and
// cores/csharp/src/SaveState.cs). Custom binary format with magic + version
// tag, manual field-by-field writes via DataView (no reflection).
//
// Layout (all integers little-endian):
//   [4] magic "NESS" (0x5353454E LE)
//   [4] version (u32 = SAVE_STATE_VERSION)
//   [4] total_payload_size (u32)
//   --- payload ---
//   CPU: A, X, Y, Sp, Pc(2), Status, Flags, pad (9 bytes)
//   RAM (2048 bytes)
//   PPU arch state
//   APU open-bus (24 bytes)
//   APU state (all channels + frame counter + mixer)
//   Joypad state (7 bytes)
//   Cartridge present flag + header(12) + mapper state
//   Emulator-level: dma_stall(4), cpu_cycle_count(8), sample_accumulator(4),
//     audio_buffer_count(4), audio_buffer(count*4), ppu_cycle_carry(4), region(4)
//
// The format only needs to round-trip within the TS core (per M4.5 spec); it
// is NOT required to be byte-compatible with the C core's save states.

import { Emulator } from './emulator';
import { Ppu } from './ppu';
import { Mirroring } from './mapper';
import { Region } from './region';
import { PpuRender } from './ppu_render';

export const SAVE_STATE_MAGIC: number = 0x5353454E; // "NESS" LE
export const SAVE_STATE_VERSION: number = 1;
const SS_HEADER_SIZE = 12;

// Size constants (must match the write/read layout below).
const CPU_STATE_SIZE = 9;       // A,X,Y,Sp(4) + Pc(2) + Status,Flags(2) + pad(1)
const INES_HEADER_SIZE = 12;
const PULSE_STATE_SIZE = 20;
const TRIANGLE_STATE_SIZE = 12;
const NOISE_STATE_SIZE = 17;
const DMC_STATE_SIZE = 22;
const APU_LEVEL_SIZE = 48;
const APU_STATE_SIZE = PULSE_STATE_SIZE * 2 + TRIANGLE_STATE_SIZE + NOISE_STATE_SIZE + DMC_STATE_SIZE + APU_LEVEL_SIZE;
const JOYPAD_STATE_SIZE = 7;

// ---- Little-endian writer ----
class Writer {
    buf: Uint8Array;
    view: DataView;
    off: number = 0;

    constructor(cap: number) {
        this.buf = new Uint8Array(cap);
        this.view = new DataView(this.buf.buffer);
    }

    u8(v: number): void { this.buf[this.off++] = v & 0xFF; }
    u16(v: number): void { this.view.setUint16(this.off, v, true); this.off += 2; }
    u32(v: number): void { this.view.setUint32(this.off, v >>> 0, true); this.off += 4; }
    u64(v: number): void {
        // Write as two u32 (low, high) — JS numbers can't hold full u64 but
        // cpu_cycle_count stays well below 2^32 in practice.
        this.u32(v & 0xFFFFFFFF);
        this.u32(Math.floor(v / 0x100000000) & 0xFFFFFFFF);
    }
    f32(v: number): void { this.view.setFloat32(this.off, v, true); this.off += 4; }
    bool(v: boolean): void { this.u8(v ? 1 : 0); }
    bytes(src: Uint8Array, len: number): void {
        this.buf.set(src.subarray(0, len), this.off);
        this.off += len;
    }
}

// ---- Little-endian reader ----
class Reader {
    buf: Uint8Array;
    view: DataView;
    off: number = 0;
    end: number;

    constructor(buf: Uint8Array, end: number) {
        this.buf = buf;
        this.view = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
        this.end = end;
    }

    remaining(): number { return this.end - this.off; }
    u8(): number { return this.buf[this.off++]; }
    u16(): number { const v = this.view.getUint16(this.off, true); this.off += 2; return v; }
    u32(): number { const v = this.view.getUint32(this.off, true); this.off += 4; return v; }
    u64(): number {
        const lo = this.u32();
        const hi = this.u32();
        return lo + hi * 0x100000000;
    }
    f32(): number { const v = this.view.getFloat32(this.off, true); this.off += 4; return v; }
    bool(): boolean { return this.u8() !== 0; }
    bytes(dst: Uint8Array, len: number): void {
        dst.set(this.buf.subarray(this.off, this.off + len));
        this.off += len;
    }
}

// ---- PPU state size ----
function ppuStateSize(p: Ppu): number {
    return 4 + p.vramSize + Ppu.OamSize + Ppu.PaletteSize
        + 1 + 1 + 1 + 1 + 2 + 2 + 1 + 1 + 1 + 1 + 2 + 2 + 1 + 4 + 4;
}

// ---- Required-size computation ----
export function requiredSize(emu: Emulator): number {
    let payload = 0;
    payload += CPU_STATE_SIZE;
    payload += 2048; // Bus.RamSize
    payload += ppuStateSize(emu.bus.ppu);
    payload += 24;  // Bus.ApuIoRegCount
    payload += APU_STATE_SIZE;
    payload += JOYPAD_STATE_SIZE;
    if (emu.cartridge !== null) {
        payload += 1 + INES_HEADER_SIZE + emu.cartridge.mapper.saveState(null);
    } else {
        payload += 1;
    }
    payload += 4 + 8 + 4 + 4 + emu.audioBufferCount * 4 + 4 + 4;
    return SS_HEADER_SIZE + payload;
}

// ---- Save ----
export function serialize(emu: Emulator): Uint8Array {
    const required = requiredSize(emu);
    const w = new Writer(required);

    const payload = required - SS_HEADER_SIZE;
    w.u32(SAVE_STATE_MAGIC);
    w.u32(SAVE_STATE_VERSION);
    w.u32(payload);

    // CPU
    w.u8(emu.cpu.a);
    w.u8(emu.cpu.x);
    w.u8(emu.cpu.y);
    w.u8(emu.cpu.sp);
    w.u16(emu.cpu.pc);
    w.u8(emu.cpu.status);
    w.u8(emu.cpu.flags);
    w.u8(0); // pad

    // RAM
    w.bytes(emu.bus.ram, 2048);

    // PPU arch state
    writePpuState(w, emu.bus.ppu);

    // APU open-bus
    w.bytes(emu.bus.apuOpenBus, 24);

    // APU state
    writeApuState(w, emu.bus.apu);

    // Joypad
    writeJoypadState(w, emu.bus.joypad);

    // Cartridge
    if (emu.cartridge !== null) {
        w.u8(1);
        writeInesHeader(w, emu.cartridge.header);
        const msz = emu.cartridge.mapper.saveState(null);
        if (msz > 0) {
            const mb = new Uint8Array(msz);
            emu.cartridge.mapper.saveState(mb);
            w.bytes(mb, msz);
        }
    } else {
        w.u8(0);
    }

    // Emulator-level
    w.u32(emu.bus.dmaStallCycles);
    w.u64(emu.bus.cpuCycleCount);
    w.f32(emu.sampleAccumulator);
    w.u32(emu.audioBufferCount);
    for (let i = 0; i < emu.audioBufferCount; ++i) {
        w.f32(emu.audioBuffer[i]);
    }
    w.u32(emu.ppuCycleCarry);
    w.u32(emu.region);

    return w.buf.subarray(0, w.off);
}

// ---- Load ----
export function deserialize(emu: Emulator, data: Uint8Array): boolean {
    if (data.length < SS_HEADER_SIZE) return false;
    const r = new Reader(data, data.length);

    const magic = r.u32();
    if (magic !== SAVE_STATE_MAGIC) return false;
    const version = r.u32();
    if (version !== SAVE_STATE_VERSION) return false;
    const payload = r.u32();
    if (payload + SS_HEADER_SIZE > data.length) return false;
    r.end = SS_HEADER_SIZE + payload;

    // CPU
    if (r.remaining() < CPU_STATE_SIZE) return false;
    emu.cpu.a = r.u8();
    emu.cpu.x = r.u8();
    emu.cpu.y = r.u8();
    emu.cpu.sp = r.u8();
    emu.cpu.pc = r.u16();
    emu.cpu.status = r.u8();
    emu.cpu.flags = r.u8();
    r.u8(); // pad

    // RAM
    if (r.remaining() < 2048) return false;
    r.bytes(emu.bus.ram, 2048);

    // PPU arch state
    if (!readPpuState(r, emu.bus.ppu)) return false;

    // APU open-bus
    if (r.remaining() < 24) return false;
    r.bytes(emu.bus.apuOpenBus, 24);

    // APU state
    if (!readApuState(r, emu.bus.apu)) return false;

    // Joypad
    if (!readJoypadState(r, emu.bus.joypad)) return false;

    // Cartridge
    if (r.remaining() < 1) return false;
    const cartPresent = r.u8();
    if (cartPresent !== 0) {
        if (r.remaining() < INES_HEADER_SIZE) return false;
        const hdr = readInesHeader(r);
        if (emu.cartridge === null) return false;
        if (hdr.mapperNumber !== emu.cartridge.header.mapperNumber) return false;
        emu.cartridge.header = hdr;
        const msz = emu.cartridge.mapper.saveState(null);
        if (r.remaining() < msz) return false;
        const mb = new Uint8Array(msz);
        r.bytes(mb, msz);
        if (!emu.cartridge.mapper.loadState(mb, msz)) return false;
        emu.bus.ppu.setMirroring(emu.cartridge.mirrorMode());
        emu.bus.insertCartridge(emu.cartridge);
    } else {
        emu.bus.removeCartridge();
    }

    // Emulator-level
    if (r.remaining() < 4) return false;
    emu.bus.dmaStallCycles = r.u32();
    if (r.remaining() < 8) return false;
    emu.bus.cpuCycleCount = r.u64();
    if (r.remaining() < 4) return false;
    emu.sampleAccumulator = r.f32();
    if (r.remaining() < 4) return false;
    const audioCount = r.u32();
    if (audioCount > r.remaining() / 4) return false;
    const audio = new Float32Array(audioCount);
    for (let i = 0; i < audioCount; ++i) {
        audio[i] = r.f32();
    }
    emu.setAudioBuffer(audio, audioCount);
    if (r.remaining() < 4) return false;
    emu.ppuCycleCarry = r.u32();
    if (r.remaining() < 4) return false;
    const regn = r.u32();
    if (regn <= Region.Dendy) emu.setRegion(regn);

    // Clear framebuffer to universal bg (derived data).
    const universalBg = PpuRender.universalBgArgb(emu.bus.ppu);
    PpuRender.clearFramebuffer(emu.bus.ppu, universalBg);

    return true;
}

// ---- PPU state ----
function writePpuState(w: Writer, p: Ppu): void {
    w.u32(p.vramSize);
    w.bytes(p.vram, p.vramSize);
    w.bytes(p.oam, Ppu.OamSize);
    w.bytes(p.palette, Ppu.PaletteSize);
    w.u8(p.ppuCtrl);
    w.u8(p.ppuMask);
    w.u8(p.oamAddr);
    w.u8(p.ppuStatus);
    w.u16(p.v);
    w.u16(p.t);
    w.u8(p.fineX);
    w.bool(p.w);
    w.u8(p.ppuDataBuffer);
    w.u8(p.openBus);
    w.u16(p.scanline);
    w.u16(p.cycle);
    w.bool(p.nmiRequest);
    w.u32(p.mirroring);
    w.u32(p.region);
}

function readPpuState(r: Reader, p: Ppu): boolean {
    if (r.remaining() < 4) return false;
    const vramSize = r.u32();
    if (vramSize > Ppu.VramSize4k) return false;
    const ppuNeed = 4 + vramSize + Ppu.OamSize + Ppu.PaletteSize
        + 1 + 1 + 1 + 1 + 2 + 2 + 1 + 1 + 1 + 1 + 2 + 2 + 1 + 4 + 4;
    if (r.remaining() < ppuNeed - 4) return false;

    // Read VRAM into temp.
    const vram = new Uint8Array(vramSize);
    r.bytes(vram, vramSize);

    const oam = new Uint8Array(Ppu.OamSize);
    r.bytes(oam, Ppu.OamSize);

    const palette = new Uint8Array(Ppu.PaletteSize);
    r.bytes(palette, Ppu.PaletteSize);

    const ppuctrl = r.u8();
    const ppumask = r.u8();
    const oamaddr = r.u8();
    const ppustatus = r.u8();
    const v = r.u16();
    const t = r.u16();
    const fineX = r.u8();
    const wFlag = r.bool();
    const ppudataBuffer = r.u8();
    const openBus = r.u8();
    const scanline = r.u16();
    const cycle = r.u16();
    const nmiRequest = r.bool();
    const mirroring = r.u32();
    const region = r.u32();

    // Apply mirroring/region BEFORE copying VRAM (setMirroring may resize).
    if (mirroring <= Mirroring.SingleScreen3) p.setMirroring(mirroring);
    if (region <= Region.Dendy) p.setRegion(region);
    p.vram.set(vram.subarray(0, vramSize));
    p.vramSize = vramSize;
    p.oam.set(oam);
    p.palette.set(palette);
    p.ppuCtrl = ppuctrl;
    p.ppuMask = ppumask;
    p.oamAddr = oamaddr;
    p.ppuStatus = ppustatus;
    p.v = v;
    p.t = t;
    p.fineX = fineX;
    p.w = wFlag;
    p.ppuDataBuffer = ppudataBuffer;
    p.openBus = openBus;
    p.scanline = scanline;
    p.cycle = cycle;
    p.nmiRequest = nmiRequest;
    return true;
}

// ---- Pulse channel state ----
function writePulseState(w: Writer, ch: any): void {
    w.u8(ch.duty);
    w.bool(ch.halt);
    w.bool(ch.constantVolume);
    w.u8(ch.volume);
    w.bool(ch.sweepEnabled);
    w.u8(ch.sweepPeriod);
    w.bool(ch.sweepNegate);
    w.u8(ch.sweepShift);
    w.u8(ch.sweepDivider);
    w.bool(ch.sweepReload);
    w.u16(ch.timerPeriod);
    w.u16(ch.timer);
    w.u8(ch.sequence);
    w.u8(ch.lengthCounter);
    w.bool(ch.enabled);
    w.u8(ch.envelopeDivider);
    w.u8(ch.envelopeDecay);
    w.bool(ch.envelopeStart);
}

function readPulseState(r: Reader, ch: any): boolean {
    if (r.remaining() < PULSE_STATE_SIZE) return false;
    ch.duty = r.u8();
    ch.halt = r.bool();
    ch.constantVolume = r.bool();
    ch.volume = r.u8();
    ch.sweepEnabled = r.bool();
    ch.sweepPeriod = r.u8();
    ch.sweepNegate = r.bool();
    ch.sweepShift = r.u8();
    ch.sweepDivider = r.u8();
    ch.sweepReload = r.bool();
    ch.timerPeriod = r.u16();
    ch.timer = r.u16();
    ch.sequence = r.u8();
    ch.lengthCounter = r.u8();
    ch.enabled = r.bool();
    ch.envelopeDivider = r.u8();
    ch.envelopeDecay = r.u8();
    ch.envelopeStart = r.bool();
    return true;
}

// ---- Triangle channel state ----
function writeTriangleState(w: Writer, ch: any): void {
    w.bool(ch.halt);
    w.u8(ch.linearReload);
    w.u8(ch.linearCounter);
    w.u16(ch.timerPeriod);
    w.u16(ch.timer);
    w.u8(ch.sequence);
    w.u8(ch.lengthCounter);
    w.bool(ch.enabled);
    w.bool(ch.linearStart);
    w.u8(0); // pad
}

function readTriangleState(r: Reader, ch: any): boolean {
    if (r.remaining() < TRIANGLE_STATE_SIZE) return false;
    ch.halt = r.bool();
    ch.linearReload = r.u8();
    ch.linearCounter = r.u8();
    ch.timerPeriod = r.u16();
    ch.timer = r.u16();
    ch.sequence = r.u8();
    ch.lengthCounter = r.u8();
    ch.enabled = r.bool();
    ch.linearStart = r.bool();
    r.u8(); // pad
    return true;
}

// ---- Noise channel state ----
function writeNoiseState(w: Writer, ch: any): void {
    w.bool(ch.halt);
    w.bool(ch.constantVolume);
    w.u8(ch.volume);
    w.bool(ch.mode);
    w.u8(ch.periodIndex);
    w.u16(ch.timerPeriod);
    w.u16(ch.timer);
    w.u16(ch.lfsr);
    w.u8(ch.lengthCounter);
    w.bool(ch.enabled);
    w.u8(ch.envelopeDivider);
    w.u8(ch.envelopeDecay);
    w.bool(ch.envelopeStart);
    w.u8(0); // pad
}

function readNoiseState(r: Reader, ch: any): boolean {
    if (r.remaining() < NOISE_STATE_SIZE) return false;
    ch.halt = r.bool();
    ch.constantVolume = r.bool();
    ch.volume = r.u8();
    ch.mode = r.bool();
    ch.periodIndex = r.u8();
    ch.timerPeriod = r.u16();
    ch.timer = r.u16();
    ch.lfsr = r.u16();
    ch.lengthCounter = r.u8();
    ch.enabled = r.bool();
    ch.envelopeDivider = r.u8();
    ch.envelopeDecay = r.u8();
    ch.envelopeStart = r.bool();
    r.u8(); // pad
    return true;
}

// ---- DMC channel state ----
function writeDmcState(w: Writer, ch: any): void {
    w.bool(ch.irqEnable);
    w.bool(ch.loopFlag);
    w.u8(ch.rateIndex);
    w.u16(ch.timerPeriod);
    w.u16(ch.timer);
    w.u8(ch.outputCounter);
    w.u8(ch.sampleBuffer);
    w.u8(ch.bufferBits);
    w.u16(ch.sampleAddrBase);
    w.u16(ch.sampleAddress);
    w.u16(ch.sampleLength);
    w.u16(ch.bytesRemaining);
    w.bool(ch.enabled);
    w.bool(ch.irqFlag);
    w.u8(0); // pad
    w.u8(0); // pad
}

function readDmcState(r: Reader, ch: any): boolean {
    if (r.remaining() < DMC_STATE_SIZE) return false;
    ch.irqEnable = r.bool();
    ch.loopFlag = r.bool();
    ch.rateIndex = r.u8();
    ch.timerPeriod = r.u16();
    ch.timer = r.u16();
    ch.outputCounter = r.u8();
    ch.sampleBuffer = r.u8();
    ch.bufferBits = r.u8();
    ch.sampleAddrBase = r.u16();
    ch.sampleAddress = r.u16();
    ch.sampleLength = r.u16();
    ch.bytesRemaining = r.u16();
    ch.enabled = r.bool();
    ch.irqFlag = r.bool();
    r.u8(); // pad
    r.u8(); // pad
    return true;
}

// ---- APU state ----
function writeApuState(w: Writer, a: any): void {
    writePulseState(w, a.pulse1);
    writePulseState(w, a.pulse2);
    writeTriangleState(w, a.triangle);
    writeNoiseState(w, a.noise);
    writeDmcState(w, a.dmc);
    w.u32(a.cycleAccumulator);
    w.f32(a.sampleAccumulator);
    w.f32(a.lpfPrev);
    w.f32(a.dcPrevX);
    w.f32(a.dcPrevY);
    w.f32(a.mixAccumulator);
    w.u32(a.mixCount);
    w.f32(a.lastDecimated);
    w.u32(a.frameCycle);
    w.bool(a.frameMode5Step);
    w.bool(a.frameIrqInhibit);
    w.bool(a.frameIrq);
    w.u32(a.frameResetDelay);
    w.u32(a.region);
    w.u8(a.selectedChannel);
}

function readApuState(r: Reader, a: any): boolean {
    if (!readPulseState(r, a.pulse1)) return false;
    if (!readPulseState(r, a.pulse2)) return false;
    if (!readTriangleState(r, a.triangle)) return false;
    if (!readNoiseState(r, a.noise)) return false;
    if (!readDmcState(r, a.dmc)) return false;
    if (r.remaining() < APU_LEVEL_SIZE) return false;
    a.cycleAccumulator = r.u32();
    a.sampleAccumulator = r.f32();
    a.lpfPrev = r.f32();
    a.dcPrevX = r.f32();
    a.dcPrevY = r.f32();
    a.mixAccumulator = r.f32();
    a.mixCount = r.u32();
    a.lastDecimated = r.f32();
    a.frameCycle = r.u32();
    a.frameMode5Step = r.bool();
    a.frameIrqInhibit = r.bool();
    a.frameIrq = r.bool();
    a.frameResetDelay = r.u32();
    const regn = r.u32();
    if (regn <= Region.Dendy) a.region = regn;
    a.selectedChannel = r.u8();
    return true;
}

// ---- Joypad state ----
function writeJoypadState(w: Writer, j: any): void {
    w.u8(j.current[0]);
    w.u8(j.current[1]);
    w.bool(j.strobe);
    w.u8(j.shift[0]);
    w.u8(j.shift[1]);
    w.u8(j.counter[0]);
    w.u8(j.counter[1]);
}

function readJoypadState(r: Reader, j: any): boolean {
    if (r.remaining() < JOYPAD_STATE_SIZE) return false;
    j.current[0] = r.u8();
    j.current[1] = r.u8();
    j.strobe = r.bool();
    j.shift[0] = r.u8();
    j.shift[1] = r.u8();
    j.counter[0] = r.u8();
    j.counter[1] = r.u8();
    return true;
}

// ---- InesHeader ----
function writeInesHeader(w: Writer, h: any): void {
    w.u8(h.prgRomBanks);
    w.u8(h.chrRomBanks);
    w.u16(h.mapperNumber);
    w.u8(h.mirroring);
    w.bool(h.hasTrainer);
    w.bool(h.hasBattery);
    w.u8(h.tvSystem);
    w.u8(0); w.u8(0); w.u8(0); w.u8(0); // pad
}

function readInesHeader(r: Reader): any {
    const prgRomBanks = r.u8();
    const chrRomBanks = r.u8();
    const mapperNumber = r.u16();
    const mirr = r.u8();
    const mirroring = (mirr <= Mirroring.FourScreen) ? mirr : Mirroring.Horizontal;
    const hasTrainer = r.bool();
    const hasBattery = r.bool();
    const tvSystem = r.u8();
    r.u8(); r.u8(); r.u8(); r.u8(); // pad
    return { prgRomBanks, chrRomBanks, mapperNumber, mirroring, hasTrainer, hasBattery, tvSystem };
}
