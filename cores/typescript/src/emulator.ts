// Emulator state: ties CPU, PPU, APU, bus, and cartridge together in a
// frame-locked loop. Port of cores/c/src/emulator.c.
//
// Frame loop: 1 CPU cycle = 3 PPU cycles; audio samples accumulated from
// the APU + expansion audio; frame boundary detected by the PPU scanline
// wrapping from the prerender scanline back to 0; leftover PPU cycles
// carried into the next frame.
//
// See: https://www.nesdev.org/wiki/Cycle_reference

import { Cpu } from './cpu';
import './cpu_execute';
import './cpu_unofficial';
import { Bus } from './bus';
import { Cartridge } from './cartridge';
import { Ppu } from './ppu';
import { PpuRender } from './ppu_render';
import { Region, scanlinesPerFrame, scanlinePrerender, cpuCyclesPerSample } from './region';

// Audio buffer capacity per frame. NTSC ~735 samples/frame, PAL ~882; reserve
// headroom. (emulator.c EMU_AUDIO_CAP_*.)
const AUDIO_CAP_NTSC = 800;
const AUDIO_CAP_PAL = 950;

export class Emulator {
    cpu: Cpu = new Cpu();
    bus: Bus = new Bus();
    cartridge: Cartridge | null = null;
    region: number = Region.Ntsc;

    sampleAccumulator: number = 0.0;
    ppuCycleCarry: number = 0;

    audioBuffer: Float32Array = new Float32Array(AUDIO_CAP_NTSC);
    audioBufferCapacity: number = AUDIO_CAP_NTSC;
    audioBufferCount: number = 0;

    constructor() {
        this.initWithRegion(Region.Ntsc);
    }

    initWithRegion(region: number): void {
        this.cpu = new Cpu();
        this.bus = new Bus();
        this.cpu.bus = this.bus;
        this.bus.ppu.setRegion(region);
        this.bus.apu.setRegion(region);
        this.region = region;
        this.sampleAccumulator = 0.0;
        this.ppuCycleCarry = 0;
        this.cartridge = null;
        const cap = (scanlinesPerFrame(region) > 262) ? AUDIO_CAP_PAL : AUDIO_CAP_NTSC;
        this.audioBuffer = new Float32Array(cap);
        this.audioBufferCapacity = cap;
        this.audioBufferCount = 0;
    }

    loadRom(rom: Uint8Array): boolean {
        const cart = Cartridge.fromBytes(rom, rom.length);
        if (cart === null) return false;
        this.cartridge = cart;
        this.bus.initWithCartridge(cart);
        this.cpu.bus = this.bus;
        return true;
    }

    reset(): void {
        this.cpu.reset();
    }

    setRegion(region: number): void {
        this.region = region;
        this.bus.ppu.setRegion(region);
        this.bus.apu.setRegion(region);
    }

    private pushAudio(sample: number): void {
        if (this.audioBufferCount >= this.audioBufferCapacity) {
            let newCap = this.audioBufferCapacity * 2;
            if (newCap < 16) newCap = 16;
            const nb = new Float32Array(newCap);
            if (this.audioBufferCount > 0)
                nb.set(this.audioBuffer.subarray(0, this.audioBufferCount));
            this.audioBuffer = nb;
            this.audioBufferCapacity = newCap;
        }
        this.audioBuffer[this.audioBufferCount++] = sample;
    }

    // Run one CPU instruction plus its PPU/APU/mapper side-effects, returning
    // the CPU cycles consumed this tick and whether the frame just completed.
    private stepOneCpuTick(prevScanline: number, cyclesPerSample: number,
        prerender: number): { cycles: number; frameDone: boolean } {
        let cpuCycles = 0;

        const stepCycles = this.cpu.step();
        cpuCycles += stepCycles;

        const dmaCycles = this.bus.takeDmaStallCycles();
        cpuCycles += dmaCycles;

        this.bus.advanceCpuCycles(stepCycles + dmaCycles);

        const apuCycles = stepCycles + dmaCycles;
        this.bus.stepApu(apuCycles);
        this.bus.clockCartCpu(apuCycles);

        if (this.bus.apuIrqPending()) this.cpu.setIrqPending(true);
        if (this.bus.cartIrqPending()) this.cpu.setIrqPending(true);

        this.sampleAccumulator += apuCycles;
        while (this.sampleAccumulator >= cyclesPerSample) {
            this.sampleAccumulator -= cyclesPerSample;
            let internal = this.bus.apu.output();
            const expansion = this.bus.expansionAudioSample();
            let mixed = internal + expansion;
            if (mixed < -1.0) mixed = -1.0;
            if (mixed > 1.0) mixed = 1.0;
            this.pushAudio(mixed);
        }

        let ppuCycles = 3 * apuCycles + this.ppuCycleCarry;
        this.ppuCycleCarry = 0;
        let remaining = ppuCycles;
        let frameDone = false;
        while (remaining > 0) {
            let chunk = remaining;
            if (chunk > Ppu.CyclesPerScanline) chunk = Ppu.CyclesPerScanline;
            this.bus.stepPpu(chunk);
            remaining -= chunk;

            if (this.bus.takeNmiRequest()) this.cpu.setNmiPending(true);

            const currScanline = this.bus.ppu.scanline;
            if (currScanline === 0 && prevScanline === prerender) {
                this.ppuCycleCarry = remaining;
                frameDone = true;
                break;
            }
        }

        return { cycles: cpuCycles, frameDone };
    }

    stepFrame(): number {
        let cpuCycles = 0;

        // Clear framebuffer to universal background color first.
        const universalBg = PpuRender.universalBgArgb(this.bus.ppu);
        PpuRender.clearFramebuffer(this.bus.ppu, universalBg);
        this.bus.ppu.resetRenderedFlag();

        const cyclesPerSample = cpuCyclesPerSample(this.region);
        const prerender = scanlinePrerender(this.region);

        for (;;) {
            const prevScanline = this.bus.ppu.scanline;
            const r = this.stepOneCpuTick(prevScanline, cyclesPerSample, prerender);
            cpuCycles += r.cycles;
            if (r.frameDone) break;
        }

        // Fallback: if no per-pixel output happened, render the whole frame.
        if (!this.bus.ppu.renderedThisFrame()) {
            this.bus.renderFrame();
        }

        return cpuCycles;
    }

    stepInstruction(): number {
        const prevScanline = this.bus.ppu.scanline;
        const cyclesPerSample = cpuCyclesPerSample(this.region);
        const prerender = scanlinePrerender(this.region);
        return this.stepOneCpuTick(prevScanline, cyclesPerSample, prerender).cycles;
    }

    framebuffer(): Uint32Array {
        return this.bus.ppu.framebuffer;
    }

    mapperNumber(): number {
        if (this.cartridge !== null) return this.cartridge.header.mapperNumber;
        return 0;
    }

    // Drain audio samples (float [-1,1]) into a caller-provided Int16 buffer,
    // converting with the same formula as the C# core so output is byte-identical.
    // Returns the number of int16 samples written.
    takeAudio(outBuf: Int16Array, cap: number): number {
        let n = this.audioBufferCount;
        if (n > cap) n = cap;
        for (let i = 0; i < n; ++i) {
            let s = this.audioBuffer[i];
            if (s > 1.0) s = 1.0;
            if (s < -1.0) s = -1.0;
            let v: number;
            if (s >= 0.0) {
                v = (s * 32767.0) | 0;
                if (v > 32767) v = 32767;
            } else {
                v = (s * 32768.0) | 0;
                if (v < -32768) v = -32768;
            }
            outBuf[i] = v;
        }
        this.audioBufferCount = 0;
        return n;
    }

    // Drain audio as a freshly-allocated Int16Array (used by the subprocess
    // protocol where the caller does not pre-allocate). Not used in the hot
    // frame loop.
    drainAudio(): Int16Array {
        const n = this.audioBufferCount;
        const out = new Int16Array(n);
        const got = this.takeAudio(out, n);
        return out.subarray(0, got) as Int16Array;
    }

    setAudioBuffer(buf: Float32Array, count: number): void {
        if (count > this.audioBufferCapacity) {
            this.audioBuffer = new Float32Array(count);
            this.audioBufferCapacity = count;
        }
        if (count > 0) {
            this.audioBuffer.set(buf.subarray(0, count));
        }
        this.audioBufferCount = count;
    }

    saveState(): Uint8Array {
        // Lazy import to avoid a circular dependency at module-load time.
        const ss = require('./save_state');
        return ss.serialize(this) as Uint8Array;
    }

    loadState(data: Uint8Array): boolean {
        const ss = require('./save_state');
        return ss.deserialize(this, data) as boolean;
    }

    implName(): string { return 'typescript-nes'; }
    implVersion(): string { return '0.1.0'; }
}
