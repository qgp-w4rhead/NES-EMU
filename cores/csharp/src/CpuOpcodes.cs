namespace NesCore;

// Official opcode handlers and dispatch (opcodes.rs).
public sealed partial class Cpu
{
    // ---- load/store/transfer ----
    private void Lda(byte v) { A = v; SetNz(v); }
    private void Ldx(byte v) { X = v; SetNz(v); }
    private void Ldy(byte v) { Y = v; SetNz(v); }
    private void Tax() { X = A; SetNz(X); }
    private void Tay() { Y = A; SetNz(Y); }
    private void Txa() { A = X; SetNz(A); }
    private void Tya() { A = Y; SetNz(A); }
    private void Tsx() { X = Sp; SetNz(X); }
    private void SetSpFromX() { Sp = X; }

    // ---- logic ----
    private void AndOp(byte v) { A = (byte)(A & v); SetNz(A); }
    private void OraOp(byte v) { A = (byte)(A | v); SetNz(A); }
    private void EorOp(byte v) { A = (byte)(A ^ v); SetNz(A); }

    private void BitOp(byte m)
    {
        byte result = (byte)(A & m);
        SetZero(result == 0);
        SetNegative((m & 0x80) != 0);
        SetOverflow((m & 0x40) != 0);
    }

    // ---- arithmetic ----
    private void Adc(byte m)
    {
        ushort a = A;
        ushort mm = m;
        ushort cc = Carry() ? (ushort)1 : (ushort)0;
        ushort sum = (ushort)(a + mm + cc);
        SetCarry(sum > 0xFF);
        byte result = (byte)(sum & 0xFF);
        SetOverflow((((a ^ mm) & 0x80) == 0) && (((a ^ sum) & 0x80) != 0));
        A = result;
        SetNz(result);
    }

    private void Sbc(byte m)
    {
        ushort a = A;
        ushort mm = (byte)(~m);
        ushort cc = Carry() ? (ushort)1 : (ushort)0;
        ushort sum = (ushort)(a + mm + cc);
        SetCarry(sum > 0xFF);
        byte result = (byte)(sum & 0xFF);
        SetOverflow((((a ^ mm) & 0x80) == 0) && (((a ^ sum) & 0x80) != 0));
        A = result;
        SetNz(result);
    }

    // ---- compare ----
    private void CmpOp(byte r, byte m)
    {
        byte diff = (byte)(r - m);
        SetCarry(r >= m);
        SetNz(diff);
    }

    // ---- shifts/rotates/inc/dec value transforms ----
    private byte AslValue(byte v) { SetCarry((v & 0x80) != 0); return (byte)(v << 1); }
    private byte LsrValue(byte v) { SetCarry((v & 0x01) != 0); return (byte)(v >> 1); }
    private byte RolValue(byte v)
    {
        bool newC = (v & 0x80) != 0;
        byte result = (byte)((v << 1) | (Carry() ? 1 : 0));
        SetCarry(newC);
        return result;
    }
    private byte RorValue(byte v)
    {
        bool newC = (v & 0x01) != 0;
        byte result = (byte)((v >> 1) | (Carry() ? 0x80 : 0x00));
        SetCarry(newC);
        return result;
    }
    private byte IncValue(byte v) => (byte)(v + 1);
    private byte DecValue(byte v) => (byte)(v - 1);

    // ---- branches ----
    private byte Branch(bool branchOnSet, byte condFlag)
    {
        bool flagSet = (Status & condFlag) != 0;
        bool take = (flagSet == branchOnSet);
        if (!take)
        {
            FetchByte();
            return 2;
        }
        ushort pcBefore = Pc;
        ushort target = AmRelative();
        ushort pcAfter = (ushort)(pcBefore + 1);
        bool pageCross = (pcAfter & 0xFF00) != (target & 0xFF00);
        Pc = target;
        return (byte)(2 + 1 + (pageCross ? 1 : 0));
    }
}
