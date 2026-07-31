/*
 * addressing.h — 6502 addressing modes: enum, operand tag, dummy-read enum.
 *
 * Port of src/cpu/addressing.rs to C (M4.1). The 13 addressing modes and the
 * Dummy-read behaviour are reproduced exactly. The Operand enum is a tagged
 * struct (None / Accumulator / Address(u16)).
 *
 * See: https://www.nesdev.org/wiki/CPU_addressing_modes
 * See: https://www.nesdev.org/6502.txt — "Dummy reads" section.
 */
#ifndef NES_CORE_C_ADDRESSING_H
#define NES_CORE_C_ADDRESSING_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Forward decl — Bus is defined in bus.h; Cpu in cpu.h. Avoid circular include
 * by forward-declaring here and having cpu.h include bus.h before us. */
typedef struct Bus Bus;
typedef struct Cpu Cpu;

/* The 13 addressing modes supported by the 6502 (addressing.rs `AddrMode`). */
typedef enum {
    ADDR_MODE_IMPLIED,    /* Implied — no operand (DEX, TAX, CLC). */
    ADDR_MODE_ACCUMULATOR,/* Accumulator — operand is A (ASL A, LSR A). */
    ADDR_MODE_IMMEDIATE,  /* Immediate — operand byte follows opcode. */
    ADDR_MODE_ZERO_PAGE,  /* Zero-page — operand is a zero-page address. */
    ADDR_MODE_ZERO_PAGE_X,/* Zero-page,X — (operand + X) & 0xFF. */
    ADDR_MODE_ZERO_PAGE_Y,/* Zero-page,Y — (operand + Y) & 0xFF. */
    ADDR_MODE_ABSOLUTE,   /* Absolute — 16-bit operand is the address. */
    ADDR_MODE_ABSOLUTE_X, /* Absolute,X — operand + X. */
    ADDR_MODE_ABSOLUTE_Y, /* Absolute,Y — operand + Y. */
    ADDR_MODE_INDIRECT,   /* Indirect — JMP ($1000); page-wrap bug. */
    ADDR_MODE_INDIRECT_X, /* Indirect,X — (operand,X) zero-page indexed indirect. */
    ADDR_MODE_INDIRECT_Y, /* Indirect,Y — (operand),Y indirect indexed. */
    ADDR_MODE_RELATIVE    /* Relative — branch target = PC + signed offset. */
} AddrMode;

/* Dummy-read behaviour for indexed addressing modes (addressing.rs `Dummy`). */
typedef enum {
    DUMMY_NONE, /* No dummy read — used by resolve() / disassembler. */
    DUMMY_READ, /* Dummy read on page cross only — read opcodes (LDA/AND/...). */
    DUMMY_RMW   /* Unconditional dummy read — RMW and store opcodes. */
} Dummy;

/* Operand tag values for the tagged-union Operand. */
#define OPERAND_NONE 0
#define OPERAND_ACC  1
#define OPERAND_ADDR 2

/* The result of resolving an addressing mode (addressing.rs `Operand`).
 * `addr` is only meaningful when tag == OPERAND_ADDR. */
typedef struct Operand {
    int     tag;
    uint16_t addr;
} Operand;

static inline Operand operand_none(void)            { Operand o; o.tag = OPERAND_NONE; o.addr = 0;    return o; }
static inline Operand operand_accumulator(void)     { Operand o; o.tag = OPERAND_ACC;  o.addr = 0;    return o; }
static inline Operand operand_address(uint16_t a)   { Operand o; o.tag = OPERAND_ADDR; o.addr = a;    return o; }

/* ---- Addressing-mode resolvers (addressing.rs) ----------------------- */

/* Resolve the effective address for `mode` (no dummy reads). (addressing.rs `resolve`.) */
Operand cpu_resolve(Cpu* cpu, Bus* bus, AddrMode mode);

/* Immediate: effective "address" is PC; PC advances past the byte. */
uint16_t cpu_am_immediate(Cpu* cpu);
/* Zero-page: operand byte is a zero-page address. */
uint16_t cpu_am_zero_page(Cpu* cpu, Bus* bus);
/* Zero-page,X: (operand + X) & 0xFF; dummy read at base if dummy != None. */
uint16_t cpu_am_zero_page_x(Cpu* cpu, Bus* bus, Dummy dummy);
/* Zero-page,Y: (operand + Y) & 0xFF; dummy read at base if dummy != None. */
uint16_t cpu_am_zero_page_y(Cpu* cpu, Bus* bus, Dummy dummy);
/* Absolute: 16-bit operand is the effective address. */
uint16_t cpu_am_absolute(Cpu* cpu, Bus* bus);
/* Absolute,X: operand + X. Returns addr and sets *page_cross. Dummy read per `dummy`. */
uint16_t cpu_am_absolute_x(Cpu* cpu, Bus* bus, Dummy dummy, bool* page_cross);
/* Absolute,Y: operand + Y. Returns addr and sets *page_cross. Dummy read per `dummy`. */
uint16_t cpu_am_absolute_y(Cpu* cpu, Bus* bus, Dummy dummy, bool* page_cross);
/* Indirect: JMP ($1000) with the 6502 page-wrap bug (hi byte from same page). */
uint16_t cpu_am_indirect(Cpu* cpu, Bus* bus);
/* Indirect,X: (operand,X) zero-page indexed indirect. */
uint16_t cpu_am_indirect_x(Cpu* cpu, Bus* bus);
/* Indirect,Y: (operand),Y indirect indexed. Returns addr and sets *page_cross. */
uint16_t cpu_am_indirect_y(Cpu* cpu, Bus* bus, Dummy dummy, bool* page_cross);
/* Relative: branch target = PC (after offset byte) + signed offset. */
uint16_t cpu_am_relative(Cpu* cpu, Bus* bus);

/* Read the 16-bit base pointer for (zp),Y without adding Y (unofficial_special.rs).
 * Used by AHX (ind),Y so the page-cross quirk can be applied on the full base+Y. */
uint16_t cpu_am_indirect_y_base(Cpu* cpu, Bus* bus);

#ifdef __cplusplus
}
#endif
#endif /* NES_CORE_C_ADDRESSING_H */

