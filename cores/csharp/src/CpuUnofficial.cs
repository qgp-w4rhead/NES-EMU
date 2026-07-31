namespace NesCore;

// Unofficial / illegal 6502 opcodes (unofficial.rs + unofficial_rmw.rs +
// unofficial_special.rs).
public sealed partial class Cpu
{
    private const byte XaaMagic = 0x00;

    // ---- immediate combined ops ----
    private void Lax(byte m) { A = m; X = m; SetNz(m); }
    private void Anc(byte m) { A = (byte)(A & m); SetNz(A); SetCarry((A & 0x80) != 0); }
    private void Alr(byte m)
    {
        byte v = (byte)(A & m);
        SetCarry((v & 0x01) != 0);
        byte result = (byte)(v >> 1);
        A = result;
        SetNz(result);
    }
    private void Arr(byte m)
    {
        byte v = (byte)(A & m);
        byte result = (byte)((v >> 1) | (Carry() ? 0x80 : 0x00));
        A = result;
        SetNz(result);
        SetCarry((result & 0x40) != 0);
        SetOverflow(((result ^ (byte)(result << 1)) & 0x40) != 0);
    }
    private void Axs(byte m)
    {
        byte ax = (byte)(A & X);
        SetCarry(ax >= m);
        byte result = (byte)(ax - m);
        X = result;
        SetNz(result);
    }
    private void Xaa(byte m)
    {
        byte result = (byte)((byte)((A | XaaMagic) & X) & m);
        A = result;
        SetNz(result);
    }

    // ---- unstable indexed stores ----
    private static ushort StoreDummyReadAddr(ushort b, byte index)
        => (ushort)((b & 0xFF00) | ((b + index) & 0x00FF));

    private static ushort QuirkAddrY(ushort b, byte y, byte value)
    {
        ushort eff = (ushort)(b + y);
        if ((b & 0xFF00) != (eff & 0xFF00))
            return (ushort)(((ushort)value << 8) | (eff & 0x00FF));
        return eff;
    }

    private static ushort QuirkAddrX(ushort b, byte x, byte value)
    {
        ushort eff = (ushort)(b + x);
        if ((b & 0xFF00) != (eff & 0xFF00))
            return (ushort)(((ushort)value << 8) | (eff & 0x00FF));
        return eff;
    }

    private void TasStore(ushort b)
    {
        Sp = (byte)(A & X);
        byte h = (byte)(b >> 8);
        byte value = (byte)(Sp & (byte)(h + 1));
        ushort storeAddr = QuirkAddrY(b, Y, value);
        Bus.Read(StoreDummyReadAddr(b, Y));
        Bus.Write(storeAddr, value);
    }

    private void AhxStore(ushort b, byte reg)
    {
        byte h = (byte)(b >> 8);
        byte value = (byte)(reg & (byte)(h + 1));
        ushort storeAddr = QuirkAddrY(b, Y, value);
        Bus.Read(StoreDummyReadAddr(b, Y));
        Bus.Write(storeAddr, value);
    }

    private void ShyStore(ushort b)
    {
        byte h = (byte)(b >> 8);
        byte value = (byte)(Y & (byte)(h + 1));
        ushort storeAddr = QuirkAddrX(b, X, value);
        Bus.Read(StoreDummyReadAddr(b, X));
        Bus.Write(storeAddr, value);
    }

    // ---- RMW-combo helpers ----
    private void RmwCombo(Operand op, ValueFn f)
    {
        byte v = ReadOperand(op);
        byte neu = f(v);
        WriteOperand(op, neu);
        SetNz(neu);
    }

    private void Dcp(Operand op) { RmwCombo(op, DecValue); CmpOp(A, ReadOperand(op)); }
    private void Isc(Operand op) { RmwCombo(op, IncValue); Sbc(ReadOperand(op)); }
    private void Slo(Operand op) { RmwCombo(op, AslValue); OraOp(ReadOperand(op)); }
    private void Rla(Operand op) { RmwCombo(op, RolValue); AndOp(ReadOperand(op)); }
    private void Sre(Operand op) { RmwCombo(op, LsrValue); EorOp(ReadOperand(op)); }
    private void Rra(Operand op) { RmwCombo(op, RorValue); Adc(ReadOperand(op)); }

    private byte ExecuteUnofficialRmw(byte opcode)
    {
        switch (opcode)
        {
            // DCP
            case 0xC7: { var o = OpZp(); Dcp(o); return 5; }
            case 0xD7: { var o = OpZpX(); Dcp(o); return 6; }
            case 0xCF: { var o = OpAbs(); Dcp(o); return 6; }
            case 0xDF: { var o = OpAbsX(); Dcp(o); return 7; }
            case 0xDB: { var o = OpAbsY(); Dcp(o); return 7; }
            case 0xC3: { var o = OpIndX(); Dcp(o); return 8; }
            case 0xD3: { var o = OpIndY(); Dcp(o); return 8; }

            // ISC
            case 0xE7: { var o = OpZp(); Isc(o); return 5; }
            case 0xF7: { var o = OpZpX(); Isc(o); return 6; }
            case 0xEF: { var o = OpAbs(); Isc(o); return 6; }
            case 0xFF: { var o = OpAbsX(); Isc(o); return 7; }
            case 0xFB: { var o = OpAbsY(); Isc(o); return 7; }
            case 0xE3: { var o = OpIndX(); Isc(o); return 8; }
            case 0xF3: { var o = OpIndY(); Isc(o); return 8; }

            // SLO
            case 0x07: { var o = OpZp(); Slo(o); return 5; }
            case 0x17: { var o = OpZpX(); Slo(o); return 6; }
            case 0x0F: { var o = OpAbs(); Slo(o); return 6; }
            case 0x1F: { var o = OpAbsX(); Slo(o); return 7; }
            case 0x1B: { var o = OpAbsY(); Slo(o); return 7; }
            case 0x03: { var o = OpIndX(); Slo(o); return 8; }
            case 0x13: { var o = OpIndY(); Slo(o); return 8; }

            // RLA
            case 0x27: { var o = OpZp(); Rla(o); return 5; }
            case 0x37: { var o = OpZpX(); Rla(o); return 6; }
            case 0x2F: { var o = OpAbs(); Rla(o); return 6; }
            case 0x3F: { var o = OpAbsX(); Rla(o); return 7; }
            case 0x3B: { var o = OpAbsY(); Rla(o); return 7; }
            case 0x23: { var o = OpIndX(); Rla(o); return 8; }
            case 0x33: { var o = OpIndY(); Rla(o); return 8; }

            // SRE
            case 0x47: { var o = OpZp(); Sre(o); return 5; }
            case 0x57: { var o = OpZpX(); Sre(o); return 6; }
            case 0x4F: { var o = OpAbs(); Sre(o); return 6; }
            case 0x5F: { var o = OpAbsX(); Sre(o); return 7; }
            case 0x5B: { var o = OpAbsY(); Sre(o); return 7; }
            case 0x43: { var o = OpIndX(); Sre(o); return 8; }
            case 0x53: { var o = OpIndY(); Sre(o); return 8; }

            // RRA
            case 0x67: { var o = OpZp(); Rra(o); return 5; }
            case 0x77: { var o = OpZpX(); Rra(o); return 6; }
            case 0x6F: { var o = OpAbs(); Rra(o); return 6; }
            case 0x7F: { var o = OpAbsX(); Rra(o); return 7; }
            case 0x7B: { var o = OpAbsY(); Rra(o); return 7; }
            case 0x63: { var o = OpIndX(); Rra(o); return 8; }
            case 0x73: { var o = OpIndY(); Rra(o); return 8; }

            // TAS/SHS
            case 0x9B: { ushort b = FetchWord(); TasStore(b); return 5; }

            // AHX/SHA
            case 0x9F: { ushort b = FetchWord(); AhxStore(b, (byte)(A & X)); return 5; }
            case 0x93: { ushort b = AmIndirectYBase(); AhxStore(b, (byte)(A & X)); return 6; }

            // SHX/SXA
            case 0x9E: { ushort b = FetchWord(); AhxStore(b, X); return 5; }

            // SHY/SYA
            case 0x9C: { ushort b = FetchWord(); ShyStore(b); return 5; }

            default: return 2;
        }
    }

    private byte ExecuteUnofficial(byte opcode)
    {
        switch (opcode)
        {
            // Implied NOPs
            case 0x1A: case 0x3A: case 0x5A: case 0x7A: case 0xDA: case 0xFA: return 2;
            // Immediate NOPs
            case 0x80: case 0x82: case 0x89: case 0xC2: case 0xE2: FetchByte(); return 2;
            // Zero-page NOPs
            case 0x04: case 0x44: case 0x64: FetchByte(); return 3;
            // Zero-page,X NOPs
            case 0x14: case 0x34: case 0x54: case 0x74: case 0xD4: case 0xF4: AmZeroPageX(Dummy.Rmw); return 4;
            // Absolute NOP
            case 0x0C: FetchWord(); return 4;
            // Absolute,X NOPs
            case 0x1C: case 0x3C: case 0x5C: case 0x7C: case 0xDC: case 0xFC:
            {
                AmAbsoluteX(Dummy.Read, out bool pc);
                return (byte)(4 + (pc ? 1 : 0));
            }

            // LAX
            case 0xA7: { byte v = RdZp(); Lax(v); return 3; }
            case 0xB7: { byte v = RdZpY(); Lax(v); return 4; }
            case 0xAF: { byte v = RdAbs(); Lax(v); return 4; }
            case 0xBF: { byte v = RdAbsY(out bool pc); Lax(v); return (byte)(4 + (pc ? 1 : 0)); }
            case 0xA3: { byte v = RdIndX(); Lax(v); return 6; }
            case 0xB3: { byte v = RdIndY(out bool pc); Lax(v); return (byte)(5 + (pc ? 1 : 0)); }

            // SAX
            case 0x87: WrZp((byte)(A & X)); return 3;
            case 0x97: WrZpY((byte)(A & X)); return 4;
            case 0x8F: WrAbs((byte)(A & X)); return 4;
            case 0x83: WrIndX((byte)(A & X)); return 6;

            // ANC
            case 0x0B: case 0x2B: { byte v = RdImm(); Anc(v); return 2; }
            // ALR
            case 0x4B: { byte v = RdImm(); Alr(v); return 2; }
            // ARR
            case 0x6B: { byte v = RdImm(); Arr(v); return 2; }
            // AXS/SBX
            case 0xCB: { byte v = RdImm(); Axs(v); return 2; }
            // XAA
            case 0x8B: { byte v = RdImm(); Xaa(v); return 2; }
            // LAS/LAR
            case 0xBB:
            {
                byte v = RdAbsY(out bool pc);
                byte r = (byte)(v & Sp);
                A = r; X = r; Sp = r;
                SetNz(r);
                return (byte)(4 + (pc ? 1 : 0));
            }

            // KIL/JAM/HLT
            case 0x02: case 0x12: case 0x22: case 0x32:
            case 0x42: case 0x52: case 0x62: case 0x72:
            case 0x92: case 0xB2: case 0xD2: case 0xF2:
                SetHalted(true);
                return 1;

            default: return ExecuteUnofficialRmw(opcode);
        }
    }
}
