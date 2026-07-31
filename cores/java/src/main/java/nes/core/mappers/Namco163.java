package nes.core.mappers;

import nes.core.Mapper;
import nes.core.Mirroring;

// Mapper 19 (Namco 163 with wavetable expansion audio).
// Port of cores/csharp/src/Mappers/Namco163.cs.
public final class Namco163 extends Mapper {
    private static final int PRG_8K_SIZE = 8192;
    private static final int CHR_1K_SIZE = 1024;
    private static final int PRG_RAM_SIZE = 8192;
    private static final int WAVE_RAM_SIZE = 0x80;
    private static final int WAVE_CHANNELS = 8;

    private static final class WaveChan {
        int freq;
        int length;
        int volume;
        int offset;
        int phase;
        boolean enabled;
    }

    private final byte[] _prgRom;
    private final int _prgSize;
    private byte[] _chr;
    private int _chrSize;
    private boolean _chrIsRam;
    private final byte[] _prgRam = new byte[PRG_RAM_SIZE];
    private final boolean _hasBattery;
    private final byte[] _prgBanks = new byte[4];
    private final byte[] _chrBanks = new byte[8];
    private int _mirror;
    private boolean _prgRamEnable;
    private boolean _prgRamWriteProtect;
    private int _irqLatch;
    private int _irqCounter;
    private boolean _irqEnable;
    private boolean _irqPending;
    private boolean _irqLatchHigh;
    private final byte[] _waveRam = new byte[WAVE_RAM_SIZE];
    private byte _waveAddr;
    private final WaveChan[] _chans = new WaveChan[WAVE_CHANNELS];

    public Namco163(byte[] prg, int prgSize, byte[] chr, int chrSize, int mirroring, boolean hasBattery) {
        mapperNum = 19;
        _prgSize = prgSize;
        _prgRom = new byte[prgSize > 0 ? prgSize : 1];
        if (prgSize > 0) System.arraycopy(prg, 0, _prgRom, 0, prgSize);
        if (chrSize > 0) {
            _chrSize = chrSize;
            _chr = new byte[chrSize];
            System.arraycopy(chr, 0, _chr, 0, chrSize);
            _chrIsRam = false;
        } else {
            _chrSize = 8192;
            _chr = new byte[8192];
            _chrIsRam = true;
        }
        _hasBattery = hasBattery;
        _mirror = mirroring;
        for (int i = 0; i < WAVE_CHANNELS; ++i) _chans[i] = new WaveChan();
    }

    private int prg8kCount() { return _prgSize / PRG_8K_SIZE > 0 ? _prgSize / PRG_8K_SIZE : 1; }
    private int chr1kCount() { return _chrSize / CHR_1K_SIZE > 0 ? _chrSize / CHR_1K_SIZE : 1; }

    private int prgReadBank(int bank, int offset) {
        int count = prg8kCount();
        bank %= count;
        int idx = bank * PRG_8K_SIZE + offset;
        if (idx >= _prgSize) return 0x00;
        return _prgRom[idx] & 0xFF;
    }

    private int waveRead() { return _waveRam[_waveAddr & 0x7F] & 0xFF; }

    private void waveWrite(int value) {
        int a = _waveAddr & 0x7F;
        _waveRam[a] = (byte) value;
        if ((_waveAddr & 0x80) != 0)
            _waveAddr = (byte) ((_waveAddr & 0x80) | ((_waveAddr + 1) & 0x7F));
    }

    private int chanParamRead(int addr) {
        int off = addr - 0x5000;
        int ch = off / 8;
        int sub = off & 0x07;
        if (ch >= WAVE_CHANNELS) return 0;
        WaveChan c = _chans[ch];
        switch (sub) {
            case 0: return c.freq & 0xFF;
            case 1: return ((c.freq >> 8) & 0x0F) | ((c.length & 0x0F) << 4);
            case 2: return c.volume & 0x0F;
            case 3: return (c.offset & 0x60) | (c.enabled ? 0x80 : 0x00);
            default: return 0;
        }
    }

    private void chanParamWrite(int addr, int value) {
        int off = addr - 0x5000;
        int ch = off / 8;
        int sub = off & 0x07;
        if (ch >= WAVE_CHANNELS) return;
        WaveChan c = _chans[ch];
        switch (sub) {
            case 0: c.freq = (c.freq & 0x0F00) | value; break;
            case 1:
                c.freq = (c.freq & 0x00FF) | ((value & 0x0F) << 8);
                c.length = (value >> 4) & 0x0F;
                break;
            case 2: c.volume = value & 0x0F; break;
            case 3:
                c.offset = value & 0x60;
                c.enabled = (value & 0x80) != 0;
                break;
        }
    }

    private int chanSample(int ch) {
        WaveChan c = _chans[ch];
        if (!c.enabled) return 0;
        int lengthNibbles = (c.length + 1) * 8;
        if (lengthNibbles == 0) lengthNibbles = 1;
        int phaseNibble = (c.phase >>> 16) % lengthNibbles;
        int byteIdx = (c.offset + phaseNibble / 2) & 0x7F;
        int b = _waveRam[byteIdx] & 0xFF;
        int nibble = (phaseNibble & 1) != 0 ? (b >> 4) : (b & 0x0F);
        return (nibble * c.volume) / 15;
    }

    @Override
    public int readPrg(int addr) {
        if (addr >= 0x6000 && addr < 0x8000) {
            if (_prgRamEnable) return _prgRam[(addr - 0x6000) & (PRG_RAM_SIZE - 1)] & 0xFF;
            return 0x00;
        }
        if (addr >= 0x4800 && addr < 0x5000) return waveRead();
        if (addr >= 0x5000 && addr < 0x5800) return chanParamRead(addr);
        int local = addr - 0x8000;
        int slot = local / PRG_8K_SIZE;
        int offset = local & (PRG_8K_SIZE - 1);
        int bank = (_prgBanks[slot] & 0xFF) % prg8kCount();
        return prgReadBank(bank, offset);
    }

    @Override
    public void writePrg(int addr, int value) {
        if (addr >= 0x6000 && addr < 0x8000) {
            if (_prgRamEnable && !_prgRamWriteProtect)
                _prgRam[(addr - 0x6000) & (PRG_RAM_SIZE - 1)] = (byte) value;
            return;
        }
        if (addr >= 0x4800 && addr < 0x5000) { waveWrite(value); return; }
        if (addr >= 0x5000 && addr < 0x5800) { chanParamWrite(addr, value); return; }
        int reg = addr & 0xF801;
        switch (reg) {
            case 0x8000: _chrBanks[0] = (byte) value; break;
            case 0x8800: _chrBanks[1] = (byte) value; break;
            case 0x9000: _chrBanks[2] = (byte) value; break;
            case 0x9800: _chrBanks[3] = (byte) value; break;
            case 0xA000: _chrBanks[4] = (byte) value; break;
            case 0xA800: _chrBanks[5] = (byte) value; break;
            case 0xB000: _chrBanks[6] = (byte) value; break;
            case 0xB800: _chrBanks[7] = (byte) value; break;
            case 0xC000: _prgBanks[0] = (byte) (value & 0x3F); break;
            case 0xC800: _prgBanks[1] = (byte) (value & 0x3F); break;
            case 0xD000: _prgBanks[2] = (byte) (value & 0x3F); break;
            case 0xD800: _prgBanks[3] = (byte) (value & 0x3F); break;
            case 0xE000:
                switch (value & 0x03) {
                    case 0: _mirror = Mirroring.VERTICAL; break;
                    case 1: _mirror = Mirroring.HORIZONTAL; break;
                    case 2: _mirror = Mirroring.SINGLE_SCREEN0; break;
                    default: _mirror = Mirroring.SINGLE_SCREEN1; break;
                }
                break;
            case 0xE800:
                _prgRamEnable = (value & 0x80) != 0;
                _prgRamWriteProtect = (value & 0x40) != 0;
                break;
            case 0xF000:
                if (!_irqLatchHigh) {
                    _irqLatch = (_irqLatch & 0xFF00) | value;
                    _irqLatchHigh = true;
                } else {
                    _irqLatch = (_irqLatch & 0x00FF) | (value << 8);
                    _irqLatchHigh = false;
                }
                break;
            case 0xF800: _waveAddr = (byte) value; break;
        }
    }

    @Override
    public int readChr(int addr) {
        int slot = addr / CHR_1K_SIZE;
        int offset = addr & (CHR_1K_SIZE - 1);
        int count = chr1kCount();
        int bank = (_chrBanks[slot] & 0xFF) % count;
        int idx = bank * CHR_1K_SIZE + offset;
        if (idx >= _chrSize) return 0x00;
        return _chr[idx] & 0xFF;
    }

    @Override
    public void writeChr(int addr, int value) {
        if (!_chrIsRam) return;
        int slot = addr / CHR_1K_SIZE;
        int offset = addr & (CHR_1K_SIZE - 1);
        int count = chr1kCount();
        int bank = (_chrBanks[slot] & 0xFF) % count;
        int idx = bank * CHR_1K_SIZE + offset;
        if (idx < _chrSize) _chr[idx] = (byte) value;
    }

    @Override
    public int mirrorMode() { return _mirror; }
    @Override
    public boolean chrIsRam() { return _chrIsRam; }
    @Override
    public boolean hasBattery() { return _hasBattery; }
    @Override
    public boolean irqPending() { return _irqPending; }

    @Override
    public void clockCpu(int cpuCycles) {
        for (int i = 0; i < cpuCycles; ++i) {
            if (_irqCounter == 0) {
                _irqCounter = _irqLatch;
                if (_irqEnable) _irqPending = true;
            } else {
                _irqCounter--;
            }
        }
        int active = 0;
        for (int ch = 0; ch < WAVE_CHANNELS; ++ch)
            if (_chans[ch].enabled) ++active;
        if (active == 0) active = 1;
        for (int i = 0; i < cpuCycles; ++i) {
            for (int ch = 0; ch < WAVE_CHANNELS; ++ch) {
                WaveChan c = _chans[ch];
                if (!c.enabled) continue;
                int inc = c.freq << 8;
                c.phase = c.phase + (inc / active);
            }
        }
    }

    @Override
    public float expansionAudioSample() {
        int sum = 0;
        for (int ch = 0; ch < WAVE_CHANNELS; ++ch)
            sum += chanSample(ch);
        final float N163_GAIN = 0.4f;
        float v = sum / 120.0f;
        if (v < -1.0f) v = -1.0f;
        if (v > 1.0f) v = 1.0f;
        return v * N163_GAIN;
    }
}
