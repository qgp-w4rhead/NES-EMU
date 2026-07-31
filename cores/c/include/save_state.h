/*
 * save_state.h - Serialisation/deserialisation of full emulator state.
 *
 * Port of src/save_state/mod.rs to C (M4.5). Uses a custom binary format
 * (NOT bincode) so the C core's save states are self-contained and do not
 * depend on a Rust crate. The format embeds a magic + version tag so it
 * can be validated on load.
 *
 * See: https://www.nesdev.org/wiki/Save_state
 */
#ifndef NES_CORE_C_SAVE_STATE_H
#define NES_CORE_C_SAVE_STATE_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>
#include "emulator.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Save state format version. Increment when the layout changes.
 * (save_state/mod.rs `SAVE_STATE_VERSION`.) */
#define SAVE_STATE_VERSION 1u

/* Magic bytes "NESS" (little-endian u32 = 0x5353454E). */
#define SAVE_STATE_MAGIC 0x5353454Eu

/* Serialize the full emulator state into `buf` (at most `cap` bytes).
 * Returns the number of bytes written, or 0 on failure (buffer too small
 * or NULL core). If `buf` is NULL, returns the required buffer size
 * (equivalent to emulator_save_state_size). (save_state/mod.rs
 * `EmulatorState::save_state`.) */
size_t emulator_save_state(const EmulatorState* emu, uint8_t* buf, size_t cap);

/* Restore emulator state from `buf` (`len` bytes previously produced by
 * emulator_save_state). Returns true on success, false on failure (bad
 * magic / version, size mismatch, NULL core/buf). On failure the core is
 * left in an indeterminate state - the caller should emulator_reset
 * before further use. (save_state/mod.rs `EmulatorState::load_state`.) */
bool emulator_load_state(EmulatorState* emu, const uint8_t* buf, size_t len);

/* Required buffer size for emulator_save_state. (Convenience wrapper.) */
size_t emulator_save_state_size(const EmulatorState* emu);

#ifdef __cplusplus
}
#endif
#endif /* NES_CORE_C_SAVE_STATE_H */
