/*
 * bench/gen_rom.c — minimal iNES ROM generator for benchmarking (scaffold).
 *
 * Generates a tiny valid iNES ROM (NROM mapper 0) so the harness has a
 * deterministic fixture without shipping copyrighted test ROMs. Full
 * implementation lands in M3. For now this just confirms the file compiles
 * against the protocol header.
 */
#include "../cores/protocol.h"

#include <stdint.h>
#include <stdio.h>

/* Placeholder: M3 will synthesize a 16-byte iNES header + minimal PRG/CHR
 * banks. Kept as a no-op so the scaffold links. */
int gen_rom_main(void) {
    printf("gen_rom scaffold ok\n");
    return 0;
}
