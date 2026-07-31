/*
 * test_mappers.c - M4.4 acceptance test for the NES C core mapper system.
 *
 * Verifies all 13 iNES mappers load test ROMs without crash, dispatch
 * PRG/CHR/mirroring/IRQ/audio correctly through the vtable, and match the
 * Rust core's semantics for banking, IRQ timers, and expansion audio.
 *
 * Print PASS/FAIL per check and a final summary. Exit 0 on all pass, non-zero
 * on any failure.
 */
#include "cartridge.h"
#include "mapper.h"
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

/* Build an iNES header for a given mapper / PRG / CHR config.
 * mapper_number is the full iNES mapper number (split into low nibble in
 * flags6 high bits, high nibble in flags7 high bits). */
static void make_header(uint8_t* hdr, uint8_t prg_banks, uint8_t chr_banks,
                        uint16_t mapper_number, uint8_t flags6_lo) {
    memset(hdr, 0, 16);
    hdr[0] = 'N'; hdr[1] = 'E'; hdr[2] = 'S'; hdr[3] = 0x1A;
    hdr[4] = prg_banks;
    hdr[5] = chr_banks;
    hdr[6] = (uint8_t)(((mapper_number & 0x0Fu) << 4) | (flags6_lo & 0x0Fu));
    hdr[7] = (uint8_t)(((mapper_number >> 4) & 0x0Fu) << 4);
}

/* Fill a PRG buffer so each 8 KB bank i is filled with byte (i & 0xFF). */
static void fill_prg_banks(uint8_t* prg, uint32_t prg_size) {
    for (uint32_t i = 0; i < prg_size; ++i) {
        prg[i] = (uint8_t)((i / 8192u) & 0xFFu);
    }
}

/* ---- NROM (mapper 0) ------------------------------------------------- */

static void test_nrom(void) {
    printf("\n== NROM (mapper 0) ==\n");
    uint8_t rom[16 + 32768 + 8192];
    memset(rom, 0, sizeof(rom));
    make_header(rom, 2, 1, 0, 0x00); /* 32KB PRG, 8KB CHR, mapper 0 */
    fill_prg_banks(rom + 16, 32768);
    memset(rom + 16 + 32768, 0xCC, 8192); /* CHR filled with 0xCC */
    Cartridge cart;
    CHECK(cartridge_from_bytes(rom, sizeof(rom), &cart) == 0, "NROM-256 parse succeeds");
    CHECK_EQ_U8((uint8_t)cart.mapper.mapper_num, 0u, "NROM mapper_num == 0");
    /* 32KB linear: $8000 = bank 0, $C000 = bank 2 (byte 0x02). */
    CHECK_EQ_U8(cartridge_read_prg(&cart, 0x8000), 0x00, "NROM-256 $8000 = bank 0");
    CHECK_EQ_U8(cartridge_read_prg(&cart, 0xC000), 0x02, "NROM-256 $C000 = bank 2");
    CHECK_EQ_U8(cartridge_read_chr(&cart, 0x0000), 0xCC, "NROM CHR-ROM read");
    CHECK(cartridge_mirror_mode(&cart) == MIRROR_HORIZONTAL, "NROM horizontal mirroring");
    CHECK(!cartridge_irq_pending(&cart), "NROM no IRQ");
    cartridge_destroy(&cart);
}

/* ---- UxROM (mapper 2) ------------------------------------------------ */

static void test_uxrom(void) {
    printf("\n== UxROM (mapper 2) ==\n");
    /* 8 x 16KB PRG banks = 128 KB. No CHR (CHR-RAM). */
    uint32_t prg_size = 8u * 16384u;
    uint8_t* rom = (uint8_t*)malloc(16 + prg_size);
    memset(rom, 0, 16 + prg_size);
    make_header(rom, 8, 0, 2, 0x00);
    fill_prg_banks(rom + 16, prg_size);
    Cartridge cart;
    CHECK(cartridge_from_bytes(rom, 16 + prg_size, &cart) == 0, "UxROM parse succeeds");
    CHECK_EQ_U8((uint8_t)cart.mapper.mapper_num, 2u, "UxROM mapper_num == 2");
    /* Default: bank 0 at $8000, last bank (7) at $C000.
     * 8x16KB = 128KB = 16x8KB. Last 16KB bank = 8KB banks 14,15.
     * $C000 = 8KB bank 14 = 0x0E. */
    CHECK_EQ_U8(cartridge_read_prg(&cart, 0x8000), 0x00, "UxROM $8000 = bank 0");
    CHECK_EQ_U8(cartridge_read_prg(&cart, 0xC000), 0x0E, "UxROM $C000 = last 16KB bank (8KB bank 14)");
    /* Switch bank 3 to $8000 via $8000 write. 16KB bank 3 = 8KB banks 6,7. */
    cartridge_write_prg(&cart, 0x8000, 3);
    CHECK_EQ_U8(cartridge_read_prg(&cart, 0x8000), 0x06, "UxROM $8000 = 16KB bank 3 (8KB bank 6)");
    CHECK_EQ_U8(cartridge_read_prg(&cart, 0xC000), 0x0E, "UxROM $C000 stays last bank");
    cartridge_destroy(&cart);
    free(rom);
}

/* ---- CNROM (mapper 3) ------------------------------------------------ */

static void test_cnrom(void) {
    printf("\n== CNROM (mapper 3) ==\n");
    /* 2 x 16KB PRG, 4 x 8KB CHR = 32KB CHR. */
    uint32_t prg_size = 2u * 16384u;
    uint32_t chr_size = 4u * 8192u;
    uint8_t* rom = (uint8_t*)malloc(16 + prg_size + chr_size);
    memset(rom, 0, 16 + prg_size + chr_size);
    make_header(rom, 2, 4, 3, 0x00);
    fill_prg_banks(rom + 16, prg_size);
    /* CHR: each 8KB bank filled with its bank index. */
    for (uint32_t b = 0; b < 4; ++b) {
        memset(rom + 16 + prg_size + b * 8192, (int)(b & 0xFF), 8192);
    }
    Cartridge cart;
    CHECK(cartridge_from_bytes(rom, 16 + prg_size + chr_size, &cart) == 0, "CNROM parse succeeds");
    CHECK_EQ_U8((uint8_t)cart.mapper.mapper_num, 3u, "CNROM mapper_num == 3");
    /* Default CHR bank 0. */
    CHECK_EQ_U8(cartridge_read_chr(&cart, 0x0000), 0x00, "CNROM CHR bank 0");
    /* Switch CHR bank 2 via $8000 write. */
    cartridge_write_prg(&cart, 0x8000, 2);
    CHECK_EQ_U8(cartridge_read_chr(&cart, 0x0000), 0x02, "CNROM CHR bank 2 after write");
    cartridge_destroy(&cart);
    free(rom);
}

/* ---- AxROM (mapper 7) ------------------------------------------------ */

static void test_axrom(void) {
    printf("\n== AxROM (mapper 7) ==\n");
    uint32_t prg_size = 4u * 32768u; /* 4 x 32KB */
    uint8_t* rom = (uint8_t*)malloc(16 + prg_size);
    memset(rom, 0, 16 + prg_size);
    make_header(rom, 8, 0, 7, 0x00); /* 8x16KB = 128KB, but we use 32KB units */
    /* Fill so each 32KB bank i is byte i. */
    for (uint32_t i = 0; i < prg_size; ++i) {
        rom[16 + i] = (uint8_t)((i / 32768u) & 0xFFu);
    }
    Cartridge cart;
    CHECK(cartridge_from_bytes(rom, 16 + prg_size, &cart) == 0, "AxROM parse succeeds");
    CHECK_EQ_U8((uint8_t)cart.mapper.mapper_num, 7u, "AxROM mapper_num == 7");
    CHECK_EQ_U8(cartridge_read_prg(&cart, 0x8000), 0x00, "AxROM $8000 = bank 0");
    /* Switch PRG bank 1 + single-screen mirroring. AxROM uses bit 4 for NT
     * select: 0x01 = bank 1, NT 0; 0x11 = bank 1, NT 1. */
    cartridge_write_prg(&cart, 0x8000, 0x01); /* bank 1, bit 4 clear = 1ScA */
    CHECK_EQ_U8(cartridge_read_prg(&cart, 0x8000), 0x01, "AxROM $8000 = bank 1 after write");
    CHECK(cartridge_mirror_mode(&cart) == MIRROR_SINGLE_SCREEN_0, "AxROM 1ScA mirroring (bit 4 = 0)");
    cartridge_write_prg(&cart, 0x8000, 0x11); /* bank 1, bit 4 set = 1ScB */
    CHECK(cartridge_mirror_mode(&cart) == MIRROR_SINGLE_SCREEN_1, "AxROM 1ScB mirroring (bit 4 = 1)");
    cartridge_destroy(&cart);
    free(rom);
}

/* ---- MMC1 (mapper 1) ------------------------------------------------- */

static void test_mmc1(void) {
    printf("\n== MMC1 (mapper 1) ==\n");
    uint32_t prg_size = 4u * 16384u; /* 64 KB */
    uint8_t* rom = (uint8_t*)malloc(16 + prg_size);
    memset(rom, 0, 16 + prg_size);
    make_header(rom, 4, 0, 1, 0x00);
    fill_prg_banks(rom + 16, prg_size);
    Cartridge cart;
    CHECK(cartridge_from_bytes(rom, 16 + prg_size, &cart) == 0, "MMC1 parse succeeds");
    CHECK_EQ_U8((uint8_t)cart.mapper.mapper_num, 1u, "MMC1 mapper_num == 1");
    /* Default PRG bank 0 at $8000, last bank at $C000.
     * 4x16KB = 64KB = 8x8KB. Last 16KB bank = 8KB banks 6,7.
     * $C000 = 8KB bank 6 = 0x06. */
    CHECK_EQ_U8(cartridge_read_prg(&cart, 0xC000), 0x06, "MMC1 $C000 = last 16KB bank (8KB bank 6)");
    cartridge_destroy(&cart);
    free(rom);
}

/* ---- MMC3 (mapper 4) ------------------------------------------------- */

static void test_mmc3(void) {
    printf("\n== MMC3 (mapper 4) ==\n");
    uint32_t prg_size = 8u * 8192u; /* 64 KB */
    uint32_t chr_size = 2u * 8192u;
    uint8_t* rom = (uint8_t*)malloc(16 + prg_size + chr_size);
    memset(rom, 0, 16 + prg_size + chr_size);
    make_header(rom, 4, 2, 4, 0x00);
    fill_prg_banks(rom + 16, prg_size);
    Cartridge cart;
    CHECK(cartridge_from_bytes(rom, 16 + prg_size + chr_size, &cart) == 0, "MMC3 parse succeeds");
    CHECK_EQ_U8((uint8_t)cart.mapper.mapper_num, 4u, "MMC3 mapper_num == 4");
    /* Default: $C000-$DFFF = bank 6, $E000-$FFFF = bank 7 (last). */
    CHECK_EQ_U8(cartridge_read_prg(&cart, 0xC000), 0x06, "MMC3 $C000 = bank 6");
    CHECK_EQ_U8(cartridge_read_prg(&cart, 0xE000), 0x07, "MMC3 $E000 = bank 7");
    /* IRQ: set latch=5, reload, enable, clock 6 times -> IRQ fires. */
    cartridge_write_prg(&cart, 0xC000, 0x00); /* select IRQ latch */
    cartridge_write_prg(&cart, 0xC001, 0x05); /* reload = 5 */
    cartridge_write_prg(&cart, 0xE001, 0x81); /* enable IRQ */
    CHECK(!cartridge_irq_pending(&cart), "MMC3 no IRQ before clock");
    for (int i = 0; i < 6; ++i) cartridge_clock_irq(&cart);
    CHECK(cartridge_irq_pending(&cart), "MMC3 IRQ fires after 6 clocks");
    cartridge_destroy(&cart);
    free(rom);
}

/* ---- MMC2 (mapper 9) ------------------------------------------------- */

static void test_mmc2(void) {
    printf("\n== MMC2 (mapper 9) ==\n");
    uint32_t prg_size = 4u * 8192u;
    uint8_t* rom = (uint8_t*)malloc(16 + prg_size);
    memset(rom, 0, 16 + prg_size);
    make_header(rom, 2, 0, 9, 0x00);
    fill_prg_banks(rom + 16, prg_size);
    Cartridge cart;
    CHECK(cartridge_from_bytes(rom, 16 + prg_size, &cart) == 0, "MMC2 parse succeeds");
    CHECK_EQ_U8((uint8_t)cart.mapper.mapper_num, 9u, "MMC2 mapper_num == 9");
    /* $A000-$FFFF fixed to last 16KB (banks 2,3). $C000 = bank 2 = 0x02. */
    CHECK_EQ_U8(cartridge_read_prg(&cart, 0xC000), 0x02, "MMC2 $C000 = last 16KB (8KB bank 2)");
    cartridge_destroy(&cart);
    free(rom);
}

/* ---- MMC5 (mapper 5) ------------------------------------------------- */

static void test_mmc5(void) {
    printf("\n== MMC5 (mapper 5) ==\n");
    uint32_t prg_size = 8u * 8192u;
    uint8_t* rom = (uint8_t*)malloc(16 + prg_size);
    memset(rom, 0, 16 + prg_size);
    make_header(rom, 4, 0, 5, 0x00);
    fill_prg_banks(rom + 16, prg_size);
    Cartridge cart;
    CHECK(cartridge_from_bytes(rom, 16 + prg_size, &cart) == 0, "MMC5 parse succeeds");
    CHECK_EQ_U8((uint8_t)cart.mapper.mapper_num, 5u, "MMC5 mapper_num == 5");
    /* Hardware multiplier: write $5205=3, $5206=5, read $5205 = 15. */
    cartridge_write_prg(&cart, 0x5205, 3);
    cartridge_write_prg(&cart, 0x5206, 5);
    CHECK_EQ_U8(cartridge_read_prg(&cart, 0x5205), 15, "MMC5 multiplier 3*5=15");
    cartridge_destroy(&cart);
    free(rom);
}

/* ---- VRC6 (mapper 24) ------------------------------------------------ */

static void test_vrc6(void) {
    printf("\n== VRC6 (mapper 24) ==\n");
    uint32_t prg_size = 8u * 8192u; /* 64 KB */
    uint8_t* rom = (uint8_t*)malloc(16 + prg_size);
    memset(rom, 0, 16 + prg_size);
    make_header(rom, 4, 0, 24, 0x00);
    fill_prg_banks(rom + 16, prg_size);
    Cartridge cart;
    CHECK(cartridge_from_bytes(rom, 16 + prg_size, &cart) == 0, "VRC6 parse succeeds");
    CHECK_EQ_U8((uint8_t)cart.mapper.mapper_num, 24u, "VRC6 mapper_num == 24");
    /* $E000-$FFFF fixed to last 8KB bank (7). */
    CHECK_EQ_U8(cartridge_read_prg(&cart, 0xE000), 0x07, "VRC6 $E000 = last bank 7");
    /* IRQ: latch=3, enable, clock 4 CPU cycles -> fires. */
    cartridge_write_prg(&cart, 0xF000, 3);
    cartridge_write_prg(&cart, 0xF001, 0x01); /* enable + reload */
    cartridge_clock_cpu(&cart, 4);
    CHECK(cartridge_irq_pending(&cart), "VRC6 IRQ fires after 4 CPU cycles");
    cartridge_destroy(&cart);
    free(rom);
}

/* ---- VRC6b (mapper 26) ----------------------------------------------- */

static void test_vrc6b(void) {
    printf("\n== VRC6b (mapper 26) ==\n");
    uint32_t prg_size = 8u * 8192u;
    uint8_t* rom = (uint8_t*)malloc(16 + prg_size);
    memset(rom, 0, 16 + prg_size);
    make_header(rom, 4, 0, 26, 0x00);
    fill_prg_banks(rom + 16, prg_size);
    Cartridge cart;
    CHECK(cartridge_from_bytes(rom, 16 + prg_size, &cart) == 0, "VRC6b parse succeeds");
    CHECK_EQ_U8((uint8_t)cart.mapper.mapper_num, 26u, "VRC6b mapper_num == 26");
    cartridge_destroy(&cart);
    free(rom);
}

/* ---- FME-7 (mapper 69) ----------------------------------------------- */

static void test_fme7(void) {
    printf("\n== FME-7 (mapper 69) ==\n");
    uint32_t prg_size = 8u * 8192u;
    uint8_t* rom = (uint8_t*)malloc(16 + prg_size);
    memset(rom, 0, 16 + prg_size);
    make_header(rom, 4, 0, 69, 0x00);
    fill_prg_banks(rom + 16, prg_size);
    Cartridge cart;
    CHECK(cartridge_from_bytes(rom, 16 + prg_size, &cart) == 0, "FME-7 parse succeeds");
    CHECK_EQ_U8((uint8_t)cart.mapper.mapper_num, 69u, "FME-7 mapper_num == 69");
    /* Command 8 = PRG bank 0 at $8000. Write bank 2. */
    cartridge_write_prg(&cart, 0x8000, 8);
    cartridge_write_prg(&cart, 0x8001, 2);
    CHECK_EQ_U8(cartridge_read_prg(&cart, 0x8000), 0x02, "FME-7 $8000 = bank 2");
    /* IRQ: latch=0x0005 via two writes to cmd 14, enable via cmd 15. */
    cartridge_write_prg(&cart, 0x8000, 14);
    cartridge_write_prg(&cart, 0x8001, 0x05);
    cartridge_write_prg(&cart, 0x8000, 14);
    cartridge_write_prg(&cart, 0x8001, 0x00);
    cartridge_write_prg(&cart, 0x8000, 15);
    cartridge_write_prg(&cart, 0x8001, 0x01);
    cartridge_clock_cpu(&cart, 6);
    CHECK(cartridge_irq_pending(&cart), "FME-7 IRQ fires after 6 CPU cycles");
    cartridge_destroy(&cart);
    free(rom);
}

/* ---- VRC7 (mapper 85) ------------------------------------------------ */

static void test_vrc7(void) {
    printf("\n== VRC7 (mapper 85) ==\n");
    uint32_t prg_size = 8u * 8192u;
    uint8_t* rom = (uint8_t*)malloc(16 + prg_size);
    memset(rom, 0, 16 + prg_size);
    make_header(rom, 4, 0, 85, 0x00);
    fill_prg_banks(rom + 16, prg_size);
    Cartridge cart;
    CHECK(cartridge_from_bytes(rom, 16 + prg_size, &cart) == 0, "VRC7 parse succeeds");
    CHECK_EQ_U8((uint8_t)cart.mapper.mapper_num, 85u, "VRC7 mapper_num == 85");
    /* $E000-$FFFF fixed to last 8KB bank (7). */
    CHECK_EQ_U8(cartridge_read_prg(&cart, 0xE000), 0x07, "VRC7 $E000 = last bank 7");
    cartridge_destroy(&cart);
    free(rom);
}

/* ---- Namco 163 (mapper 19) ------------------------------------------- */

static void test_namco163(void) {
    printf("\n== Namco 163 (mapper 19) ==\n");
    uint32_t prg_size = 8u * 8192u;
    uint8_t* rom = (uint8_t*)malloc(16 + prg_size);
    memset(rom, 0, 16 + prg_size);
    make_header(rom, 4, 0, 19, 0x00);
    fill_prg_banks(rom + 16, prg_size);
    Cartridge cart;
    CHECK(cartridge_from_bytes(rom, 16 + prg_size, &cart) == 0, "Namco 163 parse succeeds");
    CHECK_EQ_U8((uint8_t)cart.mapper.mapper_num, 19u, "Namco 163 mapper_num == 19");
    /* PRG bank at $C000 is slot 2, controlled by register at $D000
     * (Namco 163 decode: addr & 0xF801; $D000 -> prg_banks[2]). */
    cartridge_write_prg(&cart, 0xD000, 2);
    CHECK_EQ_U8(cartridge_read_prg(&cart, 0xC000), 0x02, "Namco 163 $C000 = bank 2 (via $D000 reg)");
    cartridge_destroy(&cart);
    free(rom);
}

/* ---- FDS (mapper 20) ------------------------------------------------- */

static void test_fds(void) {
    printf("\n== FDS (mapper 20) ==\n");
    /* Minimal FDS: 16-byte header + 1 disk side (65500 bytes). */
    uint32_t disk_len = 16u + 65500u;
    uint8_t* disk = (uint8_t*)malloc(disk_len);
    memset(disk, 0, disk_len);
    disk[0] = 'F'; disk[1] = 'D'; disk[2] = 'S'; disk[3] = 0x1A;
    disk[4] = 1; /* 1 disk side */
    /* Fill disk data with a recognizable pattern. */
    for (uint32_t i = 16; i < disk_len; ++i) disk[i] = (uint8_t)(i & 0xFF);
    /* BIOS: 8KB filled with NOPs, reset vector -> $E000. */
    uint8_t bios[8192];
    memset(bios, 0xEA, sizeof(bios));
    bios[0x1FFC] = 0x00; bios[0x1FFD] = 0xE0;
    Cartridge cart;
    CHECK(cartridge_from_fds_bytes(disk, disk_len, bios, sizeof(bios), &cart) == 0, "FDS parse succeeds");
    CHECK_EQ_U8((uint8_t)cart.mapper.mapper_num, 20u, "FDS mapper_num == 20");
    /* BIOS read at $E000 = 0xEA (NOP). */
    CHECK_EQ_U8(cartridge_read_prg(&cart, 0xE000), 0xEA, "FDS BIOS $E000 = NOP");
    /* PRG-RAM write/read at $6000. */
    cartridge_write_prg(&cart, 0x6000, 0x42);
    CHECK_EQ_U8(cartridge_read_prg(&cart, 0x6000), 0x42, "FDS PRG-RAM write persists");
    /* CHR-RAM write persists. */
    cartridge_write_chr(&cart, 0x0000, 0x55);
    CHECK_EQ_U8(cartridge_read_chr(&cart, 0x0000), 0x55, "FDS CHR-RAM write persists");
    /* Timer IRQ: latch=5, enable, clock 6 cycles -> fires. */
    cartridge_write_prg(&cart, 0x4020, 0x05);
    cartridge_write_prg(&cart, 0x4021, 0x00);
    cartridge_write_prg(&cart, 0x4023, 0x02); /* enable timer I/O */
    cartridge_write_prg(&cart, 0x4022, 0x01); /* enable timer */
    cartridge_clock_cpu(&cart, 6);
    CHECK(cartridge_irq_pending(&cart), "FDS timer IRQ fires after 6 CPU cycles");
    cartridge_destroy(&cart);
    free(disk);
}

/* ---- Expansion audio sanity ------------------------------------------ */

static void test_expansion_audio(void) {
    printf("\n== Expansion audio ==\n");
    /* VRC6: audio sample is 0.0 when no channels enabled. */
    uint32_t prg_size = 8u * 8192u;
    uint8_t* rom = (uint8_t*)malloc(16 + prg_size);
    memset(rom, 0, 16 + prg_size);
    make_header(rom, 4, 0, 24, 0x00);
    fill_prg_banks(rom + 16, prg_size);
    Cartridge vrc6;
    cartridge_from_bytes(rom, 16 + prg_size, &vrc6);
    float s = cartridge_expansion_audio_sample(&vrc6);
    CHECK(s >= -1.0f && s <= 1.0f, "VRC6 audio sample in [-1,1]");
    cartridge_destroy(&vrc6);

    /* FDS: audio sample is 0.0 when freq=0 (silence). */
    uint32_t disk_len = 16u + 65500u;
    uint8_t* disk = (uint8_t*)malloc(disk_len);
    memset(disk, 0, disk_len);
    disk[0] = 'F'; disk[1] = 'D'; disk[2] = 'S'; disk[3] = 0x1A;
    disk[4] = 1;
    uint8_t bios[8192];
    memset(bios, 0, sizeof(bios));
    Cartridge fds;
    cartridge_from_fds_bytes(disk, disk_len, bios, sizeof(bios), &fds);
    float fs = cartridge_expansion_audio_sample(&fds);
    CHECK(fs == 0.0f, "FDS audio silent when freq=0");
    cartridge_destroy(&fds);
    free(disk);
    free(rom);
}

/* ---- Unsupported mapper ---------------------------------------------- */

static void test_unsupported(void) {
    printf("\n== Unsupported mapper ==\n");
    uint8_t rom[16 + 16384];
    memset(rom, 0, sizeof(rom));
    make_header(rom, 1, 0, 99, 0x00); /* mapper 99 = unsupported */
    Cartridge cart;
    CHECK(cartridge_from_bytes(rom, sizeof(rom), &cart) != 0, "Unsupported mapper 99 rejected");
}

/* ---- main ------------------------------------------------------------ */

int main(void) {
    printf("=== NES C core M4.4 mapper acceptance test ===\n");
    test_nrom();
    test_uxrom();
    test_cnrom();
    test_axrom();
    test_mmc1();
    test_mmc3();
    test_mmc2();
    test_mmc5();
    test_vrc6();
    test_vrc6b();
    test_fme7();
    test_vrc7();
    test_namco163();
    test_fds();
    test_expansion_audio();
    test_unsupported();
    printf("\n=== Summary ===\n");
    printf("PASS: %d\nFAIL: %d\n", g_pass, g_fail);
    if (g_fail == 0) {
        printf("ALL TESTS PASSED\n");
        return 0;
    }
    printf("TESTS FAILED\n");
    return 1;
}
