/*
 * bus.h — CPU memory bus: address-space routing + mirroring.
 *
 * Port of src/bus.rs to C. M4.1 introduced the address-space routing/decode
 * with PPU/APU devices routed to the open-bus latch (no real devices yet).
 * M4.2 plugs in a real PPU: the Bus now owns a `Ppu` struct (like Rust's Bus
 * owns Ppu), PPU register reads/writes ($2000-$3FFF) route to the PPU, and
 * OAM-DMA ($4014) copies 256 bytes into the PPU's OAM. The APU device is still
 * M4.3 — APU register accesses ($4000-$4017) continue to route to the
 * `apu_open_bus` latch.
 *
 * See: https://www.nesdev.org/wiki/CPU_memory_map
 */
#ifndef NES_CORE_C_BUS_H
#define NES_CORE_C_BUS_H

#include <stdint.h>
#include <stdbool.h>
#include "ppu.h"
#include "apu.h"

#ifdef __cplusplus
extern "C" {
#endif

/* Size of the CPU internal RAM in bytes (2 KB). (bus.rs `RAM_SIZE`.) */
#define BUS_RAM_SIZE 0x0800u
/* Mask applied to addresses in $0000-$1FFF to de-mirror RAM. (bus.rs `RAM_MASK`.) */
#define BUS_RAM_MASK 0x07FFu
/* Number of distinct PPU registers ($2000-$2007). (bus.rs `PPU_REG_COUNT`.) */
#define BUS_PPU_REG_COUNT 8u
/* Mask applied to addresses in $2000-$3FFF to find the PPU register index. */
#define BUS_PPU_REG_MASK 0x0007u
/* Base address of PPU register space. (bus.rs `PPU_REG_BASE`.) */
#define BUS_PPU_REG_BASE 0x2000u
/* Base address of APU / I/O register space. (bus.rs `APU_IO_BASE`.) */
#define BUS_APU_IO_BASE 0x4000u
/* Number of bytes in the APU / I/O register window ($4000-$4017). (bus.rs `APU_IO_REG_COUNT`.) */
#define BUS_APU_IO_REG_COUNT 0x18u
/* First address of cartridge space. (bus.rs `CART_BASE`.) */
#define BUS_CART_BASE 0x4020u
/* Last address of the APU / I/O test region (disabled on retail). (bus.rs `APU_IO_TEST_END`.) */
#define BUS_APU_IO_TEST_END 0x401Fu

struct Cartridge; /* forward decl from cartridge.h */

/* The CPU memory bus — owns RAM, the PPU, APU/IO open-bus latch, optional
 * cartridge, OAM-DMA stall counter, and CPU cycle counter. (bus.rs `struct
 * Bus`; the Bus owns the Ppu like Rust's Bus owns Ppu — M4.2.) */
typedef struct Bus {
    /* 2 KB internal CPU RAM ($0000-$07FF). */
    uint8_t ram[BUS_RAM_SIZE];

    /* The Picture Processing Unit. Owned by the bus (M4.2). PPU register
     * accesses ($2000-$3FFF) and OAM-DMA ($4014) route here. (bus.rs `ppu`.) */
    Ppu ppu;

    /* The Audio Processing Unit. Owned by the bus (M4.3). APU register
     * accesses ($4000-$4017) route here. (bus.rs `apu`.) */
    Apu apu;

    /* Open-bus latch for the APU/IO register window ($4000-$4017). Indexed by
     * (addr - 0x4000). Reads of write-only registers return the last written
     * value from this latch. (bus.rs `apu_open_bus`.) */
    uint8_t apu_open_bus[BUS_APU_IO_REG_COUNT];

    /* Loaded cartridge, if any. NULL when no cartridge is inserted. The Bus
     * does NOT own the cartridge (the caller owns it); this is a non-owning
     * pointer. (bus.rs uses Option<Cartridge> which owns; in C we keep the
     * ownership with the caller for simplicity in M4.1.) */
    struct Cartridge* cartridge;

    /* Pending OAM-DMA stall cycles (512 per DMA, 513 if aligned to odd CPU
     * cycle). (bus.rs `dma_stall_cycles`.) */
    uint32_t dma_stall_cycles;

    /* Total CPU cycles elapsed since power-on. Used to determine even/odd
     * cycle alignment for OAM-DMA. (bus.rs `cpu_cycle_count`.) */
    uint64_t cpu_cycle_count;
} Bus;

/* ---- Construction ----------------------------------------------------- */

/* Construct an empty bus with no cartridge and zeroed RAM/open-bus. (bus.rs `Bus::new`.) */
void bus_init(Bus* bus);

/* Construct a bus with a loaded cartridge (non-owning pointer). (bus.rs `with_cartridge`.) */
void bus_init_with_cartridge(Bus* bus, struct Cartridge* cartridge);

/* Replace the loaded cartridge (non-owning). Returns the previous pointer. */
struct Cartridge* bus_insert_cartridge(Bus* bus, struct Cartridge* cartridge);

/* Remove the loaded cartridge, returning the previous pointer (or NULL). */
struct Cartridge* bus_remove_cartridge(Bus* bus);

/* Borrow the loaded cartridge, if any. */
struct Cartridge* bus_cartridge(const Bus* bus);

/* ---- Read / Write (bus.rs `read` / `write`) --------------------------- */

/* Read a byte from CPU address space (side-effectful). Address decode matches
 * bus.rs `read` exactly: $0000-$1FFF RAM (mirrored), $2000-$3FFF PPU regs
 * (open-bus in M4.1), $4000-$4017 APU/IO (open-bus latch), $4018-$401F
 * disabled (0), $4020-$FFFF cartridge. */
uint8_t bus_read(Bus* bus, uint16_t addr);

/* Write a byte to the CPU address space. Address decode matches bus.rs `write`
 * exactly. PPU/APU register writes latch the open bus; $4014 triggers OAM-DMA. */
void bus_write(Bus* bus, uint16_t addr, uint8_t value);

/* Side-effect-free read for debug tools (bus.rs `peek`). PPU/APU ranges
 * return 0; cartridge space delegates to cart read_prg. */
uint8_t bus_peek(const Bus* bus, uint16_t addr);

/* ---- OAM-DMA (bus.rs `oam_dma` / `take_dma_stall_cycles`) ------------- */

/* Consume and return pending OAM-DMA stall cycles. */
uint32_t bus_take_dma_stall_cycles(Bus* bus);

/* ---- CPU cycle counter (bus.rs `advance_cpu_cycles` etc.) ------------- */

void bus_advance_cpu_cycles(Bus* bus, uint32_t cycles);
uint64_t bus_cpu_cycle_count(const Bus* bus);
void bus_set_cpu_cycle_count(Bus* bus, uint64_t count);

/* Pending DMA stall cycles accessors (bus.rs `dma_stall_cycles` / `set_dma_stall_cycles`). */
uint32_t bus_dma_stall_cycles(const Bus* bus);
void bus_set_dma_stall_cycles(Bus* bus, uint32_t cycles);

/* ---- RAM direct access (bus.rs `ram` / `ram_mut`) --------------------- */

const uint8_t* bus_ram(const Bus* bus);
uint8_t* bus_ram_mut(Bus* bus);

/* ---- APU open-bus direct access (bus.rs `apu_open_bus` / `_mut`) ------ */

const uint8_t* bus_apu_open_bus(const Bus* bus);
uint8_t* bus_apu_open_bus_mut(Bus* bus);

/* ---- PPU direct access (M4.2) ---------------------------------------- */

/* Borrow the bus's owned PPU. (The Bus owns the Ppu; this is the C equivalent
 * of bus.rs `&mut self.ppu`.) */
Ppu* bus_ppu(Bus* bus);
const Ppu* bus_ppu_const(const Bus* bus);

/* ---- APU direct access (M4.3) ---------------------------------------- */

/* Borrow the bus's owned APU. (The Bus owns the Apu; this is the C equivalent
 * of bus.rs `&mut self.apu`.) */
Apu* bus_apu(Bus* bus);
const Apu* bus_apu_const(const Bus* bus);

/* Advance APU by `cpu_cycles` (DMC DMA reads from RAM/cartridge).
 * (bus.rs `step_apu`.) */
void bus_step_apu(Bus* bus, uint32_t cpu_cycles);

/* Whether the APU has a pending IRQ (frame counter or DMC).
 * (bus.rs `apu_irq_pending`.) */
bool bus_apu_irq_pending(const Bus* bus);

#ifdef __cplusplus
}
#endif
#endif /* NES_CORE_C_BUS_H */

