/*
 * opcodes.hpp - Constexpr 6502 opcode metadata table (M5).
 *
 * A `constexpr std::array<OpcodeEntry, 256>` mapping every opcode byte to its
 * addressing mode, base cycle count, and page-cross penalty flag. This is the
 * C++ idiom layer: the dispatch can use this table instead of a hardcoded
 * switch. The data matches the cycle counts in cpu.cpp's cpu_execute switch
 * and the addressing modes from addressing.hpp.
 *
 * Acceptance criterion: "constexpr opcode table" - verified by test_cpu.cpp.
 */
#ifndef NES_CORE_CPP_OPCODES_HPP
#define NES_CORE_CPP_OPCODES_HPP

#include "addressing.hpp"
#include <array>
#include <cstdint>

namespace nes_cpp {

// Metadata for a single opcode.
struct OpcodeEntry {
    AddrMode  mode;         // Addressing mode for this opcode.
    uint8_t   base_cycles;  // Base cycle count (before page-cross penalty).
    bool      page_cross;   // Whether a page cross adds +1 cycle.
    bool      is_valid;     // Whether this is a documented/implemented opcode.
};

// Helper to make the table more readable.
constexpr OpcodeEntry OP(AddrMode m, uint8_t c, bool pc = false, bool v = true) {
    return OpcodeEntry{m, c, pc, v};
}

constexpr OpcodeEntry INVALID = OpcodeEntry{ADDR_MODE_IMPLIED, 0, false, false};

// The full 256-entry opcode table. Each entry matches the cycle count and
// addressing mode used by the corresponding case in cpu_execute (cpu.cpp).
// Unofficial opcodes are marked is_valid=false here (they are handled by
// cpu_execute_unofficial). Official opcodes that are NOPs in the 6502
// (e.g. 0x1A, 0x3A, etc.) are also marked invalid since they're unofficial.
constexpr std::array<OpcodeEntry, 256> OPCODE_TABLE = []() {
    std::array<OpcodeEntry, 256> t{};
    // Initialize all as invalid first.
    for (int i = 0; i < 256; ++i) t[i] = INVALID;

    // ---- LDA ----
    t[0xA9] = OP(ADDR_MODE_IMMEDIATE,  2);
    t[0xA5] = OP(ADDR_MODE_ZERO_PAGE,  3);
    t[0xB5] = OP(ADDR_MODE_ZERO_PAGE_X,4);
    t[0xAD] = OP(ADDR_MODE_ABSOLUTE,   4);
    t[0xBD] = OP(ADDR_MODE_ABSOLUTE_X, 4, true);
    t[0xB9] = OP(ADDR_MODE_ABSOLUTE_Y, 4, true);
    t[0xA1] = OP(ADDR_MODE_INDIRECT_X, 6);
    t[0xB1] = OP(ADDR_MODE_INDIRECT_Y, 5, true);

    // ---- LDX ----
    t[0xA2] = OP(ADDR_MODE_IMMEDIATE,  2);
    t[0xA6] = OP(ADDR_MODE_ZERO_PAGE,  3);
    t[0xB6] = OP(ADDR_MODE_ZERO_PAGE_Y,4);
    t[0xAE] = OP(ADDR_MODE_ABSOLUTE,   4);
    t[0xBE] = OP(ADDR_MODE_ABSOLUTE_Y, 4, true);

    // ---- LDY ----
    t[0xA0] = OP(ADDR_MODE_IMMEDIATE,  2);
    t[0xA4] = OP(ADDR_MODE_ZERO_PAGE,  3);
    t[0xB4] = OP(ADDR_MODE_ZERO_PAGE_X,4);
    t[0xAC] = OP(ADDR_MODE_ABSOLUTE,   4);
    t[0xBC] = OP(ADDR_MODE_ABSOLUTE_X, 4, true);

    // ---- STA ----
    t[0x85] = OP(ADDR_MODE_ZERO_PAGE,  3);
    t[0x95] = OP(ADDR_MODE_ZERO_PAGE_X,4);
    t[0x8D] = OP(ADDR_MODE_ABSOLUTE,   4);
    t[0x9D] = OP(ADDR_MODE_ABSOLUTE_X, 5);
    t[0x99] = OP(ADDR_MODE_ABSOLUTE_Y, 5);
    t[0x81] = OP(ADDR_MODE_INDIRECT_X, 6);
    t[0x91] = OP(ADDR_MODE_INDIRECT_Y, 6);

    // ---- STX ----
    t[0x86] = OP(ADDR_MODE_ZERO_PAGE,  3);
    t[0x96] = OP(ADDR_MODE_ZERO_PAGE_Y,4);
    t[0x8E] = OP(ADDR_MODE_ABSOLUTE,   4);

    // ---- STY ----
    t[0x84] = OP(ADDR_MODE_ZERO_PAGE,  3);
    t[0x94] = OP(ADDR_MODE_ZERO_PAGE_X,4);
    t[0x8C] = OP(ADDR_MODE_ABSOLUTE,   4);

    // ---- Register transfers ----
    t[0xAA] = OP(ADDR_MODE_IMPLIED, 2); // TAX
    t[0xA8] = OP(ADDR_MODE_IMPLIED, 2); // TAY
    t[0x8A] = OP(ADDR_MODE_IMPLIED, 2); // TXA
    t[0x98] = OP(ADDR_MODE_IMPLIED, 2); // TYA
    t[0xBA] = OP(ADDR_MODE_IMPLIED, 2); // TSX
    t[0x9A] = OP(ADDR_MODE_IMPLIED, 2); // TXS

    // ---- Stack ----
    t[0x48] = OP(ADDR_MODE_IMPLIED, 3); // PHA
    t[0x08] = OP(ADDR_MODE_IMPLIED, 3); // PHP
    t[0x68] = OP(ADDR_MODE_IMPLIED, 4); // PLA
    t[0x28] = OP(ADDR_MODE_IMPLIED, 4); // PLP

    // ---- AND ----
    t[0x29] = OP(ADDR_MODE_IMMEDIATE,  2);
    t[0x25] = OP(ADDR_MODE_ZERO_PAGE,  3);
    t[0x35] = OP(ADDR_MODE_ZERO_PAGE_X,4);
    t[0x2D] = OP(ADDR_MODE_ABSOLUTE,   4);
    t[0x3D] = OP(ADDR_MODE_ABSOLUTE_X, 4, true);
    t[0x39] = OP(ADDR_MODE_ABSOLUTE_Y, 4, true);
    t[0x21] = OP(ADDR_MODE_INDIRECT_X, 6);
    t[0x31] = OP(ADDR_MODE_INDIRECT_Y, 5, true);

    // ---- ORA ----
    t[0x09] = OP(ADDR_MODE_IMMEDIATE,  2);
    t[0x05] = OP(ADDR_MODE_ZERO_PAGE,  3);
    t[0x15] = OP(ADDR_MODE_ZERO_PAGE_X,4);
    t[0x0D] = OP(ADDR_MODE_ABSOLUTE,   4);
    t[0x1D] = OP(ADDR_MODE_ABSOLUTE_X, 4, true);
    t[0x19] = OP(ADDR_MODE_ABSOLUTE_Y, 4, true);
    t[0x01] = OP(ADDR_MODE_INDIRECT_X, 6);
    t[0x11] = OP(ADDR_MODE_INDIRECT_Y, 5, true);

    // ---- EOR ----
    t[0x49] = OP(ADDR_MODE_IMMEDIATE,  2);
    t[0x45] = OP(ADDR_MODE_ZERO_PAGE,  3);
    t[0x55] = OP(ADDR_MODE_ZERO_PAGE_X,4);
    t[0x4D] = OP(ADDR_MODE_ABSOLUTE,   4);
    t[0x5D] = OP(ADDR_MODE_ABSOLUTE_X, 4, true);
    t[0x59] = OP(ADDR_MODE_ABSOLUTE_Y, 4, true);
    t[0x41] = OP(ADDR_MODE_INDIRECT_X, 6);
    t[0x51] = OP(ADDR_MODE_INDIRECT_Y, 5, true);

    // ---- ADC ----
    t[0x69] = OP(ADDR_MODE_IMMEDIATE,  2);
    t[0x65] = OP(ADDR_MODE_ZERO_PAGE,  3);
    t[0x75] = OP(ADDR_MODE_ZERO_PAGE_X,4);
    t[0x6D] = OP(ADDR_MODE_ABSOLUTE,   4);
    t[0x7D] = OP(ADDR_MODE_ABSOLUTE_X, 4, true);
    t[0x79] = OP(ADDR_MODE_ABSOLUTE_Y, 4, true);
    t[0x61] = OP(ADDR_MODE_INDIRECT_X, 6);
    t[0x71] = OP(ADDR_MODE_INDIRECT_Y, 5, true);

    // ---- SBC ----
    t[0xE9] = OP(ADDR_MODE_IMMEDIATE,  2);
    t[0xE5] = OP(ADDR_MODE_ZERO_PAGE,  3);
    t[0xF5] = OP(ADDR_MODE_ZERO_PAGE_X,4);
    t[0xED] = OP(ADDR_MODE_ABSOLUTE,   4);
    t[0xFD] = OP(ADDR_MODE_ABSOLUTE_X, 4, true);
    t[0xF9] = OP(ADDR_MODE_ABSOLUTE_Y, 4, true);
    t[0xE1] = OP(ADDR_MODE_INDIRECT_X, 6);
    t[0xF1] = OP(ADDR_MODE_INDIRECT_Y, 5, true);

    // ---- CMP ----
    t[0xC9] = OP(ADDR_MODE_IMMEDIATE,  2);
    t[0xC5] = OP(ADDR_MODE_ZERO_PAGE,  3);
    t[0xD5] = OP(ADDR_MODE_ZERO_PAGE_X,4);
    t[0xCD] = OP(ADDR_MODE_ABSOLUTE,   4);
    t[0xDD] = OP(ADDR_MODE_ABSOLUTE_X, 4, true);
    t[0xD9] = OP(ADDR_MODE_ABSOLUTE_Y, 4, true);
    t[0xC1] = OP(ADDR_MODE_INDIRECT_X, 6);
    t[0xD1] = OP(ADDR_MODE_INDIRECT_Y, 5, true);

    // ---- CPX ----
    t[0xE0] = OP(ADDR_MODE_IMMEDIATE,  2);
    t[0xE4] = OP(ADDR_MODE_ZERO_PAGE,  3);
    t[0xEC] = OP(ADDR_MODE_ABSOLUTE,   4);

    // ---- CPY ----
    t[0xC0] = OP(ADDR_MODE_IMMEDIATE,  2);
    t[0xC4] = OP(ADDR_MODE_ZERO_PAGE,  3);
    t[0xCC] = OP(ADDR_MODE_ABSOLUTE,   4);

    // ---- INC / DEC ----
    t[0xE6] = OP(ADDR_MODE_ZERO_PAGE,  5);
    t[0xF6] = OP(ADDR_MODE_ZERO_PAGE_X,6);
    t[0xEE] = OP(ADDR_MODE_ABSOLUTE,   6);
    t[0xFE] = OP(ADDR_MODE_ABSOLUTE_X, 7);
    t[0xC6] = OP(ADDR_MODE_ZERO_PAGE,  5);
    t[0xD6] = OP(ADDR_MODE_ZERO_PAGE_X,6);
    t[0xCE] = OP(ADDR_MODE_ABSOLUTE,   6);
    t[0xDE] = OP(ADDR_MODE_ABSOLUTE_X, 7);

    // ---- INX/INY/DEX/DEY ----
    t[0xE8] = OP(ADDR_MODE_IMPLIED, 2); // INX
    t[0xC8] = OP(ADDR_MODE_IMPLIED, 2); // INY
    t[0xCA] = OP(ADDR_MODE_IMPLIED, 2); // DEX
    t[0x88] = OP(ADDR_MODE_IMPLIED, 2); // DEY

    // ---- Shifts/rotates (accumulator) ----
    t[0x0A] = OP(ADDR_MODE_ACCUMULATOR, 2); // ASL A
    t[0x4A] = OP(ADDR_MODE_ACCUMULATOR, 2); // LSR A
    t[0x2A] = OP(ADDR_MODE_ACCUMULATOR, 2); // ROL A
    t[0x6A] = OP(ADDR_MODE_ACCUMULATOR, 2); // ROR A

    // ---- Shifts/rotates (zero-page / absolute / indexed) ----
    // ASL
    t[0x06] = OP(ADDR_MODE_ZERO_PAGE,  5);
    t[0x16] = OP(ADDR_MODE_ZERO_PAGE_X,6);
    t[0x0E] = OP(ADDR_MODE_ABSOLUTE,   6);
    t[0x1E] = OP(ADDR_MODE_ABSOLUTE_X, 7);
    // LSR
    t[0x46] = OP(ADDR_MODE_ZERO_PAGE,  5);
    t[0x56] = OP(ADDR_MODE_ZERO_PAGE_X,6);
    t[0x4E] = OP(ADDR_MODE_ABSOLUTE,   6);
    t[0x5E] = OP(ADDR_MODE_ABSOLUTE_X, 7);
    // ROL
    t[0x26] = OP(ADDR_MODE_ZERO_PAGE,  5);
    t[0x36] = OP(ADDR_MODE_ZERO_PAGE_X,6);
    t[0x2E] = OP(ADDR_MODE_ABSOLUTE,   6);
    t[0x3E] = OP(ADDR_MODE_ABSOLUTE_X, 7);
    // ROR
    t[0x66] = OP(ADDR_MODE_ZERO_PAGE,  5);
    t[0x76] = OP(ADDR_MODE_ZERO_PAGE_X,6);
    t[0x6E] = OP(ADDR_MODE_ABSOLUTE,   6);
    t[0x7E] = OP(ADDR_MODE_ABSOLUTE_X, 7);

    // ---- BIT ----
    t[0x24] = OP(ADDR_MODE_ZERO_PAGE, 3);
    t[0x2C] = OP(ADDR_MODE_ABSOLUTE,  4);

    // ---- Jumps ----
    t[0x4C] = OP(ADDR_MODE_ABSOLUTE,  3); // JMP abs
    t[0x6C] = OP(ADDR_MODE_INDIRECT,  5); // JMP (ind)

    // ---- Subroutines ----
    t[0x20] = OP(ADDR_MODE_ABSOLUTE,  6); // JSR
    t[0x60] = OP(ADDR_MODE_IMPLIED,   6); // RTS
    t[0x40] = OP(ADDR_MODE_IMPLIED,   6); // RTI

    // ---- Branches ----
    t[0x10] = OP(ADDR_MODE_RELATIVE, 2); // BPL
    t[0x30] = OP(ADDR_MODE_RELATIVE, 2); // BMI
    t[0x50] = OP(ADDR_MODE_RELATIVE, 2); // BVC
    t[0x70] = OP(ADDR_MODE_RELATIVE, 2); // BVS
    t[0x90] = OP(ADDR_MODE_RELATIVE, 2); // BCC
    t[0xB0] = OP(ADDR_MODE_RELATIVE, 2); // BCS
    t[0xD0] = OP(ADDR_MODE_RELATIVE, 2); // BNE
    t[0xF0] = OP(ADDR_MODE_RELATIVE, 2); // BEQ

    // ---- Flags ----
    t[0x18] = OP(ADDR_MODE_IMPLIED, 2); // CLC
    t[0x38] = OP(ADDR_MODE_IMPLIED, 2); // SEC
    t[0x58] = OP(ADDR_MODE_IMPLIED, 2); // CLI
    t[0x78] = OP(ADDR_MODE_IMPLIED, 2); // SEI
    t[0xB8] = OP(ADDR_MODE_IMPLIED, 2); // CLV
    t[0xD8] = OP(ADDR_MODE_IMPLIED, 2); // CLD
    t[0xF8] = OP(ADDR_MODE_IMPLIED, 2); // SED

    // ---- NOP ----
    t[0xEA] = OP(ADDR_MODE_IMPLIED, 2); // NOP

    // ---- BRK ----
    t[0x00] = OP(ADDR_MODE_IMMEDIATE, 7); // BRK

    // ---- KIL / JAM (halt) - unofficial but handled in execute ----
    t[0x02] = OP(ADDR_MODE_IMPLIED, 1); // KIL
    t[0x12] = OP(ADDR_MODE_IMPLIED, 1); // KIL
    t[0x22] = OP(ADDR_MODE_IMPLIED, 1); // KIL
    t[0x32] = OP(ADDR_MODE_IMPLIED, 1); // KIL
    t[0x42] = OP(ADDR_MODE_IMPLIED, 1); // KIL
    t[0x52] = OP(ADDR_MODE_IMPLIED, 1); // KIL
    t[0x62] = OP(ADDR_MODE_IMPLIED, 1); // KIL
    t[0x72] = OP(ADDR_MODE_IMPLIED, 1); // KIL
    t[0x92] = OP(ADDR_MODE_IMPLIED, 1); // KIL
    t[0xB2] = OP(ADDR_MODE_IMPLIED, 1); // KIL
    t[0xD2] = OP(ADDR_MODE_IMPLIED, 1); // KIL
    t[0xF2] = OP(ADDR_MODE_IMPLIED, 1); // KIL

    return t;
}();

} // namespace nes_cpp

#endif // NES_CORE_CPP_OPCODES_HPP
