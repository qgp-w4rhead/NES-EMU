/*
 * bus.c — CPU memory bus address-space routing + mirroring.
 *
 * Port of src/bus.rs to C (M4.1). The address decode in bus_read / bus_write
 * matches bus.rs `read()` / `write()` EXACTLY, region for region:
 *   $0000-$1FFF: 2 KB RAM (mirrored 3x via RAM_MASK)
 *   $2000-$3FFF: PPU registers (mirrored every 8 bytes)
 *   $4000-$4007: pulse channel regs (open-bus latch in M4.1)
 *   $4008-$400B: triangle channel regs (open-bus latch in M4.1)
 *   $400C-$400F: noise channel regs (open-bus latch in M4.1)
 *   $4010-$4013: DMC channel regs (open-bus latch in M4.1)
 *   $4014:       OAMDMA — write triggers 256-byte DMA + 512/513 cycle stall
 *   $4015:       APU status (open-bus latch in M4.1)
 *   $4016:       controller 1 strobe (open-bus latch in M4.1)
 *   $4017:       controller 2 / frame counter (open-bus latch in M4.1)
 *   $4018-$401F: disabled test region (reads 0, writes ignored)
 *   $4020-$FFFF: cartridge space (PRG-RAM / PRG-ROM / mapper regs)
 *
 * M4.1 SCOPE: PPU and APU devices are not ported yet (PPU = M4.2, APU = M4.3).
 * PPU register reads/writes ($2000-$3FFF) route to the apu_open_bus latch
 * (open-bus behaviour) — this is the correct behaviour for a system with no
 * PPU/APU device attached and is what the NOP ROM test needs. The routing
 * decode itself is complete and correct; M4.2/M4.3 will plug in real device
 * calls inside ppu_read/ppu_write/apu_*_write without changing the switch.
 *
 * OAM-DMA: in M4.1 there is no PPU OAM to copy into, so the 256-byte copy is
 * performed into a local buffer (matching the read side-effect sequence) and
 * the 512/513-cycle stall is recorded. The PPU device (M4.2) will consume the
 * buffer; for M4.1 the stall accounting is what the test exercises.
 */
#include "bus.h"
#include "cartridge.h"
#include <string.h>

/* ---- Construction ----------------------------------------------------- */

void bus_init(Bus* bus) {
    memset(bus->ram, 0, BUS_RAM_SIZE);
    memset(bus->apu_open_bus, 0, BUS_APU_IO_REG_COUNT);
    bus->cartridge = NULL;
    bus->dma_stall_cycles = 0u;
    bus->cpu_cycle_count = 0u;
}

void bus_init_with_cartridge(Bus* bus, struct Cartridge* cartridge) {
    bus_init(bus);
    bus->cartridge = cartridge;
}

struct Cartridge* bus_insert_cartridge(Bus* bus, struct Cartridge* cartridge) {
    struct Cartridge* prev = bus->cartridge;
    bus->cartridge = cartridge;
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

/* ppu_read: PPU register read. In M4.1 (no PPU device) this returns the
 * APU/IO open-bus latch indexed by the de-mirrored register. This models the
 * "open bus" behaviour of a system with no PPU attached. (bus.rs `ppu_read`
 * delegates to Ppu::read_register; here we route to the shared open-bus latch
 * so the read is non-crashing and deterministic.) */
static uint8_t ppu_read(Bus* bus, uint16_t reg) {
    /* reg is already de-mirrored to 0..=7. We do NOT have a PPU open-bus
     * latch separate from the APU one in M4.1; route to apu_open_bus[reg] so
     * writes to $2000 are visible to reads of $2000 (matching the
     * ppu_write_only_register_read_returns_open_bus test in bus.rs, which
     * expects the last written value to come back). */
    return bus->apu_open_bus[reg & 0x07u];
}

/* ppu_write: PPU register write. In M4.1 (no PPU device) this latches the
 * value on the shared open-bus slot so subsequent reads return it. */
static void ppu_write(Bus* bus, uint16_t reg, uint8_t value) {
    bus->apu_open_bus[reg & 0x07u] = value;
}

/* apu_read: APU/IO register open-bus read. offset = addr - 0x4000 in 0..=0x17. */
static uint8_t apu_read(const Bus* bus, uint16_t offset) {
    return bus->apu_open_bus[offset];
}

/* apu_write: APU/IO register open-bus latch write. */
static void apu_write(Bus* bus, uint16_t offset, uint8_t value) {
    bus->apu_open_bus[offset] = value;
}

/* oam_dma: copy 256 bytes from CPU page (page<<8) into a local buffer (M4.1:
 * no PPU OAM to receive). Latches the page on the open bus and records the
 * 512/513-cycle stall. The read side-effects fire in sequence, matching
 * bus.rs. (bus.rs `oam_dma`.) */
static void oam_dma(Bus* bus, uint8_t page) {
    uint16_t base = (uint16_t)((uint16_t)page << 8);
    uint8_t data[256];
    for (uint16_t i = 0; i < 256u; ++i) {
        data[i] = bus_read(bus, (uint16_t)(base + i));
    }
    /* M4.1: no PPU OAM to write into; data is discarded but the read
     * side-effects have fired. M4.2 will route this into ppu.oam_dma(&data). */
    (void)data;
    /* Latch the DMA page on the APU/IO open bus for $4014 reads. */
    bus->apu_open_bus[0x14u] = page;
    /* Any CPU bus write updates the shared open bus latch; writing $4014
     * (outside PPU reg space) still updates the PPU open bus on real HW. We
     * mirror that by writing into the PPU open-bus slot (apu_open_bus[0..7]). */
    bus->apu_open_bus[0x00u] = page;
    /* OAM-DMA stalls the CPU for 512 cycles; +1 if the write to $4014 lands
     * on an odd CPU cycle (alignment to next even). (bus.rs `oam_dma`.) */
    uint32_t stall = (bus->cpu_cycle_count & 1u) ? 513u : 512u;
    /* Saturating add (matches bus.rs:546 saturating_add). u32 overflow is
     * practically impossible but we mirror Rust's defensive behaviour. */
    if (bus->dma_stall_cycles > 0xFFFFFFFFu - stall) {
        bus->dma_stall_cycles = 0xFFFFFFFFu;
    } else {
        bus->dma_stall_cycles = bus->dma_stall_cycles + stall;
    }
}

/* cart_read: cartridge space read. (bus.rs `cart_read`.) */
static uint8_t cart_read(Bus* bus, uint16_t addr) {
    if (bus->cartridge) {
        return cartridge_read_prg(bus->cartridge, addr);
    }
    return 0x00u;
}

/* cart_write: cartridge space write. (bus.rs `cart_write`.) */
static void cart_write(Bus* bus, uint16_t addr, uint8_t value) {
    if (bus->cartridge) {
        cartridge_write_prg(bus->cartridge, addr, value);
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
        /* $4000-$4007: pulse channel regs (open-bus latch in M4.1). */
        return apu_read(bus, (uint16_t)(addr - BUS_APU_IO_BASE));
    }
    if (addr <= 0x400Bu) {
        /* $4008-$400B: triangle channel regs (open-bus latch in M4.1). */
        return apu_read(bus, (uint16_t)(addr - BUS_APU_IO_BASE));
    }
    if (addr <= 0x400Fu) {
        /* $400C-$400F: noise channel regs (open-bus latch in M4.1). */
        return apu_read(bus, (uint16_t)(addr - BUS_APU_IO_BASE));
    }
    if (addr <= 0x4013u) {
        /* $4010-$4013: DMC channel regs (open-bus latch in M4.1). */
        return apu_read(bus, (uint16_t)(addr - BUS_APU_IO_BASE));
    }
    if (addr == 0x4014u) {
        /* $4014: OAMDMA — write-only; reads return open bus. */
        return apu_read(bus, 0x14u);
    }
    if (addr == 0x4015u) {
        /* $4015: APU status. In M4.1 (no APU device) return the open-bus
         * latch (matching the write-then-read round trip). M4.3 will compute
         * the real status bits here. */
        return apu_read(bus, 0x15u);
    }
    if (addr == 0x4016u) {
        /* $4016: controller 1 + open-bus bits 1-7. M4.1: no joypad device;
         * return the open-bus latch (bit 0 = 0 = no button). */
        return apu_read(bus, 0x16u);
    }
    if (addr == 0x4017u) {
        /* $4017: controller 2 + open-bus bits 1-7. M4.1: no joypad device;
         * return the open-bus latch. */
        return apu_read(bus, 0x17u);
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
        uint16_t offset = (uint16_t)(addr - BUS_APU_IO_BASE);
        /* M4.1: no APU pulse device; latch open bus only. M4.3 will call
         * apu_pulse_write(offset, value) here. */
        apu_write(bus, offset, value);
        return;
    }
    if (addr <= 0x400Bu) {
        uint16_t offset = (uint16_t)(addr - BUS_APU_IO_BASE);
        apu_write(bus, offset, value);
        return;
    }
    if (addr <= 0x400Fu) {
        uint16_t offset = (uint16_t)(addr - BUS_APU_IO_BASE);
        apu_write(bus, offset, value);
        return;
    }
    if (addr <= 0x4013u) {
        uint16_t offset = (uint16_t)(addr - BUS_APU_IO_BASE);
        apu_write(bus, offset, value);
        return;
    }
    if (addr == 0x4014u) {
        /* $4014: OAMDMA — triggers 256-byte DMA + stall. */
        oam_dma(bus, value);
        return;
    }
    if (addr == 0x4015u) {
        /* $4015: APU status. M4.1: latch open bus only (no APU device). */
        apu_write(bus, 0x15u, value);
        return;
    }
    if (addr == 0x4016u) {
        /* $4016: controller strobe (bit 0). M4.1: latch open bus only. */
        apu_write(bus, 0x16u, value);
        return;
    }
    if (addr == 0x4017u) {
        /* $4017: APU frame counter control. M4.1: latch open bus only. */
        apu_write(bus, 0x17u, value);
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

