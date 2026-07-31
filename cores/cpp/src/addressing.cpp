/*
 * addressing.c — 6502 addressing-mode resolvers with dummy-read support.
 *
 * Port of src/cpu/addressing.rs to C (M4.1). Each resolver matches the Rust
 * implementation byte-for-byte, including the dummy-read firing rules:
 *   Dummy::None  -> never
 *   Dummy::Read  -> only on page cross (indexed absolute / indirect,Y reads)
 *   Dummy::Rmw   -> unconditionally (RMW + store opcodes)
 * and the 6502 indirect JMP page-wrap bug (am_indirect).
 *
 * The Cpu struct and fetch helpers are declared in cpu.h; we include it here
 * to access cpu->pc / cpu->x / cpu->y and cpu_fetch_byte / cpu_fetch_word.
 */
#include "cpu.hpp"

/* ---- resolve: dispatch over AddrMode with Dummy::None (addressing.rs `resolve`) ---- */

Operand cpu_resolve(Cpu* cpu, Bus* bus, AddrMode mode) {
    bool pc_dummy;
    switch (mode) {
        case ADDR_MODE_IMPLIED:    return operand_none();
        case ADDR_MODE_ACCUMULATOR:return operand_accumulator();
        case ADDR_MODE_IMMEDIATE:  return operand_address(cpu_am_immediate(cpu));
        case ADDR_MODE_ZERO_PAGE:  return operand_address(cpu_am_zero_page(cpu, bus));
        case ADDR_MODE_ZERO_PAGE_X:return operand_address(cpu_am_zero_page_x(cpu, bus, DUMMY_NONE));
        case ADDR_MODE_ZERO_PAGE_Y:return operand_address(cpu_am_zero_page_y(cpu, bus, DUMMY_NONE));
        case ADDR_MODE_ABSOLUTE:   return operand_address(cpu_am_absolute(cpu, bus));
        case ADDR_MODE_ABSOLUTE_X: return operand_address(cpu_am_absolute_x(cpu, bus, DUMMY_NONE, &pc_dummy));
        case ADDR_MODE_ABSOLUTE_Y: return operand_address(cpu_am_absolute_y(cpu, bus, DUMMY_NONE, &pc_dummy));
        case ADDR_MODE_INDIRECT:   return operand_address(cpu_am_indirect(cpu, bus));
        case ADDR_MODE_INDIRECT_X: return operand_address(cpu_am_indirect_x(cpu, bus));
        case ADDR_MODE_INDIRECT_Y: return operand_address(cpu_am_indirect_y(cpu, bus, DUMMY_NONE, &pc_dummy));
        case ADDR_MODE_RELATIVE:   return operand_address(cpu_am_relative(cpu, bus));
        default:                   return operand_none();
    }
}

/* ---- am_immediate: effective "address" is PC; PC advances past the byte ---- */
uint16_t cpu_am_immediate(Cpu* cpu) {
    uint16_t addr = cpu->pc;
    cpu->pc = (uint16_t)(cpu->pc + 1u);
    return addr;
}

/* ---- am_zero_page: operand byte is a zero-page address ---- */
uint16_t cpu_am_zero_page(Cpu* cpu, Bus* bus) {
    return (uint16_t)cpu_fetch_byte(cpu, bus);
}

/* ---- am_zero_page_x: (operand + X) & 0xFF; dummy read at base if dummy != None ---- */
uint16_t cpu_am_zero_page_x(Cpu* cpu, Bus* bus, Dummy dummy) {
    uint8_t base = cpu_fetch_byte(cpu, bus);
    if (dummy != DUMMY_NONE) {
        (void)bus_read(bus, (uint16_t)base);
    }
    return (uint16_t)(uint8_t)(base + cpu->x);
}

/* ---- am_zero_page_y: (operand + Y) & 0xFF; dummy read at base if dummy != None ---- */
uint16_t cpu_am_zero_page_y(Cpu* cpu, Bus* bus, Dummy dummy) {
    uint8_t base = cpu_fetch_byte(cpu, bus);
    if (dummy != DUMMY_NONE) {
        (void)bus_read(bus, (uint16_t)base);
    }
    return (uint16_t)(uint8_t)(base + cpu->y);
}

/* ---- am_absolute: 16-bit operand is the effective address ---- */
uint16_t cpu_am_absolute(Cpu* cpu, Bus* bus) {
    return cpu_fetch_word(cpu, bus);
}

/* ---- am_absolute_x: operand + X; page_cross out; dummy read per `dummy` ---- */
uint16_t cpu_am_absolute_x(Cpu* cpu, Bus* bus, Dummy dummy, bool* page_cross) {
    uint16_t base = cpu_fetch_word(cpu, bus);
    uint16_t eff  = (uint16_t)(base + (uint16_t)cpu->x);
    bool cross = (base & 0xFF00u) != (eff & 0xFF00u);
    if ((dummy == DUMMY_READ && cross) || dummy == DUMMY_RMW) {
        (void)bus_read(bus, (uint16_t)((base & 0xFF00u) | (eff & 0x00FFu)));
    }
    if (page_cross) *page_cross = cross;
    return eff;
}

/* ---- am_absolute_y: operand + Y; page_cross out; dummy read per `dummy` ---- */
uint16_t cpu_am_absolute_y(Cpu* cpu, Bus* bus, Dummy dummy, bool* page_cross) {
    uint16_t base = cpu_fetch_word(cpu, bus);
    uint16_t eff  = (uint16_t)(base + (uint16_t)cpu->y);
    bool cross = (base & 0xFF00u) != (eff & 0xFF00u);
    if ((dummy == DUMMY_READ && cross) || dummy == DUMMY_RMW) {
        (void)bus_read(bus, (uint16_t)((base & 0xFF00u) | (eff & 0x00FFu)));
    }
    if (page_cross) *page_cross = cross;
    return eff;
}

/* ---- am_indirect: JMP ($1000) with the 6502 page-wrap bug ----
 * If the pointer's low byte is $FF, the high byte is fetched from the same
 * page's $00 rather than the next page. E.g. JMP ($30FF) reads lo from $30FF
 * and hi from $3000 (not $3100).
 */
uint16_t cpu_am_indirect(Cpu* cpu, Bus* bus) {
    uint16_t ptr = cpu_fetch_word(cpu, bus);
    uint8_t  lo  = bus_read(bus, ptr);
    uint16_t hi_addr = (uint16_t)((ptr & 0xFF00u) | (uint16_t)(uint8_t)(uint8_t)(ptr + 1u));
    uint8_t  hi  = bus_read(bus, hi_addr);
    return (uint16_t)((uint16_t)lo | ((uint16_t)hi << 8));
}

/* ---- am_indirect_x: (operand,X) zero-page indexed indirect ----
 * Pointer location is (operand + X) & 0xFF (wraps in zero page). The 16-bit
 * pointer is read from two consecutive zero-page addresses (second also wraps).
 */
uint16_t cpu_am_indirect_x(Cpu* cpu, Bus* bus) {
    uint8_t zp   = cpu_fetch_byte(cpu, bus);
    uint8_t ptr  = (uint8_t)(zp + cpu->x);
    uint8_t lo   = bus_read(bus, (uint16_t)ptr);
    uint8_t hi   = bus_read(bus, (uint16_t)(uint8_t)(ptr + 1u));
    return (uint16_t)((uint16_t)lo | ((uint16_t)hi << 8));
}

/* ---- am_indirect_y: (operand),Y indirect indexed; page_cross out; dummy read ---- */
uint16_t cpu_am_indirect_y(Cpu* cpu, Bus* bus, Dummy dummy, bool* page_cross) {
    uint8_t zp = cpu_fetch_byte(cpu, bus);
    uint8_t lo = bus_read(bus, (uint16_t)zp);
    uint8_t hi = bus_read(bus, (uint16_t)(uint8_t)(zp + 1u));
    uint16_t base = (uint16_t)((uint16_t)lo | ((uint16_t)hi << 8));
    uint16_t eff  = (uint16_t)(base + (uint16_t)cpu->y);
    bool cross = (base & 0xFF00u) != (eff & 0xFF00u);
    if ((dummy == DUMMY_READ && cross) || dummy == DUMMY_RMW) {
        (void)bus_read(bus, (uint16_t)((base & 0xFF00u) | (eff & 0x00FFu)));
    }
    if (page_cross) *page_cross = cross;
    return eff;
}

/* ---- am_relative: branch target = PC (after offset byte) + signed offset ---- */
uint16_t cpu_am_relative(Cpu* cpu, Bus* bus) {
    int8_t offset = (int8_t)cpu_fetch_byte(cpu, bus);
    return (uint16_t)(cpu->pc + (uint16_t)(int16_t)offset);
}

/* ---- am_indirect_y_base: read (zp),Y base pointer without adding Y ----
 * (unofficial_special.rs `am_indirect_y_base`.) Used by AHX (ind),Y so the
 * page-cross quirk can be applied on the full base + Y without double-firing
 * a dummy read.
 */
uint16_t cpu_am_indirect_y_base(Cpu* cpu, Bus* bus) {
    uint8_t zp = cpu_fetch_byte(cpu, bus);
    uint8_t lo = bus_read(bus, (uint16_t)zp);
    uint8_t hi = bus_read(bus, (uint16_t)(uint8_t)(zp + 1u));
    return (uint16_t)((uint16_t)lo | ((uint16_t)hi << 8));
}

