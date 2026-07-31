// Official opcode handlers (port of cores/c/src/cpu/opcodes.c).

import { Cpu, Operand } from './cpu';

const XaaMagic = 0x00;

declare module './cpu' {
    interface Cpu {
        lda(v: number): void;
        ldx(v: number): void;
        ldy(v: number): void;
        tax(): void;
        tay(): void;
        txa(): void;
        tya(): void;
        tsx(): void;
        setSpFromX(): void;
        andOp(v: number): void;
        oraOp(v: number): void;
        eorOp(v: number): void;
        bitOp(m: number): void;
        adc(m: number): void;
        sbc(m: number): void;
        cmpOp(r: number, m: number): void;
        aslValue(v: number): number;
        lsrValue(v: number): number;
        rolValue(v: number): number;
        rorValue(v: number): number;
        incValue(v: number): number;
        decValue(v: number): number;
        branch(branchOnSet: boolean, condFlag: number): number;
        lax(m: number): void;
        anc(m: number): void;
        alr(m: number): void;
        arr(m: number): void;
        axs(m: number): void;
        xaa(m: number): void;
        tasStore(b: number): void;
        ahxStore(b: number, reg: number): void;
        shyStore(b: number): void;
        rmwCombo(op: Operand, f: (v: number) => number): void;
        dcp(op: Operand): void;
        isc(op: Operand): void;
        slo(op: Operand): void;
        rla(op: Operand): void;
        sre(op: Operand): void;
        rra(op: Operand): void;
    }
}

// ---- load/store/transfer ----
Cpu.prototype.lda = function (v: number): void { this.a = v & 0xFF; this.setNz(this.a); };
Cpu.prototype.ldx = function (v: number): void { this.x = v & 0xFF; this.setNz(this.x); };
Cpu.prototype.ldy = function (v: number): void { this.y = v & 0xFF; this.setNz(this.y); };
Cpu.prototype.tax = function (): void { this.x = this.a; this.setNz(this.x); };
Cpu.prototype.tay = function (): void { this.y = this.a; this.setNz(this.y); };
Cpu.prototype.txa = function (): void { this.a = this.x; this.setNz(this.a); };
Cpu.prototype.tya = function (): void { this.a = this.y; this.setNz(this.a); };
Cpu.prototype.tsx = function (): void { this.x = this.sp; this.setNz(this.x); };
Cpu.prototype.setSpFromX = function (): void { this.sp = this.x; };

// ---- logic ----
Cpu.prototype.andOp = function (v: number): void { this.a = (this.a & v) & 0xFF; this.setNz(this.a); };
Cpu.prototype.oraOp = function (v: number): void { this.a = (this.a | v) & 0xFF; this.setNz(this.a); };
Cpu.prototype.eorOp = function (v: number): void { this.a = (this.a ^ v) & 0xFF; this.setNz(this.a); };

Cpu.prototype.bitOp = function (m: number): void {
    const result = (this.a & m) & 0xFF;
    this.setZero(result === 0);
    this.setNegative((m & 0x80) !== 0);
    this.setOverflow((m & 0x40) !== 0);
};

// ---- arithmetic ----
Cpu.prototype.adc = function (m: number): void {
    const a = this.a;
    const mm = m & 0xFF;
    const cc = this.carry() ? 1 : 0;
    const sum = (a + mm + cc) | 0;
    this.setCarry(sum > 0xFF);
    const result = sum & 0xFF;
    this.setOverflow((((a ^ mm) & 0x80) === 0) && (((a ^ sum) & 0x80) !== 0));
    this.a = result;
    this.setNz(result);
};

Cpu.prototype.sbc = function (m: number): void {
    const a = this.a;
    const mm = (~m) & 0xFF;
    const cc = this.carry() ? 1 : 0;
    const sum = (a + mm + cc) | 0;
    this.setCarry(sum > 0xFF);
    const result = sum & 0xFF;
    this.setOverflow((((a ^ mm) & 0x80) === 0) && (((a ^ sum) & 0x80) !== 0));
    this.a = result;
    this.setNz(result);
};

// ---- compare ----
Cpu.prototype.cmpOp = function (r: number, m: number): void {
    const diff = (r - m) & 0xFF;
    this.setCarry(r >= m);
    this.setNz(diff);
};

// ---- shifts/rotates/inc/dec value transforms ----
Cpu.prototype.aslValue = function (v: number): number { this.setCarry((v & 0x80) !== 0); return (v << 1) & 0xFF; };
Cpu.prototype.lsrValue = function (v: number): number { this.setCarry((v & 0x01) !== 0); return (v >> 1) & 0xFF; };
Cpu.prototype.rolValue = function (v: number): number {
    const newC = (v & 0x80) !== 0;
    const result = ((v << 1) | (this.carry() ? 1 : 0)) & 0xFF;
    this.setCarry(newC);
    return result;
};
Cpu.prototype.rorValue = function (v: number): number {
    const newC = (v & 0x01) !== 0;
    const result = ((v >> 1) | (this.carry() ? 0x80 : 0x00)) & 0xFF;
    this.setCarry(newC);
    return result;
};
Cpu.prototype.incValue = function (v: number): number { return (v + 1) & 0xFF; };
Cpu.prototype.decValue = function (v: number): number { return (v - 1) & 0xFF; };

// ---- branches ----
Cpu.prototype.branch = function (branchOnSet: boolean, condFlag: number): number {
    const flagSet = (this.status & condFlag) !== 0;
    const take = (flagSet === branchOnSet);
    if (!take) {
        this.fetchByte();
        return 2;
    }
    const pcBefore = this.pc;
    const target = this.amRelative();
    const pcAfter = (pcBefore + 1) & 0xFFFF;
    const pageCross = (pcAfter & 0xFF00) !== (target & 0xFF00);
    this.pc = target;
    return 2 + 1 + (pageCross ? 1 : 0);
};

// ---- unofficial immediate combined ops ----
Cpu.prototype.lax = function (m: number): void { this.a = m & 0xFF; this.x = m & 0xFF; this.setNz(m & 0xFF); };
Cpu.prototype.anc = function (m: number): void { this.a = (this.a & m) & 0xFF; this.setNz(this.a); this.setCarry((this.a & 0x80) !== 0); };
Cpu.prototype.alr = function (m: number): void {
    const v = (this.a & m) & 0xFF;
    this.setCarry((v & 0x01) !== 0);
    const result = (v >> 1) & 0xFF;
    this.a = result;
    this.setNz(result);
};
Cpu.prototype.arr = function (m: number): void {
    const v = (this.a & m) & 0xFF;
    const result = ((v >> 1) | (this.carry() ? 0x80 : 0x00)) & 0xFF;
    this.a = result;
    this.setNz(result);
    this.setCarry((result & 0x40) !== 0);
    this.setOverflow(((result ^ ((result << 1) & 0xFF)) & 0x40) !== 0);
};
Cpu.prototype.axs = function (m: number): void {
    const ax = (this.a & this.x) & 0xFF;
    this.setCarry(ax >= m);
    const result = (ax - m) & 0xFF;
    this.x = result;
    this.setNz(result);
};
Cpu.prototype.xaa = function (m: number): void {
    const result = (((this.a | XaaMagic) & this.x) & m) & 0xFF;
    this.a = result;
    this.setNz(result);
};

// ---- unofficial unstable indexed stores ----
function storeDummyReadAddr(b: number, index: number): number {
    return ((b & 0xFF00) | ((b + index) & 0x00FF)) & 0xFFFF;
}

function quirkAddrY(b: number, y: number, value: number): number {
    const eff = (b + y) & 0xFFFF;
    if ((b & 0xFF00) !== (eff & 0xFF00))
        return (((value << 8) & 0xFF00) | (eff & 0x00FF)) & 0xFFFF;
    return eff;
}

function quirkAddrX(b: number, x: number, value: number): number {
    const eff = (b + x) & 0xFFFF;
    if ((b & 0xFF00) !== (eff & 0xFF00))
        return (((value << 8) & 0xFF00) | (eff & 0x00FF)) & 0xFFFF;
    return eff;
}

Cpu.prototype.tasStore = function (b: number): void {
    this.sp = (this.a & this.x) & 0xFF;
    const h = (b >> 8) & 0xFF;
    const value = (this.sp & ((h + 1) & 0xFF)) & 0xFF;
    const storeAddr = quirkAddrY(b, this.y, value);
    this.bus.read(storeDummyReadAddr(b, this.y));
    this.bus.write(storeAddr, value);
};

Cpu.prototype.ahxStore = function (b: number, reg: number): void {
    const h = (b >> 8) & 0xFF;
    const value = (reg & ((h + 1) & 0xFF)) & 0xFF;
    const storeAddr = quirkAddrY(b, this.y, value);
    this.bus.read(storeDummyReadAddr(b, this.y));
    this.bus.write(storeAddr, value);
};

Cpu.prototype.shyStore = function (b: number): void {
    const h = (b >> 8) & 0xFF;
    const value = (this.y & ((h + 1) & 0xFF)) & 0xFF;
    const storeAddr = quirkAddrX(b, this.x, value);
    this.bus.read(storeDummyReadAddr(b, this.x));
    this.bus.write(storeAddr, value);
};

// ---- RMW-combo helpers ----
Cpu.prototype.rmwCombo = function (op: any, f: (v: number) => number): void {
    const v = this.readOperand(op);
    const neu = f(v) & 0xFF;
    this.writeOperand(op, neu);
    this.setNz(neu);
};

Cpu.prototype.dcp = function (op: any): void {
    const self = this;
    this.rmwCombo(op, (v) => self.decValue(v));
    this.cmpOp(this.a, this.readOperand(op));
};
Cpu.prototype.isc = function (op: any): void {
    const self = this;
    this.rmwCombo(op, (v) => self.incValue(v));
    this.sbc(this.readOperand(op));
};
Cpu.prototype.slo = function (op: any): void {
    const self = this;
    this.rmwCombo(op, (v) => self.aslValue(v));
    this.oraOp(this.readOperand(op));
};
Cpu.prototype.rla = function (op: any): void {
    const self = this;
    this.rmwCombo(op, (v) => self.rolValue(v));
    this.andOp(this.readOperand(op));
};
Cpu.prototype.sre = function (op: any): void {
    const self = this;
    this.rmwCombo(op, (v) => self.lsrValue(v));
    this.eorOp(this.readOperand(op));
};
Cpu.prototype.rra = function (op: any): void {
    const self = this;
    this.rmwCombo(op, (v) => self.rorValue(v));
    this.adc(this.readOperand(op));
};
