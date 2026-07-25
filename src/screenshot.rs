//! Minimal PNG encoder for screenshots (std-only, no external crates).
//!
//! Uses stored/uncompressed deflate blocks; truecolor RGB, 8-bit depth.

use std::fs::File;
use std::io::{self, Write};
use std::path::Path;

/// PNG 8-byte file signature.
const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

/// Encode an ARGB framebuffer (`0xAARRGGBB` per pixel, row-major,
/// top-to-bottom) as a PNG byte vector. `width` and `height` are in
/// pixels; `fb.len()` must equal `width * height`.
///
/// The output is a complete, valid PNG file (signature + IHDR + IDAT +
/// IEND) using truecolor RGB (color type 2, bit depth 8) and zlib
/// stored-block compression.
pub fn encode_to_bytes(fb: &[u32], width: u32, height: u32) -> Vec<u8> {
    assert_eq!(
        fb.len(),
        (width as usize) * (height as usize),
        "framebuffer length must equal width * height"
    );

    let mut out: Vec<u8> = Vec::with_capacity(fb.len() * 3 + 256);
    out.extend_from_slice(&PNG_SIGNATURE);

    // IHDR: width, height, bit_depth=8, color_type=2 (RGB),
    // compression=0 (deflate), filter=0 (adaptive), interlace=0 (none).
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8; // bit depth
    ihdr[9] = 2; // color type: truecolor RGB
    ihdr[10] = 0; // compression method: deflate
    ihdr[11] = 0; // filter method: adaptive (per-row filter byte)
    ihdr[12] = 0; // interlace method: none
    write_chunk(&mut out, *b"IHDR", &ihdr);

    // Build the raw (pre-deflate) image data: one filter byte (0 = None)
    // per scanline, followed by the RGB triple for each pixel.
    let row_bytes = (width as usize) * 3;
    let mut raw = Vec::with_capacity((row_bytes + 1) * height as usize);
    for y in 0..height as usize {
        raw.push(0u8); // filter type: None
        let row_start = y * width as usize;
        for x in 0..width as usize {
            let px = fb[row_start + x];
            // ARGB 0xAARRGGBB → R, G, B
            raw.push(((px >> 16) & 0xFF) as u8);
            raw.push(((px >> 8) & 0xFF) as u8);
            raw.push((px & 0xFF) as u8);
        }
    }

    // Wrap the raw scanlines in a zlib stream (stored deflate blocks +
    // Adler-32 trailer) and emit as a single IDAT chunk.
    let zlib = zlib_stored(&raw);
    write_chunk(&mut out, *b"IDAT", &zlib);

    // IEND: empty data.
    write_chunk(&mut out, *b"IEND", &[]);

    out
}

/// Encode an ARGB framebuffer and write it to `path` as a PNG file.
/// Creates parent directories if needed. Returns an error if the file
/// cannot be written.
pub fn encode_to_path(fb: &[u32], width: u32, height: u32, path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let bytes = encode_to_bytes(fb, width, height);
    let mut file = File::create(path)?;
    file.write_all(&bytes)?;
    file.flush()?;
    Ok(())
}

/// Write a PNG chunk: `[length: u32 BE][type][data][CRC32: u32 BE]`.
/// The CRC covers `type + data` (per the PNG spec).
fn write_chunk(out: &mut Vec<u8>, chunk_type: [u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(&chunk_type);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(&chunk_type);
    crc_input.extend_from_slice(data);
    let crc = crc32(&crc_input);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// Wrap `data` in a zlib stream using uncompressed (stored) deflate
/// blocks. The stream is `[CMF][FLG][stored blocks...][Adler-32 BE]`.
///
/// Stored blocks have a max payload of 65535 bytes, so we split the
/// data into multiple blocks when it exceeds that limit.
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::with_capacity(data.len() + 16);
    // CMF: deflate method (8) + window size 32KB (7) → 0x78.
    const CMF: u8 = 0x78;
    // FLG: choose so (CMF*256 + FLG) % 31 == 0. 0x78*256 = 0x7800 = 30720.
    // 30720 % 31 = 30720 - 991*31 = 30720 - 30721 = -1 → need FLG such
    // that (30720 + FLG) % 31 == 0. 30720 % 31 = 30720 - 991*31 = -1
    // (i.e. 30 mod 31). FLG = 1 makes it 31 mod 31 = 0. No preset
    // dictionary (bit 5 of FLG = 0), compression level 0 (bits 6-7 = 0).
    const FLG: u8 = 0x01;
    out.push(CMF);
    out.push(FLG);

    if data.is_empty() {
        // Single empty final stored block: BFINAL=1, BTYPE=00, LEN=0,
        // NLEN=0xFFFF. The byte is 0x01 (BFINAL=1, BTYPE=00, padded
        // with zero bits).
        out.push(0x01);
        out.extend_from_slice(&0u16.to_le_bytes()); // LEN
        out.extend_from_slice(&0xFFFFu16.to_le_bytes()); // NLEN
    } else {
        let mut offset = 0usize;
        while offset < data.len() {
            let remaining = data.len() - offset;
            let block_len = remaining.min(65535);
            let is_final = offset + block_len == data.len();
            // Block header byte: bit 0 = BFINAL (1 if last block),
            // bits 1-2 = BTYPE (00 = stored). The remaining 5 bits are
            // padding (0). So the byte is 0x01 if final, 0x00 otherwise.
            out.push(if is_final { 0x01 } else { 0x00 });
            out.extend_from_slice(&(block_len as u16).to_le_bytes()); // LEN
            out.extend_from_slice(&(!(block_len as u16)).to_le_bytes()); // NLEN
            out.extend_from_slice(&data[offset..offset + block_len]);
            offset += block_len;
        }
    }

    // Adler-32 trailer (big-endian per RFC 1950).
    let adler = adler32(data);
    out.extend_from_slice(&adler.to_be_bytes());
    out
}

/// Compute the Adler-32 checksum of `data` per RFC 1950.
///
/// `s1 = 1 + sum(bytes)`, `s2 = sum of all s1 values`, both mod 65521.
/// The final value is `(s2 << 16) | s1`.
fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let mut s1: u32 = 1;
    let mut s2: u32 = 0;
    for &b in data {
        s1 = (s1 + b as u32) % MOD;
        s2 = (s2 + s1) % MOD;
    }
    (s2 << 16) | s1
}

/// Compute the CRC-32 checksum of `data` using the standard PNG/zlib
/// polynomial (0xEDB88320, reversed) with initial value 0xFFFFFFFF and
/// final XOR 0xFFFFFFFF.
///
/// A 256-entry table is computed lazily on first call and reused via a
/// `std::sync::OnceLock`.
fn crc32(data: &[u8]) -> u32 {
    use std::sync::OnceLock;
    static TABLE: OnceLock<[u32; 256]> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for n in 0..256u32 {
            let mut c = n;
            for _ in 0..8 {
                if c & 1 != 0 {
                    c = 0xEDB88320 ^ (c >> 1);
                } else {
                    c >>= 1;
                }
            }
            t[n as usize] = c;
        }
        t
    });

    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        let idx = ((crc ^ b as u32) & 0xFF) as usize;
        crc = (crc >> 8) ^ table[idx];
    }
    crc ^ 0xFFFF_FFFF
}

#[cfg(test)]
mod tests {
    use super::*;

    /// PNG signature must be exactly the 8 bytes from the spec.
    #[test]
    fn png_signature_is_correct() {
        assert_eq!(PNG_SIGNATURE, [137, 80, 78, 71, 13, 10, 26, 10]);
    }

    /// Adler-32 of an empty buffer is 1 (s1=1, s2=0).
    #[test]
    fn adler32_empty() {
        assert_eq!(adler32(&[]), 1);
    }

    /// Adler-32 of "Wikipedia" is 0x11E60398 (a known test vector from
    /// zlib's test suite).
    #[test]
    fn adler32_wikipedia_vector() {
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    /// CRC-32 of "123456789" is 0xCBF43926 (a standard test vector).
    #[test]
    fn crc32_standard_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    /// Decoding the zlib stored-block stream must yield the original
    /// raw scanline data (filter byte + RGB triples).
    #[test]
    fn zlib_stored_round_trips() {
        let raw = [0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let z = zlib_stored(&raw);
        // Strip 2-byte header + 4-byte adler trailer; decode stored
        // blocks.
        let mut decoded = Vec::new();
        let mut i = 2usize;
        let end = z.len() - 4;
        while i < end {
            let header = z[i];
            let bfinal = header & 1 != 0;
            i += 1;
            let len = u16::from_le_bytes([z[i], z[i + 1]]) as usize;
            i += 2;
            let _nlen = u16::from_le_bytes([z[i], z[i + 1]]);
            i += 2;
            decoded.extend_from_slice(&z[i..i + len]);
            i += len;
            if bfinal {
                break;
            }
        }
        assert_eq!(decoded, raw);
        // Adler-32 trailer must match.
        let adler_be = u32::from_be_bytes([
            z[z.len() - 4],
            z[z.len() - 3],
            z[z.len() - 2],
            z[z.len() - 1],
        ]);
        assert_eq!(adler_be, adler32(&raw));
    }
}
