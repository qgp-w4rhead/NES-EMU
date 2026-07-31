/*
 * bench/gen_rom.c — minimal iNES ROM generator for benchmarking.
 *
 * Synthesizes a deterministic NROM-128 ROM (mapper 0) entirely in memory
 * (no file I/O) so the harness has a stable fixture without shipping any
 * copyrighted test ROM. The bytes match the Rust Criterion bench's
 * `make_nop_cart` helper (benches/frame_bench.rs):
 *
 *   - 16-byte iNES header: "NES\x1A", PRG=1 (16 KB), CHR=1 (8 KB),
 *     mapper 0, flags 0.
 *   - 16 KB PRG-ROM filled with 0xEA (NOP).
 *   - RESET vector at $FFFC/$FFFD -> $C000 (mirrors $8000 in NROM-128).
 *   - NMI / IRQ vectors at $FFFA/$FFFB and $FFFE/$FFFF -> $0000 (unused).
 *   - 8 KB CHR-ROM zeroed (CHR-RAM equivalent for benchmarking).
 *
 * Total size: 16 + 16384 + 8192 = 24592 bytes.
 *
 * See: https://www.nesdev.org/wiki/INES
 */
#include "gen_rom.h"

#include <stdint.h>
#include <string.h>

/* iNES header constants. */
#define INES_MAGIC0 'N'
#define INES_MAGIC1 'E'
#define INES_MAGIC2 'S'
#define INES_MAGIC3 0x1A

#define PRG_ROM_UNIT 16384u /* 16 KB */
#define CHR_ROM_UNIT 8192u  /* 8 KB  */

#define NOP_ROM_TOTAL (16 + PRG_ROM_UNIT + CHR_ROM_UNIT) /* 24592 */

/* Static buffer holding the synthesized ROM. Populated once on first call
 * by `nop_rom()`. Sized exactly to the ROM so there is no file I/O and no
 * per-call allocation — the harness reads this buffer directly. */
static uint8_t g_rom[NOP_ROM_TOTAL];
static int     g_rom_built = 0;

const uint8_t* nop_rom(size_t* out_len) {
    if (!g_rom_built) {
        uint8_t* p = g_rom;

        /* Header (16 bytes). */
        p[0] = INES_MAGIC0;
        p[1] = INES_MAGIC1;
        p[2] = INES_MAGIC2;
        p[3] = INES_MAGIC3;
        p[4] = 1;  /* PRG-ROM size in 16 KB units (NROM-128). */
        p[5] = 1;  /* CHR-ROM size in 8 KB units.            */
        p[6] = 0;  /* Mapper low nibble + flags (horizontal mirroring). */
        p[7] = 0;  /* Mapper high nibble + flags. */
        /* Bytes 8..15: zeroed (PRG/CHR RAM, TV system, etc.). */
        memset(p + 8, 0, 8);

        /* PRG-ROM (16 KB) filled with NOP (0xEA). */
        memset(p + 16, 0xEA, PRG_ROM_UNIT);

        /* RESET vector at $FFFC/$FFFD -> $C000.
         * PRG starts at offset 16; $FFFC within the 16 KB bank is offset
         * 0x3FFC, so absolute offset = 16 + 0x3FFC. */
        {
            size_t reset_off = 16 + 0x3FFC;
            g_rom[reset_off]     = 0x00; /* low  byte of $C000 */
            g_rom[reset_off + 1] = 0xC0; /* high byte of $C000 */
        }
        /* NMI ($FFFA/$FFFB) and IRQ ($FFFE/$FFFF) vectors stay 0x0000
         * (already zeroed by memset above) — unused for the NOP ROM. */

        /* CHR-ROM (8 KB) zeroed. */
        memset(p + 16 + PRG_ROM_UNIT, 0x00, CHR_ROM_UNIT);

        g_rom_built = 1;
    }
    if (out_len) {
        *out_len = NOP_ROM_TOTAL;
    }
    return g_rom;
}

size_t nop_rom_size(void) {
    return NOP_ROM_TOTAL;
}
