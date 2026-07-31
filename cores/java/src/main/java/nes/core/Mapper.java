package nes.core;

// Mapper base class. Subclasses (final) implement the virtual methods.
// PRG addresses are $6000..=FFFF; CHR are $0000..=1FFF.
// Port of cores/csharp/src/Mapper.cs.
public abstract class Mapper {
    public int mapperNum;

    public abstract int readPrg(int addr);
    public int readPrgMut(int addr) { return readPrg(addr); }
    public abstract void writePrg(int addr, int value);
    public abstract int readChr(int addr);
    public int readChrLatched(int addr) { return readChr(addr); }
    public abstract void writeChr(int addr, int value);
    public abstract int mirrorMode();
    public boolean chrIsRam() { return false; }
    public boolean hasBattery() { return false; }
    public boolean irqPending() { return false; }
    public void clockIrq() {}
    public void resetScanlineCounter() {}
    public void clockCpu(int cpuCycles) {}
    public float expansionAudioSample() { return 0.0f; }

    // Save state: if buf is null, return required size; else write and return bytes written.
    public int saveState(byte[] buf) { return 0; }
    public boolean loadState(byte[] buf, int len) { return true; }
}
