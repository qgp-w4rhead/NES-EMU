//! Unofficial 6502 opcodes (M33) — immediate-combined + unstable-store
//! helpers.
//!
//! This module holds the helper methods for the immediate combined ops
//! (ANC, ALR, ARR, AXS, XAA) and the unstable indexed stores (TAS, AHX,
//! SHX, SHY), plus the shared page-cross-quirk address calculator. The
//! dispatch arms that call these helpers live in
//! [`super::unofficial`] (immediate ops) and [`super::unofficial_rmw`]
//! (stores).
//!
//! # Unstable opcodes
//!
//! XAA, TAS, AHX, SHX, and SHY are documented as "highly unstable" on
//! real silicon. The implementations here follow the most commonly
//! emulated model (per NESdev wiki) so the majority of test ROMs and
//! commercial titles behave correctly. Determinism is preserved.
//!
//! See: https://www.nesdev.org/wiki/CPU_unofficial_opcodes

use super::Cpu;
use crate::bus::Bus;

/// Magic constant for the unstable XAA opcode. Real silicon varies; the
/// NESdev wiki documents values of 0x00, 0xFF, 0xEE, etc. across chip
/// revisions. We use 0x00 — the most common stable approximation — so
/// `XAA #i` computes `A = (A | 0) & X & #i = A & X & #i`. This matches the
/// behavior observed on the majority of 2A03 chips and used by blargg's
/// `instr_test-v5` unofficial-opcode test ROMs.
///
/// See: https://www.nesdev.org/wiki/CPU_unofficial_opcodes#XAA
const XAA_MAGIC: u8 = 0x00;

impl Cpu {
    // ---- Immediate combined ops ---------------------------------------

    /// ANC — `A = A & imm`; then `C = N = bit 7 of A`. Two opcode aliases
    /// (0x0B and 0x2B) share this behavior.
    ///
    /// See: https://www.nesdev.org/wiki/CPU_unofficial_opcodes#ANC
    pub(crate) fn anc(&mut self, m: u8) {
        self.a &= m;
        self.set_nz(self.a);
        // C is set to the N flag (bit 7 of A).
        self.set_carry((self.a & 0x80) != 0);
    }

    /// ALR (ALR) — `A = (A & imm) >> 1`; `C = old bit 0 of (A & imm)`.
    /// N/Z from the shifted result.
    ///
    /// See: https://www.nesdev.org/wiki/CPU_unofficial_opcodes#ALR
    pub(crate) fn alr(&mut self, m: u8) {
        let v = self.a & m;
        self.set_carry((v & 0x01) != 0);
        let result = v >> 1;
        self.a = result;
        self.set_nz(result);
    }

    /// ARR — `A = (A & imm)` then `A = ROR A`, but with special V and C
    /// flag behavior that differs from a plain ROR:
    /// - `C` = bit 6 of the result.
    /// - `V` = bit 6 XOR bit 5 of the result.
    ///
    /// N/Z are set from the result as usual.
    ///
    /// See: https://www.nesdev.org/wiki/CPU_unofficial_opcodes#ARR
    pub(crate) fn arr(&mut self, m: u8) {
        let v = self.a & m;
        // ROR using the current carry.
        let result = (v >> 1) | (u8::from(self.carry()) << 7);
        self.a = result;
        self.set_nz(result);
        // Special C and V per NESdev.
        self.set_carry((result & 0x40) != 0);
        self.set_overflow(((result ^ (result << 1)) & 0x40) != 0);
    }

    /// AXS (SBX) — `X = (A & X) - imm`. Sets C when no borrow
    /// (`(A & X) >= imm`), and N/Z from the result. A is unchanged.
    ///
    /// See: https://www.nesdev.org/wiki/CPU_unofficial_opcodes#AXS
    pub(crate) fn axs(&mut self, m: u8) {
        let ax = self.a & self.x;
        self.set_carry(ax >= m);
        let result = ax.wrapping_sub(m);
        self.x = result;
        self.set_nz(result);
    }

    /// XAA — unstable. Computes `A = (A | XAA_MAGIC) & X & imm`. With
    /// `XAA_MAGIC = 0` this is `A = A & X & imm`. Sets N/Z from the result.
    ///
    /// See: https://www.nesdev.org/wiki/CPU_unofficial_opcodes#XAA
    pub(crate) fn xaa(&mut self, m: u8) {
        let result = (self.a | XAA_MAGIC) & self.x & m;
        self.a = result;
        self.set_nz(result);
    }

    // ---- Unstable indexed stores --------------------------------------
    //
    // TAS, AHX, SHX, and SHY all compute a value that is ANDed with
    // `(high byte of base address) + 1`, then store it. On a page cross,
    // the high byte of the effective address is *replaced* with the value
    // being stored (a quirk of the 6502's address-add unit during these
    // unstable opcodes). We model that quirk so the store lands at the
    // same address real silicon would target.
    //
    // See: https://www.nesdev.org/wiki/CPU_unofficial_opcodes#SHA
    // See: https://www.nesdev.org/wiki/CPU_unofficial_opcodes#SHX
    // See: https://www.nesdev.org/wiki/CPU_unofficial_opcodes#SHY
    // See: https://www.nesdev.org/wiki/CPU_unofficial_opcodes#TAS

    /// TAS (SHS) — `SP = A & X`, then store `SP & (H + 1)` at `base + Y`
    /// (with the page-cross quirk). `H` is the high byte of `base`.
    pub(crate) fn tas_store(&mut self, bus: &mut Bus, base: u16) {
        // Set SP = A & X first.
        self.sp = self.a & self.x;
        let h = (base >> 8) as u8;
        let value = self.sp & h.wrapping_add(1);
        let store_addr = self.quirk_addr_y(base, value);
        // Issue the standard store dummy read at the page-wrap address so
        // read-sensitive devices see the side-effect at the right cycle.
        let _ = bus.read(self.store_dummy_read_addr(base, self.y));
        bus.write(store_addr, value);
    }

    /// AHX (SHA) — store `reg & (H + 1)` at `base + Y` with the page-cross
    /// quirk. `reg` is `A & X` for AHX proper or `X` for SHX (both reuse
    /// this helper).
    pub(crate) fn ahx_store(&mut self, bus: &mut Bus, base: u16, reg: u8) {
        let h = (base >> 8) as u8;
        let value = reg & h.wrapping_add(1);
        let store_addr = self.quirk_addr_y(base, value);
        let _ = bus.read(self.store_dummy_read_addr(base, self.y));
        bus.write(store_addr, value);
    }

    /// SHY (SYA) — store `Y & (H + 1)` at `base + X` with the page-cross
    /// quirk (indexed by X, not Y).
    pub(crate) fn shy_store(&mut self, bus: &mut Bus, base: u16) {
        let h = (base >> 8) as u8;
        let value = self.y & h.wrapping_add(1);
        let store_addr = self.quirk_addr_x(base, value);
        let _ = bus.read(self.store_dummy_read_addr(base, self.x));
        bus.write(store_addr, value);
    }

    /// Compute the dummy-read address for an unstable indexed store: the
    /// high byte of `base` ORed with the low byte of `base + index`. This
    /// is the page-wrap address the 6502 reads as a stepping stone before
    /// the real store, and it fires read side-effects on devices like
    /// PPUSTATUS / OAMDATA / PPUDATA at the correct cycle. Shared by
    /// `tas_store`, `ahx_store` (both Y-indexed) and `shy_store`
    /// (X-indexed) to avoid divergent dummy-read formulas.
    fn store_dummy_read_addr(&self, base: u16, index: u8) -> u16 {
        (base & 0xFF00) | ((base.wrapping_add(index as u16)) & 0x00FF)
    }

    /// Compute the effective store address for a Y-indexed unstable store,
    /// applying the page-cross quirk: when `base + Y` crosses a page
    /// boundary, the high byte of the store address becomes `value` (the
    /// AND result being stored) instead of the normal carry-incremented
    /// high byte.
    pub(crate) fn quirk_addr_y(&self, base: u16, value: u8) -> u16 {
        let eff = base.wrapping_add(self.y as u16);
        if (base & 0xFF00) != (eff & 0xFF00) {
            // Page cross: high byte = value being stored.
            ((value as u16) << 8) | (eff & 0x00FF)
        } else {
            eff
        }
    }

    /// Same as [`quirk_addr_y`] but indexed by X (used by SHY).
    pub(crate) fn quirk_addr_x(&self, base: u16, value: u8) -> u16 {
        let eff = base.wrapping_add(self.x as u16);
        if (base & 0xFF00) != (eff & 0xFF00) {
            ((value as u16) << 8) | (eff & 0x00FF)
        } else {
            eff
        }
    }

    /// Read the 16-bit base pointer for `(zp),Y` addressing without adding
    /// Y. Used by AHX `(ind),Y` so we can apply the page-cross quirk on
    /// the full base + Y. PC has already advanced past the opcode byte;
    /// this fetches the zero-page operand byte and the two pointer bytes.
    ///
    /// This exists separately from `am_indirect_y_rmw` (in
    /// [`super::addressing`]) because the unstable stores issue their own
    /// dummy read inside `ahx_store`/`shy_store` via
    /// [`store_dummy_read_addr`]; reusing `am_indirect_y_rmw` would fire a
    /// *second* dummy read at a different (page-wrap) address, double-
    /// counting side-effects on read-sensitive devices.
    pub(crate) fn am_indirect_y_base(&mut self, bus: &mut Bus) -> u16 {
        let zp = self.fetch_byte(bus);
        let lo = bus.read(zp as u16);
        let hi = bus.read(zp.wrapping_add(1) as u16);
        (lo as u16) | ((hi as u16) << 8)
    }
}
