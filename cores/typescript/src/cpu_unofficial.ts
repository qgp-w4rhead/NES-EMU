// Unofficial / illegal 6502 opcodes (port of cores/c/src/cpu/unofficial.c).

import { Cpu, Dummy } from './cpu';
import './cpu_addressing';
import './cpu_opcodes';

declare module './cpu' {
    interface Cpu {
        executeUnofficialRmw(opcode: number): number;
        executeUnofficial(opcode: number): number;
    }
}

Cpu.prototype.executeUnofficialRmw = function (opcode: number): number {
    switch (opcode) {
        // DCP
        case 0xC7: { const o = this.opZp(); this.dcp(o); return 5; }
        case 0xD7: { const o = this.opZpX(); this.dcp(o); return 6; }
        case 0xCF: { const o = this.opAbs(); this.dcp(o); return 6; }
        case 0xDF: { const o = this.opAbsX(); this.dcp(o); return 7; }
        case 0xDB: { const o = this.opAbsY(); this.dcp(o); return 7; }
        case 0xC3: { const o = this.opIndX(); this.dcp(o); return 8; }
        case 0xD3: { const o = this.opIndY(); this.dcp(o); return 8; }

        // ISC
        case 0xE7: { const o = this.opZp(); this.isc(o); return 5; }
        case 0xF7: { const o = this.opZpX(); this.isc(o); return 6; }
        case 0xEF: { const o = this.opAbs(); this.isc(o); return 6; }
        case 0xFF: { const o = this.opAbsX(); this.isc(o); return 7; }
        case 0xFB: { const o = this.opAbsY(); this.isc(o); return 7; }
        case 0xE3: { const o = this.opIndX(); this.isc(o); return 8; }
        case 0xF3: { const o = this.opIndY(); this.isc(o); return 8; }

        // SLO
        case 0x07: { const o = this.opZp(); this.slo(o); return 5; }
        case 0x17: { const o = this.opZpX(); this.slo(o); return 6; }
        case 0x0F: { const o = this.opAbs(); this.slo(o); return 6; }
        case 0x1F: { const o = this.opAbsX(); this.slo(o); return 7; }
        case 0x1B: { const o = this.opAbsY(); this.slo(o); return 7; }
        case 0x03: { const o = this.opIndX(); this.slo(o); return 8; }
        case 0x13: { const o = this.opIndY(); this.slo(o); return 8; }

        // RLA
        case 0x27: { const o = this.opZp(); this.rla(o); return 5; }
        case 0x37: { const o = this.opZpX(); this.rla(o); return 6; }
        case 0x2F: { const o = this.opAbs(); this.rla(o); return 6; }
        case 0x3F: { const o = this.opAbsX(); this.rla(o); return 7; }
        case 0x3B: { const o = this.opAbsY(); this.rla(o); return 7; }
        case 0x23: { const o = this.opIndX(); this.rla(o); return 8; }
        case 0x33: { const o = this.opIndY(); this.rla(o); return 8; }

        // SRE
        case 0x47: { const o = this.opZp(); this.sre(o); return 5; }
        case 0x57: { const o = this.opZpX(); this.sre(o); return 6; }
        case 0x4F: { const o = this.opAbs(); this.sre(o); return 6; }
        case 0x5F: { const o = this.opAbsX(); this.sre(o); return 7; }
        case 0x5B: { const o = this.opAbsY(); this.sre(o); return 7; }
        case 0x43: { const o = this.opIndX(); this.sre(o); return 8; }
        case 0x53: { const o = this.opIndY(); this.sre(o); return 8; }

        // RRA
        case 0x67: { const o = this.opZp(); this.rra(o); return 5; }
        case 0x77: { const o = this.opZpX(); this.rra(o); return 6; }
        case 0x6F: { const o = this.opAbs(); this.rra(o); return 6; }
        case 0x7F: { const o = this.opAbsX(); this.rra(o); return 7; }
        case 0x7B: { const o = this.opAbsY(); this.rra(o); return 7; }
        case 0x63: { const o = this.opIndX(); this.rra(o); return 8; }
        case 0x73: { const o = this.opIndY(); this.rra(o); return 8; }

        // TAS/SHS
        case 0x9B: { const b = this.fetchWord(); this.tasStore(b); return 5; }

        // AHX/SHA
        case 0x9F: { const b = this.fetchWord(); this.ahxStore(b, (this.a & this.x) & 0xFF); return 5; }
        case 0x93: { const b = this.amIndirectYBase(); this.ahxStore(b, (this.a & this.x) & 0xFF); return 6; }

        // SHX/SXA
        case 0x9E: { const b = this.fetchWord(); this.ahxStore(b, this.x); return 5; }

        // SHY/SYA
        case 0x9C: { const b = this.fetchWord(); this.shyStore(b); return 5; }

        default: return 2;
    }
};

Cpu.prototype.executeUnofficial = function (opcode: number): number {
    switch (opcode) {
        // Implied NOPs
        case 0x1A: case 0x3A: case 0x5A: case 0x7A: case 0xDA: case 0xFA: return 2;
        // Immediate NOPs
        case 0x80: case 0x82: case 0x89: case 0xC2: case 0xE2: this.fetchByte(); return 2;
        // Zero-page NOPs
        case 0x04: case 0x44: case 0x64: this.fetchByte(); return 3;
        // Zero-page,X NOPs
        case 0x14: case 0x34: case 0x54: case 0x74: case 0xD4: case 0xF4: this.amZeroPageX(Dummy.Rmw); return 4;
        // Absolute NOP
        case 0x0C: this.fetchWord(); return 4;
        // Absolute,X NOPs
        case 0x1C: case 0x3C: case 0x5C: case 0x7C: case 0xDC: case 0xFC: {
            const r = this.amAbsoluteX(Dummy.Read);
            return 4 + (r.pageCross ? 1 : 0);
        }

        // LAX
        case 0xA7: { const v = this.rdZp(); this.lax(v); return 3; }
        case 0xB7: { const v = this.rdZpY(); this.lax(v); return 4; }
        case 0xAF: { const v = this.rdAbs(); this.lax(v); return 4; }
        case 0xBF: { const r = this.rdAbsY(); this.lax(r.addr); return 4 + (r.pageCross ? 1 : 0); }
        case 0xA3: { const v = this.rdIndX(); this.lax(v); return 6; }
        case 0xB3: { const r = this.rdIndY(); this.lax(r.addr); return 5 + (r.pageCross ? 1 : 0); }

        // SAX
        case 0x87: this.wrZp((this.a & this.x) & 0xFF); return 3;
        case 0x97: this.wrZpY((this.a & this.x) & 0xFF); return 4;
        case 0x8F: this.wrAbs((this.a & this.x) & 0xFF); return 4;
        case 0x83: this.wrIndX((this.a & this.x) & 0xFF); return 6;

        // ANC
        case 0x0B: case 0x2B: { const v = this.rdImm(); this.anc(v); return 2; }
        // ALR
        case 0x4B: { const v = this.rdImm(); this.alr(v); return 2; }
        // ARR
        case 0x6B: { const v = this.rdImm(); this.arr(v); return 2; }
        // AXS/SBX
        case 0xCB: { const v = this.rdImm(); this.axs(v); return 2; }
        // XAA
        case 0x8B: { const v = this.rdImm(); this.xaa(v); return 2; }
        // LAS/LAR
        case 0xBB: {
            const r = this.rdAbsY();
            const v = r.addr;
            const res = (v & this.sp) & 0xFF;
            this.a = res; this.x = res; this.sp = res;
            this.setNz(res);
            return 4 + (r.pageCross ? 1 : 0);
        }

        // KIL/JAM/HLT
        case 0x02: case 0x12: case 0x22: case 0x32:
        case 0x42: case 0x52: case 0x62: case 0x72:
        case 0x92: case 0xB2: case 0xD2: case 0xF2:
            this.setHalted(true);
            return 1;

        default: return this.executeUnofficialRmw(opcode);
    }
};
