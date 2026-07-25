//! 6502 disassembler — static opcode table + instruction formatting.
//!
//! Uses side-effect-free `Bus::peek`; unofficial opcodes decode as `???`.

use crate::bus::Bus;
use crate::cpu::AddrMode;

/// One entry in the static opcode table.
#[derive(Clone, Copy)]
struct OpcodeEntry {
    mnemonic: &'static str,
    mode: AddrMode,
    len: u8,
}

/// Byte length of an instruction for the given addressing mode.
const fn mode_len(mode: AddrMode) -> u8 {
    match mode {
        AddrMode::Implied | AddrMode::Accumulator => 1,
        // BRK is Implied but has a signature byte (2 bytes total); handled
        // specially in the table below by overriding `len`.
        AddrMode::Immediate
        | AddrMode::ZeroPage
        | AddrMode::ZeroPageX
        | AddrMode::ZeroPageY
        | AddrMode::IndirectX
        | AddrMode::IndirectY
        | AddrMode::Relative => 2,
        AddrMode::Absolute | AddrMode::AbsoluteX | AddrMode::AbsoluteY | AddrMode::Indirect => 3,
    }
}

/// Build an opcode table entry with the default length for its mode.
const fn op(mnemonic: &'static str, mode: AddrMode) -> OpcodeEntry {
    OpcodeEntry {
        mnemonic,
        mode,
        len: mode_len(mode),
    }
}

/// BRK is Implied but carries a signature byte → 2 bytes total.
const fn brk() -> OpcodeEntry {
    OpcodeEntry {
        mnemonic: "BRK",
        mode: AddrMode::Implied,
        len: 2,
    }
}

/// Placeholder for unofficial / undefined opcodes.
const fn unk() -> OpcodeEntry {
    OpcodeEntry {
        mnemonic: "???",
        mode: AddrMode::Implied,
        len: 1,
    }
}

/// The full 256-entry opcode table. Indexed by opcode byte.
///
/// Generated from the official 6502 opcode map (see
/// <https://www.nesdev.org/6502.txt>). Unofficial opcodes decode as `???`
/// with length 1 so the disassembler skips one byte and resynchronises.
static OPCODES: [OpcodeEntry; 256] = {
    // Start with every entry as `unk()`, then fill in the 151 official
    // opcodes. `OpcodeEntry` is `Copy` and `unk()` is `const fn`, so
    // `[unk(); 256]` is const-evaluable and collapses 256 lines of `unk(),` into one.
    let mut t: [OpcodeEntry; 256] = [unk(); 256];

    // ---- ADC ----
    t[0x69] = op("ADC", AddrMode::Immediate);
    t[0x65] = op("ADC", AddrMode::ZeroPage);
    t[0x75] = op("ADC", AddrMode::ZeroPageX);
    t[0x6D] = op("ADC", AddrMode::Absolute);
    t[0x7D] = op("ADC", AddrMode::AbsoluteX);
    t[0x79] = op("ADC", AddrMode::AbsoluteY);
    t[0x61] = op("ADC", AddrMode::IndirectX);
    t[0x71] = op("ADC", AddrMode::IndirectY);
    // ---- AND ----
    t[0x29] = op("AND", AddrMode::Immediate);
    t[0x25] = op("AND", AddrMode::ZeroPage);
    t[0x35] = op("AND", AddrMode::ZeroPageX);
    t[0x2D] = op("AND", AddrMode::Absolute);
    t[0x3D] = op("AND", AddrMode::AbsoluteX);
    t[0x39] = op("AND", AddrMode::AbsoluteY);
    t[0x21] = op("AND", AddrMode::IndirectX);
    t[0x31] = op("AND", AddrMode::IndirectY);
    // ---- ASL ----
    t[0x0A] = op("ASL", AddrMode::Accumulator);
    t[0x06] = op("ASL", AddrMode::ZeroPage);
    t[0x16] = op("ASL", AddrMode::ZeroPageX);
    t[0x0E] = op("ASL", AddrMode::Absolute);
    t[0x1E] = op("ASL", AddrMode::AbsoluteX);
    // ---- Branches ----
    t[0x90] = op("BCC", AddrMode::Relative);
    t[0xB0] = op("BCS", AddrMode::Relative);
    t[0xF0] = op("BEQ", AddrMode::Relative);
    t[0x30] = op("BMI", AddrMode::Relative);
    t[0xD0] = op("BNE", AddrMode::Relative);
    t[0x10] = op("BPL", AddrMode::Relative);
    t[0x50] = op("BVC", AddrMode::Relative);
    t[0x70] = op("BVS", AddrMode::Relative);
    // ---- BIT ----
    t[0x24] = op("BIT", AddrMode::ZeroPage);
    t[0x2C] = op("BIT", AddrMode::Absolute);
    // ---- BRK ----
    t[0x00] = brk();
    // ---- Flag ops ----
    t[0x18] = op("CLC", AddrMode::Implied);
    t[0xD8] = op("CLD", AddrMode::Implied);
    t[0x58] = op("CLI", AddrMode::Implied);
    t[0xB8] = op("CLV", AddrMode::Implied);
    t[0x38] = op("SEC", AddrMode::Implied);
    t[0xF8] = op("SED", AddrMode::Implied);
    t[0x78] = op("SEI", AddrMode::Implied);
    // ---- CMP ----
    t[0xC9] = op("CMP", AddrMode::Immediate);
    t[0xC5] = op("CMP", AddrMode::ZeroPage);
    t[0xD5] = op("CMP", AddrMode::ZeroPageX);
    t[0xCD] = op("CMP", AddrMode::Absolute);
    t[0xDD] = op("CMP", AddrMode::AbsoluteX);
    t[0xD9] = op("CMP", AddrMode::AbsoluteY);
    t[0xC1] = op("CMP", AddrMode::IndirectX);
    t[0xD1] = op("CMP", AddrMode::IndirectY);
    // ---- CPX / CPY ----
    t[0xE0] = op("CPX", AddrMode::Immediate);
    t[0xE4] = op("CPX", AddrMode::ZeroPage);
    t[0xEC] = op("CPX", AddrMode::Absolute);
    t[0xC0] = op("CPY", AddrMode::Immediate);
    t[0xC4] = op("CPY", AddrMode::ZeroPage);
    t[0xCC] = op("CPY", AddrMode::Absolute);
    // ---- DEC / DEX / DEY ----
    t[0xC6] = op("DEC", AddrMode::ZeroPage);
    t[0xD6] = op("DEC", AddrMode::ZeroPageX);
    t[0xCE] = op("DEC", AddrMode::Absolute);
    t[0xDE] = op("DEC", AddrMode::AbsoluteX);
    t[0xCA] = op("DEX", AddrMode::Implied);
    t[0x88] = op("DEY", AddrMode::Implied);
    // ---- EOR ----
    t[0x49] = op("EOR", AddrMode::Immediate);
    t[0x45] = op("EOR", AddrMode::ZeroPage);
    t[0x55] = op("EOR", AddrMode::ZeroPageX);
    t[0x4D] = op("EOR", AddrMode::Absolute);
    t[0x5D] = op("EOR", AddrMode::AbsoluteX);
    t[0x59] = op("EOR", AddrMode::AbsoluteY);
    t[0x41] = op("EOR", AddrMode::IndirectX);
    t[0x51] = op("EOR", AddrMode::IndirectY);
    // ---- INC / INX / INY ----
    t[0xE6] = op("INC", AddrMode::ZeroPage);
    t[0xF6] = op("INC", AddrMode::ZeroPageX);
    t[0xEE] = op("INC", AddrMode::Absolute);
    t[0xFE] = op("INC", AddrMode::AbsoluteX);
    t[0xE8] = op("INX", AddrMode::Implied);
    t[0xC8] = op("INY", AddrMode::Implied);
    // ---- JMP / JSR ----
    t[0x4C] = op("JMP", AddrMode::Absolute);
    t[0x6C] = op("JMP", AddrMode::Indirect);
    t[0x20] = op("JSR", AddrMode::Absolute);
    // ---- LDA ----
    t[0xA9] = op("LDA", AddrMode::Immediate);
    t[0xA5] = op("LDA", AddrMode::ZeroPage);
    t[0xB5] = op("LDA", AddrMode::ZeroPageX);
    t[0xAD] = op("LDA", AddrMode::Absolute);
    t[0xBD] = op("LDA", AddrMode::AbsoluteX);
    t[0xB9] = op("LDA", AddrMode::AbsoluteY);
    t[0xA1] = op("LDA", AddrMode::IndirectX);
    t[0xB1] = op("LDA", AddrMode::IndirectY);
    // ---- LDX ----
    t[0xA2] = op("LDX", AddrMode::Immediate);
    t[0xA6] = op("LDX", AddrMode::ZeroPage);
    t[0xB6] = op("LDX", AddrMode::ZeroPageY);
    t[0xAE] = op("LDX", AddrMode::Absolute);
    t[0xBE] = op("LDX", AddrMode::AbsoluteY);
    // ---- LDY ----
    t[0xA0] = op("LDY", AddrMode::Immediate);
    t[0xA4] = op("LDY", AddrMode::ZeroPage);
    t[0xB4] = op("LDY", AddrMode::ZeroPageX);
    t[0xAC] = op("LDY", AddrMode::Absolute);
    t[0xBC] = op("LDY", AddrMode::AbsoluteX);
    // ---- LSR ----
    t[0x4A] = op("LSR", AddrMode::Accumulator);
    t[0x46] = op("LSR", AddrMode::ZeroPage);
    t[0x56] = op("LSR", AddrMode::ZeroPageX);
    t[0x4E] = op("LSR", AddrMode::Absolute);
    t[0x5E] = op("LSR", AddrMode::AbsoluteX);
    // ---- NOP ----
    t[0xEA] = op("NOP", AddrMode::Implied);
    // ---- ORA ----
    t[0x09] = op("ORA", AddrMode::Immediate);
    t[0x05] = op("ORA", AddrMode::ZeroPage);
    t[0x15] = op("ORA", AddrMode::ZeroPageX);
    t[0x0D] = op("ORA", AddrMode::Absolute);
    t[0x1D] = op("ORA", AddrMode::AbsoluteX);
    t[0x19] = op("ORA", AddrMode::AbsoluteY);
    t[0x01] = op("ORA", AddrMode::IndirectX);
    t[0x11] = op("ORA", AddrMode::IndirectY);
    // ---- Stack ----
    t[0x48] = op("PHA", AddrMode::Implied);
    t[0x08] = op("PHP", AddrMode::Implied);
    t[0x68] = op("PLA", AddrMode::Implied);
    t[0x28] = op("PLP", AddrMode::Implied);
    // ---- ROL / ROR ----
    t[0x2A] = op("ROL", AddrMode::Accumulator);
    t[0x26] = op("ROL", AddrMode::ZeroPage);
    t[0x36] = op("ROL", AddrMode::ZeroPageX);
    t[0x2E] = op("ROL", AddrMode::Absolute);
    t[0x3E] = op("ROL", AddrMode::AbsoluteX);
    t[0x6A] = op("ROR", AddrMode::Accumulator);
    t[0x66] = op("ROR", AddrMode::ZeroPage);
    t[0x76] = op("ROR", AddrMode::ZeroPageX);
    t[0x6E] = op("ROR", AddrMode::Absolute);
    t[0x7E] = op("ROR", AddrMode::AbsoluteX);
    // ---- RTI / RTS ----
    t[0x40] = op("RTI", AddrMode::Implied);
    t[0x60] = op("RTS", AddrMode::Implied);
    // ---- SBC ----
    t[0xE9] = op("SBC", AddrMode::Immediate);
    t[0xE5] = op("SBC", AddrMode::ZeroPage);
    t[0xF5] = op("SBC", AddrMode::ZeroPageX);
    t[0xED] = op("SBC", AddrMode::Absolute);
    t[0xFD] = op("SBC", AddrMode::AbsoluteX);
    t[0xF9] = op("SBC", AddrMode::AbsoluteY);
    t[0xE1] = op("SBC", AddrMode::IndirectX);
    t[0xF1] = op("SBC", AddrMode::IndirectY);
    // ---- STA ----
    t[0x85] = op("STA", AddrMode::ZeroPage);
    t[0x95] = op("STA", AddrMode::ZeroPageX);
    t[0x8D] = op("STA", AddrMode::Absolute);
    t[0x9D] = op("STA", AddrMode::AbsoluteX);
    t[0x99] = op("STA", AddrMode::AbsoluteY);
    t[0x81] = op("STA", AddrMode::IndirectX);
    t[0x91] = op("STA", AddrMode::IndirectY);
    // ---- STX / STY ----
    t[0x86] = op("STX", AddrMode::ZeroPage);
    t[0x96] = op("STX", AddrMode::ZeroPageY);
    t[0x8E] = op("STX", AddrMode::Absolute);
    t[0x84] = op("STY", AddrMode::ZeroPage);
    t[0x94] = op("STY", AddrMode::ZeroPageX);
    t[0x8C] = op("STY", AddrMode::Absolute);
    // ---- Transfers ----
    t[0xAA] = op("TAX", AddrMode::Implied);
    t[0xA8] = op("TAY", AddrMode::Implied);
    t[0xBA] = op("TSX", AddrMode::Implied);
    t[0x8A] = op("TXA", AddrMode::Implied);
    t[0x9A] = op("TXS", AddrMode::Implied);
    t[0x98] = op("TYA", AddrMode::Implied);

    // ---- Unofficial opcodes (M33) ------------------------------------
    // NOP variants. Implied NOPs (1-byte).
    t[0x1A] = op("NOP", AddrMode::Implied);
    t[0x3A] = op("NOP", AddrMode::Implied);
    t[0x5A] = op("NOP", AddrMode::Implied);
    t[0x7A] = op("NOP", AddrMode::Implied);
    t[0xDA] = op("NOP", AddrMode::Implied);
    t[0xFA] = op("NOP", AddrMode::Implied);
    // Immediate NOPs (2-byte, "NOP #imm").
    t[0x80] = op("NOP", AddrMode::Immediate);
    t[0x82] = op("NOP", AddrMode::Immediate);
    t[0x89] = op("NOP", AddrMode::Immediate);
    t[0xC2] = op("NOP", AddrMode::Immediate);
    t[0xE2] = op("NOP", AddrMode::Immediate);
    // Zero-page NOPs (2-byte).
    t[0x04] = op("NOP", AddrMode::ZeroPage);
    t[0x44] = op("NOP", AddrMode::ZeroPage);
    t[0x64] = op("NOP", AddrMode::ZeroPage);
    // Zero-page,X NOPs (2-byte).
    t[0x14] = op("NOP", AddrMode::ZeroPageX);
    t[0x34] = op("NOP", AddrMode::ZeroPageX);
    t[0x54] = op("NOP", AddrMode::ZeroPageX);
    t[0x74] = op("NOP", AddrMode::ZeroPageX);
    t[0xD4] = op("NOP", AddrMode::ZeroPageX);
    t[0xF4] = op("NOP", AddrMode::ZeroPageX);
    // Absolute NOP (3-byte).
    t[0x0C] = op("NOP", AddrMode::Absolute);
    // Absolute,X NOPs (3-byte).
    t[0x1C] = op("NOP", AddrMode::AbsoluteX);
    t[0x3C] = op("NOP", AddrMode::AbsoluteX);
    t[0x5C] = op("NOP", AddrMode::AbsoluteX);
    t[0x7C] = op("NOP", AddrMode::AbsoluteX);
    t[0xDC] = op("NOP", AddrMode::AbsoluteX);
    t[0xFC] = op("NOP", AddrMode::AbsoluteX);

    // LAX (load A and X).
    t[0xA7] = op("LAX", AddrMode::ZeroPage);
    t[0xB7] = op("LAX", AddrMode::ZeroPageY);
    t[0xAF] = op("LAX", AddrMode::Absolute);
    t[0xBF] = op("LAX", AddrMode::AbsoluteY);
    t[0xA3] = op("LAX", AddrMode::IndirectX);
    t[0xB3] = op("LAX", AddrMode::IndirectY);

    // SAX (store A & X).
    t[0x87] = op("SAX", AddrMode::ZeroPage);
    t[0x97] = op("SAX", AddrMode::ZeroPageY);
    t[0x8F] = op("SAX", AddrMode::Absolute);
    t[0x83] = op("SAX", AddrMode::IndirectX);

    // DCP (DEC then CMP).
    t[0xC7] = op("DCP", AddrMode::ZeroPage);
    t[0xD7] = op("DCP", AddrMode::ZeroPageX);
    t[0xCF] = op("DCP", AddrMode::Absolute);
    t[0xDF] = op("DCP", AddrMode::AbsoluteX);
    t[0xDB] = op("DCP", AddrMode::AbsoluteY);
    t[0xC3] = op("DCP", AddrMode::IndirectX);
    t[0xD3] = op("DCP", AddrMode::IndirectY);

    // ISC (INC then SBC).
    t[0xE7] = op("ISC", AddrMode::ZeroPage);
    t[0xF7] = op("ISC", AddrMode::ZeroPageX);
    t[0xEF] = op("ISC", AddrMode::Absolute);
    t[0xFF] = op("ISC", AddrMode::AbsoluteX);
    t[0xFB] = op("ISC", AddrMode::AbsoluteY);
    t[0xE3] = op("ISC", AddrMode::IndirectX);
    t[0xF3] = op("ISC", AddrMode::IndirectY);

    // SLO (ASL then ORA).
    t[0x07] = op("SLO", AddrMode::ZeroPage);
    t[0x17] = op("SLO", AddrMode::ZeroPageX);
    t[0x0F] = op("SLO", AddrMode::Absolute);
    t[0x1F] = op("SLO", AddrMode::AbsoluteX);
    t[0x1B] = op("SLO", AddrMode::AbsoluteY);
    t[0x03] = op("SLO", AddrMode::IndirectX);
    t[0x13] = op("SLO", AddrMode::IndirectY);

    // RLA (ROL then AND).
    t[0x27] = op("RLA", AddrMode::ZeroPage);
    t[0x37] = op("RLA", AddrMode::ZeroPageX);
    t[0x2F] = op("RLA", AddrMode::Absolute);
    t[0x3F] = op("RLA", AddrMode::AbsoluteX);
    t[0x3B] = op("RLA", AddrMode::AbsoluteY);
    t[0x23] = op("RLA", AddrMode::IndirectX);
    t[0x33] = op("RLA", AddrMode::IndirectY);

    // SRE (LSR then EOR).
    t[0x47] = op("SRE", AddrMode::ZeroPage);
    t[0x57] = op("SRE", AddrMode::ZeroPageX);
    t[0x4F] = op("SRE", AddrMode::Absolute);
    t[0x5F] = op("SRE", AddrMode::AbsoluteX);
    t[0x5B] = op("SRE", AddrMode::AbsoluteY);
    t[0x43] = op("SRE", AddrMode::IndirectX);
    t[0x53] = op("SRE", AddrMode::IndirectY);

    // RRA (ROR then ADC).
    t[0x67] = op("RRA", AddrMode::ZeroPage);
    t[0x77] = op("RRA", AddrMode::ZeroPageX);
    t[0x6F] = op("RRA", AddrMode::Absolute);
    t[0x7F] = op("RRA", AddrMode::AbsoluteX);
    t[0x7B] = op("RRA", AddrMode::AbsoluteY);
    t[0x63] = op("RRA", AddrMode::IndirectX);
    t[0x73] = op("RRA", AddrMode::IndirectY);

    // Immediate combined ops.
    t[0x0B] = op("ANC", AddrMode::Immediate);
    t[0x2B] = op("ANC", AddrMode::Immediate);
    t[0x4B] = op("ALR", AddrMode::Immediate);
    t[0x6B] = op("ARR", AddrMode::Immediate);
    t[0xCB] = op("AXS", AddrMode::Immediate);
    t[0x8B] = op("XAA", AddrMode::Immediate);

    // Unstable indexed stores.
    t[0x9B] = op("TAS", AddrMode::AbsoluteY);
    t[0x9F] = op("AHX", AddrMode::AbsoluteY);
    t[0x93] = op("AHX", AddrMode::IndirectY);
    t[0x9E] = op("SHX", AddrMode::AbsoluteY);
    t[0x9C] = op("SHY", AddrMode::AbsoluteX);
    t[0xBB] = op("LAS", AddrMode::AbsoluteY);

    // KIL / JAM / HLT (1-byte, halts the CPU).
    t[0x02] = op("KIL", AddrMode::Implied);
    t[0x12] = op("KIL", AddrMode::Implied);
    t[0x22] = op("KIL", AddrMode::Implied);
    t[0x32] = op("KIL", AddrMode::Implied);
    t[0x42] = op("KIL", AddrMode::Implied);
    t[0x52] = op("KIL", AddrMode::Implied);
    t[0x62] = op("KIL", AddrMode::Implied);
    t[0x72] = op("KIL", AddrMode::Implied);
    t[0x92] = op("KIL", AddrMode::Implied);
    t[0xB2] = op("KIL", AddrMode::Implied);
    t[0xD2] = op("KIL", AddrMode::Implied);
    t[0xF2] = op("KIL", AddrMode::Implied);

    t
};

/// A single disassembled instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisassembledInstruction {
    /// Address of the opcode byte.
    pub addr: u16,
    /// Raw opcode + operand bytes (up to 3). Bytes beyond `len` are 0.
    pub bytes: [u8; 3],
    /// Number of valid bytes in `bytes` (1, 2, or 3).
    pub len: u8,
    /// Formatted disassembly text, e.g. `LDA #$44`, `STA $0200,X`.
    pub text: String,
    /// Address of the next instruction (`addr + len`, wrapping).
    pub next_pc: u16,
}

/// Disassemble one instruction starting at `addr`, reading operand bytes
/// via [`Bus::peek`] (no side-effects). The returned `next_pc` is
/// `addr + len` (16-bit wrapping) so the caller can chain calls for a
/// disassembly window.
pub fn disassemble_at(bus: &Bus, addr: u16) -> DisassembledInstruction {
    let opcode = bus.peek(addr);
    let entry = OPCODES[opcode as usize];
    let len = entry.len as u16;
    let b0 = opcode;
    let b1 = if len >= 2 {
        bus.peek(addr.wrapping_add(1))
    } else {
        0
    };
    let b2 = if len >= 3 {
        bus.peek(addr.wrapping_add(2))
    } else {
        0
    };
    let text = format_instruction(entry.mnemonic, entry.mode, addr, b0, b1, b2);
    DisassembledInstruction {
        addr,
        bytes: [b0, b1, b2],
        len: entry.len,
        text,
        next_pc: addr.wrapping_add(len),
    }
}

/// Disassemble `count` consecutive instructions starting at `start`,
/// stopping early if the address wraps past `$FFFF`. Useful for the
/// debugger's "current + next N" disassembly window.
pub fn disassemble_window(bus: &Bus, start: u16, count: usize) -> Vec<DisassembledInstruction> {
    let mut out = Vec::with_capacity(count);
    let mut addr = start;
    for _ in 0..count {
        let instr = disassemble_at(bus, addr);
        // Stop if we have wrapped around the top of address space.
        // (Instruction length is always ≥1, so next_pc <= addr implies a wrap.)
        if instr.next_pc <= addr {
            out.push(instr);
            break;
        }
        addr = instr.next_pc;
        out.push(instr);
    }
    out
}

/// Format an instruction's text given its mnemonic, addressing mode, and
/// raw bytes. `addr` is the instruction's address (used for relative
/// branch target calculation).
fn format_instruction(
    mnemonic: &str,
    mode: AddrMode,
    addr: u16,
    _b0: u8,
    b1: u8,
    b2: u8,
) -> String {
    match mode {
        AddrMode::Implied => mnemonic.to_string(),
        AddrMode::Accumulator => format!("{mnemonic} A"),
        AddrMode::Immediate => format!("{mnemonic} #${b1:02X}"),
        AddrMode::ZeroPage => format!("{mnemonic} ${b1:02X}"),
        AddrMode::ZeroPageX => format!("{mnemonic} ${b1:02X},X"),
        AddrMode::ZeroPageY => format!("{mnemonic} ${b1:02X},Y"),
        AddrMode::Absolute => format!("{mnemonic} ${b2:02X}{b1:02X}"),
        AddrMode::AbsoluteX => format!("{mnemonic} ${b2:02X}{b1:02X},X"),
        AddrMode::AbsoluteY => format!("{mnemonic} ${b2:02X}{b1:02X},Y"),
        AddrMode::Indirect => format!("{mnemonic} (${b2:02X}{b1:02X})"),
        AddrMode::IndirectX => format!("{mnemonic} (${b1:02X},X)"),
        AddrMode::IndirectY => format!("{mnemonic} (${b1:02X}),Y"),
        AddrMode::Relative => {
            // Branch target = PC (after the offset byte) + signed offset.
            // PC after fetching opcode+offset = addr + 2.
            let target = (addr.wrapping_add(2)).wrapping_add(b1 as i8 as u16);
            format!("{mnemonic} ${target:04X}")
        }
    }
}
