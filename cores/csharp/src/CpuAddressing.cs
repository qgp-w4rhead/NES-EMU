using System.Runtime.CompilerServices;

namespace NesCore;

// Addressing-mode resolvers and read/write helpers (addressing.rs + opcodes.rs).
public sealed partial class Cpu
{
    public Operand Resolve(AddrMode mode)
    {
        switch (mode)
        {
            case AddrMode.Implied: return Operand.MakeNone();
            case AddrMode.Accumulator: return Operand.MakeAcc();
            case AddrMode.Immediate: return Operand.MakeAddr(AmImmediate());
            case AddrMode.ZeroPage: return Operand.MakeAddr(AmZeroPage());
            case AddrMode.ZeroPageX: return Operand.MakeAddr(AmZeroPageX(Dummy.None));
            case AddrMode.ZeroPageY: return Operand.MakeAddr(AmZeroPageY(Dummy.None));
            case AddrMode.Absolute: return Operand.MakeAddr(AmAbsolute());
            case AddrMode.AbsoluteX: return Operand.MakeAddr(AmAbsoluteX(Dummy.None, out _));
            case AddrMode.AbsoluteY: return Operand.MakeAddr(AmAbsoluteY(Dummy.None, out _));
            case AddrMode.Indirect: return Operand.MakeAddr(AmIndirect());
            case AddrMode.IndirectX: return Operand.MakeAddr(AmIndirectX());
            case AddrMode.IndirectY: return Operand.MakeAddr(AmIndirectY(Dummy.None, out _));
            case AddrMode.Relative: return Operand.MakeAddr(AmRelative());
            default: return Operand.MakeNone();
        }
    }

    public ushort AmImmediate()
    {
        ushort addr = Pc;
        Pc++;
        return addr;
    }

    public ushort AmZeroPage() => FetchByte();

    public ushort AmZeroPageX(Dummy dummy)
    {
        byte b = FetchByte();
        if (dummy != Dummy.None) Bus.Read(b);
        return (byte)(b + X);
    }

    public ushort AmZeroPageY(Dummy dummy)
    {
        byte b = FetchByte();
        if (dummy != Dummy.None) Bus.Read(b);
        return (byte)(b + Y);
    }

    public ushort AmAbsolute() => FetchWord();

    public ushort AmAbsoluteX(Dummy dummy, out bool pageCross)
    {
        ushort b = FetchWord();
        ushort eff = (ushort)(b + X);
        bool cross = (b & 0xFF00) != (eff & 0xFF00);
        if ((dummy == Dummy.Read && cross) || dummy == Dummy.Rmw)
            Bus.Read((ushort)((b & 0xFF00) | (eff & 0x00FF)));
        pageCross = cross;
        return eff;
    }

    public ushort AmAbsoluteY(Dummy dummy, out bool pageCross)
    {
        ushort b = FetchWord();
        ushort eff = (ushort)(b + Y);
        bool cross = (b & 0xFF00) != (eff & 0xFF00);
        if ((dummy == Dummy.Read && cross) || dummy == Dummy.Rmw)
            Bus.Read((ushort)((b & 0xFF00) | (eff & 0x00FF)));
        pageCross = cross;
        return eff;
    }

    public ushort AmIndirect()
    {
        ushort ptr = FetchWord();
        byte lo = Bus.Read(ptr);
        ushort hiAddr = (ushort)((ptr & 0xFF00) | (byte)(ptr + 1));
        byte hi = Bus.Read(hiAddr);
        return (ushort)(lo | (hi << 8));
    }

    public ushort AmIndirectX()
    {
        byte zp = FetchByte();
        byte ptr = (byte)(zp + X);
        byte lo = Bus.Read(ptr);
        byte hi = Bus.Read((byte)(ptr + 1));
        return (ushort)(lo | (hi << 8));
    }

    public ushort AmIndirectY(Dummy dummy, out bool pageCross)
    {
        byte zp = FetchByte();
        byte lo = Bus.Read(zp);
        byte hi = Bus.Read((byte)(zp + 1));
        ushort b = (ushort)(lo | (hi << 8));
        ushort eff = (ushort)(b + Y);
        bool cross = (b & 0xFF00) != (eff & 0xFF00);
        if ((dummy == Dummy.Read && cross) || dummy == Dummy.Rmw)
            Bus.Read((ushort)((b & 0xFF00) | (eff & 0x00FF)));
        pageCross = cross;
        return eff;
    }

    public ushort AmRelative()
    {
        sbyte offset = (sbyte)FetchByte();
        return (ushort)(Pc + (int)offset);
    }

    public ushort AmIndirectYBase()
    {
        byte zp = FetchByte();
        byte lo = Bus.Read(zp);
        byte hi = Bus.Read((byte)(zp + 1));
        return (ushort)(lo | (hi << 8));
    }

    // ---- read/write helpers (opcodes.rs rd_*/wr_*) ----

    public delegate byte ValueFn(byte v);

    public byte RdImm() => FetchByte();
    public byte RdZp() => ReadOperand(Resolve(AddrMode.ZeroPage));
    public byte RdZpX() => Bus.Read(AmZeroPageX(Dummy.Rmw));
    public byte RdZpY() => Bus.Read(AmZeroPageY(Dummy.Rmw));
    public byte RdAbs() => ReadOperand(Resolve(AddrMode.Absolute));
    public byte RdAbsX(out bool pc) => Bus.Read(AmAbsoluteX(Dummy.Read, out pc));
    public byte RdAbsY(out bool pc) => Bus.Read(AmAbsoluteY(Dummy.Read, out pc));
    public byte RdIndX() => Bus.Read(AmIndirectX());
    public byte RdIndY(out bool pc) => Bus.Read(AmIndirectY(Dummy.Read, out pc));

    public void WrZp(byte v) => WriteOperand(Resolve(AddrMode.ZeroPage), v);
    public void WrZpX(byte v) => Bus.Write(AmZeroPageX(Dummy.Rmw), v);
    public void WrZpY(byte v) => Bus.Write(AmZeroPageY(Dummy.Rmw), v);
    public void WrAbs(byte v) => WriteOperand(Resolve(AddrMode.Absolute), v);
    public void WrAbsX(byte v) => Bus.Write(AmAbsoluteX(Dummy.Rmw, out _), v);
    public void WrAbsY(byte v) => Bus.Write(AmAbsoluteY(Dummy.Rmw, out _), v);
    public void WrIndX(byte v) => Bus.Write(AmIndirectX(), v);
    public void WrIndY(byte v) => Bus.Write(AmIndirectY(Dummy.Rmw, out _), v);

    public Operand OpZp() => Resolve(AddrMode.ZeroPage);
    public Operand OpZpX() => Operand.MakeAddr(AmZeroPageX(Dummy.Rmw));
    public Operand OpAbs() => Resolve(AddrMode.Absolute);
    public Operand OpAbsX() => Operand.MakeAddr(AmAbsoluteX(Dummy.Rmw, out _));
    public Operand OpAbsY() => Operand.MakeAddr(AmAbsoluteY(Dummy.Rmw, out _));
    public Operand OpIndX() => Operand.MakeAddr(AmIndirectX());
    public Operand OpIndY() => Operand.MakeAddr(AmIndirectY(Dummy.Rmw, out _));

    private void Rmw(Operand op, ValueFn f)
    {
        byte v = ReadOperand(op);
        byte neu = f(v);
        WriteOperand(op, neu);
        SetNz(neu);
    }

    private void RmwZp(ValueFn f) => Rmw(Resolve(AddrMode.ZeroPage), f);
    private void RmwZpX(ValueFn f) => Rmw(Operand.MakeAddr(AmZeroPageX(Dummy.Rmw)), f);
    private void RmwAbs(ValueFn f) => Rmw(Resolve(AddrMode.Absolute), f);
    private void RmwAbsX(ValueFn f) => Rmw(Operand.MakeAddr(AmAbsoluteX(Dummy.Rmw, out _)), f);
}
