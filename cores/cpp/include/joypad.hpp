/*
 * joypad.h - NES standard controller input ($4016/$4017 strobe + shift
 *            register polling for two controllers).
 *
 * Port of src/joypad.rs to C (M4.5). The NES controller is read through a
 * serial shift register: strobe on (write 1 to $4016 bit 0) continuously
 * reloads the shift register with the live button state; strobe off (1->0
 * transition) freezes a snapshot and resets the read counters; subsequent
 * reads of $4016/$4017 return the next button bit in order A, B, Select,
 * Start, Up, Down, Left, Right. After 8 reads the shift register is
 * exhausted and subsequent reads return 1.
 *
 * See: https://www.nesdev.org/wiki/Controller_port
 * See: https://www.nesdev.org/wiki/Standard_controller
 */
#ifndef NES_CORE_C_JOYPAD_H
#define NES_CORE_C_JOYPAD_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

/* Number of controllers supported (controller 1 at $4016, controller 2 at
 * $4017). (joypad.rs `CONTROLLER_COUNT`.) */
#define JOYPAD_CONTROLLER_COUNT 2
/* Number of buttons on a standard NES controller. (joypad.rs `BUTTON_COUNT`.) */
#define JOYPAD_BUTTON_COUNT 8

/* Button bit indices in the snapshot byte. These double as the `button`
 * argument to joypad_set_button. (joypad.rs `button` module.) */
#define JOYPAD_BUTTON_A      0u
#define JOYPAD_BUTTON_B      1u
#define JOYPAD_BUTTON_SELECT 2u
#define JOYPAD_BUTTON_START  3u
#define JOYPAD_BUTTON_UP     4u
#define JOYPAD_BUTTON_DOWN   5u
#define JOYPAD_BUTTON_LEFT   6u
#define JOYPAD_BUTTON_RIGHT  7u

/* The NES joypad state for two standard controllers. Owns the live button
 * state (updated by the host input layer) and the shift-register snapshot
 * used by the CPU polling sequence. The strobe logic follows the NESdev
 * wiki: a high strobe continuously reloads the shift register; a 1->0
 * strobe transition freezes a snapshot and resets the read counters.
 * (joypad.rs `struct Joypad`.) */
typedef struct Joypad {
    /* Live button state for each controller, one bit per button using the
     * button module bit layout. Updated by the host input layer via
     * joypad_set_button. (joypad.rs `current` - #[serde(skip)].) */
    uint8_t current[JOYPAD_CONTROLLER_COUNT];

    /* Current strobe line state (bit 0 of the last write to $4016). */
    bool strobe;

    /* Frozen snapshot of the button state at the moment the strobe went
     * from high to low. Read out one bit at a time by joypad_read. */
    uint8_t shift[JOYPAD_CONTROLLER_COUNT];

    /* Number of reads performed since the last 1->0 strobe transition,
     * per controller. Bits 0..8 of `shift` are returned in order; reads
     * beyond 8 return 1. */
    uint8_t counter[JOYPAD_CONTROLLER_COUNT];
} Joypad;

/* Construct a joypad with no buttons pressed and the strobe low.
 * (joypad.rs `Joypad::new`.) */
void joypad_init(Joypad* j);

/* Set or clear a button on the given controller (0 or 1). `button` is one
 * of the JOYPAD_BUTTON_* constants. `pressed` is true when the button is
 * held down. While the strobe is high, the shift register tracks the live
 * state so this update is immediately visible to reads.
 * (joypad.rs `Joypad::set_button`.) */
void joypad_set_button(Joypad* j, uint8_t controller, uint8_t button, bool pressed);

/* Write to $4016 bit 0 - the controller strobe line. A 1->0 transition
 * snapshots the live button state into the shift registers and resets the
 * read counters. While the strobe is high the shift registers continuously
 * track the live state. (joypad.rs `Joypad::write_strobe`.) */
void joypad_write_strobe(Joypad* j, uint8_t value);

/* Read one bit from the given controller (0 = $4016, 1 = $4017). Returns
 * 1 or 0 in bit 0. While the strobe is high, reads return the live A
 * button state. Otherwise each read returns the next button in order
 * (A, B, Select, Start, Up, Down, Left, Right) and advances the counter;
 * reads beyond 8 return 1. (joypad.rs `Joypad::read`.) */
uint8_t joypad_read(Joypad* j, uint8_t controller);

/* Current strobe line state. (joypad.rs `Joypad::strobe`.) */
bool joypad_strobe(const Joypad* j);

/* Clear all buttons on both controllers (e.g. on host focus loss).
 * (joypad.rs `Joypad::clear`.) */
void joypad_clear(Joypad* j);

/* Live button state for a controller (for diagnostics / tests).
 * (joypad.rs `Joypad::current`.) */
uint8_t joypad_current(const Joypad* j, uint8_t controller);

/* Frozen snapshot for a controller (for diagnostics / tests).
 * (joypad.rs `Joypad::shift`.) */
uint8_t joypad_shift(const Joypad* j, uint8_t controller);

#ifdef __cplusplus
}
#endif
#endif /* NES_CORE_C_JOYPAD_H */
