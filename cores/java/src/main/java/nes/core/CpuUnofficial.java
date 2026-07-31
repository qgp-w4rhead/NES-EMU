package nes.core;

// Unofficial / illegal 6502 opcodes (port of cores/csharp/src/CpuUnofficial.cs).
// Static helpers operating on a Cpu instance (Java has no partial classes).
final class CpuUnofficial {
    private static final int XAA_MAGIC = 0x00;

    private CpuUnofficial() {}

    // ---- immediate combined ops ----
    private static void lax(Cpu c, int m) { c.a = m & 0xFF; c.x = m & 0xFF; c.setNz(m & 0xFF); }
    private static void anc(Cpu c, int m) { c.a = (c.a & m) & 0xFF; c.setNz(c.a); c.setCarry((c.a & 0x80) != 0); }
    private static void alr(Cpu c, int m) {
        int v = (c.a & m) & 0xFF;
        c.setCarry((v & 0x01) != 0);
        int result = (v >>> 1) & 0xFF;
        c.a = result;
        c.setNz(result);
    }
    private static void arr(Cpu c, int m) {
        int v = (c.a & m) & 0xFF;
        int result = ((v >>> 1) | (c.carry() ? 0x80 : 0x00)) & 0xFF;
        c.a = result;
        c.setNz(result);
        c.setCarry((result & 0x40) != 0);
        c.setOverflow(((result ^ ((result << 1) & 0xFF)) & 0x40) != 0);
    }
    private static void axs(Cpu c, int m) {
        int ax = (c.a & c.x) & 0xFF;
        int mm = m & 0xFF;
        c.setCarry(ax >= mm);
        int result = (ax - mm) & 0xFF;
        c.x = result;
        c.setNz(result);
    }
    private static void xaa(Cpu c, int m) {
        int result = (((c.a | XAA_MAGIC) & c.x) & m) & 0xFF;
        c.a = result;
        c.setNz(result);
    }

    // ---- unstable indexed stores ----
    private static int storeDummyReadAddr(int b, int index) {
        return (b & 0xFF00) | ((b + index) & 0x00FF);
    }
    private static int quirkAddrY(int b, int y, int value) {
        int eff = (b + y) & 0xFFFF;
        if ((b & 0xFF00) != (eff & 0xFF00))
            return (((value & 0xFF) << 8) | (eff & 0x00FF)) & 0xFFFF;
        return eff;
    }
    private static int quirkAddrX(int b, int x, int value) {
        int eff = (b + x) & 0xFFFF;
        if ((b & 0xFF00) != (eff & 0xFF00))
            return (((value & 0xFF) << 8) | (eff & 0x00FF)) & 0xFFFF;
        return eff;
    }

    private static void tasStore(Cpu c, int b) {
        c.sp = (c.a & c.x) & 0xFF;
        int h = (b >> 8) & 0xFF;
        int value = (c.sp & ((h + 1) & 0xFF)) & 0xFF;
        int storeAddr = quirkAddrY(b, c.y, value);
        c.bus.read(storeDummyReadAddr(b, c.y));
        c.bus.write(storeAddr, value);
    }
    private static void ahxStore(Cpu c, int b, int reg) {
        int h = (b >> 8) & 0xFF;
        int value = (reg & ((h + 1) & 0xFF)) & 0xFF;
        int storeAddr = quirkAddrY(b, c.y, value);
        c.bus.read(storeDummyReadAddr(b, c.y));
        c.bus.write(storeAddr, value);
    }
    private static void shyStore(Cpu c, int b) {
        int h = (b >> 8) & 0xFF;
        int value = (c.y & ((h + 1) & 0xFF)) & 0xFF;
        int storeAddr = quirkAddrX(b, c.x, value);
        c.bus.read(storeDummyReadAddr(b, c.x));
        c.bus.write(storeAddr, value);
    }

    // ---- RMW-combo helpers ----
    private static void rmwCombo(Cpu c, Operand op, ValueFn f) {
        int v = c.readOperand(op);
        int neu = f.apply(v) & 0xFF;
        c.writeOperand(op, neu);
        c.setNz(neu);
    }
    private static void dcp(Cpu c, Operand op) { rmwCombo(c, op, v -> c.decValue(v)); c.cmpOp(c.a, c.readOperand(op)); }
    private static void isc(Cpu c, Operand op) { rmwCombo(c, op, v -> c.incValue(v)); c.sbc(c.readOperand(op)); }
    private static void slo(Cpu c, Operand op) { rmwCombo(c, op, v -> c.aslValue(v)); c.oraOp(c.readOperand(op)); }
    private static void rla(Cpu c, Operand op) { rmwCombo(c, op, v -> c.rolValue(v)); c.andOp(c.readOperand(op)); }
    private static void sre(Cpu c, Operand op) { rmwCombo(c, op, v -> c.lsrValue(v)); c.eorOp(c.readOperand(op)); }
    private static void rra(Cpu c, Operand op) { rmwCombo(c, op, v -> c.rorValue(v)); c.adc(c.readOperand(op)); }

    private static int execRmw(Cpu c, int opcode) {
        switch (opcode) {
            // DCP
            case 0xC7: { Operand o = c.opZp(); dcp(c, o); return 5; }
            case 0xD7: { Operand o = c.opZpX(); dcp(c, o); return 6; }
            case 0xCF: { Operand o = c.opAbs(); dcp(c, o); return 6; }
            case 0xDF: { Operand o = c.opAbsX(); dcp(c, o); return 7; }
            case 0xDB: { Operand o = c.opAbsY(); dcp(c, o); return 7; }
            case 0xC3: { Operand o = c.opIndX(); dcp(c, o); return 8; }
            case 0xD3: { Operand o = c.opIndY(); dcp(c, o); return 8; }
            // ISC
            case 0xE7: { Operand o = c.opZp(); isc(c, o); return 5; }
            case 0xF7: { Operand o = c.opZpX(); isc(c, o); return 6; }
            case 0xEF: { Operand o = c.opAbs(); isc(c, o); return 6; }
            case 0xFF: { Operand o = c.opAbsX(); isc(c, o); return 7; }
            case 0xFB: { Operand o = c.opAbsY(); isc(c, o); return 7; }
            case 0xE3: { Operand o = c.opIndX(); isc(c, o); return 8; }
            case 0xF3: { Operand o = c.opIndY(); isc(c, o); return 8; }
            // SLO
            case 0x07: { Operand o = c.opZp(); slo(c, o); return 5; }
            case 0x17: { Operand o = c.opZpX(); slo(c, o); return 6; }
            case 0x0F: { Operand o = c.opAbs(); slo(c, o); return 6; }
            case 0x1F: { Operand o = c.opAbsX(); slo(c, o); return 7; }
            case 0x1B: { Operand o = c.opAbsY(); slo(c, o); return 7; }
            case 0x03: { Operand o = c.opIndX(); slo(c, o); return 8; }
            case 0x13: { Operand o = c.opIndY(); slo(c, o); return 8; }
            // RLA
            case 0x27: { Operand o = c.opZp(); rla(c, o); return 5; }
            case 0x37: { Operand o = c.opZpX(); rla(c, o); return 6; }
            case 0x2F: { Operand o = c.opAbs(); rla(c, o); return 6; }
            case 0x3F: { Operand o = c.opAbsX(); rla(c, o); return 7; }
            case 0x3B: { Operand o = c.opAbsY(); rla(c, o); return 7; }
            case 0x23: { Operand o = c.opIndX(); rla(c, o); return 8; }
            case 0x33: { Operand o = c.opIndY(); rla(c, o); return 8; }
            // SRE
            case 0x47: { Operand o = c.opZp(); sre(c, o); return 5; }
            case 0x57: { Operand o = c.opZpX(); sre(c, o); return 6; }
            case 0x4F: { Operand o = c.opAbs(); sre(c, o); return 6; }
            case 0x5F: { Operand o = c.opAbsX(); sre(c, o); return 7; }
            case 0x5B: { Operand o = c.opAbsY(); sre(c, o); return 7; }
            case 0x43: { Operand o = c.opIndX(); sre(c, o); return 8; }
            case 0x53: { Operand o = c.opIndY(); sre(c, o); return 8; }
            // RRA
            case 0x67: { Operand o = c.opZp(); rra(c, o); return 5; }
            case 0x77: { Operand o = c.opZpX(); rra(c, o); return 6; }
            case 0x6F: { Operand o = c.opAbs(); rra(c, o); return 6; }
            case 0x7F: { Operand o = c.opAbsX(); rra(c, o); return 7; }
            case 0x7B: { Operand o = c.opAbsY(); rra(c, o); return 7; }
            case 0x63: { Operand o = c.opIndX(); rra(c, o); return 8; }
            case 0x73: { Operand o = c.opIndY(); rra(c, o); return 8; }
            // TAS/SHS
            case 0x9B: { int b = c.fetchWord(); tasStore(c, b); return 5; }
            // AHX/SHA
            case 0x9F: { int b = c.fetchWord(); ahxStore(c, b, (c.a & c.x) & 0xFF); return 5; }
            case 0x93: { int b = c.amIndirectYBase(); ahxStore(c, b, (c.a & c.x) & 0xFF); return 6; }
            // SHX/SXA
            case 0x9E: { int b = c.fetchWord(); ahxStore(c, b, c.x & 0xFF); return 5; }
            // SHY/SYA
            case 0x9C: { int b = c.fetchWord(); shyStore(c, b); return 5; }
            default: return 2;
        }
    }

    static int execute(Cpu c, int opcode) {
        switch (opcode) {
            // Implied NOPs
            case 0x1A: case 0x3A: case 0x5A: case 0x7A: case 0xDA: case 0xFA: return 2;
            // Immediate NOPs
            case 0x80: case 0x82: case 0x89: case 0xC2: case 0xE2: c.fetchByte(); return 2;
            // Zero-page NOPs
            case 0x04: case 0x44: case 0x64: c.fetchByte(); return 3;
            // Zero-page,X NOPs
            case 0x14: case 0x34: case 0x54: case 0x74: case 0xD4: case 0xF4: c.amZeroPageX(Dummy.RMW); return 4;
            // Absolute NOP
            case 0x0C: c.fetchWord(); return 4;
            // Absolute,X NOPs
            case 0x1C: case 0x3C: case 0x5C: case 0x7C: case 0xDC: case 0xFC: {
                c.amAbsoluteX(Dummy.READ);
                return 4 + (c.pageCrossFlag ? 1 : 0);
            }
            // LAX
            case 0xA7: { int v = c.rdZp(); lax(c, v); return 3; }
            case 0xB7: { int v = c.rdZpY(); lax(c, v); return 4; }
            case 0xAF: { int v = c.rdAbs(); lax(c, v); return 4; }
            case 0xBF: { int v = c.rdAbsY(); lax(c, v); return 4 + (c.pageCrossFlag ? 1 : 0); }
            case 0xA3: { int v = c.rdIndX(); lax(c, v); return 6; }
            case 0xB3: { int v = c.rdIndY(); lax(c, v); return 5 + (c.pageCrossFlag ? 1 : 0); }
            // SAX
            case 0x87: c.wrZp((c.a & c.x) & 0xFF); return 3;
            case 0x97: c.wrZpY((c.a & c.x) & 0xFF); return 4;
            case 0x8F: c.wrAbs((c.a & c.x) & 0xFF); return 4;
            case 0x83: c.wrIndX((c.a & c.x) & 0xFF); return 6;
            // ANC
            case 0x0B: case 0x2B: { int v = c.rdImm(); anc(c, v); return 2; }
            // ALR
            case 0x4B: { int v = c.rdImm(); alr(c, v); return 2; }
            // ARR
            case 0x6B: { int v = c.rdImm(); arr(c, v); return 2; }
            // AXS/SBX
            case 0xCB: { int v = c.rdImm(); axs(c, v); return 2; }
            // XAA
            case 0x8B: { int v = c.rdImm(); xaa(c, v); return 2; }
            // LAS/LAR
            case 0xBB: {
                int v = c.rdAbsY();
                int r = (v & c.sp) & 0xFF;
                c.a = r; c.x = r; c.sp = r;
                c.setNz(r);
                return 4 + (c.pageCrossFlag ? 1 : 0);
            }
            // KIL/JAM/HLT
            case 0x02: case 0x12: case 0x22: case 0x32:
            case 0x42: case 0x52: case 0x62: case 0x72:
            case 0x92: case 0xB2: case 0xD2: case 0xF2:
                c.setHalted(true);
                return 1;
            default: return execRmw(c, opcode);
        }
    }
}
