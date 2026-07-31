/*
 * bench/harness.c — cross-language NES core benchmark harness.
 *
 * Loads any core implementing cores/protocol.h as a shared library
 * (LoadLibrary / dlopen), drives it through the resolved `nes_core_api`
 * vtable with a fixed NOP ROM, and records per-benchmark timing. Also
 * implements the subprocess binary protocol (M3.md) for future
 * TypeScript / Python cores and exposes it via `--subprocess-test`.
 *
 * Benchmarks (see tasklist/M3.md):
 *   step_frame   — 1000 frames, us/frame
 *   cpu_step     — 100000 instructions, ns/instruction
 *   save_state   — 1000 save+load round-trips, us/round-trip
 *   render_frame — 1000 frames, us/frame (via step_frame; the C ABI
 *                  exposes no separate render entry — rendering happens
 *                  inside step_frame — so this measures the same path
 *                  plus a framebuffer pointer read)
 *
 * Warm-up: 10% of iterations run before timing (primes caches / branch
 * predictors / JITs). Timer: QueryPerformanceCounter (Windows),
 * clock_gettime(CLOCK_MONOTONIC) (Linux), mach_absolute_time (macOS).
 *
 * Output: JSON (default stdout or --output <file>) and optional CSV
 * (--csv). Statistics: mean, min, max, stddev, p50, p95, p99.
 *
 * See: https://www.nesdev.org/wiki/Cycle_reference (frame timing budget)
 */
#include "harness.h"
#include "gen_rom.h"

#include <ctype.h>
#include <errno.h>
#include <math.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>

/* -------------------------------------------------------------------------
 * Platform: dynamic loading + high-resolution timer
 * ------------------------------------------------------------------------- */

#if defined(_WIN32)
#  define WIN32_LEAN_AND_MEAN
#  include <windows.h>

typedef HMODULE lib_handle_t;

static lib_handle_t lib_open(const char* path) {
    return LoadLibraryA(path);
}
static void lib_close(lib_handle_t h) {
    if (h) { FreeLibrary(h); }
}
static void* lib_sym(lib_handle_t h, const char* name) {
    return (void*)GetProcAddress(h, name); /* cast through void* for fn ptrs */
}

/* QueryPerformanceCounter -> nanoseconds. */
static int64_t now_ns(void) {
    static LARGE_INTEGER freq = {0};
    LARGE_INTEGER counter;
    if (freq.QuadPart == 0) {
        QueryPerformanceFrequency(&freq);
    }
    QueryPerformanceCounter(&counter);
    /* counter * 1e9 / freq, computed in 128-bit-safe order via double for
     * large values; counter/freq is seconds. Use mul/div to keep precision. */
    return (int64_t)((double)counter.QuadPart * 1e9 / (double)freq.QuadPart);
}

static const char* platform_name(void) {
#  if defined(_M_X64) || defined(__x86_64__)
    return "windows-x86_64";
#  else
    return "windows-x86";
#  endif
}

#else /* POSIX */
#  include <dlfcn.h>

#  if defined(__APPLE__)
#    include <mach/mach_time.h>
#  endif

typedef void* lib_handle_t;

static lib_handle_t lib_open(const char* path) {
    return dlopen(path, RTLD_NOW | RTLD_LOCAL);
}
static void lib_close(lib_handle_t h) {
    if (h) { dlclose(h); }
}
static void* lib_sym(lib_handle_t h, const char* name) {
    return dlsym(h, name);
}

static int64_t now_ns(void) {
#  if defined(__APPLE__)
    static mach_timebase_info_data_t tb = {0};
    if (tb.denom == 0) { mach_timebase_info(&tb); }
    int64_t t = (int64_t)mach_absolute_time();
    return (int64_t)((double)t * (double)tb.numer / (double)tb.denom);
#  else
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (int64_t)ts.tv_sec * 1000000000LL + (int64_t)ts.tv_nsec;
#  endif
}

static const char* platform_name(void) {
#  if defined(__APPLE__)
    return "macos-x86_64";
#  elif defined(__x86_64__)
    return "linux-x86_64";
#  else
    return "posix";
#  endif
}
#endif

/* Single loaded library handle. The harness loads one core at a time
 * (load → benchmark → unload → next), so a process-global handle is safe. */
static lib_handle_t g_lib = 0;

/* -------------------------------------------------------------------------
 * Core loading
 * ------------------------------------------------------------------------- */

#define RESOLVE(field, name) \
    do { \
        api->field = (nes_core_##field##_fn)lib_sym(g_lib, name); \
        if (!api->field) { fprintf(stderr, "harness: missing symbol %s\n", name); return 1; } \
    } while (0)

int core_load(const char* path, nes_core_api* api) {
    memset(api, 0, sizeof(*api));
    g_lib = lib_open(path);
    if (!g_lib) {
#if defined(_WIN32)
        fprintf(stderr, "harness: LoadLibraryA(\"%s\") failed (err=%lu)\n",
                path, (unsigned long)GetLastError());
#else
        fprintf(stderr, "harness: dlopen(\"%s\") failed: %s\n", path, dlerror());
#endif
        return 1;
    }
    RESOLVE(create,           "nes_core_create");
    RESOLVE(destroy,          "nes_core_destroy");
    RESOLVE(reset,            "nes_core_reset");
    RESOLVE(set_region,       "nes_core_set_region");
    RESOLVE(step_frame,       "nes_core_step_frame");
    RESOLVE(step_instruction, "nes_core_step_instruction");
    RESOLVE(framebuffer,      "nes_core_framebuffer");
    RESOLVE(take_audio,       "nes_core_take_audio");
    RESOLVE(save_state,       "nes_core_save_state");
    RESOLVE(load_state,       "nes_core_load_state");
    RESOLVE(mapper_number,    "nes_core_mapper_number");
    RESOLVE(impl_name,        "nes_core_impl_name");
    RESOLVE(impl_version,     "nes_core_impl_version");
    return 0;
}

#undef RESOLVE

void core_unload(nes_core_api* api) {
    (void)api;
    if (g_lib) {
        lib_close(g_lib);
        g_lib = 0;
    }
}

/* -------------------------------------------------------------------------
 * Statistics
 *
 * Sorts the sample array in place (ascending) and computes mean/min/max/
 * stddev plus nearest-rank percentiles (p50/p95/p99). Population stddev
 * is used (the full timed sample set, not a sample-of-sample).
 * ------------------------------------------------------------------------- */

static int cmp_i64(const void* a, const void* b) {
    int64_t x = *(const int64_t*)a;
    int64_t y = *(const int64_t*)b;
    if (x < y) return -1;
    if (x > y) return 1;
    return 0;
}

/* Nearest-rank percentile: index = ceil(p/100 * n) - 1, clamped to [0,n-1]. */
static double percentile_sorted(const int64_t* sorted, uint64_t n, double p) {
    if (n == 0) return 0.0;
    double rank = (p / 100.0) * (double)n;
    uint64_t idx = (uint64_t)ceil(rank);
    if (idx == 0) idx = 1;
    if (idx > n) idx = n;
    return (double)sorted[idx - 1];
}

void stats_compute(const int64_t* samples, uint64_t n, bench_stats* out) {
    memset(out, 0, sizeof(*out));
    out->n = n;
    if (n == 0) return;

    /* Copy + sort for percentile computation (don't mutate caller's array). */
    int64_t* sorted = (int64_t*)malloc(n * sizeof(int64_t));
    if (!sorted) {
        /* OOM: fall back to mean-only from the unsorted input. */
        double sum = 0.0;
        for (uint64_t i = 0; i < n; ++i) sum += (double)samples[i];
        out->mean_ns = sum / (double)n;
        out->min_ns = out->mean_ns;
        out->max_ns = out->mean_ns;
        return;
    }
    memcpy(sorted, samples, n * sizeof(int64_t));
    qsort(sorted, n, sizeof(int64_t), cmp_i64);

    double sum = 0.0;
    for (uint64_t i = 0; i < n; ++i) sum += (double)samples[i];
    double mean = sum / (double)n;

    double sq = 0.0;
    for (uint64_t i = 0; i < n; ++i) {
        double d = (double)samples[i] - mean;
        sq += d * d;
    }
    double stddev = sqrt(sq / (double)n);

    out->mean_ns   = mean;
    out->min_ns    = (double)sorted[0];
    out->max_ns    = (double)sorted[n - 1];
    out->stddev_ns = stddev;
    out->p50_ns    = percentile_sorted(sorted, n, 50.0);
    out->p95_ns    = percentile_sorted(sorted, n, 95.0);
    out->p99_ns    = percentile_sorted(sorted, n, 99.0);

    free(sorted);
}

/* -------------------------------------------------------------------------
 * Benchmark driver
 *
 * Each benchmark: create, reset, warm-up (10% of iterations), then time
 * the configured number of iterations. Per-iteration samples are stored
 * in a heap array and reduced via stats_compute.
 * ------------------------------------------------------------------------- */

/* Warm-up fraction (M3.md: 10% of iterations before timing). */
#define WARMUP_FRACTION 10

/* Audio scratch buffer drained each frame so the core's queue never
 * overflows during a long benchmark run. */
#define AUDIO_SCRATCH 4096

/* Save-state scratch: generous upper bound for the Rust core's bincode
 * blob (~hundreds of KB). Grown on demand if save_state reports 0 with a
 * non-zero cap (buffer too small). */
#define SAVE_STATE_INITIAL (1u << 20) /* 1 MiB */

static uint64_t warmup_count(uint64_t iters) {
    return iters / WARMUP_FRACTION;
}

/* Run `iters` step_frame iterations against `core`, recording per-iter ns
 * into `samples`. Returns 0 on success. */
static int bench_step_frame_loop(const nes_core_api* api, nes_core_t* core,
                                 uint64_t iters, int64_t* samples,
                                 int16_t* audio_scratch) {
    for (uint64_t i = 0; i < iters; ++i) {
        int64_t t0 = now_ns();
        uint32_t cycles = api->step_frame(core);
        int64_t t1 = now_ns();
        /* Drain audio so the queue does not grow unbounded. */
        api->take_audio(core, audio_scratch, AUDIO_SCRATCH);
        (void)cycles;
        samples[i] = t1 - t0;
    }
    return 0;
}

/* step_frame benchmark: create, reset, warm-up, time `frames` iters. */
static int bench_step_frame(const nes_core_api* api,
                            const uint8_t* rom, size_t rom_len,
                            uint32_t frames, bench_result* out) {
    int16_t audio_scratch[AUDIO_SCRATCH];
    nes_core_t* core = api->create(rom, rom_len);
    if (!core) { fprintf(stderr, "harness: step_frame: create failed\n"); return 1; }
    api->reset(core);

    uint64_t warm = warmup_count(frames);
    for (uint64_t i = 0; i < warm; ++i) {
        api->step_frame(core);
        api->take_audio(core, audio_scratch, AUDIO_SCRATCH);
    }

    int64_t* samples = (int64_t*)malloc((size_t)frames * sizeof(int64_t));
    if (!samples) { api->destroy(core); return 1; }

    bench_step_frame_loop(api, core, frames, samples, audio_scratch);
    stats_compute(samples, frames, &out->stats);

    free(samples);
    api->destroy(core);
    out->name = "step_frame";
    out->iterations = frames;
    return 0;
}

/* cpu_step benchmark: create, reset, time `instructions` single-instruction
 * steps. ns per instruction. */
static int bench_cpu_step(const nes_core_api* api,
                          const uint8_t* rom, size_t rom_len,
                          uint32_t instructions, bench_result* out) {
    int16_t audio_scratch[AUDIO_SCRATCH];
    nes_core_t* core = api->create(rom, rom_len);
    if (!core) { fprintf(stderr, "harness: cpu_step: create failed\n"); return 1; }
    api->reset(core);

    uint64_t warm = warmup_count(instructions);
    for (uint64_t i = 0; i < warm; ++i) {
        api->step_instruction(core);
        /* Step_instruction does not advance a full frame, so audio rarely
         * accumulates, but drain defensively every 1024 instructions. */
        if ((i & 1023) == 0) api->take_audio(core, audio_scratch, AUDIO_SCRATCH);
    }

    int64_t* samples = (int64_t*)malloc((size_t)instructions * sizeof(int64_t));
    if (!samples) { api->destroy(core); return 1; }

    for (uint64_t i = 0; i < instructions; ++i) {
        int64_t t0 = now_ns();
        uint32_t cycles = api->step_instruction(core);
        int64_t t1 = now_ns();
        (void)cycles;
        samples[i] = t1 - t0;
        if ((i & 1023) == 0) api->take_audio(core, audio_scratch, AUDIO_SCRATCH);
    }
    stats_compute(samples, instructions, &out->stats);

    free(samples);
    api->destroy(core);
    out->name = "cpu_step";
    out->iterations = instructions;
    return 0;
}

/* save_state benchmark: create, reset, 1 warm-up frame, then time `iters`
 * save+load round-trips. us per round-trip. */
static int bench_save_state(const nes_core_api* api,
                            const uint8_t* rom, size_t rom_len,
                            uint32_t iters, bench_result* out) {
    int16_t audio_scratch[AUDIO_SCRATCH];
    nes_core_t* core = api->create(rom, rom_len);
    if (!core) { fprintf(stderr, "harness: save_state: create failed\n"); return 1; }
    api->reset(core);
    /* 1 warm-up frame (M3.md setup). */
    api->step_frame(core);
    api->take_audio(core, audio_scratch, AUDIO_SCRATCH);

    /* Probe the required save buffer size once. */
    size_t cap = SAVE_STATE_INITIAL;
    uint8_t* save_buf = (uint8_t*)malloc(cap);
    if (!save_buf) { api->destroy(core); return 1; }

    /* If the initial probe returns 0 (too small), grow until it fits. */
    size_t needed = api->save_state(core, save_buf, cap);
    while (needed == 0) {
        cap *= 2;
        uint8_t* grown = (uint8_t*)realloc(save_buf, cap);
        if (!grown) { free(save_buf); api->destroy(core); return 1; }
        save_buf = grown;
        needed = api->save_state(core, save_buf, cap);
        if (cap > (1u << 28)) { /* 256 MiB sanity cap. */
            fprintf(stderr, "harness: save_state blob exceeds 256 MiB\n");
            free(save_buf); api->destroy(core); return 1;
        }
    }
    size_t blob_len = needed;

    uint64_t warm = warmup_count(iters);
    for (uint64_t i = 0; i < warm; ++i) {
        api->save_state(core, save_buf, cap);
        api->load_state(core, save_buf, blob_len);
    }

    int64_t* samples = (int64_t*)malloc((size_t)iters * sizeof(int64_t));
    if (!samples) { free(save_buf); api->destroy(core); return 1; }

    for (uint64_t i = 0; i < iters; ++i) {
        int64_t t0 = now_ns();
        size_t n = api->save_state(core, save_buf, cap);
        int ok = api->load_state(core, save_buf, n);
        int64_t t1 = now_ns();
        if (n == 0 || !ok) {
            fprintf(stderr, "harness: save_state round-trip %llu failed (n=%zu ok=%d)\n",
                    (unsigned long long)i, n, ok);
            free(samples); free(save_buf); api->destroy(core); return 1;
        }
        samples[i] = t1 - t0;
    }
    stats_compute(samples, iters, &out->stats);

    free(samples);
    free(save_buf);
    api->destroy(core);
    out->name = "save_state";
    out->iterations = iters;
    return 0;
}

/* render_frame benchmark: the C ABI exposes no separate render entry —
 * rendering happens inside step_frame. We measure step_frame plus a
 * framebuffer pointer read (the pointer is cached/stable, so the read is
 * near-free; this isolates the render cost from any audio-drain overhead
 * by NOT draining audio here, mirroring the Criterion render_frame bench
 * which only touches the framebuffer). */
static int bench_render_frame(const nes_core_api* api,
                              const uint8_t* rom, size_t rom_len,
                              uint32_t frames, bench_result* out) {
    nes_core_t* core = api->create(rom, rom_len);
    if (!core) { fprintf(stderr, "harness: render_frame: create failed\n"); return 1; }
    api->reset(core);

    uint64_t warm = warmup_count(frames);
    for (uint64_t i = 0; i < warm; ++i) {
        api->step_frame(core);
        const uint32_t* fb = api->framebuffer(core);
        (void)fb;
    }

    int64_t* samples = (int64_t*)malloc((size_t)frames * sizeof(int64_t));
    if (!samples) { api->destroy(core); return 1; }

    for (uint64_t i = 0; i < frames; ++i) {
        int64_t t0 = now_ns();
        api->step_frame(core);
        const uint32_t* fb = api->framebuffer(core);
        int64_t t1 = now_ns();
        (void)fb;
        samples[i] = t1 - t0;
    }
    stats_compute(samples, frames, &out->stats);

    free(samples);
    api->destroy(core);
    out->name = "render_frame";
    out->iterations = frames;
    return 0;
}

int bench_run_one(const nes_core_api* api, const uint8_t* rom, size_t rom_len,
                  const char* name, uint32_t frames, uint32_t instructions,
                  bench_result* out) {
    if (strcmp(name, "step_frame") == 0) {
        return bench_step_frame(api, rom, rom_len, frames, out);
    } else if (strcmp(name, "cpu_step") == 0) {
        return bench_cpu_step(api, rom, rom_len, instructions, out);
    } else if (strcmp(name, "save_state") == 0) {
        return bench_save_state(api, rom, rom_len, frames, out);
    } else if (strcmp(name, "render_frame") == 0) {
        return bench_render_frame(api, rom, rom_len, frames, out);
    }
    fprintf(stderr, "harness: unknown benchmark '%s'\n", name);
    return 1;
}

/* -------------------------------------------------------------------------
 * JSON / CSV output
 * ------------------------------------------------------------------------- */

/* Escape a string for a JSON string literal. Writes into `out` (capacity
 * `cap`). Handles quotes, backslashes, and control chars. */
static void json_escape(const char* s, char* out, size_t cap) {
    size_t j = 0;
    for (size_t i = 0; s && s[i] && j + 2 < cap; ++i) {
        unsigned char c = (unsigned char)s[i];
        if (c == '"' || c == '\\') {
            if (j + 2 >= cap) break;
            out[j++] = '\\';
            out[j++] = (char)c;
        } else if (c < 0x20) {
            const char* esc = NULL;
            switch (c) {
                case '\b': esc = "\\b"; break;
                case '\f': esc = "\\f"; break;
                case '\n': esc = "\\n"; break;
                case '\r': esc = "\\r"; break;
                case '\t': esc = "\\t"; break;
                default: break;
            }
            if (esc) {
                size_t len = strlen(esc);
                if (j + len + 1 >= cap) break;
                memcpy(out + j, esc, len);
                j += len;
            } else {
                if (j + 6 >= cap) break;
                j += (size_t)snprintf(out + j, cap - j, "\\u%04x", c);
            }
        } else {
            out[j++] = (char)c;
        }
    }
    out[j] = '\0';
}

static void print_stats_json(FILE* f, const char* name, const bench_result* r) {
    /* Times: step_frame/render_frame/save_state reported in microseconds,
     * cpu_step in nanoseconds (per M3.md metrics). We emit both ns and us
     * fields so consumers can pick; the *_us fields are the headline. */
    double us_mean = r->stats.mean_ns / 1000.0;
    double us_min  = r->stats.min_ns  / 1000.0;
    double us_max  = r->stats.max_ns  / 1000.0;
    double us_std  = r->stats.stddev_ns / 1000.0;
    double us_p50  = r->stats.p50_ns  / 1000.0;
    double us_p95  = r->stats.p95_ns  / 1000.0;
    double us_p99  = r->stats.p99_ns  / 1000.0;
    fprintf(f,
        "      \"%s\": {\n"
        "        \"iterations\": %llu,\n"
        "        \"mean_us\": %.4f,\n"
        "        \"min_us\": %.4f,\n"
        "        \"max_us\": %.4f,\n"
        "        \"stddev_us\": %.4f,\n"
        "        \"p50_us\": %.4f,\n"
        "        \"p95_us\": %.4f,\n"
        "        \"p99_us\": %.4f,\n"
        "        \"mean_ns\": %.1f,\n"
        "        \"min_ns\": %.1f,\n"
        "        \"max_ns\": %.1f,\n"
        "        \"stddev_ns\": %.1f,\n"
        "        \"p50_ns\": %.1f,\n"
        "        \"p95_ns\": %.1f,\n"
        "        \"p99_ns\": %.1f\n"
        "      }",
        name,
        (unsigned long long)r->iterations,
        us_mean, us_min, us_max, us_std, us_p50, us_p95, us_p99,
        r->stats.mean_ns, r->stats.min_ns, r->stats.max_ns,
        r->stats.stddev_ns, r->stats.p50_ns, r->stats.p95_ns, r->stats.p99_ns);
}

static void print_json(FILE* f, const core_result* cores, uint32_t n_cores) {
    char ts[64];
    time_t now = time(NULL);
    struct tm* tm = gmtime(&now);
    strftime(ts, sizeof(ts), "%Y-%m-%dT%H:%M:%SZ", tm);

    fprintf(f, "{\n");
    fprintf(f, "  \"timestamp\": \"%s\",\n", ts);
    fprintf(f, "  \"platform\": \"%s\",\n", platform_name());
    fprintf(f, "  \"cores\": [\n");
    for (uint32_t c = 0; c < n_cores; ++c) {
        const core_result* cr = &cores[c];
        char name_esc[128], ver_esc[128];
        json_escape(cr->name, name_esc, sizeof(name_esc));
        json_escape(cr->version, ver_esc, sizeof(ver_esc));
        fprintf(f, "    {\n");
        fprintf(f, "      \"name\": \"%s\",\n", name_esc);
        fprintf(f, "      \"version\": \"%s\",\n", ver_esc);
        fprintf(f, "      \"mapper\": %u,\n", (unsigned)cr->mapper);
        fprintf(f, "      \"benchmarks\": {\n");
        for (uint32_t b = 0; b < cr->n_results; ++b) {
            print_stats_json(f, cr->results[b].name, &cr->results[b]);
            if (b + 1 < cr->n_results) fprintf(f, ",");
            fprintf(f, "\n");
        }
        fprintf(f, "      }\n");
        fprintf(f, "    }");
        if (c + 1 < n_cores) fprintf(f, ",");
        fprintf(f, "\n");
    }
    fprintf(f, "  ]\n");
    fprintf(f, "}\n");
}

static void print_csv(FILE* f, const core_result* cores, uint32_t n_cores) {
    fprintf(f, "core,version,mapper,benchmark,iterations,mean_us,min_us,max_us,stddev_us,p50_us,p95_us,p99_us\n");
    for (uint32_t c = 0; c < n_cores; ++c) {
        const core_result* cr = &cores[c];
        for (uint32_t b = 0; b < cr->n_results; ++b) {
            const bench_result* r = &cr->results[b];
            fprintf(f, "%s,%s,%u,%s,%llu,%.4f,%.4f,%.4f,%.4f,%.4f,%.4f,%.4f\n",
                cr->name, cr->version, (unsigned)cr->mapper, r->name,
                (unsigned long long)r->iterations,
                r->stats.mean_ns / 1000.0, r->stats.min_ns / 1000.0,
                r->stats.max_ns / 1000.0, r->stats.stddev_ns / 1000.0,
                r->stats.p50_ns / 1000.0, r->stats.p95_ns / 1000.0,
                r->stats.p99_ns / 1000.0);
        }
    }
}

/* -------------------------------------------------------------------------
 * Subprocess binary protocol (M3.md)
 *
 * Frames over stdin/stdout between the harness and a TypeScript/Python core
 * subprocess. Used by `--subprocess-test` to verify framing with an echo
 * core (bench/echo_core.py). Full vtable wrapping for subprocess cores
 * lands with M10/M11 once those cores exist.
 *
 * Messages harness -> core:
 *   INIT         [0x01][rom_len:u32 LE][rom_bytes]
 *   RESET        [0x02]
 *   STEP_FRAME   [0x03][count:u32 LE]
 *   STEP_INSTR   [0x04][count:u32 LE]
 *   SAVE_STATE   [0x05]
 *   LOAD_STATE   [0x06][len:u32 LE][bytes]
 * Messages core -> harness:
 *   RESULT       [cycles:u32 LE][fb_len:u32 LE][fb pixels][audio_len:u32 LE][audio]
 *   SAVE_RESULT  [len:u32 LE][bytes]
 *   OK           [0x00]
 *   ERROR        [0xFF][msg_len:u16 LE][msg]
 * ------------------------------------------------------------------------- */

#if defined(_WIN32)
#  include <windows.h>
typedef struct {
    HANDLE proc;
    HANDLE hchild;       /* child's stdin write end (we write)  */
    HANDLE hchild_out;   /* child's stdout read end (we read)   */
} subproc_t;
#else
#  include <unistd.h>
typedef struct {
    pid_t pid;
    int   in_fd;   /* write to child stdin */
    int   out_fd;  /* read from child stdout */
} subproc_t;
#endif

/* Write exactly `len` bytes; loop over partial writes. Returns 0 on success. */
static int write_all(subproc_t* sp, const void* buf, size_t len) {
    const uint8_t* p = (const uint8_t*)buf;
    size_t off = 0;
    while (off < len) {
#if defined(_WIN32)
        DWORD wr = 0;
        if (!WriteFile(sp->hchild, p + off, (DWORD)(len - off), &wr, NULL)) return 1;
        if (wr == 0) return 1;
        off += wr;
#else
        ssize_t wr = write(sp->in_fd, p + off, len - off);
        if (wr <= 0) { if (errno == EINTR) continue; return 1; }
        off += (size_t)wr;
#endif
    }
    return 0;
}

/* Read exactly `len` bytes; loop over partial reads. Returns 0 on success. */
static int read_all(subproc_t* sp, void* buf, size_t len) {
    uint8_t* p = (uint8_t*)buf;
    size_t off = 0;
    while (off < len) {
#if defined(_WIN32)
        DWORD got = 0;
        if (!ReadFile(sp->hchild_out, p + off, (DWORD)(len - off), &got, NULL)) return 1;
        if (got == 0) return 1;
        off += got;
#else
        ssize_t got = read(sp->out_fd, p + off, len - off);
        if (got <= 0) { if (errno == EINTR) continue; return 1; }
        off += (size_t)got;
#endif
    }
    return 0;
}

static void put_u32_le(uint8_t* b, uint32_t v) {
    b[0] = (uint8_t)(v & 0xFF);
    b[1] = (uint8_t)((v >> 8) & 0xFF);
    b[2] = (uint8_t)((v >> 16) & 0xFF);
    b[3] = (uint8_t)((v >> 24) & 0xFF);
}
static uint32_t get_u32_le(const uint8_t* b) {
    return (uint32_t)b[0] | ((uint32_t)b[1] << 8) |
           ((uint32_t)b[2] << 16) | ((uint32_t)b[3] << 24);
}
static uint16_t get_u16_le(const uint8_t* b) {
    return (uint16_t)((uint16_t)b[0] | ((uint16_t)b[1] << 8));
}

static int sub_send_init(subproc_t* sp, const uint8_t* rom, size_t rom_len) {
    uint8_t hdr[5];
    hdr[0] = 0x01;
    put_u32_le(hdr + 1, (uint32_t)rom_len);
    if (write_all(sp, hdr, 5)) return 1;
    return write_all(sp, rom, rom_len);
}
static int sub_send_reset(subproc_t* sp) {
    uint8_t b = 0x02;
    return write_all(sp, &b, 1);
}
static int sub_send_step_frame(subproc_t* sp, uint32_t count) {
    uint8_t hdr[5];
    hdr[0] = 0x03;
    put_u32_le(hdr + 1, count);
    return write_all(sp, hdr, 5);
}
static int sub_send_step_instr(subproc_t* sp, uint32_t count) {
    uint8_t hdr[5];
    hdr[0] = 0x04;
    put_u32_le(hdr + 1, count);
    return write_all(sp, hdr, 5);
}
static int sub_send_save_state(subproc_t* sp) {
    uint8_t b = 0x05;
    return write_all(sp, &b, 1);
}
static int sub_send_load_state(subproc_t* sp, const uint8_t* data, size_t len) {
    uint8_t hdr[5];
    hdr[0] = 0x06;
    put_u32_le(hdr + 1, (uint32_t)len);
    if (write_all(sp, hdr, 5)) return 1;
    return write_all(sp, data, len);
}

/* Core -> harness receivers. Per M3.md the core's responses carry NO tag
 * byte — they are distinguished by context (which command the harness just
 * sent): after INIT/RESET/LOAD_STATE expect OK (0x00) or ERROR (0xFF...);
 * after STEP_FRAME/STEP_INSTR expect RESULT; after SAVE_STATE expect
 * SAVE_RESULT. Each receiver reads exactly its framed payload. */

/* Read an OK / ERROR response. Returns 0 on OK, 0xFF on ERROR (message
 * copied into `scratch` and NUL-terminated), -1 on protocol error. */
static int sub_recv_ok(subproc_t* sp, uint8_t* scratch, size_t scratch_cap) {
    uint8_t tag;
    if (read_all(sp, &tag, 1)) return -1;
    if (tag == 0x00) return 0;
    if (tag == 0xFF) {
        uint8_t lenb[2];
        if (read_all(sp, lenb, 2)) return -1;
        uint16_t mlen = get_u16_le(lenb);
        if (mlen > scratch_cap) return -1;
        if (read_all(sp, scratch, mlen)) return -1;
        scratch[mlen] = '\0';
        return 0xFF;
    }
    return -1; /* unexpected byte */
}

/* Read a RESULT response: [cycles:u32][fb_len:u32][fb][audio_len:u32][audio].
 * Framebuffer (fb_len uint32 pixels) and audio (audio_len int16 samples)
 * payloads are read into `scratch` (raw bytes) and discarded. Returns 0 on
 * success, -1 on protocol error. */
static int sub_recv_result(subproc_t* sp, uint8_t* scratch, size_t scratch_cap) {
    uint8_t hdr[12];
    if (read_all(sp, hdr, 12)) return -1;
    uint32_t fb_len = get_u32_le(hdr + 4);
    uint32_t au_len = get_u32_le(hdr + 8);
    size_t fb_bytes = (size_t)fb_len * 4;
    if (fb_bytes > scratch_cap) return -1;
    if (fb_bytes && read_all(sp, scratch, fb_bytes)) return -1;
    size_t au_bytes = (size_t)au_len * 2;
    if (au_bytes > scratch_cap) return -1;
    if (au_bytes && read_all(sp, scratch, au_bytes)) return -1;
    return 0;
}

/* Read a SAVE_RESULT response: [len:u32][bytes]. The blob is read into
 * `scratch`; `*out_len` receives the blob length. Returns 0 on success. */
static int sub_recv_save_result(subproc_t* sp, uint8_t* scratch, size_t scratch_cap,
                                size_t* out_len) {
    uint8_t lenb[4];
    if (read_all(sp, lenb, 4)) return -1;
    uint32_t blen = get_u32_le(lenb);
    if (blen > scratch_cap) return -1;
    if (blen && read_all(sp, scratch, blen)) return -1;
    if (out_len) *out_len = blen;
    return 0;
}

/* Spawn a subprocess with stdin/stdout piped to us. `cmdline` is the full
 * command line (parsed by the shell on POSIX, CreateProcess on Windows).
 * Returns 0 on success. */
static int sub_spawn(subproc_t* sp, const char* cmdline) {
    memset(sp, 0, sizeof(*sp));
#if defined(_WIN32)
    SECURITY_ATTRIBUTES sa;
    sa.nLength = sizeof(sa);
    sa.bInheritHandle = TRUE;
    sa.lpSecurityDescriptor = NULL;
    HANDLE child_in_rd = NULL, child_in_wr = NULL;
    HANDLE child_out_rd = NULL, child_out_wr = NULL;
    if (!CreatePipe(&child_in_rd, &child_in_wr, &sa, 0)) return 1;
    if (!CreatePipe(&child_out_rd, &child_out_wr, &sa, 0)) {
        CloseHandle(child_in_rd); CloseHandle(child_in_wr); return 1;
    }
    /* Ensure our ends are not inherited. */
    SetHandleInformation(child_in_wr, HANDLE_FLAG_INHERIT, 0);
    SetHandleInformation(child_out_rd, HANDLE_FLAG_INHERIT, 0);

    STARTUPINFOA si;
    PROCESS_INFORMATION pi;
    memset(&si, 0, sizeof(si));
    si.cb = sizeof(si);
    si.dwFlags = STARTF_USESTDHANDLES;
    si.hStdInput  = child_in_rd;
    si.hStdOutput = child_out_wr;
    si.hStdError  = GetStdHandle(STD_ERROR_HANDLE);
    memset(&pi, 0, sizeof(pi));

    /* CreateProcess may modify the cmdline buffer; make a writable copy. */
    char* cmd = _strdup(cmdline);
    if (!cmd) {
        CloseHandle(child_in_rd); CloseHandle(child_in_wr);
        CloseHandle(child_out_rd); CloseHandle(child_out_wr);
        return 1;
    }
    BOOL ok = CreateProcessA(NULL, cmd, NULL, NULL, TRUE,
                             CREATE_NO_WINDOW, NULL, NULL, &si, &pi);
    free(cmd);
    /* Close child-side handles in our process so EOF propagates. */
    CloseHandle(child_in_rd);
    CloseHandle(child_out_wr);
    if (!ok) {
        CloseHandle(child_in_wr); CloseHandle(child_out_rd);
        return 1;
    }
    sp->proc = pi.hProcess;
    sp->hchild = child_in_wr;       /* we write to child's stdin */
    sp->hchild_out = child_out_rd;  /* we read child's stdout    */
    CloseHandle(pi.hThread);
    return 0;
#else
    int in_pipe[2], out_pipe[2];
    if (pipe(in_pipe) || pipe(out_pipe)) return 1;
    pid_t pid = fork();
    if (pid < 0) return 1;
    if (pid == 0) {
        /* child */
        dup2(in_pipe[0], 0);
        dup2(out_pipe[1], 1);
        close(in_pipe[0]); close(in_pipe[1]);
        close(out_pipe[0]); close(out_pipe[1]);
        execl("/bin/sh", "sh", "-c", cmdline, (char*)NULL);
        _exit(127);
    }
    close(in_pipe[0]); close(out_pipe[1]);
    sp->pid = pid;
    sp->in_fd = in_pipe[1];
    sp->out_fd = out_pipe[0];
    return 0;
#endif
}

static void sub_close(subproc_t* sp) {
#if defined(_WIN32)
    if (sp->hchild) { CloseHandle(sp->hchild); sp->hchild = NULL; }
    if (sp->hchild_out) { CloseHandle(sp->hchild_out); sp->hchild_out = NULL; }
    if (sp->proc) {
        WaitForSingleObject(sp->proc, 2000);
        CloseHandle(sp->proc); sp->proc = NULL;
    }
#else
    if (sp->in_fd >= 0) { close(sp->in_fd); sp->in_fd = -1; }
    if (sp->out_fd >= 0) { close(sp->out_fd); sp->out_fd = -1; }
    if (sp->pid > 0) { waitpid(sp->pid, NULL, 0); sp->pid = -1; }
#endif
}

/* Run a full protocol round-trip against `cmdline` and report pass/fail.
 * This is the M3 acceptance "subprocess protocol works with a simple echo
 * test": INIT -> OK, RESET -> OK, STEP_FRAME(1) -> RESULT, STEP_INSTR(1)
 * -> RESULT, SAVE_STATE -> SAVE_RESULT, LOAD_STATE(saved) -> OK. */
static int subprocess_echo_test(const char* cmdline, const uint8_t* rom, size_t rom_len) {
    subproc_t sp;
    if (sub_spawn(&sp, cmdline)) {
        fprintf(stderr, "harness: subprocess-test: spawn failed for '%s'\n", cmdline);
        return 1;
    }
    /* Scratch for RESULT/SAVE_RESULT payloads and ERROR messages. */
    static uint8_t scratch[1 << 20]; /* 1 MiB; fb=256*240*4=245760 fits */
    uint8_t save_blob[1 << 16];
    size_t save_len = 0;
    int rc = 0;

    printf("[subprocess-test] spawning: %s\n", cmdline);

    if (sub_send_init(&sp, rom, rom_len)) { printf("  INIT send FAILED\n"); rc = 1; goto done; }
    if (sub_recv_ok(&sp, scratch, sizeof(scratch)) != 0) { printf("  INIT recv FAILED\n"); rc = 1; goto done; }
    printf("  INIT -> OK\n");

    if (sub_send_reset(&sp)) { printf("  RESET send FAILED\n"); rc = 1; goto done; }
    if (sub_recv_ok(&sp, scratch, sizeof(scratch)) != 0) { printf("  RESET recv FAILED\n"); rc = 1; goto done; }
    printf("  RESET -> OK\n");

    if (sub_send_step_frame(&sp, 1)) { printf("  STEP_FRAME send FAILED\n"); rc = 1; goto done; }
    if (sub_recv_result(&sp, scratch, sizeof(scratch)) != 0) { printf("  STEP_FRAME recv FAILED\n"); rc = 1; goto done; }
    printf("  STEP_FRAME(1) -> RESULT\n");

    if (sub_send_step_instr(&sp, 1)) { printf("  STEP_INSTR send FAILED\n"); rc = 1; goto done; }
    if (sub_recv_result(&sp, scratch, sizeof(scratch)) != 0) { printf("  STEP_INSTR recv FAILED\n"); rc = 1; goto done; }
    printf("  STEP_INSTR(1) -> RESULT\n");

    if (sub_send_save_state(&sp)) { printf("  SAVE_STATE send FAILED\n"); rc = 1; goto done; }
    {
        save_len = 0;
        if (sub_recv_save_result(&sp, scratch, sizeof(scratch), &save_len) != 0) {
            printf("  SAVE_STATE recv FAILED\n"); rc = 1; goto done;
        }
        if (save_len > sizeof(save_blob)) {
            printf("  SAVE_STATE blob too large (%zu) for echo buffer\n", save_len);
            rc = 1; goto done;
        }
        memcpy(save_blob, scratch, save_len);
    }
    printf("  SAVE_STATE -> SAVE_RESULT (len=%zu)\n", save_len);

    if (sub_send_load_state(&sp, save_blob, save_len)) { printf("  LOAD_STATE send FAILED\n"); rc = 1; goto done; }
    if (sub_recv_ok(&sp, scratch, sizeof(scratch)) != 0) { printf("  LOAD_STATE recv FAILED\n"); rc = 1; goto done; }
    printf("  LOAD_STATE -> OK\n");

    printf("[subprocess-test] PASS — protocol framing verified\n");
done:
    sub_close(&sp);
    return rc;
}

/* -------------------------------------------------------------------------
 * CLI
 * ------------------------------------------------------------------------- */

typedef struct {
    const char* core_filter;        /* --core <name>   (NULL = all)        */
    const char* bench_filter;       /* --bench <name>  (NULL = all)        */
    uint32_t    frames;             /* --frames (default 1000)             */
    uint32_t    instructions;       /* --instructions (default 100000)     */
    const char* output;             /* --output <file> (NULL = stdout)     */
    int         csv;                /* --csv                               */
    int         verbose;            /* --verbose                           */
    const char* subprocess_cmd;     /* --subprocess-test <cmd>             */
    uint32_t    subprocess_batch;   /* --subprocess-batch-size (default 100) */
} cli_opts;

static void cli_defaults(cli_opts* o) {
    o->core_filter = NULL;
    o->bench_filter = NULL;
    o->frames = 1000;
    o->instructions = 100000;
    o->output = NULL;
    o->csv = 0;
    o->verbose = 0;
    o->subprocess_cmd = NULL;
    o->subprocess_batch = 100;
}

static int starts_with(const char* s, const char* pfx) {
    return strncmp(s, pfx, strlen(pfx)) == 0;
}

/* Parse `--key value` and `--key=value` forms. Returns 0 on success. */
static int cli_parse(cli_opts* o, int argc, char** argv) {
    cli_defaults(o);
    for (int i = 1; i < argc; ++i) {
        const char* a = argv[i];
        const char* val = NULL;
        char eqbuf[256];
        if (strchr(a, '=') && starts_with(a, "--")) {
            const char* eq = strchr(a, '=');
            size_t klen = (size_t)(eq - a);
            if (klen >= sizeof(eqbuf)) klen = sizeof(eqbuf) - 1;
            memcpy(eqbuf, a, klen);
            eqbuf[klen] = '\0';
            val = eq + 1;
            a = eqbuf;
        }
        if (strcmp(a, "--core") == 0) {
            if (!val) { if (++i >= argc) return 1; val = argv[i]; }
            o->core_filter = val;
        } else if (strcmp(a, "--bench") == 0) {
            if (!val) { if (++i >= argc) return 1; val = argv[i]; }
            o->bench_filter = val;
        } else if (strcmp(a, "--frames") == 0) {
            if (!val) { if (++i >= argc) return 1; val = argv[i]; }
            o->frames = (uint32_t)strtoul(val, NULL, 10);
        } else if (strcmp(a, "--instructions") == 0) {
            if (!val) { if (++i >= argc) return 1; val = argv[i]; }
            o->instructions = (uint32_t)strtoul(val, NULL, 10);
        } else if (strcmp(a, "--output") == 0) {
            if (!val) { if (++i >= argc) return 1; val = argv[i]; }
            o->output = val;
        } else if (strcmp(a, "--csv") == 0) {
            o->csv = 1;
        } else if (strcmp(a, "--verbose") == 0) {
            o->verbose = 1;
        } else if (strcmp(a, "--subprocess-test") == 0) {
            if (!val) { if (++i >= argc) return 1; val = argv[i]; }
            o->subprocess_cmd = val;
        } else if (strcmp(a, "--subprocess-batch-size") == 0) {
            if (!val) { if (++i >= argc) return 1; val = argv[i]; }
            o->subprocess_batch = (uint32_t)strtoul(val, NULL, 10);
        } else if (strcmp(a, "--help") == 0 || strcmp(a, "-h") == 0) {
            printf(
                "Usage: harness [options] [core.dll ...]\n"
                "  --core <name>       Run only the named core\n"
                "  --bench <name>      Run only the named benchmark\n"
                "  --frames <N>        Frames for step_frame (default 1000)\n"
                "  --instructions <N>  Instructions for cpu_step (default 100000)\n"
                "  --output <file>     JSON results (default stdout)\n"
                "  --csv               Also output CSV summary\n"
                "  --verbose           Per-iteration timing\n"
                "  --subprocess-test <cmd>   Run subprocess protocol echo test\n"
                "  --subprocess-batch-size N (default 100)\n");
            exit(0);
        } else if (starts_with(a, "--")) {
            fprintf(stderr, "harness: unknown option '%s'\n", a);
            return 1;
        }
        /* Non-option args are treated as core library paths (positional). */
    }
    return 0;
}

/* Collect positional core library paths from argv (everything not starting
 * with -- and not consumed as a value). Returns a malloc'd NULL-terminated
 * array; caller frees the array (not the strings, which point into argv). */
static const char** collect_core_paths(int argc, char** argv, int* out_n) {
    const char** paths = (const char**)calloc((size_t)argc, sizeof(char*));
    if (!paths) return NULL;
    int n = 0;
    for (int i = 1; i < argc; ++i) {
        const char* a = argv[i];
        if (starts_with(a, "--")) {
            /* Skip its value if it takes one and wasn't --key=value. */
            if (!strchr(a, '=') &&
                (strcmp(a, "--core") == 0 || strcmp(a, "--bench") == 0 ||
                 strcmp(a, "--frames") == 0 || strcmp(a, "--instructions") == 0 ||
                 strcmp(a, "--output") == 0 || strcmp(a, "--subprocess-test") == 0 ||
                 strcmp(a, "--subprocess-batch-size") == 0)) {
                ++i; /* consume value */
            }
            continue;
        }
        paths[n++] = a;
    }
    *out_n = n;
    return paths;
}

/* -------------------------------------------------------------------------
 * main
 * ------------------------------------------------------------------------- */

/* Derive a short core display name from a library path. Strips directory
 * and the lib prefix / nes_core_ prefix / platform extension. */
static void core_display_name(const char* path, char* out, size_t cap) {
    const char* base = path;
    const char* slash = strrchr(path, '/');
    const char* bslash = strrchr(path, '\\');
    if (bslash && (!slash || bslash > slash)) base = bslash + 1;
    else if (slash) base = slash + 1;
    /* Strip known prefixes. */
    const char* p = base;
    if (strncmp(p, "lib", 3) == 0) p += 3;
    if (strncmp(p, "nes_core_", 9) == 0) p += 9;
    /* Strip extension. */
    char buf[128];
    strncpy(buf, p, sizeof(buf) - 1);
    buf[sizeof(buf) - 1] = '\0';
    char* dot = strrchr(buf, '.');
    if (dot) *dot = '\0';
    strncpy(out, buf, cap - 1);
    out[cap - 1] = '\0';
}

static const char* ALL_BENCHES[4] = { "step_frame", "cpu_step", "save_state", "render_frame" };

int main(int argc, char** argv) {
    cli_opts opts;
    if (cli_parse(&opts, argc, argv)) {
        return 2;
    }

    size_t rom_len = 0;
    const uint8_t* rom = nop_rom(&rom_len);

    /* Subprocess echo test mode: verify protocol framing, then exit. */
    if (opts.subprocess_cmd) {
        return subprocess_echo_test(opts.subprocess_cmd, rom, rom_len);
    }

    int n_paths = 0;
    const char** paths = collect_core_paths(argc, argv, &n_paths);
    if (!paths || n_paths == 0) {
        fprintf(stderr, "harness: no core library paths given.\n"
                        "Usage: harness [options] <core.dll> [<core2.dll> ...]\n"
                        "       harness --subprocess-test \"<cmd>\"\n");
        free((void*)paths);
        return 2;
    }

    /* Determine which benchmarks to run. */
    const char* benches[4];
    uint32_t n_benches = 0;
    if (opts.bench_filter) {
        benches[n_benches++] = opts.bench_filter;
    } else {
        for (int i = 0; i < 4; ++i) benches[n_benches++] = ALL_BENCHES[i];
    }

    core_result* results = (core_result*)calloc((size_t)n_paths, sizeof(core_result));
    uint32_t n_results = 0;

    for (int i = 0; i < n_paths; ++i) {
        const char* path = paths[i];
        char disp[64];
        core_display_name(path, disp, sizeof(disp));

        nes_core_api api;
        if (core_load(path, &api)) {
            fprintf(stderr, "harness: failed to load core '%s' — skipping\n", path);
            continue;
        }

        /* Resolve the core's self-reported name/version. */
        const char* iname = api.impl_name();
        const char* iver  = api.impl_version();

        /* --core filter: match against the path-derived display name OR the
         * core's self-reported impl_name (so both `--core rust` and
         * `--core rust-nes` select the same core). */
        if (opts.core_filter &&
            strcmp(opts.core_filter, disp) != 0 &&
            (!iname || strcmp(opts.core_filter, iname) != 0)) {
            core_unload(&api);
            continue;
        }

        core_result* cr = &results[n_results++];
        strncpy(cr->name, disp, sizeof(cr->name) - 1);
        cr->name[sizeof(cr->name) - 1] = '\0';
        if (iname) { strncpy(cr->name, iname, sizeof(cr->name) - 1); cr->name[sizeof(cr->name)-1]='\0'; }
        if (iver)  { strncpy(cr->version, iver, sizeof(cr->version) - 1); cr->version[sizeof(cr->version)-1]='\0'; }

        /* Probe mapper number with a throwaway instance. */
        nes_core_t* probe = api.create(rom, rom_len);
        if (probe) {
            cr->mapper = api.mapper_number(probe);
            api.destroy(probe);
        }

        printf("[harness] core=%s version=%s mapper=%u — running %u benchmark(s)\n",
               cr->name, cr->version, (unsigned)cr->mapper, n_benches);

        for (uint32_t b = 0; b < n_benches; ++b) {
            bench_result* br = &cr->results[cr->n_results];
            if (bench_run_one(&api, rom, rom_len, benches[b],
                              opts.frames, opts.instructions, br)) {
                fprintf(stderr, "  benchmark '%s' FAILED\n", benches[b]);
                continue;
            }
            cr->n_results++;
            double us_mean = br->stats.mean_ns / 1000.0;
            double us_p50  = br->stats.p50_ns / 1000.0;
            printf("  %-12s iters=%-8llu mean=%.3f us  p50=%.3f us  min=%.3f  max=%.3f  std=%.3f\n",
                   br->name, (unsigned long long)br->iterations,
                   us_mean, us_p50,
                   br->stats.min_ns / 1000.0, br->stats.max_ns / 1000.0,
                   br->stats.stddev_ns / 1000.0);
            if (opts.verbose) {
                /* Re-run is wasteful; verbose here just re-prints the summary
                 * line with p95/p99. Full per-iteration logging would require
                 * retaining the sample array, which bench_run_one frees. */
                printf("    p95=%.3f us  p99=%.3f us\n",
                       br->stats.p95_ns / 1000.0, br->stats.p99_ns / 1000.0);
            }
        }

        core_unload(&api);
    }

    /* Output JSON. */
    FILE* out = stdout;
    if (opts.output) {
        out = fopen(opts.output, "w");
        if (!out) {
            fprintf(stderr, "harness: cannot open --output '%s' (%s); using stdout\n",
                    opts.output, strerror(errno));
            out = stdout;
        }
    }
    print_json(out, results, n_results);
    if (opts.csv) {
        print_csv(stdout, results, n_results);
    }
    if (out != stdout) fclose(out);

    free(results);
    free((void*)paths);
    return (n_results > 0) ? 0 : 1;
}
