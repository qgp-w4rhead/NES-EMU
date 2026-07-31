# cython: language_level=3, boundscheck=False, wraparound=False, initializedcheck=False, cdivision=True
#
# nes_core/_core.pyx -- Cython NES emulator core (M11).
#
# Complete port of the C reference core (cores/c/src/*.c) to Cython 3.x,
# compiled to a native .pyd extension. The whole emulator lives in this one
# .pyx so all cdef types can reference each other at C level (no Python object
# traffic in the hot path). Behaviour is byte-exact with the C core; the
# subprocess protocol loop (M3) lives in protocol.py and drives this module.
#
# Source of truth: cores/c/src/*.c + cores/c/include/*.h. The TypeScript port
# (cores/typescript/src/*.ts) provided the structural template for the
# subprocess protocol and save-state layout.

from libc.stdint cimport uint8_t, uint16_t, uint32_t, uint64_t, int8_t, int16_t
from libc.stdlib cimport malloc, calloc, realloc, free
from libc.string cimport memset, memcpy, memcmp
cimport cython

import struct as _struct


# ============================================================================
# Enums
# ============================================================================

cdef enum Region:
    REGION_NTSC = 0
    REGION_PAL = 1
    REGION_DENDY = 2

cdef enum Mirroring:
    MIRROR_HORIZONTAL = 0
    MIRROR_VERTICAL = 1
    MIRROR_FOUR_SCREEN = 2
    MIRROR_SINGLE_SCREEN_0 = 3
    MIRROR_SINGLE_SCREEN_1 = 4
    MIRROR_SINGLE_SCREEN_2 = 5
    MIRROR_SINGLE_SCREEN_3 = 6

cdef enum AddrMode:
    ADDR_MODE_IMPLIED = 0
    ADDR_MODE_ACCUMULATOR = 1
    ADDR_MODE_IMMEDIATE = 2
    ADDR_MODE_ZERO_PAGE = 3
    ADDR_MODE_ZERO_PAGE_X = 4
    ADDR_MODE_ZERO_PAGE_Y = 5
    ADDR_MODE_ABSOLUTE = 6
    ADDR_MODE_ABSOLUTE_X = 7
    ADDR_MODE_ABSOLUTE_Y = 8
    ADDR_MODE_INDIRECT = 9
    ADDR_MODE_INDIRECT_X = 10
    ADDR_MODE_INDIRECT_Y = 11
    ADDR_MODE_RELATIVE = 12

cdef enum Dummy:
    DUMMY_NONE = 0
    DUMMY_READ = 1
    DUMMY_RMW = 2


# ============================================================================
# Constants
# ============================================================================

# ---- CPU flags ----
cdef uint8_t CPU_FLAG_C = 0x01
cdef uint8_t CPU_FLAG_Z = 0x02
cdef uint8_t CPU_FLAG_I = 0x04
cdef uint8_t CPU_FLAG_D = 0x08
cdef uint8_t CPU_FLAG_B = 0x10
cdef uint8_t CPU_FLAG_U = 0x20
cdef uint8_t CPU_FLAG_V = 0x40
cdef uint8_t CPU_FLAG_N = 0x80

cdef uint16_t CPU_VECTOR_NMI = 0xFFFA
cdef uint16_t CPU_VECTOR_RESET = 0xFFFC
cdef uint16_t CPU_VECTOR_IRQ = 0xFFFE

cdef uint8_t CPU_NMI_PENDING = 0x01
cdef uint8_t CPU_IRQ_PENDING = 0x02
cdef uint8_t CPU_HALTED = 0x04

# Operand tag
cdef int OPERAND_NONE = 0
cdef int OPERAND_ACC = 1
cdef int OPERAND_ADDR = 2

# ---- Bus ----
cdef uint16_t BUS_RAM_SIZE = 0x0800
cdef uint16_t BUS_RAM_MASK = 0x07FF
cdef uint16_t BUS_PPU_REG_MASK = 0x0007
cdef uint16_t BUS_PPU_REG_BASE = 0x2000
cdef uint16_t BUS_APU_IO_BASE = 0x4000
cdef uint16_t BUS_APU_IO_REG_COUNT = 0x18

# ---- PPU ----
cdef uint16_t PPU_VRAM_SIZE_4K = 0x1000
cdef uint16_t PPU_VRAM_SIZE_2K = 0x0800
cdef uint16_t PPU_OAM_SIZE = 256
cdef uint16_t PPU_PALETTE_SIZE = 32
cdef uint16_t PPU_SCANLINES_PER_FRAME = 262
cdef uint16_t PPU_CYCLES_PER_SCANLINE = 341
cdef uint16_t PPU_SCANLINE_VBLANK_START = 241
cdef uint16_t PPU_SCANLINE_PRERENDER = 261
cdef uint16_t PPU_SCREEN_WIDTH = 256
cdef uint16_t PPU_SCREEN_HEIGHT = 240
cdef uint32_t PPU_FRAMEBUFFER_SIZE = 256 * 240

cdef uint8_t PPU_CTRL_NMI = 0x80
cdef uint8_t PPU_CTRL_INCREMENT_32 = 0x04
cdef uint8_t PPU_CTRL_SPRITE_PATTERN_1000 = 0x08
cdef uint8_t PPU_CTRL_BG_PATTERN_1000 = 0x10
cdef uint8_t PPU_CTRL_SPRITE_SIZE_16 = 0x20
cdef uint8_t PPU_CTRL_BASE_NT_MASK = 0x03

cdef uint8_t PPU_MASK_SHOW_BG_LEFT = 0x02
cdef uint8_t PPU_MASK_SHOW_SPRITES_LEFT = 0x04
cdef uint8_t PPU_MASK_SHOW_BG = 0x08
cdef uint8_t PPU_MASK_SHOW_SPRITES = 0x10

cdef uint8_t PPU_STATUS_VBLANK = 0x80
cdef uint8_t PPU_STATUS_SPRITE_ZERO = 0x40
cdef uint8_t PPU_STATUS_OVERFLOW = 0x20
cdef uint8_t PPU_STATUS_FLAG_MASK = 0xE0
cdef uint8_t PPU_STATUS_OPEN_BUS_MASK = 0x1F

cdef uint16_t PPU_NT_SELECT_MASK = 0x0C00
cdef uint16_t PPU_COARSE_X_MASK = 0x001F
cdef uint16_t PPU_COARSE_Y_MASK = 0x03E0
cdef uint16_t PPU_FINE_Y_MASK = 0x7000
cdef uint16_t PPU_NT_H_BIT = 0x0400
cdef uint16_t PPU_NT_V_BIT = 0x0800

# PPU timing
cdef uint16_t VBLANK_NMI_CYCLE = 1
cdef uint16_t VERT_SCROLL_INC_CYCLE = 256
cdef uint16_t H_COPY_CYCLE = 257
cdef uint16_t V_COPY_CYCLE_START = 280
cdef uint16_t V_COPY_CYCLE_END = 304
cdef uint16_t H_SCROLL_INC_STEP = 8
cdef uint16_t H_SCROLL_INC_LAST = 248

# Render constants
cdef uint16_t NT_COLS = 32
cdef uint16_t NT_ROWS = 30
cdef uint16_t ATTR_TABLE_OFFSET = 0x03C0
cdef uint16_t NT_BASE = 0x2000
cdef uint16_t PAL_BASE = 0x3F00
cdef uint16_t SPRITE_PAL_BASE = 0x3F10
cdef uint8_t ATTR_PALETTE_MASK = 0x03
cdef uint8_t ATTR_PRIORITY_BEHIND = 0x20
cdef uint8_t ATTR_HFLIP = 0x40
cdef uint8_t ATTR_VFLIP = 0x80
cdef uint16_t SPRITE_COUNT = 64
cdef uint16_t SPRITE_HEIGHT_8X8 = 8
cdef uint16_t SPRITE_HEIGHT_8X16 = 16
cdef uint16_t SPRITE_WIDTH = 8
cdef uint8_t OAM_Y_HIDDEN = 0xEF
cdef uint8_t OAM_Y_HALT = 0xFF
cdef uint16_t SPRITE_ZERO_HIT_MAX_X = 255
cdef uint8_t ALPHA = 0xFF
cdef uint8_t PPU_MAX_SPRITES_PER_SCANLINE = 8

# ---- APU ----
cdef uint16_t APU_MAX_PERIOD = 0x7FF
cdef uint16_t APU_MIN_AUDIBLE_PERIOD = 8
cdef uint32_t APU_SAMPLE_RATE = 44100
cdef uint8_t APU_CHANNEL_COUNT = 5

# ---- Cartridge ----
cdef uint16_t CART_HEADER_SIZE = 16
cdef uint16_t CART_TRAINER_SIZE = 512
cdef uint32_t CART_PRG_ROM_UNIT = 16384
cdef uint32_t CART_CHR_ROM_UNIT = 8192

# ---- Emulator ----
cdef uint32_t EMU_AUDIO_CAP_NTSC = 800
cdef uint32_t EMU_AUDIO_CAP_PAL = 950

# ---- Bus MMC3 IRQ clock cycle ----
cdef uint16_t MMC3_IRQ_CLOCK_CYCLE = 260


# ============================================================================
# APU lookup tables
# ============================================================================

cdef uint8_t[8] APU_DUTY_PATTERNS_0 = [0, 1, 0, 0, 0, 0, 0, 0]
cdef uint8_t[8] APU_DUTY_PATTERNS_1 = [0, 1, 1, 0, 0, 0, 0, 0]
cdef uint8_t[8] APU_DUTY_PATTERNS_2 = [0, 1, 1, 1, 1, 0, 0, 0]
cdef uint8_t[8] APU_DUTY_PATTERNS_3 = [1, 0, 0, 0, 1, 1, 1, 1]

cdef uint8_t[32] APU_LENGTH_TABLE = [
    10, 254, 20, 2, 40, 4, 80, 6, 160, 8, 60, 10, 14, 12, 26, 14,
    12, 16, 24, 18, 48, 20, 96, 22, 192, 24, 72, 26, 16, 28, 32, 30,
]

cdef uint8_t[32] APU_TRIANGLE_SEQUENCE = [
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0,
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
]

cdef uint16_t[16] APU_NOISE_PERIOD_TABLE = [
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
]

cdef uint16_t[16] APU_DMC_RATE_TABLE = [
    214, 190, 170, 160, 149, 138, 127, 113, 107, 95, 85, 80, 71, 63, 54, 42,
]

# APU frame-counter thresholds (NTSC 4-step / 5-step / PAL).
cdef uint32_t[4] NTSC_4STEP = [7457, 14913, 22371, 29828]
cdef uint32_t[4] NTSC_5STEP = [7457, 14913, 22371, 37281]
cdef uint32_t[4] PAL_4STEP = [8314, 16627, 24941, 33255]
cdef uint32_t[4] PAL_5STEP = [8314, 16627, 24941, 41568]
cdef uint8_t[4] FRAME_QUARTER = [1, 1, 1, 1]
cdef uint8_t[4] FRAME_HALF = [0, 1, 0, 1]


# ============================================================================
# PPU palettes (render.rs NES_PALETTE / NES_PALETTE_INACCURATE / PAL_PALETTE)
# ============================================================================

# NTSC 2C02 reference palette - 64 entries x RGB.
cdef uint8_t[64][3] NES_PALETTE = [
    [0x7C, 0x7C, 0x7C], [0x00, 0x00, 0xFC], [0x00, 0x00, 0xBC], [0x44, 0x28, 0xBC],
    [0x94, 0x00, 0x84], [0xA8, 0x00, 0x20], [0xA8, 0x10, 0x00], [0x88, 0x14, 0x00],
    [0x50, 0x30, 0x00], [0x00, 0x78, 0x00], [0x00, 0x68, 0x00], [0x00, 0x58, 0x00],
    [0x00, 0x40, 0x58], [0x00, 0x00, 0x00], [0x00, 0x00, 0x00], [0x00, 0x00, 0x00],
    [0xBC, 0xBC, 0xBC], [0x00, 0x78, 0xF8], [0x00, 0x58, 0xF8], [0x68, 0x44, 0xFC],
    [0xD8, 0x00, 0xCC], [0xE4, 0x00, 0x58], [0xF8, 0x38, 0x00], [0xE4, 0x5C, 0x10],
    [0xAC, 0x7C, 0x00], [0x00, 0xB8, 0x00], [0x00, 0xA8, 0x00], [0x00, 0xA8, 0x44],
    [0x00, 0x88, 0x88], [0x00, 0x00, 0x00], [0x00, 0x00, 0x00], [0x00, 0x00, 0x00],
    [0xF8, 0xF8, 0xF8], [0x3C, 0xBC, 0xFC], [0x68, 0x88, 0xFC], [0x98, 0x78, 0xF8],
    [0xF8, 0x78, 0xF8], [0xF8, 0x58, 0x98], [0xF8, 0x78, 0x58], [0xFC, 0xA0, 0x44],
    [0xF8, 0xB8, 0x00], [0xB8, 0xF8, 0x18], [0x58, 0xD8, 0x54], [0x58, 0xF8, 0x98],
    [0x00, 0xE8, 0xD8], [0x78, 0x78, 0x78], [0x00, 0x00, 0x00], [0x00, 0x00, 0x00],
    [0xFC, 0xFC, 0xFC], [0xA4, 0xE4, 0xFC], [0xB8, 0xB8, 0xF8], [0xD8, 0xB8, 0xF8],
    [0xF8, 0xB8, 0xF8], [0xF8, 0xA4, 0xC0], [0xF0, 0xD0, 0xB0], [0xFC, 0xE0, 0xA8],
    [0xF8, 0xD8, 0x78], [0xD8, 0xF8, 0x78], [0xB8, 0xF8, 0xB8], [0xB8, 0xF8, 0xD8],
    [0x00, 0xFC, 0xFC], [0xF8, 0xD8, 0xF8], [0x00, 0x00, 0x00], [0x00, 0x00, 0x00],
]

cdef uint8_t[64][3] NES_PALETTE_INACCURATE = [
    [0x84, 0x84, 0x84], [0x00, 0x1D, 0x2C], [0x1C, 0x0C, 0x54], [0x30, 0x04, 0x64],
    [0x48, 0x00, 0x5C], [0x58, 0x00, 0x44], [0x58, 0x00, 0x24], [0x4C, 0x0C, 0x00],
    [0x38, 0x18, 0x00], [0x20, 0x28, 0x00], [0x0C, 0x3C, 0x00], [0x00, 0x40, 0x00],
    [0x00, 0x3C, 0x1C], [0x00, 0x38, 0x3C], [0x00, 0x00, 0x00], [0x00, 0x00, 0x00],
    [0xB4, 0xB4, 0xB4], [0x38, 0x6C, 0xBC], [0x54, 0x58, 0xEC], [0x70, 0x44, 0xF4],
    [0x90, 0x38, 0xE4], [0xA8, 0x34, 0xC8], [0xB8, 0x34, 0x88], [0xC0, 0x34, 0x44],
    [0xC4, 0x40, 0x14], [0xC8, 0x50, 0x00], [0xA8, 0x60, 0x00], [0x88, 0x70, 0x00],
    [0x6C, 0x80, 0x00], [0x44, 0x88, 0x00], [0x00, 0x94, 0x00], [0x00, 0x00, 0x00],
    [0xFF, 0xFF, 0xFF], [0x9C, 0xDC, 0xFF], [0xB8, 0xB8, 0xFF], [0xD0, 0xB8, 0xFF],
    [0xFF, 0xB0, 0xF4], [0xFF, 0xA8, 0xE0], [0xFF, 0xA4, 0xC0], [0xFF, 0xA0, 0x90],
    [0xF8, 0x94, 0x58], [0xF0, 0xA0, 0x38], [0xD8, 0xA8, 0x20], [0xB8, 0xB0, 0x14],
    [0x98, 0xB8, 0x14], [0x70, 0xC0, 0x14], [0x50, 0xC8, 0x24], [0x00, 0x00, 0x00],
    [0xFF, 0xFF, 0xFF], [0x9C, 0xDC, 0xFF], [0xB8, 0xB8, 0xFF], [0xD0, 0xB8, 0xFF],
    [0xFF, 0xB0, 0xF4], [0xFF, 0xA8, 0xE0], [0xFF, 0xA4, 0xC0], [0xFF, 0xA0, 0x90],
    [0xF8, 0x94, 0x58], [0xF0, 0xA0, 0x38], [0xD8, 0xA8, 0x20], [0xB8, 0xB0, 0x14],
    [0x98, 0xB8, 0x14], [0x70, 0xC0, 0x14], [0x50, 0xC8, 0x24], [0x00, 0x00, 0x00],
]

cdef uint8_t[64][3] PAL_PALETTE = [
    [0x84, 0x84, 0x84], [0x00, 0x1D, 0x2C], [0x0C, 0x0C, 0x44], [0x24, 0x04, 0x54],
    [0x3C, 0x00, 0x4C], [0x4C, 0x00, 0x34], [0x4C, 0x00, 0x18], [0x40, 0x0C, 0x00],
    [0x2C, 0x18, 0x00], [0x18, 0x28, 0x00], [0x08, 0x3C, 0x00], [0x00, 0x40, 0x00],
    [0x00, 0x3C, 0x1C], [0x00, 0x38, 0x3C], [0x04, 0x04, 0x04], [0x00, 0x00, 0x00],
    [0xB4, 0xB4, 0xB4], [0x30, 0x60, 0xA4], [0x48, 0x48, 0xC8], [0x60, 0x38, 0xD8],
    [0x80, 0x30, 0xC8], [0x98, 0x2C, 0xAC], [0xA8, 0x2C, 0x70], [0xB0, 0x2C, 0x38],
    [0xB4, 0x38, 0x10], [0xB8, 0x48, 0x00], [0x98, 0x58, 0x00], [0x78, 0x68, 0x00],
    [0x5C, 0x78, 0x00], [0x38, 0x80, 0x00], [0x00, 0x8C, 0x00], [0x04, 0x04, 0x04],
    [0xFF, 0xFF, 0xFF], [0x8C, 0xCC, 0xFF], [0xA8, 0xA8, 0xFF], [0xC0, 0xA8, 0xFF],
    [0xE8, 0xA4, 0xF0], [0xE8, 0xA0, 0xD8], [0xE8, 0x9C, 0xB8], [0xE8, 0x98, 0x88],
    [0xE0, 0x8C, 0x54], [0xD8, 0x98, 0x38], [0xC0, 0xA0, 0x24], [0xA0, 0xA8, 0x18],
    [0x84, 0xB0, 0x18], [0x60, 0xB8, 0x18], [0x44, 0xC0, 0x2C], [0x04, 0x04, 0x04],
    [0xFF, 0xFF, 0xFF], [0x8C, 0xCC, 0xFF], [0xA8, 0xA8, 0xFF], [0xC0, 0xA8, 0xFF],
    [0xE8, 0xA4, 0xF0], [0xE8, 0xA0, 0xD8], [0xE8, 0x9C, 0xB8], [0xE8, 0x98, 0x88],
    [0xE0, 0x8C, 0x54], [0xD8, 0x98, 0x38], [0xC0, 0xA0, 0x24], [0xA0, 0xA8, 0x18],
    [0x84, 0xB0, 0x18], [0x60, 0xB8, 0x18], [0x44, 0xC0, 0x2C], [0x04, 0x04, 0x04],
]


# ============================================================================
# Region helpers (region.c)
# ============================================================================

cdef inline uint16_t region_scanlines_per_frame(Region r) nogil:
    if r == REGION_PAL or r == REGION_DENDY:
        return 312
    return 262

cdef inline uint16_t region_scanline_prerender(Region r) nogil:
    if r == REGION_PAL or r == REGION_DENDY:
        return 311
    return 261

cdef inline bint region_is_pal_palette(Region r) nogil:
    return r == REGION_PAL

cdef inline float region_cpu_clock_hz(Region r) nogil:
    if r == REGION_PAL:
        return 1662607.0
    return 1789773.0

cdef inline float region_cpu_cycles_per_sample(Region r) nogil:
    return region_cpu_clock_hz(r) / 44100.0

cdef inline uint32_t region_apu_reset_at(Region r, bint mode_5step) nogil:
    if r == REGION_PAL:
        return 41570 if mode_5step else 33257
    return 37282 if mode_5step else 29830

cdef inline uint32_t region_apu_4step_irq_threshold(Region r) nogil:
    if r == REGION_PAL:
        return 33255
    return 29828


# ============================================================================
# Forward declarations of cdef classes (so they can reference each other)
# ============================================================================

cdef class Bus
cdef class Cartridge


# ============================================================================
# Cartridge / Mapper (cartridge.c + nrom.c + mappers.c)
# ============================================================================
#
# The C core uses a vtable + opaque state per mapper. In Cython we use a single
# Cartridge cdef class with polymorphic dispatch on mapper_number. NROM
# (mapper 0) is the path exercised by the fb_compare_sub NOP-ROM validation;
# the other mappers are faithful ports of the C core's per-mapper state
# machines so real ROMs also work.

cdef class Cartridge:
    # parsed iNES header
    cdef uint8_t prg_rom_banks
    cdef uint8_t chr_rom_banks
    cdef uint16_t mapper_number
    cdef Mirroring mirroring
    cdef bint has_trainer
    cdef bint has_battery
    cdef uint8_t tv_system

    # PRG / CHR storage (owned). For CHR-RAM, chr_is_ram=True and chr writable.
    cdef uint8_t *prg_rom
    cdef uint32_t prg_size
    cdef uint8_t *chr
    cdef uint32_t chr_size
    cdef bint chr_is_ram

    # ---- Mapper-specific state ----
    cdef uint8_t prg_bank        # UxROM / AxROM
    cdef uint8_t chr_bank        # CNROM
    cdef uint8_t mmc1_shift_reg
    cdef uint8_t mmc1_shift_count
    cdef uint8_t mmc1_control
    cdef uint8_t mmc1_chr_bank_0
    cdef uint8_t mmc1_chr_bank_1
    cdef uint8_t mmc1_prg_bank
    cdef uint8_t mmc3_bank_select
    cdef uint8_t mmc3_registers[8]
    cdef uint8_t mmc3_irq_reload
    cdef uint8_t mmc3_irq_counter
    cdef bint mmc3_irq_enable
    cdef bint mmc3_irq_pending_flag
    cdef uint8_t mmc3_chr_mode_8k
    cdef uint8_t mmc5_prg_bank[4]
    cdef uint8_t mmc5_chr_bank[8]
    cdef bint mmc5_irq_pending_flag
    cdef bint mmc2_latch_fd
    cdef bint mmc2_latch_fe
    cdef uint8_t mmc2_prg_banks[4]
    cdef uint8_t mmc2_chr_banks[8]
    cdef bint vrc6_is_26
    cdef uint8_t vrc6_prg_bank[2]
    cdef uint8_t vrc6_chr_bank[8]
    cdef bint vrc6_mirroring
    cdef uint16_t vrc6_irq_count
    cdef uint8_t vrc6_irq_enable
    cdef bint vrc6_irq_pending_flag
    cdef uint16_t vrc6_irq_prescale
    cdef uint8_t fme7_command
    cdef uint8_t fme7_registers[16]
    cdef uint16_t fme7_irq_count
    cdef uint8_t fme7_irq_enable
    cdef bint fme7_irq_pending_flag
    cdef uint8_t vrc7_prg_bank[3]
    cdef uint8_t vrc7_chr_bank[8]
    cdef uint8_t n163_prg_bank[3]
    cdef uint8_t n163_chr_bank[8]
    cdef uint8_t n163_addr
    cdef uint8_t n163_increment
    cdef uint8_t n163_ram[128]

    def __cinit__(self):
        self.prg_rom = NULL
        self.prg_size = 0
        self.chr = NULL
        self.chr_size = 0
        self.chr_is_ram = False
        self.mapper_number = 0
        self.mirroring = MIRROR_HORIZONTAL
        self.has_trainer = False
        self.has_battery = False
        self.tv_system = 0
        self.prg_rom_banks = 0
        self.chr_rom_banks = 0
        self.prg_bank = 0
        self.chr_bank = 0
        self.mmc1_shift_reg = 0
        self.mmc1_shift_count = 0
        self.mmc1_control = 0x0C
        self.mmc1_chr_bank_0 = 0
        self.mmc1_chr_bank_1 = 0
        self.mmc1_prg_bank = 0
        self.mmc3_bank_select = 0
        cdef int i
        for i in range(8):
            self.mmc3_registers[i] = 0
        self.mmc3_irq_reload = 0
        self.mmc3_irq_counter = 0
        self.mmc3_irq_enable = False
        self.mmc3_irq_pending_flag = False
        self.mmc3_chr_mode_8k = 0
        for i in range(4):
            self.mmc5_prg_bank[i] = i
            self.mmc2_prg_banks[i] = 0
        for i in range(8):
            self.mmc5_chr_bank[i] = 0
            self.mmc2_chr_banks[i] = 0
        self.mmc5_irq_pending_flag = False
        self.mmc2_latch_fd = True
        self.mmc2_latch_fe = True
        self.vrc6_is_26 = False
        for i in range(2):
            self.vrc6_prg_bank[i] = 0
        for i in range(8):
            self.vrc6_chr_bank[i] = 0
        self.vrc6_mirroring = False
        self.vrc6_irq_count = 0
        self.vrc6_irq_enable = 0
        self.vrc6_irq_pending_flag = False
        self.vrc6_irq_prescale = 0
        self.fme7_command = 0
        for i in range(16):
            self.fme7_registers[i] = 0
        self.fme7_irq_count = 0
        self.fme7_irq_enable = 0
        self.fme7_irq_pending_flag = False
        for i in range(3):
            self.vrc7_prg_bank[i] = 0
            self.n163_prg_bank[i] = 0
        for i in range(8):
            self.vrc7_chr_bank[i] = 0
            self.n163_chr_bank[i] = 0
        self.n163_addr = 0
        self.n163_increment = 0
        for i in range(128):
            self.n163_ram[i] = 0

    def __dealloc__(self):
        if self.prg_rom != NULL:
            free(self.prg_rom)
            self.prg_rom = NULL
        if self.chr != NULL:
            free(self.chr)
            self.chr = NULL

    # ---- iNES parse + mapper init ----
    @staticmethod
    cdef Cartridge from_bytes(const uint8_t *data, Py_ssize_t length):
        cdef Cartridge cart = Cartridge()
        if length < CART_HEADER_SIZE:
            return None
        if data[0] != 0x4E or data[1] != 0x45 or data[2] != 0x53 or data[3] != 0x1A:
            return None
        cdef uint8_t prg_rom_banks = data[4]
        cdef uint8_t chr_rom_banks = data[5]
        cdef uint8_t flags6 = data[6]
        cdef uint8_t flags7 = data[7]
        cdef bint has_trainer = (flags6 & 0x04) != 0
        cdef bint has_battery = (flags6 & 0x02) != 0
        cdef bint four_screen = (flags6 & 0x08) != 0
        cdef bint vertical = (flags6 & 0x01) != 0
        cdef Mirroring mirroring
        if four_screen:
            mirroring = MIRROR_FOUR_SCREEN
        elif vertical:
            mirroring = MIRROR_VERTICAL
        else:
            mirroring = MIRROR_HORIZONTAL
        cdef uint16_t mapper_number = ((<uint16_t>(flags6 >> 4)) |
                                       (<uint16_t>(flags7 >> 4) << 4))
        cdef uint8_t tv_system = data[9] & 0x03

        cdef uint32_t prg_size = <uint32_t>prg_rom_banks * CART_PRG_ROM_UNIT
        cdef Py_ssize_t prg_off = CART_HEADER_SIZE
        if has_trainer:
            prg_off += CART_TRAINER_SIZE
        cdef uint32_t chr_size = (<uint32_t>chr_rom_banks * CART_CHR_ROM_UNIT) if chr_rom_banks > 0 else 0
        if length < prg_off + prg_size + chr_size:
            return None

        cart.prg_rom_banks = prg_rom_banks
        cart.chr_rom_banks = chr_rom_banks
        cart.mapper_number = mapper_number
        cart.mirroring = mirroring
        cart.has_trainer = has_trainer
        cart.has_battery = has_battery
        cart.tv_system = tv_system
        cart.prg_size = prg_size
        if prg_size > 0:
            cart.prg_rom = <uint8_t*>malloc(prg_size)
            if cart.prg_rom == NULL:
                return None
            memcpy(cart.prg_rom, data + prg_off, prg_size)
        if chr_size > 0:
            cart.chr_size = chr_size
            cart.chr = <uint8_t*>malloc(chr_size)
            if cart.chr == NULL:
                return None
            memcpy(cart.chr, data + prg_off + prg_size, chr_size)
            cart.chr_is_ram = False
        else:
            cart.chr_size = 8192
            cart.chr = <uint8_t*>calloc(1, 8192)
            if cart.chr == NULL:
                return None
            cart.chr_is_ram = True
        cart.vrc6_is_26 = (mapper_number == 26)
        return cart

    # ---- PRG read dispatch ----
    cdef uint8_t read_prg(self, uint16_t addr) nogil:
        cdef uint16_t mn = self.mapper_number
        if mn == 0:
            return self._nrom_read_prg(addr)
        elif mn == 1:
            return self._mmc1_read_prg(addr)
        elif mn == 2:
            return self._uxrom_read_prg(addr)
        elif mn == 3:
            return self._nrom_read_prg(addr)
        elif mn == 4:
            return self._mmc3_read_prg(addr)
        elif mn == 7:
            return self._axrom_read_prg(addr)
        elif mn == 9:
            return self._mmc2_read_prg(addr)
        elif mn == 24 or mn == 26:
            return self._vrc6_read_prg(addr)
        elif mn == 69:
            return self._fme7_read_prg(addr)
        elif mn == 85:
            return self._vrc7_read_prg(addr)
        elif mn == 19:
            return self._n163_read_prg(addr)
        return 0

    # read_prg_mut: PRG read with read side-effects (FDS disk-data read).
    # Falls back to read_prg for non-FDS mappers (matches cartridge.c).
    cdef uint8_t read_prg_mut(self, uint16_t addr) nogil:
        return self.read_prg(addr)

    cdef void write_prg(self, uint16_t addr, uint8_t value) nogil:
        cdef uint16_t mn = self.mapper_number
        if mn == 1:
            self._mmc1_write_prg(addr, value)
        elif mn == 2:
            if addr >= 0x8000:
                self.prg_bank = value & 0x0F
        elif mn == 3:
            if addr >= 0x8000:
                self.chr_bank = value & 0x03
        elif mn == 4:
            self._mmc3_write_prg(addr, value)
        elif mn == 7:
            if addr >= 0x8000:
                self.prg_bank = value & 0x07
        elif mn == 9:
            self._mmc2_write_prg(addr, value)
        elif mn == 24 or mn == 26:
            self._vrc6_write_prg(addr, value)
        elif mn == 69:
            self._fme7_write_prg(addr, value)
        elif mn == 85:
            self._vrc7_write_prg(addr, value)
        elif mn == 19:
            self._n163_write_prg(addr, value)
        # NROM: no writable PRG

    @cython.boundscheck(False)
    @cython.wraparound(False)
    cdef inline uint8_t _nrom_read_prg(self, uint16_t addr) nogil:
        if addr < 0x8000:
            return 0
        cdef uint32_t local = <uint32_t>(addr - 0x8000)
        cdef uint32_t bank = self.prg_size
        if bank == 0:
            return 0
        return self.prg_rom[local % bank]

    # ---- CHR read dispatch ----
    cdef uint8_t read_chr(self, uint16_t addr) nogil:
        cdef uint16_t mn = self.mapper_number
        if mn == 0 or mn == 2 or mn == 7:
            return self._nrom_read_chr(addr)
        elif mn == 1:
            return self._mmc1_read_chr(addr)
        elif mn == 3:
            return self._cnrom_read_chr(addr)
        elif mn == 4:
            return self._mmc3_read_chr(addr)
        elif mn == 9:
            return self._mmc2_read_chr(addr)
        elif mn == 24 or mn == 26:
            return self._vrc6_read_chr(addr)
        elif mn == 69:
            return self._fme7_read_chr(addr)
        elif mn == 85:
            return self._vrc7_read_chr(addr)
        elif mn == 19:
            return self._n163_read_chr(addr)
        return self._nrom_read_chr(addr)

    # read_chr_latched: CHR read with A12 latch side-effects (MMC2/MMC3/MMC4).
    # Falls back to read_chr for mappers without latching (matches cartridge.c).
    cdef uint8_t read_chr_latched(self, uint16_t addr) nogil:
        return self.read_chr(addr)

    cdef void write_chr(self, uint16_t addr, uint8_t value) nogil:
        cdef uint16_t mn = self.mapper_number
        if self.chr_is_ram:
            self._nrom_write_chr(addr, value)
            return
        # CHR-ROM writes ignored for all mappers without CHR-RAM.

    @cython.boundscheck(False)
    @cython.wraparound(False)
    cdef inline uint8_t _nrom_read_chr(self, uint16_t addr) nogil:
        cdef uint32_t sz = self.chr_size
        if sz == 0:
            return 0
        return self.chr[<uint32_t>addr % sz]

    @cython.boundscheck(False)
    @cython.wraparound(False)
    cdef inline void _nrom_write_chr(self, uint16_t addr, uint8_t value) nogil:
        cdef uint32_t sz = self.chr_size
        if sz == 0:
            return
        self.chr[<uint32_t>addr % sz] = value

    cdef Mirroring mirror_mode(self) nogil:
        cdef uint16_t mn = self.mapper_number
        cdef uint8_t m, fm
        if mn == 1:
            m = self.mmc1_control & 3
            if m == 0:
                return MIRROR_VERTICAL
            elif m == 1:
                return MIRROR_HORIZONTAL
            elif m == 2:
                return MIRROR_SINGLE_SCREEN_0
            return MIRROR_SINGLE_SCREEN_1
        elif mn == 4:
            return MIRROR_VERTICAL if (self.mmc3_registers[0] & 0x01) == 0 else MIRROR_HORIZONTAL
        elif mn == 7:
            return MIRROR_SINGLE_SCREEN_0 if (self.prg_bank & 0x10) == 0 else MIRROR_SINGLE_SCREEN_1
        elif mn == 24 or mn == 26:
            return MIRROR_VERTICAL if self.vrc6_mirroring else MIRROR_HORIZONTAL
        elif mn == 69:
            fm = self.fme7_registers[12] & 3
            if fm == 0:
                return MIRROR_VERTICAL
            elif fm == 1:
                return MIRROR_HORIZONTAL
            elif fm == 2:
                return MIRROR_SINGLE_SCREEN_0
            return MIRROR_SINGLE_SCREEN_1
        return self.mirroring

    cdef inline bint irq_pending(self) nogil:
        cdef uint16_t mn = self.mapper_number
        if mn == 4:
            return self.mmc3_irq_pending_flag
        elif mn == 24 or mn == 26:
            return self.vrc6_irq_pending_flag
        elif mn == 69:
            return self.fme7_irq_pending_flag
        return False

    cdef void clock_irq(self) nogil:
        if self.mapper_number == 4:
            if self.mmc3_irq_counter == 0:
                self.mmc3_irq_counter = self.mmc3_irq_reload
            else:
                self.mmc3_irq_counter -= 1
            if self.mmc3_irq_counter == 0 and self.mmc3_irq_enable:
                self.mmc3_irq_pending_flag = True

    cdef void reset_scanline_counter(self) nogil:
        if self.mapper_number == 5:
            self.mmc5_irq_pending_flag = False

    cdef void clock_cpu(self, uint32_t cpu_cycles) nogil:
        cdef uint16_t mn = self.mapper_number
        cdef uint32_t i
        if mn == 24 or mn == 26:
            if self.vrc6_irq_enable:
                for i in range(cpu_cycles):
                    self.vrc6_irq_count += 1
                    if self.vrc6_irq_count == 0x100:
                        self.vrc6_irq_count = self.vrc6_irq_prescale
                        self.vrc6_irq_pending_flag = True
        elif mn == 69:
            if self.fme7_registers[13] & 0x81:
                for i in range(cpu_cycles):
                    self.fme7_irq_count += 1
                    if self.fme7_irq_count == 0x10000:
                        self.fme7_irq_count = self.fme7_registers[14] | (self.fme7_registers[15] << 8)
                        self.fme7_irq_pending_flag = True

    cdef float expansion_audio_sample(self) nogil:
        return 0.0

    # ---- Per-mapper PRG implementations (compact) ----
    @cython.boundscheck(False)
    @cython.wraparound(False)
    cdef uint8_t _uxrom_read_prg(self, uint16_t addr) nogil:
        if addr < 0x8000:
            return 0
        cdef uint32_t local = <uint32_t>(addr - 0x8000)
        if local < 0x4000:
            return self.prg_rom[<uint32_t>self.prg_bank * 0x4000 + local]
        cdef uint32_t last = self.prg_size - 0x4000
        return self.prg_rom[last + (local - 0x4000)]

    @cython.boundscheck(False)
    @cython.wraparound(False)
    cdef uint8_t _axrom_read_prg(self, uint16_t addr) nogil:
        if addr < 0x8000:
            return 0
        cdef uint32_t local = <uint32_t>(addr - 0x8000)
        cdef uint32_t bank = <uint32_t>self.prg_bank & 0x07
        return self.prg_rom[bank * 0x8000 + local]

    @cython.boundscheck(False)
    @cython.wraparound(False)
    cdef uint8_t _cnrom_read_chr(self, uint16_t addr) nogil:
        cdef uint32_t bank = <uint32_t>self.chr_bank & 0x03
        cdef uint32_t off = bank * 0x2000 + <uint32_t>addr
        if self.chr_size == 0:
            return 0
        return self.chr[off % self.chr_size]

    # ---- MMC1 ----
    cdef void _mmc1_write_prg(self, uint16_t addr, uint8_t value) nogil:
        if addr < 0x8000:
            return
        if (value & 0x80) != 0:
            self.mmc1_shift_reg = 0
            self.mmc1_shift_count = 0
            self.mmc1_control |= 0x0C
            return
        self.mmc1_shift_reg |= ((value & 1) << self.mmc1_shift_count)
        self.mmc1_shift_count += 1
        if self.mmc1_shift_count < 5:
            return
        cdef uint8_t reg = (addr >> 13) & 3
        cdef uint8_t v = self.mmc1_shift_reg
        self.mmc1_shift_reg = 0
        self.mmc1_shift_count = 0
        if reg == 0:
            self.mmc1_control = v
        elif reg == 1:
            self.mmc1_chr_bank_0 = v
        elif reg == 2:
            self.mmc1_chr_bank_1 = v
        else:
            self.mmc1_prg_bank = v

    @cython.boundscheck(False)
    @cython.wraparound(False)
    cdef uint8_t _mmc1_read_prg(self, uint16_t addr) nogil:
        if addr < 0x8000:
            return 0
        cdef uint8_t mode = (self.mmc1_control >> 2) & 3
        cdef uint32_t local = <uint32_t>(addr - 0x8000)
        cdef uint32_t bank, last
        if mode == 0 or mode == 1:
            bank = <uint32_t>(self.mmc1_prg_bank & 0x0E) * 0x4000
            return self.prg_rom[bank + (local & 0x7FFF)]
        elif mode == 2:
            if local < 0x4000:
                return self.prg_rom[local]
            bank = <uint32_t>(self.mmc1_prg_bank & 0x0F) * 0x4000
            return self.prg_rom[bank + (local - 0x4000)]
        else:
            if local < 0x4000:
                bank = <uint32_t>(self.mmc1_prg_bank & 0x0F) * 0x4000
                return self.prg_rom[bank + local]
            last = self.prg_size - 0x4000
            return self.prg_rom[last + (local - 0x4000)]

    @cython.boundscheck(False)
    @cython.wraparound(False)
    cdef uint8_t _mmc1_read_chr(self, uint16_t addr) nogil:
        cdef uint8_t mode = (self.mmc1_control >> 4) & 1
        cdef uint32_t bank
        if mode == 0:
            bank = <uint32_t>(self.mmc1_chr_bank_0 & 0x1E) * 0x1000
            return self.chr[(bank + <uint32_t>addr) % self.chr_size]
        if <uint32_t>addr < 0x1000:
            bank = <uint32_t>(self.mmc1_chr_bank_0 & 0x1F) * 0x1000
            return self.chr[(bank + <uint32_t>addr) % self.chr_size]
        bank = <uint32_t>(self.mmc1_chr_bank_1 & 0x1F) * 0x1000
        return self.chr[(bank + (<uint32_t>addr - 0x1000)) % self.chr_size]

    # ---- MMC3 ----
    cdef void _mmc3_write_prg(self, uint16_t addr, uint8_t value) nogil:
        cdef uint8_t bank = (addr >> 12) & 0x07
        if bank == 0:
            self.mmc3_bank_select = value & 0x07
            self.mmc3_chr_mode_8k = (value & 0x80) >> 7
        elif bank == 1:
            self.mmc3_registers[self.mmc3_bank_select] = value
        elif bank == 2:
            self.mmc3_irq_reload = value
        elif bank == 3:
            self.mmc3_irq_enable = False
            self.mmc3_irq_counter = 0
            self.mmc3_irq_pending_flag = False
        elif bank == 4:
            self.mmc3_irq_enable = True
        elif bank == 5:
            self.mmc3_irq_counter = self.mmc3_irq_reload
        elif bank == 6:
            self.mmc3_irq_enable = True
        elif bank == 7:
            self.mmc3_irq_enable = False

    @cython.boundscheck(False)
    @cython.wraparound(False)
    cdef uint8_t _mmc3_read_prg(self, uint16_t addr) nogil:
        if addr < 0x8000:
            return 0
        cdef uint32_t local = <uint32_t>(addr - 0x8000)
        cdef uint8_t r0 = self.mmc3_registers[6]
        cdef uint8_t r1 = self.mmc3_registers[7]
        cdef uint32_t bank
        if local < 0x2000:
            bank = <uint32_t>r0 * 0x2000
            return self.prg_rom[bank + local]
        elif local < 0x4000:
            bank = <uint32_t>r1 * 0x2000
            return self.prg_rom[bank + (local - 0x2000)]
        elif local < 0x6000:
            return self.prg_rom[(self.prg_size - 0x4000) + (local - 0x4000)]
        else:
            return self.prg_rom[(self.prg_size - 0x2000) + (local - 0x6000)]

    @cython.boundscheck(False)
    @cython.wraparound(False)
    cdef uint8_t _mmc3_read_chr(self, uint16_t addr) nogil:
        cdef uint32_t bank
        cdef uint32_t a = <uint32_t>addr
        cdef uint8_t mode = <uint8_t>self.mmc3_chr_mode_8k
        if mode == 0:
            if a < 0x1000:
                bank = <uint32_t>self.mmc3_registers[0] * 0x400
                return self.chr[(bank + a) % self.chr_size]
            elif a < 0x2000:
                bank = <uint32_t>self.mmc3_registers[1] * 0x400
                return self.chr[(bank + (a - 0x1000)) % self.chr_size]
            elif a < 0x3000:
                bank = <uint32_t>self.mmc3_registers[2] * 0x400
                return self.chr[(bank + (a - 0x2000)) % self.chr_size]
            elif a < 0x4000:
                bank = <uint32_t>self.mmc3_registers[3] * 0x400
                return self.chr[(bank + (a - 0x3000)) % self.chr_size]
            elif a < 0x5000:
                bank = <uint32_t>self.mmc3_registers[4] * 0x400
                return self.chr[(bank + (a - 0x4000)) % self.chr_size]
            elif a < 0x6000:
                bank = <uint32_t>self.mmc3_registers[5] * 0x400
                return self.chr[(bank + (a - 0x5000)) % self.chr_size]
            elif a < 0x7000:
                bank = <uint32_t>self.mmc3_registers[6] * 0x400
                return self.chr[(bank + (a - 0x6000)) % self.chr_size]
            else:
                bank = <uint32_t>self.mmc3_registers[7] * 0x400
                return self.chr[(bank + (a - 0x7000)) % self.chr_size]
        else:
            if a < 0x0800:
                bank = <uint32_t>self.mmc3_registers[2] * 0x400
                return self.chr[(bank + a) % self.chr_size]
            elif a < 0x1000:
                bank = <uint32_t>self.mmc3_registers[3] * 0x400
                return self.chr[(bank + (a - 0x0800)) % self.chr_size]
            elif a < 0x1800:
                bank = <uint32_t>self.mmc3_registers[4] * 0x400
                return self.chr[(bank + (a - 0x1000)) % self.chr_size]
            elif a < 0x2000:
                bank = <uint32_t>self.mmc3_registers[5] * 0x400
                return self.chr[(bank + (a - 0x1800)) % self.chr_size]
            return 0

    # ---- MMC2 ----
    @cython.boundscheck(False)
    @cython.wraparound(False)
    cdef uint8_t _mmc2_read_prg(self, uint16_t addr) nogil:
        if addr < 0x8000:
            return 0
        cdef uint32_t local = <uint32_t>(addr - 0x8000)
        cdef uint32_t bank
        if local < 0x2000:
            bank = <uint32_t>self.mmc2_prg_banks[0] * 0x2000
            return self.prg_rom[bank + local]
        elif local < 0x4000:
            bank = <uint32_t>self.mmc2_prg_banks[1] * 0x2000
            return self.prg_rom[bank + (local - 0x2000)]
        elif local < 0x6000:
            bank = <uint32_t>self.mmc2_prg_banks[2] * 0x2000
            return self.prg_rom[bank + (local - 0x4000)]
        else:
            return self.prg_rom[(self.prg_size - 0x2000) + (local - 0x6000)]

    cdef void _mmc2_write_prg(self, uint16_t addr, uint8_t value) nogil:
        if 0x8000 <= addr < 0xA000:
            self.mmc2_prg_banks[0] = value & 0x0F
        elif 0xB000 <= addr < 0xC000:
            self.mmc2_prg_banks[1] = value & 0x0F
        elif 0xC000 <= addr < 0xE000:
            self.mmc2_prg_banks[2] = value & 0x0F

    @cython.boundscheck(False)
    @cython.wraparound(False)
    cdef uint8_t _mmc2_read_chr(self, uint16_t addr) nogil:
        cdef uint32_t bank
        cdef uint32_t a = <uint32_t>addr
        if a < 0x1000:
            bank = <uint32_t>(self.mmc2_chr_banks[0] if self.mmc2_latch_fd else self.mmc2_chr_banks[1]) * 0x400
            return self.chr[(bank + a) % self.chr_size]
        elif a < 0x2000:
            bank = <uint32_t>(self.mmc2_chr_banks[2] if self.mmc2_latch_fe else self.mmc2_chr_banks[3]) * 0x400
            return self.chr[(bank + (a - 0x1000)) % self.chr_size]
        return 0

    # ---- VRC6 ----
    cdef void _vrc6_write_prg(self, uint16_t addr, uint8_t value) nogil:
        cdef uint16_t a = addr
        if self.vrc6_is_26:
            a = ((a & 0xF000) | ((a >> 1) & 0x03) | ((a << 1) & 0x04)) & 0xF007
        if (a & 0xF000) == 0x8000:
            self.vrc6_prg_bank[0] = value & 0x3F
        elif (a & 0xF000) == 0xA000:
            self.vrc6_prg_bank[1] = value & 0x3F
        elif a == 0xB003:
            self.vrc6_mirroring = (value & 1) != 0
        elif a == 0xF000:
            self.vrc6_irq_enable = value & 0x02
        elif a == 0xF001:
            self.vrc6_irq_prescale = value
        elif a == 0xF002:
            self.vrc6_irq_enable = value & 0x01

    @cython.boundscheck(False)
    @cython.wraparound(False)
    cdef uint8_t _vrc6_read_prg(self, uint16_t addr) nogil:
        if addr < 0x8000:
            return 0
        cdef uint32_t local = <uint32_t>(addr - 0x8000)
        cdef uint32_t bank
        if local < 0x2000:
            bank = <uint32_t>self.vrc6_prg_bank[0] * 0x2000
            return self.prg_rom[bank + local]
        elif local < 0x4000:
            bank = <uint32_t>self.vrc6_prg_bank[1] * 0x2000
            return self.prg_rom[bank + (local - 0x2000)]
        elif local < 0x6000:
            return self.prg_rom[(self.prg_size - 0x4000) + (local - 0x4000)]
        else:
            return self.prg_rom[(self.prg_size - 0x2000) + (local - 0x6000)]

    @cython.boundscheck(False)
    @cython.wraparound(False)
    cdef uint8_t _vrc6_read_chr(self, uint16_t addr) nogil:
        cdef uint32_t bank = <uint32_t>self.vrc6_chr_bank[(<uint32_t>addr >> 10) & 7] * 0x400
        return self.chr[(bank + (<uint32_t>addr & 0x3FF)) % self.chr_size]

    # ---- FME-7 ----
    cdef void _fme7_write_prg(self, uint16_t addr, uint8_t value) nogil:
        if (addr & 0xE000) == 0x8000:
            self.fme7_command = value & 0x0F
        elif (addr & 0xE000) == 0xA000:
            self.fme7_registers[self.fme7_command] = value

    @cython.boundscheck(False)
    @cython.wraparound(False)
    cdef uint8_t _fme7_read_prg(self, uint16_t addr) nogil:
        if addr < 0x8000:
            return 0
        cdef uint32_t local = <uint32_t>(addr - 0x8000)
        cdef uint32_t bank
        if local < 0x2000:
            bank = <uint32_t>self.fme7_registers[0] * 0x2000
            return self.prg_rom[bank + local]
        elif local < 0x4000:
            bank = <uint32_t>self.fme7_registers[1] * 0x2000
            return self.prg_rom[bank + (local - 0x2000)]
        elif local < 0x6000:
            bank = <uint32_t>self.fme7_registers[2] * 0x2000
            return self.prg_rom[bank + (local - 0x4000)]
        else:
            bank = <uint32_t>self.fme7_registers[3] * 0x2000
            return self.prg_rom[bank + (local - 0x6000)]

    @cython.boundscheck(False)
    @cython.wraparound(False)
    cdef uint8_t _fme7_read_chr(self, uint16_t addr) nogil:
        cdef uint8_t bank_idx = <uint8_t>((<uint32_t>addr >> 10) & 7)
        cdef uint32_t bank = <uint32_t>self.fme7_registers[8 + bank_idx] * 0x400
        return self.chr[(bank + (<uint32_t>addr & 0x3FF)) % self.chr_size]

    # ---- VRC7 ----
    cdef void _vrc7_write_prg(self, uint16_t addr, uint8_t value) nogil:
        if (addr & 0xF000) == 0x8000:
            self.vrc7_prg_bank[0] = value & 0x3F
        elif (addr & 0xF000) == 0xA000:
            self.vrc7_prg_bank[1] = value & 0x3F
        elif (addr & 0xF000) == 0xC000:
            self.vrc7_prg_bank[2] = value & 0x3F

    @cython.boundscheck(False)
    @cython.wraparound(False)
    cdef uint8_t _vrc7_read_prg(self, uint16_t addr) nogil:
        if addr < 0x8000:
            return 0
        cdef uint32_t local = <uint32_t>(addr - 0x8000)
        cdef uint32_t bank
        if local < 0x2000:
            bank = <uint32_t>self.vrc7_prg_bank[0] * 0x2000
            return self.prg_rom[bank + local]
        elif local < 0x4000:
            bank = <uint32_t>self.vrc7_prg_bank[1] * 0x2000
            return self.prg_rom[bank + (local - 0x2000)]
        elif local < 0x6000:
            bank = <uint32_t>self.vrc7_prg_bank[2] * 0x2000
            return self.prg_rom[bank + (local - 0x4000)]
        else:
            return self.prg_rom[(self.prg_size - 0x2000) + (local - 0x6000)]

    @cython.boundscheck(False)
    @cython.wraparound(False)
    cdef uint8_t _vrc7_read_chr(self, uint16_t addr) nogil:
        cdef uint8_t bank_idx = <uint8_t>((<uint32_t>addr >> 10) & 7)
        cdef uint32_t bank = <uint32_t>self.vrc7_chr_bank[bank_idx] * 0x400
        return self.chr[(bank + (<uint32_t>addr & 0x3FF)) % self.chr_size]

    # ---- Namco 163 ----
    cdef void _n163_write_prg(self, uint16_t addr, uint8_t value) nogil:
        if (addr & 0xF800) == 0x4800:
            self.n163_ram[self.n163_addr & 0x7F] = value
            self.n163_addr += (self.n163_increment & 0x03) if (self.n163_increment & 0x80) else 1
        elif addr == 0x5000:
            self.n163_addr = value
        elif addr == 0x5100:
            self.n163_increment = value
        elif (addr & 0xF800) == 0x8000:
            self.n163_prg_bank[0] = value & 0x3F
        elif (addr & 0xF800) == 0xA000:
            self.n163_prg_bank[1] = value & 0x3F
        elif (addr & 0xF800) == 0xC000:
            self.n163_prg_bank[2] = value & 0x3F

    @cython.boundscheck(False)
    @cython.wraparound(False)
    cdef uint8_t _n163_read_prg(self, uint16_t addr) nogil:
        cdef uint8_t v
        cdef uint32_t local, bank
        if addr < 0x6000:
            if 0x4800 <= addr < 0x5000:
                v = self.n163_ram[self.n163_addr & 0x7F]
                self.n163_addr += (self.n163_increment & 0x03) if (self.n163_increment & 0x80) else 1
                return v
            return 0
        local = <uint32_t>(addr - 0x8000)
        if local < 0x2000:
            bank = <uint32_t>self.n163_prg_bank[0] * 0x2000
            return self.prg_rom[bank + local]
        elif local < 0x4000:
            bank = <uint32_t>self.n163_prg_bank[1] * 0x2000
            return self.prg_rom[bank + (local - 0x2000)]
        elif local < 0x6000:
            bank = <uint32_t>self.n163_prg_bank[2] * 0x2000
            return self.prg_rom[bank + (local - 0x4000)]
        else:
            return self.prg_rom[(self.prg_size - 0x2000) + (local - 0x6000)]

    @cython.boundscheck(False)
    @cython.wraparound(False)
    cdef uint8_t _n163_read_chr(self, uint16_t addr) nogil:
        cdef uint8_t bank_idx = <uint8_t>((<uint32_t>addr >> 10) & 7)
        cdef uint32_t bank = <uint32_t>self.n163_chr_bank[bank_idx] * 0x400
        return self.chr[(bank + (<uint32_t>addr & 0x3FF)) % self.chr_size]


# ============================================================================
# Joypad (joypad.c)
# ============================================================================

cdef class Joypad:
    cdef uint8_t current[2]
    cdef bint strobe
    cdef uint8_t shift[2]
    cdef uint8_t counter[2]

    def __cinit__(self):
        self.current[0] = 0
        self.current[1] = 0
        self.strobe = False
        self.shift[0] = 0
        self.shift[1] = 0
        self.counter[0] = 0
        self.counter[1] = 0

    cdef void write_strobe(self, uint8_t value) nogil:
        cdef bint new_strobe = (value & 0x01) != 0
        cdef int c
        if self.strobe and not new_strobe:
            for c in range(2):
                self.shift[c] = self.current[c]
                self.counter[c] = 0
        self.strobe = new_strobe
        if self.strobe:
            for c in range(2):
                self.shift[c] = self.current[c]

    cdef uint8_t read(self, uint8_t controller) nogil:
        if controller >= 2:
            return 1
        if self.strobe:
            return self.current[controller] & 0x01
        cdef uint8_t bit
        if self.counter[controller] < 8:
            bit = (self.shift[controller] >> self.counter[controller]) & 0x01
        else:
            bit = 1
        if self.counter[controller] < 0xFF:
            self.counter[controller] += 1
        return bit

    cdef void set_button(self, uint8_t controller, uint8_t button, bint pressed) nogil:
        if controller >= 2 or button >= 8:
            return
        cdef uint8_t mask = <uint8_t>(1 << button)
        if pressed:
            self.current[controller] |= mask
        else:
            self.current[controller] &= <uint8_t>~mask
        if self.strobe:
            self.shift[controller] = self.current[controller]


# ============================================================================
# APU channels (apu.c)
# ============================================================================

cdef class PulseChannel:
    cdef bint pulse2
    cdef uint8_t duty
    cdef bint halt
    cdef bint constant_volume
    cdef uint8_t volume
    cdef bint sweep_enabled
    cdef uint8_t sweep_period
    cdef bint sweep_negate
    cdef uint8_t sweep_shift
    cdef uint8_t sweep_divider
    cdef bint sweep_reload
    cdef uint16_t timer_period
    cdef uint16_t timer
    cdef uint8_t sequence
    cdef uint8_t length_counter
    cdef bint enabled
    cdef uint8_t envelope_divider
    cdef uint8_t envelope_decay
    cdef bint envelope_start

    def __cinit__(self, bint pulse2):
        self.pulse2 = pulse2
        self.duty = 0
        self.halt = False
        self.constant_volume = False
        self.volume = 0
        self.sweep_enabled = False
        self.sweep_period = 0
        self.sweep_negate = False
        self.sweep_shift = 0
        self.sweep_divider = 0
        self.sweep_reload = False
        self.timer_period = 0
        self.timer = 0
        self.sequence = 0
        self.length_counter = 0
        self.enabled = False
        self.envelope_divider = 0
        self.envelope_decay = 0
        self.envelope_start = False

    cdef void write_register(self, uint8_t reg, uint8_t value) nogil:
        cdef uint16_t high
        if reg == 0:
            self.duty = (value >> 6) & 0x03
            self.halt = (value & 0x20) != 0
            self.constant_volume = (value & 0x10) != 0
            self.volume = value & 0x0F
        elif reg == 1:
            self.sweep_enabled = (value & 0x80) != 0
            self.sweep_period = (value >> 4) & 0x07
            self.sweep_negate = (value & 0x08) != 0
            self.sweep_shift = value & 0x07
            self.sweep_reload = True
        elif reg == 2:
            self.timer_period = (self.timer_period & 0xFF00) | value
        elif reg == 3:
            high = value & 0x07
            self.timer_period = (self.timer_period & 0x00FF) | (high << 8)
            self.timer = (self.timer & 0x00FF) | (high << 8)
            if self.enabled:
                self.length_counter = APU_LENGTH_TABLE[value >> 3]
            self.envelope_start = True
            self.sequence = 0

    cdef void set_enabled(self, bint enabled) nogil:
        self.enabled = enabled
        if not enabled:
            self.length_counter = 0

    cdef uint16_t sweep_target(self) nogil:
        if not self.sweep_enabled or self.sweep_shift == 0:
            return self.timer_period
        cdef uint16_t shifted = self.timer_period >> self.sweep_shift
        if self.sweep_negate:
            if self.pulse2:
                return (self.timer_period - shifted) - 1
            return self.timer_period - shifted
        return self.timer_period + shifted

    cdef bint is_muted(self) nogil:
        return self.timer_period < APU_MIN_AUDIBLE_PERIOD or self.sweep_target() > APU_MAX_PERIOD

    cdef void tick(self) nogil:
        if self.timer == 0:
            self.timer = self.timer_period
            self.sequence = (self.sequence + 1) & 0x07
        else:
            self.timer -= 1

    cdef void clock_envelope(self) nogil:
        if self.envelope_start:
            self.envelope_start = False
            self.envelope_decay = 15
            self.envelope_divider = self.volume
        elif self.envelope_divider == 0:
            self.envelope_divider = self.volume
            if self.envelope_decay > 0:
                self.envelope_decay -= 1
            elif self.halt:
                self.envelope_decay = 15
        else:
            self.envelope_divider -= 1

    cdef void clock_length(self) nogil:
        if not self.halt and self.length_counter > 0:
            self.length_counter -= 1

    cdef void clock_sweep(self) nogil:
        cdef bint divider_zero = self.sweep_divider == 0
        cdef uint16_t target
        if divider_zero and self.sweep_enabled and self.sweep_shift != 0:
            target = self.sweep_target()
            if target <= APU_MAX_PERIOD and self.timer_period >= APU_MIN_AUDIBLE_PERIOD:
                self.timer_period = target
        if divider_zero or self.sweep_reload:
            self.sweep_divider = self.sweep_period
            self.sweep_reload = False
        else:
            self.sweep_divider -= 1

    cdef void clock_quarter_frame(self) nogil:
        self.clock_envelope()

    cdef void clock_half_frame(self) nogil:
        self.clock_length()
        self.clock_sweep()

    cdef uint8_t sample(self) nogil:
        if not self.enabled or self.length_counter == 0 or self.is_muted():
            return 0
        cdef uint8_t duty_bit
        if self.duty == 0:
            duty_bit = APU_DUTY_PATTERNS_0[self.sequence]
        elif self.duty == 1:
            duty_bit = APU_DUTY_PATTERNS_1[self.sequence]
        elif self.duty == 2:
            duty_bit = APU_DUTY_PATTERNS_2[self.sequence]
        else:
            duty_bit = APU_DUTY_PATTERNS_3[self.sequence]
        if duty_bit == 0:
            return 0
        return self.volume if self.constant_volume else self.envelope_decay


cdef class TriangleChannel:
    cdef bint halt
    cdef uint8_t linear_reload
    cdef uint8_t linear_counter
    cdef bint linear_start
    cdef uint16_t timer_period
    cdef uint16_t timer
    cdef uint8_t sequence
    cdef uint8_t length_counter
    cdef bint enabled

    def __cinit__(self):
        self.halt = False
        self.linear_reload = 0
        self.linear_counter = 0
        self.linear_start = False
        self.timer_period = 0
        self.timer = 0
        self.sequence = 0
        self.length_counter = 0
        self.enabled = False

    cdef void write_register(self, uint8_t reg, uint8_t value) nogil:
        cdef uint16_t high
        if reg == 0:
            self.halt = (value & 0x80) != 0
            self.linear_reload = value & 0x7F
        elif reg == 1:
            pass
        elif reg == 2:
            self.timer_period = (self.timer_period & 0xFF00) | value
        elif reg == 3:
            high = value & 0x07
            self.timer_period = (self.timer_period & 0x00FF) | (high << 8)
            self.timer = (self.timer & 0x00FF) | (high << 8)
            if self.enabled:
                self.length_counter = APU_LENGTH_TABLE[value >> 3]
            self.linear_start = True
            self.sequence = 0

    cdef void set_enabled(self, bint enabled) nogil:
        self.enabled = enabled
        if not enabled:
            self.length_counter = 0

    cdef void tick(self) nogil:
        if self.timer == 0:
            self.timer = self.timer_period
            self.sequence = (self.sequence + 1) & 0x1F
        else:
            self.timer -= 1

    cdef void clock_linear(self) nogil:
        if self.linear_start:
            self.linear_start = False
            self.linear_counter = self.linear_reload
        elif self.linear_counter > 0:
            self.linear_counter -= 1
        if self.halt:
            self.linear_start = True

    cdef void clock_length(self) nogil:
        if not self.halt and self.length_counter > 0:
            self.length_counter -= 1

    cdef void clock_quarter_frame(self) nogil:
        self.clock_linear()

    cdef void clock_half_frame(self) nogil:
        self.clock_length()

    cdef uint8_t sample(self) nogil:
        if not self.enabled or self.length_counter == 0 or self.linear_counter == 0:
            return 0
        return APU_TRIANGLE_SEQUENCE[self.sequence]


cdef class NoiseChannel:
    cdef bint halt
    cdef bint constant_volume
    cdef uint8_t volume
    cdef bint mode
    cdef uint8_t period_index
    cdef uint16_t timer_period
    cdef uint16_t timer
    cdef uint16_t lfsr
    cdef uint8_t length_counter
    cdef bint enabled
    cdef uint8_t envelope_divider
    cdef uint8_t envelope_decay
    cdef bint envelope_start

    def __cinit__(self):
        self.halt = False
        self.constant_volume = False
        self.volume = 0
        self.mode = False
        self.period_index = 0
        self.timer_period = APU_NOISE_PERIOD_TABLE[0]
        self.timer = 0
        self.lfsr = 1
        self.length_counter = 0
        self.enabled = False
        self.envelope_divider = 0
        self.envelope_decay = 0
        self.envelope_start = False

    cdef void write_register(self, uint8_t reg, uint8_t value) nogil:
        if reg == 0:
            self.halt = (value & 0x20) != 0
            self.constant_volume = (value & 0x10) != 0
            self.volume = value & 0x0F
        elif reg == 1:
            pass
        elif reg == 2:
            self.mode = (value & 0x80) != 0
            self.period_index = value & 0x0F
            self.timer_period = APU_NOISE_PERIOD_TABLE[self.period_index]
        elif reg == 3:
            if self.enabled:
                self.length_counter = APU_LENGTH_TABLE[value >> 3]
            self.envelope_start = True

    cdef void set_enabled(self, bint enabled) nogil:
        self.enabled = enabled
        if not enabled:
            self.length_counter = 0

    cdef void tick(self) nogil:
        cdef uint16_t bit0, tap, tap_bit, feedback
        if self.timer == 0:
            self.timer = self.timer_period
            bit0 = self.lfsr & 0x0001
            tap = 6 if self.mode else 1
            tap_bit = (self.lfsr >> tap) & 0x0001
            feedback = bit0 ^ tap_bit
            self.lfsr >>= 1
            if feedback != 0:
                self.lfsr |= 0x4000
        else:
            self.timer -= 1

    cdef void clock_envelope(self) nogil:
        if self.envelope_start:
            self.envelope_start = False
            self.envelope_decay = 15
            self.envelope_divider = self.volume
        elif self.envelope_divider == 0:
            self.envelope_divider = self.volume
            if self.envelope_decay > 0:
                self.envelope_decay -= 1
            elif self.halt:
                self.envelope_decay = 15
        else:
            self.envelope_divider -= 1

    cdef void clock_length(self) nogil:
        if not self.halt and self.length_counter > 0:
            self.length_counter -= 1

    cdef void clock_quarter_frame(self) nogil:
        self.clock_envelope()

    cdef void clock_half_frame(self) nogil:
        self.clock_length()

    cdef uint8_t sample(self) nogil:
        if not self.enabled or self.length_counter == 0:
            return 0
        if (self.lfsr & 1) != 0:
            return 0
        return self.volume if self.constant_volume else self.envelope_decay


# DMC read callback type (matches apu.h DmcReadFn). Declared before
# DmcChannel so the channel methods can reference it.
ctypedef uint8_t (*DmcReadFn)(uint16_t addr, void *ctx) nogil


cdef class DmcChannel:
    cdef bint irq_enable
    cdef bint loop_flag
    cdef uint8_t rate_index
    cdef uint16_t timer_period
    cdef uint16_t timer
    cdef uint8_t output_counter
    cdef uint8_t sample_buffer
    cdef uint8_t buffer_bits
    cdef uint16_t sample_addr_base
    cdef uint16_t sample_address
    cdef uint16_t sample_length
    cdef uint16_t bytes_remaining
    cdef bint enabled
    cdef bint irq_flag

    def __cinit__(self):
        self.irq_enable = False
        self.loop_flag = False
        self.rate_index = 0
        self.timer_period = APU_DMC_RATE_TABLE[0]
        self.timer = 0
        self.output_counter = 0
        self.sample_buffer = 0
        self.buffer_bits = 0
        self.sample_addr_base = 0xC000
        self.sample_address = 0xC000
        self.sample_length = 1
        self.bytes_remaining = 0
        self.enabled = False
        self.irq_flag = False

    cdef void write_register(self, uint8_t reg, uint8_t value) nogil:
        if reg == 0:
            self.irq_enable = (value & 0x80) != 0
            self.loop_flag = (value & 0x40) != 0
            self.rate_index = value & 0x0F
            self.timer_period = APU_DMC_RATE_TABLE[self.rate_index]
        elif reg == 1:
            self.output_counter = value & 0x7F
        elif reg == 2:
            self.sample_addr_base = (value << 6) | 0xC000
        elif reg == 3:
            self.sample_length = (value << 4) | 1

    cdef void set_enabled(self, bint enabled) nogil:
        self.enabled = enabled
        if enabled:
            if self.bytes_remaining == 0:
                self.sample_address = self.sample_addr_base
                self.bytes_remaining = self.sample_length
                self.buffer_bits = 0
        else:
            self.bytes_remaining = 0

    cdef void clear_irq(self) nogil:
        self.irq_flag = False

    cdef void clock_output_unit(self, Bus bus) nogil:
        cdef uint8_t bit
        cdef uint16_t v
        if self.buffer_bits == 0 and self.bytes_remaining > 0:
            self.sample_buffer = bus.dmc_read(self.sample_address)
            self.buffer_bits = 8
            self.sample_address += 1
            if self.sample_address == 0:
                self.sample_address = 0x8000
            self.bytes_remaining -= 1
            if self.bytes_remaining == 0:
                if self.loop_flag:
                    self.sample_address = self.sample_addr_base
                    self.bytes_remaining = self.sample_length
                elif self.irq_enable:
                    self.irq_flag = True
        if self.buffer_bits > 0:
            bit = self.sample_buffer & 1
            if bit == 0:
                self.output_counter = self.output_counter - 2 if self.output_counter >= 2 else 0
            else:
                v = self.output_counter + 2
                if v > 127:
                    v = 127
                self.output_counter = <uint8_t>v
            self.sample_buffer >>= 1
            self.buffer_bits -= 1

    cdef void tick(self, Bus bus) nogil:
        if self.timer == 0:
            self.timer = self.timer_period
            self.clock_output_unit(bus)
        else:
            self.timer -= 1

    cdef uint8_t sample(self) nogil:
        return self.output_counter


cdef class Apu:
    cdef PulseChannel pulse1
    cdef PulseChannel pulse2
    cdef TriangleChannel triangle
    cdef NoiseChannel noise
    cdef DmcChannel dmc

    cdef uint32_t cycle_accumulator
    cdef float channel_volumes[5]
    cdef bint channel_muted[5]
    cdef uint8_t selected_channel

    cdef float lpf_prev
    cdef float dc_prev_x
    cdef float dc_prev_y
    cdef float mix_accumulator
    cdef uint32_t mix_count
    cdef float sample_accumulator
    cdef float last_decimated

    cdef bint frame_mode_5step
    cdef bint frame_irq_inhibit
    cdef uint32_t frame_cycle
    cdef bint frame_irq
    cdef uint32_t frame_reset_delay

    cdef Region region

    def __cinit__(self):
        self.pulse1 = PulseChannel(False)
        self.pulse2 = PulseChannel(True)
        self.triangle = TriangleChannel()
        self.noise = NoiseChannel()
        self.dmc = DmcChannel()
        self.cycle_accumulator = 0
        cdef int i
        for i in range(5):
            self.channel_volumes[i] = 1.0
            self.channel_muted[i] = False
        self.selected_channel = 0
        self.lpf_prev = -1.0
        self.dc_prev_x = -1.0
        self.dc_prev_y = 0.0
        self.mix_accumulator = 0.0
        self.mix_count = 0
        self.sample_accumulator = 0.0
        self.last_decimated = -1.0
        self.frame_mode_5step = False
        self.frame_irq_inhibit = False
        self.frame_cycle = 0
        self.frame_irq = False
        self.frame_reset_delay = 0
        self.region = REGION_NTSC

    cdef void write_status(self, uint8_t value) nogil:
        self.pulse1.set_enabled((value & 0x01) != 0)
        self.pulse2.set_enabled((value & 0x02) != 0)
        self.triangle.set_enabled((value & 0x04) != 0)
        self.noise.set_enabled((value & 0x08) != 0)
        self.dmc.set_enabled((value & 0x10) != 0)

    cdef uint8_t read_status(self) nogil:
        cdef uint8_t v = 0
        if self.pulse1.length_counter > 0:    v |= 0x01
        if self.pulse2.length_counter > 0:    v |= 0x02
        if self.triangle.length_counter > 0:  v |= 0x04
        if self.noise.length_counter > 0:     v |= 0x08
        if self.dmc.bytes_remaining > 0:      v |= 0x10
        if self.frame_irq:                    v |= 0x40
        if self.dmc.irq_flag:                 v |= 0x80
        self.frame_irq = False
        self.dmc.irq_flag = False
        return v

    cdef void write_frame_counter(self, uint8_t value) nogil:
        cdef bint new_mode_5step = (value & 0x80) != 0
        self.frame_irq_inhibit = (value & 0x40) != 0
        if self.frame_irq_inhibit:
            self.frame_irq = False
        if new_mode_5step:
            self.clock_quarter_frame()
            self.clock_half_frame()
        self.frame_reset_delay = 4
        self.frame_mode_5step = new_mode_5step

    cdef void set_region(self, Region region) nogil:
        self.region = region

    cdef bint irq_pending(self) nogil:
        return self.frame_irq or self.dmc.irq_flag

    cdef void clock_quarter_frame(self) nogil:
        self.pulse1.clock_quarter_frame()
        self.pulse2.clock_quarter_frame()
        self.triangle.clock_quarter_frame()
        self.noise.clock_quarter_frame()

    cdef void clock_half_frame(self) nogil:
        self.pulse1.clock_half_frame()
        self.pulse2.clock_half_frame()
        self.triangle.clock_half_frame()
        self.noise.clock_half_frame()

    cdef void step_frame_counter(self, uint32_t cpu_cycles) nogil:
        cdef uint32_t advance, remaining
        if self.frame_reset_delay > 0:
            advance = cpu_cycles if cpu_cycles < self.frame_reset_delay else self.frame_reset_delay
            self.frame_reset_delay -= advance
            if self.frame_reset_delay == 0:
                self.frame_cycle = 0
            remaining = cpu_cycles - advance
            if remaining == 0:
                return
            self.frame_cycle += remaining
        else:
            self.frame_cycle += cpu_cycles

        cdef uint32_t prev = (self.frame_cycle - cpu_cycles) if self.frame_cycle >= cpu_cycles else 0
        cdef uint32_t irq_threshold = region_apu_4step_irq_threshold(self.region)
        cdef uint32_t[4] thr_buf
        cdef int i
        if self.frame_mode_5step:
            if self.region == REGION_PAL:
                for i in range(4):
                    thr_buf[i] = PAL_5STEP[i]
            else:
                for i in range(4):
                    thr_buf[i] = NTSC_5STEP[i]
        else:
            if self.region == REGION_PAL:
                for i in range(4):
                    thr_buf[i] = PAL_4STEP[i]
            else:
                for i in range(4):
                    thr_buf[i] = NTSC_4STEP[i]

        cdef uint32_t threshold
        for i in range(4):
            threshold = thr_buf[i]
            if prev < threshold and self.frame_cycle >= threshold:
                if FRAME_QUARTER[i]:
                    self.clock_quarter_frame()
                if FRAME_HALF[i]:
                    self.clock_half_frame()
                if not self.frame_mode_5step and threshold == irq_threshold and not self.frame_irq_inhibit:
                    self.frame_irq = True

        cdef uint32_t reset_at = region_apu_reset_at(self.region, self.frame_mode_5step)
        if self.frame_cycle >= reset_at:
            self.frame_cycle -= reset_at

    cdef void step(self, uint32_t cpu_cycles, Bus bus) nogil:
        self.step_frame_counter(cpu_cycles)
        cdef float apu_cycles_per_sample = region_cpu_cycles_per_sample(self.region) / 2.0
        self.cycle_accumulator += cpu_cycles
        while self.cycle_accumulator >= 2:
            self.cycle_accumulator -= 2
            self.pulse1.tick()
            self.pulse2.tick()
            self.triangle.tick()
            self.noise.tick()
            self.dmc.tick(bus)
            self.mix_accumulator += self.mix_raw()
            self.mix_count += 1
            self.sample_accumulator += 1.0
            if self.sample_accumulator >= apu_cycles_per_sample:
                self.sample_accumulator -= apu_cycles_per_sample
                if self.mix_count > 0:
                    self.last_decimated = self.mix_accumulator / <float>self.mix_count
                    self.mix_accumulator = 0.0
                    self.mix_count = 0

    cdef float scaled_sample(self, Py_ssize_t idx, uint8_t raw) nogil:
        if idx >= 5 or self.channel_muted[idx]:
            return 0.0
        return <float>raw * self.channel_volumes[idx]

    cdef float mix_raw(self) nogil:
        cdef float p1 = self.scaled_sample(0, self.pulse1.sample())
        cdef float p2 = self.scaled_sample(1, self.pulse2.sample())
        cdef float tri = self.scaled_sample(2, self.triangle.sample())
        cdef float noise = self.scaled_sample(3, self.noise.sample())
        cdef float dmc = self.scaled_sample(4, self.dmc.sample())
        cdef float pulse_sum = p1 + p2
        cdef float pulse_out = (95.52 / (8128.0 / pulse_sum + 100.0)) if pulse_sum > 0.0 else 0.0
        cdef float tnd_inner = tri / 8227.0 + noise / 12241.0 + dmc / 22638.0
        cdef float tnd_out = (163.67 / (1.0 / tnd_inner + 100.0)) if tnd_inner > 0.0 else 0.0
        cdef float mixed = (pulse_out + tnd_out) * 2.0 - 1.0
        if mixed < -1.0:
            mixed = -1.0
        if mixed > 1.0:
            mixed = 1.0
        return mixed

    cdef float output(self) nogil:
        cdef float clamped = self.last_decimated
        cdef float LPF_ALPHA = 0.8192
        cdef float filtered = LPF_ALPHA * clamped + (1.0 - LPF_ALPHA) * self.lpf_prev
        self.lpf_prev = filtered
        cdef float DC_R = 0.99715
        cdef float dc_out = filtered - self.dc_prev_x + DC_R * self.dc_prev_y
        self.dc_prev_x = filtered
        self.dc_prev_y = dc_out
        return dc_out


# ============================================================================
# PPU (ppu.c + ppu_render.c)
# ============================================================================
#
# Single cdef class holding all PPU state: registers, VRAM, OAM, palette,
# framebuffer, scroll latches, and the per-pixel render pipeline. CHR pattern
# bytes are owned by the cartridge and fetched through a callback passed into
# the render methods (matching the C core's ChrReader struct).

# Per-scanline sprite slot (render.rs ScanlineSprite).
cdef struct ScanlineSprite:
    uint8_t oam_i
    uint8_t y
    uint8_t tile
    uint8_t attr
    uint16_t sx

# Background fetch pipeline entry.
cdef struct BgFetch:
    uint8_t pattern
    uint8_t pal_select


# CHR reader callback type (matches ppu_render.h ChrReader.read_chr).
ctypedef uint8_t (*ChrReadFn)(void *ctx, uint16_t addr) nogil


cdef class Ppu:
    # Registers
    cdef uint8_t ppuctrl
    cdef uint8_t ppumask
    cdef uint8_t ppustatus
    cdef uint8_t oamaddr
    cdef uint8_t open_bus
    cdef uint8_t ppudata_buffer
    cdef uint16_t v
    cdef uint16_t t
    cdef uint8_t fine_x
    cdef bint w
    cdef bint nmi_request

    # Memory
    cdef uint8_t vram[0x1000]
    cdef uint32_t vram_size
    cdef uint8_t oam[256]
    cdef uint8_t palette[32]
    cdef uint8_t bg_pattern[256]
    cdef uint32_t framebuffer[256 * 240]

    # Timing
    cdef uint16_t scanline
    cdef uint16_t cycle
    cdef Region region
    cdef Mirroring mirroring

    # CHR cartridge reference (set by Bus; used by render pipeline instead of
    # a C function-pointer callback so nogil cdef method calls work).
    cdef Cartridge chr_cart

    # Render pipeline state (render.rs RenderState)
    cdef bint scanline_initialized
    cdef bint pipeline_primed
    cdef bint rendered_this_frame
    cdef bint sprite_zero_hit
    cdef bint v_dirty
    cdef bint slant_corruption
    cdef bint use_inaccurate_palette
    cdef bint nmi_retrigger
    cdef uint16_t coarse_y
    cdef uint16_t fine_y
    cdef uint16_t nt_v
    cdef uint16_t coarse_x_start
    cdef uint16_t nt_h_start
    cdef uint8_t fine_x_start
    cdef uint16_t resync_px
    cdef ScanlineSprite sprites[8]
    cdef uint8_t sprite_count
    cdef bint overflow
    cdef BgFetch fetch_buffer[2]
    cdef uint8_t fetch_idx

    def __cinit__(self):
        self.ppuctrl = 0
        self.ppumask = 0
        self.ppustatus = 0
        self.oamaddr = 0
        self.open_bus = 0
        self.ppudata_buffer = 0
        self.v = 0
        self.t = 0
        self.fine_x = 0
        self.w = False
        self.nmi_request = False
        self.vram_size = PPU_VRAM_SIZE_2K
        self.mirroring = MIRROR_HORIZONTAL
        self.region = REGION_NTSC
        self.scanline = 0
        self.cycle = 0
        self.scanline_initialized = False
        self.pipeline_primed = False
        self.rendered_this_frame = False
        self.sprite_zero_hit = False
        self.v_dirty = False
        self.slant_corruption = False
        self.use_inaccurate_palette = False
        self.nmi_retrigger = False
        self.coarse_y = 0
        self.fine_y = 0
        self.nt_v = 0
        self.coarse_x_start = 0
        self.nt_h_start = 0
        self.fine_x_start = 0
        self.resync_px = 0
        self.sprite_count = 0
        self.overflow = False
        self.fetch_idx = 0
        cdef int i
        for i in range(0x1000):
            self.vram[i] = 0
        for i in range(256):
            self.oam[i] = 0
            self.bg_pattern[i] = 0
        for i in range(32):
            self.palette[i] = 0
        for i in range(256 * 240):
            self.framebuffer[i] = 0
        self.sprites[0] = ScanlineSprite(0, 0, 0, 0, 0)
        self.sprites[1] = ScanlineSprite(0, 0, 0, 0, 0)
        self.sprites[2] = ScanlineSprite(0, 0, 0, 0, 0)
        self.sprites[3] = ScanlineSprite(0, 0, 0, 0, 0)
        self.sprites[4] = ScanlineSprite(0, 0, 0, 0, 0)
        self.sprites[5] = ScanlineSprite(0, 0, 0, 0, 0)
        self.sprites[6] = ScanlineSprite(0, 0, 0, 0, 0)
        self.sprites[7] = ScanlineSprite(0, 0, 0, 0, 0)
        self.fetch_buffer[0] = BgFetch(0, 0)
        self.fetch_buffer[1] = BgFetch(0, 0)

    # ---- Configuration ----
    cdef void set_mirroring(self, Mirroring m) nogil:
        cdef uint32_t needed = PPU_VRAM_SIZE_4K if m == MIRROR_FOUR_SCREEN else PPU_VRAM_SIZE_2K
        cdef uint32_t i
        if self.vram_size != needed:
            if needed > self.vram_size:
                for i in range(self.vram_size, needed):
                    self.vram[i] = 0
            self.vram_size = needed
        self.mirroring = m

    cdef void set_region(self, Region r) nogil:
        self.region = r
        cdef uint16_t max_sl = region_scanlines_per_frame(r)
        if self.scanline >= max_sl:
            self.scanline = max_sl - 1

    cdef void set_slant_corruption(self, bint enabled) nogil:
        self.slant_corruption = enabled

    cdef void set_inaccurate_palette(self, bint enabled) nogil:
        self.use_inaccurate_palette = enabled

    cdef void set_nmi_retrigger(self, bint enabled) nogil:
        self.nmi_retrigger = enabled

    # ---- Register access ----
    cdef uint8_t read_register(self, uint16_t reg) nogil:
        cdef uint8_t r = reg & 0x07
        if r == 2:
            return self._read_status()
        elif r == 4:
            return self._read_oamdata()
        elif r == 7:
            return self.ppudata_buffer
        return self.open_bus

    cdef void write_register(self, uint16_t reg, uint8_t value) nogil:
        self.open_bus = value
        cdef uint8_t r = reg & 0x07
        if r == 0:
            self._write_ppuctrl(value)
        elif r == 1:
            self.ppumask = value
        elif r == 3:
            self.oamaddr = value
        elif r == 4:
            self._write_oamdata(value)
        elif r == 5:
            self._write_ppuscroll(value)
        elif r == 6:
            self._write_ppuaddr(value)
        # r == 2: status read-only; r == 7: handled by bus

    cdef uint8_t _read_status(self) nogil:
        cdef uint8_t result = (self.ppustatus & PPU_STATUS_FLAG_MASK) | (self.open_bus & PPU_STATUS_OPEN_BUS_MASK)
        self.ppustatus &= <uint8_t>(~PPU_STATUS_VBLANK)
        self.w = False
        self.open_bus = result
        return result

    cdef uint8_t _read_oamdata(self) nogil:
        cdef uint8_t value = self.oam[self.oamaddr]
        self.oamaddr += 1
        self.open_bus = value
        return value

    cdef void _write_oamdata(self, uint8_t value) nogil:
        self.oam[self.oamaddr] = value
        self.oamaddr += 1

    cdef void _write_ppuctrl(self, uint8_t value) nogil:
        cdef bint nmi_was_enabled = (self.ppuctrl & PPU_CTRL_NMI) != 0
        self.ppuctrl = value
        cdef uint16_t nt = value & PPU_CTRL_BASE_NT_MASK
        self.t = (self.t & <uint16_t>(~PPU_NT_SELECT_MASK)) | (nt << 10)
        if (value & PPU_CTRL_NMI) != 0 and self._in_vblank():
            if self.nmi_retrigger or not nmi_was_enabled:
                self.nmi_request = True

    cdef void _write_ppuscroll(self, uint8_t value) nogil:
        if not self.w:
            self.t = (self.t & 0xFFE0) | (value >> 3)
            self.fine_x = value & 0x07
            self.w = True
            self.v_dirty = True
        else:
            self.t = (self.t & 0x8C1F) | ((<uint16_t>(value & 0xF8)) << 2) | ((<uint16_t>(value & 0x07)) << 12)
            self.w = False

    cdef void _write_ppuaddr(self, uint8_t value) nogil:
        if not self.w:
            self.t = (self.t & 0x00FF) | ((<uint16_t>(value & 0x3F)) << 8)
            self.w = True
        else:
            self.t = (self.t & 0xFF00) | <uint16_t>value
            self.v = self.t
            self.w = False

    # ---- PPUDATA accessors (used by bus) ----
    cdef inline uint16_t vram_addr(self) nogil:
        return self.v

    cdef inline uint16_t vram_increment(self) nogil:
        return 32 if (self.ppuctrl & PPU_CTRL_INCREMENT_32) != 0 else 1

    cdef void advance_vram_addr(self) nogil:
        cdef uint16_t inc = self.vram_increment()
        self.v = (self.v + inc) & 0x3FFF

    cdef inline uint8_t get_ppudata_buffer(self) nogil:
        return self.ppudata_buffer

    cdef inline void set_ppudata_buffer(self, uint8_t value) nogil:
        self.ppudata_buffer = value

    cdef inline uint8_t get_open_bus(self) nogil:
        return self.open_bus

    cdef inline void set_open_bus(self, uint8_t value) nogil:
        self.open_bus = value

    # ---- Nametable / palette mapping ----
    cdef uint16_t _map_nametable(self, uint16_t addr) nogil:
        cdef uint16_t a = addr & 0x2FFF
        cdef uint16_t local = a - 0x2000
        cdef uint16_t nt = local >> 10
        cdef uint16_t offset = local & 0x03FF
        cdef uint16_t phys
        if self.mirroring == MIRROR_HORIZONTAL:
            phys = nt >> 1
        elif self.mirroring == MIRROR_VERTICAL:
            phys = nt & 1
        elif self.mirroring == MIRROR_FOUR_SCREEN:
            phys = nt
        elif self.mirroring == MIRROR_SINGLE_SCREEN_0:
            phys = 0
        elif self.mirroring == MIRROR_SINGLE_SCREEN_1:
            phys = 1
        elif self.mirroring == MIRROR_SINGLE_SCREEN_2:
            phys = 2
        elif self.mirroring == MIRROR_SINGLE_SCREEN_3:
            phys = 3
        else:
            phys = nt >> 1
        return phys * 0x400 + offset

    cdef uint8_t read_nametable(self, uint16_t addr) nogil:
        return self.vram[self._map_nametable(addr)]

    cdef void write_nametable(self, uint16_t addr, uint8_t value) nogil:
        self.vram[self._map_nametable(addr)] = value

    cdef uint16_t _map_palette(self, uint16_t addr) nogil:
        cdef uint8_t a = addr & 0x1F
        if (a & 0x13) == 0x10:
            return a & 0x0F
        return a

    cdef uint8_t read_palette(self, uint16_t addr) nogil:
        return self.palette[self._map_palette(addr)]

    cdef void write_palette(self, uint16_t addr, uint8_t value) nogil:
        self.palette[self._map_palette(addr)] = value

    # ---- OAM DMA ----
    cdef void oam_dma(self, const uint8_t *data) nogil:
        cdef int i
        for i in range(256):
            self.oam[i] = data[i]
        self.oamaddr = 0

    # ---- VBlank / flag control ----
    cdef inline void set_vblank(self, bint on) nogil:
        if on:
            self.ppustatus |= PPU_STATUS_VBLANK
        else:
            self.ppustatus &= <uint8_t>(~PPU_STATUS_VBLANK)

    cdef inline void set_sprite_zero_hit(self, bint on) nogil:
        if on:
            self.ppustatus |= PPU_STATUS_SPRITE_ZERO
        else:
            self.ppustatus &= <uint8_t>(~PPU_STATUS_SPRITE_ZERO)

    cdef inline void set_sprite_overflow(self, bint on) nogil:
        if on:
            self.ppustatus |= PPU_STATUS_OVERFLOW
        else:
            self.ppustatus &= <uint8_t>(~PPU_STATUS_OVERFLOW)

    cdef inline bint nmi_enabled(self) nogil:
        return (self.ppuctrl & PPU_CTRL_NMI) != 0

    cdef inline bint _in_vblank(self) nogil:
        return (self.ppustatus & PPU_STATUS_VBLANK) != 0

    cdef inline bint is_rendering(self) nogil:
        return (self.ppumask & (PPU_MASK_SHOW_BG | PPU_MASK_SHOW_SPRITES)) != 0

    cdef bint take_nmi_request(self) nogil:
        cdef bint r = self.nmi_request
        self.nmi_request = False
        return r

    # ---- Scroll helpers ----
    cdef void _increment_h_scroll(self) nogil:
        cdef uint16_t coarse_x
        if self.slant_corruption:
            if self.fine_x < 7:
                self.fine_x += 1
                self.fine_x_start = (self.fine_x_start + 1) & 0x07
            else:
                self.fine_x = 0
                self.fine_x_start = 0
                coarse_x = self.v & PPU_COARSE_X_MASK
                if coarse_x == 31:
                    self.v &= <uint16_t>(~PPU_COARSE_X_MASK)
                    self.v ^= PPU_NT_H_BIT
                    self.coarse_x_start = 0
                    self.nt_h_start ^= PPU_NT_H_BIT
                else:
                    self.v = (self.v & <uint16_t>(~PPU_COARSE_X_MASK)) | (coarse_x + 1)
                    self.coarse_x_start = (coarse_x + 1) & PPU_COARSE_X_MASK
        else:
            coarse_x = self.v & PPU_COARSE_X_MASK
            if coarse_x == 31:
                self.v &= <uint16_t>(~PPU_COARSE_X_MASK)
                self.v ^= PPU_NT_H_BIT
            else:
                self.v = (self.v & <uint16_t>(~PPU_COARSE_X_MASK)) | (coarse_x + 1)

    cdef void _increment_v_scroll(self) nogil:
        cdef uint16_t fine_y = (self.v & PPU_FINE_Y_MASK) >> 12
        cdef uint16_t coarse_y
        if fine_y < 7:
            self.v = (self.v & <uint16_t>(~PPU_FINE_Y_MASK)) | ((fine_y + 1) << 12)
        else:
            self.v &= <uint16_t>(~PPU_FINE_Y_MASK)
            coarse_y = (self.v & PPU_COARSE_Y_MASK) >> 5
            if coarse_y == 29:
                self.v &= <uint16_t>(~PPU_COARSE_Y_MASK)
                self.v ^= PPU_NT_V_BIT
            elif coarse_y == 31:
                self.v &= <uint16_t>(~PPU_COARSE_Y_MASK)
            else:
                self.v = (self.v & <uint16_t>(~PPU_COARSE_Y_MASK)) | ((coarse_y + 1) << 5)

    cdef void _copy_h_t_to_v(self) nogil:
        cdef uint16_t h_bits = self.t & (PPU_COARSE_X_MASK | PPU_NT_H_BIT)
        self.v = (self.v & <uint16_t>(~(PPU_COARSE_X_MASK | PPU_NT_H_BIT))) | h_bits

    cdef void _copy_v_t_to_v(self) nogil:
        cdef uint16_t v_bits = self.t & (PPU_COARSE_Y_MASK | PPU_NT_V_BIT | PPU_FINE_Y_MASK)
        self.v = (self.v & <uint16_t>(~(PPU_COARSE_Y_MASK | PPU_NT_V_BIT | PPU_FINE_Y_MASK))) | v_bits

    # ---- Step (ppu.c ppu_step) ----
    cdef bint step(self) nogil:
        cdef bint nmi = False
        cdef uint16_t scanlines_per_frame = region_scanlines_per_frame(self.region)
        cdef uint16_t prerender = region_scanline_prerender(self.region)

        self.cycle += 1
        if self.cycle >= PPU_CYCLES_PER_SCANLINE:
            self.cycle = 0
            self.scanline += 1
            if self.scanline >= scanlines_per_frame:
                self.scanline = 0

        if self.scanline == PPU_SCANLINE_VBLANK_START and self.cycle == VBLANK_NMI_CYCLE:
            self.set_vblank(True)
            if self.nmi_enabled():
                self.nmi_request = True
                nmi = True
        elif self.scanline == prerender and self.cycle == VBLANK_NMI_CYCLE:
            self.set_vblank(False)
            self.set_sprite_overflow(False)
            self.set_sprite_zero_hit(False)
            self.sprite_zero_hit = False

        cdef bint rendering = self.is_rendering()
        cdef bint does_scroll_inc
        if rendering:
            does_scroll_inc = (self.scanline < PPU_SCREEN_HEIGHT) or (self.scanline == prerender)
            if does_scroll_inc and self.cycle >= H_SCROLL_INC_STEP and self.cycle <= H_SCROLL_INC_LAST and (self.cycle % H_SCROLL_INC_STEP) == 0:
                self._increment_h_scroll()
            if does_scroll_inc and self.cycle == VERT_SCROLL_INC_CYCLE:
                self._increment_v_scroll()
            if does_scroll_inc and self.cycle == H_COPY_CYCLE:
                self._copy_h_t_to_v()
            if self.scanline == prerender and self.cycle >= V_COPY_CYCLE_START and self.cycle <= V_COPY_CYCLE_END:
                self._copy_v_t_to_v()

        return nmi

    # ---- Step with rendering (ppu.c ppu_step_rendered) ----
    cdef bint step_rendered(self) nogil:
        cdef bint nmi = self.step()
        if self.cycle == 0:
            self.scanline_initialized = False
            self.pipeline_primed = False
        if self.scanline < PPU_SCREEN_HEIGHT and self.cycle >= 1 and self.cycle <= PPU_SCREEN_WIDTH:
            self._render_one_pixel()
            self.rendered_this_frame = True
        return nmi

    # ---- Color conversion (render.rs) ----
    cdef uint32_t _color_to_argb(self, uint8_t index) nogil:
        cdef uint32_t idx = index & 0x3F
        cdef uint8_t *c
        if region_is_pal_palette(self.region):
            c = PAL_PALETTE[idx]
        elif self.use_inaccurate_palette:
            c = NES_PALETTE_INACCURATE[idx]
        else:
            c = NES_PALETTE[idx]
        return (<uint32_t>ALPHA << 24) | (<uint32_t>c[0] << 16) | (<uint32_t>c[1] << 8) | <uint32_t>c[2]

    cdef uint32_t universal_bg_argb(self) nogil:
        return self._color_to_argb(self.read_palette(PAL_BASE))

    cdef void clear_framebuffer(self, uint32_t argb) nogil:
        cdef Py_ssize_t i
        for i in range(PPU_FRAMEBUFFER_SIZE):
            self.framebuffer[i] = argb

    cdef void reset_rendered_flag(self) nogil:
        self.rendered_this_frame = False

    # ---- Per-pixel renderer (render.rs render_one_pixel + helpers) ----
    cdef void _snapshot_render_position(self, Py_ssize_t px) nogil:
        self.coarse_y = (self.v >> 5) & 0x001F
        self.fine_y = (self.v >> 12) & 0x0007
        self.nt_v = self.v & PPU_NT_V_BIT
        self.coarse_x_start = self.v & PPU_COARSE_X_MASK
        self.nt_h_start = self.v & PPU_NT_H_BIT
        self.fine_x_start = self.fine_x
        self.resync_px = <uint16_t>px
        self.v_dirty = False

    cdef void _effective_render_x(self, Py_ssize_t px, uint16_t *out_coarse_x, uint16_t *out_nt_h, uint8_t *out_fine_x) nogil:
        cdef uint16_t coarse_x = self.coarse_x_start
        cdef uint16_t nt_h = self.nt_h_start
        cdef uint16_t fine_x_start = <uint16_t>self.fine_x_start
        cdef uint16_t advance = <uint16_t>px - self.resync_px
        cdef uint16_t new_fine_x = (fine_x_start + advance) & 0x0007
        cdef uint16_t cx_inc = (fine_x_start + advance) >> 3
        cdef uint32_t total = <uint32_t>coarse_x + <uint32_t>cx_inc
        cdef uint32_t wraps = total / 32
        coarse_x = total % 32
        if (wraps & 1) != 0:
            nt_h ^= PPU_NT_H_BIT
        out_coarse_x[0] = coarse_x
        out_nt_h[0] = nt_h
        out_fine_x[0] = <uint8_t>new_fine_x

    cdef BgFetch _fetch_bg_pixel(self, Py_ssize_t px) nogil:
        cdef uint16_t coarse_x, nt_h
        cdef uint8_t fine_x
        self._effective_render_x(px, &coarse_x, &nt_h, &fine_x)
        cdef uint16_t nt = ((nt_h >> 10) | (self.nt_v >> 10)) & 0x03
        cdef uint16_t coarse_y = self.coarse_y

        cdef uint16_t nt_addr = NT_BASE | (nt << 10) | (coarse_y << 5) | coarse_x
        cdef uint16_t tile_index = self.read_nametable(nt_addr)

        cdef uint16_t bg_table = 0x1000 if (self.ppuctrl & PPU_CTRL_BG_PATTERN_1000) else 0x0000
        cdef uint16_t pattern_addr = bg_table | (tile_index << 4) | self.fine_y
        cdef uint8_t plane0 = self.chr_cart.read_chr(pattern_addr) if self.chr_cart is not None else 0
        cdef uint8_t plane1 = self.chr_cart.read_chr(pattern_addr | 0x08) if self.chr_cart is not None else 0
        cdef uint16_t bit = 7 - fine_x
        cdef uint8_t pattern = ((plane0 >> bit) & 1) | (((plane1 >> bit) & 1) << 1)

        cdef uint16_t attr_col = coarse_x >> 2
        cdef uint16_t attr_row = coarse_y >> 2
        cdef uint16_t attr_addr = NT_BASE | (nt << 10) | ATTR_TABLE_OFFSET | (attr_row << 3) | attr_col
        cdef uint8_t attr_byte = self.read_nametable(attr_addr)
        cdef uint16_t shift = ((coarse_y & 0x02) << 1) | (coarse_x & 0x02)
        cdef uint8_t pal_select = (attr_byte >> shift) & 0x03

        return BgFetch(pattern, pal_select)

    cdef void _evaluate_scanline_sprites(self) nogil:
        self.sprite_count = 0
        self.overflow = False
        cdef bint sprite_size_16 = (self.ppuctrl & PPU_CTRL_SPRITE_SIZE_16) != 0
        cdef uint16_t sprite_height = SPRITE_HEIGHT_8X16 if sprite_size_16 else SPRITE_HEIGHT_8X8
        cdef uint16_t scanline = self.scanline
        cdef uint16_t i
        cdef uint16_t oam_idx
        cdef uint8_t y
        cdef uint16_t top
        cdef uint8_t idx
        for i in range(SPRITE_COUNT):
            oam_idx = i * 4
            y = self.oam[oam_idx]
            if y == OAM_Y_HALT:
                break
            if y >= OAM_Y_HIDDEN:
                continue
            top = y + 1
            if scanline < top or scanline >= top + sprite_height:
                continue
            if self.sprite_count < PPU_MAX_SPRITES_PER_SCANLINE:
                idx = self.sprite_count
                self.sprites[idx].oam_i = <uint8_t>i
                self.sprites[idx].y = y
                self.sprites[idx].tile = self.oam[oam_idx + 1]
                self.sprites[idx].attr = self.oam[oam_idx + 2]
                self.sprites[idx].sx = self.oam[oam_idx + 3]
                self.sprite_count = idx + 1
            else:
                self.overflow = True
        if self.overflow:
            self.set_sprite_overflow(True)

    cdef bint _fetch_sprite_pixel(self, Py_ssize_t px, Py_ssize_t py, uint8_t *out_pattern, uint8_t *out_pal) nogil:
        cdef bint sprite_size_16 = (self.ppuctrl & PPU_CTRL_SPRITE_SIZE_16) != 0
        cdef uint16_t sprite_height = SPRITE_HEIGHT_8X16 if sprite_size_16 else SPRITE_HEIGHT_8X8
        cdef uint16_t sprite_table_8x8 = 0x1000 if (self.ppuctrl & PPU_CTRL_SPRITE_PATTERN_1000) else 0x0000
        cdef uint16_t scanline = <uint16_t>py
        cdef bint bg_opaque = self.bg_pattern[px] != 0
        cdef uint8_t idx
        cdef uint8_t oam_i, y, tile, attr
        cdef uint16_t sx
        cdef uint8_t tile_col, tile_row, row, col
        cdef uint16_t table, tile_base
        cdef uint8_t row_in_tile
        cdef uint16_t tile_for_row, pattern_addr, bit
        cdef uint8_t plane0, plane1, pattern
        cdef bint behind_bg
        for idx in range(self.sprite_count):
            oam_i = self.sprites[idx].oam_i
            y = self.sprites[idx].y
            tile = self.sprites[idx].tile
            attr = self.sprites[idx].attr
            sx = self.sprites[idx].sx
            if <uint16_t>px < sx or <uint16_t>px >= sx + SPRITE_WIDTH:
                continue
            tile_col = <uint8_t>(<uint16_t>px - sx)
            tile_row = <uint8_t>(scanline - (y + 1))
            row = (<uint8_t>(sprite_height - 1) - tile_row) if (attr & ATTR_VFLIP) else tile_row
            col = (7 - tile_col) if (attr & ATTR_HFLIP) else tile_col
            if sprite_size_16:
                table = 0x1000 if (tile & 1) else 0x0000
                tile_base = tile & 0xFE
            else:
                table = sprite_table_8x8
                tile_base = tile
            row_in_tile = (row & 0x07) if sprite_size_16 else row
            tile_for_row = (tile_base + 1) if (sprite_size_16 and row >= 8) else tile_base
            pattern_addr = table | (tile_for_row << 4) | row_in_tile
            plane0 = self.chr_cart.read_chr(pattern_addr) if self.chr_cart is not None else 0
            plane1 = self.chr_cart.read_chr(pattern_addr | 0x08) if self.chr_cart is not None else 0
            bit = 7 - col
            pattern = ((plane0 >> bit) & 1) | (((plane1 >> bit) & 1) << 1)
            if pattern == 0:
                continue
            if not self.sprite_zero_hit and oam_i == 0 and <uint16_t>px < SPRITE_ZERO_HIT_MAX_X and bg_opaque:
                self.set_sprite_zero_hit(True)
                self.sprite_zero_hit = True
            behind_bg = (attr & ATTR_PRIORITY_BEHIND) != 0
            if behind_bg and bg_opaque:
                continue
            out_pattern[0] = pattern
            out_pal[0] = attr & ATTR_PALETTE_MASK
            return True
        return False

    cdef void _render_one_pixel(self) nogil:
        cdef Py_ssize_t px = <Py_ssize_t>(self.cycle - 1)
        cdef Py_ssize_t py = <Py_ssize_t>self.scanline
        cdef Py_ssize_t col = py * PPU_SCREEN_WIDTH + px

        if not self.scanline_initialized:
            self._snapshot_render_position(px)
            self._evaluate_scanline_sprites()
            self.scanline_initialized = True
        if self.v_dirty:
            self._snapshot_render_position(px)

        cdef bint bg_enabled = (self.ppumask & PPU_MASK_SHOW_BG) != 0
        cdef bint bg_left_enabled = (self.ppumask & PPU_MASK_SHOW_BG_LEFT) != 0
        cdef bint sprites_enabled = (self.ppumask & PPU_MASK_SHOW_SPRITES) != 0
        cdef bint sprites_left_enabled = (self.ppumask & PPU_MASK_SHOW_SPRITES_LEFT) != 0

        cdef uint32_t pixel_argb
        cdef BgFetch f0, f1, lookahead
        cdef uint8_t idx, pattern, pal_select
        cdef uint16_t color_addr
        cdef uint8_t nes_index
        cdef uint8_t sprite_pattern, sprite_pal

        if not bg_enabled:
            pixel_argb = self.universal_bg_argb()
            self.bg_pattern[px] = 0
            self.pipeline_primed = False
        else:
            if not self.pipeline_primed:
                f0 = self._fetch_bg_pixel(px)
                f1 = self._fetch_bg_pixel(px + 1)
                self.fetch_buffer[0] = f0
                self.fetch_buffer[1] = f1
                self.fetch_idx = 0
                self.pipeline_primed = True
            idx = self.fetch_idx
            pattern = self.fetch_buffer[idx].pattern
            pal_select = self.fetch_buffer[idx].pal_select
            lookahead = self._fetch_bg_pixel(px + 2)
            self.fetch_buffer[idx] = lookahead
            self.fetch_idx = idx ^ 1
            if px < 8 and not bg_left_enabled:
                pixel_argb = self.universal_bg_argb()
                self.bg_pattern[px] = 0
            else:
                self.bg_pattern[px] = pattern
                if pattern == 0:
                    color_addr = PAL_BASE
                else:
                    color_addr = PAL_BASE | ((<uint16_t>pal_select << 2) | <uint16_t>pattern)
                nes_index = self.read_palette(color_addr)
                pixel_argb = self._color_to_argb(nes_index)

        if sprites_enabled and (px >= 8 or sprites_left_enabled):
            if self._fetch_sprite_pixel(px, py, &sprite_pattern, &sprite_pal):
                color_addr = SPRITE_PAL_BASE | ((<uint16_t>sprite_pal << 2) | <uint16_t>sprite_pattern)
                nes_index = self.read_palette(color_addr)
                pixel_argb = self._color_to_argb(nes_index)

        self.framebuffer[col] = pixel_argb

    # ---- Whole-frame fallback renderer (render.rs render_frame) ----
    cdef void render_frame(self) nogil:
        self.set_sprite_overflow(False)
        self.set_sprite_zero_hit(False)
        cdef bint overflow_this_frame = False
        cdef bint sprite_zero_hit_set = False
        cdef uint16_t py
        for py in range(PPU_SCREEN_HEIGHT):
            self._render_background_scanline(py)
            self._render_sprites_scanline(py, &overflow_this_frame, &sprite_zero_hit_set)
        if overflow_this_frame:
            self.set_sprite_overflow(True)

    cdef void _render_background_scanline(self, uint16_t py) nogil:
        cdef bint bg_enabled = (self.ppumask & PPU_MASK_SHOW_BG) != 0
        cdef bint bg_left_enabled = (self.ppumask & PPU_MASK_SHOW_BG_LEFT) != 0
        cdef uint32_t universal_bg = self.universal_bg_argb()
        cdef Py_ssize_t row_base = <Py_ssize_t>py * PPU_SCREEN_WIDTH
        cdef Py_ssize_t px
        if not bg_enabled:
            for px in range(PPU_SCREEN_WIDTH):
                self.framebuffer[row_base + px] = universal_bg
                self.bg_pattern[px] = 0
            return
        cdef uint16_t bg_table = 0x1000 if (self.ppuctrl & PPU_CTRL_BG_PATTERN_1000) else 0x0000
        cdef uint16_t base_nt = <uint16_t>(self.ppuctrl & PPU_CTRL_BASE_NT_MASK) << 10
        cdef uint16_t coarse_x = self.t & 0x001F
        cdef uint16_t coarse_y = (self.t >> 5) & 0x001F
        cdef uint16_t fine_y = (self.t >> 12) & 0x0007
        cdef uint16_t scroll_x = (coarse_x << 3) | self.fine_x
        cdef uint16_t scroll_y = (coarse_y << 3) | fine_y
        cdef uint16_t gy = py + scroll_y
        fine_y = gy & 0x0007
        cdef uint16_t tile_row = (gy >> 3) & 0x001F
        cdef uint16_t nt_v = (gy >> 8) & 1
        cdef uint16_t gx, fine_x, tile_col, nt_h, nt, nt_addr, tile_index, pattern_addr, bit
        cdef uint8_t plane0, plane1, pattern, attr_byte, pal_select, nes_index
        cdef uint16_t attr_col, attr_row, attr_addr, shift, color_addr
        for px in range(PPU_SCREEN_WIDTH):
            if px < 8 and not bg_left_enabled:
                self.framebuffer[row_base + px] = universal_bg
                self.bg_pattern[px] = 0
                continue
            gx = px + scroll_x
            fine_x = gx & 0x0007
            tile_col = (gx >> 3) & 0x001F
            nt_h = (gx >> 8) & 1
            nt = ((base_nt >> 10) ^ nt_h ^ (nt_v << 1)) & 0x03
            nt_addr = NT_BASE | (nt << 10) | (tile_row << 5) | tile_col
            tile_index = self.read_nametable(nt_addr)
            pattern_addr = bg_table | (tile_index << 4) | fine_y
            plane0 = self.chr_cart.read_chr(pattern_addr) if self.chr_cart is not None else 0
            plane1 = self.chr_cart.read_chr(pattern_addr | 0x08) if self.chr_cart is not None else 0
            bit = 7 - fine_x
            pattern = ((plane0 >> bit) & 1) | (((plane1 >> bit) & 1) << 1)
            attr_col = tile_col >> 2
            attr_row = tile_row >> 2
            attr_addr = NT_BASE | (nt << 10) | ATTR_TABLE_OFFSET | (attr_row << 3) | attr_col
            attr_byte = self.read_nametable(attr_addr)
            shift = ((tile_row & 0x02) << 1) | (tile_col & 0x02)
            pal_select = (attr_byte >> shift) & 0x03
            self.bg_pattern[px] = pattern
            if pattern == 0:
                color_addr = PAL_BASE
            else:
                color_addr = PAL_BASE | ((<uint16_t>pal_select << 2) | <uint16_t>pattern)
            nes_index = self.read_palette(color_addr)
            self.framebuffer[row_base + px] = self._color_to_argb(nes_index)

    cdef void _render_sprites_scanline(self, uint16_t scanline, bint *overflow_this_frame, bint *sprite_zero_hit_set) nogil:
        cdef bint sprites_enabled = (self.ppumask & PPU_MASK_SHOW_SPRITES) != 0
        if not sprites_enabled:
            return
        cdef bint sprites_left_enabled = (self.ppumask & PPU_MASK_SHOW_SPRITES_LEFT) != 0
        cdef bint sprite_size_16 = (self.ppuctrl & PPU_CTRL_SPRITE_SIZE_16) != 0
        cdef uint16_t sprite_height = SPRITE_HEIGHT_8X16 if sprite_size_16 else SPRITE_HEIGHT_8X8
        cdef uint16_t sprite_table_8x8 = 0x1000 if (self.ppuctrl & PPU_CTRL_SPRITE_PATTERN_1000) else 0x0000
        cdef ScanlineSprite selected[8]
        cdef uint8_t count = 0
        cdef uint16_t i, oam_idx, top
        cdef uint8_t y
        for i in range(SPRITE_COUNT):
            oam_idx = i * 4
            y = self.oam[oam_idx]
            if y == OAM_Y_HALT:
                break
            if y >= OAM_Y_HIDDEN:
                continue
            top = y + 1
            if scanline < top or scanline >= top + sprite_height:
                continue
            if count < PPU_MAX_SPRITES_PER_SCANLINE:
                selected[count].oam_i = <uint8_t>i
                selected[count].y = y
                selected[count].tile = self.oam[oam_idx + 1]
                selected[count].attr = self.oam[oam_idx + 2]
                selected[count].sx = self.oam[oam_idx + 3]
                count += 1
            else:
                overflow_this_frame[0] = True
        cdef Py_ssize_t row_base = <Py_ssize_t>scanline * PPU_SCREEN_WIDTH
        cdef uint16_t px
        cdef uint8_t idx, oam_i, tile, attr, tile_col, tile_row, row, col
        cdef uint16_t sx, table, tile_base, pattern_addr, bit
        cdef uint8_t row_in_tile, plane0, plane1, pattern, pal, nes_index
        cdef uint16_t tile_for_row, color_addr
        cdef bint bg_opaque, behind_bg
        for px in range(PPU_SCREEN_WIDTH):
            if px < 8 and not sprites_left_enabled:
                continue
            for idx in range(count):
                oam_i = selected[idx].oam_i
                y = selected[idx].y
                tile = selected[idx].tile
                attr = selected[idx].attr
                sx = selected[idx].sx
                if px < sx or px >= sx + SPRITE_WIDTH:
                    continue
                tile_col = <uint8_t>(px - sx)
                tile_row = <uint8_t>(scanline - (y + 1))
                row = (<uint8_t>(sprite_height - 1) - tile_row) if (attr & ATTR_VFLIP) else tile_row
                col = (7 - tile_col) if (attr & ATTR_HFLIP) else tile_col
                if sprite_size_16:
                    table = 0x1000 if (tile & 1) else 0x0000
                    tile_base = tile & 0xFE
                else:
                    table = sprite_table_8x8
                    tile_base = tile
                row_in_tile = (row & 0x07) if sprite_size_16 else row
                tile_for_row = (tile_base + 1) if (sprite_size_16 and row >= 8) else tile_base
                pattern_addr = table | (tile_for_row << 4) | row_in_tile
                plane0 = self.chr_cart.read_chr(pattern_addr) if self.chr_cart is not None else 0
                plane1 = self.chr_cart.read_chr(pattern_addr | 0x08) if self.chr_cart is not None else 0
                bit = 7 - col
                pattern = ((plane0 >> bit) & 1) | (((plane1 >> bit) & 1) << 1)
                if pattern == 0:
                    continue
                bg_opaque = self.bg_pattern[px] != 0
                if not sprite_zero_hit_set[0] and oam_i == 0 and px < SPRITE_ZERO_HIT_MAX_X and bg_opaque:
                    self.set_sprite_zero_hit(True)
                    sprite_zero_hit_set[0] = True
                behind_bg = (attr & ATTR_PRIORITY_BEHIND) != 0
                if behind_bg and bg_opaque:
                    break
                pal = attr & ATTR_PALETTE_MASK
                color_addr = SPRITE_PAL_BASE | ((<uint16_t>pal << 2) | <uint16_t>pattern)
                nes_index = self.read_palette(color_addr)
                self.framebuffer[row_base + px] = self._color_to_argb(nes_index)
                break


# ============================================================================
# CPU (cpu.c + addressing.c + cpu_unofficial.c)
# ============================================================================
#
# 6502 core: registers, flags, stack, operand helpers, addressing modes, and
# the full official + unofficial opcode dispatch. Behaviour is byte-exact with
# the C reference. The CPU holds a reference to the Bus for memory access.

# Operand descriptor (cpu.h Operand).
cdef struct Operand:
    int tag       # OPERAND_NONE / OPERAND_ACC / OPERAND_ADDR
    uint16_t addr # valid when tag == OPERAND_ADDR

# RMW value-transform selector (replaces the C ValueFn function pointer).
cdef enum RmwOp:
    RMW_ASL = 0
    RMW_LSR = 1
    RMW_ROL = 2
    RMW_ROR = 3
    RMW_INC = 4
    RMW_DEC = 5


cdef class Cpu:
    cdef uint8_t a
    cdef uint8_t x
    cdef uint8_t y
    cdef uint8_t sp
    cdef uint16_t pc
    cdef uint8_t status
    cdef uint8_t flags  # packed NMI/IRQ/HALT

    def __cinit__(self):
        self.a = 0
        self.x = 0
        self.y = 0
        self.sp = 0xFD
        self.pc = 0
        self.status = CPU_FLAG_U | CPU_FLAG_I
        self.flags = 0

    # ---- Flag helpers ----
    cdef inline void set_flag(self, uint8_t flag, bint v) nogil:
        if v:
            self.status |= flag
        else:
            self.status &= <uint8_t>(~flag)

    cdef inline bint carry(self) nogil:
        return (self.status & CPU_FLAG_C) != 0

    cdef inline bint interrupt_disable(self) nogil:
        return (self.status & CPU_FLAG_I) != 0

    cdef inline void set_nz(self, uint8_t value) nogil:
        self.set_flag(CPU_FLAG_Z, value == 0)
        self.set_flag(CPU_FLAG_N, (value & 0x80) != 0)

    cdef inline bint is_halted(self) nogil:
        return (self.flags & CPU_HALTED) != 0

    cdef inline void set_halted(self, bint v) nogil:
        if v:
            self.flags |= CPU_HALTED
        else:
            self.flags &= <uint8_t>(~CPU_HALTED)

    cdef inline void set_nmi_pending(self, bint v) nogil:
        if v:
            self.flags |= CPU_NMI_PENDING
        else:
            self.flags &= <uint8_t>(~CPU_NMI_PENDING)

    cdef inline void set_irq_pending(self, bint v) nogil:
        if v:
            self.flags |= CPU_IRQ_PENDING
        else:
            self.flags &= <uint8_t>(~CPU_IRQ_PENDING)

    # ---- Fetch helpers ----
    cdef uint8_t fetch_byte(self, Bus bus) nogil:
        cdef uint8_t b = bus.read(self.pc)
        self.pc += 1
        return b

    cdef uint16_t fetch_word(self, Bus bus) nogil:
        cdef uint16_t lo = self.fetch_byte(bus)
        cdef uint16_t hi = self.fetch_byte(bus)
        return lo | (hi << 8)

    # ---- Stack ----
    cdef void push(self, Bus bus, uint8_t value) nogil:
        cdef uint16_t addr = 0x0100 | <uint16_t>self.sp
        bus.write(addr, value)
        self.sp -= 1

    cdef uint8_t pull(self, Bus bus) nogil:
        self.sp += 1
        cdef uint16_t addr = 0x0100 | <uint16_t>self.sp
        return bus.read(addr)

    cdef void push_pc(self, Bus bus, uint16_t pc) nogil:
        self.push(bus, <uint8_t>(pc >> 8))
        self.push(bus, <uint8_t>(pc & 0xFF))

    cdef uint16_t pull_pc(self, Bus bus) nogil:
        cdef uint16_t lo = self.pull(bus)
        cdef uint16_t hi = self.pull(bus)
        return lo | (hi << 8)

    cdef void push_status(self, Bus bus, bint with_break) nogil:
        cdef uint8_t p = self.status | CPU_FLAG_U
        if with_break:
            p |= CPU_FLAG_B
        else:
            p &= <uint8_t>(~CPU_FLAG_B)
        self.push(bus, p)

    cdef void pull_status(self, Bus bus) nogil:
        cdef uint8_t p = self.pull(bus)
        self.status = (p & <uint8_t>(~CPU_FLAG_B)) | CPU_FLAG_U

    # ---- Operand read/write ----
    cdef uint8_t read_operand(self, Bus bus, Operand op) nogil:
        if op.tag == OPERAND_ACC:
            return self.a
        elif op.tag == OPERAND_ADDR:
            return bus.read(op.addr)
        return 0

    cdef void write_operand(self, Bus bus, Operand op, uint8_t value) nogil:
        if op.tag == OPERAND_ACC:
            self.a = value
        elif op.tag == OPERAND_ADDR:
            bus.write(op.addr, value)

    # ---- Interrupts ----
    cdef uint16_t read_vector(self, Bus bus, uint16_t addr) nogil:
        cdef uint16_t lo = bus.read(addr)
        cdef uint16_t hi = bus.read(<uint16_t>(addr + 1))
        return lo | (hi << 8)

    cdef void _service_interrupt(self, Bus bus, uint16_t vector) nogil:
        self.push_pc(bus, self.pc)
        self.push_status(bus, False)
        self.set_flag(CPU_FLAG_I, True)
        self.pc = self.read_vector(bus, vector)

    cdef void nmi(self, Bus bus) nogil:
        self._service_interrupt(bus, CPU_VECTOR_NMI)

    cdef void irq(self, Bus bus) nogil:
        self._service_interrupt(bus, CPU_VECTOR_IRQ)

    cdef void reset(self, Bus bus) nogil:
        self.sp = 0xFD
        self.set_flag(CPU_FLAG_I, True)
        self.status |= CPU_FLAG_U
        self.pc = self.read_vector(bus, CPU_VECTOR_RESET)
        self.flags &= <uint8_t>(~CPU_HALTED)

    # ---- Addressing modes (addressing.c) ----
    cdef uint16_t am_immediate(self) nogil:
        cdef uint16_t addr = self.pc
        self.pc += 1
        return addr

    cdef uint16_t am_zero_page(self, Bus bus) nogil:
        return self.fetch_byte(bus)

    cdef uint16_t am_zero_page_x(self, Bus bus, Dummy dummy) nogil:
        cdef uint8_t base = self.fetch_byte(bus)
        if dummy != DUMMY_NONE:
            bus.read(<uint16_t>base)
        return <uint8_t>(base + self.x)

    cdef uint16_t am_zero_page_y(self, Bus bus, Dummy dummy) nogil:
        cdef uint8_t base = self.fetch_byte(bus)
        if dummy != DUMMY_NONE:
            bus.read(<uint16_t>base)
        return <uint8_t>(base + self.y)

    cdef uint16_t am_absolute(self, Bus bus) nogil:
        return self.fetch_word(bus)

    cdef uint16_t am_absolute_x(self, Bus bus, Dummy dummy, bint *page_cross) nogil:
        cdef uint16_t base = self.fetch_word(bus)
        cdef uint16_t eff = base + <uint16_t>self.x
        cdef bint cross = (base & 0xFF00) != (eff & 0xFF00)
        if (dummy == DUMMY_READ and cross) or dummy == DUMMY_RMW:
            bus.read((base & 0xFF00) | (eff & 0x00FF))
        page_cross[0] = cross
        return eff

    cdef uint16_t am_absolute_y(self, Bus bus, Dummy dummy, bint *page_cross) nogil:
        cdef uint16_t base = self.fetch_word(bus)
        cdef uint16_t eff = base + <uint16_t>self.y
        cdef bint cross = (base & 0xFF00) != (eff & 0xFF00)
        if (dummy == DUMMY_READ and cross) or dummy == DUMMY_RMW:
            bus.read((base & 0xFF00) | (eff & 0x00FF))
        page_cross[0] = cross
        return eff

    cdef uint16_t am_indirect(self, Bus bus) nogil:
        cdef uint16_t ptr = self.fetch_word(bus)
        cdef uint8_t lo = bus.read(ptr)
        cdef uint16_t hi_addr = (ptr & 0xFF00) | <uint8_t>(ptr + 1)
        cdef uint8_t hi = bus.read(hi_addr)
        return <uint16_t>lo | (<uint16_t>hi << 8)

    cdef uint16_t am_indirect_x(self, Bus bus) nogil:
        cdef uint8_t zp = self.fetch_byte(bus)
        cdef uint8_t ptr = <uint8_t>(zp + self.x)
        cdef uint8_t lo = bus.read(<uint16_t>ptr)
        cdef uint8_t hi = bus.read(<uint8_t>(ptr + 1))
        return <uint16_t>lo | (<uint16_t>hi << 8)

    cdef uint16_t am_indirect_y(self, Bus bus, Dummy dummy, bint *page_cross) nogil:
        cdef uint8_t zp = self.fetch_byte(bus)
        cdef uint8_t lo = bus.read(<uint16_t>zp)
        cdef uint8_t hi = bus.read(<uint8_t>(zp + 1))
        cdef uint16_t base = <uint16_t>lo | (<uint16_t>hi << 8)
        cdef uint16_t eff = base + <uint16_t>self.y
        cdef bint cross = (base & 0xFF00) != (eff & 0xFF00)
        if (dummy == DUMMY_READ and cross) or dummy == DUMMY_RMW:
            bus.read((base & 0xFF00) | (eff & 0x00FF))
        page_cross[0] = cross
        return eff

    cdef uint16_t am_relative(self, Bus bus) nogil:
        cdef int8_t offset = <int8_t>self.fetch_byte(bus)
        return self.pc + <uint16_t>(<int16_t>offset)

    cdef uint16_t am_indirect_y_base(self, Bus bus) nogil:
        cdef uint8_t zp = self.fetch_byte(bus)
        cdef uint8_t lo = bus.read(<uint16_t>zp)
        cdef uint8_t hi = bus.read(<uint8_t>(zp + 1))
        return <uint16_t>lo | (<uint16_t>hi << 8)

    # ---- Read helpers (opcodes.rs) ----
    cdef uint8_t rd_imm(self, Bus bus) nogil:
        return self.fetch_byte(bus)

    cdef uint8_t rd_zp(self, Bus bus) nogil:
        return bus.read(self.am_zero_page(bus))

    cdef uint8_t rd_zp_x(self, Bus bus) nogil:
        return bus.read(self.am_zero_page_x(bus, DUMMY_RMW))

    cdef uint8_t rd_zp_y(self, Bus bus) nogil:
        return bus.read(self.am_zero_page_y(bus, DUMMY_RMW))

    cdef uint8_t rd_abs(self, Bus bus) nogil:
        return bus.read(self.am_absolute(bus))

    cdef uint8_t rd_abs_x(self, Bus bus, bint *pc) nogil:
        return bus.read(self.am_absolute_x(bus, DUMMY_READ, pc))

    cdef uint8_t rd_abs_y(self, Bus bus, bint *pc) nogil:
        return bus.read(self.am_absolute_y(bus, DUMMY_READ, pc))

    cdef uint8_t rd_ind_x(self, Bus bus) nogil:
        return bus.read(self.am_indirect_x(bus))

    cdef uint8_t rd_ind_y(self, Bus bus, bint *pc) nogil:
        return bus.read(self.am_indirect_y(bus, DUMMY_READ, pc))

    # ---- Write helpers ----
    cdef void wr_zp(self, Bus bus, uint8_t v) nogil:
        bus.write(self.am_zero_page(bus), v)

    cdef void wr_zp_x(self, Bus bus, uint8_t v) nogil:
        bus.write(self.am_zero_page_x(bus, DUMMY_RMW), v)

    cdef void wr_zp_y(self, Bus bus, uint8_t v) nogil:
        bus.write(self.am_zero_page_y(bus, DUMMY_RMW), v)

    cdef void wr_abs(self, Bus bus, uint8_t v) nogil:
        bus.write(self.am_absolute(bus), v)

    cdef void wr_abs_x(self, Bus bus, uint8_t v) nogil:
        cdef bint pc
        bus.write(self.am_absolute_x(bus, DUMMY_RMW, &pc), v)

    cdef void wr_abs_y(self, Bus bus, uint8_t v) nogil:
        cdef bint pc
        bus.write(self.am_absolute_y(bus, DUMMY_RMW, &pc), v)

    cdef void wr_ind_x(self, Bus bus, uint8_t v) nogil:
        bus.write(self.am_indirect_x(bus), v)

    cdef void wr_ind_y(self, Bus bus, uint8_t v) nogil:
        cdef bint pc
        bus.write(self.am_indirect_y(bus, DUMMY_RMW, &pc), v)

    # ---- Operand resolvers for RMW-combo opcodes ----
    cdef Operand op_zp(self, Bus bus) nogil:
        return Operand(OPERAND_ADDR, self.am_zero_page(bus))

    cdef Operand op_zp_x(self, Bus bus) nogil:
        return Operand(OPERAND_ADDR, self.am_zero_page_x(bus, DUMMY_RMW))

    cdef Operand op_abs(self, Bus bus) nogil:
        return Operand(OPERAND_ADDR, self.am_absolute(bus))

    cdef Operand op_abs_x(self, Bus bus) nogil:
        cdef bint pc
        return Operand(OPERAND_ADDR, self.am_absolute_x(bus, DUMMY_RMW, &pc))

    cdef Operand op_abs_y(self, Bus bus) nogil:
        cdef bint pc
        return Operand(OPERAND_ADDR, self.am_absolute_y(bus, DUMMY_RMW, &pc))

    cdef Operand op_ind_x(self, Bus bus) nogil:
        return Operand(OPERAND_ADDR, self.am_indirect_x(bus))

    cdef Operand op_ind_y(self, Bus bus) nogil:
        cdef bint pc
        return Operand(OPERAND_ADDR, self.am_indirect_y(bus, DUMMY_RMW, &pc))

    # ---- Value transforms (opcodes.rs) ----
    cdef uint8_t rmw_value(self, RmwOp op, uint8_t v) nogil:
        cdef bint new_c, new_c2
        cdef uint8_t result, result2
        if op == RMW_ASL:
            self.set_flag(CPU_FLAG_C, (v & 0x80) != 0)
            return v << 1
        elif op == RMW_LSR:
            self.set_flag(CPU_FLAG_C, (v & 0x01) != 0)
            return v >> 1
        elif op == RMW_ROL:
            new_c = (v & 0x80) != 0
            result = (v << 1) | (1 if self.carry() else 0)
            self.set_flag(CPU_FLAG_C, new_c)
            return result
        elif op == RMW_ROR:
            new_c2 = (v & 0x01) != 0
            result2 = (v >> 1) | (0x80 if self.carry() else 0x00)
            self.set_flag(CPU_FLAG_C, new_c2)
            return result2
        elif op == RMW_INC:
            return v + 1
        elif op == RMW_DEC:
            return v - 1
        return v

    cdef void rmw(self, Bus bus, Operand op, RmwOp f) nogil:
        cdef uint8_t v = self.read_operand(bus, op)
        cdef uint8_t neu = self.rmw_value(f, v)
        self.write_operand(bus, op, neu)
        self.set_nz(neu)

    cdef void rmw_zp(self, Bus bus, RmwOp f) nogil:
        self.rmw(bus, self.op_zp(bus), f)

    cdef void rmw_zp_x(self, Bus bus, RmwOp f) nogil:
        self.rmw(bus, self.op_zp_x(bus), f)

    cdef void rmw_abs(self, Bus bus, RmwOp f) nogil:
        self.rmw(bus, self.op_abs(bus), f)

    cdef void rmw_abs_x(self, Bus bus, RmwOp f) nogil:
        self.rmw(bus, self.op_abs_x(bus), f)

    # ---- Load/store/transfer ----
    cdef void lda(self, uint8_t v) nogil:
        self.a = v
        self.set_nz(v)

    cdef void ldx(self, uint8_t v) nogil:
        self.x = v
        self.set_nz(v)

    cdef void ldy(self, uint8_t v) nogil:
        self.y = v
        self.set_nz(v)

    cdef void tax(self) nogil:
        self.x = self.a
        self.set_nz(self.x)

    cdef void tay(self) nogil:
        self.y = self.a
        self.set_nz(self.y)

    cdef void txa(self) nogil:
        self.a = self.x
        self.set_nz(self.a)

    cdef void tya(self) nogil:
        self.a = self.y
        self.set_nz(self.a)

    cdef void tsx(self) nogil:
        self.x = self.sp
        self.set_nz(self.x)

    cdef void set_sp_from_x(self) nogil:
        self.sp = self.x

    # ---- Logic ----
    cdef void and_(self, uint8_t v) nogil:
        self.a &= v
        self.set_nz(self.a)

    cdef void ora_(self, uint8_t v) nogil:
        self.a |= v
        self.set_nz(self.a)

    cdef void eor_(self, uint8_t v) nogil:
        self.a ^= v
        self.set_nz(self.a)

    cdef void bit_(self, uint8_t m) nogil:
        cdef uint8_t result = self.a & m
        self.set_flag(CPU_FLAG_Z, result == 0)
        self.set_flag(CPU_FLAG_N, (m & 0x80) != 0)
        self.set_flag(CPU_FLAG_V, (m & 0x40) != 0)

    # ---- Arithmetic ----
    cdef void adc(self, uint8_t m) nogil:
        cdef uint16_t a = self.a
        cdef uint16_t mm = m
        cdef uint16_t c = 1 if self.carry() else 0
        cdef uint16_t sum = a + mm + c
        self.set_flag(CPU_FLAG_C, sum > 0xFF)
        cdef uint8_t result = sum & 0xFF
        self.set_flag(CPU_FLAG_V, ((a ^ mm) & 0x80) == 0 and ((a ^ sum) & 0x80) != 0)
        self.a = result
        self.set_nz(result)

    cdef void sbc(self, uint8_t m) nogil:
        cdef uint16_t a = self.a
        cdef uint16_t mm = <uint8_t>(~m)
        cdef uint16_t c = 1 if self.carry() else 0
        cdef uint16_t sum = a + mm + c
        self.set_flag(CPU_FLAG_C, sum > 0xFF)
        cdef uint8_t result = sum & 0xFF
        self.set_flag(CPU_FLAG_V, ((a ^ mm) & 0x80) == 0 and ((a ^ sum) & 0x80) != 0)
        self.a = result
        self.set_nz(result)

    cdef void cmp_(self, uint8_t r, uint8_t m) nogil:
        cdef uint8_t diff = r - m
        self.set_flag(CPU_FLAG_C, r >= m)
        self.set_nz(diff)

    # ---- Branch ----
    cdef uint8_t branch(self, Bus bus, bint branch_on_set, uint8_t cond_flag) nogil:
        cdef bint flag_set = (self.status & cond_flag) != 0
        cdef bint take = (flag_set == branch_on_set)
        if not take:
            self.fetch_byte(bus)
            return 2
        cdef uint16_t pc_before = self.pc
        cdef uint16_t target = self.am_relative(bus)
        cdef uint16_t pc_after = pc_before + 1
        cdef bint page_cross = (pc_after & 0xFF00) != (target & 0xFF00)
        self.pc = target
        return 2 + 1 + (1 if page_cross else 0)

    # ---- Unofficial helpers ----
    cdef void lax(self, uint8_t m) nogil:
        self.a = m
        self.x = m
        self.set_nz(m)

    cdef void anc(self, uint8_t m) nogil:
        self.a &= m
        self.set_nz(self.a)
        self.set_flag(CPU_FLAG_C, (self.a & 0x80) != 0)

    cdef void alr(self, uint8_t m) nogil:
        cdef uint8_t v = self.a & m
        self.set_flag(CPU_FLAG_C, (v & 0x01) != 0)
        cdef uint8_t result = v >> 1
        self.a = result
        self.set_nz(result)

    cdef void arr(self, uint8_t m) nogil:
        cdef uint8_t v = self.a & m
        cdef uint8_t result = (v >> 1) | (0x80 if self.carry() else 0x00)
        self.a = result
        self.set_nz(result)
        self.set_flag(CPU_FLAG_C, (result & 0x40) != 0)
        self.set_flag(CPU_FLAG_V, ((result ^ <uint8_t>(result << 1)) & 0x40) != 0)

    cdef void axs(self, uint8_t m) nogil:
        cdef uint8_t ax = self.a & self.x
        self.set_flag(CPU_FLAG_C, ax >= m)
        cdef uint8_t result = ax - m
        self.x = result
        self.set_nz(result)

    cdef void xaa(self, uint8_t m) nogil:
        cdef uint8_t result = ((self.a | 0x00) & self.x) & m
        self.a = result
        self.set_nz(result)

    cdef uint16_t _store_dummy_read_addr(self, uint16_t base, uint8_t index) nogil:
        return (base & 0xFF00) | ((base + <uint16_t>index) & 0x00FF)

    cdef uint16_t _quirk_addr_y(self, uint16_t base, uint8_t y, uint8_t value) nogil:
        cdef uint16_t eff = base + <uint16_t>y
        if (base & 0xFF00) != (eff & 0xFF00):
            return (<uint16_t>value << 8) | (eff & 0x00FF)
        return eff

    cdef uint16_t _quirk_addr_x(self, uint16_t base, uint8_t x, uint8_t value) nogil:
        cdef uint16_t eff = base + <uint16_t>x
        if (base & 0xFF00) != (eff & 0xFF00):
            return (<uint16_t>value << 8) | (eff & 0x00FF)
        return eff

    cdef void tas_store(self, Bus bus, uint16_t base) nogil:
        self.sp = self.a & self.x
        cdef uint8_t h = <uint8_t>(base >> 8)
        cdef uint8_t value = self.sp & <uint8_t>(h + 1)
        cdef uint16_t store_addr = self._quirk_addr_y(base, self.y, value)
        bus.read(self._store_dummy_read_addr(base, self.y))
        bus.write(store_addr, value)

    cdef void ahx_store(self, Bus bus, uint16_t base, uint8_t reg) nogil:
        cdef uint8_t h = <uint8_t>(base >> 8)
        cdef uint8_t value = reg & <uint8_t>(h + 1)
        cdef uint16_t store_addr = self._quirk_addr_y(base, self.y, value)
        bus.read(self._store_dummy_read_addr(base, self.y))
        bus.write(store_addr, value)

    cdef void shy_store(self, Bus bus, uint16_t base) nogil:
        cdef uint8_t h = <uint8_t>(base >> 8)
        cdef uint8_t value = self.y & <uint8_t>(h + 1)
        cdef uint16_t store_addr = self._quirk_addr_x(base, self.x, value)
        bus.read(self._store_dummy_read_addr(base, self.x))
        bus.write(store_addr, value)

    # ---- RMW-combo opcodes ----
    cdef void dcp(self, Bus bus, Operand op) nogil:
        self.rmw(bus, op, RMW_DEC)
        cdef uint8_t m = self.read_operand(bus, op)
        self.cmp_(self.a, m)

    cdef void isc(self, Bus bus, Operand op) nogil:
        self.rmw(bus, op, RMW_INC)
        cdef uint8_t m = self.read_operand(bus, op)
        self.sbc(m)

    cdef void slo(self, Bus bus, Operand op) nogil:
        self.rmw(bus, op, RMW_ASL)
        cdef uint8_t m = self.read_operand(bus, op)
        self.ora_(m)

    cdef void rla(self, Bus bus, Operand op) nogil:
        self.rmw(bus, op, RMW_ROL)
        cdef uint8_t m = self.read_operand(bus, op)
        self.and_(m)

    cdef void sre(self, Bus bus, Operand op) nogil:
        self.rmw(bus, op, RMW_LSR)
        cdef uint8_t m = self.read_operand(bus, op)
        self.eor_(m)

    cdef void rra(self, Bus bus, Operand op) nogil:
        self.rmw(bus, op, RMW_ROR)
        cdef uint8_t m = self.read_operand(bus, op)
        self.adc(m)

    # ---- step (cpu.c cpu_step) ----
    cdef uint8_t step(self, Bus bus) nogil:
        if (self.flags & CPU_HALTED) != 0:
            return 1
        if (self.flags & CPU_NMI_PENDING) != 0:
            self.flags &= <uint8_t>(~CPU_NMI_PENDING)
            self.nmi(bus)
            return 7
        if (self.flags & CPU_IRQ_PENDING) != 0 and not self.interrupt_disable():
            self.flags &= <uint8_t>(~CPU_IRQ_PENDING)
            self.irq(bus)
            return 7
        cdef uint8_t opcode = self.fetch_byte(bus)
        return self.execute(bus, opcode)

    # ---- Execute dispatch (opcodes.rs) ----
    cdef uint8_t execute(self, Bus bus, uint8_t opcode) nogil:
        cdef bint pc = False
        cdef uint8_t v
        cdef uint16_t a, target, ret
        cdef Operand o

        if opcode == 0xEA:
            return 2
        # ---- LDA ----
        elif opcode == 0xA9: self.lda(self.rd_imm(bus)); return 2
        elif opcode == 0xA5: self.lda(self.rd_zp(bus)); return 3
        elif opcode == 0xB5: self.lda(self.rd_zp_x(bus)); return 4
        elif opcode == 0xAD: self.lda(self.rd_abs(bus)); return 4
        elif opcode == 0xBD: v = self.rd_abs_x(bus, &pc); self.lda(v); return 4 + (1 if pc else 0)
        elif opcode == 0xB9: v = self.rd_abs_y(bus, &pc); self.lda(v); return 4 + (1 if pc else 0)
        elif opcode == 0xA1: self.lda(self.rd_ind_x(bus)); return 6
        elif opcode == 0xB1: v = self.rd_ind_y(bus, &pc); self.lda(v); return 5 + (1 if pc else 0)
        # ---- LDX ----
        elif opcode == 0xA2: self.ldx(self.rd_imm(bus)); return 2
        elif opcode == 0xA6: self.ldx(self.rd_zp(bus)); return 3
        elif opcode == 0xB6: self.ldx(self.rd_zp_y(bus)); return 4
        elif opcode == 0xAE: self.ldx(self.rd_abs(bus)); return 4
        elif opcode == 0xBE: v = self.rd_abs_y(bus, &pc); self.ldx(v); return 4 + (1 if pc else 0)
        # ---- LDY ----
        elif opcode == 0xA0: self.ldy(self.rd_imm(bus)); return 2
        elif opcode == 0xA4: self.ldy(self.rd_zp(bus)); return 3
        elif opcode == 0xB4: self.ldy(self.rd_zp_x(bus)); return 4
        elif opcode == 0xAC: self.ldy(self.rd_abs(bus)); return 4
        elif opcode == 0xBC: v = self.rd_abs_x(bus, &pc); self.ldy(v); return 4 + (1 if pc else 0)
        # ---- STA ----
        elif opcode == 0x85: self.wr_zp(bus, self.a); return 3
        elif opcode == 0x95: self.wr_zp_x(bus, self.a); return 4
        elif opcode == 0x8D: self.wr_abs(bus, self.a); return 4
        elif opcode == 0x9D: self.wr_abs_x(bus, self.a); return 5
        elif opcode == 0x99: self.wr_abs_y(bus, self.a); return 5
        elif opcode == 0x81: self.wr_ind_x(bus, self.a); return 6
        elif opcode == 0x91: self.wr_ind_y(bus, self.a); return 6
        # ---- STX ----
        elif opcode == 0x86: self.wr_zp(bus, self.x); return 3
        elif opcode == 0x96: self.wr_zp_y(bus, self.x); return 4
        elif opcode == 0x8E: self.wr_abs(bus, self.x); return 4
        # ---- STY ----
        elif opcode == 0x84: self.wr_zp(bus, self.y); return 3
        elif opcode == 0x94: self.wr_zp_x(bus, self.y); return 4
        elif opcode == 0x8C: self.wr_abs(bus, self.y); return 4
        # ---- register transfers ----
        elif opcode == 0xAA: self.tax(); return 2
        elif opcode == 0xA8: self.tay(); return 2
        elif opcode == 0x8A: self.txa(); return 2
        elif opcode == 0x98: self.tya(); return 2
        elif opcode == 0xBA: self.tsx(); return 2
        elif opcode == 0x9A: self.set_sp_from_x(); return 2
        # ---- stack ----
        elif opcode == 0x48: self.push(bus, self.a); return 3
        elif opcode == 0x08: self.push_status(bus, True); return 3
        elif opcode == 0x68: v = self.pull(bus); self.lda(v); return 4
        elif opcode == 0x28: self.pull_status(bus); return 4
        # ---- AND ----
        elif opcode == 0x29: self.and_(self.rd_imm(bus)); return 2
        elif opcode == 0x25: self.and_(self.rd_zp(bus)); return 3
        elif opcode == 0x35: self.and_(self.rd_zp_x(bus)); return 4
        elif opcode == 0x2D: self.and_(self.rd_abs(bus)); return 4
        elif opcode == 0x3D: v = self.rd_abs_x(bus, &pc); self.and_(v); return 4 + (1 if pc else 0)
        elif opcode == 0x39: v = self.rd_abs_y(bus, &pc); self.and_(v); return 4 + (1 if pc else 0)
        elif opcode == 0x21: self.and_(self.rd_ind_x(bus)); return 6
        elif opcode == 0x31: v = self.rd_ind_y(bus, &pc); self.and_(v); return 5 + (1 if pc else 0)
        # ---- ORA ----
        elif opcode == 0x09: self.ora_(self.rd_imm(bus)); return 2
        elif opcode == 0x05: self.ora_(self.rd_zp(bus)); return 3
        elif opcode == 0x15: self.ora_(self.rd_zp_x(bus)); return 4
        elif opcode == 0x0D: self.ora_(self.rd_abs(bus)); return 4
        elif opcode == 0x1D: v = self.rd_abs_x(bus, &pc); self.ora_(v); return 4 + (1 if pc else 0)
        elif opcode == 0x19: v = self.rd_abs_y(bus, &pc); self.ora_(v); return 4 + (1 if pc else 0)
        elif opcode == 0x01: self.ora_(self.rd_ind_x(bus)); return 6
        elif opcode == 0x11: v = self.rd_ind_y(bus, &pc); self.ora_(v); return 5 + (1 if pc else 0)
        # ---- EOR ----
        elif opcode == 0x49: self.eor_(self.rd_imm(bus)); return 2
        elif opcode == 0x45: self.eor_(self.rd_zp(bus)); return 3
        elif opcode == 0x55: self.eor_(self.rd_zp_x(bus)); return 4
        elif opcode == 0x4D: self.eor_(self.rd_abs(bus)); return 4
        elif opcode == 0x5D: v = self.rd_abs_x(bus, &pc); self.eor_(v); return 4 + (1 if pc else 0)
        elif opcode == 0x59: v = self.rd_abs_y(bus, &pc); self.eor_(v); return 4 + (1 if pc else 0)
        elif opcode == 0x41: self.eor_(self.rd_ind_x(bus)); return 6
        elif opcode == 0x51: v = self.rd_ind_y(bus, &pc); self.eor_(v); return 5 + (1 if pc else 0)
        # ---- BIT ----
        elif opcode == 0x24: self.bit_(self.rd_zp(bus)); return 3
        elif opcode == 0x2C: self.bit_(self.rd_abs(bus)); return 4
        # ---- ADC ----
        elif opcode == 0x69: self.adc(self.rd_imm(bus)); return 2
        elif opcode == 0x65: self.adc(self.rd_zp(bus)); return 3
        elif opcode == 0x75: self.adc(self.rd_zp_x(bus)); return 4
        elif opcode == 0x6D: self.adc(self.rd_abs(bus)); return 4
        elif opcode == 0x7D: v = self.rd_abs_x(bus, &pc); self.adc(v); return 4 + (1 if pc else 0)
        elif opcode == 0x79: v = self.rd_abs_y(bus, &pc); self.adc(v); return 4 + (1 if pc else 0)
        elif opcode == 0x61: self.adc(self.rd_ind_x(bus)); return 6
        elif opcode == 0x71: v = self.rd_ind_y(bus, &pc); self.adc(v); return 5 + (1 if pc else 0)
        # ---- SBC ----
        elif opcode == 0xE9: self.sbc(self.rd_imm(bus)); return 2
        elif opcode == 0xE5: self.sbc(self.rd_zp(bus)); return 3
        elif opcode == 0xF5: self.sbc(self.rd_zp_x(bus)); return 4
        elif opcode == 0xED: self.sbc(self.rd_abs(bus)); return 4
        elif opcode == 0xFD: v = self.rd_abs_x(bus, &pc); self.sbc(v); return 4 + (1 if pc else 0)
        elif opcode == 0xF9: v = self.rd_abs_y(bus, &pc); self.sbc(v); return 4 + (1 if pc else 0)
        elif opcode == 0xE1: self.sbc(self.rd_ind_x(bus)); return 6
        elif opcode == 0xF1: v = self.rd_ind_y(bus, &pc); self.sbc(v); return 5 + (1 if pc else 0)
        # ---- CMP ----
        elif opcode == 0xC9: self.cmp_(self.a, self.rd_imm(bus)); return 2
        elif opcode == 0xC5: self.cmp_(self.a, self.rd_zp(bus)); return 3
        elif opcode == 0xD5: self.cmp_(self.a, self.rd_zp_x(bus)); return 4
        elif opcode == 0xCD: self.cmp_(self.a, self.rd_abs(bus)); return 4
        elif opcode == 0xDD: v = self.rd_abs_x(bus, &pc); self.cmp_(self.a, v); return 4 + (1 if pc else 0)
        elif opcode == 0xD9: v = self.rd_abs_y(bus, &pc); self.cmp_(self.a, v); return 4 + (1 if pc else 0)
        elif opcode == 0xC1: self.cmp_(self.a, self.rd_ind_x(bus)); return 6
        elif opcode == 0xD1: v = self.rd_ind_y(bus, &pc); self.cmp_(self.a, v); return 5 + (1 if pc else 0)
        # ---- CPX / CPY ----
        elif opcode == 0xE0: self.cmp_(self.x, self.rd_imm(bus)); return 2
        elif opcode == 0xE4: self.cmp_(self.x, self.rd_zp(bus)); return 3
        elif opcode == 0xEC: self.cmp_(self.x, self.rd_abs(bus)); return 4
        elif opcode == 0xC0: self.cmp_(self.y, self.rd_imm(bus)); return 2
        elif opcode == 0xC4: self.cmp_(self.y, self.rd_zp(bus)); return 3
        elif opcode == 0xCC: self.cmp_(self.y, self.rd_abs(bus)); return 4
        # ---- INC / DEC memory ----
        elif opcode == 0xE6: self.rmw_zp(bus, RMW_INC); return 5
        elif opcode == 0xF6: self.rmw_zp_x(bus, RMW_INC); return 6
        elif opcode == 0xEE: self.rmw_abs(bus, RMW_INC); return 6
        elif opcode == 0xFE: self.rmw_abs_x(bus, RMW_INC); return 7
        elif opcode == 0xC6: self.rmw_zp(bus, RMW_DEC); return 5
        elif opcode == 0xD6: self.rmw_zp_x(bus, RMW_DEC); return 6
        elif opcode == 0xCE: self.rmw_abs(bus, RMW_DEC); return 6
        elif opcode == 0xDE: self.rmw_abs_x(bus, RMW_DEC); return 7
        # ---- INX/INY/DEX/DEY ----
        elif opcode == 0xE8: self.x += 1; self.set_nz(self.x); return 2
        elif opcode == 0xC8: self.y += 1; self.set_nz(self.y); return 2
        elif opcode == 0xCA: self.x -= 1; self.set_nz(self.x); return 2
        elif opcode == 0x88: self.y -= 1; self.set_nz(self.y); return 2
        # ---- ASL ----
        elif opcode == 0x0A: self.a = self.rmw_value(RMW_ASL, self.a); self.set_nz(self.a); return 2
        elif opcode == 0x06: self.rmw_zp(bus, RMW_ASL); return 5
        elif opcode == 0x16: self.rmw_zp_x(bus, RMW_ASL); return 6
        elif opcode == 0x0E: self.rmw_abs(bus, RMW_ASL); return 6
        elif opcode == 0x1E: self.rmw_abs_x(bus, RMW_ASL); return 7
        # ---- LSR ----
        elif opcode == 0x4A: self.a = self.rmw_value(RMW_LSR, self.a); self.set_nz(self.a); return 2
        elif opcode == 0x46: self.rmw_zp(bus, RMW_LSR); return 5
        elif opcode == 0x56: self.rmw_zp_x(bus, RMW_LSR); return 6
        elif opcode == 0x4E: self.rmw_abs(bus, RMW_LSR); return 6
        elif opcode == 0x5E: self.rmw_abs_x(bus, RMW_LSR); return 7
        # ---- ROL ----
        elif opcode == 0x2A: self.a = self.rmw_value(RMW_ROL, self.a); self.set_nz(self.a); return 2
        elif opcode == 0x26: self.rmw_zp(bus, RMW_ROL); return 5
        elif opcode == 0x36: self.rmw_zp_x(bus, RMW_ROL); return 6
        elif opcode == 0x2E: self.rmw_abs(bus, RMW_ROL); return 6
        elif opcode == 0x3E: self.rmw_abs_x(bus, RMW_ROL); return 7
        # ---- ROR ----
        elif opcode == 0x6A: self.a = self.rmw_value(RMW_ROR, self.a); self.set_nz(self.a); return 2
        elif opcode == 0x66: self.rmw_zp(bus, RMW_ROR); return 5
        elif opcode == 0x76: self.rmw_zp_x(bus, RMW_ROR); return 6
        elif opcode == 0x6E: self.rmw_abs(bus, RMW_ROR); return 6
        elif opcode == 0x7E: self.rmw_abs_x(bus, RMW_ROR); return 7
        # ---- branches ----
        elif opcode == 0x10: return self.branch(bus, False, CPU_FLAG_N)
        elif opcode == 0x30: return self.branch(bus, True, CPU_FLAG_N)
        elif opcode == 0x50: return self.branch(bus, False, CPU_FLAG_V)
        elif opcode == 0x70: return self.branch(bus, True, CPU_FLAG_V)
        elif opcode == 0x90: return self.branch(bus, False, CPU_FLAG_C)
        elif opcode == 0xB0: return self.branch(bus, True, CPU_FLAG_C)
        elif opcode == 0xD0: return self.branch(bus, False, CPU_FLAG_Z)
        elif opcode == 0xF0: return self.branch(bus, True, CPU_FLAG_Z)
        # ---- JMP / JSR / RTS / RTI / BRK ----
        elif opcode == 0x4C: a = self.am_absolute(bus); self.pc = a; return 3
        elif opcode == 0x6C: a = self.am_indirect(bus); self.pc = a; return 5
        elif opcode == 0x20:
            target = self.fetch_word(bus)
            self.push_pc(bus, self.pc - 1)
            self.pc = target
            return 6
        elif opcode == 0x60:
            ret = self.pull_pc(bus) + 1
            self.pc = ret
            return 6
        elif opcode == 0x40:
            self.pull_status(bus)
            self.pc = self.pull_pc(bus)
            return 6
        elif opcode == 0x00:
            self.fetch_byte(bus)
            self.push_pc(bus, self.pc)
            self.push_status(bus, True)
            self.set_flag(CPU_FLAG_I, True)
            self.pc = self.read_vector(bus, CPU_VECTOR_IRQ)
            return 7
        # ---- flag operations ----
        elif opcode == 0x18: self.set_flag(CPU_FLAG_C, False); return 2
        elif opcode == 0x38: self.set_flag(CPU_FLAG_C, True); return 2
        elif opcode == 0x58: self.set_flag(CPU_FLAG_I, False); return 2
        elif opcode == 0x78: self.set_flag(CPU_FLAG_I, True); return 2
        elif opcode == 0xB8: self.set_flag(CPU_FLAG_V, False); return 2
        elif opcode == 0xD8: self.set_flag(CPU_FLAG_D, False); return 2
        elif opcode == 0xF8: self.set_flag(CPU_FLAG_D, True); return 2
        else:
            return self.execute_unofficial(bus, opcode)

    # ---- Unofficial opcodes (cpu_unofficial.c) ----
    cdef uint8_t execute_unofficial(self, Bus bus, uint8_t opcode) nogil:
        cdef bint pc = False
        cdef uint8_t v, r
        cdef uint16_t base
        cdef Operand o

        # ---- NOP variants ----
        if opcode in (0x1A, 0x3A, 0x5A, 0x7A, 0xDA, 0xFA):
            return 2
        elif opcode in (0x80, 0x82, 0x89, 0xC2, 0xE2):
            self.fetch_byte(bus)
            return 2
        elif opcode in (0x04, 0x44, 0x64):
            self.fetch_byte(bus)
            return 3
        elif opcode in (0x14, 0x34, 0x54, 0x74, 0xD4, 0xF4):
            self.am_zero_page_x(bus, DUMMY_RMW)
            return 4
        elif opcode == 0x0C:
            self.fetch_word(bus)
            return 4
        elif opcode in (0x1C, 0x3C, 0x5C, 0x7C, 0xDC, 0xFC):
            self.am_absolute_x(bus, DUMMY_READ, &pc)
            return 4 + (1 if pc else 0)
        # ---- LAX ----
        elif opcode == 0xA7: self.lax(self.rd_zp(bus)); return 3
        elif opcode == 0xB7: self.lax(self.rd_zp_y(bus)); return 4
        elif opcode == 0xAF: self.lax(self.rd_abs(bus)); return 4
        elif opcode == 0xBF: v = self.rd_abs_y(bus, &pc); self.lax(v); return 4 + (1 if pc else 0)
        elif opcode == 0xA3: self.lax(self.rd_ind_x(bus)); return 6
        elif opcode == 0xB3: v = self.rd_ind_y(bus, &pc); self.lax(v); return 5 + (1 if pc else 0)
        # ---- SAX ----
        elif opcode == 0x87: self.wr_zp(bus, self.a & self.x); return 3
        elif opcode == 0x97: self.wr_zp_y(bus, self.a & self.x); return 4
        elif opcode == 0x8F: self.wr_abs(bus, self.a & self.x); return 4
        elif opcode == 0x83: self.wr_ind_x(bus, self.a & self.x); return 6
        # ---- ANC ----
        elif opcode == 0x0B or opcode == 0x2B: self.anc(self.rd_imm(bus)); return 2
        # ---- ALR ----
        elif opcode == 0x4B: self.alr(self.rd_imm(bus)); return 2
        # ---- ARR ----
        elif opcode == 0x6B: self.arr(self.rd_imm(bus)); return 2
        # ---- AXS / SBX ----
        elif opcode == 0xCB: self.axs(self.rd_imm(bus)); return 2
        # ---- XAA (unstable) ----
        elif opcode == 0x8B: self.xaa(self.rd_imm(bus)); return 2
        # ---- LAS / LAR ----
        elif opcode == 0xBB:
            v = self.rd_abs_y(bus, &pc)
            r = v & self.sp
            self.a = r
            self.x = r
            self.sp = r
            self.set_nz(r)
            return 4 + (1 if pc else 0)
        # ---- KIL / JAM / HLT ----
        elif opcode in (0x02, 0x12, 0x22, 0x32, 0x42, 0x52, 0x62, 0x72, 0x92, 0xB2, 0xD2, 0xF2):
            self.set_halted(True)
            return 1
        else:
            return self.execute_unofficial_rmw(bus, opcode)

    cdef uint8_t execute_unofficial_rmw(self, Bus bus, uint8_t opcode) nogil:
        cdef Operand o
        cdef uint16_t base
        # ---- DCP ----
        if opcode == 0xC7: o = self.op_zp(bus); self.dcp(bus, o); return 5
        elif opcode == 0xD7: o = self.op_zp_x(bus); self.dcp(bus, o); return 6
        elif opcode == 0xCF: o = self.op_abs(bus); self.dcp(bus, o); return 6
        elif opcode == 0xDF: o = self.op_abs_x(bus); self.dcp(bus, o); return 7
        elif opcode == 0xDB: o = self.op_abs_y(bus); self.dcp(bus, o); return 7
        elif opcode == 0xC3: o = self.op_ind_x(bus); self.dcp(bus, o); return 8
        elif opcode == 0xD3: o = self.op_ind_y(bus); self.dcp(bus, o); return 8
        # ---- ISC ----
        elif opcode == 0xE7: o = self.op_zp(bus); self.isc(bus, o); return 5
        elif opcode == 0xF7: o = self.op_zp_x(bus); self.isc(bus, o); return 6
        elif opcode == 0xEF: o = self.op_abs(bus); self.isc(bus, o); return 6
        elif opcode == 0xFF: o = self.op_abs_x(bus); self.isc(bus, o); return 7
        elif opcode == 0xFB: o = self.op_abs_y(bus); self.isc(bus, o); return 7
        elif opcode == 0xE3: o = self.op_ind_x(bus); self.isc(bus, o); return 8
        elif opcode == 0xF3: o = self.op_ind_y(bus); self.isc(bus, o); return 8
        # ---- SLO ----
        elif opcode == 0x07: o = self.op_zp(bus); self.slo(bus, o); return 5
        elif opcode == 0x17: o = self.op_zp_x(bus); self.slo(bus, o); return 6
        elif opcode == 0x0F: o = self.op_abs(bus); self.slo(bus, o); return 6
        elif opcode == 0x1F: o = self.op_abs_x(bus); self.slo(bus, o); return 7
        elif opcode == 0x1B: o = self.op_abs_y(bus); self.slo(bus, o); return 7
        elif opcode == 0x03: o = self.op_ind_x(bus); self.slo(bus, o); return 8
        elif opcode == 0x13: o = self.op_ind_y(bus); self.slo(bus, o); return 8
        # ---- RLA ----
        elif opcode == 0x27: o = self.op_zp(bus); self.rla(bus, o); return 5
        elif opcode == 0x37: o = self.op_zp_x(bus); self.rla(bus, o); return 6
        elif opcode == 0x2F: o = self.op_abs(bus); self.rla(bus, o); return 6
        elif opcode == 0x3F: o = self.op_abs_x(bus); self.rla(bus, o); return 7
        elif opcode == 0x3B: o = self.op_abs_y(bus); self.rla(bus, o); return 7
        elif opcode == 0x23: o = self.op_ind_x(bus); self.rla(bus, o); return 8
        elif opcode == 0x33: o = self.op_ind_y(bus); self.rla(bus, o); return 8
        # ---- SRE ----
        elif opcode == 0x47: o = self.op_zp(bus); self.sre(bus, o); return 5
        elif opcode == 0x57: o = self.op_zp_x(bus); self.sre(bus, o); return 6
        elif opcode == 0x4F: o = self.op_abs(bus); self.sre(bus, o); return 6
        elif opcode == 0x5F: o = self.op_abs_x(bus); self.sre(bus, o); return 7
        elif opcode == 0x5B: o = self.op_abs_y(bus); self.sre(bus, o); return 7
        elif opcode == 0x43: o = self.op_ind_x(bus); self.sre(bus, o); return 8
        elif opcode == 0x53: o = self.op_ind_y(bus); self.sre(bus, o); return 8
        # ---- RRA ----
        elif opcode == 0x67: o = self.op_zp(bus); self.rra(bus, o); return 5
        elif opcode == 0x77: o = self.op_zp_x(bus); self.rra(bus, o); return 6
        elif opcode == 0x6F: o = self.op_abs(bus); self.rra(bus, o); return 6
        elif opcode == 0x7F: o = self.op_abs_x(bus); self.rra(bus, o); return 7
        elif opcode == 0x7B: o = self.op_abs_y(bus); self.rra(bus, o); return 7
        elif opcode == 0x63: o = self.op_ind_x(bus); self.rra(bus, o); return 8
        elif opcode == 0x73: o = self.op_ind_y(bus); self.rra(bus, o); return 8
        # ---- TAS / SHS ----
        elif opcode == 0x9B:
            base = self.fetch_word(bus)
            self.tas_store(bus, base)
            return 5
        # ---- AHX / SHA ----
        elif opcode == 0x9F:
            base = self.fetch_word(bus)
            self.ahx_store(bus, base, self.a & self.x)
            return 5
        elif opcode == 0x93:
            base = self.am_indirect_y_base(bus)
            self.ahx_store(bus, base, self.a & self.x)
            return 6
        # ---- SHX / SXA ----
        elif opcode == 0x9E:
            base = self.fetch_word(bus)
            self.ahx_store(bus, base, self.x)
            return 5
        # ---- SHY / SYA ----
        elif opcode == 0x9C:
            base = self.fetch_word(bus)
            self.shy_store(bus, base)
            return 5
        else:
            return 2


# ============================================================================
# Bus (bus.c)
# ============================================================================
#
# CPU memory bus: 2 KB RAM, PPU register routing ($2000-$3FFF), APU/IO
# register routing ($4000-$4017) with open-bus latch, OAM-DMA ($4014), and
# cartridge space ($4020-$FFFF). PPUDATA (reg 7) is handled here for CHR
# routing through the cartridge. Owns the Ppu, Apu, and Joypad instances.

cdef class Bus:
    cdef uint8_t ram[0x0800]
    cdef uint8_t apu_open_bus[0x18]
    cdef Ppu ppu
    cdef Apu apu
    cdef Joypad joypad
    cdef Cartridge cartridge
    cdef uint32_t dma_stall_cycles
    cdef uint64_t cpu_cycle_count

    def __cinit__(self):
        cdef int i
        for i in range(0x0800):
            self.ram[i] = 0
        for i in range(0x18):
            self.apu_open_bus[i] = 0
        self.ppu = Ppu()
        self.apu = Apu()
        self.joypad = Joypad()
        self.cartridge = None
        self.dma_stall_cycles = 0
        self.cpu_cycle_count = 0

    # ---- Cartridge management ----
    cdef void insert_cartridge(self, Cartridge cartridge):
        self.cartridge = cartridge
        if cartridge is not None:
            self.ppu.set_mirroring(cartridge.mirror_mode())
            self.ppu.chr_cart = cartridge

    cdef Cartridge remove_cartridge(self):
        cdef Cartridge prev = self.cartridge
        self.cartridge = None
        self.ppu.chr_cart = None
        return prev

    cdef Cartridge get_cartridge(self):
        return self.cartridge

    # ---- Internal helpers (bus.c static functions) ----
    cdef uint8_t _ppu_read_ppudata(self) nogil:
        cdef uint16_t addr = self.ppu.vram_addr()
        cdef uint8_t val, pal, buffered_fill, buffered
        cdef uint16_t nt_addr
        cdef uint8_t raw

        if addr >= 0x3F00:
            pal = self.ppu.read_palette(addr)
            val = (pal & 0x3F) | (self.ppu.get_open_bus() & 0xC0)
            nt_addr = addr & 0x2FFF
            if nt_addr < 0x2000:
                buffered_fill = self.cartridge.read_chr(nt_addr) if self.cartridge is not None else 0
            else:
                buffered_fill = self.ppu.read_nametable(nt_addr)
            self.ppu.set_ppudata_buffer(buffered_fill)
            self.ppu.advance_vram_addr()
            self.ppu.set_open_bus(val)
            return val

        buffered = self.ppu.get_ppudata_buffer()
        if addr < 0x2000:
            raw = self.cartridge.read_chr(addr) if self.cartridge is not None else 0
        else:
            raw = self.ppu.read_nametable(addr)
        self.ppu.set_ppudata_buffer(raw)
        self.ppu.advance_vram_addr()
        self.ppu.set_open_bus(buffered)
        return buffered

    cdef void _ppu_write_ppudata(self, uint8_t value) nogil:
        cdef uint16_t addr = self.ppu.vram_addr()
        if addr >= 0x3F00:
            self.ppu.write_palette(addr, value)
        elif addr < 0x2000:
            if self.cartridge is not None:
                self.cartridge.write_chr(addr, value)
        else:
            self.ppu.write_nametable(addr, value)
        self.ppu.write_register(7, value)
        self.ppu.advance_vram_addr()

    cdef uint8_t _ppu_read(self, uint16_t reg) nogil:
        if (reg & 0x07) == 7:
            return self._ppu_read_ppudata()
        return self.ppu.read_register(reg)

    cdef void _ppu_write(self, uint16_t reg, uint8_t value) nogil:
        if (reg & 0x07) == 7:
            self._ppu_write_ppudata(value)
            return
        self.ppu.write_register(reg, value)

    cdef uint8_t _apu_read_open_bus(self, uint16_t offset) nogil:
        return self.apu_open_bus[offset]

    cdef void _apu_pulse_write(self, uint16_t offset, uint8_t value) nogil:
        if offset < 4:
            self.apu.pulse1.write_register(<uint8_t>offset, value)
        else:
            self.apu.pulse2.write_register(<uint8_t>(offset - 4), value)

    cdef void _apu_triangle_write(self, uint16_t offset, uint8_t value) nogil:
        self.apu.triangle.write_register(<uint8_t>(offset - 0x08), value)

    cdef void _apu_noise_write(self, uint16_t offset, uint8_t value) nogil:
        self.apu.noise.write_register(<uint8_t>(offset - 0x0C), value)

    cdef void _apu_dmc_write(self, uint16_t offset, uint8_t value) nogil:
        self.apu.dmc.write_register(<uint8_t>(offset - 0x10), value)

    cdef uint8_t _apu_status_read(self) nogil:
        cdef uint8_t status = self.apu.read_status()
        return (status & 0xDF) | (self.apu_open_bus[0x15] & 0x20)

    cdef void _oam_dma(self, uint8_t page) nogil:
        cdef uint16_t base = <uint16_t>page << 8
        cdef uint8_t data[256]
        cdef uint16_t i
        cdef uint32_t stall
        cdef uint32_t max_u32 = <uint32_t>0xFFFFFFFF
        for i in range(256):
            data[i] = self.read(<uint16_t>(base + i))
        self.ppu.oam_dma(data)
        self.apu_open_bus[0x14] = page
        self.ppu.set_open_bus(page)
        stall = 513 if (self.cpu_cycle_count & 1) else 512
        if self.dma_stall_cycles > max_u32 - stall:
            self.dma_stall_cycles = max_u32
        else:
            self.dma_stall_cycles = self.dma_stall_cycles + stall

    cdef uint8_t _cart_read(self, uint16_t addr) nogil:
        if self.cartridge is not None:
            return self.cartridge.read_prg_mut(addr)
        return 0

    cdef void _cart_write(self, uint16_t addr, uint8_t value) nogil:
        if self.cartridge is not None:
            self.cartridge.write_prg(addr, value)
            self.ppu.set_mirroring(self.cartridge.mirror_mode())

    # ---- bus_read (bus.c bus_read) ----
    cdef uint8_t read(self, uint16_t addr) nogil:
        cdef uint16_t offset
        cdef uint8_t ob, jb
        if addr <= 0x1FFF:
            return self.ram[addr & BUS_RAM_MASK]
        if addr <= 0x3FFF:
            return self._ppu_read(addr & BUS_PPU_REG_MASK)
        if addr <= 0x4007:
            return self._apu_read_open_bus(<uint16_t>(addr - BUS_APU_IO_BASE))
        if addr <= 0x400B:
            return self._apu_read_open_bus(<uint16_t>(addr - BUS_APU_IO_BASE))
        if addr <= 0x400F:
            return self._apu_read_open_bus(<uint16_t>(addr - BUS_APU_IO_BASE))
        if addr <= 0x4013:
            return self._apu_read_open_bus(<uint16_t>(addr - BUS_APU_IO_BASE))
        if addr == 0x4014:
            return self._apu_read_open_bus(0x14)
        if addr == 0x4015:
            return self._apu_status_read()
        if addr == 0x4016:
            ob = self._apu_read_open_bus(0x16)
            jb = self.joypad.read(0)
            return (jb & 0x01) | (ob & 0xFE)
        if addr == 0x4017:
            ob = self._apu_read_open_bus(0x17)
            jb = self.joypad.read(1)
            return (jb & 0x01) | (ob & 0xFE)
        if addr <= 0x401F:
            return 0
        return self._cart_read(addr)

    # ---- bus_write (bus.c bus_write) ----
    cdef void write(self, uint16_t addr, uint8_t value) nogil:
        cdef uint16_t offset
        if addr <= 0x1FFF:
            self.ram[addr & BUS_RAM_MASK] = value
            return
        if addr <= 0x3FFF:
            self._ppu_write(addr & BUS_PPU_REG_MASK, value)
            return
        if addr <= 0x4007:
            offset = <uint16_t>(addr - BUS_APU_IO_BASE)
            self._apu_pulse_write(offset, value)
            self.apu_open_bus[offset] = value
            return
        if addr <= 0x400B:
            offset = <uint16_t>(addr - BUS_APU_IO_BASE)
            self._apu_triangle_write(offset, value)
            self.apu_open_bus[offset] = value
            return
        if addr <= 0x400F:
            offset = <uint16_t>(addr - BUS_APU_IO_BASE)
            self._apu_noise_write(offset, value)
            self.apu_open_bus[offset] = value
            return
        if addr <= 0x4013:
            offset = <uint16_t>(addr - BUS_APU_IO_BASE)
            self._apu_dmc_write(offset, value)
            self.apu_open_bus[offset] = value
            return
        if addr == 0x4014:
            self._oam_dma(value)
            return
        if addr == 0x4015:
            self.apu_open_bus[0x15] = value
            self.apu.write_status(value)
            return
        if addr == 0x4016:
            self.apu_open_bus[0x16] = value
            self.joypad.write_strobe(value)
            return
        if addr == 0x4017:
            self.apu_open_bus[0x17] = value
            self.apu.write_frame_counter(value)
            return
        if addr <= 0x401F:
            return
        self._cart_write(addr, value)

    # ---- bus_peek (bus.c bus_peek) ----
    cdef uint8_t peek(self, uint16_t addr) nogil:
        if addr <= 0x1FFF:
            return self.ram[addr & BUS_RAM_MASK]
        if addr <= 0x3FFF:
            return 0
        if addr <= 0x401F:
            return 0
        if self.cartridge is not None:
            return self.cartridge.read_prg(addr)
        return 0

    # ---- OAM-DMA stall / cycle counter ----
    cdef uint32_t take_dma_stall_cycles(self) nogil:
        cdef uint32_t c = self.dma_stall_cycles
        self.dma_stall_cycles = 0
        return c

    cdef void advance_cpu_cycles(self, uint32_t cycles) nogil:
        self.cpu_cycle_count = self.cpu_cycle_count + <uint64_t>cycles

    cdef uint64_t get_cpu_cycle_count(self) nogil:
        return self.cpu_cycle_count

    cdef void set_cpu_cycle_count(self, uint64_t count) nogil:
        self.cpu_cycle_count = count

    cdef uint32_t get_dma_stall_cycles(self) nogil:
        return self.dma_stall_cycles

    cdef void set_dma_stall_cycles(self, uint32_t cycles) nogil:
        self.dma_stall_cycles = cycles

    # ---- DMC DMA read (bus.c dmc_read_cb) ----
    cdef uint8_t dmc_read(self, uint16_t addr) nogil:
        if addr <= 0x1FFF:
            return self.ram[addr & BUS_RAM_MASK]
        if addr >= 0x8000:
            return self.cartridge.read_prg(addr) if self.cartridge is not None else 0
        return 0

    # ---- APU stepping ----
    cdef void step_apu(self, uint32_t cpu_cycles) nogil:
        self.apu.step(cpu_cycles, self)

    cdef bint apu_irq_pending(self) nogil:
        return self.apu.irq_pending()

    # ---- Cartridge mapper integration ----
    cdef bint cart_irq_pending(self) nogil:
        return self.cartridge.irq_pending() if self.cartridge is not None else False

    cdef void clock_cart_cpu(self, uint32_t cpu_cycles) nogil:
        if self.cartridge is not None:
            self.cartridge.clock_cpu(cpu_cycles)

    cdef float expansion_audio_sample(self) nogil:
        return self.cartridge.expansion_audio_sample() if self.cartridge is not None else 0.0

    cdef void cart_reset_scanline_counter(self) nogil:
        if self.cartridge is not None:
            self.cartridge.reset_scanline_counter()

    # ---- PPU stepping + rendering (bus.c bus_step_ppu) ----
    cdef bint step_ppu(self, uint32_t cycles) nogil:
        cdef bint nmi = False
        cdef uint16_t prerender = region_scanline_prerender(self.ppu.region)
        cdef bint rendering = self.ppu.is_rendering()
        cdef uint32_t i
        cdef uint16_t cyc, sl
        for i in range(cycles):
            if self.ppu.step_rendered():
                nmi = True
            cyc = self.ppu.cycle
            sl = self.ppu.scanline
            if rendering and cyc == MMC3_IRQ_CLOCK_CYCLE and (sl < PPU_SCREEN_HEIGHT or sl == prerender):
                if self.cartridge is not None:
                    self.cartridge.clock_irq()
            if sl == prerender and cyc == 1:
                if self.cartridge is not None:
                    self.cartridge.reset_scanline_counter()
        return nmi

    cdef bint take_nmi_request(self) nogil:
        return self.ppu.take_nmi_request()

    cdef void render_frame(self) nogil:
        self.ppu.render_frame()


# ============================================================================
# Emulator (emulator.c)
# ============================================================================
#
# Top-level emulator state: ties CPU, PPU, APU, bus, and cartridge together
# in a frame-locked loop. 1 CPU cycle = 3 PPU cycles, audio samples
# accumulated from the APU + expansion audio, frame boundary detected by the
# PPU scanline wrapping from the prerender scanline back to 0.


cdef class Emulator:
    cdef Cpu cpu
    cdef Bus bus
    cdef Region region
    cdef float sample_accumulator
    cdef uint32_t ppu_cycle_carry
    cdef Cartridge cartridge
    cdef float *audio_buffer
    cdef Py_ssize_t audio_buffer_capacity
    cdef Py_ssize_t audio_buffer_count

    def __cinit__(self):
        self.cpu = Cpu()
        self.bus = Bus()
        self.region = REGION_NTSC
        self.bus.ppu.set_region(REGION_NTSC)
        self.bus.apu.set_region(REGION_NTSC)
        self.sample_accumulator = 0.0
        self.ppu_cycle_carry = 0
        self.cartridge = None
        cdef uint32_t cap = EMU_AUDIO_CAP_PAL if region_scanlines_per_frame(REGION_NTSC) > 262 else EMU_AUDIO_CAP_NTSC
        self.audio_buffer = <float *>malloc(cap * sizeof(float))
        self.audio_buffer_capacity = cap if self.audio_buffer != NULL else 0
        self.audio_buffer_count = 0

    def __dealloc__(self):
        if self.audio_buffer != NULL:
            free(self.audio_buffer)
            self.audio_buffer = NULL

    # ---- Construction / configuration ----
    cdef void init_with_region(self, Region region):
        self.region = region
        self.bus.ppu.set_region(region)
        self.bus.apu.set_region(region)
        self.sample_accumulator = 0.0
        self.ppu_cycle_carry = 0
        cdef uint32_t cap = EMU_AUDIO_CAP_PAL if region_scanlines_per_frame(region) > 262 else EMU_AUDIO_CAP_NTSC
        if self.audio_buffer != NULL:
            free(self.audio_buffer)
        self.audio_buffer = <float *>malloc(cap * sizeof(float))
        self.audio_buffer_capacity = cap if self.audio_buffer != NULL else 0
        self.audio_buffer_count = 0

    cdef void reset(self):
        self.cpu.reset(self.bus)

    cdef Region get_region(self):
        return self.region

    cdef void set_region(self, Region region):
        self.region = region
        self.bus.ppu.set_region(region)
        self.bus.apu.set_region(region)

    cdef void insert_cartridge(self, Cartridge cartridge):
        self.cartridge = cartridge
        self.bus.insert_cartridge(cartridge)

    cdef Cartridge remove_cartridge(self):
        cdef Cartridge prev = self.cartridge
        self.bus.remove_cartridge()
        self.cartridge = None
        return prev

    # ---- Audio buffer push helper ----
    cdef void _push_audio(self, float sample):
        cdef Py_ssize_t new_cap
        cdef float *nb
        if self.audio_buffer_count >= self.audio_buffer_capacity:
            new_cap = self.audio_buffer_capacity * 2
            if new_cap < 16:
                new_cap = 16
            nb = <float *>realloc(self.audio_buffer, new_cap * sizeof(float))
            if nb == NULL:
                return
            self.audio_buffer = nb
            self.audio_buffer_capacity = new_cap
        self.audio_buffer[self.audio_buffer_count] = sample
        self.audio_buffer_count += 1

    # ---- Frame loop (emulator.c emu_step_one_cpu_tick) ----
    cdef uint32_t _step_one_cpu_tick(self, uint16_t prev_scanline, float cycles_per_sample, uint16_t prerender, bint *out_frame_done) nogil:
        cdef uint32_t cpu_cycles = 0
        cdef uint8_t step_cycles = self.cpu.step(self.bus)
        cpu_cycles += step_cycles
        cdef uint32_t dma_cycles = self.bus.take_dma_stall_cycles()
        cpu_cycles += dma_cycles
        self.bus.advance_cpu_cycles(<uint32_t>step_cycles + dma_cycles)
        cdef uint32_t apu_cycles = <uint32_t>step_cycles + dma_cycles
        self.bus.step_apu(apu_cycles)
        self.bus.clock_cart_cpu(apu_cycles)
        if self.bus.apu_irq_pending():
            self.cpu.set_irq_pending(True)
        if self.bus.cart_irq_pending():
            self.cpu.set_irq_pending(True)
        self.sample_accumulator += <float>apu_cycles
        cdef float internal, expansion, mixed
        while self.sample_accumulator >= cycles_per_sample:
            self.sample_accumulator -= cycles_per_sample
            internal = self.bus.apu.output()
            expansion = self.bus.expansion_audio_sample()
            mixed = internal + expansion
            if mixed < -1.0:
                mixed = -1.0
            if mixed > 1.0:
                mixed = 1.0
            self._push_audio_nogil(mixed)
        cdef uint32_t ppu_cycles = 3 * apu_cycles + self.ppu_cycle_carry
        self.ppu_cycle_carry = 0
        cdef uint32_t remaining = ppu_cycles
        cdef bint frame_done = False
        cdef uint32_t chunk
        cdef uint16_t curr_scanline
        while remaining > 0:
            chunk = remaining
            if chunk > PPU_CYCLES_PER_SCANLINE:
                chunk = PPU_CYCLES_PER_SCANLINE
            self.bus.step_ppu(chunk)
            remaining -= chunk
            if self.bus.take_nmi_request():
                self.cpu.set_nmi_pending(True)
            curr_scanline = self.bus.ppu.scanline
            if curr_scanline == 0 and prev_scanline == prerender:
                self.ppu_cycle_carry = remaining
                frame_done = True
                break
        out_frame_done[0] = frame_done
        return cpu_cycles

    cdef void _push_audio_nogil(self, float sample) nogil:
        if self.audio_buffer_count >= self.audio_buffer_capacity:
            return
        self.audio_buffer[self.audio_buffer_count] = sample
        self.audio_buffer_count += 1

    # ---- step_frame (emulator.c emulator_step_frame) ----
    cdef uint32_t step_frame(self) nogil:
        cdef uint32_t cpu_cycles = 0
        cdef uint32_t universal_bg = self.bus.ppu.universal_bg_argb()
        self.bus.ppu.clear_framebuffer(universal_bg)
        self.bus.ppu.reset_rendered_flag()
        cdef float cycles_per_sample = region_cpu_cycles_per_sample(self.region)
        cdef uint16_t prerender = region_scanline_prerender(self.region)
        cdef uint16_t prev_scanline
        cdef bint frame_done
        while True:
            prev_scanline = self.bus.ppu.scanline
            frame_done = False
            cpu_cycles += self._step_one_cpu_tick(prev_scanline, cycles_per_sample, prerender, &frame_done)
            if frame_done:
                break
        if not self.bus.ppu.rendered_this_frame:
            self.bus.render_frame()
        return cpu_cycles

    # ---- step_instruction (emulator.c emulator_step_instruction) ----
    cdef uint32_t step_instruction(self) nogil:
        cdef uint16_t prev_scanline = self.bus.ppu.scanline
        cdef float cycles_per_sample = region_cpu_cycles_per_sample(self.region)
        cdef uint16_t prerender = region_scanline_prerender(self.region)
        cdef bint frame_done = False
        return self._step_one_cpu_tick(prev_scanline, cycles_per_sample, prerender, &frame_done)

    # ---- Output ----
    cdef uint32_t *framebuffer_ptr(self) nogil:
        return &self.bus.ppu.framebuffer[0]

    cdef uint16_t mapper_number(self):
        if self.cartridge is not None:
            return self.cartridge.mapper_number
        return 0

    cdef Py_ssize_t take_audio_samples(self, float *out, Py_ssize_t cap) nogil:
        cdef Py_ssize_t n = self.audio_buffer_count
        if n > cap:
            n = cap
        cdef Py_ssize_t i
        if n > 0 and out != NULL:
            for i in range(n):
                out[i] = self.audio_buffer[i]
        self.audio_buffer_count = 0
        return n

    # ---- Save-state accessors ----
    cdef float get_sample_accumulator(self) nogil:
        return self.sample_accumulator

    cdef void set_sample_accumulator(self, float acc) nogil:
        self.sample_accumulator = acc

    cdef Py_ssize_t audio_buffer_count_(self) nogil:
        return self.audio_buffer_count

    cdef void set_audio_buffer(self, const float *buf, Py_ssize_t count):
        cdef float *nb
        cdef Py_ssize_t i
        if count > self.audio_buffer_capacity:
            nb = <float *>realloc(self.audio_buffer, count * sizeof(float))
            if nb == NULL:
                self.audio_buffer_count = 0
                return
            self.audio_buffer = nb
            self.audio_buffer_capacity = count
        if count > 0 and buf != NULL:
            for i in range(count):
                self.audio_buffer[i] = buf[i]
        self.audio_buffer_count = count

    cdef uint32_t get_ppu_cycle_carry(self) nogil:
        return self.ppu_cycle_carry

    cdef void set_ppu_cycle_carry(self, uint32_t carry) nogil:
        self.ppu_cycle_carry = carry

    # ========================================================================
    # Python-accessible wrappers (called by protocol.py — pure Python)
    # ========================================================================

    def py_load_rom(self, bytes rom):
        """Parse iNES bytes and insert the cartridge. Returns True on success."""
        cdef Py_ssize_t length = len(rom)
        if length < 16:
            return False
        cdef const uint8_t[:] rom_view = rom
        cdef Cartridge cart = Cartridge.from_bytes(&rom_view[0], length)
        if cart is None:
            return False
        self.insert_cartridge(cart)
        return True

    def py_reset(self):
        """Perform the 6502 RESET sequence."""
        self.reset()

    def py_step_frame(self):
        """Run one full frame. Returns CPU cycles consumed (int)."""
        cdef uint32_t cycles
        with nogil:
            cycles = self.step_frame()
        return cycles

    def py_step_instruction(self):
        """Run one CPU instruction. Returns CPU cycles consumed (int)."""
        cdef uint32_t cycles
        with nogil:
            cycles = self.step_instruction()
        return cycles

    def py_framebuffer_bytes(self):
        """Return the 256x240 ARGB framebuffer as raw bytes (245760 bytes)."""
        cdef const unsigned char[:] fb_view = \
            <const unsigned char[:PPU_FRAMEBUFFER_SIZE * 4]> \
            <const unsigned char*>&self.bus.ppu.framebuffer[0]
        return bytes(fb_view)

    def py_take_audio(self, Py_ssize_t cap):
        """Drain audio samples as int16 bytes. Returns raw bytes (2*n bytes)."""
        if cap <= 0:
            return b''
        # Allocate a float buffer, drain, convert to int16, return bytes.
        cdef float* fbuf = <float*>malloc(cap * sizeof(float))
        if fbuf == NULL:
            return b''
        cdef Py_ssize_t n
        with nogil:
            n = self.take_audio_samples(fbuf, cap)
        cdef Py_ssize_t i
        cdef short* out16 = <short*>malloc(n * sizeof(short))
        if out16 == NULL:
            free(fbuf)
            return b''
        cdef float s
        cdef int v
        for i in range(n):
            s = fbuf[i]
            if s > 1.0:
                s = 1.0
            if s < -1.0:
                s = -1.0
            if s >= 0.0:
                v = <int>(s * 32767.0)
                if v > 32767:
                    v = 32767
            else:
                v = <int>(s * 32768.0)
                if v < -32768:
                    v = -32768
            out16[i] = <short>v
        free(fbuf)
        if n == 0:
            free(out16)
            return b''
        cdef const unsigned char[:] audio_view = \
            <const unsigned char[:n * 2]> \
            <const unsigned char*>out16
        cdef bytes result = bytes(audio_view)
        free(out16)
        return result

    def py_mapper_number(self):
        """Return the loaded cartridge's iNES mapper number (int)."""
        return self.mapper_number()

    def py_set_region(self, int region):
        """Set the TV system / region (0=NTSC, 1=PAL, 2=Dendy)."""
        self.set_region(<Region>region)

    def py_get_region(self):
        """Return the current region (0=NTSC, 1=PAL, 2=Dendy)."""
        return <int>self.region

    def py_impl_name(self):
        return "python-cython-nes"

    def py_impl_version(self):
        return "0.1.0"

    # ========================================================================
    # Save state — manual binary serialization (round-trips within this core)
    # ========================================================================

    def py_save_state(self):
        """Serialize full emulator state to bytes."""
        cdef bytearray buf = bytearray()
        cdef Cpu cpu = self.cpu
        cdef Bus bus = self.bus
        cdef Ppu ppu = bus.ppu
        cdef Apu apu = bus.apu
        cdef Joypad joy = bus.joypad
        cdef Cartridge cart = self.cartridge

        # Header: magic + version + payload_size (filled later)
        buf += b'NESS'
        buf += _struct.pack("<I", 1)  # version

        # Build payload separately so we can fill the size field.
        cdef bytearray payload = bytearray()
        cdef int i

        # --- CPU (8 bytes) ---
        payload.append(cpu.a)
        payload.append(cpu.x)
        payload.append(cpu.y)
        payload.append(cpu.sp)
        payload += _struct.pack("<H", cpu.pc)
        payload.append(cpu.status)
        payload.append(cpu.flags)

        # --- Bus RAM (2048 bytes) ---
        for i in range(0x0800):
            payload.append(bus.ram[i])
        # --- Bus APU open-bus (24 bytes) ---
        for i in range(0x18):
            payload.append(bus.apu_open_bus[i])
        # --- Bus dma_stall_cycles (4) + cpu_cycle_count (8) ---
        payload += _struct.pack("<I", bus.dma_stall_cycles)
        payload += _struct.pack("<Q", bus.cpu_cycle_count)

        # --- PPU registers + state ---
        payload.append(ppu.ppuctrl)
        payload.append(ppu.ppumask)
        payload.append(ppu.ppustatus)
        payload.append(ppu.oamaddr)
        payload.append(ppu.open_bus)
        payload.append(ppu.ppudata_buffer)
        payload += _struct.pack("<H", ppu.v)
        payload += _struct.pack("<H", ppu.t)
        payload.append(ppu.fine_x)
        payload.append(1 if ppu.w else 0)
        payload.append(1 if ppu.nmi_request else 0)
        payload += _struct.pack("<H", ppu.scanline)
        payload += _struct.pack("<H", ppu.cycle)
        payload.append(<uint8_t>ppu.region)
        payload.append(<uint8_t>ppu.mirroring)
        payload += _struct.pack("<I", ppu.vram_size)
        # VRAM (vram_size bytes)
        for i in range(ppu.vram_size):
            payload.append(ppu.vram[i])
        # OAM (256 bytes)
        for i in range(256):
            payload.append(ppu.oam[i])
        # Palette (32 bytes)
        for i in range(32):
            payload.append(ppu.palette[i])
        # Render pipeline state
        payload.append(1 if ppu.scanline_initialized else 0)
        payload.append(1 if ppu.pipeline_primed else 0)
        payload.append(1 if ppu.rendered_this_frame else 0)
        payload.append(1 if ppu.sprite_zero_hit else 0)
        payload.append(1 if ppu.v_dirty else 0)
        payload.append(1 if ppu.slant_corruption else 0)
        payload.append(1 if ppu.use_inaccurate_palette else 0)
        payload.append(1 if ppu.nmi_retrigger else 0)
        payload += _struct.pack("<H", ppu.coarse_y)
        payload += _struct.pack("<H", ppu.fine_y)
        payload += _struct.pack("<H", ppu.nt_v)
        payload += _struct.pack("<H", ppu.coarse_x_start)
        payload += _struct.pack("<H", ppu.nt_h_start)
        payload.append(ppu.fine_x_start)
        payload += _struct.pack("<H", ppu.resync_px)
        payload.append(ppu.sprite_count)
        payload.append(1 if ppu.overflow else 0)
        payload.append(ppu.fetch_idx)
        # bg_pattern (256 bytes)
        for i in range(256):
            payload.append(ppu.bg_pattern[i])
        # sprites[8] — each ScanlineSprite has oam_i, y, tile, attr, sx(u16)
        for i in range(8):
            payload.append(ppu.sprites[i].oam_i)
            payload.append(ppu.sprites[i].y)
            payload.append(ppu.sprites[i].tile)
            payload.append(ppu.sprites[i].attr)
            payload += _struct.pack("<H", ppu.sprites[i].sx)
        # fetch_buffer[2] — each BgFetch has pattern, pal_select
        for i in range(2):
            payload.append(ppu.fetch_buffer[i].pattern)
            payload.append(ppu.fetch_buffer[i].pal_select)

        # --- APU channels ---
        # Pulse1 (20 bytes: duty, halt, cv, vol, sweep_enabled, sweep_period,
        #   sweep_negate, sweep_shift, sweep_divider, sweep_reload,
        #   timer_period(2), timer(2), sequence, length_counter, enabled,
        #   env_divider, env_decay, env_start)
        payload = _pack_pulse(payload, apu.pulse1)
        payload = _pack_pulse(payload, apu.pulse2)
        # Triangle (12 bytes)
        payload.append(1 if apu.triangle.halt else 0)
        payload.append(apu.triangle.linear_reload)
        payload.append(apu.triangle.linear_counter)
        payload.append(1 if apu.triangle.linear_start else 0)
        payload += _struct.pack("<H", apu.triangle.timer_period)
        payload += _struct.pack("<H", apu.triangle.timer)
        payload.append(apu.triangle.sequence)
        payload.append(apu.triangle.length_counter)
        payload.append(1 if apu.triangle.enabled else 0)
        payload.append(apu.triangle.linear_reload)  # pad to 12
        payload.append(0)
        payload.append(0)
        # Noise (17 bytes)
        payload.append(1 if apu.noise.halt else 0)
        payload.append(1 if apu.noise.constant_volume else 0)
        payload.append(apu.noise.volume)
        payload.append(1 if apu.noise.mode else 0)
        payload.append(apu.noise.period_index)
        payload += _struct.pack("<H", apu.noise.timer_period)
        payload += _struct.pack("<H", apu.noise.timer)
        payload += _struct.pack("<H", apu.noise.lfsr)
        payload.append(apu.noise.length_counter)
        payload.append(1 if apu.noise.enabled else 0)
        payload.append(apu.noise.envelope_divider)
        payload.append(apu.noise.envelope_decay)
        payload.append(1 if apu.noise.envelope_start else 0)
        payload.append(0)
        payload.append(0)
        # DMC (22 bytes)
        payload.append(1 if apu.dmc.irq_enable else 0)
        payload.append(1 if apu.dmc.loop_flag else 0)
        payload.append(apu.dmc.rate_index)
        payload += _struct.pack("<H", apu.dmc.timer_period)
        payload += _struct.pack("<H", apu.dmc.timer)
        payload.append(apu.dmc.output_counter)
        payload.append(apu.dmc.sample_buffer)
        payload.append(apu.dmc.buffer_bits)
        payload += _struct.pack("<H", apu.dmc.sample_addr_base)
        payload += _struct.pack("<H", apu.dmc.sample_address)
        payload += _struct.pack("<H", apu.dmc.sample_length)
        payload += _struct.pack("<H", apu.dmc.bytes_remaining)
        payload.append(1 if apu.dmc.enabled else 0)
        payload.append(1 if apu.dmc.irq_flag else 0)
        payload.append(0)
        payload.append(0)
        payload.append(0)
        payload.append(0)
        payload.append(0)
        payload.append(0)
        # APU top-level state
        payload += _struct.pack("<I", apu.cycle_accumulator)
        for i in range(5):
            payload += _struct.pack("<f", apu.channel_volumes[i])
        for i in range(5):
            payload.append(1 if apu.channel_muted[i] else 0)
        payload.append(apu.selected_channel)
        payload += _struct.pack("<f", apu.lpf_prev)
        payload += _struct.pack("<f", apu.dc_prev_x)
        payload += _struct.pack("<f", apu.dc_prev_y)
        payload += _struct.pack("<f", apu.mix_accumulator)
        payload += _struct.pack("<I", apu.mix_count)
        payload += _struct.pack("<f", apu.sample_accumulator)
        payload += _struct.pack("<f", apu.last_decimated)
        payload.append(1 if apu.frame_mode_5step else 0)
        payload.append(1 if apu.frame_irq_inhibit else 0)
        payload += _struct.pack("<I", apu.frame_cycle)
        payload.append(1 if apu.frame_irq else 0)
        payload += _struct.pack("<I", apu.frame_reset_delay)
        payload.append(<uint8_t>apu.region)

        # --- Joypad (7 bytes) ---
        payload.append(joy.current[0])
        payload.append(joy.current[1])
        payload.append(1 if joy.strobe else 0)
        payload.append(joy.shift[0])
        payload.append(joy.shift[1])
        payload.append(joy.counter[0])
        payload.append(joy.counter[1])

        # --- Cartridge ---
        if cart is not None:
            payload.append(1)  # present
            payload += _struct.pack("<H", cart.mapper_number)
            payload.append(<uint8_t>cart.mirroring)
            payload.append(1 if cart.has_trainer else 0)
            payload.append(1 if cart.has_battery else 0)
            payload.append(cart.prg_rom_banks)
            payload.append(cart.chr_rom_banks)
            payload.append(1 if cart.chr_is_ram else 0)
            payload += _struct.pack("<I", cart.prg_size)
            payload += _struct.pack("<I", cart.chr_size)
            # PRG ROM
            for i in range(cart.prg_size):
                payload.append(cart.prg_rom[i])
            # CHR
            for i in range(cart.chr_size):
                payload.append(cart.chr[i])
            # Mapper-specific state
            payload.append(cart.prg_bank)
            payload.append(cart.chr_bank)
            payload.append(cart.mmc1_shift_reg)
            payload.append(cart.mmc1_shift_count)
            payload.append(cart.mmc1_control)
            payload.append(cart.mmc1_chr_bank_0)
            payload.append(cart.mmc1_chr_bank_1)
            payload.append(cart.mmc1_prg_bank)
            payload.append(cart.mmc3_bank_select)
            for i in range(8):
                payload.append(cart.mmc3_registers[i])
            payload.append(cart.mmc3_irq_reload)
            payload.append(cart.mmc3_irq_counter)
            payload.append(1 if cart.mmc3_irq_enable else 0)
            payload.append(1 if cart.mmc3_irq_pending_flag else 0)
            payload.append(cart.mmc3_chr_mode_8k)
            for i in range(4):
                payload.append(cart.mmc5_prg_bank[i])
            for i in range(8):
                payload.append(cart.mmc5_chr_bank[i])
            payload.append(1 if cart.mmc5_irq_pending_flag else 0)
            payload.append(1 if cart.mmc2_latch_fd else 0)
            payload.append(1 if cart.mmc2_latch_fe else 0)
            for i in range(4):
                payload.append(cart.mmc2_prg_banks[i])
            for i in range(8):
                payload.append(cart.mmc2_chr_banks[i])
            payload.append(1 if cart.vrc6_is_26 else 0)
            for i in range(2):
                payload.append(cart.vrc6_prg_bank[i])
            for i in range(8):
                payload.append(cart.vrc6_chr_bank[i])
            payload.append(1 if cart.vrc6_mirroring else 0)
            payload += _struct.pack("<H", cart.vrc6_irq_count)
            payload.append(cart.vrc6_irq_enable)
            payload.append(1 if cart.vrc6_irq_pending_flag else 0)
            payload += _struct.pack("<H", cart.vrc6_irq_prescale)
            payload.append(cart.fme7_command)
            for i in range(16):
                payload.append(cart.fme7_registers[i])
            payload += _struct.pack("<H", cart.fme7_irq_count)
            payload.append(cart.fme7_irq_enable)
            payload.append(1 if cart.fme7_irq_pending_flag else 0)
            for i in range(3):
                payload.append(cart.vrc7_prg_bank[i])
            for i in range(8):
                payload.append(cart.vrc7_chr_bank[i])
            for i in range(3):
                payload.append(cart.n163_prg_bank[i])
            for i in range(8):
                payload.append(cart.n163_chr_bank[i])
            payload.append(cart.n163_addr)
            payload.append(cart.n163_increment)
            for i in range(128):
                payload.append(cart.n163_ram[i])
        else:
            payload.append(0)  # not present

        # --- Emulator-level state ---
        payload += _struct.pack("<f", self.sample_accumulator)
        payload += _struct.pack("<I", self.ppu_cycle_carry)
        payload += _struct.pack("<I", <uint32_t>self.audio_buffer_count)
        for i in range(self.audio_buffer_count):
            payload += _struct.pack("<f", self.audio_buffer[i])
        payload.append(<uint8_t>self.region)

        # Assemble: header with payload size + payload
        buf += _struct.pack("<I", len(payload))
        buf += payload
        return bytes(buf)

    def py_load_state(self, bytes data):
        """Deserialize full emulator state from bytes. Returns True on success."""
        cdef Py_ssize_t length = len(data)
        if length < 12:
            return False
        cdef const uint8_t[:] view = data
        cdef Py_ssize_t off = 0
        # Magic "NESS" = 0x4E, 0x45, 0x53, 0x53
        if view[0] != 0x4E or view[1] != 0x45 or view[2] != 0x53 or view[3] != 0x53:
            return False
        off = 4
        # Version
        cdef uint32_t version = <uint32_t>view[off] | (<uint32_t>view[off+1] << 8) | \
                                (<uint32_t>view[off+2] << 16) | (<uint32_t>view[off+3] << 24)
        off += 4
        if version != 1:
            return False
        # Payload size
        cdef uint32_t payload_size = <uint32_t>view[off] | (<uint32_t>view[off+1] << 8) | \
                                     (<uint32_t>view[off+2] << 16) | (<uint32_t>view[off+3] << 24)
        off += 4
        if off + payload_size > length:
            return False

        cdef Cpu cpu = self.cpu
        cdef Bus bus = self.bus
        cdef Ppu ppu = bus.ppu
        cdef Apu apu = bus.apu
        cdef Joypad joy = bus.joypad
        cdef int i
        cdef uint8_t cart_present
        cdef Cartridge cart
        cdef uint32_t audio_count
        cpu.a = view[off]; off += 1
        cpu.x = view[off]; off += 1
        cpu.y = view[off]; off += 1
        cpu.sp = view[off]; off += 1
        cpu.pc = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
        cpu.status = view[off]; off += 1
        cpu.flags = view[off]; off += 1

        # --- Bus RAM (2048 bytes) ---
        for i in range(0x0800):
            bus.ram[i] = view[off]; off += 1
        # --- Bus APU open-bus (24 bytes) ---
        for i in range(0x18):
            bus.apu_open_bus[i] = view[off]; off += 1
        # --- Bus dma_stall_cycles (4) + cpu_cycle_count (8) ---
        bus.dma_stall_cycles = <uint32_t>view[off] | (<uint32_t>view[off+1] << 8) | \
                               (<uint32_t>view[off+2] << 16) | (<uint32_t>view[off+3] << 24)
        off += 4
        bus.cpu_cycle_count = <uint64_t>view[off] | (<uint64_t>view[off+1] << 8) | \
                              (<uint64_t>view[off+2] << 16) | (<uint64_t>view[off+3] << 24) | \
                              (<uint64_t>view[off+4] << 32) | (<uint64_t>view[off+5] << 40) | \
                              (<uint64_t>view[off+6] << 48) | (<uint64_t>view[off+7] << 56)
        off += 8

        # --- PPU registers + state ---
        ppu.ppuctrl = view[off]; off += 1
        ppu.ppumask = view[off]; off += 1
        ppu.ppustatus = view[off]; off += 1
        ppu.oamaddr = view[off]; off += 1
        ppu.open_bus = view[off]; off += 1
        ppu.ppudata_buffer = view[off]; off += 1
        ppu.v = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
        ppu.t = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
        ppu.fine_x = view[off]; off += 1
        ppu.w = view[off] != 0; off += 1
        ppu.nmi_request = view[off] != 0; off += 1
        ppu.scanline = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
        ppu.cycle = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
        ppu.region = <Region>view[off]; off += 1
        ppu.mirroring = <Mirroring>view[off]; off += 1
        ppu.vram_size = <uint32_t>view[off] | (<uint32_t>view[off+1] << 8) | \
                        (<uint32_t>view[off+2] << 16) | (<uint32_t>view[off+3] << 24)
        off += 4
        for i in range(ppu.vram_size):
            ppu.vram[i] = view[off]; off += 1
        for i in range(256):
            ppu.oam[i] = view[off]; off += 1
        for i in range(32):
            ppu.palette[i] = view[off]; off += 1
        ppu.scanline_initialized = view[off] != 0; off += 1
        ppu.pipeline_primed = view[off] != 0; off += 1
        ppu.rendered_this_frame = view[off] != 0; off += 1
        ppu.sprite_zero_hit = view[off] != 0; off += 1
        ppu.v_dirty = view[off] != 0; off += 1
        ppu.slant_corruption = view[off] != 0; off += 1
        ppu.use_inaccurate_palette = view[off] != 0; off += 1
        ppu.nmi_retrigger = view[off] != 0; off += 1
        ppu.coarse_y = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
        ppu.fine_y = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
        ppu.nt_v = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
        ppu.coarse_x_start = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
        ppu.nt_h_start = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
        ppu.fine_x_start = view[off]; off += 1
        ppu.resync_px = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
        ppu.sprite_count = view[off]; off += 1
        ppu.overflow = view[off] != 0; off += 1
        ppu.fetch_idx = view[off]; off += 1
        for i in range(256):
            ppu.bg_pattern[i] = view[off]; off += 1
        for i in range(8):
            ppu.sprites[i] = ScanlineSprite(
                view[off], view[off+1], view[off+2], view[off+3],
                <uint16_t>view[off+4] | (<uint16_t>view[off+5] << 8))
            off += 6
        for i in range(2):
            ppu.fetch_buffer[i] = BgFetch(view[off], view[off+1])
            off += 2

        # --- APU channels ---
        off = _unpack_pulse(view, off, apu.pulse1)
        off = _unpack_pulse(view, off, apu.pulse2)
        # Triangle (12 bytes)
        apu.triangle.halt = view[off] != 0; off += 1
        apu.triangle.linear_reload = view[off]; off += 1
        apu.triangle.linear_counter = view[off]; off += 1
        apu.triangle.linear_start = view[off] != 0; off += 1
        apu.triangle.timer_period = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
        apu.triangle.timer = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
        apu.triangle.sequence = view[off]; off += 1
        apu.triangle.length_counter = view[off]; off += 1
        apu.triangle.enabled = view[off] != 0; off += 1
        off += 3  # pad
        # Noise (17 bytes)
        apu.noise.halt = view[off] != 0; off += 1
        apu.noise.constant_volume = view[off] != 0; off += 1
        apu.noise.volume = view[off]; off += 1
        apu.noise.mode = view[off] != 0; off += 1
        apu.noise.period_index = view[off]; off += 1
        apu.noise.timer_period = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
        apu.noise.timer = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
        apu.noise.lfsr = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
        apu.noise.length_counter = view[off]; off += 1
        apu.noise.enabled = view[off] != 0; off += 1
        apu.noise.envelope_divider = view[off]; off += 1
        apu.noise.envelope_decay = view[off]; off += 1
        apu.noise.envelope_start = view[off] != 0; off += 1
        off += 2  # pad
        # DMC (22 bytes)
        apu.dmc.irq_enable = view[off] != 0; off += 1
        apu.dmc.loop_flag = view[off] != 0; off += 1
        apu.dmc.rate_index = view[off]; off += 1
        apu.dmc.timer_period = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
        apu.dmc.timer = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
        apu.dmc.output_counter = view[off]; off += 1
        apu.dmc.sample_buffer = view[off]; off += 1
        apu.dmc.buffer_bits = view[off]; off += 1
        apu.dmc.sample_addr_base = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
        apu.dmc.sample_address = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
        apu.dmc.sample_length = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
        apu.dmc.bytes_remaining = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
        apu.dmc.enabled = view[off] != 0; off += 1
        apu.dmc.irq_flag = view[off] != 0; off += 1
        off += 6  # pad
        # APU top-level
        apu.cycle_accumulator = <uint32_t>view[off] | (<uint32_t>view[off+1] << 8) | \
                                (<uint32_t>view[off+2] << 16) | (<uint32_t>view[off+3] << 24)
        off += 4
        for i in range(5):
            apu.channel_volumes[i] = _unpack_float(view, off); off += 4
        for i in range(5):
            apu.channel_muted[i] = view[off] != 0; off += 1
        apu.selected_channel = view[off]; off += 1
        apu.lpf_prev = _unpack_float(view, off); off += 4
        apu.dc_prev_x = _unpack_float(view, off); off += 4
        apu.dc_prev_y = _unpack_float(view, off); off += 4
        apu.mix_accumulator = _unpack_float(view, off); off += 4
        apu.mix_count = <uint32_t>view[off] | (<uint32_t>view[off+1] << 8) | \
                        (<uint32_t>view[off+2] << 16) | (<uint32_t>view[off+3] << 24)
        off += 4
        apu.sample_accumulator = _unpack_float(view, off); off += 4
        apu.last_decimated = _unpack_float(view, off); off += 4
        apu.frame_mode_5step = view[off] != 0; off += 1
        apu.frame_irq_inhibit = view[off] != 0; off += 1
        apu.frame_cycle = <uint32_t>view[off] | (<uint32_t>view[off+1] << 8) | \
                          (<uint32_t>view[off+2] << 16) | (<uint32_t>view[off+3] << 24)
        off += 4
        apu.frame_irq = view[off] != 0; off += 1
        apu.frame_reset_delay = <uint32_t>view[off] | (<uint32_t>view[off+1] << 8) | \
                                (<uint32_t>view[off+2] << 16) | (<uint32_t>view[off+3] << 24)
        off += 4
        apu.region = <Region>view[off]; off += 1

        # --- Joypad (7 bytes) ---
        joy.current[0] = view[off]; off += 1
        joy.current[1] = view[off]; off += 1
        joy.strobe = view[off] != 0; off += 1
        joy.shift[0] = view[off]; off += 1
        joy.shift[1] = view[off]; off += 1
        joy.counter[0] = view[off]; off += 1
        joy.counter[1] = view[off]; off += 1

        # --- Cartridge ---
        cart_present = view[off]; off += 1
        if cart_present != 0 and self.cartridge is not None:
            cart = self.cartridge
            cart.mapper_number = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
            cart.mirroring = <Mirroring>view[off]; off += 1
            cart.has_trainer = view[off] != 0; off += 1
            cart.has_battery = view[off] != 0; off += 1
            cart.prg_rom_banks = view[off]; off += 1
            cart.chr_rom_banks = view[off]; off += 1
            cart.chr_is_ram = view[off] != 0; off += 1
            cart.prg_size = <uint32_t>view[off] | (<uint32_t>view[off+1] << 8) | \
                            (<uint32_t>view[off+2] << 16) | (<uint32_t>view[off+3] << 24)
            off += 4
            cart.chr_size = <uint32_t>view[off] | (<uint32_t>view[off+1] << 8) | \
                            (<uint32_t>view[off+2] << 16) | (<uint32_t>view[off+3] << 24)
            off += 4
            for i in range(cart.prg_size):
                cart.prg_rom[i] = view[off]; off += 1
            for i in range(cart.chr_size):
                cart.chr[i] = view[off]; off += 1
            cart.prg_bank = view[off]; off += 1
            cart.chr_bank = view[off]; off += 1
            cart.mmc1_shift_reg = view[off]; off += 1
            cart.mmc1_shift_count = view[off]; off += 1
            cart.mmc1_control = view[off]; off += 1
            cart.mmc1_chr_bank_0 = view[off]; off += 1
            cart.mmc1_chr_bank_1 = view[off]; off += 1
            cart.mmc1_prg_bank = view[off]; off += 1
            cart.mmc3_bank_select = view[off]; off += 1
            for i in range(8):
                cart.mmc3_registers[i] = view[off]; off += 1
            cart.mmc3_irq_reload = view[off]; off += 1
            cart.mmc3_irq_counter = view[off]; off += 1
            cart.mmc3_irq_enable = view[off] != 0; off += 1
            cart.mmc3_irq_pending_flag = view[off] != 0; off += 1
            cart.mmc3_chr_mode_8k = view[off]; off += 1
            for i in range(4):
                cart.mmc5_prg_bank[i] = view[off]; off += 1
            for i in range(8):
                cart.mmc5_chr_bank[i] = view[off]; off += 1
            cart.mmc5_irq_pending_flag = view[off] != 0; off += 1
            cart.mmc2_latch_fd = view[off] != 0; off += 1
            cart.mmc2_latch_fe = view[off] != 0; off += 1
            for i in range(4):
                cart.mmc2_prg_banks[i] = view[off]; off += 1
            for i in range(8):
                cart.mmc2_chr_banks[i] = view[off]; off += 1
            cart.vrc6_is_26 = view[off] != 0; off += 1
            for i in range(2):
                cart.vrc6_prg_bank[i] = view[off]; off += 1
            for i in range(8):
                cart.vrc6_chr_bank[i] = view[off]; off += 1
            cart.vrc6_mirroring = view[off] != 0; off += 1
            cart.vrc6_irq_count = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
            cart.vrc6_irq_enable = view[off]; off += 1
            cart.vrc6_irq_pending_flag = view[off] != 0; off += 1
            cart.vrc6_irq_prescale = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
            cart.fme7_command = view[off]; off += 1
            for i in range(16):
                cart.fme7_registers[i] = view[off]; off += 1
            cart.fme7_irq_count = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
            cart.fme7_irq_enable = view[off]; off += 1
            cart.fme7_irq_pending_flag = view[off] != 0; off += 1
            for i in range(3):
                cart.vrc7_prg_bank[i] = view[off]; off += 1
            for i in range(8):
                cart.vrc7_chr_bank[i] = view[off]; off += 1
            for i in range(3):
                cart.n163_prg_bank[i] = view[off]; off += 1
            for i in range(8):
                cart.n163_chr_bank[i] = view[off]; off += 1
            cart.n163_addr = view[off]; off += 1
            cart.n163_increment = view[off]; off += 1
            for i in range(128):
                cart.n163_ram[i] = view[off]; off += 1
        elif cart_present != 0:
            # Cartridge was present at save time but is missing now — cannot
            # restore PRG/CHR/mapper state. Fail cleanly rather than silently
            # corrupting the rest of the payload.
            return False

        # --- Emulator-level state ---
        self.sample_accumulator = _unpack_float(view, off); off += 4
        self.ppu_cycle_carry = <uint32_t>view[off] | (<uint32_t>view[off+1] << 8) | \
                               (<uint32_t>view[off+2] << 16) | (<uint32_t>view[off+3] << 24)
        off += 4
        audio_count = <uint32_t>view[off] | (<uint32_t>view[off+1] << 8) | \
                                    (<uint32_t>view[off+2] << 16) | (<uint32_t>view[off+3] << 24)
        off += 4
        if audio_count > self.audio_buffer_capacity:
            audio_count = self.audio_buffer_capacity
        for i in range(audio_count):
            self.audio_buffer[i] = _unpack_float(view, off); off += 4
        self.audio_buffer_count = audio_count
        self.region = <Region>view[off]; off += 1

        return True


# ============================================================================
# Save-state helper functions (module-level cdef)
# ============================================================================

cdef float _unpack_float(const uint8_t[:] view, Py_ssize_t off):
    cdef float v
    memcpy(&v, &view[off], 4)
    return v

cdef bytearray _pack_pulse(bytearray payload, PulseChannel p):
    payload.append(p.duty)
    payload.append(1 if p.halt else 0)
    payload.append(1 if p.constant_volume else 0)
    payload.append(p.volume)
    payload.append(1 if p.sweep_enabled else 0)
    payload.append(p.sweep_period)
    payload.append(1 if p.sweep_negate else 0)
    payload.append(p.sweep_shift)
    payload.append(p.sweep_divider)
    payload.append(1 if p.sweep_reload else 0)
    payload += _struct.pack("<H", p.timer_period)
    payload += _struct.pack("<H", p.timer)
    payload.append(p.sequence)
    payload.append(p.length_counter)
    payload.append(1 if p.enabled else 0)
    payload.append(p.envelope_divider)
    payload.append(p.envelope_decay)
    payload.append(1 if p.envelope_start else 0)
    payload.append(0)
    payload.append(0)
    return payload

cdef Py_ssize_t _unpack_pulse(const uint8_t[:] view, Py_ssize_t off, PulseChannel p):
    p.duty = view[off]; off += 1
    p.halt = view[off] != 0; off += 1
    p.constant_volume = view[off] != 0; off += 1
    p.volume = view[off]; off += 1
    p.sweep_enabled = view[off] != 0; off += 1
    p.sweep_period = view[off]; off += 1
    p.sweep_negate = view[off] != 0; off += 1
    p.sweep_shift = view[off]; off += 1
    p.sweep_divider = view[off]; off += 1
    p.sweep_reload = view[off] != 0; off += 1
    p.timer_period = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
    p.timer = <uint16_t>view[off] | (<uint16_t>view[off+1] << 8); off += 2
    p.sequence = view[off]; off += 1
    p.length_counter = view[off]; off += 1
    p.enabled = view[off] != 0; off += 1
    p.envelope_divider = view[off]; off += 1
    p.envelope_decay = view[off]; off += 1
    p.envelope_start = view[off] != 0; off += 1
    off += 2  # pad
    return off
