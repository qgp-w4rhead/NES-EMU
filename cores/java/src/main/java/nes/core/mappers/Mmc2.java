package nes.core.mappers;

import nes.core.Mapper;
import nes.core.Mirroring;

// Mapper 9 (MMC2). Port of cores/csharp/src/Mappers/Mmc2.cs.
public final class Mmc2 extends Mapper {
    private static final int PRG_BANK_SIZE = 8192;
    private static final int CHR_4K_SIZE = 4096;
    private static final int PRG_RAM_SIZE = 1024;
    private static final int LATCH_LEFT_B0_LO = 0x0FD8, LATCH_LEFT_B0_HI = 0x0FDF;
    private static final int LATCH_LEFT_B1_LO = 0x0FE8, LATCH_LEFT_B1_HI = 0x0FEF;
    private static final int LATCH_RIGHT_B0_LO = 0x1FD8, LATCH_RIGHT_B0_HI = 0x1FDF;
    private static final int LATCH_RIGHT_B1_LO = 0x1FE8, LATCH_RIGHT_B1_HI = 0x1FEF;

    private final byte[] _prgRom;
    private final int _prgSize;
    private byte[] _chr;
    private int _chrSize;
    private boolean _chrIsRam;
    private final byte[] _prgRam = new byte[PRG_RAM_SIZE];
    private final boolean _hasBattery;
    private byte _prgBank;
    private final byte[] _chrBanks = new byte[4];
    private byte _latchLeft;
    private byte _latchRight;
    private final int _mirroring;

    public Mmc2(byte[] prg, int prgSize, byte[] chr, int chrSize, int mirroring, boolean hasBattery) {
        mapperNum = 9;
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
        _prgBank = 0;
        _mirroring = mirroring;
    }

    private int prgBankCount() { return _prgSize / PRG_BANK_SIZE > 0 ? _prgSize / PRG_BANK_SIZE : 1; }
    private int chrBankCount() { return _chrSize / CHR_4K_SIZE > 0 ? _chrSize / CHR_4K_SIZE : 1; }

    private int prgReadBank(int bank, int offset) {
        int count = prgBankCount();
        bank %= count;
        int idx = bank * PRG_BANK_SIZE + offset;
        if (idx >= _prgSize) return 0x00;
        return _prgRom[idx] & 0xFF;
    }

    private byte leftBank() { return _latchLeft == 0 ? _chrBanks[0] : _chrBanks[1]; }
    private byte rightBank() { return _latchRight == 0 ? _chrBanks[2] : _chrBanks[3]; }

    private void updateLatches(int addr) {
        if (addr >= LATCH_LEFT_B0_LO && addr <= LATCH_LEFT_B0_HI) _latchLeft = 0;
        else if (addr >= LATCH_LEFT_B1_LO && addr <= LATCH_LEFT_B1_HI) _latchLeft = 1;
        else if (addr >= LATCH_RIGHT_B0_LO && addr <= LATCH_RIGHT_B0_HI) _latchRight = 0;
        else if (addr >= LATCH_RIGHT_B1_LO && addr <= LATCH_RIGHT_B1_HI) _latchRight = 1;
    }

    @Override
    public int readPrg(int addr) {
        if (addr >= 0x6000 && addr < 0x8000)
            return _prgRam[(addr - 0x6000) & (PRG_RAM_SIZE - 1)] & 0xFF;
        int local = addr - 0x8000;
        int count = prgBankCount();
        if (local < PRG_BANK_SIZE) {
            int bank = (_prgBank & 0xFF) % count;
            return prgReadBank(bank, local);
        }
        int fixedOffset = local - PRG_BANK_SIZE;
        int fixedBankBase = count >= 3 ? count - 3 : 0;
        int bank2 = fixedBankBase + (fixedOffset / PRG_BANK_SIZE);
        int offset = fixedOffset & (PRG_BANK_SIZE - 1);
        return prgReadBank(bank2, offset);
    }

    @Override
    public void writePrg(int addr, int value) {
        int v = value & 0xFF;
        if (addr >= 0x6000 && addr < 0x8000) {
            _prgRam[(addr - 0x6000) & (PRG_RAM_SIZE - 1)] = (byte) v;
            return;
        }
        switch (addr) {
            case 0xA000: _prgBank = (byte) (v & 0x0F); break;
            case 0xB000: _chrBanks[0] = (byte) (v & 0x3F); break;
            case 0xB001: _chrBanks[1] = (byte) (v & 0x3F); break;
            case 0xB002: _chrBanks[2] = (byte) (v & 0x3F); break;
            case 0xB003: _chrBanks[3] = (byte) (v & 0x3F); break;
        }
    }

    @Override
    public int readChr(int addr) {
        byte bank = (addr < CHR_4K_SIZE) ? leftBank() : rightBank();
        int count = chrBankCount();
        int b = (bank & 0xFF) % count;
        int offset = addr & (CHR_4K_SIZE - 1);
        int idx = b * CHR_4K_SIZE + offset;
        if (idx >= _chrSize) return 0x00;
        return _chr[idx] & 0xFF;
    }

    @Override
    public int readChrLatched(int addr) {
        updateLatches(addr);
        return readChr(addr);
    }

    @Override
    public void writeChr(int addr, int value) {
        if (!_chrIsRam) return;
        byte bank = (addr < CHR_4K_SIZE) ? leftBank() : rightBank();
        int count = chrBankCount();
        int b = (bank & 0xFF) % count;
        int offset = addr & (CHR_4K_SIZE - 1);
        int idx = b * CHR_4K_SIZE + offset;
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
        int total = 6 + PRG_RAM_SIZE + (_chrIsRam ? _chrSize : 0);
        if (buf == null) return total;
        int p = 0;
        buf[p++] = _prgBank;
        for (int i = 0; i < 4; ++i) buf[p++] = _chrBanks[i];
        buf[p++] = _latchLeft;
        buf[p++] = _latchRight;
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
        _prgBank = buf[p++];
        for (int i = 0; i < 4; ++i) _chrBanks[i] = buf[p++];
        _latchLeft = buf[p++];
        _latchRight = buf[p++];
        System.arraycopy(buf, p, _prgRam, 0, PRG_RAM_SIZE); p += PRG_RAM_SIZE;
        if (_chrIsRam && _chrSize > 0) {
            System.arraycopy(buf, p, _chr, 0, _chrSize);
            p += _chrSize;
        }
        return true;
    }
}
