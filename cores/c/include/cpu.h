/*
 * cpu.h — 6502 CPU core: registers, flags, step/reset/nmi/irq, stack, operand helpers.
 *
 * Port of src/cpu/mod.rs to C (M4.1). The Cpu struct mirrors the Rust struct
 * field-for-field. The packed `flags` byte holds NMI_PENDING/IRQ_PENDING/HALTED
 * exactly as in Rust. There is NO cycle_count field on the Cpu (cycles are
 * tracked on the Bus/emulator in the Rust source — see bus.rs cpu_cycle_count).
 *
 * See: https://www.nesdev.org/wiki/CPU_registers
 */
#ifndef NES_CORE_C_CPU_H
#define NES_CORE_C_CPU_H

#include <stdint.h>
#include <stdbool.h>
#include "addressing.h"
#include "bus.h"

#ifdef __cplusplus
extern "C" {
#endif

/* ---- Status flag bit masks (mod.rs `flags` module) -------------------- */
#define CPU_FLAG_C 0x01u  /* Carry      (bit 0) */
#define CPU_FLAG_Z 0x02u  /* Zero       (bit 1) */
#define CPU_FLAG_I 0x04u  /* Interrupt  (bit 2) */
#define CPU_FLAG_D 0x08u  /* Decimal    (bit 3 — no effect on 2A03) */
#define CPU_FLAG_B 0x10u  /* Break      (bit 4 — only in pushed copy) */
#define CPU_FLAG_U 0x20u  /* Unused     (bit 5 — always reads 1) */
#define CPU_FLAG_V 0x40u  /* Overflow   (bit 6) */
#define CPU_FLAG_N 0x80u  /* Negative   (bit 7) */

/* ---- Interrupt vectors (mod.rs `vectors` module) ---------------------- */
#define CPU_VECTOR_NMI   0xFFFAu
#define CPU_VECTOR_RESET 0xFFFCu
#define CPU_VECTOR_IRQ   0xFFFEu

/* ---- Packed internal flags byte (mod.rs `flags` field) ---------------- */
#define CPU_NMI_PENDING 0x01u
#define CPU_IRQ_PENDING 0x02u
#define CPU_HALTED      0x04u

/* The 6502 CPU. Field order matches src/cpu/mod.rs `struct Cpu`. */
typedef struct Cpu {
    uint8_t  a;       /* Accumulator */
    uint8_t  x;       /* X index */
    uint8_t  y;       /* Y index */
    uint8_t  sp;      /* Stack pointer (stack at $0100..=$01FF) */
    uint16_t pc;      /* Program counter */
    uint8_t  status;  /* P register */
    uint8_t  flags;   /* packed: NMI_PENDING | IRQ_PENDING | HALTED */
} Cpu;

/* ---- Construction ----------------------------------------------------- */

/* Construct a CPU in the simplified power-on state (mod.rs `Cpu::new`):
 * registers zeroed, SP=$FD, status = U|I, flags=0, PC=$0000. */
void cpu_init(Cpu* cpu);

/* ---- Flag helpers (mod.rs) ------------------------------------------- */

static inline bool cpu_carry(const Cpu* c)            { return (c->status & CPU_FLAG_C) != 0; }
static inline bool cpu_zero(const Cpu* c)             { return (c->status & CPU_FLAG_Z) != 0; }
static inline bool cpu_interrupt_disable(const Cpu* c){ return (c->status & CPU_FLAG_I) != 0; }
static inline bool cpu_decimal(const Cpu* c)          { return (c->status & CPU_FLAG_D) != 0; }
static inline bool cpu_overflow(const Cpu* c)         { return (c->status & CPU_FLAG_V) != 0; }
static inline bool cpu_negative(const Cpu* c)         { return (c->status & CPU_FLAG_N) != 0; }

void cpu_set_carry(Cpu* c, bool v);
void cpu_set_zero(Cpu* c, bool v);
void cpu_set_interrupt_disable(Cpu* c, bool v);
void cpu_set_decimal(Cpu* c, bool v);
void cpu_set_overflow(Cpu* c, bool v);
void cpu_set_negative(Cpu* c, bool v);

/* Set or clear a flag bit (mod.rs `set_flag`). */
void cpu_set_flag(Cpu* c, uint8_t flag, bool v);

/* Set N and Z from a result byte (mod.rs `set_nz`). */
void cpu_set_nz(Cpu* c, uint8_t value);

/* ---- Fetch helpers (mod.rs) ------------------------------------------ */

/* Read a byte at PC and advance PC by one. */
uint8_t cpu_fetch_byte(Cpu* cpu, Bus* bus);
/* Read a little-endian 16-bit word at PC and advance PC by two. */
uint16_t cpu_fetch_word(Cpu* cpu, Bus* bus);

/* ---- Stack access (mod.rs) ------------------------------------------- */

void cpu_push(Cpu* cpu, Bus* bus, uint8_t value);
uint8_t cpu_pull(Cpu* cpu, Bus* bus);
void cpu_push_pc(Cpu* cpu, Bus* bus, uint16_t pc);
uint16_t cpu_pull_pc(Cpu* cpu, Bus* bus);
void cpu_push_status(Cpu* cpu, Bus* bus, bool with_break);
void cpu_pull_status(Cpu* cpu, Bus* bus);

/* ---- Operand read/write helpers (mod.rs) ----------------------------- */

uint8_t cpu_read_operand(Cpu* cpu, Bus* bus, Operand op);
void cpu_write_operand(Cpu* cpu, Bus* bus, Operand op, uint8_t value);

/* ---- Interrupt / step (mod.rs) --------------------------------------- */

/* NMI: push PC + status (B clear, U set), set I, load PC from $FFFA/$FFFB. 7 cycles. */
void cpu_nmi(Cpu* cpu, Bus* bus);
/* IRQ: push PC + status (B clear, U set), set I, load PC from $FFFE/$FFFF. 7 cycles. */
void cpu_irq(Cpu* cpu, Bus* bus);
/* RESET: SP=$FD, set I, set U, PC from $FFFC/$FFFD, clear HALTED. */
void cpu_reset(Cpu* cpu, Bus* bus);

/* Fetch and execute one instruction; returns cycle count (mod.rs `step`).
 * If HALTED, returns 1 without fetching. Services pending NMI/IRQ first. */
uint8_t cpu_step(Cpu* cpu, Bus* bus);

/* ---- Halt / interrupt pending accessors (mod.rs) --------------------- */

bool cpu_is_halted(const Cpu* c);
void cpu_set_halted(Cpu* c, bool v);
bool cpu_nmi_pending(const Cpu* c);
void cpu_set_nmi_pending(Cpu* c, bool v);
bool cpu_irq_pending(const Cpu* c);
void cpu_set_irq_pending(Cpu* c, bool v);

/* ---- Execute dispatch (opcodes.rs) ----------------------------------- */

/* Decode and execute a single opcode (already fetched; PC advanced past it).
 * Returns cycle count. Falls through to unofficial dispatch for non-official
 * opcodes. */
uint8_t cpu_execute(Cpu* cpu, Bus* bus, uint8_t opcode);

/* Unofficial-opcode dispatch root (unofficial.rs). */
uint8_t cpu_execute_unofficial(Cpu* cpu, Bus* bus, uint8_t opcode);

/* RMW-combo + unstable-store unofficial dispatch (unofficial_rmw.rs). */
uint8_t cpu_execute_unofficial_rmw(Cpu* cpu, Bus* bus, uint8_t opcode);

/* Read a little-endian 16-bit vector from CPU address space (opcodes.rs). */
uint16_t cpu_read_vector(const Cpu* cpu, Bus* bus, uint16_t addr);

/* ---- Internal operand-resolving helpers shared with cpu_unofficial.c ----
 * (opcodes.rs `op_zp` / `op_zp_x` / `op_abs` / `op_abs_x` / `op_abs_y` /
 *  `op_ind_x` / `op_ind_y`.) These use Dummy::Rmw so the dummy read fires
 *  unconditionally, matching the RMW-combo opcode requirements. */
Operand op_zp(Cpu* cpu, Bus* bus);
Operand op_zp_x(Cpu* cpu, Bus* bus);
Operand op_abs(Cpu* cpu, Bus* bus);
Operand op_abs_x(Cpu* cpu, Bus* bus);
Operand op_abs_y(Cpu* cpu, Bus* bus);
Operand op_ind_x(Cpu* cpu, Bus* bus);
Operand op_ind_y(Cpu* cpu, Bus* bus);

/* ---- Internal read/write helpers shared with cpu_unofficial.c --------
 * (opcodes.rs `rd_*` / `wr_*`.) Exposed so the unofficial dispatch can fetch
 * operands with the same Dummy-read semantics as the official opcodes. */
typedef uint8_t (*ValueFn)(Cpu*, uint8_t);

uint8_t rd_imm(Cpu* cpu, Bus* bus);
uint8_t rd_zp(Cpu* cpu, Bus* bus);
uint8_t rd_zp_x(Cpu* cpu, Bus* bus);
uint8_t rd_zp_y(Cpu* cpu, Bus* bus);
uint8_t rd_abs(Cpu* cpu, Bus* bus);
uint8_t rd_abs_x(Cpu* cpu, Bus* bus, bool* page_cross);
uint8_t rd_abs_y(Cpu* cpu, Bus* bus, bool* page_cross);
uint8_t rd_ind_x(Cpu* cpu, Bus* bus);
uint8_t rd_ind_y(Cpu* cpu, Bus* bus, bool* page_cross);

void wr_zp(Cpu* cpu, Bus* bus, uint8_t v);
void wr_zp_x(Cpu* cpu, Bus* bus, uint8_t v);
void wr_zp_y(Cpu* cpu, Bus* bus, uint8_t v);
void wr_abs(Cpu* cpu, Bus* bus, uint8_t v);
void wr_abs_x(Cpu* cpu, Bus* bus, uint8_t v);
void wr_abs_y(Cpu* cpu, Bus* bus, uint8_t v);
void wr_ind_x(Cpu* cpu, Bus* bus, uint8_t v);
void wr_ind_y(Cpu* cpu, Bus* bus, uint8_t v);

#ifdef __cplusplus
}
#endif
#endif /* NES_CORE_C_CPU_H */

