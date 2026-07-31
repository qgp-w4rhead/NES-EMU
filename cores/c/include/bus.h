/*
 * bus.h — CPU memory bus: address-space routing + mirroring.
 *
 * Port of src/bus.rs to C (M4.1). The Bus struct owns 2 KB RAM, the APU/IO
 * open-bus latch array (0x18 bytes covering $4000-$4017), an optional
 * Cartridge pointer, the OAM-DMA stall counter, and the CPU cycle counter.
 *
 * IMPORTANT (M4.1 scope): The PPU and APU devices are NOT ported yet
 * (PPU = M4.2, APU = M4.3). The address-space ROUTING/DECODE here is complete
 * and matches bus.rs read()/write() EXACTLY, but PPU register accesses
 * ($2000-$3FFF) and APU register accesses ($4000-$4017) route to the
 * `apu_open_bus` latch (open-bus behaviour) for now. This is the correct
 * behaviour for a system with no PPU/APU device attached and is exactly what
 * the NOP ROM test requires. When M4.2/M4.3 plug in real devices, the device
 * calls inside ppu_read/ppu_write/apu_*_write will be filled in — the routing
 * switch itself does not change.
 *
 * See: https://www.nesdev.org/wiki/CPU_memory_map
 */
#ifndef NES_CORE_C_BUS_H
#define NES_CORE_C_BUS_H

#include <stdint.h>
#include <stdbool.h>

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

/* The CPU memory bus — owns RAM, APU/IO open-bus latch, optional cartridge,
 * OAM-DMA stall counter, and CPU cycle counter. (bus.rs `struct Bus`, minus
 * the PPU/APU/Joypad devices which are M4.2/M4.3.) */
typedef struct Bus {
    /* 2 KB internal CPU RAM ($0000-$07FF). */
    uint8_t ram[BUS_RAM_SIZE];

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

#ifdef __cplusplus
}
#endif
#endif /* NES_CORE_C_BUS_H */

