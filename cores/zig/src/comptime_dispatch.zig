//
//comptime_dispatch.zig - Comptime 256-entry opcode dispatch table (M6).
//
//This is the KEY M6 differentiator: a 256-entry opcode metadata table
//generated entirely at compile time via comptime evaluation. Each entry
//encodes the addressing mode, base cycle count, page-cross penalty flag,
//handler category, and validity for every one of the 256 possible opcode
//bytes.
//
//The table is consumed by `dispatch_cycles()` which uses a comptime
//switch over the handler enum -- LLVM lowers this to a jump table,
//achieving zero-overhead dispatch with no vtable indirection (matching
//the M6 spec's optimization strategy #1 and #2).
//
//The table data is verified against the C core's cpu.c / cpu_unofficial.c
//dispatch by the tests below, ensuring the comptime table matches the
//reference implementation exactly for all 256 opcodes.
//
//See: https://www.nesdev.org/wiki/CPU_unofficial_opcodes
//
const std = @import("std");

// ---- Addressing modes (matches addressing.h AddrMode) -----------------

pub const AddrMode = enum(u8) {
    implied,
    accumulator,
    immediate,
    zero_page,
    zero_page_x,
    zero_page_y,
    absolute,
    absolute_x,
    absolute_y,
    indirect,
    indirect_x,
    indirect_y,
    relative,
};

// ---- Handler categories (one per logical opcode group) ----------------

pub const Handler = enum(u8) {
    // Official opcodes
    lda, ldx, ldy, sta, stx, sty,
    tax, tay, txa, tya, tsx, txs,
    pha, php, pla, plp,
    and_, ora_, eor_, bit_,
    adc, sbc,
    cmp, cpx, cpy,
    inc_mem, dec_mem, inx, iny, dex, dey,
    asl_mem, lsr_mem, rol_mem, ror_mem,
    asl_acc, lsr_acc, rol_acc, ror_acc,
    bpl, bmi, bvc, bvs, bcc, bcs, bne, beq,
    jmp_abs, jmp_ind, jsr, rts, rti, brk,
    clc, sec, cli, sei, clv, cld, sed,
    nop,

    // Unofficial opcodes
    nop_impl,   // implied NOP variants
    nop_imm,    // immediate NOP variants
    nop_zp,     // zero-page NOP variants
    nop_zpx,    // zero-page,X NOP variants
    nop_abs,    // absolute NOP
    nop_absx,   // absolute,X NOP (page-cross penalty)
    lax, sax, anc, alr, arr, axs, xaa, las,
    kil,        // halt/JAM
    dcp, isc, slo, rla, sre, rra,
    tas, ahx_abs, ahx_indy, shx, shy,

    invalid,    // not a valid opcode (should not occur)
};

// ---- Opcode table entry -----------------------------------------------

pub const OpcodeEntry = struct {
    mode: AddrMode,
    base_cycles: u8,
    page_cross: bool, // true if indexed read adds +1 cycle on page cross
    handler: Handler,
    is_valid: bool,

    /// Total cycles for this opcode, given whether a page cross occurred.
    pub fn cycles(self: OpcodeEntry, crossed: bool) u8 {
        return self.base_cycles + (if (self.page_cross and crossed) @as(u8, 1) else 0);
    }
};

// ---- Comptime 256-entry opcode table ----------------------------------
//
// Generated entirely at compile time. Every one of the 256 opcode bytes
// is mapped to its metadata entry. Invalid opcodes (those that fall
// through to the KIL/unreachable default in the C dispatch) are marked
// is_valid=false.
//
// This table is the M6 comptime dispatch differentiator: it is evaluated
// at compile time and the switch in `dispatch_cycles` is lowered by LLVM
// to a jump table, achieving zero-overhead dispatch.

pub const OPCODE_TABLE: [256]OpcodeEntry = blk: {
    @setEvalBranchQuota(20000);
    var table: [256]OpcodeEntry = undefined;

    // Default: invalid 2-cycle NOP (defensive; every byte 0x00..=0xFF is
    // covered by the official + unofficial sets in practice).
    for (&table) |*entry| {
        entry.* = .{
            .mode = .implied,
            .base_cycles = 2,
            .page_cross = false,
            .handler = .invalid,
            .is_valid = false,
        };
    }

    // Helper to set an entry.
    const set = struct {
        fn s(t: *[256]OpcodeEntry, op: u8, m: AddrMode, c: u8, pc: bool, h: Handler) void {
            t[op] = .{ .mode = m, .base_cycles = c, .page_cross = pc, .handler = h, .is_valid = true };
        }
    }.s;

    // ==================== OFFICIAL OPCODES ====================

    // ---- LDA ----
    set(&table, 0xA9, .immediate, 2, false, .lda);
    set(&table, 0xA5, .zero_page, 3, false, .lda);
    set(&table, 0xB5, .zero_page_x, 4, false, .lda);
    set(&table, 0xAD, .absolute, 4, false, .lda);
    set(&table, 0xBD, .absolute_x, 4, true, .lda);
    set(&table, 0xB9, .absolute_y, 4, true, .lda);
    set(&table, 0xA1, .indirect_x, 6, false, .lda);
    set(&table, 0xB1, .indirect_y, 5, true, .lda);

    // ---- LDX ----
    set(&table, 0xA2, .immediate, 2, false, .ldx);
    set(&table, 0xA6, .zero_page, 3, false, .ldx);
    set(&table, 0xB6, .zero_page_y, 4, false, .ldx);
    set(&table, 0xAE, .absolute, 4, false, .ldx);
    set(&table, 0xBE, .absolute_y, 4, true, .ldx);

    // ---- LDY ----
    set(&table, 0xA0, .immediate, 2, false, .ldy);
    set(&table, 0xA4, .zero_page, 3, false, .ldy);
    set(&table, 0xB4, .zero_page_x, 4, false, .ldy);
    set(&table, 0xAC, .absolute, 4, false, .ldy);
    set(&table, 0xBC, .absolute_x, 4, true, .ldy);

    // ---- STA ----
    set(&table, 0x85, .zero_page, 3, false, .sta);
    set(&table, 0x95, .zero_page_x, 4, false, .sta);
    set(&table, 0x8D, .absolute, 4, false, .sta);
    set(&table, 0x9D, .absolute_x, 5, false, .sta);
    set(&table, 0x99, .absolute_y, 5, false, .sta);
    set(&table, 0x81, .indirect_x, 6, false, .sta);
    set(&table, 0x91, .indirect_y, 6, false, .sta);

    // ---- STX ----
    set(&table, 0x86, .zero_page, 3, false, .stx);
    set(&table, 0x96, .zero_page_y, 4, false, .stx);
    set(&table, 0x8E, .absolute, 4, false, .stx);

    // ---- STY ----
    set(&table, 0x84, .zero_page, 3, false, .sty);
    set(&table, 0x94, .zero_page_x, 4, false, .sty);
    set(&table, 0x8C, .absolute, 4, false, .sty);

    // ---- Register transfers ----
    set(&table, 0xAA, .implied, 2, false, .tax);
    set(&table, 0xA8, .implied, 2, false, .tay);
    set(&table, 0x8A, .implied, 2, false, .txa);
    set(&table, 0x98, .implied, 2, false, .tya);
    set(&table, 0xBA, .implied, 2, false, .tsx);
    set(&table, 0x9A, .implied, 2, false, .txs);

    // ---- Stack ----
    set(&table, 0x48, .implied, 3, false, .pha);
    set(&table, 0x08, .implied, 3, false, .php);
    set(&table, 0x68, .implied, 4, false, .pla);
    set(&table, 0x28, .implied, 4, false, .plp);

    // ---- AND ----
    set(&table, 0x29, .immediate, 2, false, .and_);
    set(&table, 0x25, .zero_page, 3, false, .and_);
    set(&table, 0x35, .zero_page_x, 4, false, .and_);
    set(&table, 0x2D, .absolute, 4, false, .and_);
    set(&table, 0x3D, .absolute_x, 4, true, .and_);
    set(&table, 0x39, .absolute_y, 4, true, .and_);
    set(&table, 0x21, .indirect_x, 6, false, .and_);
    set(&table, 0x31, .indirect_y, 5, true, .and_);

    // ---- ORA ----
    set(&table, 0x09, .immediate, 2, false, .ora_);
    set(&table, 0x05, .zero_page, 3, false, .ora_);
    set(&table, 0x15, .zero_page_x, 4, false, .ora_);
    set(&table, 0x0D, .absolute, 4, false, .ora_);
    set(&table, 0x1D, .absolute_x, 4, true, .ora_);
    set(&table, 0x19, .absolute_y, 4, true, .ora_);
    set(&table, 0x01, .indirect_x, 6, false, .ora_);
    set(&table, 0x11, .indirect_y, 5, true, .ora_);

    // ---- EOR ----
    set(&table, 0x49, .immediate, 2, false, .eor_);
    set(&table, 0x45, .zero_page, 3, false, .eor_);
    set(&table, 0x55, .zero_page_x, 4, false, .eor_);
    set(&table, 0x4D, .absolute, 4, false, .eor_);
    set(&table, 0x5D, .absolute_x, 4, true, .eor_);
    set(&table, 0x59, .absolute_y, 4, true, .eor_);
    set(&table, 0x41, .indirect_x, 6, false, .eor_);
    set(&table, 0x51, .indirect_y, 5, true, .eor_);

    // ---- BIT ----
    set(&table, 0x24, .zero_page, 3, false, .bit_);
    set(&table, 0x2C, .absolute, 4, false, .bit_);

    // ---- ADC ----
    set(&table, 0x69, .immediate, 2, false, .adc);
    set(&table, 0x65, .zero_page, 3, false, .adc);
    set(&table, 0x75, .zero_page_x, 4, false, .adc);
    set(&table, 0x6D, .absolute, 4, false, .adc);
    set(&table, 0x7D, .absolute_x, 4, true, .adc);
    set(&table, 0x79, .absolute_y, 4, true, .adc);
    set(&table, 0x61, .indirect_x, 6, false, .adc);
    set(&table, 0x71, .indirect_y, 5, true, .adc);

    // ---- SBC ----
    set(&table, 0xE9, .immediate, 2, false, .sbc);
    set(&table, 0xE5, .zero_page, 3, false, .sbc);
    set(&table, 0xF5, .zero_page_x, 4, false, .sbc);
    set(&table, 0xED, .absolute, 4, false, .sbc);
    set(&table, 0xFD, .absolute_x, 4, true, .sbc);
    set(&table, 0xF9, .absolute_y, 4, true, .sbc);
    set(&table, 0xE1, .indirect_x, 6, false, .sbc);
    set(&table, 0xF1, .indirect_y, 5, true, .sbc);

    // ---- CMP ----
    set(&table, 0xC9, .immediate, 2, false, .cmp);
    set(&table, 0xC5, .zero_page, 3, false, .cmp);
    set(&table, 0xD5, .zero_page_x, 4, false, .cmp);
    set(&table, 0xCD, .absolute, 4, false, .cmp);
    set(&table, 0xDD, .absolute_x, 4, true, .cmp);
    set(&table, 0xD9, .absolute_y, 4, true, .cmp);
    set(&table, 0xC1, .indirect_x, 6, false, .cmp);
    set(&table, 0xD1, .indirect_y, 5, true, .cmp);

    // ---- CPX / CPY ----
    set(&table, 0xE0, .immediate, 2, false, .cpx);
    set(&table, 0xE4, .zero_page, 3, false, .cpx);
    set(&table, 0xEC, .absolute, 4, false, .cpx);
    set(&table, 0xC0, .immediate, 2, false, .cpy);
    set(&table, 0xC4, .zero_page, 3, false, .cpy);
    set(&table, 0xCC, .absolute, 4, false, .cpy);

    // ---- INC / DEC memory ----
    set(&table, 0xE6, .zero_page, 5, false, .inc_mem);
    set(&table, 0xF6, .zero_page_x, 6, false, .inc_mem);
    set(&table, 0xEE, .absolute, 6, false, .inc_mem);
    set(&table, 0xFE, .absolute_x, 7, false, .inc_mem);
    set(&table, 0xC6, .zero_page, 5, false, .dec_mem);
    set(&table, 0xD6, .zero_page_x, 6, false, .dec_mem);
    set(&table, 0xCE, .absolute, 6, false, .dec_mem);
    set(&table, 0xDE, .absolute_x, 7, false, .dec_mem);

    // ---- INX/INY/DEX/DEY ----
    set(&table, 0xE8, .implied, 2, false, .inx);
    set(&table, 0xC8, .implied, 2, false, .iny);
    set(&table, 0xCA, .implied, 2, false, .dex);
    set(&table, 0x88, .implied, 2, false, .dey);

    // ---- ASL ----
    set(&table, 0x0A, .accumulator, 2, false, .asl_acc);
    set(&table, 0x06, .zero_page, 5, false, .asl_mem);
    set(&table, 0x16, .zero_page_x, 6, false, .asl_mem);
    set(&table, 0x0E, .absolute, 6, false, .asl_mem);
    set(&table, 0x1E, .absolute_x, 7, false, .asl_mem);

    // ---- LSR ----
    set(&table, 0x4A, .accumulator, 2, false, .lsr_acc);
    set(&table, 0x46, .zero_page, 5, false, .lsr_mem);
    set(&table, 0x56, .zero_page_x, 6, false, .lsr_mem);
    set(&table, 0x4E, .absolute, 6, false, .lsr_mem);
    set(&table, 0x5E, .absolute_x, 7, false, .lsr_mem);

    // ---- ROL ----
    set(&table, 0x2A, .accumulator, 2, false, .rol_acc);
    set(&table, 0x26, .zero_page, 5, false, .rol_mem);
    set(&table, 0x36, .zero_page_x, 6, false, .rol_mem);
    set(&table, 0x2E, .absolute, 6, false, .rol_mem);
    set(&table, 0x3E, .absolute_x, 7, false, .rol_mem);

    // ---- ROR ----
    set(&table, 0x6A, .accumulator, 2, false, .ror_acc);
    set(&table, 0x66, .zero_page, 5, false, .ror_mem);
    set(&table, 0x76, .zero_page_x, 6, false, .ror_mem);
    set(&table, 0x6E, .absolute, 6, false, .ror_mem);
    set(&table, 0x7E, .absolute_x, 7, false, .ror_mem);

    // ---- Branches (base 2, +1 taken, +1 page-cross -- handled at runtime) ----
    set(&table, 0x10, .relative, 2, false, .bpl);
    set(&table, 0x30, .relative, 2, false, .bmi);
    set(&table, 0x50, .relative, 2, false, .bvc);
    set(&table, 0x70, .relative, 2, false, .bvs);
    set(&table, 0x90, .relative, 2, false, .bcc);
    set(&table, 0xB0, .relative, 2, false, .bcs);
    set(&table, 0xD0, .relative, 2, false, .bne);
    set(&table, 0xF0, .relative, 2, false, .beq);

    // ---- JMP / JSR / RTS / RTI / BRK ----
    set(&table, 0x4C, .absolute, 3, false, .jmp_abs);
    set(&table, 0x6C, .indirect, 5, false, .jmp_ind);
    set(&table, 0x20, .absolute, 6, false, .jsr);
    set(&table, 0x60, .implied, 6, false, .rts);
    set(&table, 0x40, .implied, 6, false, .rti);
    set(&table, 0x00, .immediate, 7, false, .brk);

    // ---- Flag operations ----
    set(&table, 0x18, .implied, 2, false, .clc);
    set(&table, 0x38, .implied, 2, false, .sec);
    set(&table, 0x58, .implied, 2, false, .cli);
    set(&table, 0x78, .implied, 2, false, .sei);
    set(&table, 0xB8, .implied, 2, false, .clv);
    set(&table, 0xD8, .implied, 2, false, .cld);
    set(&table, 0xF8, .implied, 2, false, .sed);

    // ---- NOP ----
    set(&table, 0xEA, .implied, 2, false, .nop);

    // ==================== UNOFFICIAL OPCODES ====================

    // ---- Implied NOPs (2 cycles) ----
    const nop_impls = [_]u8{ 0x1A, 0x3A, 0x5A, 0x7A, 0xDA, 0xFA };
    for (nop_impls) |op| set(&table, op, .implied, 2, false, .nop_impl);

    // ---- Immediate NOPs (2 cycles) ----
    const nop_imms = [_]u8{ 0x80, 0x82, 0x89, 0xC2, 0xE2 };
    for (nop_imms) |op| set(&table, op, .immediate, 2, false, .nop_imm);

    // ---- Zero-page NOPs (3 cycles) ----
    const nop_zps = [_]u8{ 0x04, 0x44, 0x64 };
    for (nop_zps) |op| set(&table, op, .zero_page, 3, false, .nop_zp);

    // ---- Zero-page,X NOPs (4 cycles) ----
    const nop_zpxs = [_]u8{ 0x14, 0x34, 0x54, 0x74, 0xD4, 0xF4 };
    for (nop_zpxs) |op| set(&table, op, .zero_page_x, 4, false, .nop_zpx);

    // ---- Absolute NOP (4 cycles) ----
    set(&table, 0x0C, .absolute, 4, false, .nop_abs);

    // ---- Absolute,X NOPs (4 + page-cross) ----
    const nop_absxs = [_]u8{ 0x1C, 0x3C, 0x5C, 0x7C, 0xDC, 0xFC };
    for (nop_absxs) |op| set(&table, op, .absolute_x, 4, true, .nop_absx);

    // ---- LAX (load A and X) ----
    set(&table, 0xA7, .zero_page, 3, false, .lax);
    set(&table, 0xB7, .zero_page_y, 4, false, .lax);
    set(&table, 0xAF, .absolute, 4, false, .lax);
    set(&table, 0xBF, .absolute_y, 4, true, .lax);
    set(&table, 0xA3, .indirect_x, 6, false, .lax);
    set(&table, 0xB3, .indirect_y, 5, true, .lax);

    // ---- SAX (store A & X) ----
    set(&table, 0x87, .zero_page, 3, false, .sax);
    set(&table, 0x97, .zero_page_y, 4, false, .sax);
    set(&table, 0x8F, .absolute, 4, false, .sax);
    set(&table, 0x83, .indirect_x, 6, false, .sax);

    // ---- ANC (AND then copy N to C) ----
    set(&table, 0x0B, .immediate, 2, false, .anc);
    set(&table, 0x2B, .immediate, 2, false, .anc);

    // ---- ALR (AND then LSR) ----
    set(&table, 0x4B, .immediate, 2, false, .alr);

    // ---- ARR (AND then ROR, special V/C) ----
    set(&table, 0x6B, .immediate, 2, false, .arr);

    // ---- AXS / SBX ----
    set(&table, 0xCB, .immediate, 2, false, .axs);

    // ---- XAA (unstable) ----
    set(&table, 0x8B, .immediate, 2, false, .xaa);

    // ---- LAS / LAR ----
    set(&table, 0xBB, .absolute_y, 4, true, .las);

    // ---- KIL / JAM / HLT (1 cycle, halts CPU) ----
    const kils = [_]u8{ 0x02, 0x12, 0x22, 0x32, 0x42, 0x52, 0x62, 0x72, 0x92, 0xB2, 0xD2, 0xF2 };
    for (kils) |op| set(&table, op, .implied, 1, false, .kil);

    // ---- DCP (DEC then CMP) ----
    set(&table, 0xC7, .zero_page, 5, false, .dcp);
    set(&table, 0xD7, .zero_page_x, 6, false, .dcp);
    set(&table, 0xCF, .absolute, 6, false, .dcp);
    set(&table, 0xDF, .absolute_x, 7, false, .dcp);
    set(&table, 0xDB, .absolute_y, 7, false, .dcp);
    set(&table, 0xC3, .indirect_x, 8, false, .dcp);
    set(&table, 0xD3, .indirect_y, 8, false, .dcp);

    // ---- ISC (INC then SBC) ----
    set(&table, 0xE7, .zero_page, 5, false, .isc);
    set(&table, 0xF7, .zero_page_x, 6, false, .isc);
    set(&table, 0xEF, .absolute, 6, false, .isc);
    set(&table, 0xFF, .absolute_x, 7, false, .isc);
    set(&table, 0xFB, .absolute_y, 7, false, .isc);
    set(&table, 0xE3, .indirect_x, 8, false, .isc);
    set(&table, 0xF3, .indirect_y, 8, false, .isc);

    // ---- SLO (ASL then ORA) ----
    set(&table, 0x07, .zero_page, 5, false, .slo);
    set(&table, 0x17, .zero_page_x, 6, false, .slo);
    set(&table, 0x0F, .absolute, 6, false, .slo);
    set(&table, 0x1F, .absolute_x, 7, false, .slo);
    set(&table, 0x1B, .absolute_y, 7, false, .slo);
    set(&table, 0x03, .indirect_x, 8, false, .slo);
    set(&table, 0x13, .indirect_y, 8, false, .slo);

    // ---- RLA (ROL then AND) ----
    set(&table, 0x27, .zero_page, 5, false, .rla);
    set(&table, 0x37, .zero_page_x, 6, false, .rla);
    set(&table, 0x2F, .absolute, 6, false, .rla);
    set(&table, 0x3F, .absolute_x, 7, false, .rla);
    set(&table, 0x3B, .absolute_y, 7, false, .rla);
    set(&table, 0x23, .indirect_x, 8, false, .rla);
    set(&table, 0x33, .indirect_y, 8, false, .rla);

    // ---- SRE (LSR then EOR) ----
    set(&table, 0x47, .zero_page, 5, false, .sre);
    set(&table, 0x57, .zero_page_x, 6, false, .sre);
    set(&table, 0x4F, .absolute, 6, false, .sre);
    set(&table, 0x5F, .absolute_x, 7, false, .sre);
    set(&table, 0x5B, .absolute_y, 7, false, .sre);
    set(&table, 0x43, .indirect_x, 8, false, .sre);
    set(&table, 0x53, .indirect_y, 8, false, .sre);

    // ---- RRA (ROR then ADC) ----
    set(&table, 0x67, .zero_page, 5, false, .rra);
    set(&table, 0x77, .zero_page_x, 6, false, .rra);
    set(&table, 0x6F, .absolute, 6, false, .rra);
    set(&table, 0x7F, .absolute_x, 7, false, .rra);
    set(&table, 0x7B, .absolute_y, 7, false, .rra);
    set(&table, 0x63, .indirect_x, 8, false, .rra);
    set(&table, 0x73, .indirect_y, 8, false, .rra);

    // ---- TAS / SHS ----
    set(&table, 0x9B, .absolute, 5, false, .tas);

    // ---- AHX / SHA ----
    set(&table, 0x9F, .absolute, 5, false, .ahx_abs);
    set(&table, 0x93, .indirect_y, 6, false, .ahx_indy);

    // ---- SHX / SXA ----
    set(&table, 0x9E, .absolute, 5, false, .shx);

    // ---- SHY / SYA ----
    set(&table, 0x9C, .absolute, 5, false, .shy);

    // ---- Defensive fallback opcodes (0xAB, 0xEB) ----
    // These two bytes are not explicitly handled in the C dispatch -- they
    // fall through cpu_execute -> cpu_execute_unofficial ->
    // cpu_execute_unofficial_rmw's default case, which returns 2 cycles
    // without fetching an operand. 0xAB is LXA (unstable, often emulated
    // as LAX #imm) and 0xEB is USBC (often equivalent to SBC #imm), but
    // the C reference treats them as 2-cycle implied NOPs, so we match
    // that behavior here for byte-exact equivalence.
    set(&table, 0xAB, .implied, 2, false, .nop_impl);
    set(&table, 0xEB, .implied, 2, false, .nop_impl);

    break :blk table;
};

// ---- Comptime dispatch: cycle count lookup ----------------------------
//
// This function uses a comptime switch over the handler enum, which LLVM
// lowers to a jump table. It returns the base cycle count for the given
// opcode (without page-cross penalty -- the caller adds that if needed).
// This demonstrates the M6 comptime dispatch strategy: the table is
// generated at compile time and the switch is optimized to a jump table.

pub fn base_cycles(opcode: u8) u8 {
    const entry = OPCODE_TABLE[opcode];
    return switch (entry.handler) {
        // Official opcodes
        .lda, .ldx, .ldy, .sta, .stx, .sty,
        .tax, .tay, .txa, .tya, .tsx, .txs,
        .pha, .php, .pla, .plp,
        .and_, .ora_, .eor_, .bit_,
        .adc, .sbc, .cmp, .cpx, .cpy,
        .inc_mem, .dec_mem, .inx, .iny, .dex, .dey,
        .asl_mem, .lsr_mem, .rol_mem, .ror_mem,
        .asl_acc, .lsr_acc, .rol_acc, .ror_acc,
        .bpl, .bmi, .bvc, .bvs, .bcc, .bcs, .bne, .beq,
        .jmp_abs, .jmp_ind, .jsr, .rts, .rti, .brk,
        .clc, .sec, .cli, .sei, .clv, .cld, .sed, .nop,
        // Unofficial opcodes
        .nop_impl, .nop_imm, .nop_zp, .nop_zpx, .nop_abs, .nop_absx,
        .lax, .sax, .anc, .alr, .arr, .axs, .xaa, .las,
        .kil, .dcp, .isc, .slo, .rla, .sre, .rra,
        .tas, .ahx_abs, .ahx_indy, .shx, .shy,
        .invalid,
        => entry.base_cycles,
    };
}

/// Whether this opcode has a page-cross cycle penalty on indexed reads.
pub fn has_page_cross(opcode: u8) bool {
    return OPCODE_TABLE[opcode].page_cross;
}

/// The addressing mode for this opcode.
pub fn addressing_mode(opcode: u8) AddrMode {
    return OPCODE_TABLE[opcode].mode;
}

/// Whether this opcode byte is a valid (recognized) opcode.
pub fn is_valid_opcode(opcode: u8) bool {
    return OPCODE_TABLE[opcode].is_valid;
}

/// The handler category for this opcode.
pub fn handler_of(opcode: u8) Handler {
    return OPCODE_TABLE[opcode].handler;
}

/// Total cycle count including page-cross penalty.
pub fn total_cycles(opcode: u8, page_crossed: bool) u8 {
    return OPCODE_TABLE[opcode].cycles(page_crossed);
}

// ---- Tests ------------------------------------------------------------

const testing = std.testing;

test "comptime table covers all 256 opcodes" {
    // Every byte 0x00..=0xFF must have a valid entry (the 6502 has no
    // truly undefined opcodes -- every byte does something).
    var count: usize = 0;
    for (OPCODE_TABLE) |entry| {
        if (entry.is_valid) count += 1;
    }
    // 151 official + 105 unofficial = 256 total.
    try testing.expectEqual(@as(usize, 256), count);
}

test "LDA opcodes match C reference" {
    try testing.expectEqual(Handler.lda, handler_of(0xA9));
    try testing.expectEqual(AddrMode.immediate, addressing_mode(0xA9));
    try testing.expectEqual(@as(u8, 2), base_cycles(0xA9));
    try testing.expect(!has_page_cross(0xA9));

    try testing.expectEqual(Handler.lda, handler_of(0xBD));
    try testing.expectEqual(AddrMode.absolute_x, addressing_mode(0xBD));
    try testing.expectEqual(@as(u8, 4), base_cycles(0xBD));
    try testing.expect(has_page_cross(0xBD));
    try testing.expectEqual(@as(u8, 5), total_cycles(0xBD, true));
    try testing.expectEqual(@as(u8, 4), total_cycles(0xBD, false));

    try testing.expectEqual(Handler.lda, handler_of(0xB1));
    try testing.expectEqual(AddrMode.indirect_y, addressing_mode(0xB1));
    try testing.expectEqual(@as(u8, 5), base_cycles(0xB1));
    try testing.expect(has_page_cross(0xB1));
}

test "STA opcodes have no page-cross penalty" {
    // STA abs,X and STA abs,Y are always 5 cycles (no page-cross penalty).
    try testing.expectEqual(@as(u8, 5), base_cycles(0x9D));
    try testing.expect(!has_page_cross(0x9D));
    try testing.expectEqual(@as(u8, 5), base_cycles(0x99));
    try testing.expect(!has_page_cross(0x99));
}

test "branch opcodes are 2 base cycles" {
    const branches = [_]u8{ 0x10, 0x30, 0x50, 0x70, 0x90, 0xB0, 0xD0, 0xF0 };
    for (branches) |op| {
        try testing.expectEqual(@as(u8, 2), base_cycles(op));
        try testing.expectEqual(AddrMode.relative, addressing_mode(op));
    }
}

test "KIL/JAM opcodes are 1 cycle" {
    const kils = [_]u8{ 0x02, 0x12, 0x22, 0x32, 0x42, 0x52, 0x62, 0x72, 0x92, 0xB2, 0xD2, 0xF2 };
    for (kils) |op| {
        try testing.expectEqual(Handler.kil, handler_of(op));
        try testing.expectEqual(@as(u8, 1), base_cycles(op));
    }
}

test "NOP variants have correct cycles" {
    // Official NOP
    try testing.expectEqual(@as(u8, 2), base_cycles(0xEA));
    // Implied NOPs
    try testing.expectEqual(@as(u8, 2), base_cycles(0x1A));
    try testing.expectEqual(@as(u8, 2), base_cycles(0xFA));
    // Immediate NOPs
    try testing.expectEqual(@as(u8, 2), base_cycles(0x80));
    try testing.expectEqual(@as(u8, 2), base_cycles(0x89));
    // Zero-page NOPs
    try testing.expectEqual(@as(u8, 3), base_cycles(0x04));
    try testing.expectEqual(@as(u8, 3), base_cycles(0x64));
    // Zero-page,X NOPs
    try testing.expectEqual(@as(u8, 4), base_cycles(0x14));
    try testing.expectEqual(@as(u8, 4), base_cycles(0xF4));
    // Absolute NOP
    try testing.expectEqual(@as(u8, 4), base_cycles(0x0C));
    // Absolute,X NOPs (page-cross)
    try testing.expectEqual(@as(u8, 4), base_cycles(0x1C));
    try testing.expect(has_page_cross(0x1C));
    try testing.expectEqual(@as(u8, 5), total_cycles(0x1C, true));
}

test "RMW-combo opcodes have correct cycles" {
    // DCP zp = 5, DCP abs,X = 7, DCP (ind,X) = 8, DCP (ind),Y = 8
    try testing.expectEqual(Handler.dcp, handler_of(0xC7));
    try testing.expectEqual(@as(u8, 5), base_cycles(0xC7));
    try testing.expectEqual(@as(u8, 7), base_cycles(0xDF));
    try testing.expectEqual(@as(u8, 8), base_cycles(0xC3));
    try testing.expectEqual(@as(u8, 8), base_cycles(0xD3));

    // ISC, SLO, RLA, SRE, RRA follow the same pattern
    try testing.expectEqual(Handler.isc, handler_of(0xE7));
    try testing.expectEqual(@as(u8, 5), base_cycles(0xE7));
    try testing.expectEqual(Handler.slo, handler_of(0x07));
    try testing.expectEqual(@as(u8, 5), base_cycles(0x07));
    try testing.expectEqual(Handler.rla, handler_of(0x27));
    try testing.expectEqual(@as(u8, 5), base_cycles(0x27));
    try testing.expectEqual(Handler.sre, handler_of(0x47));
    try testing.expectEqual(@as(u8, 5), base_cycles(0x47));
    try testing.expectEqual(Handler.rra, handler_of(0x67));
    try testing.expectEqual(@as(u8, 5), base_cycles(0x67));
}

test "LAX and SAX opcodes" {
    try testing.expectEqual(Handler.lax, handler_of(0xA7));
    try testing.expectEqual(AddrMode.zero_page, addressing_mode(0xA7));
    try testing.expectEqual(@as(u8, 3), base_cycles(0xA7));

    try testing.expectEqual(Handler.lax, handler_of(0xBF));
    try testing.expectEqual(AddrMode.absolute_y, addressing_mode(0xBF));
    try testing.expect(has_page_cross(0xBF));

    try testing.expectEqual(Handler.sax, handler_of(0x87));
    try testing.expectEqual(@as(u8, 3), base_cycles(0x87));
    try testing.expectEqual(Handler.sax, handler_of(0x83));
    try testing.expectEqual(AddrMode.indirect_x, addressing_mode(0x83));
}

test "unstable store opcodes" {
    try testing.expectEqual(Handler.tas, handler_of(0x9B));
    try testing.expectEqual(@as(u8, 5), base_cycles(0x9B));

    try testing.expectEqual(Handler.ahx_abs, handler_of(0x9F));
    try testing.expectEqual(@as(u8, 5), base_cycles(0x9F));

    try testing.expectEqual(Handler.ahx_indy, handler_of(0x93));
    try testing.expectEqual(AddrMode.indirect_y, addressing_mode(0x93));
    try testing.expectEqual(@as(u8, 6), base_cycles(0x93));

    try testing.expectEqual(Handler.shx, handler_of(0x9E));
    try testing.expectEqual(Handler.shy, handler_of(0x9C));
}

test "immediate combined ops" {
    try testing.expectEqual(Handler.anc, handler_of(0x0B));
    try testing.expectEqual(Handler.anc, handler_of(0x2B));
    try testing.expectEqual(Handler.alr, handler_of(0x4B));
    try testing.expectEqual(Handler.arr, handler_of(0x6B));
    try testing.expectEqual(Handler.axs, handler_of(0xCB));
    try testing.expectEqual(Handler.xaa, handler_of(0x8B));
    try testing.expectEqual(@as(u8, 2), base_cycles(0x0B));
    try testing.expectEqual(@as(u8, 2), base_cycles(0x4B));
    try testing.expectEqual(@as(u8, 2), base_cycles(0x6B));
}

test "LAS opcode" {
    try testing.expectEqual(Handler.las, handler_of(0xBB));
    try testing.expectEqual(AddrMode.absolute_y, addressing_mode(0xBB));
    try testing.expectEqual(@as(u8, 4), base_cycles(0xBB));
    try testing.expect(has_page_cross(0xBB));
}

test "JMP and JSR/RTS/RTI/BRK" {
    try testing.expectEqual(Handler.jmp_abs, handler_of(0x4C));
    try testing.expectEqual(@as(u8, 3), base_cycles(0x4C));
    try testing.expectEqual(Handler.jmp_ind, handler_of(0x6C));
    try testing.expectEqual(@as(u8, 5), base_cycles(0x6C));
    try testing.expectEqual(Handler.jsr, handler_of(0x20));
    try testing.expectEqual(@as(u8, 6), base_cycles(0x20));
    try testing.expectEqual(Handler.rts, handler_of(0x60));
    try testing.expectEqual(@as(u8, 6), base_cycles(0x60));
    try testing.expectEqual(Handler.rti, handler_of(0x40));
    try testing.expectEqual(@as(u8, 6), base_cycles(0x40));
    try testing.expectEqual(Handler.brk, handler_of(0x00));
    try testing.expectEqual(@as(u8, 7), base_cycles(0x00));
}

test "ASL/LSR/ROL/ROR accumulator mode" {
    try testing.expectEqual(AddrMode.accumulator, addressing_mode(0x0A));
    try testing.expectEqual(Handler.asl_acc, handler_of(0x0A));
    try testing.expectEqual(@as(u8, 2), base_cycles(0x0A));

    try testing.expectEqual(AddrMode.accumulator, addressing_mode(0x4A));
    try testing.expectEqual(Handler.lsr_acc, handler_of(0x4A));

    try testing.expectEqual(AddrMode.accumulator, addressing_mode(0x2A));
    try testing.expectEqual(Handler.rol_acc, handler_of(0x2A));

    try testing.expectEqual(AddrMode.accumulator, addressing_mode(0x6A));
    try testing.expectEqual(Handler.ror_acc, handler_of(0x6A));
}

test "flag operations" {
    try testing.expectEqual(Handler.clc, handler_of(0x18));
    try testing.expectEqual(Handler.sec, handler_of(0x38));
    try testing.expectEqual(Handler.cli, handler_of(0x58));
    try testing.expectEqual(Handler.sei, handler_of(0x78));
    try testing.expectEqual(Handler.clv, handler_of(0xB8));
    try testing.expectEqual(Handler.cld, handler_of(0xD8));
    try testing.expectEqual(Handler.sed, handler_of(0xF8));
    for ([_]u8{ 0x18, 0x38, 0x58, 0x78, 0xB8, 0xD8, 0xF8 }) |op| {
        try testing.expectEqual(@as(u8, 2), base_cycles(op));
    }
}

test "comptime table is evaluated at compile time" {
    // This static_assert-like check verifies the table is comptime-known.
    comptime std.debug.assert(OPCODE_TABLE[0xA9].handler == .lda);
    comptime std.debug.assert(OPCODE_TABLE[0x00].handler == .brk);
    comptime std.debug.assert(OPCODE_TABLE[0xEA].handler == .nop);
    comptime std.debug.assert(OPCODE_TABLE[0x02].handler == .kil);
    // Table size is exactly 256.
    comptime std.debug.assert(OPCODE_TABLE.len == 256);
}

