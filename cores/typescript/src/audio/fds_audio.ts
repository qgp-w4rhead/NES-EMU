// Famicom Disk System expansion audio (wavetable + modulator).
// Port of cores/c/src/fds_audio.c.

export const FDS_WAVE_TABLE_SIZE = 64;
export const FDS_MOD_TABLE_SIZE = 64;

function satSubI8(a: number, b: number): number {
    let r = a - b;
    if (r < -128) r = -128;
    if (r > 127) r = 127;
    return r;
}
function satAddI8(a: number, b: number): number {
    let r = a + b;
    if (r < -128) r = -128;
    if (r > 127) r = 127;
    return r;
}

export class FdsAudio {
    waveRam: Uint8Array = new Uint8Array(FDS_WAVE_TABLE_SIZE);
    waveAddr: number = 0;
    masterVolume: number = 0;
    envDisabled: boolean = false;
    envIncrease: boolean = false;
    volumeGain: number = 0;
    freq: number = 0;
    phaseAcc: number = 0;
    modFreq: number = 0;
    modPhaseAcc: number = 0;
    modWave: Uint8Array = new Uint8Array(FDS_MOD_TABLE_SIZE);
    modWaveAddr: number = 0;
    modDisabled: boolean = true;
    modGain: number = 0;
    modSweepCounter: number = 0;
    modSweepNeg: number = 0;
    modSweepPos: number = 0;
    modGainOutput: number = 0;

    init(): void {
        this.waveRam.fill(0);
        this.waveAddr = 0;
        this.masterVolume = 0;
        this.envDisabled = false;
        this.envIncrease = false;
        this.volumeGain = 0;
        this.freq = 0;
        this.phaseAcc = 0;
        this.modFreq = 0;
        this.modPhaseAcc = 0;
        this.modWave.fill(0);
        this.modWaveAddr = 0;
        this.modDisabled = true;
        this.modGain = 0;
        this.modSweepCounter = 0;
        this.modSweepNeg = 0;
        this.modSweepPos = 0;
        this.modGainOutput = 0;
    }

    writeRegister(addr: number, value: number): void {
        if (addr >= 0x4040 && addr <= 0x407F) {
            this.waveRam[this.waveAddr % FDS_WAVE_TABLE_SIZE] = value & 0x3F;
            this.waveAddr = (this.waveAddr + 1) & 0x3F;
            return;
        }
        switch (addr) {
            case 0x4080:
                this.masterVolume = value & 0x3F;
                this.envDisabled = (value & 0x40) !== 0;
                this.envIncrease = (value & 0x80) !== 0;
                if (this.envDisabled) this.volumeGain = this.masterVolume;
                else this.volumeGain = this.envIncrease ? 0 : 0x3F;
                break;
            case 0x4081:
                this.freq = (this.freq & 0x00FF) | ((value & 0x0F) << 8);
                break;
            case 0x4082:
                this.freq = (this.freq & 0x0F00) | value;
                break;
            case 0x4083:
                this.modFreq = (this.modFreq & 0x00FF) | ((value & 0x0F) << 8);
                if (value & 0x80) { this.modDisabled = true; this.modPhaseAcc = 0; }
                break;
            case 0x4084:
                this.modSweepNeg = value & 0x0F;
                break;
            case 0x4085:
                this.modSweepPos = value & 0x0F;
                break;
            case 0x4086: {
                const neg = value & 0x3F;
                this.modGain = satSubI8(this.modGain, neg);
                break;
            }
            case 0x4087: {
                const pos = value & 0x3F;
                this.modGain = satAddI8(this.modGain, pos);
                if (value & 0x80) this.modDisabled = true;
                break;
            }
            case 0x4088:
                this.modWave[this.modWaveAddr % FDS_MOD_TABLE_SIZE] = value & 0x07;
                this.modWaveAddr = (this.modWaveAddr + 1) & 0x3F;
                break;
            case 0x4089:
                this.masterVolume = (this.masterVolume & 0x3C) | (value & 0x03);
                break;
            case 0x408A:
                if (value & 0x80) this.modDisabled = true;
                break;
        }
    }

    readRegister(addr: number): number {
        switch (addr) {
            case 0x4090: return this.volumeGain & 0x3F;
            case 0x4092: return this.modGainOutput & 0x3F;
            default: return 0;
        }
    }

    clock(apuCycles: number): void {
        for (let c = 0; c < apuCycles; ++c) {
            this.phaseAcc = (this.phaseAcc + this.freq) >>> 0;
            if (!this.modDisabled) {
                this.modPhaseAcc = (this.modPhaseAcc + this.modFreq) >>> 0;
                this.modSweepCounter = (this.modSweepCounter + 1) & 0xFF;
                if (this.modSweepCounter >= 8) {
                    this.modSweepCounter = 0;
                    if (this.modSweepNeg > 0) this.modGain = satSubI8(this.modGain, this.modSweepNeg);
                    if (this.modSweepPos > 0) this.modGain = satAddI8(this.modGain, this.modSweepPos);
                }
            }
            if (!this.envDisabled && this.modSweepCounter === 0) {
                if (this.envIncrease && this.volumeGain < 0x3F) this.volumeGain = (this.volumeGain + 1) & 0xFF;
                else if (!this.envIncrease && this.volumeGain > 0) this.volumeGain = (this.volumeGain - 1) & 0xFF;
            }
        }
    }

    sample(): number {
        if (this.freq === 0) return 0.0;
        const waveIdx = (this.phaseAcc >> 12) & 0x3F;
        let modOffset = 0;
        if (!this.modDisabled) {
            const modIdx = (this.modPhaseAcc >> 12) & 0x3F;
            const modVal = this.modWave[modIdx];
            modOffset = (this.modGain * modVal) >> 3;
        }
        const effIdx = (waveIdx + modOffset) & 0x3F;
        const waveVal = this.waveRam[effIdx];
        const vol = this.volumeGain;
        const sample = (waveVal - 32) * vol;
        let normalized = sample / 2016.0;
        if (normalized < -1.0) normalized = -1.0;
        if (normalized > 1.0) normalized = 1.0;
        return normalized;
    }
}
