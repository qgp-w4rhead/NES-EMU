/*
 * save_state.c - Serialisation/deserialisation of full emulator state.
 *
 * Port of src/save_state/mod.rs to C (M4.5). Uses a custom binary format
 * (NOT bincode) so the C core's save states are self-contained. The
 * format embeds a magic + version tag so it can be validated on load.
 *
 * Layout (all integers little-endian):
 *   [4]   magic "NESS" (0x5353454E LE)
 *   [4]   version (u32 = SAVE_STATE_VERSION)
 *   [4]   total_payload_size (u32, bytes after this 12-byte header)
 *   --- payload ---
 *   [sizeof(Cpu)]            Cpu struct (memcpy)
 *   [BUS_RAM_SIZE]           RAM (2048 bytes)
 *   [4]                      vram_size (u32)
 *   [vram_size]              VRAM
 *   [256]                    OAM
 *   [32]                     palette
 *   [1] ppuctrl, [1] ppumask, [1] oamaddr, [1] ppustatus
 *   [2] v, [2] t, [1] fine_x, [1] w (0/1)
 *   [1] ppudata_buffer, [1] open_bus
 *   [2] scanline, [2] cycle
 *   [1] nmi_request (0/1)
 *   [4] mirroring (int), [4] region (int)
 *   [BUS_APU_IO_REG_COUNT]   apu_open_bus (24 bytes)
 *   [sizeof(Apu)]            Apu struct (memcpy)
 *   [sizeof(Joypad)]         Joypad struct (memcpy)
 *   [1] cartridge_present (0/1)
 *   if present:
 *     [sizeof(InesHeader)]   InesHeader (memcpy)
 *     [mapper state via vtable save_state]
 *   [4] dma_stall_cycles (u32)
 *   [8] cpu_cycle_count (u64)
 *   [4] sample_accumulator (float)
 *   [4] audio_buffer_count (u32)
 *   [audio_buffer_count*4]  audio_buffer (floats)
 *   [4] ppu_cycle_carry (u32)
 *   [4] region (int)
 *
 * The PPU framebuffer, bg_pattern, and RenderPipeline are NOT serialized
 * (they are derived data). After load, the framebuffer is cleared to the
 * universal background color.
 *
 * See: https://www.nesdev.org/wiki/Save_state
 */
#include "save_state.hpp"
#include "emulator.hpp"
#include "bus.hpp"
#include "cpu.hpp"
#include "ppu.hpp"
#include "ppu_render.hpp"
#include "apu.hpp"
#include "joypad.hpp"
#include "cartridge.hpp"
#include "mapper.hpp"
#include "region.hpp"
#include <string.h>

/* Header: magic + version + total_payload_size. */
#define SS_HEADER_SIZE 12u

/* ---- Little-endian read/write helpers -------------------------------- */

static void ss_put_u32(uint8_t* p, uint32_t v) {
    p[0] = (uint8_t)(v & 0xFFu);
    p[1] = (uint8_t)((v >> 8) & 0xFFu);
    p[2] = (uint8_t)((v >> 16) & 0xFFu);
    p[3] = (uint8_t)((v >> 24) & 0xFFu);
}

static uint32_t ss_get_u32(const uint8_t* p) {
    return (uint32_t)p[0]
         | ((uint32_t)p[1] << 8)
         | ((uint32_t)p[2] << 16)
         | ((uint32_t)p[3] << 24);
}

static void ss_put_u64(uint8_t* p, uint64_t v) {
    for (int i = 0; i < 8; ++i) {
        p[i] = (uint8_t)((v >> (i * 8)) & 0xFFu);
    }
}

static uint64_t ss_get_u64(const uint8_t* p) {
    uint64_t v = 0;
    for (int i = 0; i < 8; ++i) {
        v |= ((uint64_t)p[i]) << (i * 8);
    }
    return v;
}

static void ss_put_float(uint8_t* p, float f) {
    uint32_t u;
    memcpy(&u, &f, sizeof(u));
    ss_put_u32(p, u);
}

static float ss_get_float(const uint8_t* p) {
    uint32_t u = ss_get_u32(p);
    float f;
    memcpy(&f, &u, sizeof(f));
    return f;
}

static void ss_put_u16(uint8_t* p, uint16_t v) {
    p[0] = (uint8_t)(v & 0xFFu);
    p[1] = (uint8_t)((v >> 8) & 0xFFu);
}

static uint16_t ss_get_u16(const uint8_t* p) {
    return (uint16_t)((uint16_t)p[0] | ((uint16_t)p[1] << 8));
}

/* ---- PPU architectural-state payload size ---------------------------- */

/* Constant-only variant for size computation (no Ppu instance needed). */
static size_t ss_ppu_state_size_const(uint32_t vram_size) {
    return 4u + vram_size + PPU_OAM_SIZE + PPU_PALETTE_SIZE
         + 4u + 2u + 2u + 1u + 1u + 1u + 1u + 2u + 2u + 1u + 4u + 4u;
}

static size_t ss_ppu_state_size(const Ppu* p) {
    return ss_ppu_state_size_const(p->vram_size);
}

/* Write PPU architectural state at `buf` (must have ss_ppu_state_size bytes).
 * Returns bytes written. */
static size_t ss_ppu_write(const Ppu* p, uint8_t* buf) {
    size_t off = 0;
    ss_put_u32(buf + off, p->vram_size); off += 4u;
    memcpy(buf + off, p->vram, p->vram_size); off += p->vram_size;
    memcpy(buf + off, p->oam, PPU_OAM_SIZE); off += PPU_OAM_SIZE;
    memcpy(buf + off, p->palette, PPU_PALETTE_SIZE); off += PPU_PALETTE_SIZE;
    buf[off++] = p->ppuctrl;
    buf[off++] = p->ppumask;
    buf[off++] = p->oamaddr;
    buf[off++] = p->ppustatus;
    ss_put_u16(buf + off, p->v); off += 2u;
    ss_put_u16(buf + off, p->t); off += 2u;
    buf[off++] = p->fine_x;
    buf[off++] = p->w ? 1u : 0u;
    buf[off++] = p->ppudata_buffer;
    buf[off++] = p->open_bus;
    ss_put_u16(buf + off, p->scanline); off += 2u;
    ss_put_u16(buf + off, p->cycle); off += 2u;
    buf[off++] = p->nmi_request ? 1u : 0u;
    ss_put_u32(buf + off, (uint32_t)p->mirroring); off += 4u;
    ss_put_u32(buf + off, (uint32_t)p->region); off += 4u;
    return off;
}

/* Read PPU architectural state from `buf`. Returns bytes read, or 0 on
 * failure (vram_size mismatch). Does NOT touch framebuffer / bg_pattern /
 * render pipeline (derived data). Caller must call ppu_set_mirroring and
 * ppu_set_region afterwards to sync config. */
static size_t ss_ppu_read(Ppu* p, const uint8_t* buf, size_t len) {
    if (len < 4u) {
        return 0;
    }
    uint32_t vram_size = ss_get_u32(buf);
    size_t need = ss_ppu_state_size_const(vram_size);
    if (len < need) {
        return 0;
    }
    if (vram_size > PPU_VRAM_SIZE_4K) {
        return 0; /* corrupt */
    }
    /* Compute the mirroring / region offsets so we can sync config BEFORE
     * copying VRAM (ppu_set_mirroring may zero-expand the vram array). */
    size_t mirr_off = 4u + vram_size + PPU_OAM_SIZE + PPU_PALETTE_SIZE
                    + 4u + 2u + 2u + 1u + 1u + 1u + 1u + 2u + 2u + 1u;
    uint32_t mirr = ss_get_u32(buf + mirr_off);
    uint32_t regn = ss_get_u32(buf + mirr_off + 4u);
    if (mirr <= (uint32_t)MIRROR_SINGLE_SCREEN_3) {
        ppu_set_mirroring(p, (Mirroring)mirr);
    }
    if (regn <= (uint32_t)REGION_DENDY) {
        ppu_set_region(p, (Region)regn);
    }
    /* Now p->vram_size matches the saved vram_size (or is as close as the
     * mirroring mode allows). Copy the VRAM data. */
    size_t off = 0;
    off += 4u; /* vram_size already read. */
    uint32_t copy = vram_size;
    if (copy > PPU_VRAM_SIZE_4K) {
        copy = PPU_VRAM_SIZE_4K;
    }
    memcpy(p->vram, buf + off, copy); off += vram_size;
    p->vram_size = vram_size;
    memcpy(p->oam, buf + off, PPU_OAM_SIZE); off += PPU_OAM_SIZE;
    memcpy(p->palette, buf + off, PPU_PALETTE_SIZE); off += PPU_PALETTE_SIZE;
    p->ppuctrl = buf[off++];
    p->ppumask = buf[off++];
    p->oamaddr = buf[off++];
    p->ppustatus = buf[off++];
    p->v = ss_get_u16(buf + off); off += 2u;
    p->t = ss_get_u16(buf + off); off += 2u;
    p->fine_x = buf[off++];
    p->w = buf[off++] ? true : false;
    p->ppudata_buffer = buf[off++];
    p->open_bus = buf[off++];
    p->scanline = ss_get_u16(buf + off); off += 2u;
    p->cycle = ss_get_u16(buf + off); off += 2u;
    p->nmi_request = buf[off++] ? true : false;
    off += 4u; /* mirroring already applied above. */
    off += 4u; /* region already applied above. */
    return off;
}

/* ---- Required-size computation --------------------------------------- */

size_t emulator_save_state_size(const EmulatorState* emu) {
    if (!emu) {
        return 0u;
    }
    size_t payload = 0;
    payload += sizeof(Cpu);
    payload += BUS_RAM_SIZE;
    /* PPU arch state: vram_size depends on current mirroring. */
    payload += ss_ppu_state_size(&emu->bus.ppu);
    payload += BUS_APU_IO_REG_COUNT;
    payload += sizeof(Apu);
    payload += sizeof(Joypad);
    /* Cartridge. */
    if (emu->cartridge) {
        payload += 1u + sizeof(InesHeader);
        payload += cartridge_save_state(emu->cartridge, NULL);
    } else {
        payload += 1u;
    }
    /* Emulator-level. */
    payload += 4u /* dma_stall_cycles */
            + 8u /* cpu_cycle_count */
            + 4u /* sample_accumulator */
            + 4u /* audio_buffer_count */
            + emu->audio_buffer_count * sizeof(float)
            + 4u /* ppu_cycle_carry */
            + 4u; /* region */
    return SS_HEADER_SIZE + payload;
}

/* ---- Save ------------------------------------------------------------ */

size_t emulator_save_state(const EmulatorState* emu, uint8_t* buf, size_t cap) {
    if (!emu) {
        return 0u;
    }
    size_t required = emulator_save_state_size(emu);
    if (!buf) {
        return required;
    }
    if (cap < required) {
        return 0u; /* buffer too small */
    }

    size_t payload = required - SS_HEADER_SIZE;
    size_t off = 0;
    /* Header. */
    ss_put_u32(buf + off, SAVE_STATE_MAGIC); off += 4u;
    ss_put_u32(buf + off, SAVE_STATE_VERSION); off += 4u;
    ss_put_u32(buf + off, (uint32_t)payload); off += 4u;

    /* Cpu struct. */
    memcpy(buf + off, &emu->cpu, sizeof(Cpu)); off += sizeof(Cpu);
    /* RAM. */
    memcpy(buf + off, emu->bus.ram, BUS_RAM_SIZE); off += BUS_RAM_SIZE;
    /* PPU architectural state. */
    off += ss_ppu_write(&emu->bus.ppu, buf + off);
    /* APU open-bus. */
    memcpy(buf + off, emu->bus.apu_open_bus, BUS_APU_IO_REG_COUNT);
    off += BUS_APU_IO_REG_COUNT;
    /* Apu struct. */
    memcpy(buf + off, &emu->bus.apu, sizeof(Apu)); off += sizeof(Apu);
    /* Joypad struct. */
    memcpy(buf + off, &emu->bus.joypad, sizeof(Joypad)); off += sizeof(Joypad);
    /* Cartridge. */
    if (emu->cartridge) {
        buf[off++] = 1u;
        memcpy(buf + off, &emu->cartridge->header, sizeof(InesHeader));
        off += sizeof(InesHeader);
        size_t msz = cartridge_save_state(emu->cartridge, buf + off);
        off += msz;
    } else {
        buf[off++] = 0u;
    }
    /* Emulator-level. */
    ss_put_u32(buf + off, bus_dma_stall_cycles(&emu->bus)); off += 4u;
    ss_put_u64(buf + off, bus_cpu_cycle_count(&emu->bus)); off += 8u;
    ss_put_float(buf + off, emu->sample_accumulator); off += 4u;
    ss_put_u32(buf + off, (uint32_t)emu->audio_buffer_count); off += 4u;
    if (emu->audio_buffer_count > 0u) {
        memcpy(buf + off, emu->audio_buffer,
               emu->audio_buffer_count * sizeof(float));
        off += emu->audio_buffer_count * sizeof(float);
    }
    ss_put_u32(buf + off, emu->ppu_cycle_carry); off += 4u;
    ss_put_u32(buf + off, (uint32_t)emu->region); off += 4u;

    return off;
}

/* ---- Load ------------------------------------------------------------ */

bool emulator_load_state(EmulatorState* emu, const uint8_t* buf, size_t len) {
    if (!emu || !buf || len < SS_HEADER_SIZE) {
        return false;
    }
    /* Header. */
    uint32_t magic = ss_get_u32(buf);
    uint32_t version = ss_get_u32(buf + 4);
    uint32_t payload = ss_get_u32(buf + 8);
    if (magic != SAVE_STATE_MAGIC) {
        return false;
    }
    if (version != SAVE_STATE_VERSION) {
        return false;
    }
    if ((size_t)payload + SS_HEADER_SIZE > len) {
        return false;
    }
    size_t off = SS_HEADER_SIZE;
    size_t end = SS_HEADER_SIZE + (size_t)payload;

    /* Cpu struct. */
    if (off + sizeof(Cpu) > end) return false;
    memcpy(&emu->cpu, buf + off, sizeof(Cpu)); off += sizeof(Cpu);
    /* RAM. */
    if (off + BUS_RAM_SIZE > end) return false;
    memcpy(emu->bus.ram, buf + off, BUS_RAM_SIZE); off += BUS_RAM_SIZE;
    /* PPU architectural state. */
    if (off + 4u > end) return false;
    uint32_t vram_size = ss_get_u32(buf + off);
    size_t ppu_need = ss_ppu_state_size_const(vram_size);
    if (off + ppu_need > end) return false;
    size_t ppu_read = ss_ppu_read(&emu->bus.ppu, buf + off, end - off);
    if (ppu_read == 0u) return false;
    off += ppu_read;
    /* APU open-bus. */
    if (off + BUS_APU_IO_REG_COUNT > end) return false;
    memcpy(emu->bus.apu_open_bus, buf + off, BUS_APU_IO_REG_COUNT);
    off += BUS_APU_IO_REG_COUNT;
    /* Apu struct. */
    if (off + sizeof(Apu) > end) return false;
    memcpy(&emu->bus.apu, buf + off, sizeof(Apu)); off += sizeof(Apu);
    /* Joypad struct. */
    if (off + sizeof(Joypad) > end) return false;
    memcpy(&emu->bus.joypad, buf + off, sizeof(Joypad)); off += sizeof(Joypad);
    /* Cartridge. */
    if (off + 1u > end) return false;
    uint8_t cart_present = buf[off++];
    if (cart_present) {
        if (off + sizeof(InesHeader) > end) return false;
        InesHeader hdr;
        memcpy(&hdr, buf + off, sizeof(InesHeader)); off += sizeof(InesHeader);
        /* Re-create the cartridge from the saved header. We need the PRG/CHR
         * data, but the save state only carries the mapper state (which
         * includes the PRG/CHR buffers for ROM, or just the writable parts
         * for RAM). The mapper state's save_state size tells us how much
         * to consume. We rebuild the cartridge by re-running
         * cartridge_from_bytes? We don't have the original ROM bytes.
         *
         * Approach: the existing cartridge (if any) was created from the
         * same ROM, so its mapper state struct already has the PRG/CHR
         * buffers allocated. We just load the saved mapper state into the
         * existing cartridge's mapper. If no cartridge is loaded, we
         * cannot restore (the PRG/CHR ROM data is not in the save state).
         * For the test (NOP ROM, deterministic), the cartridge is already
         * loaded. */
        if (!emu->cartridge) {
            return false; /* cannot restore cartridge without ROM data */
        }
        /* Verify the header matches (best-effort). */
        if (hdr.mapper_number != emu->cartridge->header.mapper_number) {
            return false;
        }
        /* Restore the header (in case battery flag etc. changed). */
        emu->cartridge->header = hdr;
        /* Determine the mapper state size from the vtable. */
        size_t msz = cartridge_save_state(emu->cartridge, NULL);
        if (off + msz > end) return false;
        if (!cartridge_load_state(emu->cartridge, buf + off, msz)) {
            return false;
        }
        off += msz;
        /* Re-sync PPU mirroring from the restored cartridge. */
        ppu_set_mirroring(&emu->bus.ppu, cartridge_mirror_mode(emu->cartridge));
        bus_insert_cartridge(&emu->bus, emu->cartridge);
    } else {
        /* No cartridge in the save state - remove any loaded cartridge.
         * (We do NOT free emu->cartridge here; the caller owns it and may
         * re-load. Just unlink from the bus.) */
        bus_remove_cartridge(&emu->bus);
    }
    /* Emulator-level. */
    if (off + 4u > end) return false;
    bus_set_dma_stall_cycles(&emu->bus, ss_get_u32(buf + off)); off += 4u;
    if (off + 8u > end) return false;
    bus_set_cpu_cycle_count(&emu->bus, ss_get_u64(buf + off)); off += 8u;
    if (off + 4u > end) return false;
    emu->sample_accumulator = ss_get_float(buf + off); off += 4u;
    if (off + 4u > end) return false;
    uint32_t audio_count = ss_get_u32(buf + off); off += 4u;
    /* Defensive overflow guard: reject a corrupt/huge audio_count before the
     * multiply so a 32-bit size_t cannot wrap to 0 and bypass the bound. */
    if (audio_count > (end - off) / sizeof(float)) return false;
    if (off + (size_t)audio_count * sizeof(float) > end) return false;
    emulator_set_audio_buffer(emu, (const float*)(buf + off), audio_count);
    off += (size_t)audio_count * sizeof(float);
    if (off + 4u > end) return false;
    emu->ppu_cycle_carry = ss_get_u32(buf + off); off += 4u;
    if (off + 4u > end) return false;
    uint32_t regn = ss_get_u32(buf + off); off += 4u;
    if (regn <= (uint32_t)REGION_DENDY) {
        emulator_set_region(emu, (Region)regn);
    }

    /* Clear the framebuffer to the universal background color (derived
     * data - the next step_frame will refill it). */
    uint32_t universal_bg = ppu_universal_bg_argb(&emu->bus.ppu);
    ppu_clear_framebuffer(&emu->bus.ppu, universal_bg);

    return true;
}
