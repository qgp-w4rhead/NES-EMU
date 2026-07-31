package nes.core.mappers;

import nes.core.Mapper;
import nes.core.Mirroring;

// Mapper 5 (MMC5). Port of cores/csharp/src/Mappers/Mmc5.cs.
public final class Mmc5 extends Mapper {
    private static final int PRG_BANK_SIZE = 8192;
    private static final int CHR_1K_SIZE = 1024;
    private static final int PRG_RAM_SIZE = 65536;
    private static final int PRG_RAM_WINDOW = 8192;

    private final byte[] _prgRom;
    private final int _prgSize;
    private byte[] _chr;
    private int _chrSize;
    private boolean _chrIsRam;
    private final byte[] _prgRam = new byte[PRG_RAM_SIZE];
    private final boolean _hasBattery;
    private byte _prgMode = 3;
    private byte _chrMode = 3;
    private byte _prgRamProtect1;
    private byte _prgRamProtect2;
    private byte _ntMirroring;
    private byte _fillTile;
    private byte _fillAttr;
    private byte _prgRamBank;
    private final byte[] _prgBanks = new byte[4];
    private final byte[] _chrBanks = new byte[8];
    private final byte[] _chrBanksEx = new byte[4];
    private byte _chrUpper;
    private byte _splitControl;
    private byte _splitYScroll;
    private byte _splitBank;
    private byte _irqScanline;
    private byte _irqControl;
    private byte _scanlineCounter;
    private boolean _irqPending;
    private boolean _inVblank;
    private byte _multA;
    private byte _multB;

    public Mmc5(byte[] prg, int prgSize, byte[] chr, int chrSize, int mirroring, boolean hasBattery) {
        mapperNum = 5;
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
        switch (mirroring) {
            case Mirroring.HORIZONTAL: _ntMirroring = 0x44; break;
            case Mirroring.VERTICAL: _ntMirroring = 0x50; break;
            case Mirroring.FOUR_SCREEN: _ntMirroring = (byte) 0xE4; break;
            case Mirroring.SINGLE_SCREEN0: _ntMirroring = 0x00; break;
            case Mirroring.SINGLE_SCREEN1: _ntMirroring = 0x55; break;
            case Mirroring.SINGLE_SCREEN2: _ntMirroring = (byte) 0xAA; break;
            case Mirroring.SINGLE_SCREEN3: _ntMirroring = (byte) 0xFF; break;
            default: _ntMirroring = 0x44; break;
        }
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

    private int product() { return (_multA & 0xFF) * (_multB & 0xFF); }

    private boolean prgRamWritesAllowed() { return _prgRamProtect1 == 0x02 && _prgRamProtect2 == 0x01; }

    private int decodeMirroring() {
        int s0 = _ntMirroring & 0x03;
        int s1 = (_ntMirroring >> 2) & 0x03;
        int s2 = (_ntMirroring >> 4) & 0x03;
        int s3 = (_ntMirroring >> 6) & 0x03;
        if (s0 == s1 && s1 == s2 && s2 == s3) {
            switch (s0) {
                case 0: return Mirroring.SINGLE_SCREEN0;
                case 1: return Mirroring.SINGLE_SCREEN1;
                case 2: return Mirroring.SINGLE_SCREEN2;
                default: return Mirroring.SINGLE_SCREEN3;
            }
        }
        if (s0 == 0 && s1 == 1 && s2 == 0 && s3 == 1) return Mirroring.HORIZONTAL;
        if (s0 == 0 && s1 == 0 && s2 == 1 && s3 == 1) return Mirroring.VERTICAL;
        if (s0 == 0 && s1 == 1 && s2 == 2 && s3 == 3) return Mirroring.FOUR_SCREEN;
        return Mirroring.HORIZONTAL;
    }

    @Override
    public int readPrg(int addr) {
        if (addr == 0x5205) return product() & 0xFF;
        if (addr == 0x5206) return (product() >> 8) & 0xFF;
        if (addr == 0x5204) {
            int status = _irqControl & 0x80;
            if (_inVblank) status |= 0x40;
            return status;
        }
        if (addr >= 0x6000 && addr < 0x8000) {
            int bank = (_prgRamBank & 0xFF) % (PRG_RAM_SIZE / PRG_RAM_WINDOW);
            int idx = bank * PRG_RAM_WINDOW + ((addr - 0x6000) & (PRG_RAM_WINDOW - 1));
            if (idx >= PRG_RAM_SIZE) return 0x00;
            return _prgRam[idx] & 0xFF;
        }
        int local = addr - 0x8000;
        int slot = local / PRG_BANK_SIZE;
        int offset = local & (PRG_BANK_SIZE - 1);
        if (slot == 3) {
            int last = prgBankCount() - 1;
            return prgReadBank(last, offset);
        }
        int reg = _prgBanks[slot] & 0xFF;
        if ((reg & 0x80) != 0) {
            int ramBank = (reg & 0x7F) % (PRG_RAM_SIZE / PRG_BANK_SIZE);
            int idx = ramBank * PRG_BANK_SIZE + offset;
            if (idx >= PRG_RAM_SIZE) return 0x00;
            return _prgRam[idx] & 0xFF;
        }
        int count = prgBankCount();
        int bank2 = reg % count;
        return prgReadBank(bank2, offset);
    }

    @Override
    public void writePrg(int addr, int value) {
        int v = value & 0xFF;
        if (addr == 0x5205) { _multA = (byte) v; return; }
        if (addr == 0x5206) { _multB = (byte) v; return; }
        if (addr == 0x5204) {
            _irqControl = (byte) (v & 0x80);
            if ((v & 0x80) == 0) _irqPending = false;
            return;
        }
        if (addr == 0x5203) { _irqScanline = (byte) v; return; }
        if (addr == 0x5200) { _splitControl = (byte) v; return; }
        if (addr == 0x5201) { _splitYScroll = (byte) v; return; }
        if (addr == 0x5202) { _splitBank = (byte) v; return; }
        switch (addr) {
            case 0x5100: _prgMode = (byte) (v & 0x03); return;
            case 0x5101: _chrMode = (byte) (v & 0x03); return;
            case 0x5102: _prgRamProtect1 = (byte) (v & 0x03); return;
            case 0x5103: _prgRamProtect2 = (byte) (v & 0x03); return;
            case 0x5104: return;
            case 0x5105: _ntMirroring = (byte) v; return;
            case 0x5106: _fillTile = (byte) v; return;
            case 0x5107: _fillAttr = (byte) v; return;
            case 0x5113: _prgRamBank = (byte) v; return;
            case 0x5114: _prgBanks[0] = (byte) v; return;
            case 0x5115: _prgBanks[1] = (byte) v; return;
            case 0x5116: _prgBanks[2] = (byte) v; return;
            case 0x5117: _prgBanks[3] = (byte) v; return;
            case 0x5120: _chrBanks[0] = (byte) v; return;
            case 0x5121: _chrBanks[1] = (byte) v; return;
            case 0x5122: _chrBanks[2] = (byte) v; return;
            case 0x5123: _chrBanks[3] = (byte) v; return;
            case 0x5124: _chrBanks[4] = (byte) v; return;
            case 0x5125: _chrBanks[5] = (byte) v; return;
            case 0x5126: _chrBanks[6] = (byte) v; return;
            case 0x5127: _chrBanks[7] = (byte) v; return;
            case 0x5128: _chrBanksEx[0] = (byte) v; return;
            case 0x5129: _chrBanksEx[1] = (byte) v; return;
            case 0x512A: _chrBanksEx[2] = (byte) v; return;
            case 0x512B: _chrBanksEx[3] = (byte) v; return;
            case 0x5130: _chrUpper = (byte) (v & 0x01); return;
        }
        if (addr >= 0x6000 && addr < 0x8000 && prgRamWritesAllowed()) {
            int bank = (_prgRamBank & 0xFF) % (PRG_RAM_SIZE / PRG_RAM_WINDOW);
            int idx = bank * PRG_RAM_WINDOW + ((addr - 0x6000) & (PRG_RAM_WINDOW - 1));
            if (idx < PRG_RAM_SIZE) _prgRam[idx] = (byte) v;
            return;
        }
        if (addr >= 0x8000 && addr < 0xE000 && prgRamWritesAllowed()) {
            int local = addr - 0x8000;
            int slot = local / PRG_BANK_SIZE;
            int offset = local & (PRG_BANK_SIZE - 1);
            if (slot < 3) {
                int reg = _prgBanks[slot] & 0xFF;
                if ((reg & 0x80) != 0) {
                    int ramBank = (reg & 0x7F) % (PRG_RAM_SIZE / PRG_BANK_SIZE);
                    int idx = ramBank * PRG_BANK_SIZE + offset;
                    if (idx < PRG_RAM_SIZE) _prgRam[idx] = (byte) v;
                }
            }
        }
    }

    @Override
    public int readChr(int addr) {
        int slot = addr / CHR_1K_SIZE;
        int offset = addr & (CHR_1K_SIZE - 1);
        int count = chr1kCount();
        int bankReg = _chrBanks[slot] & 0xFF;
        int bank = (((_chrUpper & 0xFF) << 8) | bankReg) % count;
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
        int bankReg = _chrBanks[slot] & 0xFF;
        int bank = (((_chrUpper & 0xFF) << 8) | bankReg) % count;
        int idx = bank * CHR_1K_SIZE + offset;
        if (idx < _chrSize) _chr[idx] = (byte) value;
    }

    @Override
    public int mirrorMode() { return decodeMirroring(); }
    @Override
    public boolean chrIsRam() { return _chrIsRam; }
    @Override
    public boolean hasBattery() { return _hasBattery; }
    @Override
    public boolean irqPending() { return _irqPending; }

    @Override
    public void clockIrq() {
        _scanlineCounter++;
        if (_scanlineCounter >= 240) _inVblank = true;
        if (_scanlineCounter == (_irqScanline & 0xFF) && (_irqControl & 0x80) != 0) _irqPending = true;
    }

    @Override
    public void resetScanlineCounter() {
        _scanlineCounter = 0;
        _inVblank = false;
    }

    @Override
    public int saveState(byte[] buf) {
        int scalarSize = 19;
        int total = scalarSize + 4 + 8 + 4 + PRG_RAM_SIZE + (_chrIsRam ? _chrSize : 0);
        if (buf == null) return total;
        int p = 0;
        buf[p++] = _prgMode;
        buf[p++] = _chrMode;
        buf[p++] = _prgRamProtect1;
        buf[p++] = _prgRamProtect2;
        buf[p++] = _ntMirroring;
        buf[p++] = _fillTile;
        buf[p++] = _fillAttr;
        buf[p++] = _prgRamBank;
        buf[p++] = _chrUpper;
        buf[p++] = _splitControl;
        buf[p++] = _splitYScroll;
        buf[p++] = _splitBank;
        buf[p++] = _irqScanline;
        buf[p++] = _irqControl;
        buf[p++] = _scanlineCounter;
        buf[p++] = (byte) (_inVblank ? 1 : 0);
        buf[p++] = (byte) (_irqPending ? 1 : 0);
        buf[p++] = _multA;
        buf[p++] = _multB;
        for (int i = 0; i < 4; ++i) buf[p++] = _prgBanks[i];
        for (int i = 0; i < 8; ++i) buf[p++] = _chrBanks[i];
        for (int i = 0; i < 4; ++i) buf[p++] = _chrBanksEx[i];
        System.arraycopy(_prgRam, 0, buf, p, PRG_RAM_SIZE); p += PRG_RAM_SIZE;
        if (_chrIsRam && _chrSize > 0) {
            System.arraycopy(_chr, 0, buf, p, _chrSize);
            p += _chrSize;
        }
        return p;
    }

    @Override
    public boolean loadState(byte[] buf, int len) {
        int need = 19 + 4 + 8 + 4 + PRG_RAM_SIZE + (_chrIsRam ? _chrSize : 0);
        if (len < need) return false;
        int p = 0;
        _prgMode = buf[p++];
        _chrMode = buf[p++];
        _prgRamProtect1 = buf[p++];
        _prgRamProtect2 = buf[p++];
        _ntMirroring = buf[p++];
        _fillTile = buf[p++];
        _fillAttr = buf[p++];
        _prgRamBank = buf[p++];
        _chrUpper = buf[p++];
        _splitControl = buf[p++];
        _splitYScroll = buf[p++];
        _splitBank = buf[p++];
        _irqScanline = buf[p++];
        _irqControl = buf[p++];
        _scanlineCounter = buf[p++];
        _inVblank = buf[p++] != 0;
        _irqPending = buf[p++] != 0;
        _multA = buf[p++];
        _multB = buf[p++];
        for (int i = 0; i < 4; ++i) _prgBanks[i] = buf[p++];
        for (int i = 0; i < 8; ++i) _chrBanks[i] = buf[p++];
        for (int i = 0; i < 4; ++i) _chrBanksEx[i] = buf[p++];
        System.arraycopy(buf, p, _prgRam, 0, PRG_RAM_SIZE); p += PRG_RAM_SIZE;
        if (_chrIsRam && _chrSize > 0) {
            System.arraycopy(buf, p, _chr, 0, _chrSize);
            p += _chrSize;
        }
        return true;
    }
}
