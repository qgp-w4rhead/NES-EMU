/*
 * cpu_unofficial.c — unofficial / illegal 6502 opcodes (M33).
 *
 * Port of src/cpu/unofficial.rs + src/cpu/unofficial_rmw.rs +
 * src/cpu/unofficial_special.rs to C (M4.1).
 *
 * Dispatch root `cpu_execute_unofficial` handles the "simple" unofficial
 * opcodes (NOP variants, LAX, SAX, immediate combined ops ANC/ALR/ARR/AXS/XAA,
 * LAS, KIL) and delegates the RMW-combo opcodes (DCP/ISC/SLO/RLA/SRE/RRA) and
 * unstable indexed stores (TAS/AHX/SHX/SHY) to `cpu_execute_unofficial_rmw`.
 *
 * Cycle counts follow the NESdev reference table. RMW-combo opcodes use the
 * RMW addressing helpers (Dummy::Rmw) and inherit the unconditional dummy-read
 * cycle (no page-cross penalty on indexed variants — the +1 is already paid).
 * NOP absolute,X is the exception: it is a read-style NOP and adds the +1
 * page-cross penalty.
 *
 * Unstable opcodes (XAA, LAS, TAS, AHX, SHX, SHY) follow the most commonly
 * emulated model (per NESdev wiki) so the majority of test ROMs behave
 * correctly. Determinism is preserved.
 *
 * See: https://www.nesdev.org/wiki/CPU_unofficial_opcodes
 */
#include "cpu.hpp"

/* Magic constant for the unstable XAA opcode (unofficial_special.rs `XAA_MAGIC`).
 * Real silicon varies; 0x00 is the most common stable approximation, giving
 * A = A & X & imm. */
#define XAA_MAGIC 0x00u

/* ---- RMW helper reuse from cpu.c ------------------------------------- */
/* cpu.c defines `rmw` statically; we need an equivalent here for the RMW-combo
 * opcodes. We re-implement the small rmw wrapper locally (it is trivial and
 * must match opcodes.rs `rmw` exactly). */
static void rmw_combo(Cpu* cpu, Bus* bus, Operand op, ValueFn f) {
    uint8_t v   = cpu_read_operand(cpu, bus, op);
    uint8_t neu = f(cpu, v);
    cpu_write_operand(cpu, bus, op, neu);
    cpu_set_nz(cpu, neu);
}

/* ---- Value transforms (reuse the ones from cpu.c via function pointers) ---- */
/* cpu.c declares these as static, so we cannot link to them. Re-declare thin
 * wrappers that call the same logic. To avoid divergence, we re-implement them
 * here identically to opcodes.rs. */
static uint8_t u_asl_value(Cpu* cpu, uint8_t v) { cpu_set_carry(cpu, (v & 0x80u) != 0u); return (uint8_t)(v << 1); }
static uint8_t u_lsr_value(Cpu* cpu, uint8_t v) { cpu_set_carry(cpu, (v & 0x01u) != 0u); return (uint8_t)(v >> 1); }
static uint8_t u_rol_value(Cpu* cpu, uint8_t v) {
    bool new_c = (v & 0x80u) != 0u;
    uint8_t result = (uint8_t)((v << 1) | (cpu_carry(cpu) ? 1u : 0u));
    cpu_set_carry(cpu, new_c);
    return result;
}
static uint8_t u_ror_value(Cpu* cpu, uint8_t v) {
    bool new_c = (v & 0x01u) != 0u;
    uint8_t result = (uint8_t)((v >> 1) | (cpu_carry(cpu) ? 0x80u : 0x00u));
    cpu_set_carry(cpu, new_c);
    return result;
}
static uint8_t u_inc_value(Cpu* cpu, uint8_t v) { (void)cpu; return (uint8_t)(v + 1u); }
static uint8_t u_dec_value(Cpu* cpu, uint8_t v) { (void)cpu; return (uint8_t)(v - 1u); }

/* ---- register-op helpers (re-implemented identically to opcodes.rs) ---- */
static void u_and(Cpu* cpu, uint8_t v) { cpu->a = (uint8_t)(cpu->a & v); cpu_set_nz(cpu, cpu->a); }
static void u_ora(Cpu* cpu, uint8_t v) { cpu->a = (uint8_t)(cpu->a | v); cpu_set_nz(cpu, cpu->a); }
static void u_eor(Cpu* cpu, uint8_t v) { cpu->a = (uint8_t)(cpu->a ^ v); cpu_set_nz(cpu, cpu->a); }
static void u_adc(Cpu* cpu, uint8_t m) {
    uint16_t a = (uint16_t)cpu->a;
    uint16_t mm = (uint16_t)m;
    uint16_t c = cpu_carry(cpu) ? 1u : 0u;
    uint16_t sum = (uint16_t)(a + mm + c);
    cpu_set_carry(cpu, sum > 0xFFu);
    uint8_t result = (uint8_t)(sum & 0xFFu);
    cpu_set_overflow(cpu, (((a ^ mm) & 0x80u) == 0u) && (((a ^ sum) & 0x80u) != 0u));
    cpu->a = result;
    cpu_set_nz(cpu, result);
}
static void u_sbc(Cpu* cpu, uint8_t m) {
    uint16_t a = (uint16_t)cpu->a;
    uint16_t mm = (uint16_t)((uint8_t)(~m));
    uint16_t c = cpu_carry(cpu) ? 1u : 0u;
    uint16_t sum = (uint16_t)(a + mm + c);
    cpu_set_carry(cpu, sum > 0xFFu);
    uint8_t result = (uint8_t)(sum & 0xFFu);
    cpu_set_overflow(cpu, (((a ^ mm) & 0x80u) == 0u) && (((a ^ sum) & 0x80u) != 0u));
    cpu->a = result;
    cpu_set_nz(cpu, result);
}
static void u_cmp(Cpu* cpu, uint8_t r, uint8_t m) {
    uint8_t diff = (uint8_t)(r - m);
    cpu_set_carry(cpu, r >= m);
    cpu_set_nz(cpu, diff);
}

/* ---- LAX (unofficial.rs `lax`) --------------------------------------- */
static void lax(Cpu* cpu, uint8_t m) {
    cpu->a = m;
    cpu->x = m;
    cpu_set_nz(cpu, m);
}

/* ---- Immediate combined ops (unofficial_special.rs) ------------------ */

/* ANC: A = A & imm; C = N = bit 7 of A. */
static void anc(Cpu* cpu, uint8_t m) {
    cpu->a = (uint8_t)(cpu->a & m);
    cpu_set_nz(cpu, cpu->a);
    cpu_set_carry(cpu, (cpu->a & 0x80u) != 0u);
}

/* ALR: A = (A & imm) >> 1; C = old bit 0 of (A & imm). */
static void alr(Cpu* cpu, uint8_t m) {
    uint8_t v = (uint8_t)(cpu->a & m);
    cpu_set_carry(cpu, (v & 0x01u) != 0u);
    uint8_t result = (uint8_t)(v >> 1);
    cpu->a = result;
    cpu_set_nz(cpu, result);
}

/* ARR: A = (A & imm) then ROR with current carry; special V/C.
 * C = bit 6 of result; V = bit6 XOR bit5 of result. */
static void arr(Cpu* cpu, uint8_t m) {
    uint8_t v = (uint8_t)(cpu->a & m);
    uint8_t result = (uint8_t)((v >> 1) | (cpu_carry(cpu) ? 0x80u : 0x00u));
    cpu->a = result;
    cpu_set_nz(cpu, result);
    cpu_set_carry(cpu, (result & 0x40u) != 0u);
    cpu_set_overflow(cpu, ((result ^ (uint8_t)(result << 1)) & 0x40u) != 0u);
}

/* AXS (SBX): X = (A & X) - imm. C = no borrow ((A&X) >= imm). N/Z from result. */
static void axs(Cpu* cpu, uint8_t m) {
    uint8_t ax = (uint8_t)(cpu->a & cpu->x);
    cpu_set_carry(cpu, ax >= m);
    uint8_t result = (uint8_t)(ax - m);
    cpu->x = result;
    cpu_set_nz(cpu, result);
}

/* XAA (unstable): A = (A | XAA_MAGIC) & X & imm. With XAA_MAGIC=0 -> A & X & imm. */
static void xaa(Cpu* cpu, uint8_t m) {
    uint8_t result = (uint8_t)((uint8_t)((cpu->a | XAA_MAGIC) & cpu->x) & m);
    cpu->a = result;
    cpu_set_nz(cpu, result);
}

/* ---- Unstable indexed stores (unofficial_special.rs) ----------------- */

/* Compute the dummy-read address for an unstable indexed store: high byte of
 * base ORed with low byte of (base + index). (unofficial_special.rs
 * `store_dummy_read_addr`.) */
static uint16_t store_dummy_read_addr(uint16_t base, uint8_t index) {
    return (uint16_t)((base & 0xFF00u) | ((uint16_t)(base + (uint16_t)index) & 0x00FFu));
}

/* quirk_addr_y: effective store address for a Y-indexed unstable store with
 * the page-cross quirk (high byte becomes `value` on page cross). */
static uint16_t quirk_addr_y(uint16_t base, uint8_t y, uint8_t value) {
    uint16_t eff = (uint16_t)(base + (uint16_t)y);
    if ((base & 0xFF00u) != (eff & 0xFF00u)) {
        return (uint16_t)(((uint16_t)value << 8) | (eff & 0x00FFu));
    }
    return eff;
}

/* quirk_addr_x: same as quirk_addr_y but X-indexed (used by SHY). */
static uint16_t quirk_addr_x(uint16_t base, uint8_t x, uint8_t value) {
    uint16_t eff = (uint16_t)(base + (uint16_t)x);
    if ((base & 0xFF00u) != (eff & 0xFF00u)) {
        return (uint16_t)(((uint16_t)value << 8) | (eff & 0x00FFu));
    }
    return eff;
}

/* TAS (SHS): SP = A & X; store SP & (H+1) at base+Y with page-cross quirk. */
static void tas_store(Cpu* cpu, Bus* bus, uint16_t base) {
    cpu->sp = (uint8_t)(cpu->a & cpu->x);
    uint8_t h = (uint8_t)(base >> 8);
    uint8_t value = (uint8_t)(cpu->sp & (uint8_t)(h + 1u));
    uint16_t store_addr = quirk_addr_y(base, cpu->y, value);
    (void)bus_read(bus, store_dummy_read_addr(base, cpu->y));
    bus_write(bus, store_addr, value);
}

/* AHX (SHA): store reg & (H+1) at base+Y with page-cross quirk. */
static void ahx_store(Cpu* cpu, Bus* bus, uint16_t base, uint8_t reg) {
    uint8_t h = (uint8_t)(base >> 8);
    uint8_t value = (uint8_t)(reg & (uint8_t)(h + 1u));
    uint16_t store_addr = quirk_addr_y(base, cpu->y, value);
    (void)bus_read(bus, store_dummy_read_addr(base, cpu->y));
    bus_write(bus, store_addr, value);
}

/* SHY (SYA): store Y & (H+1) at base+X with page-cross quirk (X-indexed). */
static void shy_store(Cpu* cpu, Bus* bus, uint16_t base) {
    uint8_t h = (uint8_t)(base >> 8);
    uint8_t value = (uint8_t)(cpu->y & (uint8_t)(h + 1u));
    uint16_t store_addr = quirk_addr_x(base, cpu->x, value);
    (void)bus_read(bus, store_dummy_read_addr(base, cpu->x));
    bus_write(bus, store_addr, value);
}

/* ---- RMW-combo helpers (unofficial_rmw.rs) --------------------------- */

/* DCP: DEC memory then CMP A against the new value. C/Z/N from compare. */
static void dcp(Cpu* cpu, Bus* bus, Operand op) {
    rmw_combo(cpu, bus, op, u_dec_value);
    uint8_t m = cpu_read_operand(cpu, bus, op);
    u_cmp(cpu, cpu->a, m);
}

/* ISC: INC memory then SBC A against the new value. */
static void isc(Cpu* cpu, Bus* bus, Operand op) {
    rmw_combo(cpu, bus, op, u_inc_value);
    uint8_t m = cpu_read_operand(cpu, bus, op);
    u_sbc(cpu, m);
}

/* SLO: ASL memory then ORA A with the new value. C from ASL preserved. */
static void slo(Cpu* cpu, Bus* bus, Operand op) {
    rmw_combo(cpu, bus, op, u_asl_value);
    uint8_t m = cpu_read_operand(cpu, bus, op);
    u_ora(cpu, m);
}

/* RLA: ROL memory then AND A with the new value. C from ROL preserved. */
static void rla(Cpu* cpu, Bus* bus, Operand op) {
    rmw_combo(cpu, bus, op, u_rol_value);
    uint8_t m = cpu_read_operand(cpu, bus, op);
    u_and(cpu, m);
}

/* SRE: LSR memory then EOR A with the new value. C from LSR preserved. */
static void sre(Cpu* cpu, Bus* bus, Operand op) {
    rmw_combo(cpu, bus, op, u_lsr_value);
    uint8_t m = cpu_read_operand(cpu, bus, op);
    u_eor(cpu, m);
}

/* RRA: ROR memory then ADC A with the new value. ROR-set carry is consumed
 * by the ADC as carry-in; ADC's carry-out replaces C. */
static void rra(Cpu* cpu, Bus* bus, Operand op) {
    rmw_combo(cpu, bus, op, u_ror_value);
    uint8_t m = cpu_read_operand(cpu, bus, op);
    u_adc(cpu, m);
}

/* ---- execute_unofficial_rmw (unofficial_rmw.rs `execute_unofficial_rmw`) ---- */

uint8_t cpu_execute_unofficial_rmw(Cpu* cpu, Bus* bus, uint8_t opcode) {
    switch (opcode) {
        /* ---- DCP (DEC then CMP) ---- */
        case 0xC7u: { Operand o = op_zp(cpu, bus);    dcp(cpu, bus, o); return 5u; }
        case 0xD7u: { Operand o = op_zp_x(cpu, bus);  dcp(cpu, bus, o); return 6u; }
        case 0xCFu: { Operand o = op_abs(cpu, bus);   dcp(cpu, bus, o); return 6u; }
        case 0xDFu: { Operand o = op_abs_x(cpu, bus); dcp(cpu, bus, o); return 7u; }
        case 0xDBu: { Operand o = op_abs_y(cpu, bus); dcp(cpu, bus, o); return 7u; }
        case 0xC3u: { Operand o = op_ind_x(cpu, bus); dcp(cpu, bus, o); return 8u; }
        case 0xD3u: { Operand o = op_ind_y(cpu, bus); dcp(cpu, bus, o); return 8u; }

        /* ---- ISC (INC then SBC) ---- */
        case 0xE7u: { Operand o = op_zp(cpu, bus);    isc(cpu, bus, o); return 5u; }
        case 0xF7u: { Operand o = op_zp_x(cpu, bus);  isc(cpu, bus, o); return 6u; }
        case 0xEFu: { Operand o = op_abs(cpu, bus);   isc(cpu, bus, o); return 6u; }
        case 0xFFu: { Operand o = op_abs_x(cpu, bus); isc(cpu, bus, o); return 7u; }
        case 0xFBu: { Operand o = op_abs_y(cpu, bus); isc(cpu, bus, o); return 7u; }
        case 0xE3u: { Operand o = op_ind_x(cpu, bus); isc(cpu, bus, o); return 8u; }
        case 0xF3u: { Operand o = op_ind_y(cpu, bus); isc(cpu, bus, o); return 8u; }

        /* ---- SLO (ASL then ORA) ---- */
        case 0x07u: { Operand o = op_zp(cpu, bus);    slo(cpu, bus, o); return 5u; }
        case 0x17u: { Operand o = op_zp_x(cpu, bus);  slo(cpu, bus, o); return 6u; }
        case 0x0Fu: { Operand o = op_abs(cpu, bus);   slo(cpu, bus, o); return 6u; }
        case 0x1Fu: { Operand o = op_abs_x(cpu, bus); slo(cpu, bus, o); return 7u; }
        case 0x1Bu: { Operand o = op_abs_y(cpu, bus); slo(cpu, bus, o); return 7u; }
        case 0x03u: { Operand o = op_ind_x(cpu, bus); slo(cpu, bus, o); return 8u; }
        case 0x13u: { Operand o = op_ind_y(cpu, bus); slo(cpu, bus, o); return 8u; }

        /* ---- RLA (ROL then AND) ---- */
        case 0x27u: { Operand o = op_zp(cpu, bus);    rla(cpu, bus, o); return 5u; }
        case 0x37u: { Operand o = op_zp_x(cpu, bus);  rla(cpu, bus, o); return 6u; }
        case 0x2Fu: { Operand o = op_abs(cpu, bus);   rla(cpu, bus, o); return 6u; }
        case 0x3Fu: { Operand o = op_abs_x(cpu, bus); rla(cpu, bus, o); return 7u; }
        case 0x3Bu: { Operand o = op_abs_y(cpu, bus); rla(cpu, bus, o); return 7u; }
        case 0x23u: { Operand o = op_ind_x(cpu, bus); rla(cpu, bus, o); return 8u; }
        case 0x33u: { Operand o = op_ind_y(cpu, bus); rla(cpu, bus, o); return 8u; }

        /* ---- SRE (LSR then EOR) ---- */
        case 0x47u: { Operand o = op_zp(cpu, bus);    sre(cpu, bus, o); return 5u; }
        case 0x57u: { Operand o = op_zp_x(cpu, bus);  sre(cpu, bus, o); return 6u; }
        case 0x4Fu: { Operand o = op_abs(cpu, bus);   sre(cpu, bus, o); return 6u; }
        case 0x5Fu: { Operand o = op_abs_x(cpu, bus); sre(cpu, bus, o); return 7u; }
        case 0x5Bu: { Operand o = op_abs_y(cpu, bus); sre(cpu, bus, o); return 7u; }
        case 0x43u: { Operand o = op_ind_x(cpu, bus); sre(cpu, bus, o); return 8u; }
        case 0x53u: { Operand o = op_ind_y(cpu, bus); sre(cpu, bus, o); return 8u; }

        /* ---- RRA (ROR then ADC) ---- */
        case 0x67u: { Operand o = op_zp(cpu, bus);    rra(cpu, bus, o); return 5u; }
        case 0x77u: { Operand o = op_zp_x(cpu, bus);  rra(cpu, bus, o); return 6u; }
        case 0x6Fu: { Operand o = op_abs(cpu, bus);   rra(cpu, bus, o); return 6u; }
        case 0x7Fu: { Operand o = op_abs_x(cpu, bus); rra(cpu, bus, o); return 7u; }
        case 0x7Bu: { Operand o = op_abs_y(cpu, bus); rra(cpu, bus, o); return 7u; }
        case 0x63u: { Operand o = op_ind_x(cpu, bus); rra(cpu, bus, o); return 8u; }
        case 0x73u: { Operand o = op_ind_y(cpu, bus); rra(cpu, bus, o); return 8u; }

        /* ---- TAS / SHS ---- */
        case 0x9Bu: {
            uint16_t base = cpu_fetch_word(cpu, bus);
            tas_store(cpu, bus, base);
            return 5u;
        }

        /* ---- AHX / SHA ---- */
        case 0x9Fu: {
            uint16_t base = cpu_fetch_word(cpu, bus);
            ahx_store(cpu, bus, base, (uint8_t)(cpu->a & cpu->x));
            return 5u;
        }
        case 0x93u: {
            uint16_t base = cpu_am_indirect_y_base(cpu, bus);
            ahx_store(cpu, bus, base, (uint8_t)(cpu->a & cpu->x));
            return 6u;
        }

        /* ---- SHX / SXA ---- */
        case 0x9Eu: {
            uint16_t base = cpu_fetch_word(cpu, bus);
            ahx_store(cpu, bus, base, cpu->x);
            return 5u;
        }

        /* ---- SHY / SYA ---- */
        case 0x9Cu: {
            uint16_t base = cpu_fetch_word(cpu, bus);
            shy_store(cpu, bus, base);
            return 5u;
        }

        /* Defensive fallback: 2-cycle NOP (unreachable in practice — every
         * byte 0x00..=0xFF is covered by official + unofficial sets). */
        default:
            return 2u;
    }
}

/* ---- execute_unofficial (unofficial.rs `execute_unofficial`) --------- */

uint8_t cpu_execute_unofficial(Cpu* cpu, Bus* bus, uint8_t opcode) {
    bool pc; /* page-cross scratch */
    switch (opcode) {
        /* ---- NOP variants ---- */
        /* Implied NOPs (2 cycles): 0x1A, 0x3A, 0x5A, 0x7A, 0xDA, 0xFA. */
        case 0x1Au: case 0x3Au: case 0x5Au: case 0x7Au: case 0xDAu: case 0xFAu:
            return 2u;
        /* Immediate NOPs (2 cycles): 0x80, 0x82, 0x89, 0xC2, 0xE2. */
        case 0x80u: case 0x82u: case 0x89u: case 0xC2u: case 0xE2u:
            (void)cpu_fetch_byte(cpu, bus);
            return 2u;
        /* Zero-page NOPs (3 cycles): 0x04, 0x44, 0x64. */
        case 0x04u: case 0x44u: case 0x64u:
            (void)cpu_fetch_byte(cpu, bus);
            return 3u;
        /* Zero-page,X NOPs (4 cycles): 0x14, 0x34, 0x54, 0x74, 0xD4, 0xF4. */
        case 0x14u: case 0x34u: case 0x54u: case 0x74u: case 0xD4u: case 0xF4u:
            (void)cpu_am_zero_page_x(cpu, bus, DUMMY_RMW);
            return 4u;
        /* Absolute NOP (4 cycles): 0x0C. */
        case 0x0Cu:
            (void)cpu_fetch_word(cpu, bus);
            return 4u;
        /* Absolute,X NOPs (4 + page-cross): 0x1C, 0x3C, 0x5C, 0x7C, 0xDC, 0xFC.
         * Read-style NOPs: dummy read on page cross, +1 penalty. */
        case 0x1Cu: case 0x3Cu: case 0x5Cu: case 0x7Cu: case 0xDCu: case 0xFCu: {
            (void)cpu_am_absolute_x(cpu, bus, DUMMY_READ, &pc);
            return (uint8_t)(4u + (pc ? 1u : 0u));
        }

        /* ---- LAX (load A and X) ---- */
        case 0xA7u: { uint8_t v = rd_zp(cpu, bus);           lax(cpu, v); return 3u; }
        case 0xB7u: { uint8_t v = rd_zp_y(cpu, bus);         lax(cpu, v); return 4u; }
        case 0xAFu: { uint8_t v = rd_abs(cpu, bus);          lax(cpu, v); return 4u; }
        case 0xBFu: { uint8_t v = rd_abs_y(cpu, bus, &pc);   lax(cpu, v); return (uint8_t)(4u + (pc ? 1u : 0u)); }
        case 0xA3u: { uint8_t v = rd_ind_x(cpu, bus);        lax(cpu, v); return 6u; }
        case 0xB3u: { uint8_t v = rd_ind_y(cpu, bus, &pc);   lax(cpu, v); return (uint8_t)(5u + (pc ? 1u : 0u)); }

        /* ---- SAX (store A & X) ---- */
        case 0x87u: { wr_zp(cpu, bus, (uint8_t)(cpu->a & cpu->x));    return 3u; }
        case 0x97u: { wr_zp_y(cpu, bus, (uint8_t)(cpu->a & cpu->x));  return 4u; }
        case 0x8Fu: { wr_abs(cpu, bus, (uint8_t)(cpu->a & cpu->x));   return 4u; }
        case 0x83u: { wr_ind_x(cpu, bus, (uint8_t)(cpu->a & cpu->x)); return 6u; }

        /* ---- ANC (AND then copy N to C) — 0x0B and 0x2B ---- */
        case 0x0Bu: case 0x2Bu: { uint8_t v = rd_imm(cpu, bus); anc(cpu, v); return 2u; }

        /* ---- ALR (AND then LSR) ---- */
        case 0x4Bu: { uint8_t v = rd_imm(cpu, bus); alr(cpu, v); return 2u; }

        /* ---- ARR (AND then ROR, special V/C) ---- */
        case 0x6Bu: { uint8_t v = rd_imm(cpu, bus); arr(cpu, v); return 2u; }

        /* ---- AXS / SBX ---- */
        case 0xCBu: { uint8_t v = rd_imm(cpu, bus); axs(cpu, v); return 2u; }

        /* ---- XAA (unstable) ---- */
        case 0x8Bu: { uint8_t v = rd_imm(cpu, bus); xaa(cpu, v); return 2u; }

        /* ---- LAS / LAR (A = X = SP = M & SP) ---- */
        case 0xBBu: {
            uint8_t v = rd_abs_y(cpu, bus, &pc);
            uint8_t r = (uint8_t)(v & cpu->sp);
            cpu->a = r;
            cpu->x = r;
            cpu->sp = r;
            cpu_set_nz(cpu, r);
            return (uint8_t)(4u + (pc ? 1u : 0u));
        }

        /* ---- KIL / JAM / HLT — halt the CPU ---- */
        case 0x02u: case 0x12u: case 0x22u: case 0x32u:
        case 0x42u: case 0x52u: case 0x62u: case 0x72u:
        case 0x92u: case 0xB2u: case 0xD2u: case 0xF2u:
            cpu_set_halted(cpu, true);
            return 1u;

        /* ---- RMW-combo + unstable stores: delegate to sibling dispatch ---- */
        default:
            return cpu_execute_unofficial_rmw(cpu, bus, opcode);
    }
}

