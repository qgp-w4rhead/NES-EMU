package nes.core.mappers;

import nes.core.Mapper;
import nes.core.Mirroring;

// Mapper 85 (VRC7 with YM2413 OPLL audio). Port of cores/csharp/src/Mappers/Vrc7.cs.
public final class Vrc7 extends Mapper {
    private static final int PRG_8K_SIZE = 8192;
    private static final int CHR_1K_SIZE = 1024;
    private static final int PRG_RAM_SIZE = 8192;

    private final byte[] _prgRom;
    private final int _prgSize;
    private byte[] _chr;
    private int _chrSize;
    private boolean _chrIsRam;
    private final byte[] _prgRam = new byte[PRG_RAM_SIZE];
    private final boolean _hasBattery;
    private final byte[] _prgBanks = new byte[3];
    private final byte[] _chrBanks = new byte[8];
    private boolean _mirrorHorizontal;
    private int _irqLatch;
    private int _irqCounter;
    private boolean _irqLatchHigh;
    private boolean _irqEnable;
    private boolean _irqPending;
    private final Opll _opll = new Opll();

    public Vrc7(byte[] prg, int prgSize, byte[] chr, int chrSize, int mirroring, boolean hasBattery) {
        mapperNum = 85;
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

    @Override
    public int readPrg(int addr) {
        if (addr >= 0x6000 && addr < 0x8000)
            return _prgRam[(addr - 0x6000) & (PRG_RAM_SIZE - 1)] & 0xFF;
        int local = addr - 0x8000;
        int slot = local / PRG_8K_SIZE;
        int offset = local & (PRG_8K_SIZE - 1);
        if (slot < 3) {
            int bank = (_prgBanks[slot] & 0xFF) % prg8kCount();
            return prgReadBank(bank, offset);
        }
        int last = prg8kCount() - 1;
        return prgReadBank(last, offset);
    }

    @Override
    public void writePrg(int addr, int value) {
        if (addr >= 0x6000 && addr < 0x8000) {
            _prgRam[(addr - 0x6000) & (PRG_RAM_SIZE - 1)] = (byte) value;
            return;
        }
        int reg = addr & 0xF03D;
        switch (reg) {
            case 0x8000: _prgBanks[0] = (byte) (value & 0x3F); break;
            case 0x8008: _prgBanks[1] = (byte) (value & 0x3F); break;
            case 0x9000: _prgBanks[2] = (byte) (value & 0x3F); break;
            case 0x9010: _opll.writeAddr(value); break;
            case 0x9030: _opll.writeData(value); break;
            case 0xB000: _mirrorHorizontal = (value & 0x01) != 0; break;
            case 0xC000: _chrBanks[0] = (byte) value; break;
            case 0xC004: _chrBanks[1] = (byte) value; break;
            case 0xC008: _chrBanks[2] = (byte) value; break;
            case 0xC00C: _chrBanks[3] = (byte) value; break;
            case 0xD000: _chrBanks[4] = (byte) value; break;
            case 0xD004: _chrBanks[5] = (byte) value; break;
            case 0xD008: _chrBanks[6] = (byte) value; break;
            case 0xD00C: _chrBanks[7] = (byte) value; break;
            case 0xE000:
                if (!_irqLatchHigh) {
                    _irqLatch = (_irqLatch & 0xFF00) | value;
                    _irqLatchHigh = true;
                } else {
                    _irqLatch = (_irqLatch & 0x00FF) | (value << 8);
                    _irqLatchHigh = false;
                }
                break;
            case 0xE008:
                if ((value & 0x02) != 0) {
                    _irqEnable = true;
                    _irqPending = false;
                    _irqCounter = _irqLatch;
                } else if ((value & 0x01) != 0) {
                    _irqEnable = true;
                    _irqCounter = _irqLatch;
                } else {
                    _irqEnable = false;
                }
                break;
            case 0xE010: _irqPending = false; break;
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
                if (_irqEnable) _irqPending = true;
            } else {
                _irqCounter--;
            }
        }
        _opll.clock(cpuCycles / 2);
    }

    @Override
    public float expansionAudioSample() {
        final float VRC7_GAIN = 0.6f;
        return _opll.sample() * VRC7_GAIN;
    }
}
