package nes.core;

// Addressing-mode resolvers and read/write helpers (port of cores/csharp/src/CpuAddressing.cs).
public final class CpuAddressing {
    private CpuAddressing() {}

    // Helper to be invoked as instance methods on Cpu. We instead place them
    // directly on Cpu via this class as static helpers taking the Cpu instance.
    public static Operand resolve(Cpu c, int mode) {
        switch (mode) {
            case AddrMode.IMPLIED: return Operand.makeNone();
            case AddrMode.ACCUMULATOR: return Operand.makeAcc();
            case AddrMode.IMMEDIATE: return Operand.makeAddr(c.amImmediate());
            case AddrMode.ZERO_PAGE: return Operand.makeAddr(c.amZeroPage());
            case AddrMode.ZERO_PAGE_X: return Operand.makeAddr(c.amZeroPageX(Dummy.NONE));
            case AddrMode.ZERO_PAGE_Y: return Operand.makeAddr(c.amZeroPageY(Dummy.NONE));
            case AddrMode.ABSOLUTE: return Operand.makeAddr(c.amAbsolute());
            case AddrMode.ABSOLUTE_X: return Operand.makeAddr(c.amAbsoluteX(Dummy.NONE));
            case AddrMode.ABSOLUTE_Y: return Operand.makeAddr(c.amAbsoluteY(Dummy.NONE));
            case AddrMode.INDIRECT: return Operand.makeAddr(c.amIndirect());
            case AddrMode.INDIRECT_X: return Operand.makeAddr(c.amIndirectX());
            case AddrMode.INDIRECT_Y: return Operand.makeAddr(c.amIndirectY(Dummy.NONE));
            case AddrMode.RELATIVE: return Operand.makeAddr(c.amRelative());
            default: return Operand.makeNone();
        }
    }
}
