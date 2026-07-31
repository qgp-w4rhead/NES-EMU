/*
 * bench/harness.c — cross-language NES core benchmark harness (scaffold).
 *
 * This is a placeholder for M3. It includes the protocol header to ensure the
 * ABI compiles cleanly under a C compiler. The full implementation (dynamic
 * loading of per-language cores, frame timing, result aggregation) lands in
 * milestone M3.
 */
#include "harness.h"

#include <stdint.h>
#include <stdio.h>

/* Stub entry kept so the translation unit links. Real work is M3. */
int bench_run(const char* lib_path, const uint8_t* rom, size_t rom_len,
              uint32_t frames) {
    (void)lib_path;
    (void)rom;
    (void)rom_len;
    (void)frames;
    /* M3 will dlopen(lib_path), resolve the nes_core_* symbols, and drive. */
    return 1;
}

/* Smoke-test main: verify the protocol symbols are visible to the linker by
 * taking their addresses. This keeps the harness compiling against the
 * header even before any core exists. */
int main(void) {
    /* Reference the function pointer types from protocol.h to confirm the
     * header is usable from C. We never call these (no core is built yet). */
    nes_core_t* (*create_fn)(const uint8_t*, size_t) = nes_core_create;
    void (*destroy_fn)(nes_core_t*) = nes_core_destroy;
    (void)create_fn;
    (void)destroy_fn;
    printf("harness scaffold ok (impl_name=%s)\n", nes_core_impl_name());
    return 0;
}
