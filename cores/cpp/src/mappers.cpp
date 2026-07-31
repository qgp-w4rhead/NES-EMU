/*
 * mappers.c - iNES mapper dispatch (from_ines). Port of src/mappers/mod.rs
 * `from_ines` to C (M4.4).
 *
 * Dispatches on the iNES mapper number to the appropriate per-mapper
 * constructor (nrom_create, mmc1_create, ...). Each constructor mallocs its
 * state, copies PRG/CHR, and fills out the Mapper struct. FDS (mapper 20) is
 * NOT handled here - it uses cartridge_from_fds_bytes (BIOS + disk data).
 */
#include "mapper.hpp"
#include "cartridge.hpp"
#include <stdint.h>

int mapper_from_ines(uint16_t mapper_number,
                     const uint8_t* prg_rom, uint32_t prg_size,
                     const uint8_t* chr_rom, uint32_t chr_size,
                     Mirroring mirroring, bool has_battery,
                     Mapper* out) {
    switch (mapper_number) {
        case 0u:  return nrom_create(prg_rom, prg_size, chr_rom, chr_size, mirroring, has_battery, out);
        case 1u:  return mmc1_create(prg_rom, prg_size, chr_rom, chr_size, mirroring, has_battery, out);
        case 2u:  return uxrom_create(prg_rom, prg_size, chr_rom, chr_size, mirroring, has_battery, out);
        case 3u:  return cnrom_create(prg_rom, prg_size, chr_rom, chr_size, mirroring, has_battery, out);
        case 4u:  return mmc3_create(prg_rom, prg_size, chr_rom, chr_size, mirroring, has_battery, out);
        case 5u:  return mmc5_create(prg_rom, prg_size, chr_rom, chr_size, mirroring, has_battery, out);
        case 7u:  return axrom_create(prg_rom, prg_size, chr_rom, chr_size, mirroring, has_battery, out);
        case 9u:  return mmc2_create(prg_rom, prg_size, chr_rom, chr_size, mirroring, has_battery, out);
        case 19u: return namco163_create(prg_rom, prg_size, chr_rom, chr_size, mirroring, has_battery, out);
        case 24u: return vrc6_create(prg_rom, prg_size, chr_rom, chr_size, mirroring, has_battery, false, out);
        case 26u: return vrc6_create(prg_rom, prg_size, chr_rom, chr_size, mirroring, has_battery, true, out);
        case 69u: return fme7_create(prg_rom, prg_size, chr_rom, chr_size, mirroring, has_battery, out);
        case 85u: return vrc7_create(prg_rom, prg_size, chr_rom, chr_size, mirroring, has_battery, out);
        default:  return 3; /* UnsupportedMapper */
    }
}
