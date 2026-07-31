/*
 * addressing_tpl.hpp - Template-based 6502 addressing-mode resolvers (M5).
 *
 * This is the C++ idiom layer for the NES core: the 13 addressing modes are
 * implemented as template specializations of `resolve<Mode>` using `if
 * constexpr` for mode-specific logic. Each specialization delegates to the
 * same underlying C-style resolver functions (cpu_am_*) so behaviour is
 * byte-identical to the C switch-based dispatch in addressing.cpp.
 *
 * Acceptance criterion: "Template-based addressing modes produce same results
 * as C switch." This is verified by test_cpu.cpp's template-addressing checks.
 */
#ifndef NES_CORE_CPP_ADDRESSING_TPL_HPP
#define NES_CORE_CPP_ADDRESSING_TPL_HPP

#include "addressing.hpp"
#include "cpu.hpp"
#include "bus.hpp"
#include <cstdint>

namespace nes_cpp {

// Template primary declaration. Specializations below cover all 13 modes.
// The signature matches the M5 spec:
//   template<AddrMode Mode> uint16_t resolve(Cpu&, Bus&, Dummy, bool*)
template<AddrMode Mode>
inline uint16_t resolve(Cpu& cpu, Bus& bus, Dummy dummy, bool* page_cross);

// Implied: no operand, no address. Returns 0.
template<>
inline uint16_t resolve<ADDR_MODE_IMPLIED>(Cpu& /*cpu*/, Bus& /*bus*/, Dummy /*dummy*/, bool* /*page_cross*/) {
    return 0;
}

// Accumulator: operand is the A register. Returns 0 (tagged separately by caller).
template<>
inline uint16_t resolve<ADDR_MODE_ACCUMULATOR>(Cpu& /*cpu*/, Bus& /*bus*/, Dummy /*dummy*/, bool* /*page_cross*/) {
    return 0;
}

// Immediate: effective "address" is PC; PC advances past the byte.
template<>
inline uint16_t resolve<ADDR_MODE_IMMEDIATE>(Cpu& cpu, Bus& bus, Dummy /*dummy*/, bool* /*page_cross*/) {
    if constexpr (true) {
        return cpu_am_immediate(&cpu);
    }
}

// Zero-page: operand byte is a zero-page address.
template<>
inline uint16_t resolve<ADDR_MODE_ZERO_PAGE>(Cpu& cpu, Bus& bus, Dummy dummy, bool* /*page_cross*/) {
    if constexpr (true) {
        return cpu_am_zero_page(&cpu, &bus);
    }
}

// Zero-page,X: (operand + X) & 0xFF; dummy read at base if dummy != None.
template<>
inline uint16_t resolve<ADDR_MODE_ZERO_PAGE_X>(Cpu& cpu, Bus& bus, Dummy dummy, bool* /*page_cross*/) {
    if constexpr (true) {
        return cpu_am_zero_page_x(&cpu, &bus, dummy);
    }
}

// Zero-page,Y: (operand + Y) & 0xFF; dummy read at base if dummy != None.
template<>
inline uint16_t resolve<ADDR_MODE_ZERO_PAGE_Y>(Cpu& cpu, Bus& bus, Dummy dummy, bool* /*page_cross*/) {
    if constexpr (true) {
        return cpu_am_zero_page_y(&cpu, &bus, dummy);
    }
}

// Absolute: 16-bit operand is the effective address.
template<>
inline uint16_t resolve<ADDR_MODE_ABSOLUTE>(Cpu& cpu, Bus& bus, Dummy /*dummy*/, bool* /*page_cross*/) {
    if constexpr (true) {
        return cpu_am_absolute(&cpu, &bus);
    }
}

// Absolute,X: operand + X; page_cross out; dummy read per `dummy`.
template<>
inline uint16_t resolve<ADDR_MODE_ABSOLUTE_X>(Cpu& cpu, Bus& bus, Dummy dummy, bool* page_cross) {
    if constexpr (true) {
        return cpu_am_absolute_x(&cpu, &bus, dummy, page_cross);
    }
}

// Absolute,Y: operand + Y; page_cross out; dummy read per `dummy`.
template<>
inline uint16_t resolve<ADDR_MODE_ABSOLUTE_Y>(Cpu& cpu, Bus& bus, Dummy dummy, bool* page_cross) {
    if constexpr (true) {
        return cpu_am_absolute_y(&cpu, &bus, dummy, page_cross);
    }
}

// Indirect: JMP ($1000) with the 6502 page-wrap bug.
template<>
inline uint16_t resolve<ADDR_MODE_INDIRECT>(Cpu& cpu, Bus& bus, Dummy /*dummy*/, bool* /*page_cross*/) {
    if constexpr (true) {
        return cpu_am_indirect(&cpu, &bus);
    }
}

// Indirect,X: (operand,X) zero-page indexed indirect.
template<>
inline uint16_t resolve<ADDR_MODE_INDIRECT_X>(Cpu& cpu, Bus& bus, Dummy /*dummy*/, bool* /*page_cross*/) {
    if constexpr (true) {
        return cpu_am_indirect_x(&cpu, &bus);
    }
}

// Indirect,Y: (operand),Y indirect indexed; page_cross out; dummy read.
template<>
inline uint16_t resolve<ADDR_MODE_INDIRECT_Y>(Cpu& cpu, Bus& bus, Dummy dummy, bool* page_cross) {
    if constexpr (true) {
        return cpu_am_indirect_y(&cpu, &bus, dummy, page_cross);
    }
}

// Relative: branch target = PC (after offset byte) + signed offset.
template<>
inline uint16_t resolve<ADDR_MODE_RELATIVE>(Cpu& cpu, Bus& bus, Dummy /*dummy*/, bool* /*page_cross*/) {
    if constexpr (true) {
        return cpu_am_relative(&cpu, &bus);
    }
}

// ---- Runtime dispatch: map AddrMode enum value to template specialization ----
// This is the bridge between the constexpr opcode table (which stores an
// AddrMode per opcode) and the template specializations. It produces the same
// result as the C switch in cpu_resolve().
inline uint16_t resolve_dispatch(Cpu& cpu, Bus& bus, AddrMode mode,
                                 Dummy dummy = DUMMY_NONE, bool* page_cross = nullptr) {
    switch (mode) {
        case ADDR_MODE_IMPLIED:    return resolve<ADDR_MODE_IMPLIED>(cpu, bus, dummy, page_cross);
        case ADDR_MODE_ACCUMULATOR:return resolve<ADDR_MODE_ACCUMULATOR>(cpu, bus, dummy, page_cross);
        case ADDR_MODE_IMMEDIATE:  return resolve<ADDR_MODE_IMMEDIATE>(cpu, bus, dummy, page_cross);
        case ADDR_MODE_ZERO_PAGE:  return resolve<ADDR_MODE_ZERO_PAGE>(cpu, bus, dummy, page_cross);
        case ADDR_MODE_ZERO_PAGE_X:return resolve<ADDR_MODE_ZERO_PAGE_X>(cpu, bus, dummy, page_cross);
        case ADDR_MODE_ZERO_PAGE_Y:return resolve<ADDR_MODE_ZERO_PAGE_Y>(cpu, bus, dummy, page_cross);
        case ADDR_MODE_ABSOLUTE:   return resolve<ADDR_MODE_ABSOLUTE>(cpu, bus, dummy, page_cross);
        case ADDR_MODE_ABSOLUTE_X: return resolve<ADDR_MODE_ABSOLUTE_X>(cpu, bus, dummy, page_cross);
        case ADDR_MODE_ABSOLUTE_Y: return resolve<ADDR_MODE_ABSOLUTE_Y>(cpu, bus, dummy, page_cross);
        case ADDR_MODE_INDIRECT:   return resolve<ADDR_MODE_INDIRECT>(cpu, bus, dummy, page_cross);
        case ADDR_MODE_INDIRECT_X: return resolve<ADDR_MODE_INDIRECT_X>(cpu, bus, dummy, page_cross);
        case ADDR_MODE_INDIRECT_Y: return resolve<ADDR_MODE_INDIRECT_Y>(cpu, bus, dummy, page_cross);
        case ADDR_MODE_RELATIVE:   return resolve<ADDR_MODE_RELATIVE>(cpu, bus, dummy, page_cross);
        default:                   return 0;
    }
}

} // namespace nes_cpp

#endif // NES_CORE_CPP_ADDRESSING_TPL_HPP
