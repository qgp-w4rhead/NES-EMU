// Addressing-mode resolvers and read/write helpers (port of cores/c/src/cpu/addressing.c).

import { Cpu, AddrMode, Dummy, Operand, makeNone, makeAcc, makeAddr } from './cpu';

export interface PageCrossResult { addr: number; pageCross: boolean; }

declare module './cpu' {
    interface Cpu {
        resolve(mode: number): Operand;
        amImmediate(): number;
        amZeroPage(): number;
        amZeroPageX(dummy: number): number;
        amZeroPageY(dummy: number): number;
        amAbsolute(): number;
        amAbsoluteX(dummy: number): PageCrossResult;
        amAbsoluteY(dummy: number): PageCrossResult;
        amIndirect(): number;
        amIndirectX(): number;
        amIndirectY(dummy: number): PageCrossResult;
        amRelative(): number;
        amIndirectYBase(): number;
        rdImm(): number;
        rdZp(): number;
        rdZpX(): number;
        rdZpY(): number;
        rdAbs(): number;
        rdAbsX(): PageCrossResult;
        rdAbsY(): PageCrossResult;
        rdIndX(): number;
        rdIndY(): PageCrossResult;
        wrZp(v: number): void;
        wrZpX(v: number): void;
        wrZpY(v: number): void;
        wrAbs(v: number): void;
        wrAbsX(v: number): void;
        wrAbsY(v: number): void;
        wrIndX(v: number): void;
        wrIndY(v: number): void;
        opZp(): Operand;
        opZpX(): Operand;
        opAbs(): Operand;
        opAbsX(): Operand;
        opAbsY(): Operand;
        opIndX(): Operand;
        opIndY(): Operand;
        rmw(op: Operand, f: (v: number) => number): void;
        rmwZp(f: (v: number) => number): void;
        rmwZpX(f: (v: number) => number): void;
        rmwAbs(f: (v: number) => number): void;
        rmwAbsX(f: (v: number) => number): void;
    }
}

Cpu.prototype.resolve = function (mode: number): Operand {
    switch (mode) {
        case AddrMode.Implied: return makeNone();
        case AddrMode.Accumulator: return makeAcc();
        case AddrMode.Immediate: return makeAddr(this.amImmediate());
        case AddrMode.ZeroPage: return makeAddr(this.amZeroPage());
        case AddrMode.ZeroPageX: return makeAddr(this.amZeroPageX(Dummy.None));
        case AddrMode.ZeroPageY: return makeAddr(this.amZeroPageY(Dummy.None));
        case AddrMode.Absolute: return makeAddr(this.amAbsolute());
        case AddrMode.AbsoluteX: return makeAddr(this.amAbsoluteX(Dummy.None).addr);
        case AddrMode.AbsoluteY: return makeAddr(this.amAbsoluteY(Dummy.None).addr);
        case AddrMode.Indirect: return makeAddr(this.amIndirect());
        case AddrMode.IndirectX: return makeAddr(this.amIndirectX());
        case AddrMode.IndirectY: return makeAddr(this.amIndirectY(Dummy.None).addr);
        case AddrMode.Relative: return makeAddr(this.amRelative());
        default: return makeNone();
    }
};

Cpu.prototype.amImmediate = function (): number {
    const addr = this.pc;
    this.pc = (this.pc + 1) & 0xFFFF;
    return addr;
};

Cpu.prototype.amZeroPage = function (): number { return this.fetchByte(); };

Cpu.prototype.amZeroPageX = function (dummy: number): number {
    const b = this.fetchByte();
    if (dummy !== Dummy.None) this.bus.read(b);
    return (b + this.x) & 0xFF;
};

Cpu.prototype.amZeroPageY = function (dummy: number): number {
    const b = this.fetchByte();
    if (dummy !== Dummy.None) this.bus.read(b);
    return (b + this.y) & 0xFF;
};

Cpu.prototype.amAbsolute = function (): number { return this.fetchWord(); };

Cpu.prototype.amAbsoluteX = function (dummy: number): PageCrossResult {
    const b = this.fetchWord();
    const eff = (b + this.x) & 0xFFFF;
    const cross = (b & 0xFF00) !== (eff & 0xFF00);
    if ((dummy === Dummy.Read && cross) || dummy === Dummy.Rmw)
        this.bus.read((b & 0xFF00) | (eff & 0x00FF));
    return { addr: eff, pageCross: cross };
};

Cpu.prototype.amAbsoluteY = function (dummy: number): PageCrossResult {
    const b = this.fetchWord();
    const eff = (b + this.y) & 0xFFFF;
    const cross = (b & 0xFF00) !== (eff & 0xFF00);
    if ((dummy === Dummy.Read && cross) || dummy === Dummy.Rmw)
        this.bus.read((b & 0xFF00) | (eff & 0x00FF));
    return { addr: eff, pageCross: cross };
};

Cpu.prototype.amIndirect = function (): number {
    const ptr = this.fetchWord();
    const lo = this.bus.read(ptr);
    const hiAddr = (ptr & 0xFF00) | ((ptr + 1) & 0xFF);
    const hi = this.bus.read(hiAddr);
    return (lo | (hi << 8)) & 0xFFFF;
};

Cpu.prototype.amIndirectX = function (): number {
    const zp = this.fetchByte();
    const ptr = (zp + this.x) & 0xFF;
    const lo = this.bus.read(ptr);
    const hi = this.bus.read((ptr + 1) & 0xFF);
    return (lo | (hi << 8)) & 0xFFFF;
};

Cpu.prototype.amIndirectY = function (dummy: number): PageCrossResult {
    const zp = this.fetchByte();
    const lo = this.bus.read(zp);
    const hi = this.bus.read((zp + 1) & 0xFF);
    const b = (lo | (hi << 8)) & 0xFFFF;
    const eff = (b + this.y) & 0xFFFF;
    const cross = (b & 0xFF00) !== (eff & 0xFF00);
    if ((dummy === Dummy.Read && cross) || dummy === Dummy.Rmw)
        this.bus.read((b & 0xFF00) | (eff & 0x00FF));
    return { addr: eff, pageCross: cross };
};

Cpu.prototype.amRelative = function (): number {
    const offset = this.fetchByte();
    const signed = (offset << 24) >> 24; // sign-extend
    return (this.pc + signed) & 0xFFFF;
};

Cpu.prototype.amIndirectYBase = function (): number {
    const zp = this.fetchByte();
    const lo = this.bus.read(zp);
    const hi = this.bus.read((zp + 1) & 0xFF);
    return (lo | (hi << 8)) & 0xFFFF;
};

// ---- read/write helpers ----
Cpu.prototype.rdImm = function (): number { return this.fetchByte(); };
Cpu.prototype.rdZp = function (): number { return this.readOperand(this.resolve(AddrMode.ZeroPage)); };
Cpu.prototype.rdZpX = function (): number { return this.bus.read(this.amZeroPageX(Dummy.Rmw)); };
Cpu.prototype.rdZpY = function (): number { return this.bus.read(this.amZeroPageY(Dummy.Rmw)); };
Cpu.prototype.rdAbs = function (): number { return this.readOperand(this.resolve(AddrMode.Absolute)); };
Cpu.prototype.rdAbsX = function (): PageCrossResult {
    const r = this.amAbsoluteX(Dummy.Read);
    return { addr: this.bus.read(r.addr), pageCross: r.pageCross };
};
Cpu.prototype.rdAbsY = function (): PageCrossResult {
    const r = this.amAbsoluteY(Dummy.Read);
    return { addr: this.bus.read(r.addr), pageCross: r.pageCross };
};
Cpu.prototype.rdIndX = function (): number { return this.bus.read(this.amIndirectX()); };
Cpu.prototype.rdIndY = function (): PageCrossResult {
    const r = this.amIndirectY(Dummy.Read);
    return { addr: this.bus.read(r.addr), pageCross: r.pageCross };
};

Cpu.prototype.wrZp = function (v: number): void { this.writeOperand(this.resolve(AddrMode.ZeroPage), v); };
Cpu.prototype.wrZpX = function (v: number): void { this.bus.write(this.amZeroPageX(Dummy.Rmw), v); };
Cpu.prototype.wrZpY = function (v: number): void { this.bus.write(this.amZeroPageY(Dummy.Rmw), v); };
Cpu.prototype.wrAbs = function (v: number): void { this.writeOperand(this.resolve(AddrMode.Absolute), v); };
Cpu.prototype.wrAbsX = function (v: number): void { this.bus.write(this.amAbsoluteX(Dummy.Rmw).addr, v); };
Cpu.prototype.wrAbsY = function (v: number): void { this.bus.write(this.amAbsoluteY(Dummy.Rmw).addr, v); };
Cpu.prototype.wrIndX = function (v: number): void { this.bus.write(this.amIndirectX(), v); };
Cpu.prototype.wrIndY = function (v: number): void { this.bus.write(this.amIndirectY(Dummy.Rmw).addr, v); };

Cpu.prototype.opZp = function (): Operand { return this.resolve(AddrMode.ZeroPage); };
Cpu.prototype.opZpX = function (): Operand { return makeAddr(this.amZeroPageX(Dummy.Rmw)); };
Cpu.prototype.opAbs = function (): Operand { return this.resolve(AddrMode.Absolute); };
Cpu.prototype.opAbsX = function (): Operand { return makeAddr(this.amAbsoluteX(Dummy.Rmw).addr); };
Cpu.prototype.opAbsY = function (): Operand { return makeAddr(this.amAbsoluteY(Dummy.Rmw).addr); };
Cpu.prototype.opIndX = function (): Operand { return makeAddr(this.amIndirectX()); };
Cpu.prototype.opIndY = function (): Operand { return makeAddr(this.amIndirectY(Dummy.Rmw).addr); };

// RMW helper
Cpu.prototype.rmw = function (op: Operand, f: (v: number) => number): void {
    const v = this.readOperand(op);
    const neu = f(v) & 0xFF;
    this.writeOperand(op, neu);
    this.setNz(neu);
};

Cpu.prototype.rmwZp = function (f: (v: number) => number): void { this.rmw(this.resolve(AddrMode.ZeroPage), f); };
Cpu.prototype.rmwZpX = function (f: (v: number) => number): void { this.rmw(makeAddr(this.amZeroPageX(Dummy.Rmw)), f); };
Cpu.prototype.rmwAbs = function (f: (v: number) => number): void { this.rmw(this.resolve(AddrMode.Absolute), f); };
Cpu.prototype.rmwAbsX = function (f: (v: number) => number): void { this.rmw(makeAddr(this.amAbsoluteX(Dummy.Rmw).addr), f); };
