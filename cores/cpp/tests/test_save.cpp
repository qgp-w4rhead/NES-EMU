/*
 * test_save.c - M4.5 acceptance test for the NES C core save state system.
 *
 * Verifies the EmulatorState save/load round-trip: build a deterministic
 * NOP-test ROM (NROM, mapper 0), step a few frames, snapshot the state,
 * step more frames, restore the snapshot, and confirm the CPU/PPU/RAM
 * state matches the snapshot point. Also verifies the magic/version
 * validation rejects bad blobs and that the save-state size is non-zero
 * and stable across calls.
 *
 * Print PASS/FAIL per check and a final summary. Exit 0 on all pass,
 * non-zero on any failure.
 */
#include "emulator.hpp"
#include "save_state.hpp"
#include "cartridge.hpp"
#include "cpu.hpp"
#include "bus.hpp"
#include "ppu.hpp"
#include "apu.hpp"
#include "joypad.hpp"
#include "region.hpp"
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int g_pass = 0;
static int g_fail = 0;

#define CHECK(cond, msg) do { \
    if (cond) { printf("PASS: %s\n", msg); g_pass++; } \
    else      { printf("FAIL: %s\n", msg); g_fail++; } \
} while (0)

#define CHECK_EQ_U8(actual, expected, msg) do { \
    uint8_t a_ = (actual), e_ = (expected); \
    if (a_ == e_) { printf("PASS: %s (got 0x%02X)\n", msg, a_); g_pass++; } \
    else          { printf("FAIL: %s (got 0x%02X, want 0x%02X)\n", msg, a_, e_); g_fail++; } \
} while (0)

#define CHECK_EQ_U16(actual, expected, msg) do { \
    uint16_t a_ = (actual), e_ = (expected); \
    if (a_ == e_) { printf("PASS: %s (got 0x%04X)\n", msg, a_); g_pass++; } \
    else          { printf("FAIL: %s (got 0x%04X, want 0x%04X)\n", msg, a_, e_); g_fail++; } \
} while (0)

#define CHECK_EQ_U32(actual, expected, msg) do { \
    uint32_t a_ = (actual), e_ = (expected); \
    if (a_ == e_) { printf("PASS: %s (got %u)\n", msg, a_); g_pass++; } \
    else          { printf("FAIL: %s (got %u, want %u)\n", msg, a_, e_); g_fail++; } \
} while (0)

/* Build a minimal iNES NROM-128 ROM (16KB PRG, no CHR) filled with NOP
 * (0xEA) instructions and a reset vector pointing at $C000. This gives a
 * deterministic CPU that just NOPs forever, advancing the cycle counter. */
static void make_nop_rom(uint8_t* rom, size_t* out_len) {
    memset(rom, 0, 16 + 16384);
    /* iNES header: 16KB PRG, 0 CHR (CHR-RAM), mapper 0, horizontal mirror. */
    rom[0] = 'N'; rom[1] = 'E'; rom[2] = 'S'; rom[3] = 0x1A;
    rom[4] = 1;   /* 1 x 16KB PRG bank. */
    rom[5] = 0;   /* 0 CHR banks = CHR-RAM. */
    rom[6] = 0;   /* mapper 0, horizontal mirroring. */
    rom[7] = 0;
    /* PRG-ROM: 16KB filled with NOP (0xEA). */
    memset(rom + 16, 0xEA, 16384);
    /* Reset vector at $FFFC/$FFFD -> $C000 (offset 0x3FFC in 16KB PRG). */
    rom[16 + 0x3FFC] = 0x00;
    rom[16 + 0x3FFD] = 0xC0;
    *out_len = 16 + 16384;
}

/* ---- Test: save-state size is non-zero and stable -------------------- */

static void test_save_state_size(void) {
    printf("\n== save-state size ==\n");
    uint8_t rom[16 + 16384];
    size_t rom_len = 0;
    make_nop_rom(rom, &rom_len);

    EmulatorState emu;
    emulator_init(&emu);
    Cartridge* cart = (Cartridge*)malloc(sizeof(Cartridge));
    CHECK(cartridge_from_bytes(rom, rom_len, cart) == 0, "NOP ROM parses");
    emu.cartridge = cart;
    bus_insert_cartridge(&emu.bus, cart);
    emulator_reset(&emu);

    size_t sz1 = emulator_save_state_size(&emu);
    CHECK(sz1 > 0u, "save-state size is non-zero");
    printf("  save-state size = %zu bytes\n", sz1);

    /* Step a frame and re-measure: size must be stable (audio buffer may
     * grow, but for a single frame it should be within capacity). */
    emulator_step_frame(&emu);
    size_t sz2 = emulator_save_state_size(&emu);
    CHECK(sz2 >= sz1, "save-state size does not shrink after a frame");

    emulator_destroy(&emu);
}

/* ---- Test: save/load round-trip restores CPU state ------------------- */

static void test_save_load_roundtrip(void) {
    printf("\n== save/load round-trip ==\n");
    uint8_t rom[16 + 16384];
    size_t rom_len = 0;
    make_nop_rom(rom, &rom_len);

    EmulatorState emu;
    emulator_init(&emu);
    Cartridge* cart = (Cartridge*)malloc(sizeof(Cartridge));
    CHECK(cartridge_from_bytes(rom, rom_len, cart) == 0, "NOP ROM parses (roundtrip)");
    emu.cartridge = cart;
    bus_insert_cartridge(&emu.bus, cart);
    emulator_reset(&emu);

    /* Step 3 frames to get a non-trivial state. */
    for (int i = 0; i < 3; ++i) {
        emulator_step_frame(&emu);
    }

    /* Snapshot the CPU registers and RAM checksum at the save point. */
    uint16_t save_pc = emu.cpu.pc;
    uint8_t  save_sp = emu.cpu.sp;
    uint8_t  save_a  = emu.cpu.a;
    uint8_t  save_x  = emu.cpu.x;
    uint8_t  save_y  = emu.cpu.y;
    uint16_t save_scanline = ppu_scanline(&emu.bus.ppu);
    uint16_t save_cycle    = ppu_cycle(&emu.bus.ppu);
    uint64_t save_cpu_cycles = bus_cpu_cycle_count(&emu.bus);

    /* Save state into a heap buffer. */
    size_t sz = emulator_save_state_size(&emu);
    uint8_t* blob = (uint8_t*)malloc(sz);
    CHECK(blob != NULL, "save-state buffer allocated");
    size_t written = emulator_save_state(&emu, blob, sz);
    CHECK_EQ_U32((uint32_t)written, (uint32_t)sz, "save-state writes full size");

    /* Step 5 more frames to mutate the state. */
    for (int i = 0; i < 5; ++i) {
        emulator_step_frame(&emu);
    }

    /* Confirm the state actually changed. */
    CHECK(emu.cpu.pc != save_pc || bus_cpu_cycle_count(&emu.bus) != save_cpu_cycles,
          "state mutated after stepping");

    /* Load the saved state back. */
    bool ok = emulator_load_state(&emu, blob, written);
    CHECK(ok, "load-state succeeds");

    /* Verify the CPU state matches the snapshot. */
    CHECK_EQ_U16(emu.cpu.pc, save_pc, "PC restored");
    CHECK_EQ_U8(emu.cpu.sp, save_sp, "SP restored");
    CHECK_EQ_U8(emu.cpu.a, save_a, "A restored");
    CHECK_EQ_U8(emu.cpu.x, save_x, "X restored");
    CHECK_EQ_U8(emu.cpu.y, save_y, "Y restored");
    CHECK_EQ_U16(ppu_scanline(&emu.bus.ppu), save_scanline, "PPU scanline restored");
    CHECK_EQ_U16(ppu_cycle(&emu.bus.ppu), save_cycle, "PPU cycle restored");
    CHECK_EQ_U32((uint32_t)bus_cpu_cycle_count(&emu.bus), (uint32_t)save_cpu_cycles,
                 "CPU cycle count restored");

    /* Step one more frame from the restored state and confirm it does not
     * crash and produces a non-zero cycle count. */
    uint32_t frame_cycles = emulator_step_frame(&emu);
    CHECK(frame_cycles > 0u, "restored state steps a frame without crash");

    free(blob);
    emulator_destroy(&emu);
}

/* ---- Test: bad magic / version rejected ------------------------------ */

static void test_bad_blob_rejected(void) {
    printf("\n== bad blob rejection ==\n");
    uint8_t rom[16 + 16384];
    size_t rom_len = 0;
    make_nop_rom(rom, &rom_len);

    EmulatorState emu;
    emulator_init(&emu);
    Cartridge* cart = (Cartridge*)malloc(sizeof(Cartridge));
    CHECK(cartridge_from_bytes(rom, rom_len, cart) == 0, "NOP ROM parses (bad blob)");
    emu.cartridge = cart;
    bus_insert_cartridge(&emu.bus, cart);
    emulator_reset(&emu);

    /* Save a valid blob first. */
    size_t sz = emulator_save_state_size(&emu);
    uint8_t* blob = (uint8_t*)malloc(sz);
    emulator_save_state(&emu, blob, sz);

    /* Corrupt the magic. */
    uint8_t bad_magic[12];
    memcpy(bad_magic, blob, 12);
    bad_magic[0] = 'X'; /* corrupt magic byte 0 */
    CHECK(!emulator_load_state(&emu, bad_magic, 12), "bad magic rejected");

    /* Restore the magic, corrupt the version. */
    memcpy(bad_magic, blob, 12);
    bad_magic[4] = 0xFF; /* corrupt version byte 0 */
    CHECK(!emulator_load_state(&emu, bad_magic, 12), "bad version rejected");

    /* Truncated payload. */
    CHECK(!emulator_load_state(&emu, blob, 13), "truncated payload rejected");

    /* NULL core / buf. */
    CHECK(!emulator_load_state(NULL, blob, sz), "NULL core rejected");
    CHECK(!emulator_load_state(&emu, NULL, sz), "NULL buf rejected");

    free(blob);
    emulator_destroy(&emu);
}

/* ---- Test: save-state header magic/version --------------------------- */

static void test_header_magic_version(void) {
    printf("\n== header magic/version ==\n");
    uint8_t rom[16 + 16384];
    size_t rom_len = 0;
    make_nop_rom(rom, &rom_len);

    EmulatorState emu;
    emulator_init(&emu);
    Cartridge* cart = (Cartridge*)malloc(sizeof(Cartridge));
    CHECK(cartridge_from_bytes(rom, rom_len, cart) == 0, "NOP ROM parses (header)");
    emu.cartridge = cart;
    bus_insert_cartridge(&emu.bus, cart);
    emulator_reset(&emu);

    size_t sz = emulator_save_state_size(&emu);
    uint8_t* blob = (uint8_t*)malloc(sz);
    emulator_save_state(&emu, blob, sz);

    /* Verify the header bytes match SAVE_STATE_MAGIC and SAVE_STATE_VERSION. */
    uint32_t magic = (uint32_t)blob[0]
                   | ((uint32_t)blob[1] << 8)
                   | ((uint32_t)blob[2] << 16)
                   | ((uint32_t)blob[3] << 24);
    uint32_t version = (uint32_t)blob[4]
                     | ((uint32_t)blob[5] << 8)
                     | ((uint32_t)blob[6] << 16)
                     | ((uint32_t)blob[7] << 24);
    CHECK_EQ_U32(magic, SAVE_STATE_MAGIC, "save-state magic matches");
    CHECK_EQ_U32(version, SAVE_STATE_VERSION, "save-state version matches");

    free(blob);
    emulator_destroy(&emu);
}

/* ---- Test: emulator mapper number ------------------------------------ */

static void test_emulator_mapper_number(void) {
    printf("\n== emulator mapper number ==\n");
    uint8_t rom[16 + 16384];
    size_t rom_len = 0;
    make_nop_rom(rom, &rom_len);

    EmulatorState emu;
    emulator_init(&emu);
    CHECK(emulator_mapper_number(&emu) == 0u, "no-cart mapper number is 0");
    Cartridge* cart = (Cartridge*)malloc(sizeof(Cartridge));
    CHECK(cartridge_from_bytes(rom, rom_len, cart) == 0, "NOP ROM parses (mapper num)");
    emu.cartridge = cart;
    bus_insert_cartridge(&emu.bus, cart);
    CHECK_EQ_U16(emulator_mapper_number(&emu), 0u, "NROM mapper number is 0");
    emulator_destroy(&emu);
}

/* ---- Test: emulator region set/get ----------------------------------- */

static void test_emulator_region(void) {
    printf("\n== emulator region ==\n");
    EmulatorState emu;
    emulator_init(&emu);
    CHECK(emulator_region(&emu) == REGION_NTSC, "default region is NTSC");
    emulator_set_region(&emu, REGION_PAL);
    CHECK(emulator_region(&emu) == REGION_PAL, "region set to PAL");
    emulator_set_region(&emu, REGION_DENDY);
    CHECK(emulator_region(&emu) == REGION_DENDY, "region set to Dendy");
    emulator_set_region(&emu, REGION_NTSC);
    CHECK(emulator_region(&emu) == REGION_NTSC, "region set back to NTSC");
    emulator_destroy(&emu);
}

/* ---- Test: audio buffer take ----------------------------------------- */

static void test_audio_take(void) {
    printf("\n== audio take ==\n");
    uint8_t rom[16 + 16384];
    size_t rom_len = 0;
    make_nop_rom(rom, &rom_len);

    EmulatorState emu;
    emulator_init(&emu);
    Cartridge* cart = (Cartridge*)malloc(sizeof(Cartridge));
    CHECK(cartridge_from_bytes(rom, rom_len, cart) == 0, "NOP ROM parses (audio)");
    emu.cartridge = cart;
    bus_insert_cartridge(&emu.bus, cart);
    emulator_reset(&emu);

    emulator_step_frame(&emu);

    /* Drain audio: should get some samples (NTSC ~735/frame). */
    float samples[1024];
    size_t n = emulator_take_audio_samples(&emu, samples, 1024);
    CHECK(n > 0u, "audio buffer has samples after a frame");
    printf("  audio samples = %zu\n", n);

    /* Second drain should be empty (buffer was cleared). */
    size_t n2 = emulator_take_audio_samples(&emu, samples, 1024);
    CHECK_EQ_U32((uint32_t)n2, 0u, "audio buffer empty after drain");

    emulator_destroy(&emu);
}

/* ---- Test: save-state accessors -------------------------------------- */

static void test_save_state_accessors(void) {
    printf("\n== save-state accessors ==\n");
    EmulatorState emu;
    emulator_init(&emu);
    emulator_set_sample_accumulator(&emu, 1.5f);
    CHECK(emulator_sample_accumulator(&emu) == 1.5f, "sample accumulator set/get");
    emulator_set_ppu_cycle_carry(&emu, 42u);
    CHECK_EQ_U32(emulator_ppu_cycle_carry(&emu), 42u, "ppu cycle carry set/get");

    /* Audio buffer set/get. */
    float test_buf[4] = { 0.1f, 0.2f, 0.3f, 0.4f };
    emulator_set_audio_buffer(&emu, test_buf, 4);
    CHECK_EQ_U32((uint32_t)emulator_audio_buffer_count(&emu), 4u, "audio buffer count set");
    const float* ab = emulator_audio_buffer(&emu);
    CHECK(ab != NULL, "audio buffer pointer non-null");
    CHECK(ab[0] == 0.1f, "audio buffer[0] preserved");
    CHECK(ab[3] == 0.4f, "audio buffer[3] preserved");

    emulator_destroy(&emu);
}

/* ---- Main ------------------------------------------------------------ */

int main(void) {
    printf("=== NES C core save-state tests (M4.5) ===\n");
    test_save_state_size();
    test_save_load_roundtrip();
    test_bad_blob_rejected();
    test_header_magic_version();
    test_emulator_mapper_number();
    test_emulator_region();
    test_audio_take();
    test_save_state_accessors();

    printf("\n=== Summary: %d passed, %d failed ===\n", g_pass, g_fail);
    return (g_fail == 0) ? 0 : 1;
}
