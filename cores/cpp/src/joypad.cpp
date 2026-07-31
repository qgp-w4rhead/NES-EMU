/*
 * joypad.c - NES standard controller input ($4016/$4017 strobe + shift
 *            register polling for two controllers).
 *
 * Port of src/joypad.rs to C (M4.5). Field-for-field and
 * function-for-function port of the Rust Joypad. The strobe falling-edge
 * snapshots the live button state into the shift registers and resets the
 * read counters; reads while strobed return the live A button; reads
 * beyond 8 return 1 (saturating counter).
 *
 * See: https://www.nesdev.org/wiki/Controller_port
 */
#include "joypad.hpp"
#include <string.h>

void joypad_init(Joypad* j) {
    memset(j, 0, sizeof(*j));
}

void joypad_set_button(Joypad* j, uint8_t controller, uint8_t button, bool pressed) {
    if (controller >= JOYPAD_CONTROLLER_COUNT || button >= JOYPAD_BUTTON_COUNT) {
        return;
    }
    uint8_t mask = (uint8_t)(1u << button);
    if (pressed) {
        j->current[controller] = (uint8_t)(j->current[controller] | mask);
    } else {
        j->current[controller] = (uint8_t)(j->current[controller] & (uint8_t)~mask);
    }
    /* While the strobe is high the shift register is continuously reloaded,
     * so live changes are reflected immediately. */
    if (j->strobe) {
        j->shift[controller] = j->current[controller];
    }
}

void joypad_write_strobe(Joypad* j, uint8_t value) {
    bool new_strobe = (value & 0x01u) != 0u;
    if (j->strobe && !new_strobe) {
        /* Falling edge: freeze a snapshot and reset the read counters. */
        for (uint8_t c = 0; c < JOYPAD_CONTROLLER_COUNT; ++c) {
            j->shift[c] = j->current[c];
            j->counter[c] = 0u;
        }
    }
    j->strobe = new_strobe;
    if (j->strobe) {
        /* While strobed, the shift register tracks the live state. */
        for (uint8_t c = 0; c < JOYPAD_CONTROLLER_COUNT; ++c) {
            j->shift[c] = j->current[c];
        }
    }
}

uint8_t joypad_read(Joypad* j, uint8_t controller) {
    if (controller >= JOYPAD_CONTROLLER_COUNT) {
        return 1u;
    }
    if (j->strobe) {
        /* While strobed, reads continuously return the A button. */
        return (uint8_t)(j->current[controller] & 0x01u);
    }
    uint8_t bit;
    if (j->counter[controller] < JOYPAD_BUTTON_COUNT) {
        bit = (uint8_t)((j->shift[controller] >> j->counter[controller]) & 0x01u);
    } else {
        /* Standard controllers return 1 after the 8 button bits. */
        bit = 1u;
    }
    /* Saturating add (matches joypad.rs `saturating_add`). */
    if (j->counter[controller] < 0xFFu) {
        j->counter[controller] = (uint8_t)(j->counter[controller] + 1u);
    }
    return bit;
}

bool joypad_strobe(const Joypad* j) {
    return j->strobe;
}

void joypad_clear(Joypad* j) {
    for (uint8_t c = 0; c < JOYPAD_CONTROLLER_COUNT; ++c) {
        j->current[c] = 0u;
        if (j->strobe) {
            j->shift[c] = 0u;
        }
    }
}

uint8_t joypad_current(const Joypad* j, uint8_t controller) {
    if (controller >= JOYPAD_CONTROLLER_COUNT) {
        return 0u;
    }
    return j->current[controller];
}

uint8_t joypad_shift(const Joypad* j, uint8_t controller) {
    if (controller >= JOYPAD_CONTROLLER_COUNT) {
        return 0u;
    }
    return j->shift[controller];
}
