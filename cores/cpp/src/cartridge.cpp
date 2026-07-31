/*
 * cartridge.c - iNES cartridge loader + mapper dispatch (M4.4).
 *
 * Port of src/cartridge.rs (header parse + from_bytes / from_fds_bytes) and
 * the FDS disk-image parser (src/fds.rs) to C. The Cartridge owns a Mapper
 * (vtable + opaque state) and dispatches all PRG/CHR/mirroring/IRQ/audio
 * calls through it. All 13 iNES mappers from the Rust core are supported.
 *
 * See: https://www.nesdev.org/wiki/INES
 * See: https://www.nesdev.org/wiki/FDS_disk_format
 */
#include "cartridge.hpp"
#include "mapper.hpp"
#include <string.h>

/* iNES file magic: bytes 0..=3 are "NES\x1A". (cartridge.rs `INE_MAGIC`.) */
static const uint8_t INES_MAGIC[4] = { 'N', 'E', 'S', 0x1A };

/* FDS disk image magic: bytes 0..=3 are "FDS\x1A". (fds.rs `FDS_MAGIC`.) */
static const uint8_t FDS_MAGIC[4] = { 'F', 'D', 'S', 0x1A };
#define FDS_HEADER_SIZE   16u
#define FDS_DISK_SIDE_SIZE 65500u
#define FDS_BIOS_SIZE     8192u

/* ---- Header parse (cartridge.rs `InesHeader::parse`) ----------------- */

int cartridge_parse_header(const uint8_t* bytes, size_t len, InesHeader* out) {
    if (len < CART_HEADER_SIZE) {
        return 1; /* TooShort */
    }
    if (memcmp(bytes, INES_MAGIC, 4) != 0) {
        return 2; /* BadMagic */
    }

    uint8_t prg_rom_banks = bytes[4];
    uint8_t chr_rom_banks = bytes[5];
    uint8_t flags6 = bytes[6];
    uint8_t flags7 = bytes[7];

    bool has_trainer  = (flags6 & 0x04u) != 0u;
    bool has_battery  = (flags6 & 0x02u) != 0u;
    bool four_screen  = (flags6 & 0x08u) != 0u;
    bool vertical     = (flags6 & 0x01u) != 0u;

    Mirroring mirroring;
    if (four_screen) {
        mirroring = MIRROR_FOUR_SCREEN;
    } else if (vertical) {
        mirroring = MIRROR_VERTICAL;
    } else {
        mirroring = MIRROR_HORIZONTAL;
    }

    /* Mapper low nibble from flags6 high nibble, high nibble from flags7 high nibble. */
    uint16_t mapper_number = (uint16_t)(((uint16_t)(flags6 >> 4)) | (uint16_t)(((uint16_t)(flags7 >> 4)) << 4));

    /* TV system hint from byte 9 bits 0-1. */
    uint8_t tv_system = (uint8_t)(bytes[9] & 0x03u);

    out->prg_rom_banks = prg_rom_banks;
    out->chr_rom_banks = chr_rom_banks;
    out->mapper_number = mapper_number;
    out->mirroring = mirroring;
    out->has_trainer = has_trainer;
    out->has_battery = has_battery;
    out->tv_system = tv_system;
    return 0;
}

/* ---- from_bytes: parse iNES + build mapper (cartridge.rs `from_bytes`) */

int cartridge_from_bytes(const uint8_t* bytes, size_t len, Cartridge* out) {
    InesHeader hdr;
    int rc = cartridge_parse_header(bytes, len, &hdr);
    if (rc != 0) {
        return rc;
    }

    uint32_t prg_size = (uint32_t)hdr.prg_rom_banks * CART_PRG_ROM_UNIT;
    /* Compute the byte offset where PRG-ROM begins in the file. */
    size_t prg_off = CART_HEADER_SIZE;
    if (hdr.has_trainer) {
        prg_off += CART_TRAINER_SIZE; /* skip 512-byte trainer */
    }
    size_t chr_size = (hdr.chr_rom_banks > 0u) ? (size_t)hdr.chr_rom_banks * CART_CHR_ROM_UNIT : 0u;
    if (len < prg_off + prg_size + chr_size) {
        return 5; /* TooShort for declared PRG/CHR. */
    }

    const uint8_t* prg_rom = bytes + prg_off;
    const uint8_t* chr_rom = (chr_size > 0u) ? (bytes + prg_off + prg_size) : NULL;

    memset(out, 0, sizeof(*out));
    out->header = hdr;

    rc = mapper_from_ines(hdr.mapper_number, prg_rom, prg_size,
                          chr_rom, (uint32_t)chr_size, hdr.mirroring,
                          hdr.has_battery, &out->mapper);
    if (rc != 0) {
        return rc;
    }
    return 0;
}

/* ---- from_fds_bytes: parse .fds + build FDS mapper ------------------- */

int cartridge_from_fds_bytes(const uint8_t* disk_data, size_t disk_len,
                             const uint8_t* bios, size_t bios_len,
                             Cartridge* out) {
    /* Validate the FDS disk image. */
    if (disk_len < FDS_HEADER_SIZE) {
        return 1; /* TooShort */
    }
    if (memcmp(disk_data, FDS_MAGIC, 4) != 0) {
        return 2; /* BadMagic */
    }
    uint8_t disk_count = disk_data[4];
    if (disk_count == 0u) {
        return 6; /* Io: disk count zero */
    }
    size_t needed = FDS_HEADER_SIZE + (size_t)disk_count * FDS_DISK_SIDE_SIZE;
    if (disk_len < needed) {
        return 1; /* truncated */
    }

    /* Concatenate all disk sides into a single raw buffer (the 16-byte FDS
     * file header is stripped - the BIOS reads starting from the disk info
     * block). We pass the raw side data to fds_create. */
    size_t raw_size = (size_t)disk_count * FDS_DISK_SIDE_SIZE;
    const uint8_t* raw_disk = disk_data + FDS_HEADER_SIZE;

    /* Synthetic iNES header for FDS: mapper 20, no PRG/CHR ROM, vertical
     * mirroring (switchable at runtime via $4025), no battery. */
    memset(out, 0, sizeof(*out));
    out->header.mapper_number = 20u;
    out->header.mirroring = MIRROR_VERTICAL;
    out->header.prg_rom_banks = 0u;
    out->header.chr_rom_banks = 0u;

    return fds_create(bios, bios_len, raw_disk, raw_size, &out->mapper);
}

/* ---- destroy --------------------------------------------------------- */

void cartridge_destroy(Cartridge* cart) {
    if (cart && cart->mapper.vt && cart->mapper.vt->destroy) {
        cart->mapper.vt->destroy(cart->mapper.state);
        cart->mapper.vt = NULL;
        cart->mapper.state = NULL;
    }
}

/* ---- PRG / CHR access (dispatch through the Mapper vtable) ----------- */

uint8_t cartridge_read_prg(const Cartridge* cart, uint16_t addr) {
    if (cart->mapper.vt && cart->mapper.vt->read_prg) {
        return cart->mapper.vt->read_prg(cart->mapper.state, addr);
    }
    return 0x00u;
}

uint8_t cartridge_read_prg_mut(Cartridge* cart, uint16_t addr) {
    if (cart->mapper.vt) {
        if (cart->mapper.vt->read_prg_mut) {
            return cart->mapper.vt->read_prg_mut(cart->mapper.state, addr);
        }
        if (cart->mapper.vt->read_prg) {
            return cart->mapper.vt->read_prg(cart->mapper.state, addr);
        }
    }
    return 0x00u;
}

void cartridge_write_prg(Cartridge* cart, uint16_t addr, uint8_t value) {
    if (cart->mapper.vt && cart->mapper.vt->write_prg) {
        cart->mapper.vt->write_prg(cart->mapper.state, addr, value);
    }
}

uint8_t cartridge_read_chr(const Cartridge* cart, uint16_t addr) {
    if (cart->mapper.vt && cart->mapper.vt->read_chr) {
        return cart->mapper.vt->read_chr(cart->mapper.state, addr);
    }
    return 0x00u;
}

uint8_t cartridge_read_chr_latched(Cartridge* cart, uint16_t addr) {
    if (cart->mapper.vt) {
        if (cart->mapper.vt->read_chr_latched) {
            return cart->mapper.vt->read_chr_latched(cart->mapper.state, addr);
        }
        if (cart->mapper.vt->read_chr) {
            return cart->mapper.vt->read_chr(cart->mapper.state, addr);
        }
    }
    return 0x00u;
}

void cartridge_write_chr(Cartridge* cart, uint16_t addr, uint8_t value) {
    if (cart->mapper.vt && cart->mapper.vt->write_chr) {
        cart->mapper.vt->write_chr(cart->mapper.state, addr, value);
    }
}

Mirroring cartridge_mirror_mode(const Cartridge* cart) {
    if (cart->mapper.vt && cart->mapper.vt->mirror_mode) {
        return cart->mapper.vt->mirror_mode(cart->mapper.state);
    }
    return cart->header.mirroring;
}

bool cartridge_chr_is_ram(const Cartridge* cart) {
    if (cart->mapper.vt && cart->mapper.vt->chr_is_ram) {
        return cart->mapper.vt->chr_is_ram(cart->mapper.state);
    }
    return false;
}

bool cartridge_has_battery(const Cartridge* cart) {
    if (cart->mapper.vt && cart->mapper.vt->has_battery) {
        return cart->mapper.vt->has_battery(cart->mapper.state);
    }
    return false;
}

bool cartridge_irq_pending(const Cartridge* cart) {
    if (cart->mapper.vt && cart->mapper.vt->irq_pending) {
        return cart->mapper.vt->irq_pending(cart->mapper.state);
    }
    return false;
}

void cartridge_clock_irq(Cartridge* cart) {
    if (cart->mapper.vt && cart->mapper.vt->clock_irq) {
        cart->mapper.vt->clock_irq(cart->mapper.state);
    }
}

void cartridge_reset_scanline_counter(Cartridge* cart) {
    if (cart->mapper.vt && cart->mapper.vt->reset_scanline_counter) {
        cart->mapper.vt->reset_scanline_counter(cart->mapper.state);
    }
}

void cartridge_clock_cpu(Cartridge* cart, uint32_t cpu_cycles) {
    if (cart->mapper.vt && cart->mapper.vt->clock_cpu) {
        cart->mapper.vt->clock_cpu(cart->mapper.state, cpu_cycles);
    }
}

float cartridge_expansion_audio_sample(const Cartridge* cart) {
    if (cart->mapper.vt && cart->mapper.vt->expansion_audio_sample) {
        return cart->mapper.vt->expansion_audio_sample(cart->mapper.state);
    }
    return 0.0f;
}

/* ---- Save / load state (M4.5) ---------------------------------------- */

size_t cartridge_save_state(const Cartridge* cart, uint8_t* buf) {
    if (cart->mapper.vt && cart->mapper.vt->save_state) {
        return cart->mapper.vt->save_state(cart->mapper.state, buf);
    }
    return 0u;
}

bool cartridge_load_state(Cartridge* cart, const uint8_t* buf, size_t len) {
    if (cart->mapper.vt && cart->mapper.vt->load_state) {
        return cart->mapper.vt->load_state(cart->mapper.state, buf, len);
    }
    return false;
}
