namespace NesCore;

// Official opcode dispatch (opcodes.rs execute).
public sealed partial class Cpu
{
    public byte Execute(byte opcode)
    {
        switch (opcode)
        {
            // LDA
            case 0xA9: { byte v = RdImm(); Lda(v); return 2; }
            case 0xA5: { byte v = RdZp(); Lda(v); return 3; }
            case 0xB5: { byte v = RdZpX(); Lda(v); return 4; }
            case 0xAD: { byte v = RdAbs(); Lda(v); return 4; }
            case 0xBD: { byte v = RdAbsX(out bool pc); Lda(v); return (byte)(4 + (pc ? 1 : 0)); }
            case 0xB9: { byte v = RdAbsY(out bool pc); Lda(v); return (byte)(4 + (pc ? 1 : 0)); }
            case 0xA1: { byte v = RdIndX(); Lda(v); return 6; }
            case 0xB1: { byte v = RdIndY(out bool pc); Lda(v); return (byte)(5 + (pc ? 1 : 0)); }

            // LDX
            case 0xA2: { byte v = RdImm(); Ldx(v); return 2; }
            case 0xA6: { byte v = RdZp(); Ldx(v); return 3; }
            case 0xB6: { byte v = RdZpY(); Ldx(v); return 4; }
            case 0xAE: { byte v = RdAbs(); Ldx(v); return 4; }
            case 0xBE: { byte v = RdAbsY(out bool pc); Ldx(v); return (byte)(4 + (pc ? 1 : 0)); }

            // LDY
            case 0xA0: { byte v = RdImm(); Ldy(v); return 2; }
            case 0xA4: { byte v = RdZp(); Ldy(v); return 3; }
            case 0xB4: { byte v = RdZpX(); Ldy(v); return 4; }
            case 0xAC: { byte v = RdAbs(); Ldy(v); return 4; }
            case 0xBC: { byte v = RdAbsX(out bool pc); Ldy(v); return (byte)(4 + (pc ? 1 : 0)); }

            // STA
            case 0x85: WrZp(A); return 3;
            case 0x95: WrZpX(A); return 4;
            case 0x8D: WrAbs(A); return 4;
            case 0x9D: WrAbsX(A); return 5;
            case 0x99: WrAbsY(A); return 5;
            case 0x81: WrIndX(A); return 6;
            case 0x91: WrIndY(A); return 6;

            // STX
            case 0x86: WrZp(X); return 3;
            case 0x96: WrZpY(X); return 4;
            case 0x8E: WrAbs(X); return 4;

            // STY
            case 0x84: WrZp(Y); return 3;
            case 0x94: WrZpX(Y); return 4;
            case 0x8C: WrAbs(Y); return 4;

            // transfers
            case 0xAA: Tax(); return 2;
            case 0xA8: Tay(); return 2;
            case 0x8A: Txa(); return 2;
            case 0x98: Tya(); return 2;
            case 0xBA: Tsx(); return 2;
            case 0x9A: SetSpFromX(); return 2;

            // stack
            case 0x48: Push(A); return 3;
            case 0x08: PushStatus(true); return 3;
            case 0x68: { byte v = Pull(); Lda(v); return 4; }
            case 0x28: PullStatus(); return 4;

            // AND
            case 0x29: { byte v = RdImm(); AndOp(v); return 2; }
            case 0x25: { byte v = RdZp(); AndOp(v); return 3; }
            case 0x35: { byte v = RdZpX(); AndOp(v); return 4; }
            case 0x2D: { byte v = RdAbs(); AndOp(v); return 4; }
            case 0x3D: { byte v = RdAbsX(out bool pc); AndOp(v); return (byte)(4 + (pc ? 1 : 0)); }
            case 0x39: { byte v = RdAbsY(out bool pc); AndOp(v); return (byte)(4 + (pc ? 1 : 0)); }
            case 0x21: { byte v = RdIndX(); AndOp(v); return 6; }
            case 0x31: { byte v = RdIndY(out bool pc); AndOp(v); return (byte)(5 + (pc ? 1 : 0)); }

            // ORA
            case 0x09: { byte v = RdImm(); OraOp(v); return 2; }
            case 0x05: { byte v = RdZp(); OraOp(v); return 3; }
            case 0x15: { byte v = RdZpX(); OraOp(v); return 4; }
            case 0x0D: { byte v = RdAbs(); OraOp(v); return 4; }
            case 0x1D: { byte v = RdAbsX(out bool pc); OraOp(v); return (byte)(4 + (pc ? 1 : 0)); }
            case 0x19: { byte v = RdAbsY(out bool pc); OraOp(v); return (byte)(4 + (pc ? 1 : 0)); }
            case 0x01: { byte v = RdIndX(); OraOp(v); return 6; }
            case 0x11: { byte v = RdIndY(out bool pc); OraOp(v); return (byte)(5 + (pc ? 1 : 0)); }

            // EOR
            case 0x49: { byte v = RdImm(); EorOp(v); return 2; }
            case 0x45: { byte v = RdZp(); EorOp(v); return 3; }
            case 0x55: { byte v = RdZpX(); EorOp(v); return 4; }
            case 0x4D: { byte v = RdAbs(); EorOp(v); return 4; }
            case 0x5D: { byte v = RdAbsX(out bool pc); EorOp(v); return (byte)(4 + (pc ? 1 : 0)); }
            case 0x59: { byte v = RdAbsY(out bool pc); EorOp(v); return (byte)(4 + (pc ? 1 : 0)); }
            case 0x41: { byte v = RdIndX(); EorOp(v); return 6; }
            case 0x51: { byte v = RdIndY(out bool pc); EorOp(v); return (byte)(5 + (pc ? 1 : 0)); }

            // BIT
            case 0x24: { byte v = RdZp(); BitOp(v); return 3; }
            case 0x2C: { byte v = RdAbs(); BitOp(v); return 4; }

            // ADC
            case 0x69: { byte v = RdImm(); Adc(v); return 2; }
            case 0x65: { byte v = RdZp(); Adc(v); return 3; }
            case 0x75: { byte v = RdZpX(); Adc(v); return 4; }
            case 0x6D: { byte v = RdAbs(); Adc(v); return 4; }
            case 0x7D: { byte v = RdAbsX(out bool pc); Adc(v); return (byte)(4 + (pc ? 1 : 0)); }
            case 0x79: { byte v = RdAbsY(out bool pc); Adc(v); return (byte)(4 + (pc ? 1 : 0)); }
            case 0x61: { byte v = RdIndX(); Adc(v); return 6; }
            case 0x71: { byte v = RdIndY(out bool pc); Adc(v); return (byte)(5 + (pc ? 1 : 0)); }

            // SBC
            case 0xE9: { byte v = RdImm(); Sbc(v); return 2; }
            case 0xE5: { byte v = RdZp(); Sbc(v); return 3; }
            case 0xF5: { byte v = RdZpX(); Sbc(v); return 4; }
            case 0xED: { byte v = RdAbs(); Sbc(v); return 4; }
            case 0xFD: { byte v = RdAbsX(out bool pc); Sbc(v); return (byte)(4 + (pc ? 1 : 0)); }
            case 0xF9: { byte v = RdAbsY(out bool pc); Sbc(v); return (byte)(4 + (pc ? 1 : 0)); }
            case 0xE1: { byte v = RdIndX(); Sbc(v); return 6; }
            case 0xF1: { byte v = RdIndY(out bool pc); Sbc(v); return (byte)(5 + (pc ? 1 : 0)); }

            // CMP
            case 0xC9: { byte v = RdImm(); CmpOp(A, v); return 2; }
            case 0xC5: { byte v = RdZp(); CmpOp(A, v); return 3; }
            case 0xD5: { byte v = RdZpX(); CmpOp(A, v); return 4; }
            case 0xCD: { byte v = RdAbs(); CmpOp(A, v); return 4; }
            case 0xDD: { byte v = RdAbsX(out bool pc); CmpOp(A, v); return (byte)(4 + (pc ? 1 : 0)); }
            case 0xD9: { byte v = RdAbsY(out bool pc); CmpOp(A, v); return (byte)(4 + (pc ? 1 : 0)); }
            case 0xC1: { byte v = RdIndX(); CmpOp(A, v); return 6; }
            case 0xD1: { byte v = RdIndY(out bool pc); CmpOp(A, v); return (byte)(5 + (pc ? 1 : 0)); }

            // CPX/CPY
            case 0xE0: { byte v = RdImm(); CmpOp(X, v); return 2; }
            case 0xE4: { byte v = RdZp(); CmpOp(X, v); return 3; }
            case 0xEC: { byte v = RdAbs(); CmpOp(X, v); return 4; }
            case 0xC0: { byte v = RdImm(); CmpOp(Y, v); return 2; }
            case 0xC4: { byte v = RdZp(); CmpOp(Y, v); return 3; }
            case 0xCC: { byte v = RdAbs(); CmpOp(Y, v); return 4; }

            // INC/DEC memory
            case 0xE6: RmwZp(IncValue); return 5;
            case 0xF6: RmwZpX(IncValue); return 6;
            case 0xEE: RmwAbs(IncValue); return 6;
            case 0xFE: RmwAbsX(IncValue); return 7;
            case 0xC6: RmwZp(DecValue); return 5;
            case 0xD6: RmwZpX(DecValue); return 6;
            case 0xCE: RmwAbs(DecValue); return 6;
            case 0xDE: RmwAbsX(DecValue); return 7;

            // INX/INY/DEX/DEY
            case 0xE8: X++; SetNz(X); return 2;
            case 0xC8: Y++; SetNz(Y); return 2;
            case 0xCA: X--; SetNz(X); return 2;
            case 0x88: Y--; SetNz(Y); return 2;

            // ASL
            case 0x0A: A = AslValue(A); SetNz(A); return 2;
            case 0x06: RmwZp(AslValue); return 5;
            case 0x16: RmwZpX(AslValue); return 6;
            case 0x0E: RmwAbs(AslValue); return 6;
            case 0x1E: RmwAbsX(AslValue); return 7;

            // LSR
            case 0x4A: A = LsrValue(A); SetNz(A); return 2;
            case 0x46: RmwZp(LsrValue); return 5;
            case 0x56: RmwZpX(LsrValue); return 6;
            case 0x4E: RmwAbs(LsrValue); return 6;
            case 0x5E: RmwAbsX(LsrValue); return 7;

            // ROL
            case 0x2A: A = RolValue(A); SetNz(A); return 2;
            case 0x26: RmwZp(RolValue); return 5;
            case 0x36: RmwZpX(RolValue); return 6;
            case 0x2E: RmwAbs(RolValue); return 6;
            case 0x3E: RmwAbsX(RolValue); return 7;

            // ROR
            case 0x6A: A = RorValue(A); SetNz(A); return 2;
            case 0x66: RmwZp(RorValue); return 5;
            case 0x76: RmwZpX(RorValue); return 6;
            case 0x6E: RmwAbs(RorValue); return 6;
            case 0x7E: RmwAbsX(RorValue); return 7;

            // branches
            case 0x10: return Branch(false, FlagN);
            case 0x30: return Branch(true, FlagN);
            case 0x50: return Branch(false, FlagV);
            case 0x70: return Branch(true, FlagV);
            case 0x90: return Branch(false, FlagC);
            case 0xB0: return Branch(true, FlagC);
            case 0xD0: return Branch(false, FlagZ);
            case 0xF0: return Branch(true, FlagZ);

            // JMP/JSR/RTS/RTI/BRK
            case 0x4C: { ushort a = AmAbsolute(); Pc = a; return 3; }
            case 0x6C: { ushort a = AmIndirect(); Pc = a; return 5; }
            case 0x20:
            {
                ushort target = FetchWord();
                PushPc((ushort)(Pc - 1));
                Pc = target;
                return 6;
            }
            case 0x60:
            {
                ushort ret = (ushort)(PullPc() + 1);
                Pc = ret;
                return 6;
            }
            case 0x40:
            {
                PullStatus();
                Pc = PullPc();
                return 6;
            }
            case 0x00:
            {
                FetchByte();
                PushPc(Pc);
                PushStatus(true);
                SetInterruptDisable(true);
                Pc = ReadVector(VectorIrq);
                return 7;
            }

            // flag ops
            case 0x18: SetCarry(false); return 2;
            case 0x38: SetCarry(true); return 2;
            case 0x58: SetInterruptDisable(false); return 2;
            case 0x78: SetInterruptDisable(true); return 2;
            case 0xB8: SetOverflow(false); return 2;
            case 0xD8: SetDecimal(false); return 2;
            case 0xF8: SetDecimal(true); return 2;

            // NOP
            case 0xEA: return 2;

            default: return ExecuteUnofficial(opcode);
        }
    }
}
