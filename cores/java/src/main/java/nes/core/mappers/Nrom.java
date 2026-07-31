package nes.core.mappers;

import nes.core.Mapper;
import nes.core.Mirroring;

// Mapper 0 (NROM). Port of cores/csharp/src/Mappers/Nrom.cs.
public final class Nrom extends Mapper {
    private static final int PRG_BANK_SIZE = 16384;
    private static final int CHR_RAM_SIZE = 8192;

    private final byte[] _prgRom;
    private final int _prgSize;
    private byte[] _chr;
    private int _chrSize;
    private boolean _chrIsRam;
    private int _mirroring;
    private boolean _hasBattery;

    public Nrom(byte[] prg, int prgSize, byte[] chr, int chrSize, int mirroring, boolean hasBattery) {
        mapperNum = 0;
        _prgSize = prgSize;
        _prgRom = new byte[prgSize > 0 ? prgSize : 1];
        if (prgSize > 0) System.arraycopy(prg, 0, _prgRom, 0, prgSize);
        if (chrSize > 0) {
            _chrSize = chrSize;
            _chr = new byte[chrSize];
            System.arraycopy(chr, 0, _chr, 0, chrSize);
            _chrIsRam = false;
        } else {
            _chrSize = CHR_RAM_SIZE;
            _chr = new byte[CHR_RAM_SIZE];
            _chrIsRam = true;
        }
        _mirroring = mirroring;
        _hasBattery = hasBattery;
    }

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
    public void writePrg(int addr, int value) {}

    @Override
    public int readChr(int addr) {
        int sz = _chrSize;
        if (sz == 0) return 0x00;
        return _chr[addr % sz] & 0xFF;
    }

    @Override
    public void writeChr(int addr, int value) {
        if (!_chrIsRam) return;
        int sz = _chrSize;
        if (sz == 0) return;
        _chr[addr % sz] = (byte) value;
    }

    @Override
    public int mirrorMode() { return _mirroring; }
    @Override
    public boolean chrIsRam() { return _chrIsRam; }
    @Override
    public boolean hasBattery() { return _hasBattery; }

    @Override
    public int saveState(byte[] buf) {
        int total = 7 + (_chrIsRam ? _chrSize : 0);
        if (buf == null) return total;
        int p = 0;
        buf[p++] = (byte) (_chrIsRam ? 1 : 0);
        buf[p++] = (byte) _mirroring;
        buf[p++] = (byte) (_hasBattery ? 1 : 0);
        buf[p++] = (byte) _chrSize;
        buf[p++] = (byte) (_chrSize >> 8);
        buf[p++] = (byte) (_chrSize >> 16);
        buf[p++] = (byte) (_chrSize >> 24);
        if (_chrIsRam && _chrSize > 0) {
            System.arraycopy(_chr, 0, buf, p, _chrSize);
            p += _chrSize;
        }
        return p;
    }

    @Override
    public boolean loadState(byte[] buf, int len) {
        if (len < 7) return false;
        int p = 0;
        _chrIsRam = buf[p++] != 0;
        _mirroring = buf[p++] & 0xFF;
        _hasBattery = buf[p++] != 0;
        _chrSize = (buf[p] & 0xFF) | ((buf[p+1] & 0xFF) << 8) | ((buf[p+2] & 0xFF) << 16) | ((buf[p+3] & 0xFF) << 24);
        p += 4;
        if (_chrIsRam && _chrSize > 0) {
            if (len < p + _chrSize) return false;
            System.arraycopy(buf, p, _chr, 0, _chrSize);
            p += _chrSize;
        }
        return true;
    }
}
