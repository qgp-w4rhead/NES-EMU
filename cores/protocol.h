/*
 * cores/protocol.h — NES polyglot core C ABI
 *
 * Single source of truth for the cross-language NES core interface. Every
 * language port (Rust, C, C++, Zig, Go, C#, Java, TypeScript, Python) must
 * expose these symbols with C linkage so the benchmark harness in `bench/`
 * can load any core as a shared library and drive it identically.
 *
 * Consumer compatibility targets:
 *   - C89/C99/C11  (any conforming C compiler)
 *   - C++11+        (extern "C" guard below)
 *   - Zig           (@cImport — header must be plain C, no C++ constructs)
 *   - C#            (P/Invoke via DllImport — header is reference-only)
 *
 * ABI rules (see tasklist/M1.md):
 *   - Default C calling convention (__cdecl on x86, standard on x64).
 *   - `nes_core_t*` is opaque — the harness never inspects its layout.
 *   - The framebuffer pointer returned by `nes_core_framebuffer` is owned by
 *     the core and remains valid until the next `nes_core_step_frame` call.
 *   - No global mutable state; one handle = one independent NES instance.
 *     Instances are not required to be thread-safe, but distinct instances
 *     must be usable from distinct threads concurrently.
 *   - On Windows, implementations export symbols without name decoration
 *     (via a .def file or __declspec(dllexport) on the *implementation* side;
 *     this consumer header carries no decoration so it can be included by
 *     any language).
 *
 * Framebuffer layout: 256x240 32-bit ARGB pixels (0xAARRGGBB), row-major,
 * stride = 256. The pointer is non-null for a valid core and is stable for
 * the lifetime of the instance (the core reuses one buffer).
 *
 * Audio: `nes_core_take_audio` drains the core's audio ring buffer into the
 * caller-supplied buffer and returns the number of samples written. Each
 * sample is one int16_t (mono). The core downmixes its native sample rate to
 * 44100 Hz before queuing.
 */
#ifndef NES_CORE_PROTOCOL_H
#define NES_CORE_PROTOCOL_H

#include <stddef.h> /* size_t */
#include <stdint.h> /* uint16_t, uint32_t */

#ifdef __cplusplus
extern "C" {
#endif

/* -------------------------------------------------------------------------
 * Opaque handle
 * ------------------------------------------------------------------------- */

/*
 * Opaque NES core instance. Implementations cast to/from their own concrete
 * struct. The harness treats this as an opaque pointer and never dereferences
 * it. Defined as an incomplete struct (not void*) so the compiler can still
 * type-check pointer usage.
 */
typedef struct nes_core_t nes_core_t;

/* -------------------------------------------------------------------------
 * Region selector for nes_core_set_region
 *
 * Values are fixed integers (not an enum) so Zig @cImport and C# P/Invoke
 * see stable integer constants rather than a C enum whose underlying type is
 * implementation-defined.
 * ------------------------------------------------------------------------- */

#define NES_REGION_NTSC  0  /* NTSC: 60.0988 Hz, 262 scanlines */
#define NES_REGION_PAL   1  /* PAL:  50.0070 Hz, 312 scanlines */
#define NES_REGION_DENDY 2  /* Dendy: PAL timing, NTSC PPU palette */

/* -------------------------------------------------------------------------
 * Lifecycle
 * ------------------------------------------------------------------------- */

/*
 * Create a new NES core instance from an iNES/FDS ROM image held in memory.
 * `rom_data` points to the raw ROM bytes (header + PRG/CHR); the core copies
 * what it needs and does not retain the pointer. Returns NULL on failure
 * (bad header, unsupported mapper, OOM). The caller owns the ROM buffer and
 * may free it immediately after this call returns.
 */
nes_core_t* nes_core_create(const uint8_t* rom_data, size_t rom_len);

/*
 * Destroy a core instance and release all owned resources. Passing NULL is
 * a no-op. The handle is invalid after this call.
 */
void nes_core_destroy(nes_core_t* core);

/*
 * Hard reset the core (equivalent to power-cycle). Soft reset (RESET vector)
 * is not exposed by this ABI; the harness only needs power-cycle semantics
 * for deterministic benchmarking. Safe to call on a freshly created core.
 */
void nes_core_reset(nes_core_t* core);

/*
 * Set the target region (NES_REGION_*). Takes effect on the next reset /
 * frame boundary. Returns the previous region, or -1 if `core` is NULL or
 * `region` is out of range.
 */
int nes_core_set_region(nes_core_t* core, int region);

/* -------------------------------------------------------------------------
 * Stepping
 * ------------------------------------------------------------------------- */

/*
 * Run the core until one full frame has been rendered. Returns the number of
 * CPU cycles consumed for the frame (≈29830 for NTSC). The framebuffer
 * pointer obtained from nes_core_framebuffer is updated in place by this
 * call. Returns 0 if `core` is NULL.
 */
uint32_t nes_core_step_frame(nes_core_t* core);

/*
 * Execute exactly one CPU instruction and return the number of CPU cycles it
 * consumed (including page-crossing and branch penalties). Useful for
 * instruction-level benchmarking and single-stepping. Returns 0 if `core` is
 * NULL.
 */
uint32_t nes_core_step_instruction(nes_core_t* core);

/* -------------------------------------------------------------------------
 * Output
 * ------------------------------------------------------------------------- */

/*
 * Return a pointer to the core's 256x240 ARGB framebuffer (256*240 uint32_t
 * values, row-major). The buffer is owned by the core and remains valid for
 * the lifetime of the instance; its contents are stable between
 * nes_core_step_frame calls. Returns NULL if `core` is NULL.
 *
 * The pointer is guaranteed stable for the instance lifetime (the core does
 * not reallocate the buffer), so callers may cache it after creation.
 */
const uint32_t* nes_core_framebuffer(nes_core_t* core);

/*
 * Drain queued audio samples into `buf` (mono int16_t PCM, 44100 Hz). At most
 * `cap` samples are written. Returns the number of samples actually written
 * (0 .. cap). If `core` is NULL or `buf` is NULL, returns 0 without touching
 * `buf`. The core's internal queue is emptied by this call for the samples
 * that fit; overflow samples are dropped (the harness polls frequently enough
 * that this should not happen in practice).
 */
size_t nes_core_take_audio(nes_core_t* core, int16_t* buf, size_t cap);

/* -------------------------------------------------------------------------
 * Save state
 * ------------------------------------------------------------------------- */

/*
 * Serialize the full core state into `buf` (at most `cap` bytes). Returns the
 * number of bytes written, or 0 on failure (buffer too small, or NULL core).
 * The serialized blob is self-describing (it embeds a version tag) so it can
 * be loaded back into the same core build via nes_core_load_state. A return
 * value of 0 with a non-NULL `core` and non-zero `cap` indicates an error.
 */
size_t nes_core_save_state(nes_core_t* core, uint8_t* buf, size_t cap);

/*
 * Restore core state from `buf` (`len` bytes previously produced by
 * nes_core_save_state). Returns 1 on success, 0 on failure (bad magic /
 * version, size mismatch, NULL core/buf). On failure the core is left in an
 * indeterminate state and the caller should nes_core_reset before further
 * use.
 */
int nes_core_load_state(nes_core_t* core, const uint8_t* buf, size_t len);

/* -------------------------------------------------------------------------
 * Introspection
 * ------------------------------------------------------------------------- */

/*
 * Return the iNES mapper number (0..255) of the loaded ROM, or 0 if no ROM is
 * loaded. Mapper 0 (NROM) is a valid return; the harness distinguishes "no
 * ROM" by checking nes_core_create's return value rather than this function.
 */
uint16_t nes_core_mapper_number(nes_core_t* core);

/*
 * Return a human-readable name for the implementation, e.g. "rust-nes",
 * "c-nes", "cpp-nes". The string is static (no allocation) and valid for the
 * entire process lifetime. Never returns NULL.
 */
const char* nes_core_impl_name(void);

/*
 * Return a human-readable version string for the implementation, e.g.
 * "0.1.0". The string is static and valid for the entire process lifetime.
 * Never returns NULL.
 */
const char* nes_core_impl_version(void);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* NES_CORE_PROTOCOL_H */
