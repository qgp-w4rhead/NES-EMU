/*
 * cpu.c — 6502 CPU core: step/reset/nmi/irq, stack, operand helpers, and the
 * full official-opcode dispatch (all 151 opcodes).
 *
 * Port of src/cpu/mod.rs + src/cpu/opcodes.rs to C (M4.1). Behaviour is
 * byte-exact with the Rust source:
 *  - flag macros, vectors, packed internal flags byte (mod.rs `flags`/`vectors`)
 *  - reset/nmi/irq/step (mod.rs)
 *  - fetch/push/pull/push_status/pull_status (mod.rs)
 *  - read_operand/write_operand (mod.rs)
 *  - execute dispatch + all official opcode handlers (opcodes.rs)
 *  - cycle counts incl. page-cross penalties, RMW dummy reads, branch +1/+1
 *
 * Unofficial opcodes (unofficial.rs / unofficial_rmw.rs / unofficial_special.rs)
 * live in cpu_unofficial.c and are reached via the default arm of the dispatch
 * switch, which calls cpu_execute_unofficial.
 */
#include "cpu.h"

/* ---- Construction (mod.rs `Cpu::new`) -------------------------------- */

void cpu_init(Cpu* cpu) {
    cpu->a = 0u;
    cpu->x = 0u;
    cpu->y = 0u;
    cpu->sp = 0xFDu;
    cpu->pc = 0u;
    cpu->status = CPU_FLAG_U | CPU_FLAG_I;
    cpu->flags = 0u;
}

/* ---- Flag helpers (mod.rs) ------------------------------------------- */

void cpu_set_flag(Cpu* c, uint8_t flag, bool v) {
    if (v) {
        c->status = (uint8_t)(c->status | flag);
    } else {
        c->status = (uint8_t)(c->status & (uint8_t)~flag);
    }
}

void cpu_set_carry(Cpu* c, bool v)            { cpu_set_flag(c, CPU_FLAG_C, v); }
void cpu_set_zero(Cpu* c, bool v)             { cpu_set_flag(c, CPU_FLAG_Z, v); }
void cpu_set_interrupt_disable(Cpu* c, bool v){ cpu_set_flag(c, CPU_FLAG_I, v); }
void cpu_set_decimal(Cpu* c, bool v)          { cpu_set_flag(c, CPU_FLAG_D, v); }
void cpu_set_overflow(Cpu* c, bool v)         { cpu_set_flag(c, CPU_FLAG_V, v); }
void cpu_set_negative(Cpu* c, bool v)         { cpu_set_flag(c, CPU_FLAG_N, v); }

void cpu_set_nz(Cpu* c, uint8_t value) {
    cpu_set_zero(c, value == 0u);
    cpu_set_negative(c, (value & 0x80u) != 0u);
}

/* ---- Fetch helpers (mod.rs) ------------------------------------------ */

uint8_t cpu_fetch_byte(Cpu* cpu, Bus* bus) {
    uint8_t b = bus_read(bus, cpu->pc);
    cpu->pc = (uint16_t)(cpu->pc + 1u);
    return b;
}

uint16_t cpu_fetch_word(Cpu* cpu, Bus* bus) {
    uint16_t lo = (uint16_t)cpu_fetch_byte(cpu, bus);
    uint16_t hi = (uint16_t)cpu_fetch_byte(cpu, bus);
    return (uint16_t)(lo | (hi << 8));
}

/* ---- Stack access (mod.rs) ------------------------------------------- */

void cpu_push(Cpu* cpu, Bus* bus, uint8_t value) {
    uint16_t addr = (uint16_t)(0x0100u | (uint16_t)cpu->sp);
    bus_write(bus, addr, value);
    cpu->sp = (uint8_t)(cpu->sp - 1u);
}

uint8_t cpu_pull(Cpu* cpu, Bus* bus) {
    cpu->sp = (uint8_t)(cpu->sp + 1u);
    uint16_t addr = (uint16_t)(0x0100u | (uint16_t)cpu->sp);
    return bus_read(bus, addr);
}

void cpu_push_pc(Cpu* cpu, Bus* bus, uint16_t pc) {
    cpu_push(cpu, bus, (uint8_t)(pc >> 8));
    cpu_push(cpu, bus, (uint8_t)(pc & 0xFFu));
}

uint16_t cpu_pull_pc(Cpu* cpu, Bus* bus) {
    uint16_t lo = (uint16_t)cpu_pull(cpu, bus);
    uint16_t hi = (uint16_t)cpu_pull(cpu, bus);
    return (uint16_t)(lo | (hi << 8));
}

void cpu_push_status(Cpu* cpu, Bus* bus, bool with_break) {
    uint8_t p = (uint8_t)(cpu->status | CPU_FLAG_U);
    if (with_break) {
        p = (uint8_t)(p | CPU_FLAG_B);
    } else {
        p = (uint8_t)(p & (uint8_t)~CPU_FLAG_B);
    }
    cpu_push(cpu, bus, p);
}

void cpu_pull_status(Cpu* cpu, Bus* bus) {
    uint8_t p = cpu_pull(cpu, bus);
    cpu->status = (uint8_t)((p & (uint8_t)~CPU_FLAG_B) | CPU_FLAG_U);
}

/* ---- Operand read/write helpers (mod.rs) ----------------------------- */

uint8_t cpu_read_operand(Cpu* cpu, Bus* bus, Operand op) {
    switch (op.tag) {
        case OPERAND_NONE: return 0u;
        case OPERAND_ACC:  return cpu->a;
        case OPERAND_ADDR: return bus_read(bus, op.addr);
        default:           return 0u;
    }
}

void cpu_write_operand(Cpu* cpu, Bus* bus, Operand op, uint8_t value) {
    switch (op.tag) {
        case OPERAND_NONE: return;
        case OPERAND_ACC:  cpu->a = value; return;
        case OPERAND_ADDR: bus_write(bus, op.addr, value); return;
        default:           return;
    }
}

/* ---- Interrupt service (mod.rs) -------------------------------------- */

static void service_interrupt(Cpu* cpu, Bus* bus, uint16_t vector) {
    cpu_push_pc(cpu, bus, cpu->pc);
    /* B cleared for hardware interrupts; U always set in pushed copy. */
    cpu_push_status(cpu, bus, false);
    cpu_set_interrupt_disable(cpu, true);
    cpu->pc = cpu_read_vector(cpu, bus, vector);
}

void cpu_nmi(Cpu* cpu, Bus* bus) {
    service_interrupt(cpu, bus, CPU_VECTOR_NMI);
}

void cpu_irq(Cpu* cpu, Bus* bus) {
    service_interrupt(cpu, bus, CPU_VECTOR_IRQ);
}

void cpu_reset(Cpu* cpu, Bus* bus) {
    /* 6502 RESET: SP=$FD, I set, U set, PC from $FFFC/$FFFD, clear HALTED. */
    cpu->sp = 0xFDu;
    cpu_set_interrupt_disable(cpu, true);
    cpu->status = (uint8_t)(cpu->status | CPU_FLAG_U);
    cpu->pc = cpu_read_vector(cpu, bus, CPU_VECTOR_RESET);
    cpu->flags = (uint8_t)(cpu->flags & (uint8_t)~CPU_HALTED);
}

/* ---- step (mod.rs `step`) -------------------------------------------- */

uint8_t cpu_step(Cpu* cpu, Bus* bus) {
    /* KIL/JAM halt: frozen; return 1 cycle so the emulator loop still
     * advances the PPU/APU. Only RESET revives the CPU. */
    if ((cpu->flags & CPU_HALTED) != 0u) {
        return 1u;
    }
    /* NMI is non-maskable and always wins over IRQ. */
    if ((cpu->flags & CPU_NMI_PENDING) != 0u) {
        cpu->flags = (uint8_t)(cpu->flags & (uint8_t)~CPU_NMI_PENDING);
        cpu_nmi(cpu, bus);
        return 7u;
    }
    /* IRQ is masked by the I flag. */
    if ((cpu->flags & CPU_IRQ_PENDING) != 0u && !cpu_interrupt_disable(cpu)) {
        cpu->flags = (uint8_t)(cpu->flags & (uint8_t)~CPU_IRQ_PENDING);
        cpu_irq(cpu, bus);
        return 7u;
    }
    uint8_t opcode = cpu_fetch_byte(cpu, bus);
    return cpu_execute(cpu, bus, opcode);
}

/* ---- Halt / interrupt pending accessors (mod.rs) --------------------- */

bool cpu_is_halted(const Cpu* c)         { return (c->flags & CPU_HALTED) != 0u; }
void cpu_set_halted(Cpu* c, bool v) {
    /* HALTED lives in the packed `flags` byte (NMI/IRQ/HALT), NOT in the P
     * register `status`. cpu_set_flag() would write to `status` and corrupt
     * the I flag (CPU_HALTED == 0x04 == CPU_FLAG_I). Match mod.rs:307-313. */
    if (v) c->flags = (uint8_t)(c->flags | CPU_HALTED);
    else   c->flags = (uint8_t)(c->flags & (uint8_t)~CPU_HALTED);
}
bool cpu_nmi_pending(const Cpu* c)       { return (c->flags & CPU_NMI_PENDING) != 0u; }
bool cpu_irq_pending(const Cpu* c)       { return (c->flags & CPU_IRQ_PENDING) != 0u; }

void cpu_set_nmi_pending(Cpu* c, bool v) {
    if (v) c->flags = (uint8_t)(c->flags | CPU_NMI_PENDING);
    else   c->flags = (uint8_t)(c->flags & (uint8_t)~CPU_NMI_PENDING);
}
void cpu_set_irq_pending(Cpu* c, bool v) {
    if (v) c->flags = (uint8_t)(c->flags | CPU_IRQ_PENDING);
    else   c->flags = (uint8_t)(c->flags & (uint8_t)~CPU_IRQ_PENDING);
}

/* ---- read_vector (opcodes.rs) ---------------------------------------- */

uint16_t cpu_read_vector(const Cpu* cpu, Bus* bus, uint16_t addr) {
    /* const_cast: bus_read takes a mutable Bus because reads can have side
     * effects on PPU/mapper registers. The Cpu pointer is const but the Bus
     * is not, so this is safe. */
    uint16_t lo = (uint16_t)bus_read(bus, addr);
    uint16_t hi = (uint16_t)bus_read(bus, (uint16_t)(addr + 1u));
    return (uint16_t)(lo | (hi << 8));
}

/* ===================================================================== */
/* ---- opcodes.rs: addressing + read/write/RMW helpers ----------------- */
/* ===================================================================== */

/* Value-transform function pointer type: takes Cpu + value, returns new value.
 * Shared with cpu_unofficial.c (non-static). */
typedef uint8_t (*ValueFn)(Cpu*, uint8_t);

/* rd_imm: immediate read = fetch_byte. */
uint8_t rd_imm(Cpu* cpu, Bus* bus) {
    return cpu_fetch_byte(cpu, bus);
}
/* rd_zp: zero-page read. */
uint8_t rd_zp(Cpu* cpu, Bus* bus) {
    Operand op = cpu_resolve(cpu, bus, ADDR_MODE_ZERO_PAGE);
    return cpu_read_operand(cpu, bus, op);
}
/* rd_zp_x: zero-page,X read (uses Dummy::Rmw per opcodes.rs). */
uint8_t rd_zp_x(Cpu* cpu, Bus* bus) {
    uint16_t a = cpu_am_zero_page_x(cpu, bus, DUMMY_RMW);
    return bus_read(bus, a);
}
/* rd_zp_y: zero-page,Y read (uses Dummy::Rmw per opcodes.rs). */
uint8_t rd_zp_y(Cpu* cpu, Bus* bus) {
    uint16_t a = cpu_am_zero_page_y(cpu, bus, DUMMY_RMW);
    return bus_read(bus, a);
}
/* rd_abs: absolute read. */
uint8_t rd_abs(Cpu* cpu, Bus* bus) {
    Operand op = cpu_resolve(cpu, bus, ADDR_MODE_ABSOLUTE);
    return cpu_read_operand(cpu, bus, op);
}
/* rd_abs_x: absolute,X read (Dummy::Read -> page-cross dummy). Returns (val, page_cross). */
uint8_t rd_abs_x(Cpu* cpu, Bus* bus, bool* page_cross) {
    uint16_t a = cpu_am_absolute_x(cpu, bus, DUMMY_READ, page_cross);
    return bus_read(bus, a);
}
/* rd_abs_y: absolute,Y read (Dummy::Read -> page-cross dummy). Returns (val, page_cross). */
uint8_t rd_abs_y(Cpu* cpu, Bus* bus, bool* page_cross) {
    uint16_t a = cpu_am_absolute_y(cpu, bus, DUMMY_READ, page_cross);
    return bus_read(bus, a);
}
/* rd_ind_x: (ind,X) read. */
uint8_t rd_ind_x(Cpu* cpu, Bus* bus) {
    uint16_t a = cpu_am_indirect_x(cpu, bus);
    return bus_read(bus, a);
}
/* rd_ind_y: (ind),Y read (Dummy::Read -> page-cross dummy). Returns (val, page_cross). */
uint8_t rd_ind_y(Cpu* cpu, Bus* bus, bool* page_cross) {
    uint16_t a = cpu_am_indirect_y(cpu, bus, DUMMY_READ, page_cross);
    return bus_read(bus, a);
}

/* ---- write helpers (opcodes.rs) -------------------------------------- */

void wr_zp(Cpu* cpu, Bus* bus, uint8_t v) {
    Operand op = cpu_resolve(cpu, bus, ADDR_MODE_ZERO_PAGE);
    cpu_write_operand(cpu, bus, op, v);
}
void wr_zp_x(Cpu* cpu, Bus* bus, uint8_t v) {
    uint16_t a = cpu_am_zero_page_x(cpu, bus, DUMMY_RMW);
    bus_write(bus, a, v);
}
void wr_zp_y(Cpu* cpu, Bus* bus, uint8_t v) {
    uint16_t a = cpu_am_zero_page_y(cpu, bus, DUMMY_RMW);
    bus_write(bus, a, v);
}
void wr_abs(Cpu* cpu, Bus* bus, uint8_t v) {
    Operand op = cpu_resolve(cpu, bus, ADDR_MODE_ABSOLUTE);
    cpu_write_operand(cpu, bus, op, v);
}
void wr_abs_x(Cpu* cpu, Bus* bus, uint8_t v) {
    bool pc;
    uint16_t a = cpu_am_absolute_x(cpu, bus, DUMMY_RMW, &pc);
    (void)pc;
    bus_write(bus, a, v);
}
void wr_abs_y(Cpu* cpu, Bus* bus, uint8_t v) {
    bool pc;
    uint16_t a = cpu_am_absolute_y(cpu, bus, DUMMY_RMW, &pc);
    (void)pc;
    bus_write(bus, a, v);
}
void wr_ind_x(Cpu* cpu, Bus* bus, uint8_t v) {
    uint16_t a = cpu_am_indirect_x(cpu, bus);
    bus_write(bus, a, v);
}
void wr_ind_y(Cpu* cpu, Bus* bus, uint8_t v) {
    bool pc;
    uint16_t a = cpu_am_indirect_y(cpu, bus, DUMMY_RMW, &pc);
    (void)pc;
    bus_write(bus, a, v);
}

/* ---- RMW helpers (opcodes.rs) ---------------------------------------- */
/* rmw: read operand, apply f, write back, set N/Z from new value. (opcodes.rs `rmw`.) */
static void rmw(Cpu* cpu, Bus* bus, Operand op, ValueFn f) {
    uint8_t v   = cpu_read_operand(cpu, bus, op);
    uint8_t neu = f(cpu, v);
    cpu_write_operand(cpu, bus, op, neu);
    cpu_set_nz(cpu, neu);
}

static void rmw_zp(Cpu* cpu, Bus* bus, ValueFn f) {
    Operand op = cpu_resolve(cpu, bus, ADDR_MODE_ZERO_PAGE);
    rmw(cpu, bus, op, f);
}
static void rmw_zp_x(Cpu* cpu, Bus* bus, ValueFn f) {
    uint16_t a = cpu_am_zero_page_x(cpu, bus, DUMMY_RMW);
    rmw(cpu, bus, operand_address(a), f);
}
static void rmw_abs(Cpu* cpu, Bus* bus, ValueFn f) {
    Operand op = cpu_resolve(cpu, bus, ADDR_MODE_ABSOLUTE);
    rmw(cpu, bus, op, f);
}
static void rmw_abs_x(Cpu* cpu, Bus* bus, ValueFn f) {
    bool pc;
    uint16_t a = cpu_am_absolute_x(cpu, bus, DUMMY_RMW, &pc);
    (void)pc;
    rmw(cpu, bus, operand_address(a), f);
}

/* ---- operand-resolving helpers for unofficial combo opcodes (opcodes.rs) ---- */

Operand op_zp(Cpu* cpu, Bus* bus) {
    return cpu_resolve(cpu, bus, ADDR_MODE_ZERO_PAGE);
}
Operand op_zp_x(Cpu* cpu, Bus* bus) {
    return operand_address(cpu_am_zero_page_x(cpu, bus, DUMMY_RMW));
}
Operand op_abs(Cpu* cpu, Bus* bus) {
    return cpu_resolve(cpu, bus, ADDR_MODE_ABSOLUTE);
}
Operand op_abs_x(Cpu* cpu, Bus* bus) {
    bool pc;
    uint16_t a = cpu_am_absolute_x(cpu, bus, DUMMY_RMW, &pc);
    (void)pc;
    return operand_address(a);
}
Operand op_abs_y(Cpu* cpu, Bus* bus) {
    bool pc;
    uint16_t a = cpu_am_absolute_y(cpu, bus, DUMMY_RMW, &pc);
    (void)pc;
    return operand_address(a);
}
Operand op_ind_x(Cpu* cpu, Bus* bus) {
    return operand_address(cpu_am_indirect_x(cpu, bus));
}
Operand op_ind_y(Cpu* cpu, Bus* bus) {
    bool pc;
    uint16_t a = cpu_am_indirect_y(cpu, bus, DUMMY_RMW, &pc);
    (void)pc;
    return operand_address(a);
}

/* ---- load / store / transfer handlers (opcodes.rs) ------------------- */

static void lda(Cpu* cpu, uint8_t v) { cpu->a = v; cpu_set_nz(cpu, v); }
static void ldx(Cpu* cpu, uint8_t v) { cpu->x = v; cpu_set_nz(cpu, v); }
static void ldy(Cpu* cpu, uint8_t v) { cpu->y = v; cpu_set_nz(cpu, v); }
static void tax(Cpu* cpu) { cpu->x = cpu->a; cpu_set_nz(cpu, cpu->x); }
static void tay(Cpu* cpu) { cpu->y = cpu->a; cpu_set_nz(cpu, cpu->y); }
static void txa(Cpu* cpu) { cpu->a = cpu->x; cpu_set_nz(cpu, cpu->a); }
static void tya(Cpu* cpu) { cpu->a = cpu->y; cpu_set_nz(cpu, cpu->a); }
static void tsx(Cpu* cpu) { cpu->x = cpu->sp; cpu_set_nz(cpu, cpu->x); }
/* TXS — does NOT update flags. */
static void set_sp_from_x(Cpu* cpu) { cpu->sp = cpu->x; }

/* ---- logic handlers (opcodes.rs) ------------------------------------- */

static void and_(Cpu* cpu, uint8_t v) { cpu->a = (uint8_t)(cpu->a & v); cpu_set_nz(cpu, cpu->a); }
static void ora_(Cpu* cpu, uint8_t v) { cpu->a = (uint8_t)(cpu->a | v); cpu_set_nz(cpu, cpu->a); }
static void eor_(Cpu* cpu, uint8_t v) { cpu->a = (uint8_t)(cpu->a ^ v); cpu_set_nz(cpu, cpu->a); }

/* BIT — Z = (A & M) == 0; N = bit 7 of M; V = bit 6 of M. A unchanged. */
static void bit_(Cpu* cpu, uint8_t m) {
    uint8_t result = (uint8_t)(cpu->a & m);
    cpu_set_zero(cpu, result == 0u);
    cpu_set_negative(cpu, (m & 0x80u) != 0u);
    cpu_set_overflow(cpu, (m & 0x40u) != 0u);
}

/* ---- arithmetic (opcodes.rs) ----------------------------------------- */

/* ADC — binary add (no decimal mode on 2A03). Overflow on signed overflow. */
static void adc(Cpu* cpu, uint8_t m) {
    uint16_t a = (uint16_t)cpu->a;
    uint16_t mm = (uint16_t)m;
    uint16_t c = cpu_carry(cpu) ? 1u : 0u;
    uint16_t sum = (uint16_t)(a + mm + c);
    cpu_set_carry(cpu, sum > 0xFFu);
    uint8_t result = (uint8_t)(sum & 0xFFu);
    /* Overflow: same-sign operands, different-sign result. */
    cpu_set_overflow(cpu, (((a ^ mm) & 0x80u) == 0u) && (((a ^ sum) & 0x80u) != 0u));
    cpu->a = result;
    cpu_set_nz(cpu, result);
}

/* SBC — A + ~M + C, sharing ADC overflow/carry logic. Carry = no borrow. */
static void sbc(Cpu* cpu, uint8_t m) {
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

/* ---- compare (opcodes.rs) -------------------------------------------- */

/* CMP/CPX/CPY: C = (r >= m), Z = (r == m), N = bit7 of (r - m). Register unchanged. */
static void cmp_(Cpu* cpu, uint8_t r, uint8_t m) {
    uint8_t diff = (uint8_t)(r - m);
    cpu_set_carry(cpu, r >= m);
    cpu_set_nz(cpu, diff);
}

/* ---- shifts / rotates / inc / dec value transforms (opcodes.rs) ------ */

static uint8_t asl_value(Cpu* cpu, uint8_t v) { cpu_set_carry(cpu, (v & 0x80u) != 0u); return (uint8_t)(v << 1); }
static uint8_t lsr_value(Cpu* cpu, uint8_t v) { cpu_set_carry(cpu, (v & 0x01u) != 0u); return (uint8_t)(v >> 1); }
static uint8_t rol_value(Cpu* cpu, uint8_t v) {
    bool new_c = (v & 0x80u) != 0u;
    uint8_t result = (uint8_t)((v << 1) | (cpu_carry(cpu) ? 1u : 0u));
    cpu_set_carry(cpu, new_c);
    return result;
}
static uint8_t ror_value(Cpu* cpu, uint8_t v) {
    bool new_c = (v & 0x01u) != 0u;
    uint8_t result = (uint8_t)((v >> 1) | (cpu_carry(cpu) ? 0x80u : 0x00u));
    cpu_set_carry(cpu, new_c);
    return result;
}
static uint8_t inc_value(Cpu* cpu, uint8_t v) { (void)cpu; return (uint8_t)(v + 1u); }
static uint8_t dec_value(Cpu* cpu, uint8_t v) { (void)cpu; return (uint8_t)(v - 1u); }

/* ---- branches (opcodes.rs `branch`) ---------------------------------- */

/* Execute a branch. branch_on_set selects taken-when-set vs taken-when-clear.
 * Returns 2 base, +1 if taken, +1 if taken and target on a different page
 * than the instruction following the branch. */
static uint8_t branch(Cpu* cpu, Bus* bus, bool branch_on_set, uint8_t cond_flag) {
    bool flag_set = (cpu->status & cond_flag) != 0u;
    bool take = (flag_set == branch_on_set);
    if (!take) {
        (void)cpu_fetch_byte(cpu, bus); /* still consume the offset byte */
        return 2u;
    }
    uint16_t pc_before = cpu->pc;
    uint16_t target = cpu_am_relative(cpu, bus);
    uint16_t pc_after = (uint16_t)(pc_before + 1u); /* PC after fetching offset */
    bool page_cross = (pc_after & 0xFF00u) != (target & 0xFF00u);
    cpu->pc = target;
    return (uint8_t)(2u + 1u + (page_cross ? 1u : 0u));
}

/* ===================================================================== */
/* ---- execute dispatch (opcodes.rs `execute`) ------------------------- */
/* ===================================================================== */

uint8_t cpu_execute(Cpu* cpu, Bus* bus, uint8_t opcode) {
    bool pc; /* page-cross flag scratch */
    switch (opcode) {
        /* ---- LDA ---- */
        case 0xA9u: { uint8_t v = rd_imm(cpu, bus);           lda(cpu, v); return 2u; }
        case 0xA5u: { uint8_t v = rd_zp(cpu, bus);            lda(cpu, v); return 3u; }
        case 0xB5u: { uint8_t v = rd_zp_x(cpu, bus);          lda(cpu, v); return 4u; }
        case 0xADu: { uint8_t v = rd_abs(cpu, bus);           lda(cpu, v); return 4u; }
        case 0xBDu: { uint8_t v = rd_abs_x(cpu, bus, &pc);    lda(cpu, v); return (uint8_t)(4u + (pc ? 1u : 0u)); }
        case 0xB9u: { uint8_t v = rd_abs_y(cpu, bus, &pc);    lda(cpu, v); return (uint8_t)(4u + (pc ? 1u : 0u)); }
        case 0xA1u: { uint8_t v = rd_ind_x(cpu, bus);         lda(cpu, v); return 6u; }
        case 0xB1u: { uint8_t v = rd_ind_y(cpu, bus, &pc);    lda(cpu, v); return (uint8_t)(5u + (pc ? 1u : 0u)); }

        /* ---- LDX ---- */
        case 0xA2u: { uint8_t v = rd_imm(cpu, bus);           ldx(cpu, v); return 2u; }
        case 0xA6u: { uint8_t v = rd_zp(cpu, bus);            ldx(cpu, v); return 3u; }
        case 0xB6u: { uint8_t v = rd_zp_y(cpu, bus);          ldx(cpu, v); return 4u; }
        case 0xAEu: { uint8_t v = rd_abs(cpu, bus);           ldx(cpu, v); return 4u; }
        case 0xBEu: { uint8_t v = rd_abs_y(cpu, bus, &pc);    ldx(cpu, v); return (uint8_t)(4u + (pc ? 1u : 0u)); }

        /* ---- LDY ---- */
        case 0xA0u: { uint8_t v = rd_imm(cpu, bus);           ldy(cpu, v); return 2u; }
        case 0xA4u: { uint8_t v = rd_zp(cpu, bus);            ldy(cpu, v); return 3u; }
        case 0xB4u: { uint8_t v = rd_zp_x(cpu, bus);          ldy(cpu, v); return 4u; }
        case 0xACu: { uint8_t v = rd_abs(cpu, bus);           ldy(cpu, v); return 4u; }
        case 0xBCu: { uint8_t v = rd_abs_x(cpu, bus, &pc);    ldy(cpu, v); return (uint8_t)(4u + (pc ? 1u : 0u)); }

        /* ---- STA ---- */
        case 0x85u: { wr_zp(cpu, bus, cpu->a);    return 3u; }
        case 0x95u: { wr_zp_x(cpu, bus, cpu->a);  return 4u; }
        case 0x8Du: { wr_abs(cpu, bus, cpu->a);   return 4u; }
        case 0x9Du: { wr_abs_x(cpu, bus, cpu->a); return 5u; }
        case 0x99u: { wr_abs_y(cpu, bus, cpu->a); return 5u; }
        case 0x81u: { wr_ind_x(cpu, bus, cpu->a); return 6u; }
        case 0x91u: { wr_ind_y(cpu, bus, cpu->a); return 6u; }

        /* ---- STX ---- */
        case 0x86u: { wr_zp(cpu, bus, cpu->x);    return 3u; }
        case 0x96u: { wr_zp_y(cpu, bus, cpu->x);  return 4u; }
        case 0x8Eu: { wr_abs(cpu, bus, cpu->x);   return 4u; }

        /* ---- STY ---- */
        case 0x84u: { wr_zp(cpu, bus, cpu->y);    return 3u; }
        case 0x94u: { wr_zp_x(cpu, bus, cpu->y);  return 4u; }
        case 0x8Cu: { wr_abs(cpu, bus, cpu->y);   return 4u; }

        /* ---- register transfers ---- */
        case 0xAAu: { tax(cpu);          return 2u; } /* TAX */
        case 0xA8u: { tay(cpu);          return 2u; } /* TAY */
        case 0x8Au: { txa(cpu);          return 2u; } /* TXA */
        case 0x98u: { tya(cpu);          return 2u; } /* TYA */
        case 0xBAu: { tsx(cpu);          return 2u; } /* TSX */
        case 0x9Au: { set_sp_from_x(cpu);return 2u; } /* TXS (no flags) */

        /* ---- stack ---- */
        case 0x48u: { cpu_push(cpu, bus, cpu->a);      return 3u; } /* PHA */
        case 0x08u: { cpu_push_status(cpu, bus, true); return 3u; } /* PHP (B set) */
        case 0x68u: { uint8_t v = cpu_pull(cpu, bus); lda(cpu, v); return 4u; } /* PLA */
        case 0x28u: { cpu_pull_status(cpu, bus);    return 4u; } /* PLP */

        /* ---- AND ---- */
        case 0x29u: { uint8_t v = rd_imm(cpu, bus);        and_(cpu, v); return 2u; }
        case 0x25u: { uint8_t v = rd_zp(cpu, bus);         and_(cpu, v); return 3u; }
        case 0x35u: { uint8_t v = rd_zp_x(cpu, bus);       and_(cpu, v); return 4u; }
        case 0x2Du: { uint8_t v = rd_abs(cpu, bus);        and_(cpu, v); return 4u; }
        case 0x3Du: { uint8_t v = rd_abs_x(cpu, bus, &pc); and_(cpu, v); return (uint8_t)(4u + (pc ? 1u : 0u)); }
        case 0x39u: { uint8_t v = rd_abs_y(cpu, bus, &pc); and_(cpu, v); return (uint8_t)(4u + (pc ? 1u : 0u)); }
        case 0x21u: { uint8_t v = rd_ind_x(cpu, bus);      and_(cpu, v); return 6u; }
        case 0x31u: { uint8_t v = rd_ind_y(cpu, bus, &pc); and_(cpu, v); return (uint8_t)(5u + (pc ? 1u : 0u)); }

        /* ---- ORA ---- */
        case 0x09u: { uint8_t v = rd_imm(cpu, bus);        ora_(cpu, v); return 2u; }
        case 0x05u: { uint8_t v = rd_zp(cpu, bus);         ora_(cpu, v); return 3u; }
        case 0x15u: { uint8_t v = rd_zp_x(cpu, bus);       ora_(cpu, v); return 4u; }
        case 0x0Du: { uint8_t v = rd_abs(cpu, bus);        ora_(cpu, v); return 4u; }
        case 0x1Du: { uint8_t v = rd_abs_x(cpu, bus, &pc); ora_(cpu, v); return (uint8_t)(4u + (pc ? 1u : 0u)); }
        case 0x19u: { uint8_t v = rd_abs_y(cpu, bus, &pc); ora_(cpu, v); return (uint8_t)(4u + (pc ? 1u : 0u)); }
        case 0x01u: { uint8_t v = rd_ind_x(cpu, bus);      ora_(cpu, v); return 6u; }
        case 0x11u: { uint8_t v = rd_ind_y(cpu, bus, &pc); ora_(cpu, v); return (uint8_t)(5u + (pc ? 1u : 0u)); }

        /* ---- EOR ---- */
        case 0x49u: { uint8_t v = rd_imm(cpu, bus);        eor_(cpu, v); return 2u; }
        case 0x45u: { uint8_t v = rd_zp(cpu, bus);         eor_(cpu, v); return 3u; }
        case 0x55u: { uint8_t v = rd_zp_x(cpu, bus);       eor_(cpu, v); return 4u; }
        case 0x4Du: { uint8_t v = rd_abs(cpu, bus);        eor_(cpu, v); return 4u; }
        case 0x5Du: { uint8_t v = rd_abs_x(cpu, bus, &pc); eor_(cpu, v); return (uint8_t)(4u + (pc ? 1u : 0u)); }
        case 0x59u: { uint8_t v = rd_abs_y(cpu, bus, &pc); eor_(cpu, v); return (uint8_t)(4u + (pc ? 1u : 0u)); }
        case 0x41u: { uint8_t v = rd_ind_x(cpu, bus);      eor_(cpu, v); return 6u; }
        case 0x51u: { uint8_t v = rd_ind_y(cpu, bus, &pc); eor_(cpu, v); return (uint8_t)(5u + (pc ? 1u : 0u)); }

        /* ---- BIT ---- */
        case 0x24u: { uint8_t v = rd_zp(cpu, bus);  bit_(cpu, v); return 3u; }
        case 0x2Cu: { uint8_t v = rd_abs(cpu, bus); bit_(cpu, v); return 4u; }

        /* ---- ADC ---- */
        case 0x69u: { uint8_t v = rd_imm(cpu, bus);        adc(cpu, v); return 2u; }
        case 0x65u: { uint8_t v = rd_zp(cpu, bus);         adc(cpu, v); return 3u; }
        case 0x75u: { uint8_t v = rd_zp_x(cpu, bus);       adc(cpu, v); return 4u; }
        case 0x6Du: { uint8_t v = rd_abs(cpu, bus);        adc(cpu, v); return 4u; }
        case 0x7Du: { uint8_t v = rd_abs_x(cpu, bus, &pc); adc(cpu, v); return (uint8_t)(4u + (pc ? 1u : 0u)); }
        case 0x79u: { uint8_t v = rd_abs_y(cpu, bus, &pc); adc(cpu, v); return (uint8_t)(4u + (pc ? 1u : 0u)); }
        case 0x61u: { uint8_t v = rd_ind_x(cpu, bus);      adc(cpu, v); return 6u; }
        case 0x71u: { uint8_t v = rd_ind_y(cpu, bus, &pc); adc(cpu, v); return (uint8_t)(5u + (pc ? 1u : 0u)); }

        /* ---- SBC ---- */
        case 0xE9u: { uint8_t v = rd_imm(cpu, bus);        sbc(cpu, v); return 2u; }
        case 0xE5u: { uint8_t v = rd_zp(cpu, bus);         sbc(cpu, v); return 3u; }
        case 0xF5u: { uint8_t v = rd_zp_x(cpu, bus);       sbc(cpu, v); return 4u; }
        case 0xEDu: { uint8_t v = rd_abs(cpu, bus);        sbc(cpu, v); return 4u; }
        case 0xFDu: { uint8_t v = rd_abs_x(cpu, bus, &pc); sbc(cpu, v); return (uint8_t)(4u + (pc ? 1u : 0u)); }
        case 0xF9u: { uint8_t v = rd_abs_y(cpu, bus, &pc); sbc(cpu, v); return (uint8_t)(4u + (pc ? 1u : 0u)); }
        case 0xE1u: { uint8_t v = rd_ind_x(cpu, bus);      sbc(cpu, v); return 6u; }
        case 0xF1u: { uint8_t v = rd_ind_y(cpu, bus, &pc); sbc(cpu, v); return (uint8_t)(5u + (pc ? 1u : 0u)); }

        /* ---- CMP ---- */
        case 0xC9u: { uint8_t v = rd_imm(cpu, bus);        cmp_(cpu, cpu->a, v); return 2u; }
        case 0xC5u: { uint8_t v = rd_zp(cpu, bus);         cmp_(cpu, cpu->a, v); return 3u; }
        case 0xD5u: { uint8_t v = rd_zp_x(cpu, bus);       cmp_(cpu, cpu->a, v); return 4u; }
        case 0xCDu: { uint8_t v = rd_abs(cpu, bus);        cmp_(cpu, cpu->a, v); return 4u; }
        case 0xDDu: { uint8_t v = rd_abs_x(cpu, bus, &pc); cmp_(cpu, cpu->a, v); return (uint8_t)(4u + (pc ? 1u : 0u)); }
        case 0xD9u: { uint8_t v = rd_abs_y(cpu, bus, &pc); cmp_(cpu, cpu->a, v); return (uint8_t)(4u + (pc ? 1u : 0u)); }
        case 0xC1u: { uint8_t v = rd_ind_x(cpu, bus);      cmp_(cpu, cpu->a, v); return 6u; }
        case 0xD1u: { uint8_t v = rd_ind_y(cpu, bus, &pc); cmp_(cpu, cpu->a, v); return (uint8_t)(5u + (pc ? 1u : 0u)); }

        /* ---- CPX / CPY ---- */
        case 0xE0u: { uint8_t v = rd_imm(cpu, bus); cmp_(cpu, cpu->x, v); return 2u; }
        case 0xE4u: { uint8_t v = rd_zp(cpu, bus);  cmp_(cpu, cpu->x, v); return 3u; }
        case 0xECu: { uint8_t v = rd_abs(cpu, bus); cmp_(cpu, cpu->x, v); return 4u; }
        case 0xC0u: { uint8_t v = rd_imm(cpu, bus); cmp_(cpu, cpu->y, v); return 2u; }
        case 0xC4u: { uint8_t v = rd_zp(cpu, bus);  cmp_(cpu, cpu->y, v); return 3u; }
        case 0xCCu: { uint8_t v = rd_abs(cpu, bus); cmp_(cpu, cpu->y, v); return 4u; }

        /* ---- INC / DEC memory ---- */
        case 0xE6u: { rmw_zp(cpu, bus, inc_value);    return 5u; }
        case 0xF6u: { rmw_zp_x(cpu, bus, inc_value);  return 6u; }
        case 0xEEu: { rmw_abs(cpu, bus, inc_value);   return 6u; }
        case 0xFEu: { rmw_abs_x(cpu, bus, inc_value); return 7u; }
        case 0xC6u: { rmw_zp(cpu, bus, dec_value);    return 5u; }
        case 0xD6u: { rmw_zp_x(cpu, bus, dec_value);  return 6u; }
        case 0xCEu: { rmw_abs(cpu, bus, dec_value);   return 6u; }
        case 0xDEu: { rmw_abs_x(cpu, bus, dec_value); return 7u; }

        /* ---- INX/INY/DEX/DEY ---- */
        case 0xE8u: { cpu->x = (uint8_t)(cpu->x + 1u); cpu_set_nz(cpu, cpu->x); return 2u; } /* INX */
        case 0xC8u: { cpu->y = (uint8_t)(cpu->y + 1u); cpu_set_nz(cpu, cpu->y); return 2u; } /* INY */
        case 0xCAu: { cpu->x = (uint8_t)(cpu->x - 1u); cpu_set_nz(cpu, cpu->x); return 2u; } /* DEX */
        case 0x88u: { cpu->y = (uint8_t)(cpu->y - 1u); cpu_set_nz(cpu, cpu->y); return 2u; } /* DEY */

        /* ---- ASL ---- */
        case 0x0Au: { cpu->a = asl_value(cpu, cpu->a); cpu_set_nz(cpu, cpu->a); return 2u; }
        case 0x06u: { rmw_zp(cpu, bus, asl_value);    return 5u; }
        case 0x16u: { rmw_zp_x(cpu, bus, asl_value);  return 6u; }
        case 0x0Eu: { rmw_abs(cpu, bus, asl_value);   return 6u; }
        case 0x1Eu: { rmw_abs_x(cpu, bus, asl_value); return 7u; }

        /* ---- LSR ---- */
        case 0x4Au: { cpu->a = lsr_value(cpu, cpu->a); cpu_set_nz(cpu, cpu->a); return 2u; }
        case 0x46u: { rmw_zp(cpu, bus, lsr_value);    return 5u; }
        case 0x56u: { rmw_zp_x(cpu, bus, lsr_value);  return 6u; }
        case 0x4Eu: { rmw_abs(cpu, bus, lsr_value);   return 6u; }
        case 0x5Eu: { rmw_abs_x(cpu, bus, lsr_value); return 7u; }

        /* ---- ROL ---- */
        case 0x2Au: { cpu->a = rol_value(cpu, cpu->a); cpu_set_nz(cpu, cpu->a); return 2u; }
        case 0x26u: { rmw_zp(cpu, bus, rol_value);    return 5u; }
        case 0x36u: { rmw_zp_x(cpu, bus, rol_value);  return 6u; }
        case 0x2Eu: { rmw_abs(cpu, bus, rol_value);   return 6u; }
        case 0x3Eu: { rmw_abs_x(cpu, bus, rol_value); return 7u; }

        /* ---- ROR ---- */
        case 0x6Au: { cpu->a = ror_value(cpu, cpu->a); cpu_set_nz(cpu, cpu->a); return 2u; }
        case 0x66u: { rmw_zp(cpu, bus, ror_value);    return 5u; }
        case 0x76u: { rmw_zp_x(cpu, bus, ror_value);  return 6u; }
        case 0x6Eu: { rmw_abs(cpu, bus, ror_value);   return 6u; }
        case 0x7Eu: { rmw_abs_x(cpu, bus, ror_value); return 7u; }

        /* ---- branches ---- */
        case 0x10u: return branch(cpu, bus, false, CPU_FLAG_N); /* BPL */
        case 0x30u: return branch(cpu, bus, true,  CPU_FLAG_N); /* BMI */
        case 0x50u: return branch(cpu, bus, false, CPU_FLAG_V); /* BVC */
        case 0x70u: return branch(cpu, bus, true,  CPU_FLAG_V); /* BVS */
        case 0x90u: return branch(cpu, bus, false, CPU_FLAG_C); /* BCC */
        case 0xB0u: return branch(cpu, bus, true,  CPU_FLAG_C); /* BCS */
        case 0xD0u: return branch(cpu, bus, false, CPU_FLAG_Z); /* BNE */
        case 0xF0u: return branch(cpu, bus, true,  CPU_FLAG_Z); /* BEQ */

        /* ---- JMP / JSR / RTS / RTI / BRK ---- */
        case 0x4Cu: { uint16_t a = cpu_am_absolute(cpu, bus); cpu->pc = a; return 3u; } /* JMP abs */
        case 0x6Cu: { uint16_t a = cpu_am_indirect(cpu, bus);  cpu->pc = a; return 5u; } /* JMP (ind) */
        case 0x20u: { /* JSR abs */
            uint16_t target = cpu_fetch_word(cpu, bus);
            /* Push PC of last byte of instruction (target-1) per 6502 RTS semantics. */
            cpu_push_pc(cpu, bus, (uint16_t)(cpu->pc - 1u));
            cpu->pc = target;
            return 6u;
        }
        case 0x60u: { /* RTS */
            uint16_t ret = (uint16_t)(cpu_pull_pc(cpu, bus) + 1u);
            cpu->pc = ret;
            return 6u;
        }
        case 0x40u: { /* RTI */
            cpu_pull_status(cpu, bus);
            cpu->pc = cpu_pull_pc(cpu, bus);
            return 6u;
        }
        case 0x00u: { /* BRK */
            /* BRK is a 2-byte instruction (the padding byte is fetched). */
            (void)cpu_fetch_byte(cpu, bus);
            cpu_push_pc(cpu, bus, cpu->pc);
            cpu_push_status(cpu, bus, true);
            cpu_set_interrupt_disable(cpu, true);
            cpu->pc = cpu_read_vector(cpu, bus, CPU_VECTOR_IRQ);
            return 7u;
        }

        /* ---- flag operations ---- */
        case 0x18u: { cpu_set_carry(cpu, false);            return 2u; } /* CLC */
        case 0x38u: { cpu_set_carry(cpu, true);             return 2u; } /* SEC */
        case 0x58u: { cpu_set_interrupt_disable(cpu, false);return 2u; } /* CLI */
        case 0x78u: { cpu_set_interrupt_disable(cpu, true); return 2u; } /* SEI */
        case 0xB8u: { cpu_set_overflow(cpu, false);         return 2u; } /* CLV */
        case 0xD8u: { cpu_set_decimal(cpu, false);          return 2u; } /* CLD */
        case 0xF8u: { cpu_set_decimal(cpu, true);           return 2u; } /* SED */

        /* ---- NOP ---- */
        case 0xEAu: return 2u;

        /* Unofficial / illegal opcodes: delegate to the unofficial dispatch. */
        default:
            return cpu_execute_unofficial(cpu, bus, opcode);
    }
}

