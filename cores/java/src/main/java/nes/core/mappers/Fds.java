package nes.core.mappers;

import nes.core.Mapper;
import nes.core.Mirroring;

// Mapper 20 (Famicom Disk System RAM adapter + disk drive).
// Port of cores/csharp/src/Mappers/Fds.cs.
public final class Fds extends Mapper {
    private static final int PRG_RAM_SIZE = 32 * 1024;
    private static final int CHR_RAM_SIZE = 8 * 1024;
    private static final int BIOS_SIZE = 8 * 1024;

    private final byte[] _prgRam = new byte[PRG_RAM_SIZE];
    private final byte[] _bios;
    private final byte[] _chrRam = new byte[CHR_RAM_SIZE];
    private byte[] _diskData;
    private int _diskSize;
    private int _diskReadPos;
    private int _diskWritePos;
    private boolean _diskMotorOn;
    private boolean _diskTransferReset;
    private boolean _diskWriteMode;
    private boolean _diskDataAvailable;
    private boolean _diskInserted;
    private byte _diskWriteLatch;
    private boolean _ioEnableDisk;
    private boolean _ioEnableTimer;
    private int _timerLatch;
    private int _timerCounter;
    private boolean _timerEnable;
    private boolean _timerIrqPending;
    private boolean _mirrorHorizontal;
    private final FdsAudio _audio = new FdsAudio();

    public Fds(byte[] bios, int biosLen, byte[] diskData, int diskLen) {
        mapperNum = 20;
        _bios = new byte[BIOS_SIZE];
        int copy = biosLen;
        if (copy > BIOS_SIZE) copy = BIOS_SIZE;
        if (copy > 0 && bios != null) System.arraycopy(bios, 0, _bios, 0, copy);
        if (diskLen > 0 && diskData != null) {
            _diskData = new byte[diskLen];
            System.arraycopy(diskData, 0, _diskData, 0, diskLen);
            _diskSize = diskLen;
        }
        _diskInserted = true;
        _ioEnableDisk = true;
        _ioEnableTimer = true;
    }

    private void writeDiskRegister(int addr, int value) {
        switch (addr) {
            case 0x4020: _timerLatch = (_timerLatch & 0xFF00) | value; break;
            case 0x4021: _timerLatch = (_timerLatch & 0x00FF) | (value << 8); break;
            case 0x4022:
                _timerEnable = (value & 0x01) != 0;
                if (!_timerEnable) _timerIrqPending = false;
                if (_timerEnable) _timerCounter = _timerLatch;
                break;
            case 0x4023:
                _ioEnableDisk = (value & 0x01) != 0;
                _ioEnableTimer = (value & 0x02) != 0;
                if (!_ioEnableTimer) _timerIrqPending = false;
                break;
            case 0x4024:
                _diskWriteLatch = (byte) value;
                if (_diskWriteMode && _ioEnableDisk) {
                    if (_diskWritePos < _diskSize) {
                        _diskData[_diskWritePos] = (byte) value;
                        _diskWritePos++;
                    }
                }
                break;
            case 0x4025:
                if (!_ioEnableDisk) return;
                boolean reset = (value & 0x01) != 0;
                if (reset && !_diskTransferReset) {
                    _diskReadPos = 0;
                    _diskWritePos = 0;
                    _diskDataAvailable = false;
                }
                _diskTransferReset = reset;
                _mirrorHorizontal = (value & 0x02) != 0;
                _diskWriteMode = (value & 0x04) != 0;
                _diskMotorOn = (value & 0x20) != 0;
                if (_diskMotorOn && !_diskWriteMode)
                    _diskDataAvailable = _diskReadPos < _diskSize;
                break;
        }
    }

    private int readDiskRegister(int addr) {
        switch (addr) {
            case 0x4030: {
                int status = 0;
                if (_diskDataAvailable) status |= 0x01;
                if (_diskMotorOn && _diskInserted) status |= 0x04;
                if (_diskTransferReset) status |= 0x10;
                if (_timerIrqPending) status |= 0x80;
                _timerIrqPending = false;
                return status;
            }
            case 0x4031: {
                if (_diskReadPos < _diskSize) {
                    int b = _diskData[_diskReadPos] & 0xFF;
                    _diskReadPos++;
                    _diskDataAvailable = _diskReadPos < _diskSize;
                    return b;
                }
                return 0x00;
            }
            case 0x4032: return _diskInserted ? 0x00 : 0x01;
            case 0x4033: return 0x80;
            default: return 0x00;
        }
    }

    @Override
    public int readPrg(int addr) {
        if (addr >= 0x6000 && addr < 0xE000)
            return _prgRam[(addr - 0x6000) & (PRG_RAM_SIZE - 1)] & 0xFF;
        if (addr >= 0xE000)
            return _bios[(addr - 0xE000) & (BIOS_SIZE - 1)] & 0xFF;
        return 0x00;
    }

    @Override
    public int readPrgMut(int addr) {
        if (addr >= 0x6000 && addr < 0xE000)
            return _prgRam[(addr - 0x6000) & (PRG_RAM_SIZE - 1)] & 0xFF;
        if (addr >= 0xE000)
            return _bios[(addr - 0xE000) & (BIOS_SIZE - 1)] & 0xFF;
        if (addr >= 0x4020 && addr <= 0x4033) return readDiskRegister(addr);
        if (addr == 0x4090 || addr == 0x4092) return _audio.readRegister(addr);
        return 0x00;
    }

    @Override
    public void writePrg(int addr, int value) {
        if (addr >= 0x6000 && addr < 0xE000) {
            _prgRam[(addr - 0x6000) & (PRG_RAM_SIZE - 1)] = (byte) value;
            return;
        }
        if (addr >= 0x4020 && addr <= 0x4033) { writeDiskRegister(addr, value); return; }
        if (addr >= 0x4040 && addr <= 0x408A) _audio.writeRegister(addr, value);
    }

    @Override
    public int readChr(int addr) { return _chrRam[addr & (CHR_RAM_SIZE - 1)] & 0xFF; }
    @Override
    public void writeChr(int addr, int value) { _chrRam[addr & (CHR_RAM_SIZE - 1)] = (byte) value; }

    @Override
    public int mirrorMode() { return _mirrorHorizontal ? Mirroring.HORIZONTAL : Mirroring.VERTICAL; }
    @Override
    public boolean chrIsRam() { return true; }
    @Override
    public boolean hasBattery() { return false; }
    @Override
    public boolean irqPending() { return _timerIrqPending; }

    @Override
    public void clockCpu(int cpuCycles) {
        if (_timerEnable && _ioEnableTimer) {
            for (int i = 0; i < cpuCycles; ++i) {
                if (_timerCounter == 0) {
                    _timerCounter = _timerLatch;
                    _timerIrqPending = true;
                } else {
                    _timerCounter--;
                }
            }
        }
        _audio.clock(cpuCycles / 2);
    }

    @Override
    public float expansionAudioSample() { return _audio.sample(); }
}
