/*
 * bus.c — CPU memory bus address-space routing + mirroring.
 *
 * Port of src/bus.rs to C. M4.1 introduced the address-space routing/decode
 * with PPU/APU devices routed to the open-bus latch. M4.2 plugs in a real PPU:
 * PPU register reads/writes ($2000-$3FFF) route to the bus's owned Ppu, with
 * PPUDATA (reg 7) handled here for CHR routing through the cartridge
 * (ppu_read_ppudata / ppu_write_ppudata, matching bus.rs:433-499). OAM-DMA
 * ($4014) now copies 256 bytes into the PPU's OAM via ppu_oam_dma. The APU
 * device is still M4.3 — APU register accesses ($4000-$4017) continue to
 * route to the `apu_open_bus` latch.
 *
 * Address decode in bus_read / bus_write matches bus.rs `read()` / `write()`
 * exactly, region for region:
 *   $0000-$1FFF: 2 KB RAM (mirrored 3x via RAM_MASK)
 *   $2000-$3FFF: PPU registers (mirrored every 8 bytes) -> Ppu (M4.2)
 *   $4000-$4007: pulse channel regs (open-bus latch; APU = M4.3)
 *   $4008-$400B: triangle channel regs (open-bus latch; APU = M4.3)
 *   $400C-$400F: noise channel regs (open-bus latch; APU = M4.3)
 *   $4010-$4013: DMC channel regs (open-bus latch; APU = M4.3)
 *   $4014:       OAMDMA — write triggers 256-byte DMA into PPU OAM + 512/513 stall
 *   $4015:       APU status (open-bus latch; APU = M4.3)
 *   $4016:       controller 1 strobe (open-bus latch; joypad = M4.5)
 *   $4017:       controller 2 / frame counter (open-bus latch; APU = M4.3)
 *   $4018-$401F: disabled test region (reads 0, writes ignored)
 *   $4020-$FFFF: cartridge space (PRG-RAM / PRG-ROM / mapper regs)
 */
#include "bus.h"
#include "cartridge.h"
#include "ppu.h"
#include "ppu_render.h"
#include "apu.h"
#include "region.h"
#include <string.h>

/* PPU cycle at which MMC3 IRQ is clocked (A12 rising edge approximation).
 * (bus.rs `MMC3_IRQ_CLOCK_CYCLE`.) */
#define MMC3_IRQ_CLOCK_CYCLE 260u

/* ---- Construction ----------------------------------------------------- */

void bus_init(Bus* bus) {
    memset(bus->ram, 0, BUS_RAM_SIZE);
    ppu_init(&bus->ppu);
    apu_init(&bus->apu);
    memset(bus->apu_open_bus, 0, BUS_APU_IO_REG_COUNT);
    bus->cartridge = NULL;
    bus->dma_stall_cycles = 0u;
    bus->cpu_cycle_count = 0u;
}

void bus_init_with_cartridge(Bus* bus, struct Cartridge* cartridge) {
    bus_init(bus);
    bus->cartridge = cartridge;
    if (cartridge) {
        ppu_set_mirroring(&bus->ppu, cartridge_mirror_mode(cartridge));
    }
}

struct Cartridge* bus_insert_cartridge(Bus* bus, struct Cartridge* cartridge) {
    struct Cartridge* prev = bus->cartridge;
    bus->cartridge = cartridge;
    if (cartridge) {
        ppu_set_mirroring(&bus->ppu, cartridge_mirror_mode(cartridge));
    }
    return prev;
}

struct Cartridge* bus_remove_cartridge(Bus* bus) {
    struct Cartridge* prev = bus->cartridge;
    bus->cartridge = NULL;
    return prev;
}

struct Cartridge* bus_cartridge(const Bus* bus) {
    return bus->cartridge;
}

/* ---- Internal helpers (mirror bus.rs private fn names) --------------- */

/* ppu_read_ppudata: PPUDATA ($2007) read with buffered-read semantics and CHR
 * routing through the cartridge. (bus.rs `ppu_read_ppudata`, lines 433-480.)
 * See: https://www.nesdev.org/wiki/PPU_registers#PPUDATA */
static uint8_t ppu_read_ppudata(Bus* bus) {
    uint16_t addr = ppu_vram_addr(&bus->ppu);

    if (addr >= 0x3F00u) {
        /* Palette: returned value comes from palette RAM directly (bypassing
         * the stale buffer), but the buffer is still loaded with the
         * nametable byte at v & 0x2FFF per NESdev. Upper two bits of the
         * value placed on the CPU bus come from the PPU open-bus latch. */
        uint8_t pal = ppu_read_palette(&bus->ppu, addr);
        uint8_t val = (uint8_t)((pal & 0x3Fu) | (ppu_open_bus(&bus->ppu) & 0xC0u));
        uint16_t nt_addr = (uint16_t)(addr & 0x2FFFu);
        uint8_t buffered_fill;
        if (nt_addr < 0x2000u) {
            buffered_fill = bus->cartridge ? cartridge_read_chr(bus->cartridge, nt_addr) : 0u;
        } else {
            buffered_fill = ppu_read_nametable(&bus->ppu, nt_addr);
        }
        ppu_set_ppudata_buffer(&bus->ppu, buffered_fill);
        ppu_advance_vram_addr(&bus->ppu);
        ppu_set_open_bus(&bus->ppu, val);
        return val;
    }

    /* Buffered read: return the stale buffer, store the fresh value. */
    uint8_t buffered = ppu_ppudata_buffer(&bus->ppu);
    uint8_t raw;
    if (addr < 0x2000u) {
        raw = bus->cartridge ? cartridge_read_chr(bus->cartridge, addr) : 0u;
    } else {
        raw = ppu_read_nametable(&bus->ppu, addr);
    }
    ppu_set_ppudata_buffer(&bus->ppu, raw);
    ppu_advance_vram_addr(&bus->ppu);
    ppu_set_open_bus(&bus->ppu, buffered);
    return buffered;
}

/* ppu_write_ppudata: PPUDATA ($2007) write with CHR routing and auto-increment.
 * (bus.rs `ppu_write_ppudata`, lines 483-499.) */
static void ppu_write_ppudata(Bus* bus, uint8_t value) {
    uint16_t addr = ppu_vram_addr(&bus->ppu);
    if (addr >= 0x3F00u) {
        ppu_write_palette(&bus->ppu, addr, value);
    } else if (addr < 0x2000u) {
        if (bus->cartridge) {
            cartridge_write_chr(bus->cartridge, addr, value);
        }
    } else {
        ppu_write_nametable(&bus->ppu, addr, value);
    }
    /* Update open-bus latch (consistent with other PPU register writes). */
    ppu_write_register(&bus->ppu, 7u, value); /* latches open_bus; reg 7 no-op */
    ppu_advance_vram_addr(&bus->ppu);
}

/* ppu_read: PPU register read. Reg 7 (PPUDATA) is handled here for CHR
 * routing; all other regs delegate to ppu_read_register. (bus.rs `ppu_read`.) */
static uint8_t ppu_read(Bus* bus, uint16_t reg) {
    if ((reg & 0x07u) == 7u) {
        return ppu_read_ppudata(bus);
    }
    return ppu_read_register(&bus->ppu, reg);
}

/* ppu_write: PPU register write. Reg 7 (PPUDATA) is handled here for CHR
 * routing; all other regs delegate to ppu_write_register. (bus.rs `ppu_write`.) */
static void ppu_write(Bus* bus, uint16_t reg, uint8_t value) {
    if ((reg & 0x07u) == 7u) {
        ppu_write_ppudata(bus, value);
        return;
    }
    ppu_write_register(&bus->ppu, reg, value);
}

/* apu_read_open_bus: APU/IO register open-bus read. offset = addr - 0x4000
 * in 0..=0x17. (bus.rs `apu_read`.) */
static uint8_t apu_read_open_bus(const Bus* bus, uint16_t offset) {
    return bus->apu_open_bus[offset];
}

/* apu_pulse_write: route a pulse register write. offset = addr - 0x4000 in
 * 0..=7: offsets 0-3 -> pulse 1, offsets 4-7 -> pulse 2.
 * (bus.rs `apu_pulse_write`.) */
static void apu_pulse_write(Bus* bus, uint16_t offset, uint8_t value) {
    if (offset < 4u) {
        pulse_write_register(&bus->apu.pulse1, (uint8_t)offset, value);
    } else {
        pulse_write_register(&bus->apu.pulse2, (uint8_t)(offset - 4u), value);
    }
}

/* apu_triangle_write: route a triangle register write. offset = addr - 0x4000
 * in 8..=0xB: reg = offset - 8. (bus.rs `apu_triangle_write`.) */
static void apu_triangle_write(Bus* bus, uint16_t offset, uint8_t value) {
    triangle_write_register(&bus->apu.triangle, (uint8_t)(offset - 0x08u), value);
}

/* apu_noise_write: route a noise register write. offset = addr - 0x4000 in
 * 0xC..=0xF: reg = offset - 0xC. (bus.rs `apu_noise_write`.) */
static void apu_noise_write(Bus* bus, uint16_t offset, uint8_t value) {
    noise_write_register(&bus->apu.noise, (uint8_t)(offset - 0x0Cu), value);
}

/* apu_dmc_write: route a DMC register write. offset = addr - 0x4000 in
 * 0x10..=0x13: reg = offset - 0x10. (bus.rs `apu_dmc_write`.) */
static void apu_dmc_write(Bus* bus, uint16_t offset, uint8_t value) {
    dmc_write_register(&bus->apu.dmc, (uint8_t)(offset - 0x10u), value);
}

/* apu_status_read: read $4015 APU status (channel bits + IRQ flags; clears
 * IRQs). Bit 5 is open bus. (bus.rs `apu_status_read`.) */
static uint8_t apu_status_read(Bus* bus) {
    uint8_t status = apu_read_status(&bus->apu);
    return (uint8_t)((status & 0xDFu) | (bus->apu_open_bus[0x15u] & 0x20u));
}

/* oam_dma: copy 256 bytes from CPU page (page<<8) into the PPU's OAM via
 * ppu_oam_dma. Latches the page on the open bus and records the 512/513-cycle
 * stall. The read side-effects fire in sequence, matching bus.rs.
 * (bus.rs `oam_dma`, lines 509-547.) */
static void oam_dma(Bus* bus, uint8_t page) {
    uint16_t base = (uint16_t)((uint16_t)page << 8);
    uint8_t data[256];
    for (uint16_t i = 0; i < 256u; ++i) {
        data[i] = bus_read(bus, (uint16_t)(base + i));
    }
    ppu_oam_dma(&bus->ppu, data);
    /* Latch the DMA page on the APU/IO open bus for $4014 reads. */
    bus->apu_open_bus[0x14u] = page;
    /* Any CPU bus write updates the shared open bus latch; writing $4014
     * (outside PPU reg space) still updates the PPU open bus on real HW. */
    ppu_set_open_bus(&bus->ppu, page);
    /* OAM-DMA stalls the CPU for 512 cycles; +1 if the write to $4014 lands
     * on an odd CPU cycle (alignment to next even). (bus.rs `oam_dma`.) */
    uint32_t stall = (bus->cpu_cycle_count & 1u) ? 513u : 512u;
    /* Saturating add (matches bus.rs:546 saturating_add). */
    if (bus->dma_stall_cycles > 0xFFFFFFFFu - stall) {
        bus->dma_stall_cycles = 0xFFFFFFFFu;
    } else {
        bus->dma_stall_cycles = bus->dma_stall_cycles + stall;
    }
}

/* cart_read: cartridge space read. Uses read_prg_mut so read side-effects
 * fire (FDS disk-data read advances the read pointer, $4030 clears timer IRQ).
 * (bus.rs `cart_read`, lines 704-710.) */
static uint8_t cart_read(Bus* bus, uint16_t addr) {
    if (bus->cartridge) {
        return cartridge_read_prg_mut(bus->cartridge, addr);
    }
    return 0x00u;
}

/* cart_write: cartridge space write. Re-syncs PPU mirroring from the
 * cartridge after every write so runtime mirroring changes (MMC1 $8000,
 * MMC3 $A000, AxROM bit 4, VRC6 $B003, FME-7 cmd 12, FDS $4025) take effect
 * immediately. (bus.rs `cart_write`, lines 713-719.) */
static void cart_write(Bus* bus, uint16_t addr, uint8_t value) {
    if (bus->cartridge) {
        cartridge_write_prg(bus->cartridge, addr, value);
        ppu_set_mirroring(&bus->ppu, cartridge_mirror_mode(bus->cartridge));
    }
}

/* ---- bus_read (bus.rs `read`) ---------------------------------------- */

uint8_t bus_read(Bus* bus, uint16_t addr) {
    if (addr <= 0x1FFFu) {
        /* $0000-$1FFF: 2 KB RAM (mirrored 3 times). */
        return bus->ram[addr & BUS_RAM_MASK];
    }
    if (addr <= 0x3FFFu) {
        /* $2000-$3FFF: PPU registers (mirrored every 8 bytes). */
        return ppu_read(bus, addr & BUS_PPU_REG_MASK);
    }
    if (addr <= 0x4007u) {
        /* $4000-$4007: pulse channel regs (write-only; reads return open bus). */
        return apu_read_open_bus(bus, (uint16_t)(addr - BUS_APU_IO_BASE));
    }
    if (addr <= 0x400Bu) {
        /* $4008-$400B: triangle channel regs (write-only; reads return open bus). */
        return apu_read_open_bus(bus, (uint16_t)(addr - BUS_APU_IO_BASE));
    }
    if (addr <= 0x400Fu) {
        /* $400C-$400F: noise channel regs (write-only; reads return open bus). */
        return apu_read_open_bus(bus, (uint16_t)(addr - BUS_APU_IO_BASE));
    }
    if (addr <= 0x4013u) {
        /* $4010-$4013: DMC channel regs (write-only; reads return open bus). */
        return apu_read_open_bus(bus, (uint16_t)(addr - BUS_APU_IO_BASE));
    }
    if (addr == 0x4014u) {
        /* $4014: OAMDMA - write-only; reads return open bus. */
        return apu_read_open_bus(bus, 0x14u);
    }
    if (addr == 0x4015u) {
        /* $4015: APU status - channel bits + IRQ flags; bit 5 is open bus.
         * Reading clears both IRQ flags. (bus.rs `apu_status_read`.) */
        return apu_status_read(bus);
    }
    if (addr == 0x4016u) {
        /* $4016: controller 1 + open-bus bits 1-7. M4.5: joypad device; until
         * then return the open-bus latch (bit 0 = 0 = no button). */
        return apu_read_open_bus(bus, 0x16u);
    }
    if (addr == 0x4017u) {
        /* $4017: controller 2 + open-bus bits 1-7. M4.5: joypad device. */
        return apu_read_open_bus(bus, 0x17u);
    }
    if (addr <= 0x401Fu) {
        /* $4018-$401F: APU/IO test mode — disabled, reads as open bus (0). */
        return 0x00u;
    }
    /* $4020-$FFFF: cartridge space. */
    return cart_read(bus, addr);
}

/* ---- bus_write (bus.rs `write`) -------------------------------------- */

void bus_write(Bus* bus, uint16_t addr, uint8_t value) {
    if (addr <= 0x1FFFu) {
        bus->ram[addr & BUS_RAM_MASK] = value;
        return;
    }
    if (addr <= 0x3FFFu) {
        ppu_write(bus, addr & BUS_PPU_REG_MASK, value);
        return;
    }
    if (addr <= 0x4007u) {
        /* $4000-$4007: pulse channel regs. Routed to APU pulse channels;
         * also latched on the open bus. (bus.rs lines 340-344.) */
        uint16_t offset = (uint16_t)(addr - BUS_APU_IO_BASE);
        apu_pulse_write(bus, offset, value);
        bus->apu_open_bus[offset] = value;
        return;
    }
    if (addr <= 0x400Bu) {
        /* $4008-$400B: triangle channel regs. (bus.rs lines 347-351.) */
        uint16_t offset = (uint16_t)(addr - BUS_APU_IO_BASE);
        apu_triangle_write(bus, offset, value);
        bus->apu_open_bus[offset] = value;
        return;
    }
    if (addr <= 0x400Fu) {
        /* $400C-$400F: noise channel regs. (bus.rs lines 354-358.) */
        uint16_t offset = (uint16_t)(addr - BUS_APU_IO_BASE);
        apu_noise_write(bus, offset, value);
        bus->apu_open_bus[offset] = value;
        return;
    }
    if (addr <= 0x4013u) {
        /* $4010-$4013: DMC channel regs. (bus.rs lines 362-366.) */
        uint16_t offset = (uint16_t)(addr - BUS_APU_IO_BASE);
        apu_dmc_write(bus, offset, value);
        bus->apu_open_bus[offset] = value;
        return;
    }
    if (addr == 0x4014u) {
        /* $4014: OAMDMA — triggers 256-byte DMA + stall. */
        oam_dma(bus, value);
        return;
    }
    if (addr == 0x4015u) {
        /* $4015: APU status — enables/disables all 5 channels (bits 0-4).
         * Also latched on the open bus for bit 5 preservation. */
        bus->apu_open_bus[0x15u] = value;
        apu_write_status(&bus->apu, value);
        return;
    }
    if (addr == 0x4016u) {
        /* $4016: controller strobe (bit 0). M4.5: joypad device; until then
         * latch open bus only. */
        bus->apu_open_bus[0x16u] = value;
        return;
    }
    if (addr == 0x4017u) {
        /* $4017: APU frame counter control. Routed to the APU frame counter;
         * also latched on the open bus. (bus.rs lines 388-391.) */
        bus->apu_open_bus[0x17u] = value;
        apu_write_frame_counter(&bus->apu, value);
        return;
    }
    if (addr <= 0x401Fu) {
        /* $4018-$401F: disabled test region — ignore writes. */
        return;
    }
    /* $4020-$FFFF: cartridge space. */
    cart_write(bus, addr, value);
}

/* ---- bus_peek (bus.rs `peek`) ---------------------------------------- */

uint8_t bus_peek(const Bus* bus, uint16_t addr) {
    if (addr <= 0x1FFFu) {
        return bus->ram[addr & BUS_RAM_MASK];
    }
    if (addr <= 0x3FFFu) {
        /* PPU registers: not safely readable without device; report 0. */
        return 0x00u;
    }
    if (addr <= 0x401Fu) {
        return 0x00u;
    }
    /* $4020-$FFFF: cartridge space (PRG reads have no read side-effects). */
    if (bus->cartridge) {
        return cartridge_read_prg(bus->cartridge, addr);
    }
    return 0x00u;
}

/* ---- OAM-DMA stall / cycle counter accessors ------------------------- */

uint32_t bus_take_dma_stall_cycles(Bus* bus) {
    uint32_t c = bus->dma_stall_cycles;
    bus->dma_stall_cycles = 0u;
    return c;
}

void bus_advance_cpu_cycles(Bus* bus, uint32_t cycles) {
    bus->cpu_cycle_count = bus->cpu_cycle_count + (uint64_t)cycles;
}

uint64_t bus_cpu_cycle_count(const Bus* bus) {
    return bus->cpu_cycle_count;
}

void bus_set_cpu_cycle_count(Bus* bus, uint64_t count) {
    bus->cpu_cycle_count = count;
}

uint32_t bus_dma_stall_cycles(const Bus* bus) {
    return bus->dma_stall_cycles;
}

void bus_set_dma_stall_cycles(Bus* bus, uint32_t cycles) {
    bus->dma_stall_cycles = cycles;
}

/* ---- RAM / APU open-bus direct access -------------------------------- */

const uint8_t* bus_ram(const Bus* bus) {
    return bus->ram;
}

uint8_t* bus_ram_mut(Bus* bus) {
    return bus->ram;
}

const uint8_t* bus_apu_open_bus(const Bus* bus) {
    return bus->apu_open_bus;
}

uint8_t* bus_apu_open_bus_mut(Bus* bus) {
    return bus->apu_open_bus;
}

/* ---- PPU direct access (M4.2) ---------------------------------------- */

Ppu* bus_ppu(Bus* bus) {
    return &bus->ppu;
}

const Ppu* bus_ppu_const(const Bus* bus) {
    return &bus->ppu;
}

/* ---- APU direct access (M4.3) ---------------------------------------- */

/* dmc_read_cb: DMC DMA read callback. Reads from CPU RAM ($0000-$1FFF,
 * mirrored) or cartridge PRG space ($8000-$FFFF); other ranges return 0.
 * (bus.rs `step_apu` closure, lines 172-184.) */
static uint8_t dmc_read_cb(uint16_t addr, void* ctx) {
    Bus* bus = (Bus*)ctx;
    if (addr <= 0x1FFFu) {
        return bus->ram[addr & BUS_RAM_MASK];
    }
    if (addr >= 0x8000u) {
        return bus->cartridge ? cartridge_read_prg(bus->cartridge, addr) : 0x00u;
    }
    return 0x00u;
}

Apu* bus_apu(Bus* bus) {
    return &bus->apu;
}

const Apu* bus_apu_const(const Bus* bus) {
    return &bus->apu;
}

void bus_step_apu(Bus* bus, uint32_t cpu_cycles) {
    /* DMC DMA reads from RAM/cartridge via dmc_read_cb. (bus.rs `step_apu`.) */
    apu_step(&bus->apu, cpu_cycles, dmc_read_cb, bus);
}

bool bus_apu_irq_pending(const Bus* bus) {
    return apu_irq_pending(&bus->apu);
}

/* ---- Cartridge mapper integration (M4.4) ----------------------------- */

bool bus_cart_irq_pending(const Bus* bus) {
    return bus->cartridge ? cartridge_irq_pending(bus->cartridge) : false;
}

void bus_clock_cart_cpu(Bus* bus, uint32_t cpu_cycles) {
    if (bus->cartridge) {
        cartridge_clock_cpu(bus->cartridge, cpu_cycles);
    }
}

float bus_expansion_audio_sample(const Bus* bus) {
    return bus->cartridge ? cartridge_expansion_audio_sample(bus->cartridge) : 0.0f;
}

void bus_cart_reset_scanline_counter(Bus* bus) {
    if (bus->cartridge) {
        cartridge_reset_scanline_counter(bus->cartridge);
    }
}

/* ---- PPU stepping + rendering (M4.4) --------------------------------- */

/* ChrReader callback: reads CHR via the bus's cartridge. */
static uint8_t bus_chr_read(void* ctx, uint16_t addr) {
    Bus* bus = (Bus*)ctx;
    return bus->cartridge ? cartridge_read_chr_latched(bus->cartridge, addr) : 0x00u;
}

bool bus_step_ppu(Bus* bus, uint32_t cycles) {
    bool nmi = false;
    uint16_t prerender = (uint16_t)region_scanline_prerender(bus->ppu.region);
    bool rendering = ppu_is_rendering(&bus->ppu);
    for (uint32_t i = 0; i < cycles; ++i) {
        ChrReader chr = { bus, bus_chr_read };
        if (ppu_step_rendered(&bus->ppu, &chr)) {
            nmi = true;
        }
        uint16_t cyc = ppu_cycle(&bus->ppu);
        uint16_t sl  = ppu_scanline(&bus->ppu);
        /* Clock MMC3 IRQ at the A12 rising-edge approximation cycle. */
        if (rendering
            && cyc == MMC3_IRQ_CLOCK_CYCLE
            && (sl < PPU_SCREEN_HEIGHT || sl == prerender)) {
            if (bus->cartridge) {
                cartridge_clock_irq(bus->cartridge);
            }
        }
        /* Reset scanline counter at prerender scanline, cycle 1. */
        if (sl == prerender && cyc == 1u) {
            if (bus->cartridge) {
                cartridge_reset_scanline_counter(bus->cartridge);
            }
        }
    }
    return nmi;
}

bool bus_take_nmi_request(Bus* bus) {
    return ppu_take_nmi_request(&bus->ppu);
}

void bus_render_frame(Bus* bus) {
    ChrReader chr = { bus, bus_chr_read };
    ppu_render_frame(&bus->ppu, &chr);
}

