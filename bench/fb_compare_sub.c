/*
 * bench/fb_compare_sub.c — compare a NES core DLL against a subprocess core.
 *
 * Loads a reference shared-library core (typically the C core) and spawns a
 * subprocess core speaking the M3 binary protocol (e.g. the TypeScript core
 * via `node dist/main.js`). Runs N frames of the NOP ROM on each, one frame
 * at a time, and compares framebuffer + audio + cycle count per frame.
 * Reports any mismatches. Used to verify subprocess cores produce
 * byte-identical output to the C reference (M10/M11 acceptance).
 *
 * Build: cl /O2 fb_compare_sub.c gen_rom.c /Fe:fb_compare_sub.exe
 * Usage: fb_compare_sub.exe <ref.dll> "<subprocess cmdline>" [frames]
 *
 * Example:
 *   fb_compare_sub.exe cores\c\nes_core_c.dll "node cores\typescript\dist\main.js" 20
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
#  include <unistd.h>
#  include <dlfcn.h>
typedef void* lib_t;
static lib_t lib_open(const char* p) { return dlopen(p, RTLD_NOW|RTLD_LOCAL); }
static void lib_close(lib_t h) { if (h) dlclose(h); }
static void* lib_sym(lib_t h, const char* n) { return dlsym(h, n); }
#endif

/* ---- reference DLL function pointers ---- */
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

/* ---- subprocess protocol (M3) ---- */
#if defined(_WIN32)
typedef struct { HANDLE proc; HANDLE hchild; HANDLE hchild_out; } subproc_t;
#else
typedef struct { pid_t pid; int in_fd; int out_fd; } subproc_t;
#endif

static int write_all(subproc_t* sp, const void* buf, size_t len) {
    const unsigned char* p = (const unsigned char*)buf;
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
static int read_all(subproc_t* sp, void* buf, size_t len) {
    unsigned char* p = (unsigned char*)buf;
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
static void put_u32_le(unsigned char* b, unsigned int v) {
    b[0] = (unsigned char)(v & 0xFF);
    b[1] = (unsigned char)((v >> 8) & 0xFF);
    b[2] = (unsigned char)((v >> 16) & 0xFF);
    b[3] = (unsigned char)((v >> 24) & 0xFF);
}
static unsigned int get_u32_le(const unsigned char* b) {
    return (unsigned int)b[0] | ((unsigned int)b[1] << 8) |
           ((unsigned int)b[2] << 16) | ((unsigned int)b[3] << 24);
}

static int sub_spawn(subproc_t* sp, const char* cmdline) {
    memset(sp, 0, sizeof(*sp));
#if defined(_WIN32)
    SECURITY_ATTRIBUTES sa;
    sa.nLength = sizeof(sa); sa.bInheritHandle = TRUE; sa.lpSecurityDescriptor = NULL;
    HANDLE cin_rd=NULL, cin_wr=NULL, cout_rd=NULL, cout_wr=NULL;
    if (!CreatePipe(&cin_rd, &cin_wr, &sa, 0)) return 1;
    if (!CreatePipe(&cout_rd, &cout_wr, &sa, 0)) { CloseHandle(cin_rd); CloseHandle(cin_wr); return 1; }
    SetHandleInformation(cin_wr, HANDLE_FLAG_INHERIT, 0);
    SetHandleInformation(cout_rd, HANDLE_FLAG_INHERIT, 0);
    STARTUPINFOA si; PROCESS_INFORMATION pi;
    memset(&si, 0, sizeof(si)); si.cb = sizeof(si);
    si.dwFlags = STARTF_USESTDHANDLES;
    si.hStdInput = cin_rd; si.hStdOutput = cout_wr; si.hStdError = GetStdHandle(STD_ERROR_HANDLE);
    memset(&pi, 0, sizeof(pi));
    char* cmd = _strdup(cmdline);
    if (!cmd) { CloseHandle(cin_rd); CloseHandle(cin_wr); CloseHandle(cout_rd); CloseHandle(cout_wr); return 1; }
    BOOL ok = CreateProcessA(NULL, cmd, NULL, NULL, TRUE, CREATE_NO_WINDOW, NULL, NULL, &si, &pi);
    free(cmd);
    CloseHandle(cin_rd); CloseHandle(cout_wr);
    if (!ok) { CloseHandle(cin_wr); CloseHandle(cout_rd); return 1; }
    sp->proc = pi.hProcess; sp->hchild = cin_wr; sp->hchild_out = cout_rd;
    CloseHandle(pi.hThread);
    return 0;
#else
    int in_pipe[2], out_pipe[2];
    if (pipe(in_pipe) || pipe(out_pipe)) return 1;
    pid_t pid = fork();
    if (pid < 0) return 1;
    if (pid == 0) {
        dup2(in_pipe[0], 0); dup2(out_pipe[1], 1);
        close(in_pipe[0]); close(in_pipe[1]); close(out_pipe[0]); close(out_pipe[1]);
        execl("/bin/sh", "sh", "-c", cmdline, (char*)NULL);
        _exit(127);
    }
    close(in_pipe[0]); close(out_pipe[1]);
    sp->pid = pid; sp->in_fd = in_pipe[1]; sp->out_fd = out_pipe[0];
    return 0;
#endif
}
static void sub_close(subproc_t* sp) {
#if defined(_WIN32)
    if (sp->hchild) { CloseHandle(sp->hchild); sp->hchild = NULL; }
    if (sp->hchild_out) { CloseHandle(sp->hchild_out); sp->hchild_out = NULL; }
    if (sp->proc) { WaitForSingleObject(sp->proc, 2000); CloseHandle(sp->proc); sp->proc = NULL; }
#else
    if (sp->in_fd >= 0) { close(sp->in_fd); sp->in_fd = -1; }
    if (sp->out_fd >= 0) { close(sp->out_fd); sp->out_fd = -1; }
    if (sp->pid > 0) { waitpid(sp->pid, NULL, 0); sp->pid = -1; }
#endif
}

static int sub_send_init(subproc_t* sp, const unsigned char* rom, size_t rom_len) {
    unsigned char hdr[5]; hdr[0] = 0x01; put_u32_le(hdr + 1, (unsigned int)rom_len);
    if (write_all(sp, hdr, 5)) return 1;
    return write_all(sp, rom, rom_len);
}
static int sub_send_reset(subproc_t* sp) {
    unsigned char b = 0x02; return write_all(sp, &b, 1);
}
static int sub_send_step_frame(subproc_t* sp, unsigned int count) {
    unsigned char hdr[5]; hdr[0] = 0x03; put_u32_le(hdr + 1, count);
    return write_all(sp, hdr, 5);
}
static int sub_recv_ok(subproc_t* sp) {
    unsigned char tag;
    if (read_all(sp, &tag, 1)) return -1;
    if (tag == 0x00) return 0;
    if (tag == 0xFF) {
        unsigned char lenb[2];
        if (read_all(sp, lenb, 2)) return -1;
        unsigned int mlen = (unsigned int)lenb[0] | ((unsigned int)lenb[1] << 8);
        char msg[1024];
        if (mlen >= sizeof(msg)) mlen = (unsigned int)sizeof(msg) - 1;
        if (mlen && read_all(sp, msg, mlen)) return -1;
        msg[mlen] = '\0';
        fprintf(stderr, "subprocess ERROR: %s\n", msg);
        return 0xFF;
    }
    return -1;
}

/* Read a RESULT response and capture cycles + framebuffer + audio into the
 * caller's buffers. fb_buf must be at least 256*240*4 bytes; audio_buf at
 * least AUDIO_CAP*2 bytes. Returns 0 on success. */
#define FB_PIXELS (256 * 240)
#define AUDIO_CAP 4096

static int sub_recv_result(subproc_t* sp, unsigned int* out_cycles,
                           unsigned int* fb, size_t fb_cap_px,
                           short* audio, size_t audio_cap, size_t* out_audio_n) {
    unsigned char hdr[12];
    if (read_all(sp, hdr, 12)) return -1;
    if (out_cycles) *out_cycles = get_u32_le(hdr);
    unsigned int fb_len = get_u32_le(hdr + 4);   /* pixel count */
    unsigned int au_len = get_u32_le(hdr + 8);   /* int16 sample count */
    if (fb_len > fb_cap_px) {
        /* drain and bail */
        size_t fb_bytes = (size_t)fb_len * 4;
        static unsigned char sink[1 << 20];
        while (fb_bytes > 0) {
            size_t n = fb_bytes > sizeof(sink) ? sizeof(sink) : fb_bytes;
            if (read_all(sp, sink, n)) return -1;
            fb_bytes -= n;
        }
        size_t au_bytes = (size_t)au_len * 2;
        while (au_bytes > 0) {
            size_t n = au_bytes > sizeof(sink) ? sizeof(sink) : au_bytes;
            if (read_all(sp, sink, n)) return -1;
            au_bytes -= n;
        }
        fprintf(stderr, "subprocess fb_len %u exceeds cap %zu\n", fb_len, fb_cap_px);
        return -1;
    }
    /* Read framebuffer as raw bytes directly into the uint32 array. */
    if (fb_len && read_all(sp, fb, (size_t)fb_len * 4)) return -1;
    if (au_len > audio_cap) {
        fprintf(stderr, "subprocess audio_len %u exceeds cap %zu\n", au_len, audio_cap);
        return -1;
    }
    if (au_len && read_all(sp, audio, (size_t)au_len * 2)) return -1;
    if (out_audio_n) *out_audio_n = au_len;
    return 0;
}

int main(int argc, char** argv) {
    if (argc < 3) {
        fprintf(stderr, "Usage: %s <ref.dll> \"<subprocess cmdline>\" [frames]\n", argv[0]);
        return 1;
    }
    int frames = (argc >= 4) ? atoi(argv[3]) : 20;

    core_api ref;
    if (load_core(argv[1], &ref)) return 1;

    subproc_t sp;
    if (sub_spawn(&sp, argv[2])) {
        fprintf(stderr, "failed to spawn subprocess: %s\n", argv[2]);
        lib_close(ref.lib);
        return 1;
    }

    size_t rom_len = 0;
    const unsigned char* rom = nop_rom(&rom_len);

    /* INIT subprocess. */
    if (sub_send_init(&sp, rom, rom_len)) { fprintf(stderr, "sub INIT send failed\n"); sub_close(&sp); lib_close(ref.lib); return 1; }
    if (sub_recv_ok(&sp) != 0) { fprintf(stderr, "sub INIT recv failed\n"); sub_close(&sp); lib_close(ref.lib); return 1; }

    void* ref_core = ref.create(rom, rom_len);
    if (!ref_core) { fprintf(stderr, "ref create failed\n"); sub_close(&sp); lib_close(ref.lib); return 1; }
    ref.reset(ref_core);
    if (sub_send_reset(&sp)) { fprintf(stderr, "sub RESET send failed\n"); }
    if (sub_recv_ok(&sp) != 0) { fprintf(stderr, "sub RESET recv failed\n"); ref.destroy(ref_core); sub_close(&sp); lib_close(ref.lib); return 1; }

    unsigned int* ref_fb_storage = (unsigned int*)malloc(FB_PIXELS * 4);
    unsigned int* sub_fb_storage = (unsigned int*)malloc(FB_PIXELS * 4);
    short* ref_audio = (short*)malloc(AUDIO_CAP * 2);
    short* sub_audio = (short*)malloc(AUDIO_CAP * 2);
    if (!ref_fb_storage || !sub_fb_storage || !ref_audio || !sub_audio) {
        fprintf(stderr, "out of memory\n");
        free(ref_fb_storage); free(sub_fb_storage); free(ref_audio); free(sub_audio);
        ref.destroy(ref_core); sub_close(&sp); lib_close(ref.lib); return 1;
    }

    int fb_mismatches = 0, audio_mismatches = 0, cycle_mismatches = 0;
    unsigned int total_fb_diff_pixels = 0;

    for (int f = 0; f < frames; f++) {
        unsigned int ref_cycles = ref.step_frame(ref_core);
        unsigned int sub_cycles = 0;
        size_t sub_audio_n = 0;
        if (sub_send_step_frame(&sp, 1)) {
            fprintf(stderr, "frame %d: sub STEP_FRAME send failed\n", f);
            goto fail;
        }
        if (sub_recv_result(&sp, &sub_cycles, sub_fb_storage, FB_PIXELS,
                            sub_audio, AUDIO_CAP, &sub_audio_n)) {
            fprintf(stderr, "frame %d: sub RESULT recv failed\n", f);
            goto fail;
        }

        if (ref_cycles != sub_cycles) {
            if (cycle_mismatches == 0)
                fprintf(stderr, "frame %d: cycle mismatch ref=%u sub=%u\n", f, ref_cycles, sub_cycles);
            cycle_mismatches++;
        }

        const unsigned int* ref_fb = ref.framebuffer(ref_core);
        int frame_diff = 0;
        for (int i = 0; i < FB_PIXELS; i++) {
            if (ref_fb[i] != sub_fb_storage[i]) {
                frame_diff++;
                if (fb_mismatches == 0 && frame_diff <= 5) {
                    fprintf(stderr, "frame %d pixel %d: ref=%08X sub=%08X\n",
                            f, i, ref_fb[i], sub_fb_storage[i]);
                }
            }
        }
        if (frame_diff > 0) { fb_mismatches++; total_fb_diff_pixels += frame_diff; }

        size_t ref_n = ref.take_audio(ref_core, ref_audio, AUDIO_CAP);
        if (ref_n != sub_audio_n) {
            if (audio_mismatches == 0)
                fprintf(stderr, "frame %d: audio count mismatch ref=%zu sub=%zu\n", f, ref_n, sub_audio_n);
            audio_mismatches++;
        } else {
            for (size_t i = 0; i < ref_n; i++) {
                if (ref_audio[i] != sub_audio[i]) {
                    if (audio_mismatches == 0)
                        fprintf(stderr, "frame %d audio %zu: ref=%d sub=%d\n", f, i, ref_audio[i], sub_audio[i]);
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

    free(ref_fb_storage); free(sub_fb_storage); free(ref_audio); free(sub_audio);
    ref.destroy(ref_core);
    sub_close(&sp);
    lib_close(ref.lib);
    return (fb_mismatches || audio_mismatches || cycle_mismatches) ? 1 : 0;

fail:
    free(ref_fb_storage); free(sub_fb_storage); free(ref_audio); free(sub_audio);
    ref.destroy(ref_core);
    sub_close(&sp);
    lib_close(ref.lib);
    return 1;
}
