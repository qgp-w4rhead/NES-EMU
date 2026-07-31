package nes.core.mappers;

import nes.core.Mapper;
import nes.core.Mirroring;

// Mapper 3 (CNROM). Port of cores/csharp/src/Mappers/Cnrom.cs.
public final class Cnrom extends Mapper {
    private static final int CHR_BANK_SIZE = 8192;

    private final byte[] _prgRom;
    private final int _prgSize;
    private byte[] _chr;
    private int _chrSize;
    private boolean _chrIsRam;
    private final int _mirroring;
    private final boolean _hasBattery;
    private byte _chrBank;

    public Cnrom(byte[] prg, int prgSize, byte[] chr, int chrSize, int mirroring, boolean hasBattery) {
        mapperNum = 3;
        _prgSize = prgSize;
        _prgRom = new byte[prgSize > 0 ? prgSize : 1];
        if (prgSize > 0) System.arraycopy(prg, 0, _prgRom, 0, prgSize);
        if (chrSize == 0) {
            _chrSize = CHR_BANK_SIZE;
            _chr = new byte[CHR_BANK_SIZE];
            _chrIsRam = true;
        } else {
            _chrSize = chrSize;
            _chr = new byte[chrSize];
            System.arraycopy(chr, 0, _chr, 0, chrSize);
            _chrIsRam = false;
        }
        _mirroring = mirroring;
        _hasBattery = hasBattery;
        _chrBank = 0;
    }

    private int chrBankCount() { return _chrSize / CHR_BANK_SIZE > 0 ? _chrSize / CHR_BANK_SIZE : 1; }

    private int prgIndex(int addr) {
        int local = addr - 0x8000;
        int bank = _prgSize;
        if (bank == 0) return 0;
        return local % bank;
    }

    @Override
    public int readPrg(int addr) {
        if (addr < 0x8000) return 0x00;
        int idx = prgIndex(addr);
        if (idx >= _prgSize) return 0x00;
        return _prgRom[idx] & 0xFF;
    }

    @Override
    public void writePrg(int addr, int value) {
        if (addr < 0x8000) return;
        _chrBank = (byte) value;
    }

    @Override
    public int readChr(int addr) {
        int count = chrBankCount();
        int bank = _chrBank % count;
        int idx = bank * CHR_BANK_SIZE + (addr & (CHR_BANK_SIZE - 1));
        if (idx >= _chrSize) return 0x00;
        return _chr[idx] & 0xFF;
    }

    @Override
    public void writeChr(int addr, int value) {
        if (!_chrIsRam) return;
        int count = chrBankCount();
        int bank = _chrBank % count;
        int idx = bank * CHR_BANK_SIZE + (addr & (CHR_BANK_SIZE - 1));
        if (idx < _chrSize) _chr[idx] = (byte) value;
    }

    @Override
    public int mirrorMode() { return _mirroring; }
    @Override
    public boolean chrIsRam() { return _chrIsRam; }
    @Override
    public boolean hasBattery() { return _hasBattery; }

    @Override
    public int saveState(byte[] buf) {
        int total = 1 + (_chrIsRam ? _chrSize : 0);
        if (buf == null) return total;
        int p = 0;
        buf[p++] = _chrBank;
        if (_chrIsRam && _chrSize > 0) {
            System.arraycopy(_chr, 0, buf, p, _chrSize);
            p += _chrSize;
        }
        return p;
    }

    @Override
    public boolean loadState(byte[] buf, int len) {
        int need = 1 + (_chrIsRam ? _chrSize : 0);
        if (len < need) return false;
        int p = 0;
        _chrBank = buf[p++];
        if (_chrIsRam && _chrSize > 0) {
            System.arraycopy(buf, p, _chr, 0, _chrSize);
            p += _chrSize;
        }
        return true;
    }
}
