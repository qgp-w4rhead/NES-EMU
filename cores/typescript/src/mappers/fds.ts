// Mapper 20 (Famicom Disk System RAM adapter + disk drive).
// Port of cores/c/src/fds.c.

import { Mapper, Mirroring } from '../mapper';
import { FdsAudio } from '../audio/fds_audio';

export class Fds extends Mapper {
    private static readonly PrgRamSize = 32 * 1024;
    private static readonly ChrRamSize = 8 * 1024;
    private static readonly BiosSize = 8 * 1024;

    private _prgRam: Uint8Array = new Uint8Array(Fds.PrgRamSize);
    private _bios: Uint8Array;
    private _chrRam: Uint8Array = new Uint8Array(Fds.ChrRamSize);
    private _diskData: Uint8Array | null;
    private _diskSize: number;
    private _diskReadPos: number;
    private _diskWritePos: number;
    private _diskMotorOn: boolean;
    private _diskTransferReset: boolean;
    private _diskWriteMode: boolean;
    private _diskDataAvailable: boolean;
    private _diskInserted: boolean;
    private _diskWriteLatch: number;
    private _ioEnableDisk: boolean;
    private _ioEnableTimer: boolean;
    private _timerLatch: number;
    private _timerCounter: number;
    private _timerEnable: boolean;
    private _timerIrqPending: boolean;
    private _mirrorHorizontal: boolean;
    private _audio: FdsAudio = new FdsAudio();

    constructor(bios: Uint8Array, biosLen: number, diskData: Uint8Array | null, diskLen: number) {
        super();
        this.mapperNum = 20;
        this._bios = new Uint8Array(Fds.BiosSize);
        const copyLen = Math.min(biosLen, Fds.BiosSize);
        if (copyLen > 0) this._bios.set(bios.subarray(0, copyLen));
        if (diskLen > 0 && diskData) {
            this._diskData = new Uint8Array(diskLen);
            this._diskData.set(diskData.subarray(0, diskLen));
            this._diskSize = diskLen;
        } else {
            this._diskData = null;
            this._diskSize = 0;
        }
        this._diskReadPos = 0;
        this._diskWritePos = 0;
        this._diskMotorOn = false;
        this._diskTransferReset = false;
        this._diskWriteMode = false;
        this._diskDataAvailable = false;
        this._diskInserted = true;
        this._diskWriteLatch = 0;
        this._ioEnableDisk = true;
        this._ioEnableTimer = true;
        this._timerLatch = 0;
        this._timerCounter = 0;
        this._timerEnable = false;
        this._timerIrqPending = false;
        this._mirrorHorizontal = false;
        this._audio.init();
    }

    private writeDiskRegister(addr: number, value: number): void {
        switch (addr) {
            case 0x4020: this._timerLatch = (this._timerLatch & 0xFF00) | value; break;
            case 0x4021: this._timerLatch = (this._timerLatch & 0x00FF) | (value << 8); break;
            case 0x4022:
                this._timerEnable = (value & 0x01) !== 0;
                if (!this._timerEnable) this._timerIrqPending = false;
                if (this._timerEnable) this._timerCounter = this._timerLatch;
                break;
            case 0x4023:
                this._ioEnableDisk = (value & 0x01) !== 0;
                this._ioEnableTimer = (value & 0x02) !== 0;
                if (!this._ioEnableTimer) this._timerIrqPending = false;
                break;
            case 0x4024:
                this._diskWriteLatch = value;
                if (this._diskWriteMode && this._ioEnableDisk) {
                    if (this._diskWritePos < this._diskSize && this._diskData) {
                        this._diskData[this._diskWritePos] = value;
                        this._diskWritePos += 1;
                    }
                }
                break;
            case 0x4025:
                if (!this._ioEnableDisk) return;
                const reset = (value & 0x01) !== 0;
                if (reset && !this._diskTransferReset) {
                    this._diskReadPos = 0;
                    this._diskWritePos = 0;
                    this._diskDataAvailable = false;
                }
                this._diskTransferReset = reset;
                this._mirrorHorizontal = (value & 0x02) !== 0;
                this._diskWriteMode = (value & 0x04) !== 0;
                this._diskMotorOn = (value & 0x20) !== 0;
                if (this._diskMotorOn && !this._diskWriteMode) {
                    this._diskDataAvailable = this._diskReadPos < this._diskSize;
                }
                break;
        }
    }

    private readDiskRegister(addr: number): number {
        switch (addr) {
            case 0x4030: {
                let status = 0;
                if (this._diskDataAvailable) status |= 0x01;
                if (this._diskMotorOn && this._diskInserted) status |= 0x04;
                if (this._diskTransferReset) status |= 0x10;
                if (this._timerIrqPending) status |= 0x80;
                this._timerIrqPending = false;
                return status;
            }
            case 0x4031: {
                if (this._diskReadPos < this._diskSize && this._diskData) {
                    const byte = this._diskData[this._diskReadPos];
                    this._diskReadPos += 1;
                    this._diskDataAvailable = this._diskReadPos < this._diskSize;
                    return byte;
                }
                return 0x00;
            }
            case 0x4032: return this._diskInserted ? 0x00 : 0x01;
            case 0x4033: return 0x80;
            default: return 0x00;
        }
    }

    readPrg(addr: number): number {
        if (addr >= 0x6000 && addr < 0xE000) return this._prgRam[(addr - 0x6000) & (Fds.PrgRamSize - 1)];
        if (addr >= 0xE000) return this._bios[(addr - 0xE000) & (Fds.BiosSize - 1)];
        return 0x00;
    }

    readPrgMut(addr: number): number {
        if (addr >= 0x6000 && addr < 0xE000) return this._prgRam[(addr - 0x6000) & (Fds.PrgRamSize - 1)];
        if (addr >= 0xE000) return this._bios[(addr - 0xE000) & (Fds.BiosSize - 1)];
        if (addr >= 0x4020 && addr <= 0x4033) return this.readDiskRegister(addr);
        if (addr === 0x4090 || addr === 0x4092) return this._audio.readRegister(addr);
        return 0x00;
    }

    writePrg(addr: number, value: number): void {
        if (addr >= 0x6000 && addr < 0xE000) { this._prgRam[(addr - 0x6000) & (Fds.PrgRamSize - 1)] = value; return; }
        if (addr >= 0x4020 && addr <= 0x4033) { this.writeDiskRegister(addr, value); return; }
        if (addr >= 0x4040 && addr <= 0x408A) this._audio.writeRegister(addr, value);
    }

    readChr(addr: number): number { return this._chrRam[addr & (Fds.ChrRamSize - 1)]; }
    writeChr(addr: number, value: number): void { this._chrRam[addr & (Fds.ChrRamSize - 1)] = value; }

    mirrorMode(): number { return this._mirrorHorizontal ? Mirroring.Horizontal : Mirroring.Vertical; }
    chrIsRam(): boolean { return true; }
    hasBattery(): boolean { return false; }
    irqPending(): boolean { return this._timerIrqPending; }

    clockCpu(cpuCycles: number): void {
        if (this._timerEnable && this._ioEnableTimer) {
            for (let i = 0; i < cpuCycles; ++i) {
                if (this._timerCounter === 0) {
                    this._timerCounter = this._timerLatch;
                    this._timerIrqPending = true;
                } else this._timerCounter = (this._timerCounter - 1) & 0xFFFF;
            }
        }
        this._audio.clock(Math.floor(cpuCycles / 2));
    }

    expansionAudioSample(): number { return this._audio.sample(); }

    saveState(buf: Uint8Array | null): number {
        const total = 64 + Fds.PrgRamSize + Fds.ChrRamSize + this._diskSize;
        if (buf === null) return total;
        let p = 0;
        new DataView(buf.buffer).setUint32(p, this._diskReadPos, true); p += 4;
        new DataView(buf.buffer).setUint32(p, this._diskWritePos, true); p += 4;
        new DataView(buf.buffer).setUint32(p, this._diskSize, true); p += 4;
        buf[p++] = this._diskMotorOn ? 1 : 0;
        buf[p++] = this._diskTransferReset ? 1 : 0;
        buf[p++] = this._diskWriteMode ? 1 : 0;
        buf[p++] = this._diskDataAvailable ? 1 : 0;
        buf[p++] = this._diskInserted ? 1 : 0;
        buf[p++] = this._diskWriteLatch;
        buf[p++] = this._ioEnableDisk ? 1 : 0;
        buf[p++] = this._ioEnableTimer ? 1 : 0;
        new DataView(buf.buffer).setUint16(p, this._timerLatch, true); p += 2;
        new DataView(buf.buffer).setUint16(p, this._timerCounter, true); p += 2;
        buf[p++] = this._timerEnable ? 1 : 0;
        buf[p++] = this._timerIrqPending ? 1 : 0;
        buf[p++] = this._mirrorHorizontal ? 1 : 0;
        while (p < 64) buf[p++] = 0;
        buf.set(this._prgRam, p); p += Fds.PrgRamSize;
        buf.set(this._chrRam, p); p += Fds.ChrRamSize;
        if (this._diskSize > 0 && this._diskData) {
            buf.set(this._diskData.subarray(0, this._diskSize), p);
            p += this._diskSize;
        }
        return p;
    }

    loadState(buf: Uint8Array, len: number): boolean {
        const need = 64 + Fds.PrgRamSize + Fds.ChrRamSize + this._diskSize;
        if (len < need) return false;
        let p = 0;
        this._diskReadPos = new DataView(buf.buffer).getUint32(p, true); p += 4;
        this._diskWritePos = new DataView(buf.buffer).getUint32(p, true); p += 4;
        const savedDiskSize = new DataView(buf.buffer).getUint32(p, true); p += 4;
        this._diskMotorOn = buf[p++] !== 0;
        this._diskTransferReset = buf[p++] !== 0;
        this._diskWriteMode = buf[p++] !== 0;
        this._diskDataAvailable = buf[p++] !== 0;
        this._diskInserted = buf[p++] !== 0;
        this._diskWriteLatch = buf[p++];
        this._ioEnableDisk = buf[p++] !== 0;
        this._ioEnableTimer = buf[p++] !== 0;
        this._timerLatch = new DataView(buf.buffer).getUint16(p, true); p += 2;
        this._timerCounter = new DataView(buf.buffer).getUint16(p, true); p += 2;
        this._timerEnable = buf[p++] !== 0;
        this._timerIrqPending = buf[p++] !== 0;
        this._mirrorHorizontal = buf[p++] !== 0;
        p = 64;
        this._prgRam.set(buf.subarray(p, p + Fds.PrgRamSize)); p += Fds.PrgRamSize;
        this._chrRam.set(buf.subarray(p, p + Fds.ChrRamSize)); p += Fds.ChrRamSize;
        if (this._diskSize > 0 && this._diskData && savedDiskSize === this._diskSize) {
            this._diskData.set(buf.subarray(p, p + this._diskSize));
            p += this._diskSize;
        }
        return true;
    }
}
