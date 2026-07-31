/*
 * bench/harness.h — cross-language NES core benchmark harness.
 *
 * The harness loads a shared library implementing cores/protocol.h, drives a
 * fixed ROM for N frames, and records per-frame timing. This is a placeholder
 * scaffold; the full harness is built in M3.
 */
#ifndef NES_BENCH_HARNESS_H
#define NES_BENCH_HARNESS_H

#include "../cores/protocol.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Benchmark a single core shared library against a ROM for `frames` frames.
 * Returns 0 on success, non-zero on error. Timing results are printed to
 * stdout in a machine-readable format. Implemented in harness.c (M3). */
int bench_run(const char* lib_path, const uint8_t* rom, size_t rom_len,
              uint32_t frames);

#ifdef __cplusplus
}
#endif

#endif /* NES_BENCH_HARNESS_H */
