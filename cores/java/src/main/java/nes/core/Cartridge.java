package nes.core;

import nes.core.mappers.Fds;

// iNES cartridge loader + mapper dispatch (port of cores/csharp/src/Cartridge.cs).
public final class Cartridge {
    public static final int HEADER_SIZE = 16;
    public static final int TRAINER_SIZE = 512;
    public static final int PRG_ROM_UNIT = 16384;
    public static final int CHR_ROM_UNIT = 8192;

    private static final byte[] INES_MAGIC = {'N', 'E', 'S', 0x1A};
    private static final byte[] FDS_MAGIC = {'F', 'D', 'S', 0x1A};
    private static final int FDS_HEADER_SIZE = 16;
    private static final int FDS_DISK_SIDE_SIZE = 65500;

    public InesHeader header;
    public Mapper mapper;

    public Cartridge() {}

    public static int parseHeader(byte[] bytes, int len, InesHeader hdr) {
        if (len < HEADER_SIZE) return 1;
        for (int i = 0; i < 4; ++i)
            if (bytes[i] != INES_MAGIC[i]) return 2;

        int prgRomBanks = bytes[4] & 0xFF;
        int chrRomBanks = bytes[5] & 0xFF;
        int flags6 = bytes[6] & 0xFF;
        int flags7 = bytes[7] & 0xFF;

        boolean hasTrainer = (flags6 & 0x04) != 0;
        boolean hasBattery = (flags6 & 0x02) != 0;
        boolean fourScreen = (flags6 & 0x08) != 0;
        boolean vertical = (flags6 & 0x01) != 0;

        int mirroring;
        if (fourScreen) mirroring = Mirroring.FOUR_SCREEN;
        else if (vertical) mirroring = Mirroring.VERTICAL;
        else mirroring = Mirroring.HORIZONTAL;

        int mapperNumber = ((flags6 >> 4) & 0x0F) | (((flags7 >> 4) & 0x0F) << 4);
        int tvSystem = bytes[9] & 0x03;

        hdr.prgRomBanks = prgRomBanks;
        hdr.chrRomBanks = chrRomBanks;
        hdr.mapperNumber = mapperNumber;
        hdr.mirroring = mirroring;
        hdr.hasTrainer = hasTrainer;
        hdr.hasBattery = hasBattery;
        hdr.tvSystem = tvSystem;
        return 0;
    }

    public static int fromBytes(byte[] bytes, int len, Cartridge[] outCart) {
        Cartridge cart = new Cartridge();
        InesHeader hdr = new InesHeader();
        int rc = parseHeader(bytes, len, hdr);
        if (rc != 0) { outCart[0] = null; return rc; }

        int prgSize = hdr.prgRomBanks * PRG_ROM_UNIT;
        int prgOff = HEADER_SIZE;
        if (hdr.hasTrainer) prgOff += TRAINER_SIZE;
        int chrSize = (hdr.chrRomBanks > 0) ? hdr.chrRomBanks * CHR_ROM_UNIT : 0;
        if (len < prgOff + prgSize + chrSize) { outCart[0] = null; return 5; }

        byte[] prgRom = new byte[prgSize];
        System.arraycopy(bytes, prgOff, prgRom, 0, prgSize);
        byte[] chrRom = (chrSize > 0) ? new byte[chrSize] : null;
        if (chrRom != null)
            System.arraycopy(bytes, prgOff + prgSize, chrRom, 0, chrSize);

        cart.header = hdr;
        cart.mapper = MapperFactory.create(hdr.mapperNumber, prgRom, prgSize, chrRom, chrSize,
                                            hdr.mirroring, hdr.hasBattery);
        outCart[0] = cart;
        return 0;
    }

    public static int fromFdsBytes(byte[] diskData, int diskLen, byte[] bios, int biosLen, Cartridge[] outCart) {
        Cartridge cart = new Cartridge();
        if (diskLen < FDS_HEADER_SIZE) { outCart[0] = null; return 1; }
        for (int i = 0; i < 4; ++i)
            if (diskData[i] != FDS_MAGIC[i]) { outCart[0] = null; return 2; }
        int diskCount = diskData[4] & 0xFF;
        if (diskCount == 0) { outCart[0] = null; return 6; }
        long needed = (long)FDS_HEADER_SIZE + (long)diskCount * FDS_DISK_SIDE_SIZE;
        if (diskLen < needed) { outCart[0] = null; return 1; }

        int rawSize = diskCount * FDS_DISK_SIDE_SIZE;
        byte[] rawDisk = new byte[rawSize];
        System.arraycopy(diskData, FDS_HEADER_SIZE, rawDisk, 0, rawSize);

        InesHeader hdr = new InesHeader();
        hdr.mapperNumber = 20;
        hdr.mirroring = Mirroring.VERTICAL;
        hdr.prgRomBanks = 0;
        hdr.chrRomBanks = 0;
        cart.header = hdr;
        cart.mapper = new Fds(bios, biosLen, rawDisk, rawSize);
        outCart[0] = cart;
        return 0;
    }

    public int readPrg(int addr) { return mapper.readPrg(addr); }
    public int readPrgMut(int addr) { return mapper.readPrgMut(addr); }
    public void writePrg(int addr, int value) { mapper.writePrg(addr, value); }
    public int readChr(int addr) { return mapper.readChr(addr); }
    public int readChrLatched(int addr) { return mapper.readChrLatched(addr); }
    public void writeChr(int addr, int value) { mapper.writeChr(addr, value); }
    public int mirrorMode() { return mapper.mirrorMode(); }
    public boolean chrIsRam() { return mapper.chrIsRam(); }
    public boolean hasBattery() { return mapper.hasBattery(); }
    public boolean irqPending() { return mapper.irqPending(); }
    public void clockIrq() { mapper.clockIrq(); }
    public void resetScanlineCounter() { mapper.resetScanlineCounter(); }
    public void clockCpu(int cpuCycles) { mapper.clockCpu(cpuCycles); }
    public float expansionAudioSample() { return mapper.expansionAudioSample(); }
    public int saveState(byte[] buf) { return mapper.saveState(buf); }
    public boolean loadState(byte[] buf, int len) { return mapper.loadState(buf, len); }
}
