/*
 * bench/gen_rom.h — in-memory NOP ROM fixture for the benchmark harness.
 */
#ifndef NES_BENCH_GEN_ROM_H
#define NES_BENCH_GEN_ROM_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/*
 * Return a pointer to the static NROM-128 NOP ROM (24592 bytes). The buffer
 * is built once on first call and lives for the program lifetime — no file
 * I/O, no per-call allocation. `out_len` (if non-NULL) receives the ROM
 * length in bytes.
 */
const uint8_t* nop_rom(size_t* out_len);

/* ROM length in bytes (24592). Constant; does not trigger a build. */
size_t nop_rom_size(void);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* NES_BENCH_GEN_ROM_H */
