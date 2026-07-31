package nes.core.mappers;

import nes.core.Mapper;
import nes.core.Mirroring;

// Mapper 69 (FME-7 / Sunsoft 5B). Port of cores/csharp/src/Mappers/Fme7.cs.
public final class Fme7 extends Mapper {
    private static final int PRG_BANK_SIZE = 8192;
    private static final int CHR_1K_SIZE = 1024;
    private static final int PRG_RAM_SIZE = 8192;

    private final byte[] _prgRom;
    private final int _prgSize;
    private byte[] _chr;
    private int _chrSize;
    private boolean _chrIsRam;
    private final byte[] _prgRam = new byte[PRG_RAM_SIZE];
    private final boolean _hasBattery;
    private byte _command;
    private final byte[] _prgBanks = new byte[4];
    private final byte[] _chrBanks = new byte[8];
    private boolean _mirrorHorizontal;
    private boolean _prgRamEnable;
    private boolean _prgRamWriteProtect;
    private int _irqLatch;
    private int _irqCounter;
    private boolean _irqLatchHigh;
    private boolean _irqEnable;
    private boolean _irqPending;
    private final Ym2149 _ym = new Ym2149();

    public Fme7(byte[] prg, int prgSize, byte[] chr, int chrSize, int mirroring, boolean hasBattery) {
        mapperNum = 69;
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

    private int prgBankCount() { return _prgSize / PRG_BANK_SIZE > 0 ? _prgSize / PRG_BANK_SIZE : 1; }
    private int chr1kCount() { return _chrSize / CHR_1K_SIZE > 0 ? _chrSize / CHR_1K_SIZE : 1; }

    private int prgReadBank(int bank, int offset) {
        int count = prgBankCount();
        bank %= count;
        int idx = bank * PRG_BANK_SIZE + offset;
        if (idx >= _prgSize) return 0x00;
        return _prgRom[idx] & 0xFF;
    }

    private void writeData(int value) {
        int cmd = _command & 0x0F;
        if (cmd <= 7) {
            _chrBanks[cmd & 0x07] = (byte) value;
        } else {
            switch (cmd) {
                case 8: _prgBanks[0] = (byte) (value & 0x3F); break;
                case 9: _prgBanks[1] = (byte) (value & 0x3F); break;
                case 10: _prgBanks[2] = (byte) (value & 0x3F); break;
                case 11: _prgBanks[3] = (byte) (value & 0x3F); break;
                case 12: _mirrorHorizontal = (value & 0x01) != 0; break;
                case 13:
                    _prgRamEnable = (value & 0x80) != 0;
                    _prgRamWriteProtect = (value & 0x40) != 0;
                    break;
                case 14:
                    if (!_irqLatchHigh) {
                        _irqLatch = (_irqLatch & 0xFF00) | value;
                        _irqLatchHigh = true;
                    } else {
                        _irqLatch = (_irqLatch & 0x00FF) | (value << 8);
                        _irqLatchHigh = false;
                    }
                    break;
                case 15:
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
            }
        }
    }

    @Override
    public int readPrg(int addr) {
        if (addr >= 0x6000 && addr < 0x8000) {
            if (_prgRamEnable) return _prgRam[(addr - 0x6000) & (PRG_RAM_SIZE - 1)] & 0xFF;
            return 0x00;
        }
        int local = addr - 0x8000;
        int slot = local / PRG_BANK_SIZE;
        int offset = local & (PRG_BANK_SIZE - 1);
        int bank = (_prgBanks[slot] & 0xFF) % prgBankCount();
        return prgReadBank(bank, offset);
    }

    @Override
    public void writePrg(int addr, int value) {
        if (addr >= 0x6000 && addr < 0x8000) {
            if (_prgRamEnable && !_prgRamWriteProtect)
                _prgRam[(addr - 0x6000) & (PRG_RAM_SIZE - 1)] = (byte) value;
            return;
        }
        switch (addr) {
            case 0x8000: _command = (byte) value; break;
            case 0x8001: writeData(value); break;
            case 0xC000: _ym.writeAddr(value); break;
            case 0xE000: _ym.writeData(value); break;
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
        _ym.clock(cpuCycles / 2);
    }

    @Override
    public float expansionAudioSample() {
        final float S5B_GAIN = 0.5f;
        return _ym.sample() * S5B_GAIN;
    }
}
