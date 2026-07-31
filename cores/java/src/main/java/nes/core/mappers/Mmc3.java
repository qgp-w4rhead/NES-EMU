package nes.core.mappers;

import nes.core.Mapper;
import nes.core.Mirroring;

// Mapper 4 (MMC3). Port of cores/csharp/src/Mappers/Mmc3.cs.
public final class Mmc3 extends Mapper {
    private static final int PRG_BANK_SIZE = 8192;
    private static final int CHR_1K_SIZE = 1024;
    private static final int CHR_2K_SIZE = 2048;
    private static final int PRG_RAM_SIZE = 8192;

    private final byte[] _prgRom;
    private final int _prgSize;
    private byte[] _chr;
    private int _chrSize;
    private boolean _chrIsRam;
    private final byte[] _prgRam = new byte[PRG_RAM_SIZE];
    private final boolean _hasBattery;
    private byte _bankSelect;
    private final byte[] _bankValues = new byte[8];
    private int _mirroring;
    private boolean _prgRamEnable;
    private boolean _prgRamWriteProtect;
    private byte _irqLatch;
    private byte _irqCounter;
    private boolean _irqReloadFlag;
    private boolean _irqEnable;
    private boolean _irqPending;

    public Mmc3(byte[] prg, int prgSize, byte[] chr, int chrSize, int mirroring, boolean hasBattery) {
        mapperNum = 4;
        _prgSize = prgSize;
        _prgRom = new byte[prgSize > 0 ? prgSize : 1];
        if (prgSize > 0) System.arraycopy(prg, 0, _prgRom, 0, prgSize);
        if (chrSize == 0) {
            _chrSize = CHR_2K_SIZE * 4;
            _chr = new byte[_chrSize];
            _chrIsRam = true;
        } else {
            _chrSize = chrSize;
            _chr = new byte[chrSize];
            System.arraycopy(chr, 0, _chr, 0, chrSize);
            _chrIsRam = false;
        }
        _hasBattery = hasBattery;
        _bankSelect = 0;
        _mirroring = mirroring;
        _prgRamEnable = false;
        _prgRamWriteProtect = false;
        _irqLatch = 0;
        _irqCounter = 0;
        _irqReloadFlag = false;
        _irqEnable = false;
        _irqPending = false;
    }

    private int prgBankCount() { return _prgSize / PRG_BANK_SIZE > 0 ? _prgSize / PRG_BANK_SIZE : 1; }
    private int chr1kCount() { return _chrSize / CHR_1K_SIZE > 0 ? _chrSize / CHR_1K_SIZE : 1; }
    private int prgMode() { return (_bankSelect >> 6) & 1; }
    private int chrMode() { return (_bankSelect >> 7) & 1; }

    private int prgReadBank(int bank, int offset) {
        int count = prgBankCount();
        bank %= count;
        int idx = bank * PRG_BANK_SIZE + offset;
        if (idx >= _prgSize) return 0x00;
        return _prgRom[idx] & 0xFF;
    }

    private int chrBankForSlot(int slot) {
        byte[] r = _bankValues;
        int cm = chrMode();
        if (cm == 0) {
            switch (slot) {
                case 0: return r[0] & 0xFE;
                case 1: return (r[0] & 0xFE) + 1;
                case 2: return r[1] & 0xFE;
                case 3: return (r[1] & 0xFE) + 1;
                case 4: return r[2] & 0xFF;
                case 5: return r[3] & 0xFF;
                case 6: return r[4] & 0xFF;
                case 7: return r[5] & 0xFF;
                default: return 0;
            }
        } else {
            switch (slot) {
                case 0: return r[2] & 0xFF;
                case 1: return r[3] & 0xFF;
                case 2: return r[4] & 0xFF;
                case 3: return r[5] & 0xFF;
                case 4: return r[0] & 0xFE;
                case 5: return (r[0] & 0xFE) + 1;
                case 6: return r[1] & 0xFE;
                case 7: return (r[1] & 0xFE) + 1;
                default: return 0;
            }
        }
    }

    private int chrIndex(int addr) {
        int slot = addr / CHR_1K_SIZE;
        int offset = addr & (CHR_1K_SIZE - 1);
        int bank = chrBankForSlot(slot);
        int count = chr1kCount();
        return (bank % count) * CHR_1K_SIZE + offset;
    }

    @Override
    public int readPrg(int addr) {
        if (addr >= 0x6000 && addr < 0x8000) {
            if (_prgRamEnable)
                return _prgRam[(addr - 0x6000) & (PRG_RAM_SIZE - 1)] & 0xFF;
            return 0x00;
        }
        int local = addr - 0x8000;
        int slot = local / PRG_BANK_SIZE;
        int offset = local & (PRG_BANK_SIZE - 1);
        int count = prgBankCount();
        int last = count - 1;
        int secondLast = count >= 2 ? count - 2 : 0;
        int bank;
        int pm = prgMode();
        if (slot == 0)
            bank = (pm == 0) ? (_bankValues[6] & 0xFF) % count : secondLast;
        else if (slot == 1)
            bank = (_bankValues[7] & 0xFF) % count;
        else if (slot == 2)
            bank = (pm == 0) ? secondLast : (_bankValues[6] & 0xFF) % count;
        else
            bank = last;
        return prgReadBank(bank, offset);
    }

    @Override
    public void writePrg(int addr, int value) {
        int v = value & 0xFF;
        if (addr >= 0x6000 && addr < 0x8000) {
            if (_prgRamEnable && !_prgRamWriteProtect)
                _prgRam[(addr - 0x6000) & (PRG_RAM_SIZE - 1)] = (byte) v;
            return;
        }
        if (addr >= 0x8000 && addr <= 0x9FFF) {
            if ((addr & 1) == 0) _bankSelect = (byte) v;
            else { int reg = _bankSelect & 0x07; _bankValues[reg] = (byte) v; }
        } else if (addr >= 0xA000 && addr <= 0xBFFF) {
            if ((addr & 1) == 0) _mirroring = (v & 1) == 0 ? Mirroring.VERTICAL : Mirroring.HORIZONTAL;
            else { _prgRamEnable = (v & 0x80) != 0; _prgRamWriteProtect = (v & 0x40) != 0; }
        } else if (addr >= 0xC000 && addr <= 0xDFFF) {
            if ((addr & 1) == 0) _irqLatch = (byte) v;
            else _irqReloadFlag = true;
        } else if (addr >= 0xE000 && addr <= 0xFFFF) {
            if ((addr & 1) == 0) { _irqEnable = false; _irqPending = false; }
            else _irqEnable = true;
        }
    }

    @Override
    public int readChr(int addr) {
        int idx = chrIndex(addr);
        if (idx >= _chrSize) return 0x00;
        return _chr[idx] & 0xFF;
    }

    @Override
    public void writeChr(int addr, int value) {
        if (!_chrIsRam) return;
        int idx = chrIndex(addr);
        if (idx < _chrSize) _chr[idx] = (byte) value;
    }

    @Override
    public int mirrorMode() { return _mirroring; }
    @Override
    public boolean chrIsRam() { return _chrIsRam; }
    @Override
    public boolean hasBattery() { return _hasBattery; }
    @Override
    public boolean irqPending() { return _irqPending; }

    @Override
    public void clockIrq() {
        if (_irqReloadFlag) {
            _irqCounter = _irqLatch;
            _irqReloadFlag = false;
        } else if (_irqCounter == 0) {
            _irqCounter = _irqLatch;
            if (_irqEnable) _irqPending = true;
        } else {
            _irqCounter--;
        }
    }

    @Override
    public int saveState(byte[] buf) {
        int total = 17 + PRG_RAM_SIZE + (_chrIsRam ? _chrSize : 0);
        if (buf == null) return total;
        int p = 0;
        buf[p++] = _bankSelect;
        for (int i = 0; i < 8; ++i) buf[p++] = _bankValues[i];
        buf[p++] = (byte) _mirroring;
        buf[p++] = (byte) (_prgRamEnable ? 1 : 0);
        buf[p++] = (byte) (_prgRamWriteProtect ? 1 : 0);
        buf[p++] = _irqLatch;
        buf[p++] = _irqCounter;
        buf[p++] = (byte) (_irqReloadFlag ? 1 : 0);
        buf[p++] = (byte) (_irqEnable ? 1 : 0);
        buf[p++] = (byte) (_irqPending ? 1 : 0);
        System.arraycopy(_prgRam, 0, buf, p, PRG_RAM_SIZE); p += PRG_RAM_SIZE;
        if (_chrIsRam && _chrSize > 0) {
            System.arraycopy(_chr, 0, buf, p, _chrSize);
            p += _chrSize;
        }
        return p;
    }

    @Override
    public boolean loadState(byte[] buf, int len) {
        int need = 17 + PRG_RAM_SIZE + (_chrIsRam ? _chrSize : 0);
        if (len < need) return false;
        int p = 0;
        _bankSelect = buf[p++];
        for (int i = 0; i < 8; ++i) _bankValues[i] = buf[p++];
        _mirroring = buf[p++] & 0xFF;
        _prgRamEnable = buf[p++] != 0;
        _prgRamWriteProtect = buf[p++] != 0;
        _irqLatch = buf[p++];
        _irqCounter = buf[p++];
        _irqReloadFlag = buf[p++] != 0;
        _irqEnable = buf[p++] != 0;
        _irqPending = buf[p++] != 0;
        System.arraycopy(buf, p, _prgRam, 0, PRG_RAM_SIZE); p += PRG_RAM_SIZE;
        if (_chrIsRam && _chrSize > 0) {
            System.arraycopy(buf, p, _chr, 0, _chrSize);
            p += _chrSize;
        }
        return true;
    }
}
