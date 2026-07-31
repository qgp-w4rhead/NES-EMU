using System.Runtime.CompilerServices;

namespace NesCore;

// 6502 addressing modes (addressing.rs AddrMode).
public enum AddrMode : byte
{
    Implied,
    Accumulator,
    Immediate,
    ZeroPage,
    ZeroPageX,
    ZeroPageY,
    Absolute,
    AbsoluteX,
    AbsoluteY,
    Indirect,
    IndirectX,
    IndirectY,
    Relative,
}

// Dummy-read behaviour (addressing.rs Dummy).
public enum Dummy : byte
{
    None,
    Read,
    Rmw,
}

// Operand tag (addressing.rs Operand).
public struct Operand
{
    public const int None = 0;
    public const int Acc = 1;
    public const int Addr = 2;

    public int Tag;
    public ushort Address;

    public static Operand MakeNone() => new Operand { Tag = None };
    public static Operand MakeAcc() => new Operand { Tag = Acc };
    public static Operand MakeAddr(ushort a) => new Operand { Tag = Addr, Address = a };
}

// 6502 CPU core (mod.rs struct Cpu).
public sealed partial class Cpu
{
    public const byte FlagC = 0x01;
    public const byte FlagZ = 0x02;
    public const byte FlagI = 0x04;
    public const byte FlagD = 0x08;
    public const byte FlagB = 0x10;
    public const byte FlagU = 0x20;
    public const byte FlagV = 0x40;
    public const byte FlagN = 0x80;

    public const ushort VectorNmi = 0xFFFA;
    public const ushort VectorReset = 0xFFFC;
    public const ushort VectorIrq = 0xFFFE;

    public const byte NmiPending = 0x01;
    public const byte IrqPending = 0x02;
    public const byte Halted = 0x04;

    public byte A;
    public byte X;
    public byte Y;
    public byte Sp;
    public ushort Pc;
    public byte Status;
    public byte Flags;

    public Bus Bus;

    public Cpu() { }

    public void Init()
    {
        A = 0; X = 0; Y = 0;
        Sp = 0xFD;
        Pc = 0;
        Status = FlagU | FlagI;
        Flags = 0;
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public bool Carry() => (Status & FlagC) != 0;
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public bool Zero() => (Status & FlagZ) != 0;
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public bool InterruptDisable() => (Status & FlagI) != 0;
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public bool Decimal() => (Status & FlagD) != 0;
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public bool Overflow() => (Status & FlagV) != 0;
    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public bool Negative() => (Status & FlagN) != 0;

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public void SetFlag(byte flag, bool v)
    {
        if (v) Status |= flag;
        else Status &= (byte)(flag ^ 0xFF);
    }

    public void SetCarry(bool v) => SetFlag(FlagC, v);
    public void SetZero(bool v) => SetFlag(FlagZ, v);
    public void SetInterruptDisable(bool v) => SetFlag(FlagI, v);
    public void SetDecimal(bool v) => SetFlag(FlagD, v);
    public void SetOverflow(bool v) => SetFlag(FlagV, v);
    public void SetNegative(bool v) => SetFlag(FlagN, v);

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public void SetNz(byte value)
    {
        SetZero(value == 0);
        SetNegative((value & 0x80) != 0);
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public byte FetchByte()
    {
        byte b = Bus.Read(Pc);
        Pc++;
        return b;
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public ushort FetchWord()
    {
        ushort lo = FetchByte();
        ushort hi = FetchByte();
        return (ushort)(lo | (hi << 8));
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public void Push(byte value)
    {
        Bus.Write((ushort)(0x0100 | Sp), value);
        Sp--;
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public byte Pull()
    {
        Sp++;
        return Bus.Read((ushort)(0x0100 | Sp));
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public void PushPc(ushort pc)
    {
        Push((byte)(pc >> 8));
        Push((byte)(pc & 0xFF));
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public ushort PullPc()
    {
        ushort lo = Pull();
        ushort hi = Pull();
        return (ushort)(lo | (hi << 8));
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public void PushStatus(bool withBreak)
    {
        byte p = (byte)(Status | FlagU);
        if (withBreak) p |= FlagB;
        else p &= (byte)(FlagB ^ 0xFF);
        Push(p);
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public void PullStatus()
    {
        byte p = Pull();
        Status = (byte)((p & (byte)(FlagB ^ 0xFF)) | FlagU);
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public byte ReadOperand(Operand op)
    {
        switch (op.Tag)
        {
            case Operand.Acc: return A;
            case Operand.Addr: return Bus.Read(op.Address);
            default: return 0;
        }
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public void WriteOperand(Operand op, byte value)
    {
        switch (op.Tag)
        {
            case Operand.Acc: A = value; return;
            case Operand.Addr: Bus.Write(op.Address, value); return;
        }
    }

    private void ServiceInterrupt(ushort vector)
    {
        PushPc(Pc);
        PushStatus(false);
        SetInterruptDisable(true);
        Pc = ReadVector(vector);
    }

    public void Nmi() => ServiceInterrupt(VectorNmi);
    public void Irq() => ServiceInterrupt(VectorIrq);

    public void Reset()
    {
        Sp = 0xFD;
        SetInterruptDisable(true);
        Status |= FlagU;
        Pc = ReadVector(VectorReset);
        Flags &= (byte)(Halted ^ 0xFF);
    }

    public byte Step()
    {
        if ((Flags & Halted) != 0) return 1;
        if ((Flags & NmiPending) != 0)
        {
            Flags &= (byte)(NmiPending ^ 0xFF);
            Nmi();
            return 7;
        }
        if ((Flags & IrqPending) != 0 && !InterruptDisable())
        {
            Flags &= (byte)(IrqPending ^ 0xFF);
            Irq();
            return 7;
        }
        byte opcode = FetchByte();
        return Execute(opcode);
    }

    public bool IsHalted() => (Flags & Halted) != 0;
    public void SetHalted(bool v)
    {
        if (v) Flags |= Halted;
        else Flags &= (byte)(Halted ^ 0xFF);
    }
    public bool NmiPendingFlag() => (Flags & NmiPending) != 0;
    public bool IrqPendingFlag() => (Flags & IrqPending) != 0;
    public void SetNmiPending(bool v)
    {
        if (v) Flags |= NmiPending;
        else Flags &= (byte)(NmiPending ^ 0xFF);
    }
    public void SetIrqPending(bool v)
    {
        if (v) Flags |= IrqPending;
        else Flags &= (byte)(IrqPending ^ 0xFF);
    }

    [MethodImpl(MethodImplOptions.AggressiveInlining)]
    public ushort ReadVector(ushort addr)
    {
        ushort lo = Bus.Read(addr);
        ushort hi = Bus.Read((ushort)(addr + 1));
        return (ushort)(lo | (hi << 8));
    }
}
