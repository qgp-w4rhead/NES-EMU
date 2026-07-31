// TV system / region (port of cores/c/src/region.c).

export enum Region {
    Ntsc = 0,
    Pal = 1,
    Dendy = 2,
}

export interface ApuFrameThreshold {
    threshold: number;
    quarter: boolean;
    half: boolean;
}

const Ntsc4Step = new Uint32Array([7457, 14913, 22371, 29828]);
const Pal4Step = new Uint32Array([8314, 16627, 24941, 33255]);
const Ntsc5Step = new Uint32Array([7457, 14913, 22371, 37281]);
const Pal5Step = new Uint32Array([8314, 16627, 24941, 41568]);
const QuarterFlags = [true, true, true, true];
const HalfFlags = [false, true, false, true];

export function scanlinesPerFrame(r: number): number {
    if (r === Region.Ntsc) return 262;
    if (r === Region.Pal) return 312;
    return 312; // Dendy
}

export function scanlinePrerender(r: number): number {
    if (r === Region.Ntsc) return 261;
    if (r === Region.Pal) return 311;
    return 311; // Dendy
}

export function isPalPalette(r: number): boolean {
    return r === Region.Pal;
}

export function cpuClockHz(r: number): number {
    if (r === Region.Pal) return 1662607.0;
    return 1789773.0;
}

export function cpuCyclesPerSample(r: number): number {
    return cpuClockHz(r) / 44100.0;
}

export function apu4StepThresholds(r: number, outs: ApuFrameThreshold[]): void {
    const t = (r === Region.Pal) ? Pal4Step : Ntsc4Step;
    for (let i = 0; i < 4; ++i) {
        outs[i].threshold = t[i];
        outs[i].quarter = QuarterFlags[i];
        outs[i].half = HalfFlags[i];
    }
}

export function apu5StepThresholds(r: number, outs: ApuFrameThreshold[]): void {
    const t = (r === Region.Pal) ? Pal5Step : Ntsc5Step;
    for (let i = 0; i < 4; ++i) {
        outs[i].threshold = t[i];
        outs[i].quarter = QuarterFlags[i];
        outs[i].half = HalfFlags[i];
    }
}

export function apuResetAt(r: number, mode5step: boolean): number {
    if (r === Region.Pal) return mode5step ? 41570 : 33257;
    return mode5step ? 37282 : 29830;
}

export function apu4StepIrqThreshold(r: number): number {
    if (r === Region.Pal) return 33255;
    return 29828;
}
