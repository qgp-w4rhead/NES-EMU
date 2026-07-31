package nes.core.mappers;

import nes.core.Mapper;
import nes.core.Mirroring;

// Mapper 1 (MMC1). Port of cores/csharp/src/Mappers/Mmc1.cs.
public final class Mmc1 extends Mapper {
    private static final int PRG_BANK_SIZE = 16384;
    private static final int CHR_4K_SIZE = 4096;
    private static final int CHR_8K_SIZE = 8192;
    private static final int PRG_RAM_SIZE = 8192;
    private static final byte CONTROL_PRG_MODE3 = 0x0C;

    private final byte[] _prgRom;
    private final int _prgSize;
    private byte[] _chr;
    private int _chrSize;
    private boolean _chrIsRam;
    private final byte[] _prgRam = new byte[PRG_RAM_SIZE];
    private final boolean _hasBattery;
    private byte _shiftReg;
    private byte _shiftCount;
    private byte _control;
    private byte _chrBank0;
    private byte _chrBank1;
    private byte _prgBank;

    public Mmc1(byte[] prg, int prgSize, byte[] chr, int chrSize, int mirroring, boolean hasBattery) {
        mapperNum = 1;
        _prgSize = prgSize;
        _prgRom = new byte[prgSize > 0 ? prgSize : 1];
        if (prgSize > 0) System.arraycopy(prg, 0, _prgRom, 0, prgSize);
        if (chrSize == 0) {
            _chrSize = CHR_8K_SIZE;
            _chr = new byte[CHR_8K_SIZE];
            _chrIsRam = true;
        } else {
            _chrSize = chrSize;
            _chr = new byte[chrSize];
            System.arraycopy(chr, 0, _chr, 0, chrSize);
            _chrIsRam = false;
        }
        _hasBattery = hasBattery;
        _shiftReg = 0;
        _shiftCount = 0;
        byte mirrBits;
        switch (mirroring) {
            case Mirroring.SINGLE_SCREEN0: mirrBits = 0x00; break;
            case Mirroring.SINGLE_SCREEN1: mirrBits = 0x01; break;
            case Mirroring.SINGLE_SCREEN2: mirrBits = 0x01; break;
            case Mirroring.SINGLE_SCREEN3: mirrBits = 0x01; break;
            case Mirroring.VERTICAL: mirrBits = 0x02; break;
            default: mirrBits = 0x03; break;
        }
        _control = (byte) (CONTROL_PRG_MODE3 | mirrBits);
        _chrBank0 = 0;
        _chrBank1 = 0;
        _prgBank = 0;
    }

    private int prgBankCount() { return _prgSize / PRG_BANK_SIZE > 0 ? _prgSize / PRG_BANK_SIZE : 1; }
    private int chrBankCount() { return _chrSize / CHR_4K_SIZE > 0 ? _chrSize / CHR_4K_SIZE : 1; }
    private int prgMode() { return (_control >> 2) & 0x03; }
    private int chrMode() { return (_control >> 4) & 1; }

    private int prgReadBank(int bank, int offset) {
        int count = prgBankCount();
        bank %= count;
        int idx = bank * PRG_BANK_SIZE + offset;
        if (idx >= _prgSize) return 0x00;
        return _prgRom[idx] & 0xFF;
    }

    private void serialWrite(int addr, int value) {
        if ((value & 0x80) != 0) {
            _shiftReg = 0;
            _shiftCount = 0;
            _control = (byte) ((_control & 0x13) | CONTROL_PRG_MODE3);
            return;
        }
        _shiftReg = (byte) ((_shiftReg >> 1) | ((value & 1) << 4));
        _shiftCount++;
        if (_shiftCount == 5) {
            int reg = (addr >> 13) & 0x03;
            switch (reg) {
                case 0: _control = _shiftReg; break;
                case 1: _chrBank0 = _shiftReg; break;
                case 2: _chrBank1 = _shiftReg; break;
                case 3: _prgBank = _shiftReg; break;
            }
            _shiftReg = 0;
            _shiftCount = 0;
        }
    }

    @Override
    public int readPrg(int addr) {
        if (addr >= 0x6000 && addr < 0x8000)
            return _prgRam[(addr - 0x6000) & (PRG_RAM_SIZE - 1)] & 0xFF;
        int local = addr - 0x8000;
        boolean inLow = local < PRG_BANK_SIZE;
        int offset = local & (PRG_BANK_SIZE - 1);
        switch (prgMode()) {
            case 0: case 1:
                return prgReadBank(_prgBank & 0x0E, local);
            case 2:
                if (inLow) return prgReadBank(0, offset);
                return prgReadBank(_prgBank & 0x0F, offset);
            default:
                if (inLow) return prgReadBank(_prgBank & 0x0F, offset);
                return prgReadBank(prgBankCount() - 1, offset);
        }
    }

    @Override
    public void writePrg(int addr, int value) {
        if (addr >= 0x6000 && addr < 0x8000) {
            _prgRam[(addr - 0x6000) & (PRG_RAM_SIZE - 1)] = (byte) value;
            return;
        }
        serialWrite(addr, value);
    }

    private int chrIndex(int a) {
        int count = chrBankCount();
        if (chrMode() == 0) {
            int bank = (_chrBank0 & 0x1E) % count;
            return bank * CHR_4K_SIZE + (a & (CHR_8K_SIZE - 1));
        } else if (a < CHR_4K_SIZE) {
            int bank = (_chrBank0 & 0x1F) % count;
            return bank * CHR_4K_SIZE + a;
        } else {
            int bank = (_chrBank1 & 0x1F) % count;
            return bank * CHR_4K_SIZE + (a - CHR_4K_SIZE);
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
    public int mirrorMode() {
        switch (_control & 0x03) {
            case 0: return Mirroring.SINGLE_SCREEN0;
            case 1: return Mirroring.SINGLE_SCREEN1;
            case 2: return Mirroring.VERTICAL;
            default: return Mirroring.HORIZONTAL;
        }
    }

    @Override
    public boolean chrIsRam() { return _chrIsRam; }
    @Override
    public boolean hasBattery() { return _hasBattery; }

    @Override
    public int saveState(byte[] buf) {
        int total = 6 + PRG_RAM_SIZE + (_chrIsRam ? _chrSize : 0);
        if (buf == null) return total;
        int p = 0;
        buf[p++] = _shiftReg;
        buf[p++] = _shiftCount;
        buf[p++] = _control;
        buf[p++] = _chrBank0;
        buf[p++] = _chrBank1;
        buf[p++] = _prgBank;
        System.arraycopy(_prgRam, 0, buf, p, PRG_RAM_SIZE); p += PRG_RAM_SIZE;
        if (_chrIsRam && _chrSize > 0) {
            System.arraycopy(_chr, 0, buf, p, _chrSize);
            p += _chrSize;
        }
        return p;
    }

    @Override
    public boolean loadState(byte[] buf, int len) {
        int need = 6 + PRG_RAM_SIZE + (_chrIsRam ? _chrSize : 0);
        if (len < need) return false;
        int p = 0;
        _shiftReg = buf[p++];
        _shiftCount = buf[p++];
        _control = buf[p++];
        _chrBank0 = buf[p++];
        _chrBank1 = buf[p++];
        _prgBank = buf[p++];
        System.arraycopy(buf, p, _prgRam, 0, PRG_RAM_SIZE); p += PRG_RAM_SIZE;
        if (_chrIsRam && _chrSize > 0) {
            System.arraycopy(buf, p, _chr, 0, _chrSize);
            p += _chrSize;
        }
        return true;
    }
}
