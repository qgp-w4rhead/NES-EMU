package nes.core.mappers;

import nes.core.Mapper;
import nes.core.Mirroring;

// Mappers 24 (VRC6a) and 26 (VRC6b). Port of cores/csharp/src/Mappers/Vrc6.cs.
public final class Vrc6 extends Mapper {
    private static final int PRG_16K_SIZE = 16384;
    private static final int PRG_8K_SIZE = 8192;
    private static final int CHR_1K_SIZE = 1024;
    private static final int PRG_RAM_SIZE = 8192;

    private static final class PulseChan {
        int control;
        int period;
        int timer;
        int step;
        boolean enabled;
        int scale;
    }

    private static final class SawChan {
        int rate;
        int period;
        int timer;
        int accum;
        boolean enabled;
    }

    private final byte[] _prgRom;
    private final int _prgSize;
    private byte[] _chr;
    private int _chrSize;
    private boolean _chrIsRam;
    private final byte[] _prgRam = new byte[PRG_RAM_SIZE];
    private final boolean _hasBattery;
    private byte _prgBank16k;
    private byte _prgBank8k;
    private final byte[] _chrBanks = new byte[8];
    private boolean _mirrorHorizontal;
    private byte _irqLatch;
    private byte _irqCounter;
    private boolean _irqEnable;
    private boolean _irqEnableAfterAck;
    private boolean _irqPending;
    private final PulseChan _pulse1 = new PulseChan();
    private final PulseChan _pulse2 = new PulseChan();
    private final SawChan _saw = new SawChan();
    private final boolean _swapAddr;

    public Vrc6(byte[] prg, int prgSize, byte[] chr, int chrSize, int mirroring, boolean hasBattery, boolean is26) {
        mapperNum = is26 ? 26 : 24;
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
        _mirrorHorizontal = (mirroring == Mirroring.HORIZONTAL);
        _swapAddr = is26;
    }

    private int prg16kCount() { return _prgSize / PRG_16K_SIZE > 0 ? _prgSize / PRG_16K_SIZE : 1; }
    private int prg8kCount() { return _prgSize / PRG_8K_SIZE > 0 ? _prgSize / PRG_8K_SIZE : 1; }
    private int chr1kCount() { return _chrSize / CHR_1K_SIZE > 0 ? _chrSize / CHR_1K_SIZE : 1; }

    private int decode(int addr) {
        int masked = addr & 0xF003;
        if (!_swapAddr) return masked;
        int a0 = (masked & 0x001) << 1;
        int a1 = (masked & 0x002) >> 1;
        return (masked & 0xFFFC) | a0 | a1;
    }

    private int prgRead8k(int bank, int offset) {
        int count = prg8kCount();
        bank %= count;
        int idx = bank * PRG_8K_SIZE + offset;
        if (idx >= _prgSize) return 0x00;
        return _prgRom[idx] & 0xFF;
    }

    private int prgRead16k(int bank, int offset) {
        int count = prg16kCount();
        bank %= count;
        int idx = bank * PRG_16K_SIZE + offset;
        if (idx >= _prgSize) return 0x00;
        return _prgRom[idx] & 0xFF;
    }

    private static int pulseDuty(PulseChan p) { return ((p.control >> 4) & 0x07) + 1; }
    private static int pulseVolume(PulseChan p) { return p.control & 0x0F; }

    private static void pulseClock(PulseChan p, int cpuCycles) {
        if (!p.enabled) return;
        int period = p.period;
        if ((p.scale & 1) != 0) {
            period = period * 2;
            if (period == 0) period = 1;
        }
        if (period == 0) period = 1;
        for (int i = 0; i < cpuCycles; ++i) {
            if (p.timer == 0) {
                p.timer = period;
                p.step = (p.step + 1) & 0x0F;
            } else {
                p.timer--;
            }
        }
    }

    private static void sawClock(SawChan saw, int cpuCycles) {
        if (!saw.enabled) return;
        int period = saw.period;
        if (period == 0) period = 1;
        for (int i = 0; i < cpuCycles; ++i) {
            if (saw.timer == 0) {
                saw.timer = period;
                saw.accum = saw.accum + (saw.rate & 0x3F);
                if (saw.accum >= 0x80) saw.accum = 0;
            } else {
                saw.timer--;
            }
        }
    }

    private int pulse1Sample() { return (_pulse1.enabled && _pulse1.step < pulseDuty(_pulse1)) ? pulseVolume(_pulse1) : 0; }
    private int pulse2Sample() { return (_pulse2.enabled && _pulse2.step < pulseDuty(_pulse2)) ? pulseVolume(_pulse2) : 0; }
    private int sawSample() { return _saw.enabled ? (_saw.accum >> 2) : 0; }

    @Override
    public int readPrg(int addr) {
        if (addr >= 0x6000 && addr < 0x8000)
            return _prgRam[(addr - 0x6000) & (PRG_RAM_SIZE - 1)] & 0xFF;
        int local = addr - 0x8000;
        if (local < PRG_16K_SIZE) {
            int bank = (_prgBank16k & 0xFF) % prg16kCount();
            return prgRead16k(bank, local);
        }
        if (local < PRG_16K_SIZE + PRG_8K_SIZE) {
            int off = local - PRG_16K_SIZE;
            int bank = (_prgBank8k & 0xFF) % prg8kCount();
            return prgRead8k(bank, off);
        }
        int off2 = local - PRG_16K_SIZE - PRG_8K_SIZE;
        int last = prg8kCount() - 1;
        return prgRead8k(last, off2);
    }

    @Override
    public void writePrg(int addr, int value) {
        if (addr >= 0x6000 && addr < 0x8000) {
            _prgRam[(addr - 0x6000) & (PRG_RAM_SIZE - 1)] = (byte) value;
            return;
        }
        int reg = decode(addr);
        switch (reg) {
            case 0x8000: _prgBank16k = (byte) (value & 0x3F); break;
            case 0x9000: _pulse1.control = value; break;
            case 0x9001: _pulse1.period = (_pulse1.period & 0x0F00) | value; break;
            case 0x9002:
                _pulse1.period = (_pulse1.period & 0x00FF) | ((value & 0x0F) << 8);
                _pulse1.enabled = (value & 0x80) != 0;
                break;
            case 0x9003: _pulse1.scale = value; break;
            case 0xA000: _pulse2.control = value; break;
            case 0xA001: _pulse2.period = (_pulse2.period & 0x0F00) | value; break;
            case 0xA002:
                _pulse2.period = (_pulse2.period & 0x00FF) | ((value & 0x0F) << 8);
                _pulse2.enabled = (value & 0x80) != 0;
                break;
            case 0xB000: _saw.rate = value; break;
            case 0xB001: _saw.period = (_saw.period & 0x0F00) | value; break;
            case 0xB002:
                _saw.period = (_saw.period & 0x00FF) | ((value & 0x0F) << 8);
                _saw.enabled = (value & 0x80) != 0;
                break;
            case 0xB003: _mirrorHorizontal = (value & 0x01) != 0; break;
            case 0xC000: _prgBank8k = (byte) (value & 0x3F); break;
            case 0xD000: _chrBanks[0] = (byte) value; break;
            case 0xD001: _chrBanks[1] = (byte) value; break;
            case 0xD002: _chrBanks[2] = (byte) value; break;
            case 0xD003: _chrBanks[3] = (byte) value; break;
            case 0xE000: _chrBanks[4] = (byte) value; break;
            case 0xE001: _chrBanks[5] = (byte) value; break;
            case 0xE002: _chrBanks[6] = (byte) value; break;
            case 0xE003: _chrBanks[7] = (byte) value; break;
            case 0xF000: _irqLatch = (byte) value; break;
            case 0xF001:
                _irqEnable = (value & 0x01) != 0;
                _irqCounter = _irqLatch;
                break;
            case 0xF002:
                _irqEnableAfterAck = (value & 0x02) != 0;
                _irqEnable = (value & 0x01) != 0;
                _irqPending = false;
                break;
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
    public int mirrorMode() { return _mirrorHorizontal ? Mirroring.HORIZONTAL : Mirroring.VERTICAL; }
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
                if (_irqEnable) {
                    _irqPending = true;
                    if (!_irqEnableAfterAck) _irqEnable = false;
                }
            } else {
                _irqCounter--;
            }
        }
        pulseClock(_pulse1, cpuCycles);
        pulseClock(_pulse2, cpuCycles);
        sawClock(_saw, cpuCycles);
    }

    @Override
    public float expansionAudioSample() {
        int p1 = pulse1Sample();
        int p2 = pulse2Sample();
        int saw = sawSample();
        int sum = p1 + p2;
        sum = (sum > 63) ? 63 : (sum + saw);
        if (sum > 63) sum = 63;
        final float VRC6_GAIN = 0.75f;
        float normalized = (sum / 63.0f) * 2.0f - 1.0f;
        return normalized * VRC6_GAIN;
    }
}
