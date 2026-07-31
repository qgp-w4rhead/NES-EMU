// Official opcode dispatch (port of cores/c/src/cpu/execute.c).

import { Cpu } from './cpu';
import './cpu_addressing';
import './cpu_opcodes';

declare module './cpu' {
    interface Cpu {
        execute(opcode: number): number;
    }
}

Cpu.prototype.execute = function (opcode: number): number {
    switch (opcode) {
        // LDA
        case 0xA9: { const v = this.rdImm(); this.lda(v); return 2; }
        case 0xA5: { const v = this.rdZp(); this.lda(v); return 3; }
        case 0xB5: { const v = this.rdZpX(); this.lda(v); return 4; }
        case 0xAD: { const v = this.rdAbs(); this.lda(v); return 4; }
        case 0xBD: { const r = this.rdAbsX(); this.lda(r.addr); return 4 + (r.pageCross ? 1 : 0); }
        case 0xB9: { const r = this.rdAbsY(); this.lda(r.addr); return 4 + (r.pageCross ? 1 : 0); }
        case 0xA1: { const v = this.rdIndX(); this.lda(v); return 6; }
        case 0xB1: { const r = this.rdIndY(); this.lda(r.addr); return 5 + (r.pageCross ? 1 : 0); }

        // LDX
        case 0xA2: { const v = this.rdImm(); this.ldx(v); return 2; }
        case 0xA6: { const v = this.rdZp(); this.ldx(v); return 3; }
        case 0xB6: { const v = this.rdZpY(); this.ldx(v); return 4; }
        case 0xAE: { const v = this.rdAbs(); this.ldx(v); return 4; }
        case 0xBE: { const r = this.rdAbsY(); this.ldx(r.addr); return 4 + (r.pageCross ? 1 : 0); }

        // LDY
        case 0xA0: { const v = this.rdImm(); this.ldy(v); return 2; }
        case 0xA4: { const v = this.rdZp(); this.ldy(v); return 3; }
        case 0xB4: { const v = this.rdZpX(); this.ldy(v); return 4; }
        case 0xAC: { const v = this.rdAbs(); this.ldy(v); return 4; }
        case 0xBC: { const r = this.rdAbsX(); this.ldy(r.addr); return 4 + (r.pageCross ? 1 : 0); }

        // STA
        case 0x85: this.wrZp(this.a); return 3;
        case 0x95: this.wrZpX(this.a); return 4;
        case 0x8D: this.wrAbs(this.a); return 4;
        case 0x9D: this.wrAbsX(this.a); return 5;
        case 0x99: this.wrAbsY(this.a); return 5;
        case 0x81: this.wrIndX(this.a); return 6;
        case 0x91: this.wrIndY(this.a); return 6;

        // STX
        case 0x86: this.wrZp(this.x); return 3;
        case 0x96: this.wrZpY(this.x); return 4;
        case 0x8E: this.wrAbs(this.x); return 4;

        // STY
        case 0x84: this.wrZp(this.y); return 3;
        case 0x94: this.wrZpX(this.y); return 4;
        case 0x8C: this.wrAbs(this.y); return 4;

        // transfers
        case 0xAA: this.tax(); return 2;
        case 0xA8: this.tay(); return 2;
        case 0x8A: this.txa(); return 2;
        case 0x98: this.tya(); return 2;
        case 0xBA: this.tsx(); return 2;
        case 0x9A: this.setSpFromX(); return 2;

        // stack
        case 0x48: this.push(this.a); return 3;
        case 0x08: this.pushStatus(true); return 3;
        case 0x68: { const v = this.pull(); this.lda(v); return 4; }
        case 0x28: this.pullStatus(); return 4;

        // AND
        case 0x29: { const v = this.rdImm(); this.andOp(v); return 2; }
        case 0x25: { const v = this.rdZp(); this.andOp(v); return 3; }
        case 0x35: { const v = this.rdZpX(); this.andOp(v); return 4; }
        case 0x2D: { const v = this.rdAbs(); this.andOp(v); return 4; }
        case 0x3D: { const r = this.rdAbsX(); this.andOp(r.addr); return 4 + (r.pageCross ? 1 : 0); }
        case 0x39: { const r = this.rdAbsY(); this.andOp(r.addr); return 4 + (r.pageCross ? 1 : 0); }
        case 0x21: { const v = this.rdIndX(); this.andOp(v); return 6; }
        case 0x31: { const r = this.rdIndY(); this.andOp(r.addr); return 5 + (r.pageCross ? 1 : 0); }

        // ORA
        case 0x09: { const v = this.rdImm(); this.oraOp(v); return 2; }
        case 0x05: { const v = this.rdZp(); this.oraOp(v); return 3; }
        case 0x15: { const v = this.rdZpX(); this.oraOp(v); return 4; }
        case 0x0D: { const v = this.rdAbs(); this.oraOp(v); return 4; }
        case 0x1D: { const r = this.rdAbsX(); this.oraOp(r.addr); return 4 + (r.pageCross ? 1 : 0); }
        case 0x19: { const r = this.rdAbsY(); this.oraOp(r.addr); return 4 + (r.pageCross ? 1 : 0); }
        case 0x01: { const v = this.rdIndX(); this.oraOp(v); return 6; }
        case 0x11: { const r = this.rdIndY(); this.oraOp(r.addr); return 5 + (r.pageCross ? 1 : 0); }

        // EOR
        case 0x49: { const v = this.rdImm(); this.eorOp(v); return 2; }
        case 0x45: { const v = this.rdZp(); this.eorOp(v); return 3; }
        case 0x55: { const v = this.rdZpX(); this.eorOp(v); return 4; }
        case 0x4D: { const v = this.rdAbs(); this.eorOp(v); return 4; }
        case 0x5D: { const r = this.rdAbsX(); this.eorOp(r.addr); return 4 + (r.pageCross ? 1 : 0); }
        case 0x59: { const r = this.rdAbsY(); this.eorOp(r.addr); return 4 + (r.pageCross ? 1 : 0); }
        case 0x41: { const v = this.rdIndX(); this.eorOp(v); return 6; }
        case 0x51: { const r = this.rdIndY(); this.eorOp(r.addr); return 5 + (r.pageCross ? 1 : 0); }

        // BIT
        case 0x24: { const v = this.rdZp(); this.bitOp(v); return 3; }
        case 0x2C: { const v = this.rdAbs(); this.bitOp(v); return 4; }

        // ADC
        case 0x69: { const v = this.rdImm(); this.adc(v); return 2; }
        case 0x65: { const v = this.rdZp(); this.adc(v); return 3; }
        case 0x75: { const v = this.rdZpX(); this.adc(v); return 4; }
        case 0x6D: { const v = this.rdAbs(); this.adc(v); return 4; }
        case 0x7D: { const r = this.rdAbsX(); this.adc(r.addr); return 4 + (r.pageCross ? 1 : 0); }
        case 0x79: { const r = this.rdAbsY(); this.adc(r.addr); return 4 + (r.pageCross ? 1 : 0); }
        case 0x61: { const v = this.rdIndX(); this.adc(v); return 6; }
        case 0x71: { const r = this.rdIndY(); this.adc(r.addr); return 5 + (r.pageCross ? 1 : 0); }

        // SBC
        case 0xE9: { const v = this.rdImm(); this.sbc(v); return 2; }
        case 0xE5: { const v = this.rdZp(); this.sbc(v); return 3; }
        case 0xF5: { const v = this.rdZpX(); this.sbc(v); return 4; }
        case 0xED: { const v = this.rdAbs(); this.sbc(v); return 4; }
        case 0xFD: { const r = this.rdAbsX(); this.sbc(r.addr); return 4 + (r.pageCross ? 1 : 0); }
        case 0xF9: { const r = this.rdAbsY(); this.sbc(r.addr); return 4 + (r.pageCross ? 1 : 0); }
        case 0xE1: { const v = this.rdIndX(); this.sbc(v); return 6; }
        case 0xF1: { const r = this.rdIndY(); this.sbc(r.addr); return 5 + (r.pageCross ? 1 : 0); }

        // CMP
        case 0xC9: { const v = this.rdImm(); this.cmpOp(this.a, v); return 2; }
        case 0xC5: { const v = this.rdZp(); this.cmpOp(this.a, v); return 3; }
        case 0xD5: { const v = this.rdZpX(); this.cmpOp(this.a, v); return 4; }
        case 0xCD: { const v = this.rdAbs(); this.cmpOp(this.a, v); return 4; }
        case 0xDD: { const r = this.rdAbsX(); this.cmpOp(this.a, r.addr); return 4 + (r.pageCross ? 1 : 0); }
        case 0xD9: { const r = this.rdAbsY(); this.cmpOp(this.a, r.addr); return 4 + (r.pageCross ? 1 : 0); }
        case 0xC1: { const v = this.rdIndX(); this.cmpOp(this.a, v); return 6; }
        case 0xD1: { const r = this.rdIndY(); this.cmpOp(this.a, r.addr); return 5 + (r.pageCross ? 1 : 0); }

        // CPX/CPY
        case 0xE0: { const v = this.rdImm(); this.cmpOp(this.x, v); return 2; }
        case 0xE4: { const v = this.rdZp(); this.cmpOp(this.x, v); return 3; }
        case 0xEC: { const v = this.rdAbs(); this.cmpOp(this.x, v); return 4; }
        case 0xC0: { const v = this.rdImm(); this.cmpOp(this.y, v); return 2; }
        case 0xC4: { const v = this.rdZp(); this.cmpOp(this.y, v); return 3; }
        case 0xCC: { const v = this.rdAbs(); this.cmpOp(this.y, v); return 4; }

        // INC/DEC memory
        case 0xE6: { const self = this; this.rmwZp((v) => self.incValue(v)); return 5; }
        case 0xF6: { const self = this; this.rmwZpX((v) => self.incValue(v)); return 6; }
        case 0xEE: { const self = this; this.rmwAbs((v) => self.incValue(v)); return 6; }
        case 0xFE: { const self = this; this.rmwAbsX((v) => self.incValue(v)); return 7; }
        case 0xC6: { const self = this; this.rmwZp((v) => self.decValue(v)); return 5; }
        case 0xD6: { const self = this; this.rmwZpX((v) => self.decValue(v)); return 6; }
        case 0xCE: { const self = this; this.rmwAbs((v) => self.decValue(v)); return 6; }
        case 0xDE: { const self = this; this.rmwAbsX((v) => self.decValue(v)); return 7; }

        // INX/INY/DEX/DEY
        case 0xE8: this.x = (this.x + 1) & 0xFF; this.setNz(this.x); return 2;
        case 0xC8: this.y = (this.y + 1) & 0xFF; this.setNz(this.y); return 2;
        case 0xCA: this.x = (this.x - 1) & 0xFF; this.setNz(this.x); return 2;
        case 0x88: this.y = (this.y - 1) & 0xFF; this.setNz(this.y); return 2;

        // ASL
        case 0x0A: { const self = this; this.a = this.aslValue(this.a); this.setNz(this.a); return 2; }
        case 0x06: { const self = this; this.rmwZp((v) => self.aslValue(v)); return 5; }
        case 0x16: { const self = this; this.rmwZpX((v) => self.aslValue(v)); return 6; }
        case 0x0E: { const self = this; this.rmwAbs((v) => self.aslValue(v)); return 6; }
        case 0x1E: { const self = this; this.rmwAbsX((v) => self.aslValue(v)); return 7; }

        // LSR
        case 0x4A: { this.a = this.lsrValue(this.a); this.setNz(this.a); return 2; }
        case 0x46: { const self = this; this.rmwZp((v) => self.lsrValue(v)); return 5; }
        case 0x56: { const self = this; this.rmwZpX((v) => self.lsrValue(v)); return 6; }
        case 0x4E: { const self = this; this.rmwAbs((v) => self.lsrValue(v)); return 6; }
        case 0x5E: { const self = this; this.rmwAbsX((v) => self.lsrValue(v)); return 7; }

        // ROL
        case 0x2A: { this.a = this.rolValue(this.a); this.setNz(this.a); return 2; }
        case 0x26: { const self = this; this.rmwZp((v) => self.rolValue(v)); return 5; }
        case 0x36: { const self = this; this.rmwZpX((v) => self.rolValue(v)); return 6; }
        case 0x2E: { const self = this; this.rmwAbs((v) => self.rolValue(v)); return 6; }
        case 0x3E: { const self = this; this.rmwAbsX((v) => self.rolValue(v)); return 7; }

        // ROR
        case 0x6A: { this.a = this.rorValue(this.a); this.setNz(this.a); return 2; }
        case 0x66: { const self = this; this.rmwZp((v) => self.rorValue(v)); return 5; }
        case 0x76: { const self = this; this.rmwZpX((v) => self.rorValue(v)); return 6; }
        case 0x6E: { const self = this; this.rmwAbs((v) => self.rorValue(v)); return 6; }
        case 0x7E: { const self = this; this.rmwAbsX((v) => self.rorValue(v)); return 7; }

        // branches
        case 0x10: return this.branch(false, Cpu.FlagN);
        case 0x30: return this.branch(true, Cpu.FlagN);
        case 0x50: return this.branch(false, Cpu.FlagV);
        case 0x70: return this.branch(true, Cpu.FlagV);
        case 0x90: return this.branch(false, Cpu.FlagC);
        case 0xB0: return this.branch(true, Cpu.FlagC);
        case 0xD0: return this.branch(false, Cpu.FlagZ);
        case 0xF0: return this.branch(true, Cpu.FlagZ);

        // JMP/JSR/RTS/RTI/BRK
        case 0x4C: { const a = this.amAbsolute(); this.pc = a; return 3; }
        case 0x6C: { const a = this.amIndirect(); this.pc = a; return 5; }
        case 0x20: {
            const target = this.fetchWord();
            this.pushPc((this.pc - 1) & 0xFFFF);
            this.pc = target;
            return 6;
        }
        case 0x60: {
            const ret = (this.pullPc() + 1) & 0xFFFF;
            this.pc = ret;
            return 6;
        }
        case 0x40: {
            this.pullStatus();
            this.pc = this.pullPc();
            return 6;
        }
        case 0x00: {
            this.fetchByte();
            this.pushPc(this.pc);
            this.pushStatus(true);
            this.setInterruptDisable(true);
            this.pc = this.readVector(Cpu.VectorIrq);
            return 7;
        }

        // flag ops
        case 0x18: this.setCarry(false); return 2;
        case 0x38: this.setCarry(true); return 2;
        case 0x58: this.setInterruptDisable(false); return 2;
        case 0x78: this.setInterruptDisable(true); return 2;
        case 0xB8: this.setOverflow(false); return 2;
        case 0xD8: this.setDecimal(false); return 2;
        case 0xF8: this.setDecimal(true); return 2;

        // NOP
        case 0xEA: return 2;

        default: return this.executeUnofficial(opcode);
    }
};
