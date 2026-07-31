/*
 * bench/fb_compare.c — framebuffer comparison between two NES core DLLs.
 *
 * Loads two cores (typically the C reference + a ported core), runs N frames
 * on each with the NOP ROM, and compares framebuffers + audio + cycle counts
 * per frame. Reports any mismatches. Used to verify ported cores produce
 * byte-identical output to the C reference.
 *
 * Build: cl /O2 fb_compare.c gen_rom.c /Fe:fb_compare.exe
 * Usage: fb_compare.exe <ref.dll> <port.dll> [frames]
 */
#include "gen_rom.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#if defined(_WIN32)
#  define WIN32_LEAN_AND_MEAN
#  include <windows.h>
typedef HMODULE lib_t;
static lib_t lib_open(const char* p) { return LoadLibraryA(p); }
static void lib_close(lib_t h) { if (h) FreeLibrary(h); }
static void* lib_sym(lib_t h, const char* n) { return (void*)GetProcAddress(h, n); }
#else
#  include <dlfcn.h>
typedef void* lib_t;
static lib_t lib_open(const char* p) { return dlopen(p, RTLD_NOW|RTLD_LOCAL); }
static void lib_close(lib_t h) { if (h) dlclose(h); }
static void* lib_sym(lib_t h, const char* n) { return dlsym(h, n); }
#endif

/* protocol.h function pointer types */
typedef void* (*create_fn)(const unsigned char*, size_t);
typedef void (*destroy_fn)(void*);
typedef void (*reset_fn)(void*);
typedef unsigned int (*step_frame_fn)(void*);
typedef const unsigned int* (*framebuffer_fn)(void*);
typedef size_t (*take_audio_fn)(void*, short*, size_t);

typedef struct {
    lib_t lib;
    create_fn create;
    destroy_fn destroy;
    reset_fn reset;
    step_frame_fn step_frame;
    framebuffer_fn framebuffer;
    take_audio_fn take_audio;
} core_api;

static int load_core(const char* path, core_api* api) {
    memset(api, 0, sizeof(*api));
    api->lib = lib_open(path);
    if (!api->lib) { fprintf(stderr, "failed to load %s\n", path); return 1; }
    api->create = (create_fn)lib_sym(api->lib, "nes_core_create");
    api->destroy = (destroy_fn)lib_sym(api->lib, "nes_core_destroy");
    api->reset = (reset_fn)lib_sym(api->lib, "nes_core_reset");
    api->step_frame = (step_frame_fn)lib_sym(api->lib, "nes_core_step_frame");
    api->framebuffer = (framebuffer_fn)lib_sym(api->lib, "nes_core_framebuffer");
    api->take_audio = (take_audio_fn)lib_sym(api->lib, "nes_core_take_audio");
    if (!api->create || !api->destroy || !api->reset || !api->step_frame ||
        !api->framebuffer || !api->take_audio) {
        fprintf(stderr, "missing symbols in %s\n", path);
        return 1;
    }
    return 0;
}

#define FB_PIXELS (256 * 240)
#define AUDIO_CAP 4096

int main(int argc, char** argv) {
    if (argc < 3) {
        fprintf(stderr, "Usage: %s <ref.dll> <port.dll> [frames]\n", argv[0]);
        return 1;
    }
    int frames = (argc >= 4) ? atoi(argv[3]) : 20;

    core_api ref, port;
    if (load_core(argv[1], &ref)) return 1;
    if (load_core(argv[2], &port)) return 1;

    size_t rom_len = 0;
    const unsigned char* rom = nop_rom(&rom_len);

    void* ref_core = ref.create(rom, rom_len);
    void* port_core = port.create(rom, rom_len);
    if (!ref_core || !port_core) {
        fprintf(stderr, "create failed (ref=%p port=%p)\n", ref_core, port_core);
        return 1;
    }
    ref.reset(ref_core);
    port.reset(port_core);

    int fb_mismatches = 0;
    int audio_mismatches = 0;
    int cycle_mismatches = 0;
    unsigned int total_fb_diff_pixels = 0;

    short ref_audio[AUDIO_CAP];
    short port_audio[AUDIO_CAP];

    for (int f = 0; f < frames; f++) {
        unsigned int ref_cycles = ref.step_frame(ref_core);
        unsigned int port_cycles = port.step_frame(port_core);
        if (ref_cycles != port_cycles) {
            if (cycle_mismatches == 0) {
                fprintf(stderr, "frame %d: cycle mismatch ref=%u port=%u\n",
                        f, ref_cycles, port_cycles);
            }
            cycle_mismatches++;
        }

        const unsigned int* ref_fb = ref.framebuffer(ref_core);
        const unsigned int* port_fb = port.framebuffer(port_core);
        int frame_diff = 0;
        for (int i = 0; i < FB_PIXELS; i++) {
            if (ref_fb[i] != port_fb[i]) {
                frame_diff++;
                if (fb_mismatches == 0 && frame_diff <= 5) {
                    fprintf(stderr, "frame %d pixel %d: ref=%08X port=%08X\n",
                            f, i, ref_fb[i], port_fb[i]);
                }
            }
        }
        if (frame_diff > 0) {
            fb_mismatches++;
            total_fb_diff_pixels += frame_diff;
        }

        size_t ref_n = ref.take_audio(ref_core, ref_audio, AUDIO_CAP);
        size_t port_n = port.take_audio(port_core, port_audio, AUDIO_CAP);
        if (ref_n != port_n) {
            if (audio_mismatches == 0) {
                fprintf(stderr, "frame %d: audio count mismatch ref=%zu port=%zu\n",
                        f, ref_n, port_n);
            }
            audio_mismatches++;
        } else {
            for (size_t i = 0; i < ref_n; i++) {
                if (ref_audio[i] != port_audio[i]) {
                    if (audio_mismatches == 0) {
                        fprintf(stderr, "frame %d audio %zu: ref=%d port=%d\n",
                                f, i, ref_audio[i], port_audio[i]);
                    }
                    audio_mismatches++;
                    break;
                }
            }
        }
    }

    printf("Frames compared: %d\n", frames);
    printf("Framebuffer mismatches: %d frames, %u total diff pixels\n",
           fb_mismatches, total_fb_diff_pixels);
    printf("Audio mismatches: %d frames\n", audio_mismatches);
    printf("Cycle count mismatches: %d frames\n", cycle_mismatches);

    ref.destroy(ref_core);
    port.destroy(port_core);
    lib_close(ref.lib);
    lib_close(port.lib);

    return (fb_mismatches || audio_mismatches || cycle_mismatches) ? 1 : 0;
}
