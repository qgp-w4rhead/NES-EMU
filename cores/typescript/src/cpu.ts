// 6502 CPU core (port of cores/c/src/cpu.c).

import { Bus } from './bus';

export enum AddrMode {
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

export enum Dummy {
    None,
    Read,
    Rmw,
}

export interface Operand {
    tag: number; // 0=None, 1=Acc, 2=Addr
    address: number;
}

export function makeNone(): Operand { return { tag: 0, address: 0 }; }
export function makeAcc(): Operand { return { tag: 1, address: 0 }; }
export function makeAddr(a: number): Operand { return { tag: 2, address: a }; }

export class Cpu {
    static readonly FlagC = 0x01;
    static readonly FlagZ = 0x02;
    static readonly FlagI = 0x04;
    static readonly FlagD = 0x08;
    static readonly FlagB = 0x10;
    static readonly FlagU = 0x20;
    static readonly FlagV = 0x40;
    static readonly FlagN = 0x80;

    static readonly VectorNmi = 0xFFFA;
    static readonly VectorReset = 0xFFFC;
    static readonly VectorIrq = 0xFFFE;

    static readonly NmiPending = 0x01;
    static readonly IrqPending = 0x02;
    static readonly Halted = 0x04;

    a: number = 0;
    x: number = 0;
    y: number = 0;
    sp: number = 0;
    pc: number = 0;
    status: number = 0;
    flags: number = 0;

    bus!: Bus;

    init(): void {
        this.a = 0; this.x = 0; this.y = 0;
        this.sp = 0xFD;
        this.pc = 0;
        this.status = Cpu.FlagU | Cpu.FlagI;
        this.flags = 0;
    }

    carry(): boolean { return (this.status & Cpu.FlagC) !== 0; }
    zero(): boolean { return (this.status & Cpu.FlagZ) !== 0; }
    interruptDisable(): boolean { return (this.status & Cpu.FlagI) !== 0; }
    decimal(): boolean { return (this.status & Cpu.FlagD) !== 0; }
    overflow(): boolean { return (this.status & Cpu.FlagV) !== 0; }
    negative(): boolean { return (this.status & Cpu.FlagN) !== 0; }

    setFlag(flag: number, v: boolean): void {
        if (v) this.status |= flag;
        else this.status &= (flag ^ 0xFF);
    }

    setCarry(v: boolean): void { this.setFlag(Cpu.FlagC, v); }
    setZero(v: boolean): void { this.setFlag(Cpu.FlagZ, v); }
    setInterruptDisable(v: boolean): void { this.setFlag(Cpu.FlagI, v); }
    setDecimal(v: boolean): void { this.setFlag(Cpu.FlagD, v); }
    setOverflow(v: boolean): void { this.setFlag(Cpu.FlagV, v); }
    setNegative(v: boolean): void { this.setFlag(Cpu.FlagN, v); }

    setNz(value: number): void {
        this.setZero(value === 0);
        this.setNegative((value & 0x80) !== 0);
    }

    fetchByte(): number {
        const b = this.bus.read(this.pc);
        this.pc = (this.pc + 1) & 0xFFFF;
        return b;
    }

    fetchWord(): number {
        const lo = this.fetchByte();
        const hi = this.fetchByte();
        return (lo | (hi << 8)) & 0xFFFF;
    }

    push(value: number): void {
        this.bus.write((0x0100 | this.sp) & 0xFFFF, value);
        this.sp = (this.sp - 1) & 0xFF;
    }

    pull(): number {
        this.sp = (this.sp + 1) & 0xFF;
        return this.bus.read((0x0100 | this.sp) & 0xFFFF);
    }

    pushPc(pc: number): void {
        this.push((pc >> 8) & 0xFF);
        this.push(pc & 0xFF);
    }

    pullPc(): number {
        const lo = this.pull();
        const hi = this.pull();
        return (lo | (hi << 8)) & 0xFFFF;
    }

    pushStatus(withBreak: boolean): void {
        let p = this.status | Cpu.FlagU;
        if (withBreak) p |= Cpu.FlagB;
        else p &= (Cpu.FlagB ^ 0xFF);
        this.push(p);
    }

    pullStatus(): void {
        const p = this.pull();
        this.status = (p & (Cpu.FlagB ^ 0xFF)) | Cpu.FlagU;
    }

    readOperand(op: Operand): number {
        switch (op.tag) {
            case 1: return this.a;
            case 2: return this.bus.read(op.address);
            default: return 0;
        }
    }

    writeOperand(op: Operand, value: number): void {
        switch (op.tag) {
            case 1: this.a = value; return;
            case 2: this.bus.write(op.address, value); return;
        }
    }

    private serviceInterrupt(vector: number): void {
        this.pushPc(this.pc);
        this.pushStatus(false);
        this.setInterruptDisable(true);
        this.pc = this.readVector(vector);
    }

    nmi(): void { this.serviceInterrupt(Cpu.VectorNmi); }
    irq(): void { this.serviceInterrupt(Cpu.VectorIrq); }

    reset(): void {
        this.sp = 0xFD;
        this.setInterruptDisable(true);
        this.status |= Cpu.FlagU;
        this.pc = this.readVector(Cpu.VectorReset);
        this.flags &= (Cpu.Halted ^ 0xFF);
    }

    step(): number {
        if ((this.flags & Cpu.Halted) !== 0) return 1;
        if ((this.flags & Cpu.NmiPending) !== 0) {
            this.flags &= (Cpu.NmiPending ^ 0xFF);
            this.nmi();
            return 7;
        }
        if ((this.flags & Cpu.IrqPending) !== 0 && !this.interruptDisable()) {
            this.flags &= (Cpu.IrqPending ^ 0xFF);
            this.irq();
            return 7;
        }
        const opcode = this.fetchByte();
        return this.execute(opcode);
    }

    isHalted(): boolean { return (this.flags & Cpu.Halted) !== 0; }
    setHalted(v: boolean): void {
        if (v) this.flags |= Cpu.Halted;
        else this.flags &= (Cpu.Halted ^ 0xFF);
    }
    nmiPendingFlag(): boolean { return (this.flags & Cpu.NmiPending) !== 0; }
    irqPendingFlag(): boolean { return (this.flags & Cpu.IrqPending) !== 0; }
    setNmiPending(v: boolean): void {
        if (v) this.flags |= Cpu.NmiPending;
        else this.flags &= (Cpu.NmiPending ^ 0xFF);
    }
    setIrqPending(v: boolean): void {
        if (v) this.flags |= Cpu.IrqPending;
        else this.flags &= (Cpu.IrqPending ^ 0xFF);
    }

    readVector(addr: number): number {
        const lo = this.bus.read(addr);
        const hi = this.bus.read((addr + 1) & 0xFFFF);
        return (lo | (hi << 8)) & 0xFFFF;
    }
}
