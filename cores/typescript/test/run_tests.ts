// Test runner for the TypeScript NES core. Imports the Emulator class
// from source and runs a suite of behavioral tests.
//
// Build:  tsc -p tsconfig.test.json   (after `npx tsc` for the main build)
// Run:    node dist/test/run_tests.js
//
// Prints "PASS: <name>" / "FAIL: <name>: <reason>" and exits 0 only if all pass.

import { Emulator } from '../src/emulator';

let passed = 0;
let failed = 0;
const failures: string[] = [];

function assert(cond: boolean, name: string, reason?: string): void {
    if (cond) {
        console.log('PASS: ' + name);
        passed++;
    } else {
        console.log('FAIL: ' + name + ': ' + (reason || 'assertion failed'));
        failed++;
        failures.push(name);
    }
}

// ---- NOP ROM construction ----
// 16-byte iNES header + 1 PRG bank (16KB of 0xEA NOP) + 1 CHR bank (8KB zero).
// Reset vector at $FFFC/$FFFD -> $C000. Total = 24592 bytes.
function makeNopRom(): Uint8Array {
    const PRG = 16384;
    const CHR = 8192;
    const rom = new Uint8Array(16 + PRG + CHR);
    // "NES\x1A"
    rom[0] = 0x4E; rom[1] = 0x45; rom[2] = 0x53; rom[3] = 0x1A;
    rom[4] = 1;   // PRG banks
    rom[5] = 1;   // CHR banks
    rom[6] = 0;   // flags6: mapper 0, horizontal mirroring, no trainer/battery
    rom[7] = 0;   // flags7: mapper 0
    // PRG filled with NOP (0xEA)
    for (let i = 0; i < PRG; i++) rom[16 + i] = 0xEA;
    // Reset vector: $FFFC/$FFFD -> $C000 (low byte 0x00, high byte 0xC0).
    // For a 16KB PRG mapped at $C000-$FFFF, $FFFC maps to PRG offset 0x3FFC.
    rom[16 + 0x3FFC] = 0x00;
    rom[16 + 0x3FFD] = 0xC0;
    // CHR left zero.
    return rom;
}

// ---- Tests ----

function test_step_frame(): void {
    const emu = new Emulator();
    assert(emu.loadRom(makeNopRom()) === true, 'step_frame.loadRom', 'loadRom returned false');
    emu.reset();
    const cycles = emu.stepFrame();
    assert(cycles > 0, 'step_frame', 'cycles should be > 0, got ' + cycles);
}

function test_step_instruction(): void {
    const emu = new Emulator();
    emu.loadRom(makeNopRom());
    emu.reset();
    const cycles = emu.stepInstruction();
    assert(cycles > 0, 'step_instruction', 'cycles should be > 0, got ' + cycles);
}

function test_mapper_number(): void {
    const emu = new Emulator();
    emu.loadRom(makeNopRom());
    assert(emu.mapperNumber() === 0, 'mapper_number', 'expected mapper 0, got ' + emu.mapperNumber());
}

function test_save_load_roundtrip(): void {
    // Run 10 frames, save, run 5 more, load, run 5 more -> compare framebuffer
    // to a fresh run of 10+5 frames.
    const rom = makeNopRom();

    const a = new Emulator();
    a.loadRom(rom);
    a.reset();
    for (let i = 0; i < 10; i++) a.stepFrame();
    const state = a.saveState();
    assert(state.length > 0, 'save_load_roundtrip.save', 'saveState produced empty buffer');
    for (let i = 0; i < 5; i++) a.stepFrame();
    const ok = a.loadState(state);
    assert(ok, 'save_load_roundtrip.load', 'loadState returned false');
    for (let i = 0; i < 5; i++) a.stepFrame();
    const fbA = a.framebuffer();

    const b = new Emulator();
    b.loadRom(rom);
    b.reset();
    for (let i = 0; i < 15; i++) b.stepFrame();
    const fbB = b.framebuffer();

    let diff = 0;
    for (let i = 0; i < fbA.length; i++) if (fbA[i] !== fbB[i]) diff++;
    assert(diff === 0, 'save_load_roundtrip', diff + ' framebuffer pixels differ after round-trip');
}

function test_set_region(): void {
    const emu = new Emulator();
    emu.loadRom(makeNopRom());
    emu.reset();
    let crashed: unknown = false;
    try {
        emu.setRegion(0); // NTSC
        emu.stepFrame();
        emu.setRegion(1); // PAL
        emu.stepFrame();
        emu.setRegion(2); // Dendy
        emu.stepFrame();
        emu.setRegion(0); // back to NTSC
        emu.stepFrame();
    } catch (e) {
        crashed = e;
    }
    assert(!crashed, 'set_region', 'crashed: ' + (crashed && (crashed as Error).message));
}

function test_1000_frame_run(): void {
    const emu1 = new Emulator();
    emu1.loadRom(makeNopRom());
    emu1.reset();
    let crashed: unknown = false;
    try {
        for (let i = 0; i < 1000; i++) emu1.stepFrame();
    } catch (e) {
        crashed = e;
    }
    assert(!crashed, '1000_frame_run', 'crashed: ' + (crashed && (crashed as Error).message));

    // Determinism: run again and compare.
    const emu2 = new Emulator();
    emu2.loadRom(makeNopRom());
    emu2.reset();
    for (let i = 0; i < 1000; i++) emu2.stepFrame();
    const fb1 = emu1.framebuffer();
    const fb2 = emu2.framebuffer();
    let diff = 0;
    for (let i = 0; i < fb1.length; i++) if (fb1[i] !== fb2[i]) diff++;
    assert(diff === 0, '1000_frame_run.determinism', diff + ' pixels differ between two 1000-frame runs');
}

function test_audio_drain(): void {
    const emu = new Emulator();
    emu.loadRom(makeNopRom());
    emu.reset();
    emu.stepFrame();
    emu.stepFrame();
    const cap = 4096;
    const buf = new Int16Array(cap);
    const n = emu.takeAudio(buf, cap);
    assert(n > 0, 'audio_drain', 'expected > 0 audio samples, got ' + n);
}

function test_framebuffer(): void {
    const emu = new Emulator();
    emu.loadRom(makeNopRom());
    emu.reset();
    emu.stepFrame();
    const fb = emu.framebuffer();
    assert(fb.length === 256 * 240, 'framebuffer.size', 'expected 61440 pixels, got ' + fb.length);
    let nonZero = 0;
    for (let i = 0; i < fb.length; i++) if (fb[i] !== 0) nonZero++;
    assert(nonZero > 0, 'framebuffer.nonzero', 'framebuffer is all zeros');
}

function test_reset(): void {
    const emu = new Emulator();
    emu.loadRom(makeNopRom());
    emu.reset();
    // After RESET, PC should be loaded from the reset vector ($C000).
    assert(emu.cpu.pc === 0xC000, 'reset.pc', 'expected PC=0xC000 after reset, got 0x' + emu.cpu.pc.toString(16));
    // SP should be 0xFD and I flag set.
    assert(emu.cpu.sp === 0xFD, 'reset.sp', 'expected SP=0xFD, got 0x' + emu.cpu.sp.toString(16));
    assert((emu.cpu.status & 0x04) !== 0, 'reset.interrupt_disable', 'I flag should be set after reset');
}

function test_determinism(): void {
    const rom = makeNopRom();
    const a = new Emulator();
    a.loadRom(rom);
    a.reset();
    for (let i = 0; i < 100; i++) a.stepFrame();
    const fbA = a.framebuffer();

    const b = new Emulator();
    b.loadRom(rom);
    b.reset();
    for (let i = 0; i < 100; i++) b.stepFrame();
    const fbB = b.framebuffer();

    let diff = 0;
    for (let i = 0; i < fbA.length; i++) if (fbA[i] !== fbB[i]) diff++;
    assert(diff === 0, 'determinism', diff + ' pixels differ between two 100-frame runs');
}

function test_impl_name(): void {
    const emu = new Emulator();
    assert(emu.implName() === 'typescript-nes', 'impl_name', 'expected typescript-nes, got ' + emu.implName());
    assert(emu.implVersion() === '0.1.0', 'impl_version', 'expected 0.1.0, got ' + emu.implVersion());
}

function test_save_state_size_stable(): void {
    // Saving twice (with no state change between) should produce identical bytes.
    const emu = new Emulator();
    emu.loadRom(makeNopRom());
    emu.reset();
    emu.stepFrame();
    const s1 = emu.saveState();
    const s2 = emu.saveState();
    assert(s1.length === s2.length, 'save_state_size_stable.length', 'length differs: ' + s1.length + ' vs ' + s2.length);
    let diff = 0;
    for (let i = 0; i < s1.length; i++) if (s1[i] !== s2[i]) diff++;
    assert(diff === 0, 'save_state_size_stable.bytes', diff + ' bytes differ between two consecutive saves');
}

// ---- Run all ----
const tests: Array<() => void> = [
    test_step_frame,
    test_step_instruction,
    test_mapper_number,
    test_save_load_roundtrip,
    test_set_region,
    test_1000_frame_run,
    test_audio_drain,
    test_framebuffer,
    test_reset,
    test_determinism,
    test_impl_name,
    test_save_state_size_stable,
];

for (const t of tests) {
    try {
        t();
    } catch (e) {
        console.log('FAIL: ' + (t as Function).name + ': exception: ' + ((e as Error).stack ? (e as Error).stack : String(e)));
        failed++;
        failures.push((t as Function).name);
    }
}

console.log('');
console.log('Total: ' + passed + ' passed, ' + failed + ' failed');
if (failed > 0) {
    console.log('Failed tests: ' + failures.join(', '));
    process.exit(1);
} else {
    console.log('ALL TESTS PASSED');
    process.exit(0);
}
