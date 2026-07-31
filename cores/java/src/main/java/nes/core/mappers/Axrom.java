package nes.core.mappers;

import nes.core.Mapper;
import nes.core.Mirroring;

// Mapper 7 (AxROM). Port of cores/csharp/src/Mappers/Axrom.cs.
public final class Axrom extends Mapper {
    private static final int PRG_BANK_SIZE = 32768;
    private static final int CHR_SIZE = 8192;

    private final byte[] _prgRom;
    private final int _prgSize;
    private byte[] _chr;
    private int _chrSize;
    private boolean _chrIsRam;
    private final boolean _hasBattery;
    private byte _prgBank;
    private byte _mirrorNt;

    public Axrom(byte[] prg, int prgSize, byte[] chr, int chrSize, int mirroring, boolean hasBattery) {
        mapperNum = 7;
        _prgSize = prgSize;
        _prgRom = new byte[prgSize > 0 ? prgSize : 1];
        if (prgSize > 0) System.arraycopy(prg, 0, _prgRom, 0, prgSize);
        if (chrSize > 0) {
            _chrSize = chrSize;
            _chr = new byte[chrSize];
            System.arraycopy(chr, 0, _chr, 0, chrSize);
            _chrIsRam = false;
        } else {
            _chrSize = CHR_SIZE;
            _chr = new byte[CHR_SIZE];
            _chrIsRam = true;
        }
        _hasBattery = hasBattery;
        _prgBank = 0;
        _mirrorNt = 0;
    }

    private int prgBankCount() { return _prgSize / PRG_BANK_SIZE > 0 ? _prgSize / PRG_BANK_SIZE : 1; }

    @Override
    public int readPrg(int addr) {
        if (addr < 0x8000) return 0x00;
        int local = addr - 0x8000;
        int count = prgBankCount();
        int bank = _prgBank % count;
        int idx = bank * PRG_BANK_SIZE + local;
        if (idx >= _prgSize) return 0x00;
        return _prgRom[idx] & 0xFF;
    }

    @Override
    public void writePrg(int addr, int value) {
        if (addr < 0x8000) return;
        _mirrorNt = (byte) ((value >> 4) & 1);
        _prgBank = (byte) (value & 0x07);
    }

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
    public int mirrorMode() { return _mirrorNt == 0 ? Mirroring.SINGLE_SCREEN0 : Mirroring.SINGLE_SCREEN1; }
    @Override
    public boolean chrIsRam() { return _chrIsRam; }
    @Override
    public boolean hasBattery() { return _hasBattery; }

    @Override
    public int saveState(byte[] buf) {
        int total = 2 + (_chrIsRam ? _chrSize : 0);
        if (buf == null) return total;
        int p = 0;
        buf[p++] = _prgBank;
        buf[p++] = _mirrorNt;
        if (_chrIsRam && _chrSize > 0) {
            System.arraycopy(_chr, 0, buf, p, _chrSize);
            p += _chrSize;
        }
        return p;
    }

    @Override
    public boolean loadState(byte[] buf, int len) {
        int need = 2 + (_chrIsRam ? _chrSize : 0);
        if (len < need) return false;
        int p = 0;
        _prgBank = buf[p++];
        _mirrorNt = buf[p++];
        if (_chrIsRam && _chrSize > 0) {
            System.arraycopy(buf, p, _chr, 0, _chrSize);
            p += _chrSize;
        }
        return true;
    }
}
