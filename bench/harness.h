/*
 * bench/harness.h — cross-language NES core benchmark harness.
 *
 * Defines the function-pointer typedefs that mirror `cores/protocol.h` so
 * the harness can resolve a core's symbols at runtime (LoadLibrary/dlopen)
 * into a `nes_core_api` struct and call them through a single vtable.
 *
 * The harness drives any core (Rust cdylib, C/C++/Zig/Go/C#/Java shared
 * lib, or a TypeScript/Python subprocess speaking the binary protocol in
 * M3.md) through this vtable and records per-benchmark timing.
 */
#ifndef NES_BENCH_HARNESS_H
#define NES_BENCH_HARNESS_H

#include "../cores/protocol.h"

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* -------------------------------------------------------------------------
 * Function-pointer typedefs matching every symbol in cores/protocol.h.
 * Each typedef is the exact pointer-to-function type for the corresponding
 * `nes_core_*` declaration. The harness fills a `nes_core_api` struct with
 * these via GetProcAddress / dlsym and never links against a core directly.
 * ------------------------------------------------------------------------- */

typedef nes_core_t* (*nes_core_create_fn)(const uint8_t* rom_data, size_t rom_len);
typedef void        (*nes_core_destroy_fn)(nes_core_t* core);
typedef void        (*nes_core_reset_fn)(nes_core_t* core);
typedef int         (*nes_core_set_region_fn)(nes_core_t* core, int region);
typedef uint32_t    (*nes_core_step_frame_fn)(nes_core_t* core);
typedef uint32_t    (*nes_core_step_instruction_fn)(nes_core_t* core);
typedef const uint32_t* (*nes_core_framebuffer_fn)(nes_core_t* core);
typedef size_t      (*nes_core_take_audio_fn)(nes_core_t* core, int16_t* buf, size_t cap);
typedef size_t      (*nes_core_save_state_fn)(nes_core_t* core, uint8_t* buf, size_t cap);
typedef int         (*nes_core_load_state_fn)(nes_core_t* core, const uint8_t* buf, size_t len);
typedef uint16_t    (*nes_core_mapper_number_fn)(nes_core_t* core);
typedef const char* (*nes_core_impl_name_fn)(void);
typedef const char* (*nes_core_impl_version_fn)(void);

/*
 * Resolved symbol table for one loaded core. Populated by `core_load` for
 * shared-library cores; for subprocess cores the harness wraps the binary
 * protocol behind the same struct so the benchmark loop is identical.
 */
typedef struct {
    nes_core_create_fn          create;
    nes_core_destroy_fn         destroy;
    nes_core_reset_fn           reset;
    nes_core_set_region_fn      set_region;
    nes_core_step_frame_fn      step_frame;
    nes_core_step_instruction_fn step_instruction;
    nes_core_framebuffer_fn    framebuffer;
    nes_core_take_audio_fn     take_audio;
    nes_core_save_state_fn      save_state;
    nes_core_load_state_fn      load_state;
    nes_core_mapper_number_fn   mapper_number;
    nes_core_impl_name_fn       impl_name;
    nes_core_impl_version_fn    impl_version;
} nes_core_api;

/* -------------------------------------------------------------------------
 * Statistics
 *
 * Computed from the per-iteration timing sample array. Times are stored in
 * nanoseconds (int64) for high-resolution cross-benchmark comparison; the
 * JSON/CSV output converts to microseconds where the spec calls for it.
 * ------------------------------------------------------------------------- */

typedef struct {
    uint64_t n;       /* number of timed samples                  */
    double   mean_ns; /* arithmetic mean                         */
    double   min_ns;  /* minimum                                 */
    double   max_ns;  /* maximum                                 */
    double   stddev_ns; /* population standard deviation          */
    double   p50_ns;  /* 50th percentile (median)                */
    double   p95_ns;  /* 95th percentile                         */
    double   p99_ns;  /* 99th percentile                         */
} bench_stats;

/* -------------------------------------------------------------------------
 * Per-benchmark result
 * ------------------------------------------------------------------------- */

typedef struct {
    const char* name;       /* "step_frame" | "cpu_step" | "save_state" | "render_frame" */
    uint64_t    iterations; /* timed iterations (excluding warm-up)                       */
    bench_stats stats;      /* computed from per-iteration ns samples                     */
} bench_result;

/* -------------------------------------------------------------------------
 * Per-core result (one entry in the JSON "cores" array)
 * ------------------------------------------------------------------------- */

typedef struct {
    char         name[64];        /* core display name (e.g. "rust")              */
    char         version[64];     /* core impl_version()                          */
    uint16_t     mapper;          /* mapper number of the loaded ROM              */
    bench_result results[4];     /* one per benchmark, in spec order             */
    uint32_t     n_results;       /* number of populated entries in `results`     */
} core_result;

/* -------------------------------------------------------------------------
 * Public entry points (implemented in harness.c)
 * ------------------------------------------------------------------------- */

/*
 * Load a core shared library and resolve all 13 protocol symbols. Returns
 * 0 on success and fills `api`; returns non-zero on failure (load error or
 * missing symbol). On Windows `path` is the DLL; on Unix it is the .so/.dylib.
 */
int core_load(const char* path, nes_core_api* api);

/* Free a previously loaded core library. Safe to call with a zeroed api. */
void core_unload(nes_core_api* api);

/* Compute statistics over `samples` (length `n`, in nanoseconds). */
void stats_compute(const int64_t* samples, uint64_t n, bench_stats* out);

/* Run a single benchmark `name` against `api` using the NOP ROM. Fills
 * `out`. Returns 0 on success. `frames`/`instructions` override defaults. */
int bench_run_one(const nes_core_api* api, const uint8_t* rom, size_t rom_len,
                  const char* name, uint32_t frames, uint32_t instructions,
                  bench_result* out);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* NES_BENCH_HARNESS_H */
