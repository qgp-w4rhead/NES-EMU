/*
 * core_api.cpp - NES polyglot core C ABI shim (cores/protocol.h).
 *
 * Implements the 13-function C ABI defined in cores/protocol.h so the
 * benchmark harness in bench/ can load the C++ core as a shared library
 * (nes_core_cpp.dll) and drive it identically to the Rust / C / Zig / etc.
 * cores. The opaque nes_core_t handle is a heap-allocated EmulatorState.
 *
 * Exports are handled by nes_core.def (not __declspec(dllexport)) so the
 * implementation matches the declarations in protocol.h exactly. NULL-core
 * checks return 0/NULL/-1 as documented in protocol.h.
 *
 * Every extern "C" export wraps its body in try/catch(...) so that no C++
 * exception can ever propagate across the FFI boundary (which would be
 * undefined behavior per [except.handle]/11). The catch-all returns the
 * spec-mandated safe default (0 / NULL / -1) for each function.
 */
#include "../../protocol.h"  /* cores/protocol.h (two levels up from cores/cpp/src/) */
#include "emulator.hpp"
#include "save_state.hpp"
#include "cartridge.hpp"
#include "region.hpp"
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

/* Static implementation identification strings (protocol.h
 * nes_core_impl_name / nes_core_impl_version). */
static const char IMPL_NAME[] = "cpp-nes";
static const char IMPL_VERSION[] = "0.1.0";

/* ---- Lifecycle ------------------------------------------------------- */

nes_core_t* nes_core_create(const uint8_t* rom_data, size_t rom_len) {
    if (!rom_data || rom_len == 0u) {
        return NULL;
    }
    try {
        Cartridge* cart = (Cartridge*)malloc(sizeof(Cartridge));
        if (!cart) {
            return NULL;
        }
        if (cartridge_from_bytes(rom_data, rom_len, cart) != 0) {
            free(cart);
            return NULL;
        }
        EmulatorState* emu = (EmulatorState*)malloc(sizeof(EmulatorState));
        if (!emu) {
            cartridge_destroy(cart);
            free(cart);
            return NULL;
        }
        emulator_init(emu);
        emu->cartridge = cart;
        bus_insert_cartridge(&emu->bus, cart);
        return (nes_core_t*)emu;
    } catch (...) {
        return NULL;
    }
}

void nes_core_destroy(nes_core_t* core) {
    if (!core) {
        return;
    }
    try {
        EmulatorState* emu = (EmulatorState*)core;
        emulator_destroy(emu);
        free(emu);
    } catch (...) {
        /* Swallow: destroy must not throw across the FFI boundary. */
    }
}

void nes_core_reset(nes_core_t* core) {
    if (!core) {
        return;
    }
    try {
        emulator_reset((EmulatorState*)core);
    } catch (...) {
        /* Swallow: reset must not throw across the FFI boundary. */
    }
}

int nes_core_set_region(nes_core_t* core, int region) {
    if (!core) {
        return -1;
    }
    if (region < 0 || region > (int)NES_REGION_DENDY) {
        return -1;
    }
    try {
        EmulatorState* emu = (EmulatorState*)core;
        int prev = (int)emulator_region(emu);
        emulator_set_region(emu, (Region)region);
        return prev;
    } catch (...) {
        return -1;
    }
}

/* ---- Stepping -------------------------------------------------------- */

uint32_t nes_core_step_frame(nes_core_t* core) {
    if (!core) {
        return 0u;
    }
    try {
        return emulator_step_frame((EmulatorState*)core);
    } catch (...) {
        return 0u;
    }
}

uint32_t nes_core_step_instruction(nes_core_t* core) {
    if (!core) {
        return 0u;
    }
    try {
        return emulator_step_instruction((EmulatorState*)core);
    } catch (...) {
        return 0u;
    }
}

/* ---- Output ---------------------------------------------------------- */

const uint32_t* nes_core_framebuffer(nes_core_t* core) {
    if (!core) {
        return NULL;
    }
    try {
        return emulator_framebuffer((const EmulatorState*)core);
    } catch (...) {
        return NULL;
    }
}

size_t nes_core_take_audio(nes_core_t* core, int16_t* buf, size_t cap) {
    if (!core || !buf || cap == 0u) {
        return 0u;
    }
    try {
        EmulatorState* emu = (EmulatorState*)core;
        /* Drain the float audio buffer into a temporary float buffer, then
         * convert to int16_t with saturation. */
        float* tmp = (float*)malloc(cap * sizeof(float));
        if (!tmp) {
            return 0u;
        }
        size_t n = emulator_take_audio_samples(emu, tmp, cap);
        for (size_t i = 0; i < n; ++i) {
            float s = tmp[i];
            if (s > 1.0f) s = 1.0f;
            if (s < -1.0f) s = -1.0f;
            /* Saturating conversion: [-1,1] -> [-32768, 32767]. */
            int32_t v;
            if (s >= 0.0f) {
                v = (int32_t)(s * 32767.0f);
                if (v > 32767) v = 32767;
            } else {
                v = (int32_t)(s * 32768.0f);
                if (v < -32768) v = -32768;
            }
            buf[i] = (int16_t)v;
        }
        free(tmp);
        return n;
    } catch (...) {
        return 0u;
    }
}

/* ---- Save state ------------------------------------------------------ */

size_t nes_core_save_state(nes_core_t* core, uint8_t* buf, size_t cap) {
    /* Match the Rust reference (lib.rs:271-273): a NULL/zero-cap buffer is an
     * error, not a size query. Callers that need the required size should
     * allocate a generously-sized buffer (the harness uses 1 MiB). */
    if (!core || !buf || cap == 0u) {
        return 0u;
    }
    try {
        return emulator_save_state((const EmulatorState*)core, buf, cap);
    } catch (...) {
        return 0u;
    }
}

int nes_core_load_state(nes_core_t* core, const uint8_t* buf, size_t len) {
    if (!core || !buf || len == 0u) {
        return 0;
    }
    try {
        return emulator_load_state((EmulatorState*)core, buf, len) ? 1 : 0;
    } catch (...) {
        return 0;
    }
}

/* ---- Introspection --------------------------------------------------- */

uint16_t nes_core_mapper_number(nes_core_t* core) {
    if (!core) {
        return 0u;
    }
    try {
        return emulator_mapper_number((const EmulatorState*)core);
    } catch (...) {
        return 0u;
    }
}

const char* nes_core_impl_name(void) {
    return IMPL_NAME;
}

const char* nes_core_impl_version(void) {
    return IMPL_VERSION;
}
