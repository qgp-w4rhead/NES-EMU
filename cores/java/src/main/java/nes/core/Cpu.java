package nes.core;

// 6502 CPU core (port of cores/csharp/src/Cpu.cs).
// Addressing modes (AddrMode).
final class AddrMode {
    static final int IMPLIED = 0;
    static final int ACCUMULATOR = 1;
    static final int IMMEDIATE = 2;
    static final int ZERO_PAGE = 3;
    static final int ZERO_PAGE_X = 4;
    static final int ZERO_PAGE_Y = 5;
    static final int ABSOLUTE = 6;
    static final int ABSOLUTE_X = 7;
    static final int ABSOLUTE_Y = 8;
    static final int INDIRECT = 9;
    static final int INDIRECT_X = 10;
    static final int INDIRECT_Y = 11;
    static final int RELATIVE = 12;
    private AddrMode() {}
}

// Dummy-read behaviour.
final class Dummy {
    static final int NONE = 0;
    static final int READ = 1;
    static final int RMW = 2;
    private Dummy() {}
}

// Operand tag.
final class Operand {
    static final int NONE = 0;
    static final int ACC = 1;
    static final int ADDR = 2;

    int tag;
    int address;

    static Operand makeNone() { Operand o = new Operand(); o.tag = NONE; return o; }
    static Operand makeAcc() { Operand o = new Operand(); o.tag = ACC; return o; }
    static Operand makeAddr(int a) { Operand o = new Operand(); o.tag = ADDR; o.address = a; return o; }
}

// Value transform functional interface (for RMW ops).
interface ValueFn {
    int apply(int v);
}

// 6502 CPU core.
public final class Cpu {
    public static final int FLAG_C = 0x01;
    public static final int FLAG_Z = 0x02;
    public static final int FLAG_I = 0x04;
    public static final int FLAG_D = 0x08;
    public static final int FLAG_B = 0x10;
    public static final int FLAG_U = 0x20;
    public static final int FLAG_V = 0x40;
    public static final int FLAG_N = 0x80;

    public static final int VECTOR_NMI = 0xFFFA;
    public static final int VECTOR_RESET = 0xFFFC;
    public static final int VECTOR_IRQ = 0xFFFE;

    public static final int NMI_PENDING = 0x01;
    public static final int IRQ_PENDING = 0x02;
    public static final int HALTED = 0x04;

    public int a;
    public int x;
    public int y;
    public int sp;
    public int pc;
    public int status;
    public int flags;

    public Bus bus;

    public Cpu() {}

    public void init() {
        a = 0; x = 0; y = 0;
        sp = 0xFD;
        pc = 0;
        status = FLAG_U | FLAG_I;
        flags = 0;
    }

    public boolean carry() { return (status & FLAG_C) != 0; }
    public boolean zero() { return (status & FLAG_Z) != 0; }
    public boolean interruptDisable() { return (status & FLAG_I) != 0; }
    public boolean decimal() { return (status & FLAG_D) != 0; }
    public boolean overflow() { return (status & FLAG_V) != 0; }
    public boolean negative() { return (status & FLAG_N) != 0; }

    public void setFlag(int flag, boolean v) {
        if (v) status |= flag;
        else status &= ~flag;
    }

    public void setCarry(boolean v) { setFlag(FLAG_C, v); }
    public void setZero(boolean v) { setFlag(FLAG_Z, v); }
    public void setInterruptDisable(boolean v) { setFlag(FLAG_I, v); }
    public void setDecimal(boolean v) { setFlag(FLAG_D, v); }
    public void setOverflow(boolean v) { setFlag(FLAG_V, v); }
    public void setNegative(boolean v) { setFlag(FLAG_N, v); }

    public void setNz(int value) {
        setZero((value & 0xFF) == 0);
        setNegative((value & 0x80) != 0);
    }

    public int fetchByte() {
        int b = bus.read(pc);
        pc = (pc + 1) & 0xFFFF;
        return b & 0xFF;
    }

    public int fetchWord() {
        int lo = fetchByte();
        int hi = fetchByte();
        return (lo | (hi << 8)) & 0xFFFF;
    }

    public void push(int value) {
        bus.write((0x0100 | sp) & 0xFFFF, value & 0xFF);
        sp = (sp - 1) & 0xFF;
    }

    public int pull() {
        sp = (sp + 1) & 0xFF;
        return bus.read((0x0100 | sp) & 0xFFFF) & 0xFF;
    }

    public void pushPc(int pcVal) {
        push((pcVal >> 8) & 0xFF);
        push(pcVal & 0xFF);
    }

    public int pullPc() {
        int lo = pull();
        int hi = pull();
        return (lo | (hi << 8)) & 0xFFFF;
    }

    public void pushStatus(boolean withBreak) {
        int p = status | FLAG_U;
        if (withBreak) p |= FLAG_B;
        else p &= ~FLAG_B;
        push(p & 0xFF);
    }

    public void pullStatus() {
        int p = pull();
        status = (p & ~FLAG_B) | FLAG_U;
    }

    public int readOperand(Operand op) {
        switch (op.tag) {
            case Operand.ACC: return a & 0xFF;
            case Operand.ADDR: return bus.read(op.address) & 0xFF;
            default: return 0;
        }
    }

    public void writeOperand(Operand op, int value) {
        switch (op.tag) {
            case Operand.ACC: a = value & 0xFF; return;
            case Operand.ADDR: bus.write(op.address, value & 0xFF); return;
        }
    }

    private void serviceInterrupt(int vector) {
        pushPc(pc);
        pushStatus(false);
        setInterruptDisable(true);
        pc = readVector(vector);
    }

    public void nmi() { serviceInterrupt(VECTOR_NMI); }
    public void irq() { serviceInterrupt(VECTOR_IRQ); }

    public void reset() {
        sp = 0xFD;
        setInterruptDisable(true);
        status |= FLAG_U;
        pc = readVector(VECTOR_RESET);
        flags &= ~HALTED;
    }

    public int step() {
        if ((flags & HALTED) != 0) return 1;
        if ((flags & NMI_PENDING) != 0) {
            flags &= ~NMI_PENDING;
            nmi();
            return 7;
        }
        if ((flags & IRQ_PENDING) != 0 && !interruptDisable()) {
            flags &= ~IRQ_PENDING;
            irq();
            return 7;
        }
        int opcode = fetchByte();
        return execute(opcode);
    }

    public boolean isHalted() { return (flags & HALTED) != 0; }
    public void setHalted(boolean v) {
        if (v) flags |= HALTED;
        else flags &= ~HALTED;
    }
    public boolean nmiPendingFlag() { return (flags & NMI_PENDING) != 0; }
    public boolean irqPendingFlag() { return (flags & IRQ_PENDING) != 0; }
    public void setNmiPending(boolean v) {
        if (v) flags |= NMI_PENDING;
        else flags &= ~NMI_PENDING;
    }
    public void setIrqPending(boolean v) {
        if (v) flags |= IRQ_PENDING;
        else flags &= ~IRQ_PENDING;
    }

    public int readVector(int addr) {
        int lo = bus.read(addr) & 0xFF;
        int hi = bus.read((addr + 1) & 0xFFFF) & 0xFF;
        return (lo | (hi << 8)) & 0xFFFF;
    }

    // ---- Addressing-mode resolvers (port of CpuAddressing.cs) ----

    public Operand resolve(int mode) {
        switch (mode) {
            case AddrMode.IMPLIED: return Operand.makeNone();
            case AddrMode.ACCUMULATOR: return Operand.makeAcc();
            case AddrMode.IMMEDIATE: return Operand.makeAddr(amImmediate());
            case AddrMode.ZERO_PAGE: return Operand.makeAddr(amZeroPage());
            case AddrMode.ZERO_PAGE_X: return Operand.makeAddr(amZeroPageX(Dummy.NONE));
            case AddrMode.ZERO_PAGE_Y: return Operand.makeAddr(amZeroPageY(Dummy.NONE));
            case AddrMode.ABSOLUTE: return Operand.makeAddr(amAbsolute());
            case AddrMode.ABSOLUTE_X: return Operand.makeAddr(amAbsoluteX(Dummy.NONE));
            case AddrMode.ABSOLUTE_Y: return Operand.makeAddr(amAbsoluteY(Dummy.NONE));
            case AddrMode.INDIRECT: return Operand.makeAddr(amIndirect());
            case AddrMode.INDIRECT_X: return Operand.makeAddr(amIndirectX());
            case AddrMode.INDIRECT_Y: return Operand.makeAddr(amIndirectY(Dummy.NONE));
            case AddrMode.RELATIVE: return Operand.makeAddr(amRelative());
            default: return Operand.makeNone();
        }
    }

    public int amImmediate() {
        int addr = pc;
        pc = (pc + 1) & 0xFFFF;
        return addr;
    }

    public int amZeroPage() { return fetchByte(); }

    public int amZeroPageX(int dummy) {
        int b = fetchByte();
        if (dummy != Dummy.NONE) bus.read(b);
        return (b + x) & 0xFF;
    }

    public int amZeroPageY(int dummy) {
        int b = fetchByte();
        if (dummy != Dummy.NONE) bus.read(b);
        return (b + y) & 0xFF;
    }

    public int amAbsolute() { return fetchWord(); }

    public int amAbsoluteX(int dummy) {
        int b = fetchWord();
        int eff = (b + x) & 0xFFFF;
        boolean cross = (b & 0xFF00) != (eff & 0xFF00);
        pageCrossFlag = cross;
        if ((dummy == Dummy.READ && cross) || dummy == Dummy.RMW)
            bus.read((b & 0xFF00) | (eff & 0x00FF));
        return eff;
    }

    public int amAbsoluteY(int dummy) {
        int b = fetchWord();
        int eff = (b + y) & 0xFFFF;
        boolean cross = (b & 0xFF00) != (eff & 0xFF00);
        pageCrossFlag = cross;
        if ((dummy == Dummy.READ && cross) || dummy == Dummy.RMW)
            bus.read((b & 0xFF00) | (eff & 0x00FF));
        return eff;
    }

    public int amIndirect() {
        int ptr = fetchWord();
        int lo = bus.read(ptr) & 0xFF;
        int hiAddr = (ptr & 0xFF00) | ((ptr + 1) & 0x00FF);
        int hi = bus.read(hiAddr) & 0xFF;
        return (lo | (hi << 8)) & 0xFFFF;
    }

    public int amIndirectX() {
        int zp = fetchByte();
        int ptr = (zp + x) & 0xFF;
        int lo = bus.read(ptr) & 0xFF;
        int hi = bus.read((ptr + 1) & 0xFF) & 0xFF;
        return (lo | (hi << 8)) & 0xFFFF;
    }

    public int amIndirectY(int dummy) {
        int zp = fetchByte();
        int lo = bus.read(zp) & 0xFF;
        int hi = bus.read((zp + 1) & 0xFF) & 0xFF;
        int b = (lo | (hi << 8)) & 0xFFFF;
        int eff = (b + y) & 0xFFFF;
        boolean cross = (b & 0xFF00) != (eff & 0xFF00);
        if ((dummy == Dummy.READ && cross) || dummy == Dummy.RMW)
            bus.read((b & 0xFF00) | (eff & 0x00FF));
        return eff;
    }

    public int amRelative() {
        int offset = fetchByte();
        if ((offset & 0x80) != 0) offset -= 0x100;
        return (pc + offset) & 0xFFFF;
    }

    public int amIndirectYBase() {
        int zp = fetchByte();
        int lo = bus.read(zp) & 0xFF;
        int hi = bus.read((zp + 1) & 0xFF) & 0xFF;
        return (lo | (hi << 8)) & 0xFFFF;
    }

    // ---- read/write helpers (opcodes.rs rd_*/wr_*) ----

    public int rdImm() { return fetchByte(); }
    public int rdZp() { return readOperand(resolve(AddrMode.ZERO_PAGE)); }
    public int rdZpX() { return bus.read(amZeroPageX(Dummy.RMW)) & 0xFF; }
    public int rdZpY() { return bus.read(amZeroPageY(Dummy.RMW)) & 0xFF; }
    public int rdAbs() { return readOperand(resolve(AddrMode.ABSOLUTE)); }
    public int rdAbsX(boolean[] pc) { pc[0] = false; int a = amAbsoluteX(Dummy.READ); int v = bus.read(a) & 0xFF; pc[0] = pageCrossFlag; return v; }
    public int rdAbsY(boolean[] pc) { pc[0] = false; int a = amAbsoluteY(Dummy.READ); int v = bus.read(a) & 0xFF; pc[0] = pageCrossFlag; return v; }
    public int rdIndX() { return bus.read(amIndirectX()) & 0xFF; }
    public int rdIndY(boolean[] pc) { pc[0] = false; int a = amIndirectY(Dummy.READ); int v = bus.read(a) & 0xFF; pc[0] = pageCrossFlag; return v; }
    // No-arg variants: read pageCrossFlag field directly (avoids boolean[1] allocation in hot path).
    public int rdAbsX() { int a = amAbsoluteX(Dummy.READ); return bus.read(a) & 0xFF; }
    public int rdAbsY() { int a = amAbsoluteY(Dummy.READ); return bus.read(a) & 0xFF; }
    public int rdIndY() { int a = amIndirectY(Dummy.READ); return bus.read(a) & 0xFF; }

    public void wrZp(int v) { writeOperand(resolve(AddrMode.ZERO_PAGE), v); }
    public void wrZpX(int v) { bus.write(amZeroPageX(Dummy.RMW), v); }
    public void wrZpY(int v) { bus.write(amZeroPageY(Dummy.RMW), v); }
    public void wrAbs(int v) { writeOperand(resolve(AddrMode.ABSOLUTE), v); }
    public void wrAbsX(int v) { bus.write(amAbsoluteX(Dummy.RMW), v); }
    public void wrAbsY(int v) { bus.write(amAbsoluteY(Dummy.RMW), v); }
    public void wrIndX(int v) { bus.write(amIndirectX(), v); }
    public void wrIndY(int v) { bus.write(amIndirectY(Dummy.RMW), v); }

    public Operand opZp() { return resolve(AddrMode.ZERO_PAGE); }
    public Operand opZpX() { return Operand.makeAddr(amZeroPageX(Dummy.RMW)); }
    public Operand opAbs() { return resolve(AddrMode.ABSOLUTE); }
    public Operand opAbsX() { return Operand.makeAddr(amAbsoluteX(Dummy.RMW)); }
    public Operand opAbsY() { return Operand.makeAddr(amAbsoluteY(Dummy.RMW)); }
    public Operand opIndX() { return Operand.makeAddr(amIndirectX()); }
    public Operand opIndY() { return Operand.makeAddr(amIndirectY(Dummy.RMW)); }

    // Set by amAbsoluteX/amAbsoluteY/amIndirectY when page crossed.
    public boolean pageCrossFlag;

    public void rmw(Operand op, ValueFn f) {
        int v = readOperand(op);
        int neu = f.apply(v) & 0xFF;
        writeOperand(op, neu);
        setNz(neu);
    }

    public void rmwZp(ValueFn f) { rmw(resolve(AddrMode.ZERO_PAGE), f); }
    public void rmwZpX(ValueFn f) { rmw(Operand.makeAddr(amZeroPageX(Dummy.RMW)), f); }
    public void rmwAbs(ValueFn f) { rmw(resolve(AddrMode.ABSOLUTE), f); }
    public void rmwAbsX(ValueFn f) { rmw(Operand.makeAddr(amAbsoluteX(Dummy.RMW)), f); }

    // ---- opcode handlers (port of CpuOpcodes.cs) ----

    void lda(int v) { a = v & 0xFF; setNz(a); }
    void ldx(int v) { x = v & 0xFF; setNz(x); }
    void ldy(int v) { y = v & 0xFF; setNz(y); }
    void tax() { x = a; setNz(x); }
    void tay() { y = a; setNz(y); }
    void txa() { a = x; setNz(a); }
    void tya() { a = y; setNz(a); }
    void tsx() { x = sp; setNz(x); }
    void setSpFromX() { sp = x & 0xFF; }

    void andOp(int v) { a = (a & v) & 0xFF; setNz(a); }
    void oraOp(int v) { a = (a | v) & 0xFF; setNz(a); }
    void eorOp(int v) { a = (a ^ v) & 0xFF; setNz(a); }

    void bitOp(int m) {
        int result = a & m;
        setZero(result == 0);
        setNegative((m & 0x80) != 0);
        setOverflow((m & 0x40) != 0);
    }

    void adc(int m) {
        int av = a & 0xFF;
        int mm = m & 0xFF;
        int cc = carry() ? 1 : 0;
        int sum = av + mm + cc;
        setCarry(sum > 0xFF);
        int result = sum & 0xFF;
        setOverflow((((av ^ mm) & 0x80) == 0) && (((av ^ sum) & 0x80) != 0));
        a = result;
        setNz(result);
    }

    void sbc(int m) {
        int av = a & 0xFF;
        int mm = (~m) & 0xFF;
        int cc = carry() ? 1 : 0;
        int sum = av + mm + cc;
        setCarry(sum > 0xFF);
        int result = sum & 0xFF;
        setOverflow((((av ^ mm) & 0x80) == 0) && (((av ^ sum) & 0x80) != 0));
        a = result;
        setNz(result);
    }

    void cmpOp(int r, int m) {
        int rr = r & 0xFF;
        int mm = m & 0xFF;
        int diff = (rr - mm) & 0xFF;
        setCarry(rr >= mm);
        setNz(diff);
    }

    int aslValue(int v) { setCarry((v & 0x80) != 0); return (v << 1) & 0xFF; }
    int lsrValue(int v) { setCarry((v & 0x01) != 0); return (v >>> 1) & 0xFF; }
    int rolValue(int v) {
        boolean newC = (v & 0x80) != 0;
        int result = ((v << 1) | (carry() ? 1 : 0)) & 0xFF;
        setCarry(newC);
        return result;
    }
    int rorValue(int v) {
        boolean newC = (v & 0x01) != 0;
        int result = ((v >>> 1) | (carry() ? 0x80 : 0x00)) & 0xFF;
        setCarry(newC);
        return result;
    }
    int incValue(int v) { return (v + 1) & 0xFF; }
    int decValue(int v) { return (v - 1) & 0xFF; }

    int branch(boolean branchOnSet, int condFlag) {
        boolean flagSet = (status & condFlag) != 0;
        boolean take = (flagSet == branchOnSet);
        if (!take) {
            fetchByte();
            return 2;
        }
        int pcBefore = pc;
        int target = amRelative();
        int pcAfter = (pcBefore + 1) & 0xFFFF;
        boolean pageCross = (pcAfter & 0xFF00) != (target & 0xFF00);
        pc = target;
        return 2 + 1 + (pageCross ? 1 : 0);
    }

    // ---- Official opcode dispatch (port of CpuExecute.cs) ----
    public int execute(int opcode) {
        switch (opcode) {
            // LDA
            case 0xA9: { int v = rdImm(); lda(v); return 2; }
            case 0xA5: { int v = rdZp(); lda(v); return 3; }
            case 0xB5: { int v = rdZpX(); lda(v); return 4; }
            case 0xAD: { int v = rdAbs(); lda(v); return 4; }
            case 0xBD: { int v = rdAbsX(); lda(v); return 4 + (pageCrossFlag ? 1 : 0); }
            case 0xB9: { int v = rdAbsY(); lda(v); return 4 + (pageCrossFlag ? 1 : 0); }
            case 0xA1: { int v = rdIndX(); lda(v); return 6; }
            case 0xB1: { int v = rdIndY(); lda(v); return 5 + (pageCrossFlag ? 1 : 0); }

            // LDX
            case 0xA2: { int v = rdImm(); ldx(v); return 2; }
            case 0xA6: { int v = rdZp(); ldx(v); return 3; }
            case 0xB6: { int v = rdZpY(); ldx(v); return 4; }
            case 0xAE: { int v = rdAbs(); ldx(v); return 4; }
            case 0xBE: { int v = rdAbsY(); ldx(v); return 4 + (pageCrossFlag ? 1 : 0); }

            // LDY
            case 0xA0: { int v = rdImm(); ldy(v); return 2; }
            case 0xA4: { int v = rdZp(); ldy(v); return 3; }
            case 0xB4: { int v = rdZpX(); ldy(v); return 4; }
            case 0xAC: { int v = rdAbs(); ldy(v); return 4; }
            case 0xBC: { int v = rdAbsX(); ldy(v); return 4 + (pageCrossFlag ? 1 : 0); }

            // STA
            case 0x85: wrZp(a); return 3;
            case 0x95: wrZpX(a); return 4;
            case 0x8D: wrAbs(a); return 4;
            case 0x9D: wrAbsX(a); return 5;
            case 0x99: wrAbsY(a); return 5;
            case 0x81: wrIndX(a); return 6;
            case 0x91: wrIndY(a); return 6;

            // STX
            case 0x86: wrZp(x); return 3;
            case 0x96: wrZpY(x); return 4;
            case 0x8E: wrAbs(x); return 4;

            // STY
            case 0x84: wrZp(y); return 3;
            case 0x94: wrZpX(y); return 4;
            case 0x8C: wrAbs(y); return 4;

            // transfers
            case 0xAA: tax(); return 2;
            case 0xA8: tay(); return 2;
            case 0x8A: txa(); return 2;
            case 0x98: tya(); return 2;
            case 0xBA: tsx(); return 2;
            case 0x9A: setSpFromX(); return 2;

            // stack
            case 0x48: push(a); return 3;
            case 0x08: pushStatus(true); return 3;
            case 0x68: { int v = pull(); lda(v); return 4; }
            case 0x28: pullStatus(); return 4;

            // AND
            case 0x29: { int v = rdImm(); andOp(v); return 2; }
            case 0x25: { int v = rdZp(); andOp(v); return 3; }
            case 0x35: { int v = rdZpX(); andOp(v); return 4; }
            case 0x2D: { int v = rdAbs(); andOp(v); return 4; }
            case 0x3D: { int v = rdAbsX(); andOp(v); return 4 + (pageCrossFlag ? 1 : 0); }
            case 0x39: { int v = rdAbsY(); andOp(v); return 4 + (pageCrossFlag ? 1 : 0); }
            case 0x21: { int v = rdIndX(); andOp(v); return 6; }
            case 0x31: { int v = rdIndY(); andOp(v); return 5 + (pageCrossFlag ? 1 : 0); }

            // ORA
            case 0x09: { int v = rdImm(); oraOp(v); return 2; }
            case 0x05: { int v = rdZp(); oraOp(v); return 3; }
            case 0x15: { int v = rdZpX(); oraOp(v); return 4; }
            case 0x0D: { int v = rdAbs(); oraOp(v); return 4; }
            case 0x1D: { int v = rdAbsX(); oraOp(v); return 4 + (pageCrossFlag ? 1 : 0); }
            case 0x19: { int v = rdAbsY(); oraOp(v); return 4 + (pageCrossFlag ? 1 : 0); }
            case 0x01: { int v = rdIndX(); oraOp(v); return 6; }
            case 0x11: { int v = rdIndY(); oraOp(v); return 5 + (pageCrossFlag ? 1 : 0); }

            // EOR
            case 0x49: { int v = rdImm(); eorOp(v); return 2; }
            case 0x45: { int v = rdZp(); eorOp(v); return 3; }
            case 0x55: { int v = rdZpX(); eorOp(v); return 4; }
            case 0x4D: { int v = rdAbs(); eorOp(v); return 4; }
            case 0x5D: { int v = rdAbsX(); eorOp(v); return 4 + (pageCrossFlag ? 1 : 0); }
            case 0x59: { int v = rdAbsY(); eorOp(v); return 4 + (pageCrossFlag ? 1 : 0); }
            case 0x41: { int v = rdIndX(); eorOp(v); return 6; }
            case 0x51: { int v = rdIndY(); eorOp(v); return 5 + (pageCrossFlag ? 1 : 0); }

            // BIT
            case 0x24: { int v = rdZp(); bitOp(v); return 3; }
            case 0x2C: { int v = rdAbs(); bitOp(v); return 4; }

            // ADC
            case 0x69: { int v = rdImm(); adc(v); return 2; }
            case 0x65: { int v = rdZp(); adc(v); return 3; }
            case 0x75: { int v = rdZpX(); adc(v); return 4; }
            case 0x6D: { int v = rdAbs(); adc(v); return 4; }
            case 0x7D: { int v = rdAbsX(); adc(v); return 4 + (pageCrossFlag ? 1 : 0); }
            case 0x79: { int v = rdAbsY(); adc(v); return 4 + (pageCrossFlag ? 1 : 0); }
            case 0x61: { int v = rdIndX(); adc(v); return 6; }
            case 0x71: { int v = rdIndY(); adc(v); return 5 + (pageCrossFlag ? 1 : 0); }

            // SBC
            case 0xE9: { int v = rdImm(); sbc(v); return 2; }
            case 0xE5: { int v = rdZp(); sbc(v); return 3; }
            case 0xF5: { int v = rdZpX(); sbc(v); return 4; }
            case 0xED: { int v = rdAbs(); sbc(v); return 4; }
            case 0xFD: { int v = rdAbsX(); sbc(v); return 4 + (pageCrossFlag ? 1 : 0); }
            case 0xF9: { int v = rdAbsY(); sbc(v); return 4 + (pageCrossFlag ? 1 : 0); }
            case 0xE1: { int v = rdIndX(); sbc(v); return 6; }
            case 0xF1: { int v = rdIndY(); sbc(v); return 5 + (pageCrossFlag ? 1 : 0); }

            // CMP
            case 0xC9: { int v = rdImm(); cmpOp(a, v); return 2; }
            case 0xC5: { int v = rdZp(); cmpOp(a, v); return 3; }
            case 0xD5: { int v = rdZpX(); cmpOp(a, v); return 4; }
            case 0xCD: { int v = rdAbs(); cmpOp(a, v); return 4; }
            case 0xDD: { int v = rdAbsX(); cmpOp(a, v); return 4 + (pageCrossFlag ? 1 : 0); }
            case 0xD9: { int v = rdAbsY(); cmpOp(a, v); return 4 + (pageCrossFlag ? 1 : 0); }
            case 0xC1: { int v = rdIndX(); cmpOp(a, v); return 6; }
            case 0xD1: { int v = rdIndY(); cmpOp(a, v); return 5 + (pageCrossFlag ? 1 : 0); }

            // CPX/CPY
            case 0xE0: { int v = rdImm(); cmpOp(x, v); return 2; }
            case 0xE4: { int v = rdZp(); cmpOp(x, v); return 3; }
            case 0xEC: { int v = rdAbs(); cmpOp(x, v); return 4; }
            case 0xC0: { int v = rdImm(); cmpOp(y, v); return 2; }
            case 0xC4: { int v = rdZp(); cmpOp(y, v); return 3; }
            case 0xCC: { int v = rdAbs(); cmpOp(y, v); return 4; }

            // INC/DEC memory
            case 0xE6: rmwZp(v -> incValue(v)); return 5;
            case 0xF6: rmwZpX(v -> incValue(v)); return 6;
            case 0xEE: rmwAbs(v -> incValue(v)); return 6;
            case 0xFE: rmwAbsX(v -> incValue(v)); return 7;
            case 0xC6: rmwZp(v -> decValue(v)); return 5;
            case 0xD6: rmwZpX(v -> decValue(v)); return 6;
            case 0xCE: rmwAbs(v -> decValue(v)); return 6;
            case 0xDE: rmwAbsX(v -> decValue(v)); return 7;

            // INX/INY/DEX/DEY
            case 0xE8: x = (x + 1) & 0xFF; setNz(x); return 2;
            case 0xC8: y = (y + 1) & 0xFF; setNz(y); return 2;
            case 0xCA: x = (x - 1) & 0xFF; setNz(x); return 2;
            case 0x88: y = (y - 1) & 0xFF; setNz(y); return 2;

            // ASL
            case 0x0A: a = aslValue(a); setNz(a); return 2;
            case 0x06: rmwZp(v -> aslValue(v)); return 5;
            case 0x16: rmwZpX(v -> aslValue(v)); return 6;
            case 0x0E: rmwAbs(v -> aslValue(v)); return 6;
            case 0x1E: rmwAbsX(v -> aslValue(v)); return 7;

            // LSR
            case 0x4A: a = lsrValue(a); setNz(a); return 2;
            case 0x46: rmwZp(v -> lsrValue(v)); return 5;
            case 0x56: rmwZpX(v -> lsrValue(v)); return 6;
            case 0x4E: rmwAbs(v -> lsrValue(v)); return 6;
            case 0x5E: rmwAbsX(v -> lsrValue(v)); return 7;

            // ROL
            case 0x2A: a = rolValue(a); setNz(a); return 2;
            case 0x26: rmwZp(v -> rolValue(v)); return 5;
            case 0x36: rmwZpX(v -> rolValue(v)); return 6;
            case 0x2E: rmwAbs(v -> rolValue(v)); return 6;
            case 0x3E: rmwAbsX(v -> rolValue(v)); return 7;

            // ROR
            case 0x6A: a = rorValue(a); setNz(a); return 2;
            case 0x66: rmwZp(v -> rorValue(v)); return 5;
            case 0x76: rmwZpX(v -> rorValue(v)); return 6;
            case 0x6E: rmwAbs(v -> rorValue(v)); return 6;
            case 0x7E: rmwAbsX(v -> rorValue(v)); return 7;

            // branches
            case 0x10: return branch(false, FLAG_N);
            case 0x30: return branch(true, FLAG_N);
            case 0x50: return branch(false, FLAG_V);
            case 0x70: return branch(true, FLAG_V);
            case 0x90: return branch(false, FLAG_C);
            case 0xB0: return branch(true, FLAG_C);
            case 0xD0: return branch(false, FLAG_Z);
            case 0xF0: return branch(true, FLAG_Z);

            // JMP/JSR/RTS/RTI/BRK
            case 0x4C: { int a2 = amAbsolute(); pc = a2; return 3; }
            case 0x6C: { int a2 = amIndirect(); pc = a2; return 5; }
            case 0x20: {
                int target = fetchWord();
                pushPc((pc - 1) & 0xFFFF);
                pc = target;
                return 6;
            }
            case 0x60: {
                int ret = (pullPc() + 1) & 0xFFFF;
                pc = ret;
                return 6;
            }
            case 0x40: {
                pullStatus();
                pc = pullPc();
                return 6;
            }
            case 0x00: {
                fetchByte();
                pushPc(pc);
                pushStatus(true);
                setInterruptDisable(true);
                pc = readVector(VECTOR_IRQ);
                return 7;
            }

            // flag ops
            case 0x18: setCarry(false); return 2;
            case 0x38: setCarry(true); return 2;
            case 0x58: setInterruptDisable(false); return 2;
            case 0x78: setInterruptDisable(true); return 2;
            case 0xB8: setOverflow(false); return 2;
            case 0xD8: setDecimal(false); return 2;
            case 0xF8: setDecimal(true); return 2;

            // NOP
            case 0xEA: return 2;

            default: return CpuUnofficial.execute(this, opcode);
        }
    }
}
